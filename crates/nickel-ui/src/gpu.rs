use cosmic_text::{
    Align, Attrs, Buffer, CacheKey, Color as TextColor, Family, FontSystem, Metrics, PhysicalGlyph,
    Shaping, Style as FontStyle, SwashCache, SwashContent, Weight, Wrap,
};
use sdl3::{
    pixels::{Color as SdlColor, PixelFormat},
    rect::Rect as SdlRect,
    render::{BlendMode, FRect, ScaleMode, Texture, WindowCanvas},
    surface::{Surface, SurfaceRef},
    video::Window,
};

use crate::{Color, GradientAxis, PaintCommand, Rect, StyledTextSpan, TextAlign};

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
    upload: Option<Surface<'static>>,
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
    wrap: bool,
    color: Color,
}

type CachedGlyphPixel = (i32, i32, u32, u32, TextColor);
type PhysicalGlyphs = Vec<(PhysicalGlyph, TextColor)>;
type StrikeLines = Vec<(Rect, Color)>;
type SpanBackgrounds = Vec<(Rect, Color)>;

/// Transitional name retained while callers move from the old wgpu backend.
pub type ComponentGpu = SdlComponentRenderer;

pub struct SdlCanvasPresenter {
    canvas: WindowCanvas,
    font_system: FontSystem,
    swash_cache: SwashCache,
    glyph_atlas: GlyphAtlas,
    image_textures: HashMap<u16, CachedImageTexture>,
    text_layouts: HashMap<PhysicalTextKey, Arc<CachedPhysicalText>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PhysicalTextKey {
    text: String,
    spans: Vec<StyledTextSpan>,
    width: u32,
    height: u32,
    size: u32,
    color: Color,
    align: u8,
    bold: bool,
    wrap: bool,
}

struct CachedPhysicalText {
    glyphs: PhysicalGlyphs,
    strikes: StrikeLines,
    backgrounds: SpanBackgrounds,
}

struct CachedImageTexture {
    identity: usize,
    texture: Texture,
}

const GLYPH_ATLAS_SIZE: u32 = 1024;
const TEXT_LAYOUT_CACHE_CAPACITY: usize = 2048;

struct GlyphAtlas {
    texture: Texture,
    allocator: ShelfAllocator,
    entries: HashMap<CacheKey, GlyphAtlasEntry>,
}

#[derive(Clone, Copy)]
struct GlyphAtlasEntry {
    source: SdlRect,
    left: i32,
    top: i32,
    colored: bool,
}

#[derive(Clone, Debug)]
struct ShelfAllocator {
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    row_height: u32,
    resets: u64,
}

impl ShelfAllocator {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            x: 1,
            y: 1,
            row_height: 0,
            resets: 0,
        }
    }

    fn allocate(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        let padded_width = width.checked_add(2)?;
        let padded_height = height.checked_add(2)?;
        if padded_width > self.width || padded_height > self.height {
            return None;
        }
        if self.x + padded_width > self.width {
            self.x = 1;
            self.y = self.y.saturating_add(self.row_height);
            self.row_height = 0;
        }
        if self.y + padded_height > self.height {
            return None;
        }
        let position = (self.x + 1, self.y + 1);
        self.x += padded_width;
        self.row_height = self.row_height.max(padded_height);
        Some(position)
    }

    fn reset(&mut self) {
        self.x = 1;
        self.y = 1;
        self.row_height = 0;
        self.resets = self.resets.saturating_add(1);
    }
}

impl GlyphAtlas {
    fn new(canvas: &WindowCanvas) -> Result<Self, String> {
        let creator = canvas.texture_creator();
        let mut texture = creator
            .create_texture_streaming(PixelFormat::ABGR8888, GLYPH_ATLAS_SIZE, GLYPH_ATLAS_SIZE)
            .map_err(|error| error.to_string())?;
        texture.set_blend_mode(BlendMode::Blend);
        texture.set_scale_mode(ScaleMode::Nearest);
        texture
            .update(
                None,
                &vec![0; (GLYPH_ATLAS_SIZE * GLYPH_ATLAS_SIZE * 4) as usize],
                (GLYPH_ATLAS_SIZE * 4) as usize,
            )
            .map_err(|error| error.to_string())?;
        Ok(Self {
            texture,
            allocator: ShelfAllocator::new(GLYPH_ATLAS_SIZE, GLYPH_ATLAS_SIZE),
            entries: HashMap::new(),
        })
    }

