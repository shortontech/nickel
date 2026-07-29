use cosmic_text::{
    Align, Attrs, Buffer, Color as TextColor, Family, FontSystem, Metrics, Shaping, SwashCache,
    Weight,
};
use nickel_render_assets::{TextAssetCache, TextRequest, TextWeight};
use sdl3::{
    pixels::{Color as SdlColor, PixelFormat},
    rect::Rect as SdlRect,
    render::{BlendMode, FRect, Texture, WindowCanvas},
    surface::{Surface, SurfaceRef},
    video::Window,
};

use crate::{Color, GradientAxis, PaintCommand, Rect, TextAlign};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pixel {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Pixel {
    const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);

    const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

/// Physical pixels changed by the latest component frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DamageRegion {
    pub rects: Vec<Rect>,
}

impl DamageRegion {
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }
}

/// SDL presentation backend for the platform-neutral component tree.
///
/// Component layout and hit testing remain in `UiTree`; this type only rasterizes
/// its paint list. The retained pixel buffer makes an eventual SDL GPU upload or
/// Wayland damage submission independent of component semantics.
pub struct SdlComponentRenderer {
    width: u32,
    height: u32,
    scale: f32,
    pixels: Vec<Pixel>,
    upload: Surface<'static>,
    previous_commands: Vec<PaintCommand>,
    font_system: FontSystem,
    swash_cache: SwashCache,
    text_cache: HashMap<TextCacheKey, Arc<Vec<CachedGlyphPixel>>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TextCacheKey {
    text: String,
    font_size: u32,
    width: u32,
    height: u32,
    align: u8,
    bold: bool,
    color: Color,
}

type CachedGlyphPixel = (i32, i32, u32, u32, TextColor);

/// Transitional name retained while callers move from the old wgpu backend.
pub type ComponentGpu = SdlComponentRenderer;

pub struct SdlCanvasPresenter {
    canvas: WindowCanvas,
    text_assets: TextAssetCache,
    text_textures: HashMap<DirectTextKey, Texture>,
    image_textures: HashMap<u16, Texture>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct DirectTextKey {
    text: String,
    size: u32,
    color: Color,
    bold: bool,
}

impl SdlCanvasPresenter {
    pub fn new(window: Window) -> Result<Self, String> {
        sdl3::hint::set("SDL_RENDER_VSYNC", "0");
        let canvas = window.into_canvas();
        let vsync_disabled = unsafe { sdl3::sys::render::SDL_SetRenderVSync(canvas.raw(), 0) };
        if !vsync_disabled {
            tracing::warn!(
                target: "nickel",
                error = %sdl3::get_error(),
                "failed to disable SDL renderer vsync"
            );
        }
        tracing::info!(
            target: "nickel",
            renderer = %canvas.renderer_name,
            "SDL accelerated presenter initialized"
        );
        Ok(Self {
            canvas,
            text_assets: TextAssetCache::new(),
            text_textures: HashMap::new(),
            image_textures: HashMap::new(),
        })
    }

    pub fn window(&self) -> &Window {
        self.canvas.window()
    }

    pub fn window_mut(&mut self) -> &mut Window {
        self.canvas.window_mut()
    }

    pub fn present_accelerated(
        &mut self,
        commands: &[PaintCommand],
        scale: f32,
    ) -> Result<DamageRegion, String> {
        let (width, height) = self.canvas.window().size_in_pixels();
        self.canvas.set_blend_mode(BlendMode::Blend);
        self.canvas.set_clip_rect(None);
        self.canvas.set_draw_color(SdlColor::RGBA(0, 0, 0, 0));
        self.canvas.clear();
        let mut clips = vec![Rect::new(0.0, 0.0, width as f32, height as f32)];
        for command in commands {
            self.draw_accelerated(command, scale, &mut clips)?;
        }
        self.canvas.set_clip_rect(None);
        self.canvas.present();
        Ok(DamageRegion {
            rects: vec![Rect::new(0.0, 0.0, width as f32, height as f32)],
        })
    }