    fn clear(&mut self) -> Result<(), String> {
        self.entries.clear();
        self.allocator.reset();
        self.texture
            .update(
                None,
                &vec![0; (GLYPH_ATLAS_SIZE * GLYPH_ATLAS_SIZE * 4) as usize],
                (GLYPH_ATLAS_SIZE * 4) as usize,
            )
            .map_err(|error| error.to_string())
    }

    fn entry(
        &mut self,
        font_system: &mut FontSystem,
        swash_cache: &mut SwashCache,
        key: CacheKey,
    ) -> Result<Option<GlyphAtlasEntry>, String> {
        if let Some(entry) = self.entries.get(&key) {
            return Ok(Some(*entry));
        }
        let Some(image) = swash_cache.get_image(font_system, key).clone() else {
            return Ok(None);
        };
        let width = image.placement.width;
        let height = image.placement.height;
        if width == 0 || height == 0 {
            return Ok(None);
        }
        let position = match self.allocator.allocate(width, height) {
            Some(position) => position,
            None => {
                self.clear()?;
                let Some(position) = self.allocator.allocate(width, height) else {
                    return Ok(None);
                };
                position
            }
        };
        let colored = image.content == SwashContent::Color;
        let pixels = match image.content {
            SwashContent::Mask => image
                .data
                .iter()
                .flat_map(|alpha| [255, 255, 255, *alpha])
                .collect::<Vec<_>>(),
            SwashContent::Color => image.data,
            SwashContent::SubpixelMask => image
                .data
                .chunks_exact(4)
                .flat_map(|channels| {
                    let alpha = channels[0].max(channels[1]).max(channels[2]);
                    [255, 255, 255, alpha]
                })
                .collect(),
        };
        let source = SdlRect::new(position.0 as i32, position.1 as i32, width, height);
        self.texture
            .update(Some(source), &pixels, (width * 4) as usize)
            .map_err(|error| error.to_string())?;
        let entry = GlyphAtlasEntry {
            source,
            left: image.placement.left,
            top: image.placement.top,
            colored,
        };
        self.entries.insert(key, entry);
        Ok(Some(entry))
    }
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
        let glyph_atlas = GlyphAtlas::new(&canvas)?;
        Ok(Self {
            canvas,
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            glyph_atlas,
            image_textures: HashMap::new(),
            text_layouts: HashMap::new(),
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
        if !command_intersects_clip(command, scale, clip) {
            return Ok(());
        }
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
                wrap,
            } => self.direct_text(
                physical_rect(*bounds, scale),
                text,
                text_size(*text_scale) * scale,
                *color,
                *align,
                *bold,
                *wrap,
                clip,
            )?,
            PaintCommand::StyledText {
                bounds,
                text,
                spans,
                scale: text_scale,
                color,
                align,
            } => self.direct_styled_text(
                physical_rect(*bounds, scale),
                text,
                spans,
                text_size(*text_scale) * scale,
                *color,
                *align,
                clip,
            )?,
            PaintCommand::Image {
                bounds,
                id,
                image,
                high_density,
            } => {
                let image = high_density
                    .as_ref()
                    .filter(|_| scale >= 1.5)
                    .unwrap_or(image);
                self.direct_image(
                    physical_rect(*bounds, scale),
                    *id,
                    Arc::as_ptr(image) as usize,
                    image,
                )?
            }
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
        wrap: bool,
        parent_clip: Rect,
    ) -> Result<(), String> {
        let key = physical_text_key(text, &[], bounds, size, color, align, bold, wrap);
        let layout = if let Some(layout) = self.text_layouts.get(&key) {
            Arc::clone(layout)
        } else {
            let relative_bounds = Rect::new(0.0, 0.0, bounds.size.width, bounds.size.height);
            let layout = Arc::new(CachedPhysicalText {
                glyphs: shape_physical_glyphs(
                    &mut self.font_system,
                    text,
                    relative_bounds,
                    size,
                    color,
                    align,
                    bold,
                    wrap,
                ),
                strikes: Vec::new(),
                backgrounds: Vec::new(),
            });
            insert_bounded_text_layout(&mut self.text_layouts, key, Arc::clone(&layout));
            layout
        };
        self.canvas.set_clip_rect(Some(sdl_rect(parent_clip)));
        for (glyph, glyph_color) in &layout.glyphs {
            let Some(entry) = self.glyph_atlas.entry(
                &mut self.font_system,
                &mut self.swash_cache,
                glyph.cache_key,
            )?
            else {
                continue;
            };
            let destination = Rect::new(
                bounds.origin.x + (glyph.x + entry.left) as f32,
                bounds.origin.y + (glyph.y - entry.top) as f32,
                entry.source.width() as f32,
                entry.source.height() as f32,
            );
            if intersection(destination, parent_clip).is_none() {
                continue;
            }
            if entry.colored {
                self.glyph_atlas.texture.set_color_mod(255, 255, 255);
            } else {
                self.glyph_atlas.texture.set_color_mod(
                    glyph_color.r(),
                    glyph_color.g(),
                    glyph_color.b(),
                );
            }
            self.glyph_atlas.texture.set_alpha_mod(pixel(color).a);
            self.canvas
                .copy(
                    &self.glyph_atlas.texture,
                    entry.source,
                    FRect::new(
                        destination.origin.x,
                        destination.origin.y,
                        destination.size.width,
                        destination.size.height,
                    ),
                )
                .map_err(|error| error.to_string())?;
        }
        self.canvas.set_clip_rect(Some(sdl_rect(parent_clip)));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn direct_styled_text(
        &mut self,
        bounds: Rect,
        text: &str,
        spans: &[StyledTextSpan],
        size: f32,
        color: Color,
        align: TextAlign,
        parent_clip: Rect,
    ) -> Result<(), String> {
        let key = physical_text_key(text, spans, bounds, size, color, align, false, true);
        let layout = if let Some(layout) = self.text_layouts.get(&key) {
            Arc::clone(layout)
        } else {
            let relative_bounds = Rect::new(0.0, 0.0, bounds.size.width, bounds.size.height);
            let (glyphs, strikes, backgrounds) = shape_styled_physical_glyphs(
                &mut self.font_system,
                text,
                spans,
                relative_bounds,
                size,
                color,
                align,
            );
            let layout = Arc::new(CachedPhysicalText {
                glyphs,
                strikes,
                backgrounds,
            });
            insert_bounded_text_layout(&mut self.text_layouts, key, Arc::clone(&layout));
            layout
        };
        self.canvas.set_clip_rect(Some(sdl_rect(parent_clip)));
        for (rect, background) in &layout.backgrounds {
            self.direct_fill(
                Rect::new(
                    bounds.origin.x + rect.origin.x,
                    bounds.origin.y + rect.origin.y,
                    rect.size.width,
                    rect.size.height,
                ),
                *background,
            )?;
        }
        for (glyph, glyph_color) in &layout.glyphs {
            let Some(entry) = self.glyph_atlas.entry(
                &mut self.font_system,
                &mut self.swash_cache,
                glyph.cache_key,
            )?
            else {
                continue;
            };
            let destination = Rect::new(
                bounds.origin.x + (glyph.x + entry.left) as f32,
                bounds.origin.y + (glyph.y - entry.top) as f32,
                entry.source.width() as f32,
                entry.source.height() as f32,
            );
            if intersection(destination, parent_clip).is_none() {
                continue;
            }
            if entry.colored {
                self.glyph_atlas.texture.set_color_mod(255, 255, 255);
            } else {
                self.glyph_atlas.texture.set_color_mod(
                    glyph_color.r(),
                    glyph_color.g(),
                    glyph_color.b(),
                );
            }
            self.glyph_atlas.texture.set_alpha_mod(glyph_color.a());
            self.canvas
                .copy(
                    &self.glyph_atlas.texture,
                    entry.source,
                    FRect::new(
                        destination.origin.x,
                        destination.origin.y,
                        destination.size.width,
                        destination.size.height,
                    ),
                )
                .map_err(|error| error.to_string())?;
        }
        for (rect, strike_color) in &layout.strikes {
            self.direct_fill(
                Rect::new(
                    bounds.origin.x + rect.origin.x,
                    bounds.origin.y + rect.origin.y,
                    rect.size.width,
                    rect.size.height,
                ),
                *strike_color,
            )?;
        }
        self.canvas.set_clip_rect(Some(sdl_rect(parent_clip)));
        Ok(())
    }

    fn direct_image(
        &mut self,
        bounds: Rect,
        id: u16,
        identity: usize,
        image: &image::RgbaImage,
    ) -> Result<(), String> {
        if !self.image_textures.contains_key(&id) {
            if self.image_textures.len() >= 512 {
                for (_, cached) in self.image_textures.drain() {
                    // SAFETY: every texture was created by `self.canvas`, which
                    // remains alive for the duration of this presenter.
                    unsafe { cached.texture.destroy() };
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
            self.image_textures
                .insert(id, CachedImageTexture { identity, texture });
        } else if self
            .image_textures
            .get(&id)
            .is_some_and(|cached| cached.identity != identity)
        {
            let cached = self
                .image_textures
                .get_mut(&id)
                .expect("cached image texture");
            cached
                .texture
                .update(None, image.as_raw(), image.width().max(1) as usize * 4)
                .map_err(|error| error.to_string())?;
            cached.identity = identity;
        }
        if image.width() == 0 || image.height() == 0 {
            return Ok(());
        }
        let destination = FRect::new(
            bounds.origin.x,
            bounds.origin.y,
            bounds.size.width,
            bounds.size.height,
        );
        self.canvas
            .copy(
                &self
                    .image_textures
                    .get(&id)
                    .expect("inserted accelerated image texture")
                    .texture,
                None,
                destination,
            )
            .map_err(|error| error.to_string())
    }
}

impl SdlComponentRenderer {
    pub fn new(width: u32, height: u32, scale: f32) -> Self {
        Self::with_sdl_upload(width, height, scale, true)
    }

    /// Create a rasterizer for a presenter that reads [`Self::pixels`]
    /// directly and therefore does not need an SDL upload surface.
    pub fn new_pixel_buffer(width: u32, height: u32, scale: f32) -> Self {
        Self::with_sdl_upload(width, height, scale, false)
    }

    fn with_sdl_upload(width: u32, height: u32, scale: f32, upload: bool) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let upload = upload.then(|| Self::create_upload_surface(width, height));
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

    fn create_upload_surface(width: u32, height: u32) -> Surface<'static> {
        let mut upload =
            Surface::new(width, height, PixelFormat::ABGR8888).expect("create SDL upload surface");
        upload
            .set_blend_mode(BlendMode::None)
            .expect("disable SDL upload blending");
        upload
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
            if self.upload.is_some() {
                self.upload = Some(Self::create_upload_surface(self.width, self.height));
            }
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
        let upload = self
            .upload
            .get_or_insert_with(|| Self::create_upload_surface(self.width, self.height));
        upload.with_lock_mut(|bytes| {
            for (color, pixel) in self.pixels.iter().zip(bytes.chunks_exact_mut(4)) {
                pixel.copy_from_slice(&[color.r, color.g, color.b, color.a]);
            }
        });
        upload
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
            PaintCommand::StyledText { .. } => self.styled_text(command, clip),
            PaintCommand::Image {
                bounds,
                image,
                high_density,
                ..
            } => {
                let image = high_density
                    .as_ref()
                    .filter(|_| self.scale >= 1.5)
                    .unwrap_or(image);
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
            wrap,
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
            wrap: *wrap,
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
            buffer.set_wrap(if *wrap { Wrap::WordOrGlyph } else { Wrap::None });
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

    fn styled_text(&mut self, command: &PaintCommand, clip: Rect) {
        let PaintCommand::StyledText {
            bounds,
            text,
            spans,
            scale,
            color,
            align,
        } = command
        else {
            return;
        };
        let font_size = text_size(*scale) * self.scale;
        let physical = physical_rect(*bounds, self.scale);
        let mut buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(font_size, font_size * 1.3),
        );
        buffer.set_wrap(Wrap::WordOrGlyph);
        buffer.set_size(
            Some(physical.size.width.max(1.0)),
            Some(physical.size.height.max(font_size * 1.4)),
        );
        let defaults = rich_attrs(*color, None, 0);
        buffer.set_rich_text(
            rich_segments(text, spans, *color),
            &defaults,
            Shaping::Advanced,
            Some(cosmic_align(*align)),
        );
        buffer.shape_until_scroll(&mut self.font_system, false);
        let mut pixels = Vec::new();
        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            text_color(*color),
            |x, y, width, height, glyph_color| {
                pixels.push((x, y, width, height, glyph_color));
            },
        );
        let strikes = styled_strikes(&buffer, spans, physical, *color, font_size);
        let origin = crate::Point {
            x: physical.origin.x.round(),
            y: physical.origin.y.round(),
        };
        for (x, y, width, height, glyph_color) in pixels {
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
        for (rect, strike_color) in strikes {
            self.fill_round(rect, 0.0, 0, strike_color, clip);
        }
    }

    fn image(&mut self, rect: Rect, image: &image::RgbaImage, clip: Rect) {
        if image.width() == 0 || image.height() == 0 {
            return;
        }
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
        PaintCommand::Text { bounds, .. }
        | PaintCommand::StyledText { bounds, .. }
        | PaintCommand::Image { bounds, .. } => Some(*bounds),
        PaintCommand::PushClip(_) | PaintCommand::PopClip => None,
    }
}

fn command_intersects_clip(command: &PaintCommand, scale: f32, clip: Rect) -> bool {
    command_bounds(command)
        .map(|bounds| physical_rect(bounds, scale))
        .is_none_or(|bounds| intersection(bounds, clip).is_some())
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

#[allow(clippy::too_many_arguments)]
fn physical_text_key(
    text: &str,
    spans: &[StyledTextSpan],
    bounds: Rect,
    size: f32,
    color: Color,
    align: TextAlign,
    bold: bool,
    wrap: bool,
) -> PhysicalTextKey {
    PhysicalTextKey {
        text: text.to_owned(),
        spans: spans.to_vec(),
        width: bounds.size.width.to_bits(),
        height: bounds.size.height.to_bits(),
        size: size.to_bits(),
        color,
        align: match align {
            TextAlign::Start => 0,
            TextAlign::Center => 1,
            TextAlign::End => 2,
        },
        bold,
        wrap,
    }
}

fn insert_bounded_text_layout(
    cache: &mut HashMap<PhysicalTextKey, Arc<CachedPhysicalText>>,
    key: PhysicalTextKey,
    layout: Arc<CachedPhysicalText>,
) {
    if cache.len() >= TEXT_LAYOUT_CACHE_CAPACITY {
        cache.clear();
    }
    cache.insert(key, layout);
}

#[allow(clippy::too_many_arguments)]
fn shape_physical_glyphs(
    font_system: &mut FontSystem,
    text: &str,
    bounds: Rect,
    size: f32,
    color: Color,
    align: TextAlign,
    bold: bool,
    wrap: bool,
) -> Vec<(PhysicalGlyph, TextColor)> {
    let mut buffer = Buffer::new(font_system, Metrics::new(size, size * 1.3));
    buffer.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
    buffer.set_size(
        Some(bounds.size.width.max(1.0)),
        Some(bounds.size.height.max(size * 1.3)),
    );
    let mut attrs = Attrs::new().family(Family::SansSerif);
    if bold {
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
    buffer.shape_until_scroll(font_system, false);
    let base = pixel(color);
    buffer
        .layout_runs()
        .flat_map(|run| {
            run.glyphs.iter().map(move |glyph| {
                (
                    glyph.physical((bounds.origin.x, bounds.origin.y + run.line_y), 1.0),
                    glyph
                        .color_opt
                        .unwrap_or(TextColor::rgb(base.r, base.g, base.b)),
                )
            })
        })
        .collect()
}

fn text_color(color: Color) -> TextColor {
    let pixel = pixel(color);
    TextColor::rgba(pixel.r, pixel.g, pixel.b, pixel.a)
}

fn cosmic_align(align: TextAlign) -> Align {
    match align {
        TextAlign::Start => Align::Left,
        TextAlign::Center => Align::Center,
        TextAlign::End => Align::Right,
    }
}

fn rich_attrs(color: Color, span: Option<&StyledTextSpan>, metadata: usize) -> Attrs<'static> {
    let mut attrs = Attrs::new()
        .family(if span.is_some_and(|span| span.monospace) {
            Family::Monospace
        } else {
            Family::SansSerif
        })
        .color(text_color(
            span.and_then(|span| span.color).unwrap_or(color),
        ))
        .metadata(metadata);
    if span.is_some_and(|span| span.bold) {
        attrs = attrs.weight(Weight::BOLD);
    }
    if span.is_some_and(|span| span.italic) {
        attrs = attrs.style(FontStyle::Italic);
    }
    attrs
}

fn rich_segments<'a>(
    text: &'a str,
    spans: &[StyledTextSpan],
    color: Color,
) -> Vec<(&'a str, Attrs<'static>)> {
    let mut segments = Vec::new();
    let mut cursor = 0;
    for (index, span) in spans.iter().enumerate() {
        let start = span.range.start.min(text.len());
        let end = span.range.end.min(text.len());
        if start > cursor && text.is_char_boundary(cursor) && text.is_char_boundary(start) {
            segments.push((&text[cursor..start], rich_attrs(color, None, 0)));
        }
        if end > start && text.is_char_boundary(start) && text.is_char_boundary(end) {
            segments.push((&text[start..end], rich_attrs(color, Some(span), index + 1)));
            cursor = end;
        }
    }
    if cursor < text.len() && text.is_char_boundary(cursor) {
        segments.push((&text[cursor..], rich_attrs(color, None, 0)));
    }
    if segments.is_empty() {
        segments.push((text, rich_attrs(color, None, 0)));
    }
    segments
}

fn styled_strikes(
    buffer: &Buffer,
    spans: &[StyledTextSpan],
    bounds: Rect,
    default_color: Color,
    font_size: f32,
) -> Vec<(Rect, Color)> {
    let mut strikes = Vec::new();
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let Some(span) = glyph
                .metadata
                .checked_sub(1)
                .and_then(|index| spans.get(index))
            else {
                continue;
            };
            if !span.strikethrough {
                continue;
            }
            let rect = Rect::new(
                bounds.origin.x + glyph.x,
                bounds.origin.y + run.line_top + run.line_height * 0.52,
                glyph.w.max(1.0),
                (font_size / 14.0).max(1.0),
            );
            strikes.push((rect, span.color.unwrap_or(default_color)));
        }
    }
    strikes
}

fn styled_backgrounds(
    buffer: &Buffer,
    spans: &[StyledTextSpan],
    bounds: Rect,
    font_size: f32,
) -> SpanBackgrounds {
    let mut backgrounds = Vec::new();
    let padding = (font_size * 0.12).max(1.0);
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let Some(background) = glyph
                .metadata
                .checked_sub(1)
                .and_then(|index| spans.get(index))
                .and_then(|span| span.background)
            else {
                continue;
            };
            backgrounds.push((
                Rect::new(
                    bounds.origin.x + glyph.x - padding,
                    bounds.origin.y + run.line_top,
                    glyph.w + padding * 2.0,
                    run.line_height,
                ),
                background,
            ));
        }
    }
    backgrounds
}