    fn draw_accelerated(
        &mut self,
        command: &PaintCommand,
        scale: f32,
        clips: &mut Vec<Rect>,
    ) -> Result<(), String> {
        let clip = *clips.last().expect("accelerated clip stack has a root");
        self.canvas.set_clip_rect(Some(sdl_rect(clip)));
        match command {
            PaintCommand::Fill { rect, color } | PaintCommand::OverlayFill { rect, color } => {
                self.direct_fill(physical_rect(*rect, scale), *color)?;
            }
            PaintCommand::TopRoundedFill {
                rect,
                color,
                radius,
            } => self.direct_rounded_fill(
                physical_rect(*rect, scale),
                radius * scale,
                0b0011,
                *color,
            )?,
            PaintCommand::RoundedFill {
                rect,
                color,
                radius,
            } => self.direct_rounded_fill(
                physical_rect(*rect, scale),
                radius * scale,
                0b1111,
                *color,
            )?,
            PaintCommand::Gradient { rect, gradient } => {
                self.direct_gradient(physical_rect(*rect, scale), *gradient)?;
            }
            PaintCommand::Stroke { rect, color, width }
            | PaintCommand::OverlayStroke { rect, color, width } => {
                self.direct_stroke(physical_rect(*rect, scale), width * scale, *color)?;
            }
            PaintCommand::Text {
                bounds,
                text,
                scale: text_scale,
                color,
                align,
                bold,
            } => self.direct_text(
                physical_rect(*bounds, scale),
                text,
                text_size(*text_scale) * scale,
                *color,
                *align,
                *bold,
                clip,
            )?,
            PaintCommand::Image {
                bounds, id, image, ..
            } => self.direct_image(physical_rect(*bounds, scale), *id, image)?,
            PaintCommand::PushClip(rect) => {
                let rect = physical_rect(*rect, scale);
                clips.push(intersection(clip, rect).unwrap_or_default());
                self.canvas
                    .set_clip_rect(Some(sdl_rect(*clips.last().expect("pushed clip"))));
            }
            PaintCommand::PopClip => {
                if clips.len() > 1 {
                    clips.pop();
                }
                self.canvas
                    .set_clip_rect(Some(sdl_rect(*clips.last().expect("restored clip"))));
            }
        }
        Ok(())
    }

    fn direct_fill(&mut self, rect: Rect, color: Color) -> Result<(), String> {
        self.canvas.set_draw_color(sdl_color(color));
        self.canvas
            .fill_rect(frect(rect))
            .map_err(|error| error.to_string())
    }

    fn direct_stroke(&mut self, rect: Rect, width: f32, color: Color) -> Result<(), String> {
        let width = width.max(1.0);
        for edge in [
            Rect::new(rect.origin.x, rect.origin.y, rect.size.width, width),
            Rect::new(
                rect.origin.x,
                rect.origin.y + rect.size.height - width,
                rect.size.width,
                width,
            ),
            Rect::new(rect.origin.x, rect.origin.y, width, rect.size.height),
            Rect::new(
                rect.origin.x + rect.size.width - width,
                rect.origin.y,
                width,
                rect.size.height,
            ),
        ] {
            self.direct_fill(edge, color)?;
        }
        Ok(())
    }

    fn direct_rounded_fill(
        &mut self,
        rect: Rect,
        radius: f32,
        corners: u8,
        color: Color,
    ) -> Result<(), String> {
        if radius <= 0.5 {
            return self.direct_fill(rect, color);
        }
        self.canvas.set_draw_color(sdl_color(color));
        let rows = rect.size.height.ceil().max(1.0) as u32;
        for row in 0..rows {
            let y = row as f32 + 0.5;
            let top = y;
            let bottom = rect.size.height - y;
            let top_corner = top < radius && corners & 0b0011 != 0;
            let bottom_corner = bottom < radius && corners & 0b1100 != 0;
            let inset = if top_corner {
                (radius - (radius * radius - (radius - top).powi(2)).sqrt()).max(0.0)
            } else if bottom_corner {
                (radius - (radius * radius - (radius - bottom).powi(2)).sqrt()).max(0.0)
            } else {
                0.0
            };
            self.canvas
                .fill_rect(FRect::new(
                    rect.origin.x + inset,
                    rect.origin.y + row as f32,
                    (rect.size.width - inset * 2.0).max(0.0),
                    1.0,
                ))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn direct_gradient(
        &mut self,
        rect: Rect,
        gradient: crate::LinearGradient,
    ) -> Result<(), String> {
        let steps = match gradient.axis {
            GradientAxis::Horizontal => rect.size.width,
            GradientAxis::Vertical => rect.size.height,
        }
        .ceil()
        .max(1.0) as u32;
        for step in 0..steps {
            let progress = if steps <= 1 {
                0.0
            } else {
                step as f32 / (steps - 1) as f32
            };
            self.canvas.set_draw_color(sdl_pixel(lerp_color(
                gradient.start,
                gradient.end,
                progress,
            )));
            let strip = match gradient.axis {
                GradientAxis::Horizontal => FRect::new(
                    rect.origin.x + step as f32,
                    rect.origin.y,
                    1.0,
                    rect.size.height,
                ),
                GradientAxis::Vertical => FRect::new(
                    rect.origin.x,
                    rect.origin.y + step as f32,
                    rect.size.width,
                    1.0,
                ),
            };
            self.canvas
                .fill_rect(strip)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn direct_text(
        &mut self,
        bounds: Rect,
        text: &str,
        size: f32,
        color: Color,
        align: TextAlign,
        bold: bool,
        parent_clip: Rect,
    ) -> Result<(), String> {
        let key = DirectTextKey {
            text: text.to_owned(),
            size: size.to_bits(),
            color,
            bold,
        };
        if !self.text_textures.contains_key(&key) {
            if self.text_textures.len() >= 512 {
                for (_, texture) in self.text_textures.drain() {
                    // SAFETY: every texture was created by `self.canvas`, which
                    // remains alive for the duration of this presenter.
                    unsafe { texture.destroy() };
                }
                self.text_assets.clear();
            }
            let rgba = pixel(color);
            let asset = self.text_assets.get(TextRequest {
                text,
                size,
                line_height: size * 1.3,
                max_width: None,
                color: [rgba.r, rgba.g, rgba.b, rgba.a],
                weight: if bold {
                    TextWeight::Bold
                } else {
                    TextWeight::Normal
                },
            });
            let creator = self.canvas.texture_creator();
            let mut texture = creator
                .create_texture_streaming(PixelFormat::ABGR8888, asset.width(), asset.height())
                .map_err(|error| error.to_string())?;
            texture
                .update(None, asset.pixels(), asset.pitch())
                .map_err(|error| error.to_string())?;
            texture.set_blend_mode(BlendMode::Blend);
            self.text_textures.insert(key.clone(), texture);
        }
        let texture = self
            .text_textures
            .get(&key)
            .expect("inserted accelerated text texture");
        let query = texture.query();
        let x = match align {
            TextAlign::Start => bounds.origin.x,
            TextAlign::Center => bounds.origin.x + (bounds.size.width - query.width as f32) * 0.5,
            TextAlign::End => bounds.origin.x + bounds.size.width - query.width as f32,
        };
        let clip = intersection(parent_clip, bounds).unwrap_or_default();
        self.canvas.set_clip_rect(Some(sdl_rect(clip)));
        self.canvas
            .copy(
                texture,
                None,
                FRect::new(x, bounds.origin.y, query.width as f32, query.height as f32),
            )
            .map_err(|error| error.to_string())?;
        self.canvas.set_clip_rect(Some(sdl_rect(parent_clip)));
        Ok(())
    }

    fn direct_image(
        &mut self,
        bounds: Rect,
        id: u16,
        image: &image::RgbaImage,
    ) -> Result<(), String> {
        if !self.image_textures.contains_key(&id) {
            if self.image_textures.len() >= 512 {
                for (_, texture) in self.image_textures.drain() {
                    // SAFETY: every texture was created by `self.canvas`, which
                    // remains alive for the duration of this presenter.
                    unsafe { texture.destroy() };
                }
            }
            let creator = self.canvas.texture_creator();
            let mut texture = creator
                .create_texture_streaming(
                    PixelFormat::ABGR8888,
                    image.width().max(1),
                    image.height().max(1),
                )
                .map_err(|error| error.to_string())?;
            texture
                .update(None, image.as_raw(), image.width().max(1) as usize * 4)
                .map_err(|error| error.to_string())?;
            texture.set_blend_mode(BlendMode::Blend);
            self.image_textures.insert(id, texture);
        }
        if image.width() == 0 || image.height() == 0 {
            return Ok(());
        }
        let fit = (bounds.size.width / image.width() as f32)
            .min(bounds.size.height / image.height() as f32);
        let width = image.width() as f32 * fit;
        let height = image.height() as f32 * fit;
        let destination = FRect::new(
            bounds.origin.x + (bounds.size.width - width) * 0.5,
            bounds.origin.y + (bounds.size.height - height) * 0.5,
            width,
            height,
        );
        self.canvas
            .copy(
                self.image_textures
                    .get(&id)
                    .expect("inserted accelerated image texture"),
                None,
                destination,
            )
            .map_err(|error| error.to_string())
    }
}

impl SdlComponentRenderer {
    pub fn new(width: u32, height: u32, scale: f32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let mut upload =
            Surface::new(width, height, PixelFormat::ABGR8888).expect("create SDL upload surface");
        upload
            .set_blend_mode(BlendMode::None)
            .expect("disable SDL upload blending");
        Self {
            width,
            height,
            scale: scale.max(0.25),
            pixels: vec![Pixel::TRANSPARENT; (width * height) as usize],
            upload,
            previous_commands: Vec::new(),
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            text_cache: HashMap::new(),
        }
    }

    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        let width = width.max(1);
        let height = height.max(1);
        self.scale = scale.max(0.25);
        if (width, height) != (self.width, self.height) {
            self.width = width;
            self.height = height;
            self.pixels
                .resize((self.width * self.height) as usize, Pixel::TRANSPARENT);
            self.upload = Surface::new(self.width, self.height, PixelFormat::ABGR8888)
                .expect("resize SDL upload surface");
            self.upload
                .set_blend_mode(BlendMode::None)
                .expect("disable SDL upload blending");
            self.previous_commands.clear();
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn pixels(&self) -> &[Pixel] {
        &self.pixels
    }

    pub fn invalidate(&mut self) {
        self.previous_commands.clear();
    }

    /// Rasterize a component display list and return its conservative damage.
    pub fn render(&mut self, commands: &[PaintCommand]) -> DamageRegion {
        let damage = self.damage(commands);
        if damage.is_empty() {
            return damage;
        }

        let full = physical_rect(
            Rect::new(0.0, 0.0, self.width as f32, self.height as f32),
            1.0,
        );
        let repaint = damage
            .rects
            .iter()
            .copied()
            .reduce(union_rect)
            .and_then(|rect| intersection(full, rect))
            .unwrap_or(full);
        self.clear(repaint);
        let mut clips = vec![repaint];
        for command in commands {
            self.draw_command(command, &mut clips);
        }
        self.previous_commands.clear();
        self.previous_commands.extend_from_slice(commands);
        damage
    }

    fn clear(&mut self, rect: Rect) {
        let left = rect.origin.x.floor().max(0.0) as u32;
        let top = rect.origin.y.floor().max(0.0) as u32;
        let right = (rect.origin.x + rect.size.width)
            .ceil()
            .clamp(0.0, self.width as f32) as u32;
        let bottom = (rect.origin.y + rect.size.height)
            .ceil()
            .clamp(0.0, self.height as f32) as u32;
        for y in top..bottom {
            let start = (y * self.width + left) as usize;
            let end = (y * self.width + right) as usize;
            self.pixels[start..end].fill(Pixel::TRANSPARENT);
        }
    }

    /// Rasterize and copy the frame into an SDL window surface.
    pub fn present(
        &mut self,
        surface: &mut SurfaceRef,
        commands: &[PaintCommand],
    ) -> Result<DamageRegion, String> {
        let damage = self.render(commands);
        if damage.is_empty() {
            return Ok(damage);
        }
        self.upload.with_lock_mut(|bytes| {
            for (color, pixel) in self.pixels.iter().zip(bytes.chunks_exact_mut(4)) {
                pixel.copy_from_slice(&[color.r, color.g, color.b, color.a]);
            }
        });
        self.upload
            .blit(None, surface, SdlRect::new(0, 0, self.width, self.height))
            .map_err(|error| error.to_string())?;
        Ok(damage)
    }

    fn damage(&self, commands: &[PaintCommand]) -> DamageRegion {
        if self.previous_commands.is_empty() {
            return DamageRegion {
                rects: vec![Rect::new(0.0, 0.0, self.width as f32, self.height as f32)],
            };
        }
        let mut union = None;
        let count = commands.len().max(self.previous_commands.len());
        for index in 0..count {
            let old = self.previous_commands.get(index);
            let new = commands.get(index);
            if old != new {
                for command in [old, new].into_iter().flatten() {
                    if let Some(rect) = command_bounds(command) {
                        let rect = physical_rect(rect, self.scale);
                        union = Some(match union {
                            Some(current) => union_rect(current, rect),
                            None => rect,
                        });
                    }
                }
            }
        }
        if union.is_none() && commands != self.previous_commands {
            union = Some(Rect::new(0.0, 0.0, self.width as f32, self.height as f32));
        }
        DamageRegion {
            rects: union.into_iter().collect(),
        }
    }

    fn draw_command(&mut self, command: &PaintCommand, clips: &mut Vec<Rect>) {
        let clip = *clips.last().expect("clip stack always has the surface");
        match command {
            PaintCommand::Fill { rect, color } | PaintCommand::OverlayFill { rect, color } => {
                self.fill_round(physical_rect(*rect, self.scale), 0.0, 0b1111, *color, clip);
            }
            PaintCommand::TopRoundedFill {
                rect,
                color,
                radius,
            } => self.fill_round(
                physical_rect(*rect, self.scale),
                radius * self.scale,
                0b0011,
                *color,
                clip,
            ),
            PaintCommand::RoundedFill {
                rect,
                color,
                radius,
            } => self.fill_round(
                physical_rect(*rect, self.scale),
                radius * self.scale,
                0b1111,
                *color,
                clip,
            ),
            PaintCommand::Gradient { rect, gradient } => {
                self.fill_gradient(physical_rect(*rect, self.scale), *gradient, clip);
            }
            PaintCommand::Stroke { rect, color, width }
            | PaintCommand::OverlayStroke { rect, color, width } => {
                self.stroke(
                    physical_rect(*rect, self.scale),
                    width * self.scale,
                    *color,
                    clip,
                );
            }
            PaintCommand::Text { .. } => self.text(command, clip),
            PaintCommand::Image { bounds, image, .. } => {
                self.image(physical_rect(*bounds, self.scale), image, clip);
            }
            PaintCommand::PushClip(rect) => {
                let rect = physical_rect(*rect, self.scale);
                clips.push(intersection(clip, rect).unwrap_or_default());
            }
            PaintCommand::PopClip => {
                if clips.len() > 1 {
                    clips.pop();
                }
            }
        }
    }

    fn fill_round(&mut self, rect: Rect, radius: f32, corners: u8, color: Color, clip: Rect) {
        let Some(bounds) = intersection(rect, clip) else {
            return;
        };
        let radius = radius
            .max(0.0)
            .min(rect.size.width / 2.0)
            .min(rect.size.height / 2.0);
        self.for_pixels(bounds, |renderer, x, y| {
            let left = x as f32 + 0.5 - rect.origin.x;
            let top = y as f32 + 0.5 - rect.origin.y;
            let right = rect.size.width - left;
            let bottom = rect.size.height - top;
            let rounded = ((corners & 0b0001 != 0 && left < radius && top < radius)
                .then_some((left - radius, top - radius)))
            .or_else(|| {
                (corners & 0b0010 != 0 && right < radius && top < radius)
                    .then_some((right - radius, top - radius))
            })
            .or_else(|| {
                (corners & 0b0100 != 0 && right < radius && bottom < radius)
                    .then_some((right - radius, bottom - radius))
            })
            .or_else(|| {
                (corners & 0b1000 != 0 && left < radius && bottom < radius)
                    .then_some((left - radius, bottom - radius))
            });
            if rounded.is_none_or(|(dx, dy)| dx * dx + dy * dy <= radius * radius) {
                renderer.blend(x, y, pixel(color));
            }
        });
    }

    fn fill_gradient(&mut self, rect: Rect, gradient: crate::LinearGradient, clip: Rect) {
        let Some(bounds) = intersection(rect, clip) else {
            return;
        };
        self.for_pixels(bounds, |renderer, x, y| {
            let progress = match gradient.axis {
                GradientAxis::Horizontal => (x as f32 + 0.5 - rect.origin.x) / rect.size.width,
                GradientAxis::Vertical => (y as f32 + 0.5 - rect.origin.y) / rect.size.height,
            }
            .clamp(0.0, 1.0);
            renderer.blend(x, y, lerp_color(gradient.start, gradient.end, progress));
        });
    }

    fn stroke(&mut self, rect: Rect, width: f32, color: Color, clip: Rect) {
        let width = width.max(1.0);
        for edge in [
            Rect::new(rect.origin.x, rect.origin.y, rect.size.width, width),
            Rect::new(
                rect.origin.x,
                rect.origin.y + rect.size.height - width,
                rect.size.width,
                width,
            ),
            Rect::new(rect.origin.x, rect.origin.y, width, rect.size.height),
            Rect::new(
                rect.origin.x + rect.size.width - width,
                rect.origin.y,
                width,
                rect.size.height,
            ),
        ] {
            self.fill_round(edge, 0.0, 0, color, clip);
        }
    }

    fn text(&mut self, command: &PaintCommand, clip: Rect) {
        let PaintCommand::Text {
            bounds,
            text,
            scale,
            color,
            align,
            bold,
        } = command
        else {
            return;
        };
        let font_size = text_size(*scale) * self.scale;
        let physical = physical_rect(*bounds, self.scale);
        let buffer_width = physical.size.width.max(1.0);
        let buffer_height = physical.size.height.max(font_size * 1.4);
        let key = TextCacheKey {
            text: text.clone(),
            font_size: font_size.to_bits(),
            width: buffer_width.to_bits(),
            height: buffer_height.to_bits(),
            align: match align {
                TextAlign::Start => 0,
                TextAlign::Center => 1,
                TextAlign::End => 2,
            },
            bold: *bold,
            color: *color,
        };
        let glyph_pixels = if let Some(cached) = self.text_cache.get(&key) {
            Arc::clone(cached)
        } else {
            let mut buffer = Buffer::new(
                &mut self.font_system,
                Metrics::new(font_size, font_size * 1.3),
            );
            buffer.set_size(Some(buffer_width), Some(buffer_height));
            let mut attrs = Attrs::new().family(Family::SansSerif);
            if *bold {
                attrs = attrs.weight(Weight::BOLD);
            }
            buffer.set_text(text, &attrs, Shaping::Advanced, None);
            for line in &mut buffer.lines {
                line.set_align(Some(match align {
                    TextAlign::Start => Align::Left,
                    TextAlign::Center => Align::Center,
                    TextAlign::End => Align::Right,
                }));
            }
            buffer.shape_until_scroll(&mut self.font_system, false);
            let pixel = pixel(*color);
            let text_color = TextColor::rgba(pixel.r, pixel.g, pixel.b, pixel.a);
            let mut pixels = Vec::new();
            buffer.draw(
                &mut self.font_system,
                &mut self.swash_cache,
                text_color,
                |x, y, width, height, glyph_color| {
                    pixels.push((x, y, width, height, glyph_color));
                },
            );
            if self.text_cache.len() >= 2048 {
                self.text_cache.clear();
            }
            let pixels = Arc::new(pixels);
            self.text_cache.insert(key, Arc::clone(&pixels));
            pixels
        };
        // Cosmic Text emits already-rasterized 1x1 physical pixels. Keep their
        // origin on the physical pixel grid; a fractional origin would make
        // `for_pixels` expand each sample across adjacent pixels.
        let origin = crate::Point {
            x: physical.origin.x.round(),
            y: physical.origin.y.round(),
        };
        for &(x, y, width, height, glyph_color) in glyph_pixels.iter() {
            let glyph = Rect::new(
                origin.x + x as f32,
                origin.y + y as f32,
                width as f32,
                height as f32,
            );
            let Some(glyph) = intersection(glyph, clip) else {
                continue;
            };
            self.for_pixels(glyph, |renderer, px, py| {
                renderer.blend(
                    px,
                    py,
                    Pixel::rgba(
                        glyph_color.r(),
                        glyph_color.g(),
                        glyph_color.b(),
                        glyph_color.a(),
                    ),
                );
            });
        }
    }

    fn image(&mut self, rect: Rect, image: &image::RgbaImage, clip: Rect) {
        if image.width() == 0 || image.height() == 0 {
            return;
        }
        let scale =
            (rect.size.width / image.width() as f32).min(rect.size.height / image.height() as f32);
        let width = image.width() as f32 * scale;
        let height = image.height() as f32 * scale;
        let rect = Rect::new(
            rect.origin.x + (rect.size.width - width) * 0.5,
            rect.origin.y + (rect.size.height - height) * 0.5,
            width,
            height,
        );
        let Some(bounds) = intersection(rect, clip) else {
            return;
        };
        self.for_pixels(bounds, |renderer, x, y| {
            let source_x =
                (((x as f32 + 0.5 - rect.origin.x) / rect.size.width) * image.width() as f32)
                    .floor()
                    .clamp(0.0, image.width().saturating_sub(1) as f32) as u32;
            let source_y =
                (((y as f32 + 0.5 - rect.origin.y) / rect.size.height) * image.height() as f32)
                    .floor()
                    .clamp(0.0, image.height().saturating_sub(1) as f32) as u32;
            let pixel = image.get_pixel(source_x, source_y).0;
            renderer.blend(
                px(x),
                px(y),
                Pixel::rgba(pixel[0], pixel[1], pixel[2], pixel[3]),
            );
        });
    }

    fn for_pixels(&mut self, rect: Rect, mut draw: impl FnMut(&mut Self, u32, u32)) {
        let x_start = rect.origin.x.floor().max(0.0) as u32;
        let y_start = rect.origin.y.floor().max(0.0) as u32;
        let x_end = (rect.origin.x + rect.size.width)
            .ceil()
            .min(self.width as f32) as u32;
        let y_end = (rect.origin.y + rect.size.height)
            .ceil()
            .min(self.height as f32) as u32;
        for y in y_start..y_end {
            for x in x_start..x_end {
                draw(self, x, y);
            }
        }
    }

    fn blend(&mut self, x: u32, y: u32, source: Pixel) {
        let target = &mut self.pixels[(y * self.width + x) as usize];
        let alpha = u16::from(source.a);
        let inverse = 255 - alpha;
        target.r = ((u16::from(source.r) * alpha + u16::from(target.r) * inverse) / 255) as u8;
        target.g = ((u16::from(source.g) * alpha + u16::from(target.g) * inverse) / 255) as u8;
        target.b = ((u16::from(source.b) * alpha + u16::from(target.b) * inverse) / 255) as u8;
        target.a = (alpha + u16::from(target.a) * inverse / 255).min(255) as u8;
    }
}

fn command_bounds(command: &PaintCommand) -> Option<Rect> {
    match command {
        PaintCommand::Fill { rect, .. }
        | PaintCommand::TopRoundedFill { rect, .. }
        | PaintCommand::RoundedFill { rect, .. }
        | PaintCommand::Gradient { rect, .. }
        | PaintCommand::Stroke { rect, .. }
        | PaintCommand::OverlayFill { rect, .. }
        | PaintCommand::OverlayStroke { rect, .. } => Some(*rect),
        PaintCommand::Text { bounds, .. } | PaintCommand::Image { bounds, .. } => Some(*bounds),
        PaintCommand::PushClip(_) | PaintCommand::PopClip => None,
    }
}

fn physical_rect(rect: Rect, scale: f32) -> Rect {
    Rect::new(
        rect.origin.x * scale,
        rect.origin.y * scale,
        rect.size.width * scale,
        rect.size.height * scale,
    )
}

fn frect(rect: Rect) -> FRect {
    FRect::new(
        rect.origin.x,
        rect.origin.y,
        rect.size.width.max(0.0),
        rect.size.height.max(0.0),
    )
}

fn sdl_rect(rect: Rect) -> SdlRect {
    let left = rect.origin.x.floor() as i32;
    let top = rect.origin.y.floor() as i32;
    let right = (rect.origin.x + rect.size.width).ceil() as i32;
    let bottom = (rect.origin.y + rect.size.height).ceil() as i32;
    SdlRect::new(
        left,
        top,
        right.saturating_sub(left) as u32,
        bottom.saturating_sub(top) as u32,
    )
}

fn sdl_color(color: Color) -> SdlColor {
    sdl_pixel(pixel(color))
}

fn sdl_pixel(color: Pixel) -> SdlColor {
    SdlColor::RGBA(color.r, color.g, color.b, color.a)
}

fn intersection(left: Rect, right: Rect) -> Option<Rect> {
    let x = left.origin.x.max(right.origin.x);
    let y = left.origin.y.max(right.origin.y);
    let right_edge = (left.origin.x + left.size.width).min(right.origin.x + right.size.width);
    let bottom_edge = (left.origin.y + left.size.height).min(right.origin.y + right.size.height);
    (right_edge > x && bottom_edge > y).then(|| Rect::new(x, y, right_edge - x, bottom_edge - y))
}

fn union_rect(left: Rect, right: Rect) -> Rect {
    let x = left.origin.x.min(right.origin.x);
    let y = left.origin.y.min(right.origin.y);
    let right_edge = (left.origin.x + left.size.width).max(right.origin.x + right.size.width);
    let bottom_edge = (left.origin.y + left.size.height).max(right.origin.y + right.size.height);
    Rect::new(x, y, right_edge - x, bottom_edge - y)
}

fn lerp_color(start: Color, end: Color, progress: f32) -> Pixel {
    let start = pixel(start);
    let end = pixel(end);
    let channel =
        |start: u8, end: u8| (start as f32 + (end as f32 - start as f32) * progress).round() as u8;
    Pixel::rgba(
        channel(start.r, end.r),
        channel(start.g, end.g),
        channel(start.b, end.b),
        channel(start.a, end.a),
    )
}

fn pixel(color: Color) -> Pixel {
    let encoded_alpha = ((color >> 24) & 0xff) as u8;
    Pixel::rgba(
        ((color >> 16) & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        (color & 0xff) as u8,
        if color <= 0x00ff_ffff {
            255
        } else {
            encoded_alpha
        },
    )
}

fn text_size(scale: f32) -> f32 {
    match scale.round() as i32 {
        0 | 1 => 12.0,
        2 => 16.0,
        3 => 22.0,
        _ => 30.0,
    }
}

fn px(value: u32) -> u32 {
    value
}
use std::{collections::HashMap, sync::Arc};