fn shape_styled_physical_glyphs(
    font_system: &mut FontSystem,
    text: &str,
    spans: &[StyledTextSpan],
    bounds: Rect,
    size: f32,
    color: Color,
    align: TextAlign,
) -> (PhysicalGlyphs, StrikeLines, SpanBackgrounds) {
    let mut buffer = Buffer::new(font_system, Metrics::new(size, size * 1.3));
    buffer.set_wrap(Wrap::WordOrGlyph);
    buffer.set_size(
        Some(bounds.size.width.max(1.0)),
        Some(bounds.size.height.max(size * 1.3)),
    );
    let defaults = rich_attrs(color, None, 0);
    buffer.set_rich_text(
        rich_segments(text, spans, color),
        &defaults,
        Shaping::Advanced,
        Some(cosmic_align(align)),
    );
    buffer.shape_until_scroll(font_system, false);
    let glyphs = buffer
        .layout_runs()
        .flat_map(|run| {
            run.glyphs.iter().map(move |glyph| {
                (
                    glyph.physical((bounds.origin.x, bounds.origin.y + run.line_y), 1.0),
                    glyph.color_opt.unwrap_or(text_color(color)),
                )
            })
        })
        .collect();
    let strikes = styled_strikes(&buffer, spans, bounds, color, size);
    let backgrounds = styled_backgrounds(&buffer, spans, bounds, size);
    (glyphs, strikes, backgrounds)
}

fn px(value: u32) -> u32 {
    value
}
use std::{collections::HashMap, sync::Arc};

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use cosmic_text::FontSystem;

    use super::{
        PaintCommand, Rect, ShelfAllocator, TextAlign, command_intersects_clip, physical_text_key,
        shape_physical_glyphs,
    };

    #[test]
    fn physical_text_layout_identity_ignores_scroll_translation() {
        let first = physical_text_key(
            "Retained text",
            &[],
            Rect::new(0.0, 10.0, 240.0, 40.0),
            16.0,
            0xffffff,
            TextAlign::Start,
            false,
            true,
        );
        let scrolled = physical_text_key(
            "Retained text",
            &[],
            Rect::new(0.0, -74.0, 240.0, 40.0),
            16.0,
            0xffffff,
            TextAlign::Start,
            false,
            true,
        );

        assert_eq!(first, scrolled);
    }

    #[test]
    fn bounded_commands_are_rejected_before_rendering_outside_physical_clip() {
        let clip = Rect::new(0.0, 0.0, 100.0, 100.0);
        let text = PaintCommand::Text {
            bounds: Rect::new(60.0, 60.0, 20.0, 10.0),
            text: "offscreen".into(),
            scale: 1.0,
            color: 0xffffff,
            align: TextAlign::Start,
            bold: false,
            wrap: true,
        };
        assert!(!command_intersects_clip(&text, 2.0, clip));
        assert!(command_intersects_clip(
            &PaintCommand::PushClip(clip),
            2.0,
            clip
        ));
    }

    #[test]
    fn glyph_shelves_are_bounded_padded_and_deterministic() {
        let mut allocator = ShelfAllocator::new(16, 16);
        assert_eq!(allocator.allocate(3, 4), Some((2, 2)));
        assert_eq!(allocator.allocate(3, 4), Some((7, 2)));
        assert_eq!(allocator.allocate(6, 4), Some((2, 8)));
        assert_eq!(allocator.allocate(20, 1), None);
        assert_eq!(allocator.allocate(6, 6), None);
    }

    #[test]
    fn glyph_shelf_reset_restarts_allocation_and_records_eviction() {
        let mut allocator = ShelfAllocator::new(16, 16);
        assert_eq!(allocator.allocate(8, 8), Some((2, 2)));
        assert!(allocator.allocate(8, 8).is_none());
        allocator.reset();
        assert_eq!(allocator.allocate(8, 8), Some((2, 2)));
        assert_eq!(allocator.resets, 1);
    }

    #[test]
    fn repeated_text_reuses_physical_glyph_keys() {
        let mut fonts = FontSystem::new();
        let glyphs = shape_physical_glyphs(
            &mut fonts,
            "reuse reuse",
            Rect::new(0.0, 0.0, 300.0, 40.0),
            16.0,
            0xffffff,
            TextAlign::Start,
            false,
            true,
        );
        let unique = glyphs
            .iter()
            .map(|(glyph, _)| glyph.cache_key)
            .collect::<HashSet<_>>();
        assert!(unique.len() < glyphs.len());
    }

    #[test]
    fn fractional_origins_and_display_scales_produce_integer_glyph_destinations() {
        let mut fonts = FontSystem::new();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let glyphs = shape_physical_glyphs(
                &mut fonts,
                "Café 👋🏽\nsecond line",
                Rect::new(13.3 * scale, 7.7 * scale, 360.0 * scale, 80.0 * scale),
                16.0 * scale,
                0xffffff,
                TextAlign::Center,
                false,
                true,
            );
            assert!(!glyphs.is_empty());
            assert!(glyphs.iter().all(|(glyph, _)| {
                glyph.x as f32 == (glyph.x as f32).round()
                    && glyph.y as f32 == (glyph.y as f32).round()
            }));
        }
    }
}
