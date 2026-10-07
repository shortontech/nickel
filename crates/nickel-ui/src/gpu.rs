use cosmic_text::{
    Align, Attrs, Buffer, Color as TextColor, Family, Metrics, Shaping, Style as FontStyle,
    SwashCache, Weight, Wrap,
};
use nickel_render_assets::ProcessFontSystem;
use smallvec::{SmallVec, smallvec};
use std::{collections::VecDeque, sync::Arc};

#[cfg(debug_assertions)]
mod image_profile {
    use std::{
        sync::{Mutex, OnceLock},
        time::{Duration, Instant},
    };

    #[derive(Default)]
    struct Totals {
        fingerprints: u64,
        fingerprint_bytes: u64,
        fingerprint_time: Duration,
        rasters: u64,
        raster_pixels: u64,
        raster_time: Duration,
    }

    struct State {
        started: Instant,
        totals: Totals,
    }

    fn state() -> Option<&'static Mutex<State>> {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        static STATE: OnceLock<Mutex<State>> = OnceLock::new();
        ENABLED
            .get_or_init(|| std::env::var_os("NICKEL_FILE_PROFILE_ICONS").is_some())
            .then(|| {
                STATE.get_or_init(|| {
                    Mutex::new(State {
                        started: Instant::now(),
                        totals: Totals::default(),
                    })
                })
            })
    }

    fn report(state: &mut State) {
        if state.started.elapsed() < Duration::from_secs(1) {
            return;
        }
        eprintln!(
            "nickel-file icon-profile: fingerprint calls={} bytes={} time={:.2?}; raster calls={} pixels={} time={:.2?}",
            state.totals.fingerprints,
            state.totals.fingerprint_bytes,
            state.totals.fingerprint_time,
            state.totals.rasters,
            state.totals.raster_pixels,
            state.totals.raster_time,
        );
        state.started = Instant::now();
        state.totals = Totals::default();
    }

    pub(super) fn fingerprint(bytes: usize, elapsed: Duration) {
        let Some(state) = state() else { return };
        let mut state = state.lock().expect("icon profile lock");
        state.totals.fingerprints += 1;
        state.totals.fingerprint_bytes =
            state.totals.fingerprint_bytes.saturating_add(bytes as u64);
        state.totals.fingerprint_time += elapsed;
        report(&mut state);
    }

    pub(super) fn raster(pixels: u64, elapsed: Duration) {
        let Some(state) = state() else { return };
        let mut state = state.lock().expect("icon profile lock");
        state.totals.rasters += 1;
        state.totals.raster_pixels = state.totals.raster_pixels.saturating_add(pixels);
        state.totals.raster_time += elapsed;
        report(&mut state);
    }
}

#[cfg(debug_assertions)]
pub(crate) fn record_image_fingerprint(bytes: usize, elapsed: std::time::Duration) {
    image_profile::fingerprint(bytes, elapsed);
}

use crate::{
    Color, GradientAxis, PaintCommand, Rect, StyledTextSpan, TextAlign, TextUnderlineStyle,
};

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
    pub rects: SmallVec<[Rect; 1]>,
}

impl DamageRegion {
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }
}

/// Runtime-neutral software rasterizer for the platform-neutral component tree.
pub struct SoftwareRenderer {
    width: u32,
    height: u32,
    scale: f32,
    pixels: Vec<Pixel>,
    previous_commands: Vec<PaintCommand>,
    presented_generation: Option<u64>,
    framebuffer_valid: bool,
    clips: Vec<Rect>,
    text_rasters: Vec<Option<CachedSoftwareText>>,
    rejected_text_rasters: Vec<bool>,
    image_rasters: VecDeque<CachedSoftwareImage>,
    image_raster_bytes: usize,
    font_system: ProcessFontSystem,
    swash_cache: Option<SwashCache>,
    glyph_bytes: usize,
    glyph_generation_misses: usize,
    raster_stats: SoftwareRasterDiagnostics,
}
type SoftwareGlyphPixels = Vec<(i32, i32, TextColor)>;
const SOFTWARE_TEXT_RASTER_BUDGET: usize = 2 * 1024 * 1024;
const SOFTWARE_GLYPH_BYTE_BUDGET: usize = 2 * 1024 * 1024;
const SOFTWARE_GLYPH_ENTRY_BUDGET: usize = 2048;
const SOFTWARE_IMAGE_RASTER_BUDGET: usize = 2 * 1024 * 1024;
const SOFTWARE_IMAGE_SOURCE_LIMIT: usize = 128 * 1024;
const SOFTWARE_IMAGE_SIDE_LIMIT: u32 = 128;

struct CachedSoftwareImage {
    source: Arc<image::RgbaImage>,
    rect_width: u32,
    rect_height: u32,
    fraction_x: u32,
    fraction_y: u32,
    width: u32,
    pixels: Arc<[Pixel]>,
}

impl CachedSoftwareImage {
    fn retained_bytes(&self) -> usize {
        self.source
            .as_raw()
            .len()
            .saturating_add(self.pixels.len() * std::mem::size_of::<Pixel>())
    }
}

struct SampleDecorations<'a> {
    renderer: &'a mut SoftwareRenderer,
    candidate: &'a mut Option<SoftwareGlyphPixels>,
    available: usize,
    physical: Rect,
    clip: Rect,
}

impl cosmic_text::Renderer for SampleDecorations<'_> {
    fn glyph(&mut self, _: cosmic_text::PhysicalGlyph, _: TextColor) {
        unreachable!("decoration rendering emits rectangles only");
    }

    fn rectangle(&mut self, x: i32, y: i32, width: u32, height: u32, color: TextColor) {
        for row in 0..height {
            for column in 0..width {
                self.renderer.text_sample(
                    self.candidate,
                    self.available,
                    self.physical,
                    self.clip,
                    x + column as i32,
                    y + row as i32,
                    color,
                );
            }
        }
    }
}

/// Known allocation capacities only; Swash's private scaler scratch and the
/// shared font system are opaque and are not estimated from source text bytes.
#[derive(Clone, Copy, Debug, Default)]
pub struct SoftwareRasterDiagnostics {
    pub framebuffer_live_bytes: usize,
    pub framebuffer_peak_bytes: usize,
    pub damage_hints_accepted: u64,
    pub damage_hints_fell_back: u64,
    pub full_repaints: u64,
    pub partial_repaints: u64,
    pub clean_frames: u64,
    pub glyph_hits: u64,
    pub glyph_misses: u64,
    pub glyph_resets: u64,
    pub glyph_rejections: u64,
    pub glyph_peak_bytes: usize,
    pub candidate_peak_bytes: usize,
    pub candidate_allocated_bytes: u64,
    pub known_cache_peak_bytes: usize,
    pub rejected_rasters: u64,
    pub image_cache_hits: u64,
    pub image_cache_misses: u64,
    pub image_cache_evictions: u64,
}

struct CachedSoftwareText {
    pixels: SoftwareGlyphPixels,
    strikes: StrikeLines,
}

impl CachedSoftwareText {
    fn retained_bytes(&self) -> usize {
        self.pixels
            .capacity()
            .saturating_mul(std::mem::size_of::<(i32, i32, TextColor)>())
            .saturating_add(
                self.strikes
                    .capacity()
                    .saturating_mul(std::mem::size_of::<(Rect, Color)>()),
            )
    }
}
type StrikeLines = Vec<(Rect, Color)>;

// Plain-text samples are stored relative to their bounds, so moving an
// otherwise identical label does not require shaping and rasterizing it again.
fn text_raster_matches(previous: &PaintCommand, current: &PaintCommand) -> bool {
    match (previous, current) {
        (
            PaintCommand::Text {
                bounds: old_bounds,
                text: old_text,
                scale: old_scale,
                color: old_color,
                align: old_align,
                bold: old_bold,
                wrap: old_wrap,
            },
            PaintCommand::Text {
                bounds: new_bounds,
                text: new_text,
                scale: new_scale,
                color: new_color,
                align: new_align,
                bold: new_bold,
                wrap: new_wrap,
            },
        ) => {
            old_bounds.size == new_bounds.size
                && old_text == new_text
                && old_scale == new_scale
                && old_color == new_color
                && old_align == new_align
                && old_bold == new_bold
                && old_wrap == new_wrap
        }
        _ => previous == current,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PresenterCacheDiagnostics {
    pub text_layouts: usize,
    pub text_layout_bytes: usize,
    pub image_textures: usize,
    pub glyphs: usize,
    pub glyph_atlas_width: u32,
    pub glyph_atlas_height: u32,
    pub glyph_atlas_bytes: usize,
    pub live_bytes: usize,
    pub peak_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub insertions: u64,
    pub evictions: u64,
    pub invalidations: u64,
    pub recomputation_nanos: u64,
}

/// Process-level accounting assembled from every live presenter. Allocator RSS
/// remains a separate operating-system measurement and is deliberately not
/// inferred from these cache-owned byte estimates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AggregatePresenterCacheDiagnostics {
    pub presenters: usize,
    pub live_entries: usize,
    pub live_bytes: usize,
    pub peak_cache_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub insertions: u64,
    pub evictions: u64,
    pub invalidations: u64,
    pub recomputation_nanos: u64,
}

impl AggregatePresenterCacheDiagnostics {
    pub fn from_presenters(
        presenters: impl IntoIterator<Item = PresenterCacheDiagnostics>,
    ) -> Self {
        presenters
            .into_iter()
            .fold(Self::default(), |mut total, item| {
                total.presenters = total.presenters.saturating_add(1);
                total.live_entries = total.live_entries.saturating_add(
                    item.text_layouts
                        .saturating_add(item.image_textures)
                        .saturating_add(item.glyphs),
                );
                total.live_bytes = total.live_bytes.saturating_add(item.live_bytes);
                total.peak_cache_bytes = total.peak_cache_bytes.saturating_add(item.peak_bytes);
                total.hits = total.hits.saturating_add(item.hits);
                total.misses = total.misses.saturating_add(item.misses);
                total.insertions = total.insertions.saturating_add(item.insertions);
                total.evictions = total.evictions.saturating_add(item.evictions);
                total.invalidations = total.invalidations.saturating_add(item.invalidations);
                total.recomputation_nanos = total
                    .recomputation_nanos
                    .saturating_add(item.recomputation_nanos);
                total
            })
    }
}

impl SoftwareRenderer {
    pub fn new(width: u32, height: u32, scale: f32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        Self {
            width,
            height,
            scale: scale.max(0.25),
            pixels: vec![Pixel::TRANSPARENT; (width * height) as usize],
            previous_commands: Vec::new(),
            presented_generation: None,
            framebuffer_valid: false,
            clips: Vec::new(),
            text_rasters: Vec::new(),
            rejected_text_rasters: Vec::new(),
            image_rasters: VecDeque::new(),
            image_raster_bytes: 0,
            font_system: ProcessFontSystem::new(),
            swash_cache: Some(SwashCache::new()),
            glyph_bytes: 0,
            glyph_generation_misses: 0,
            raster_stats: SoftwareRasterDiagnostics {
                framebuffer_peak_bytes: (width as usize)
                    .saturating_mul(height as usize)
                    .saturating_mul(std::mem::size_of::<Pixel>()),
                ..SoftwareRasterDiagnostics::default()
            },
        }
    }

    /// Alias retained for callers that explicitly describe pixel-buffer use.
    pub fn new_pixel_buffer(width: u32, height: u32, scale: f32) -> Self {
        Self::new(width, height, scale)
    }

    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        let width = width.max(1);
        let height = height.max(1);
        let scale = scale.max(0.25);
        if scale != self.scale {
            self.text_rasters.clear();
            self.rejected_text_rasters.clear();
            self.previous_commands.clear();
            self.presented_generation = None;
            self.framebuffer_valid = false;
            self.reset_glyph_cache();
        }
        self.scale = scale;
        if (width, height) != (self.width, self.height) {
            self.width = width;
            self.height = height;
            self.pixels
                .resize((self.width * self.height) as usize, Pixel::TRANSPARENT);
            // The framebuffer is invalid, but text rasters for unchanged
            // commands remain valid when the scale factor is unchanged.
            self.presented_generation = None;
            self.framebuffer_valid = false;
            self.raster_stats.framebuffer_peak_bytes = self
                .raster_stats
                .framebuffer_peak_bytes
                .max(self.pixel_capacity_bytes());
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn pixels(&self) -> &[Pixel] {
        &self.pixels
    }

    pub fn pixel_capacity_bytes(&self) -> usize {
        self.pixels
            .capacity()
            .saturating_mul(std::mem::size_of::<Pixel>())
    }

    pub fn software_raster_diagnostics(&self) -> SoftwareRasterDiagnostics {
        SoftwareRasterDiagnostics {
            framebuffer_live_bytes: self.pixel_capacity_bytes(),
            ..self.raster_stats
        }
    }

    /// Reports bounded derived data retained by the software rasterizer. The
    /// frame-sized pixel buffer is presentation storage, not a cache.
    pub fn cache_diagnostics(&self) -> PresenterCacheDiagnostics {
        let text_layouts = self.text_rasters.iter().flatten().count();
        let text_layout_bytes = self
            .text_rasters
            .iter()
            .flatten()
            .map(CachedSoftwareText::retained_bytes)
            .sum::<usize>();
        PresenterCacheDiagnostics {
            text_layouts,
            text_layout_bytes,
            image_textures: self.image_rasters.len(),
            glyphs: self
                .swash_cache
                .as_ref()
                .map_or(0, |cache| cache.image_cache.len()),
            glyph_atlas_bytes: self.glyph_bytes,
            live_bytes: text_layout_bytes
                .saturating_add(self.glyph_bytes)
                .saturating_add(self.image_raster_bytes),
            peak_bytes: self.raster_stats.known_cache_peak_bytes,
            hits: self.raster_stats.glyph_hits + self.raster_stats.image_cache_hits,
            misses: self.raster_stats.glyph_misses + self.raster_stats.image_cache_misses,
            evictions: self.raster_stats.image_cache_evictions,
            invalidations: self.raster_stats.glyph_resets,
            ..PresenterCacheDiagnostics::default()
        }
    }

    pub fn invalidate(&mut self) {
        // Framebuffer invalidation does not change command identity. Retain
        // raster hits and oversized-admission decisions across full repaints.
        self.framebuffer_valid = false;
    }

    /// Release frame-sized and derived raster resources while a presenter is
    /// hidden. The renderer remains reusable and grows to the next requested
    /// size on [`Self::resize`].
    pub fn suspend(&mut self) {
        self.width = 1;
        self.height = 1;
        self.pixels = vec![Pixel::TRANSPARENT];
        self.previous_commands = Vec::new();
        self.presented_generation = None;
        self.framebuffer_valid = false;
        self.clips = Vec::new();
        self.text_rasters = Vec::new();
        self.rejected_text_rasters = Vec::new();
        self.image_rasters.clear();
        self.image_raster_bytes = 0;
        self.reset_glyph_cache();
    }

    /// Rasterize a component display list and return its conservative damage.
    pub fn render(&mut self, commands: &[PaintCommand]) -> DamageRegion {
        self.render_with_damage_hint(commands, None)
    }

    pub(crate) fn render_frame_with_damage_hint(
        &mut self,
        commands: &[PaintCommand],
        generation: u64,
        damage_hint: Option<&[Rect]>,
    ) -> DamageRegion {
        let damage_hint = (self.presented_generation != Some(generation))
            .then_some(damage_hint)
            .flatten();
        let damage = self.render_with_damage_hint(commands, damage_hint);
        self.presented_generation = Some(generation);
        damage
    }

    fn render_with_damage_hint(
        &mut self,
        commands: &[PaintCommand],
        damage_hint: Option<&[Rect]>,
    ) -> DamageRegion {
        let hint_offered = damage_hint.is_some();
        let hint_usable = hint_offered && self.framebuffer_valid;
        if hint_usable {
            self.raster_stats.damage_hints_accepted =
                self.raster_stats.damage_hints_accepted.saturating_add(1);
        } else if hint_offered {
            self.raster_stats.damage_hints_fell_back =
                self.raster_stats.damage_hints_fell_back.saturating_add(1);
        }
        let damage = if self.framebuffer_valid {
            damage_hint.map_or_else(
                || self.damage(commands),
                |rects| DamageRegion {
                    rects: rects
                        .iter()
                        .copied()
                        .map(|rect| physical_rect(rect, self.scale))
                        .collect(),
                },
            )
        } else {
            self.damage(commands)
        };
        if damage.is_empty() {
            self.raster_stats.clean_frames = self.raster_stats.clean_frames.saturating_add(1);
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
        if repaint == full {
            self.raster_stats.full_repaints = self.raster_stats.full_repaints.saturating_add(1);
        } else {
            self.raster_stats.partial_repaints =
                self.raster_stats.partial_repaints.saturating_add(1);
        }
        self.clear(repaint);
        let mut clips = std::mem::take(&mut self.clips);
        clips.clear();
        clips.push(repaint);
        if self.text_rasters.len() < commands.len() {
            self.text_rasters.resize_with(commands.len(), || None);
        }
        self.text_rasters.truncate(commands.len());
        self.rejected_text_rasters.resize(commands.len(), false);
        for (index, command) in commands.iter().enumerate() {
            // A changed command can be skipped by clipping. Retire its old
            // raster before committing the new authoritative command identity.
            if !self
                .previous_commands
                .get(index)
                .is_some_and(|previous| text_raster_matches(previous, command))
            {
                self.text_rasters[index] = None;
                self.rejected_text_rasters[index] = false;
            }
            self.draw_command(index, command, &mut clips);
        }
        self.clips = clips;
        if self.previous_commands.len() == commands.len() {
            for (previous, command) in self.previous_commands.iter_mut().zip(commands) {
                if previous != command {
                    previous.clone_from(command);
                }
            }
        } else {
            self.previous_commands.clear();
            self.previous_commands.extend_from_slice(commands);
        }
        self.framebuffer_valid = true;
        self.raster_stats.known_cache_peak_bytes = self
            .raster_stats
            .known_cache_peak_bytes
            .max(self.cache_diagnostics().live_bytes);
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

    fn damage(&self, commands: &[PaintCommand]) -> DamageRegion {
        if !self.framebuffer_valid {
            return DamageRegion {
                rects: smallvec![Rect::new(0.0, 0.0, self.width as f32, self.height as f32)],
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

    fn draw_command(&mut self, index: usize, command: &PaintCommand, clips: &mut Vec<Rect>) {
        let clip = *clips.last().expect("clip stack always has the surface");
        match command {
            PaintCommand::BackdropBlur { .. } => {}
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
            PaintCommand::RoundedStroke {
                rect,
                color,
                width,
                radius,
            } => {
                let rect = physical_rect(*rect, self.scale);
                let width = (width * self.scale)
                    .max(0.0)
                    .min(rect.size.width / 2.0)
                    .min(rect.size.height / 2.0);
                let radius = (radius * self.scale)
                    .max(0.0)
                    .min(rect.size.width / 2.0)
                    .min(rect.size.height / 2.0);
                let source = pixel(*color);
                if width == 0.0 || source.a == 0 {
                    return;
                }
                let Some(bounds) = intersection(rect, clip) else {
                    return;
                };
                let inner = Rect::new(
                    rect.origin.x + width,
                    rect.origin.y + width,
                    rect.size.width - width * 2.0,
                    rect.size.height - width * 2.0,
                );
                // Coverage of the outer curve minus its inset leaves the center
                // transparent, without rounding fractional edges to whole pixels.
                let coverage = |rect: Rect, radius: f32, x: f32, y: f32| {
                    if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
                        return 0.0;
                    }
                    let qx = (x - rect.origin.x - rect.size.width / 2.0).abs()
                        - rect.size.width / 2.0
                        + radius;
                    let qy = (y - rect.origin.y - rect.size.height / 2.0).abs()
                        - rect.size.height / 2.0
                        + radius;
                    let distance = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
                    (0.5 - distance).clamp(0.0, 1.0)
                };
                self.for_pixels(bounds, |renderer, x, y| {
                    let outer = coverage(rect, radius, x as f32 + 0.5, y as f32 + 0.5);
                    let hole = coverage(
                        inner,
                        (radius - width).max(0.0),
                        x as f32 + 0.5,
                        y as f32 + 0.5,
                    );
                    let mut covered = source;
                    covered.a = (f32::from(source.a) * (outer - hole).max(0.0)).round() as u8;
                    if covered.a != 0 {
                        renderer.blend(x, y, covered);
                    }
                });
            }
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
            PaintCommand::Text { .. } => self.text(index, command, clip),
            PaintCommand::StyledText { .. } => self.styled_text(index, command, clip),
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
        if radius == 0.0 || corners == 0 {
            self.fill_rect(bounds, pixel(color));
            return;
        }
        let source = pixel(color);
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
            if let Some((dx, dy)) = rounded {
                let coverage = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
                if coverage == 0.0 {
                    return;
                }
                if coverage < 1.0 {
                    let mut covered = source;
                    covered.a = (f32::from(source.a) * coverage).round() as u8;
                    renderer.blend(x, y, covered);
                    return;
                }
            }
            if source.a == 255 {
                renderer.pixels[(y * renderer.width + x) as usize] = source;
            } else {
                renderer.blend(x, y, source);
            }
        });
    }

    fn fill_rect(&mut self, rect: Rect, source: Pixel) {
        let x_start = rect.origin.x.floor().max(0.0) as u32;
        let y_start = rect.origin.y.floor().max(0.0) as u32;
        let x_end = (rect.origin.x + rect.size.width)
            .ceil()
            .min(self.width as f32) as u32;
        let y_end = (rect.origin.y + rect.size.height)
            .ceil()
            .min(self.height as f32) as u32;

        if source.a == 255 {
            for y in y_start..y_end {
                let start = (y * self.width + x_start) as usize;
                let end = (y * self.width + x_end) as usize;
                self.pixels[start..end].fill(source);
            }
            return;
        }

        for y in y_start..y_end {
            for x in x_start..x_end {
                self.blend(x, y, source);
            }
        }
    }

    fn fill_gradient(&mut self, rect: Rect, gradient: crate::LinearGradient, clip: Rect) {
        let Some(bounds) = intersection(rect, clip) else {
            return;
        };
        let x_start = bounds.origin.x.floor().max(0.0) as u32;
        let y_start = bounds.origin.y.floor().max(0.0) as u32;
        let x_end = (bounds.origin.x + bounds.size.width)
            .ceil()
            .min(self.width as f32) as u32;
        let y_end = (bounds.origin.y + bounds.size.height)
            .ceil()
            .min(self.height as f32) as u32;
        match gradient.axis {
            GradientAxis::Vertical => {
                for y in y_start..y_end {
                    let progress =
                        ((y as f32 + 0.5 - rect.origin.y) / rect.size.height).clamp(0.0, 1.0);
                    let source = lerp_color(gradient.start, gradient.end, progress);
                    if source.a == 255 {
                        let start = (y * self.width + x_start) as usize;
                        let end = (y * self.width + x_end) as usize;
                        self.pixels[start..end].fill(source);
                    } else {
                        for x in x_start..x_end {
                            self.blend(x, y, source);
                        }
                    }
                }
            }
            GradientAxis::Horizontal => {
                let colors = (x_start..x_end)
                    .map(|x| {
                        let progress =
                            ((x as f32 + 0.5 - rect.origin.x) / rect.size.width).clamp(0.0, 1.0);
                        lerp_color(gradient.start, gradient.end, progress)
                    })
                    .collect::<Vec<_>>();
                for y in y_start..y_end {
                    for (x, source) in (x_start..x_end).zip(&colors) {
                        if source.a == 255 {
                            self.pixels[(y * self.width + x) as usize] = *source;
                        } else {
                            self.blend(x, y, *source);
                        }
                    }
                }
            }
        }
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

    fn text(&mut self, index: usize, command: &PaintCommand, clip: Rect) {
        let mut font_system = self.font_system.lock();
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
        let cached = self.text_rasters[index].take();
        if let Some(cached) = cached.filter(|_| {
            self.previous_commands
                .get(index)
                .is_some_and(|previous| text_raster_matches(previous, command))
        }) {
            self.draw_cached_text(&cached, physical_rect(*bounds, self.scale), clip);
            self.text_rasters[index] = Some(cached);
            return;
        }
        let font_size = text_size(*scale) * self.scale;
        let physical = physical_rect(*bounds, self.scale);
        let buffer_width = physical.size.width.max(1.0);
        let buffer_height = physical.size.height.max(font_size * 1.4);
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(font_size, font_size * 1.3));
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
        buffer.shape_until_scroll(&mut font_system, false);
        let pixel = pixel(*color);
        let text_color = TextColor::rgba(pixel.r, pixel.g, pixel.b, pixel.a);
        self.raster_text(
            index,
            &buffer,
            &mut font_system,
            text_color,
            physical,
            clip,
            Vec::new(),
        );
    }

    fn draw_cached_text(&mut self, cached: &CachedSoftwareText, physical: Rect, clip: Rect) {
        let origin = crate::Point {
            x: physical.origin.x.round(),
            y: physical.origin.y.round(),
        };
        let clip_right = clip.origin.x + clip.size.width;
        let clip_bottom = clip.origin.y + clip.size.height;
        for &(x, y, glyph_color) in &cached.pixels {
            let px = origin.x + x as f32;
            let py = origin.y + y as f32;
            if px >= 0.0
                && py >= 0.0
                && px < self.width as f32
                && py < self.height as f32
                && clip.size.width > 0.0
                && clip.size.height > 0.0
                && px + 1.0 > clip.origin.x
                && py + 1.0 > clip.origin.y
                && px < clip_right
                && py < clip_bottom
            {
                self.blend(
                    px as u32,
                    py as u32,
                    Pixel::rgba(
                        glyph_color.r(),
                        glyph_color.g(),
                        glyph_color.b(),
                        glyph_color.a(),
                    ),
                );
            }
        }
        for &(rect, strike_color) in &cached.strikes {
            self.fill_round(rect, 0.0, 0, strike_color, clip);
        }
    }

    fn reset_glyph_cache(&mut self) {
        self.swash_cache = Some(SwashCache::new());
        self.glyph_bytes = 0;
        self.glyph_generation_misses = 0;
        self.raster_stats.glyph_resets += 1;
    }

    #[allow(clippy::too_many_arguments)]
    fn raster_text(
        &mut self,
        index: usize,
        buffer: &Buffer,
        font_system: &mut cosmic_text::FontSystem,
        color: TextColor,
        physical: Rect,
        clip: Rect,
        strikes: StrikeLines,
    ) {
        let other_bytes = self
            .text_rasters
            .iter()
            .enumerate()
            .filter(|(other_index, _)| *other_index != index)
            .filter_map(|(_, cached)| cached.as_ref())
            .fold(0usize, |total, cached| {
                total.saturating_add(cached.retained_bytes())
            });
        let strike_bytes = strikes
            .capacity()
            .saturating_mul(std::mem::size_of::<(Rect, Color)>());
        let available =
            SOFTWARE_TEXT_RASTER_BUDGET.saturating_sub(other_bytes.saturating_add(strike_bytes));
        self.rejected_text_rasters
            .resize(self.text_rasters.len(), false);
        let mut candidate = (!self.rejected_text_rasters[index]).then(Vec::new);
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical_glyph = glyph.physical((0.0, run.line_y), 1.0);
                let key = physical_glyph.cache_key;
                if self
                    .swash_cache
                    .as_ref()
                    .expect("glyph cache")
                    .image_cache
                    .contains_key(&key)
                {
                    self.raster_stats.glyph_hits += 1;
                } else {
                    self.raster_stats.glyph_misses += 1;
                    // Replacing the owner also retires opaque scaler scratch.
                    if self.glyph_generation_misses >= SOFTWARE_GLYPH_ENTRY_BUDGET {
                        self.reset_glyph_cache();
                    }
                    self.glyph_generation_misses += 1;
                    let image = self
                        .swash_cache
                        .as_mut()
                        .expect("glyph cache")
                        .get_image_uncached(font_system, key);
                    let bytes = image.as_ref().map_or(0, |image| image.data.capacity());
                    // Swash exposes no preallocation limit: one newly rasterized
                    // glyph is an explicit transient peak, separate from retention.
                    self.raster_stats.glyph_peak_bytes = self
                        .raster_stats
                        .glyph_peak_bytes
                        .max(self.glyph_bytes.saturating_add(bytes));
                    let candidate_bytes = candidate.as_ref().map_or(0, |pixels| {
                        pixels
                            .capacity()
                            .saturating_mul(std::mem::size_of::<(i32, i32, TextColor)>())
                    });
                    self.raster_stats.known_cache_peak_bytes =
                        self.raster_stats.known_cache_peak_bytes.max(
                            other_bytes
                                .saturating_add(candidate_bytes)
                                .saturating_add(self.glyph_bytes)
                                .saturating_add(bytes),
                        );
                    if self.glyph_bytes.saturating_add(bytes) > SOFTWARE_GLYPH_BYTE_BUDGET
                        || self
                            .swash_cache
                            .as_ref()
                            .expect("glyph cache")
                            .image_cache
                            .len()
                            >= SOFTWARE_GLYPH_ENTRY_BUDGET
                    {
                        self.reset_glyph_cache();
                    }
                    self.glyph_bytes += bytes;
                    self.swash_cache
                        .as_mut()
                        .expect("glyph cache")
                        .image_cache
                        .insert(key, image);
                }
                let mut cache = self.swash_cache.take().expect("glyph cache");
                cache.with_pixels(
                    font_system,
                    key,
                    glyph.color_opt.unwrap_or(color),
                    |x, y, color| {
                        self.text_sample(
                            &mut candidate,
                            available,
                            physical,
                            clip,
                            physical_glyph.x + x,
                            physical_glyph.y + y,
                            color,
                        );
                    },
                );
                self.swash_cache = Some(cache);
                if self.glyph_bytes > SOFTWARE_GLYPH_BYTE_BUDGET {
                    self.raster_stats.glyph_rejections += 1;
                    self.reset_glyph_cache();
                }
            }
            let mut decorations = SampleDecorations {
                renderer: self,
                candidate: &mut candidate,
                available,
                physical,
                clip,
            };
            cosmic_text::render_decoration(&mut decorations, &run, color);
        }
        for &(rect, color) in &strikes {
            self.fill_round(rect, 0.0, 0, color, clip);
        }
        if let Some(pixels) = candidate {
            let cached = CachedSoftwareText { pixels, strikes };
            if other_bytes.saturating_add(cached.retained_bytes()) <= SOFTWARE_TEXT_RASTER_BUDGET {
                self.text_rasters[index] = Some(cached);
            }
        } else {
            self.rejected_text_rasters[index] = true;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text_sample(
        &mut self,
        candidate: &mut Option<SoftwareGlyphPixels>,
        available: usize,
        physical: Rect,
        clip: Rect,
        x: i32,
        y: i32,
        color: TextColor,
    ) {
        if let Some(pixels) = candidate {
            let element_bytes = std::mem::size_of::<(i32, i32, TextColor)>();
            if pixels.len() == pixels.capacity() {
                let previous_capacity = pixels.capacity();
                let capacity = pixels
                    .capacity()
                    .max(256)
                    .saturating_mul(2)
                    .min(available / element_bytes);
                if capacity > pixels.len() {
                    pixels.reserve_exact(capacity - pixels.len());
                }
                let bytes = pixels.capacity().saturating_mul(element_bytes);
                self.raster_stats.candidate_allocated_bytes =
                    self.raster_stats.candidate_allocated_bytes.saturating_add(
                        pixels
                            .capacity()
                            .saturating_sub(previous_capacity)
                            .saturating_mul(element_bytes) as u64,
                    );
                let retained = self
                    .text_rasters
                    .iter()
                    .flatten()
                    .map(CachedSoftwareText::retained_bytes)
                    .sum::<usize>();
                self.raster_stats.known_cache_peak_bytes =
                    self.raster_stats.known_cache_peak_bytes.max(
                        retained
                            .saturating_add(bytes)
                            .saturating_add(self.glyph_bytes),
                    );
            }
            let bytes = pixels.capacity().saturating_mul(element_bytes);
            self.raster_stats.candidate_peak_bytes =
                self.raster_stats.candidate_peak_bytes.max(bytes);
            if pixels.len() == pixels.capacity() || bytes > available {
                *candidate = None;
                self.raster_stats.rejected_rasters += 1;
            } else {
                pixels.push((x, y, color));
            }
        }
        // Samples are integral physical pixels. Testing their overlap directly
        // preserves fractional clipping without a rectangle scan per sample.
        let x = physical.origin.x.round() + x as f32;
        let y = physical.origin.y.round() + y as f32;
        if x >= 0.0
            && y >= 0.0
            && x < self.width as f32
            && y < self.height as f32
            && clip.size.width > 0.0
            && clip.size.height > 0.0
            && x + 1.0 > clip.origin.x
            && y + 1.0 > clip.origin.y
            && x < clip.origin.x + clip.size.width
            && y < clip.origin.y + clip.size.height
        {
            self.blend(
                x as u32,
                y as u32,
                Pixel::rgba(color.r(), color.g(), color.b(), color.a()),
            );
        }
    }

    fn styled_text(&mut self, index: usize, command: &PaintCommand, clip: Rect) {
        let mut font_system = self.font_system.lock();
        let PaintCommand::StyledText {
            bounds,
            text,
            spans,
            scale,
            font_size,
            color,
            align,
        } = command
        else {
            return;
        };
        let cached = self.text_rasters[index].take();
        if let Some(cached) = cached.filter(|_| {
            self.previous_commands
                .get(index)
                .is_some_and(|previous| text_raster_matches(previous, command))
        }) {
            self.draw_cached_text(&cached, physical_rect(*bounds, self.scale), clip);
            self.text_rasters[index] = Some(cached);
            return;
        }
        let font_size = font_size.unwrap_or_else(|| text_size(*scale)) * self.scale;
        let physical = physical_rect(*bounds, self.scale);
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(font_size, font_size * 1.3));
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
        buffer.shape_until_scroll(&mut font_system, false);
        self.raster_text(
            index,
            &buffer,
            &mut font_system,
            text_color(*color),
            physical,
            clip,
            Vec::new(),
        );
        let other_bytes: usize = self
            .text_rasters
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .filter_map(|(_, cached)| cached.as_ref())
            .map(CachedSoftwareText::retained_bytes)
            .sum();
        let mut cached = self.text_rasters[index].take();
        styled_strikes(
            &buffer,
            spans,
            physical,
            *color,
            font_size,
            |(rect, color)| {
                self.fill_round(rect, 0.0, 0, color, clip);
                if let Some(raster) = &mut cached {
                    let bytes = std::mem::size_of::<(Rect, Color)>();
                    if raster.strikes.len() == raster.strikes.capacity() {
                        let available = SOFTWARE_TEXT_RASTER_BUDGET
                            .saturating_sub(other_bytes.saturating_add(raster.retained_bytes()));
                        if available >= bytes {
                            raster.strikes.reserve_exact((available / bytes).min(256));
                        }
                    }
                    if raster.strikes.len() == raster.strikes.capacity()
                        || other_bytes.saturating_add(raster.retained_bytes())
                            > SOFTWARE_TEXT_RASTER_BUDGET
                    {
                        cached = None;
                        self.rejected_text_rasters[index] = true;
                        self.raster_stats.rejected_rasters += 1;
                    } else {
                        raster.strikes.push((rect, color));
                    }
                }
            },
        );
        self.text_rasters[index] = cached;
    }

    fn cached_image_samples(
        &mut self,
        rect: Rect,
        image: &Arc<image::RgbaImage>,
    ) -> Option<(Arc<[Pixel]>, u32, i32, i32)> {
        // Cache only small images on exact pixel or half-pixel origins. Those
        // positions preserve the same bilinear sample coordinates when moved.
        let fraction_x = rect.origin.x - rect.origin.x.floor();
        let fraction_y = rect.origin.y - rect.origin.y.floor();
        if image.as_raw().len() > SOFTWARE_IMAGE_SOURCE_LIMIT
            || !(fraction_x == 0.0 || fraction_x == 0.5)
            || !(fraction_y == 0.0 || fraction_y == 0.5)
            || rect.origin.x < 0.0
            || rect.origin.y < 0.0
            || rect.origin.x > 100_000.0
            || rect.origin.y > 100_000.0
        {
            return None;
        }
        let base_x = rect.origin.x.floor() as i32;
        let base_y = rect.origin.y.floor() as i32;
        let width = ((rect.origin.x + rect.size.width).ceil() as i32 - base_x) as u32;
        let height = ((rect.origin.y + rect.size.height).ceil() as i32 - base_y) as u32;
        if width == 0
            || height == 0
            || width > SOFTWARE_IMAGE_SIDE_LIMIT
            || height > SOFTWARE_IMAGE_SIDE_LIMIT
        {
            return None;
        }
        let old = self.image_rasters.iter().position(|cached| {
            Arc::ptr_eq(&cached.source, image)
                && cached.rect_width == rect.size.width.to_bits()
                && cached.rect_height == rect.size.height.to_bits()
                && cached.fraction_x == fraction_x.to_bits()
                && cached.fraction_y == fraction_y.to_bits()
        });
        if let Some(index) = old {
            self.raster_stats.image_cache_hits += 1;
            let cached = self.image_rasters.remove(index).expect("cache index");
            let samples = cached.pixels.clone();
            let sample_width = cached.width;
            self.image_rasters.push_back(cached);
            return Some((samples, sample_width, base_x, base_y));
        }
        self.raster_stats.image_cache_misses += 1;
        let mut samples = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                let source_x = ((base_x as f32 + x as f32 + 0.5 - rect.origin.x) / rect.size.width)
                    * image.width() as f32
                    - 0.5;
                let source_y = ((base_y as f32 + y as f32 + 0.5 - rect.origin.y)
                    / rect.size.height)
                    * image.height() as f32
                    - 0.5;
                let rgba = bilinear_sample(image, source_x, source_y);
                samples.push(Pixel::rgba(rgba[0], rgba[1], rgba[2], rgba[3]));
            }
        }
        let cached = CachedSoftwareImage {
            source: image.clone(),
            rect_width: rect.size.width.to_bits(),
            rect_height: rect.size.height.to_bits(),
            fraction_x: fraction_x.to_bits(),
            fraction_y: fraction_y.to_bits(),
            width,
            pixels: samples.into(),
        };
        let bytes = cached.retained_bytes();
        if bytes > SOFTWARE_IMAGE_RASTER_BUDGET {
            return None;
        }
        while self.image_raster_bytes.saturating_add(bytes) > SOFTWARE_IMAGE_RASTER_BUDGET {
            let evicted = self.image_rasters.pop_front().expect("nonempty cache");
            self.image_raster_bytes -= evicted.retained_bytes();
            self.raster_stats.image_cache_evictions += 1;
        }
        self.image_raster_bytes += bytes;
        let samples = cached.pixels.clone();
        self.image_rasters.push_back(cached);
        self.raster_stats.known_cache_peak_bytes = self
            .raster_stats
            .known_cache_peak_bytes
            .max(self.cache_diagnostics().live_bytes);
        Some((samples, width, base_x, base_y))
    }

    fn image(&mut self, rect: Rect, image: &Arc<image::RgbaImage>, clip: Rect) {
        if image.width() == 0 || image.height() == 0 {
            return;
        }
        let Some(bounds) = intersection(rect, clip) else {
            return;
        };
        if let Some((samples, sample_width, base_x, base_y)) =
            self.cached_image_samples(rect, image)
        {
            self.for_pixels(bounds, |renderer, x, y| {
                let offset_x = x as i32 - base_x;
                let offset_y = y as i32 - base_y;
                let source = samples[(offset_y as u32 * sample_width + offset_x as u32) as usize];
                renderer.blend(x, y, source);
            });
            return;
        }
        #[cfg(debug_assertions)]
        let profile_started = std::time::Instant::now();
        #[cfg(debug_assertions)]
        let profile_pixels = (bounds.size.width.ceil().max(0.0) as u64)
            .saturating_mul(bounds.size.height.ceil().max(0.0) as u64);
        self.for_pixels(bounds, |renderer, x, y| {
            let source_x =
                ((x as f32 + 0.5 - rect.origin.x) / rect.size.width) * image.width() as f32 - 0.5;
            let source_y =
                ((y as f32 + 0.5 - rect.origin.y) / rect.size.height) * image.height() as f32 - 0.5;
            let pixel = bilinear_sample(image, source_x, source_y);
            renderer.blend(
                px(x),
                px(y),
                Pixel::rgba(pixel[0], pixel[1], pixel[2], pixel[3]),
            );
        });
        #[cfg(debug_assertions)]
        image_profile::raster(profile_pixels, profile_started.elapsed());
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

fn bilinear_sample(image: &image::RgbaImage, x: f32, y: f32) -> [u8; 4] {
    let max_x = image.width().saturating_sub(1) as f32;
    let max_y = image.height().saturating_sub(1) as f32;
    let x = x.clamp(0.0, max_x);
    let y = y.clamp(0.0, max_y);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(image.width() - 1);
    let y1 = (y0 + 1).min(image.height() - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let weights = [
        ((1.0 - tx) * (1.0 - ty), image.get_pixel(x0, y0).0),
        (tx * (1.0 - ty), image.get_pixel(x1, y0).0),
        ((1.0 - tx) * ty, image.get_pixel(x0, y1).0),
        (tx * ty, image.get_pixel(x1, y1).0),
    ];
    let alpha = weights
        .iter()
        .map(|(weight, pixel)| weight * f32::from(pixel[3]))
        .sum::<f32>();
    let mut result = [0; 4];
    result[3] = alpha.round().clamp(0.0, 255.0) as u8;
    if alpha > 0.0 {
        for channel in 0..3 {
            let premultiplied = weights
                .iter()
                .map(|(weight, pixel)| weight * f32::from(pixel[channel]) * f32::from(pixel[3]))
                .sum::<f32>();
            result[channel] = (premultiplied / alpha).round().clamp(0.0, 255.0) as u8;
        }
    }
    result
}

pub(crate) fn command_bounds(command: &PaintCommand) -> Option<Rect> {
    match command {
        PaintCommand::Fill { rect, .. }
        | PaintCommand::TopRoundedFill { rect, .. }
        | PaintCommand::RoundedFill { rect, .. }
        | PaintCommand::Gradient { rect, .. }
        | PaintCommand::RoundedStroke { rect, .. }
        | PaintCommand::Stroke { rect, .. }
        | PaintCommand::OverlayFill { rect, .. }
        | PaintCommand::OverlayStroke { rect, .. } => Some(*rect),
        PaintCommand::Text { bounds, scale, .. } => Some(Rect::new(
            bounds.origin.x,
            bounds.origin.y,
            bounds.size.width.max(1.0),
            bounds.size.height.max(text_size(*scale) * 1.4),
        )),
        PaintCommand::StyledText {
            bounds,
            scale,
            font_size,
            ..
        } => {
            let font_size = font_size.unwrap_or_else(|| text_size(*scale));
            Some(Rect::new(
                bounds.origin.x,
                bounds.origin.y,
                bounds.size.width.max(1.0),
                bounds.size.height.max(font_size * 1.4),
            ))
        }
        PaintCommand::Image { bounds, .. } => Some(*bounds),
        PaintCommand::BackdropBlur { .. } | PaintCommand::PushClip(_) | PaintCommand::PopClip => {
            None
        }
    }
}

#[cfg(test)]
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

fn intersection(left: Rect, right: Rect) -> Option<Rect> {
    let x = left.origin.x.max(right.origin.x);
    let y = left.origin.y.max(right.origin.y);
    let right_edge = (left.origin.x + left.size.width).min(right.origin.x + right.size.width);
    let bottom_edge = (left.origin.y + left.size.height).min(right.origin.y + right.size.height);
    (right_edge > x && bottom_edge > y).then(|| Rect::new(x, y, right_edge - x, bottom_edge - y))
}

pub(crate) fn union_rect(left: Rect, right: Rect) -> Rect {
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
    crate::ui::text_font_size(scale)
}

#[allow(clippy::too_many_arguments)]
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

fn rich_attrs(color: Color, span: Option<&StyledTextSpan>, metadata: usize) -> Attrs<'_> {
    let family = span
        .and_then(|span| span.font_family.as_deref())
        .filter(|family| !family.eq_ignore_ascii_case("monospace"))
        .map(Family::Name)
        .unwrap_or_else(|| {
            if span.is_some_and(|span| span.monospace) {
                Family::Monospace
            } else {
                Family::SansSerif
            }
        });
    let mut attrs = Attrs::new()
        .family(family)
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
    spans: &'a [StyledTextSpan],
    color: Color,
) -> Vec<(&'a str, Attrs<'a>)> {
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
    mut emit: impl FnMut((Rect, Color)),
) {
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let Some(span) = glyph
                .metadata
                .checked_sub(1)
                .and_then(|index| spans.get(index))
            else {
                continue;
            };
            if span.strikethrough {
                let rect = Rect::new(
                    bounds.origin.x + glyph.x,
                    bounds.origin.y + run.line_top + run.line_height * 0.52,
                    glyph.w.max(1.0),
                    (font_size / 14.0).max(1.0),
                );
                emit((rect, span.color.unwrap_or(default_color)));
            }
            let thickness = (font_size / 14.0).max(1.0);
            let x = bounds.origin.x + glyph.x;
            let y = bounds.origin.y + run.line_top;
            let width = glyph.w.max(1.0);
            let color = span.color.unwrap_or(default_color);
            let mut patterned = |segment: f32, gap: f32, alternating: bool| {
                let mut offset = 0.0;
                let mut raised = false;
                while offset < width {
                    let segment_width = segment.min(width - offset);
                    emit((
                        Rect::new(
                            x + offset,
                            y + run.line_height * if raised { 0.82 } else { 0.9 },
                            segment_width,
                            thickness,
                        ),
                        color,
                    ));
                    offset += segment + gap;
                    if alternating {
                        raised = !raised;
                    }
                }
            };
            match span.underline {
                TextUnderlineStyle::None => {}
                TextUnderlineStyle::Single => emit((
                    Rect::new(x, y + run.line_height * 0.88, width, thickness),
                    color,
                )),
                TextUnderlineStyle::Double => {
                    emit((
                        Rect::new(x, y + run.line_height * 0.8, width, thickness),
                        color,
                    ));
                    emit((
                        Rect::new(x, y + run.line_height * 0.92, width, thickness),
                        color,
                    ));
                }
                TextUnderlineStyle::Curly => patterned(thickness * 1.5, thickness, true),
                TextUnderlineStyle::Dotted => patterned(thickness, thickness, false),
                TextUnderlineStyle::Dashed => patterned(thickness * 3.0, thickness * 1.5, false),
            }
        }
    }
}

fn px(value: u32) -> u32 {
    value
}

#[cfg(test)]
mod tests {
    use nickel_core::resource_owner::{DependencyOwnerKind, dependency_owner_diagnostics};

    use super::{
        PaintCommand, Pixel, Rect, SoftwareRenderer, TextAlign, bilinear_sample,
        command_intersects_clip,
    };

    #[test]
    fn image_scaling_bilinearly_filters_premultiplied_alpha() {
        let image = image::RgbaImage::from_fn(2, 1, |x, _| {
            if x == 0 {
                image::Rgba([255, 255, 255, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        });
        let sample = bilinear_sample(&image, 0.5, 0.0);
        assert_eq!(sample, [255, 255, 255, 128]);
    }

    #[test]
    fn cached_moving_images_match_uncached_bilinear_pixels() {
        use std::sync::Arc;

        let source = Arc::new(image::RgbaImage::from_fn(96, 96, |x, y| {
            image::Rgba([
                (x * 2) as u8,
                (y * 2) as u8,
                (x + y) as u8,
                ((x * 3 + y * 5) % 256) as u8,
            ])
        }));
        let mut cached = SoftwareRenderer::new(180, 120, 1.0);
        for (x, y) in [(10.0, 8.0), (40.0, 18.0), (15.5, 11.5), (52.5, 30.5)] {
            let rect = Rect::new(x, y, 63.0, 63.0);
            let clip = Rect::new(16.25, 13.25, 100.0, 80.0);
            let commands = [
                PaintCommand::PushClip(clip),
                PaintCommand::Image {
                    bounds: rect,
                    id: 1,
                    generation: 1,
                    image: source.clone(),
                    high_density: None,
                },
                PaintCommand::PopClip,
            ];
            cached.render(&commands);

            let mut reference = SoftwareRenderer::new(180, 120, 1.0);
            let bounds = super::intersection(rect, clip).unwrap();
            reference.for_pixels(bounds, |renderer, px, py| {
                let source_x = ((px as f32 + 0.5 - rect.origin.x) / rect.size.width)
                    * source.width() as f32
                    - 0.5;
                let source_y = ((py as f32 + 0.5 - rect.origin.y) / rect.size.height)
                    * source.height() as f32
                    - 0.5;
                let rgba = bilinear_sample(&source, source_x, source_y);
                renderer.blend(px, py, Pixel::rgba(rgba[0], rgba[1], rgba[2], rgba[3]));
            });
            assert_eq!(cached.pixels(), reference.pixels());
        }
        assert_eq!(cached.raster_stats.image_cache_hits, 2);
        assert_eq!(cached.raster_stats.image_cache_misses, 2);
        cached.suspend();
        assert_eq!(cached.cache_diagnostics().image_textures, 0);
    }

    #[test]
    fn gradient_rows_and_columns_match_per_pixel_reference() {
        for axis in [
            crate::GradientAxis::Horizontal,
            crate::GradientAxis::Vertical,
        ] {
            for (start, end) in [(0xff102030, 0xffc0d0e0), (0x80112233, 0x40aabbcc)] {
                let gradient = crate::LinearGradient { start, end, axis };
                let rect = Rect::new(2.5, 3.5, 49.0, 30.0);
                let clip = Rect::new(5.25, 6.25, 42.0, 25.0);
                let mut optimized = SoftwareRenderer::new(60, 40, 1.0);
                optimized.render(&[
                    PaintCommand::PushClip(clip),
                    PaintCommand::Gradient { rect, gradient },
                    PaintCommand::PopClip,
                ]);
                let mut reference = SoftwareRenderer::new(60, 40, 1.0);
                reference.for_pixels(
                    super::intersection(rect, clip).unwrap(),
                    |renderer, x, y| {
                        let progress = match axis {
                            crate::GradientAxis::Horizontal => {
                                (x as f32 + 0.5 - rect.origin.x) / rect.size.width
                            }
                            crate::GradientAxis::Vertical => {
                                (y as f32 + 0.5 - rect.origin.y) / rect.size.height
                            }
                        }
                        .clamp(0.0, 1.0);
                        renderer.blend(x, y, super::lerp_color(start, end, progress));
                    },
                );
                assert_eq!(optimized.pixels(), reference.pixels());
            }
        }
    }

    #[test]
    fn small_image_cache_remains_bounded_under_source_churn() {
        use std::sync::Arc;

        let mut renderer = SoftwareRenderer::new(80, 80, 1.0);
        for index in 0..80 {
            let source = Arc::new(image::RgbaImage::from_pixel(
                96,
                96,
                image::Rgba([index, 24, 48, 255]),
            ));
            renderer.render(&[PaintCommand::Image {
                bounds: Rect::new(1.0, 1.0, 63.0, 63.0),
                id: 1,
                generation: index as u64,
                image: source,
                high_density: None,
            }]);
            assert!(renderer.image_raster_bytes <= super::SOFTWARE_IMAGE_RASTER_BUDGET);
        }
        assert!(renderer.raster_stats.image_cache_evictions > 0);
    }

    #[test]
    fn rounded_border_preserves_center_and_corner_background_at_multiple_scales() {
        for scale in [1.0, 1.5, 2.0] {
            let mut renderer =
                SoftwareRenderer::new((40.0 * scale) as u32, (30.0 * scale) as u32, scale);
            renderer.render(&[
                PaintCommand::Fill {
                    rect: Rect::new(0.0, 0.0, 40.0, 30.0),
                    color: 0xff123456,
                },
                PaintCommand::RoundedStroke {
                    rect: Rect::new(5.0, 5.0, 30.0, 20.0),
                    color: 0xffabcdef,
                    width: 2.0,
                    radius: 6.0,
                },
            ]);
            let at = |x: f32, y: f32| {
                renderer.pixels()
                    [((y * scale) as u32 * renderer.size().0 + (x * scale) as u32) as usize]
            };
            assert_eq!(at(20.0, 15.0), Pixel::rgba(0x12, 0x34, 0x56, 255));
            assert_eq!(at(5.0, 5.0), Pixel::rgba(0x12, 0x34, 0x56, 255));
            assert_eq!(at(20.0, 6.0), Pixel::rgba(0xab, 0xcd, 0xef, 255));
        }
    }

    #[test]
    fn exact_text_pixels_match_layout_font_size() {
        assert_eq!(super::text_size(-14.0), 14.0);
        assert_eq!(super::text_size(2.0), 16.0);
    }

    fn label(styled: bool, scale: f32) -> PaintCommand {
        if styled {
            PaintCommand::StyledText {
                bounds: Rect::new(2.3, 2.7, 95.0, 32.0),
                text: "Hello 世界 🦀".into(),
                spans: Vec::new(),
                scale,
                font_size: None,
                color: 0xffaacc,
                align: TextAlign::Start,
            }
        } else {
            PaintCommand::Text {
                bounds: Rect::new(2.3, 2.7, 95.0, 32.0),
                text: "Hello 世界 🦀".into(),
                scale,
                color: 0xffaacc,
                align: TextAlign::Start,
                bold: false,
                wrap: true,
            }
        }
    }

    #[test]
    fn empty_frames_stay_clean_until_invalidated() {
        let mut renderer = SoftwareRenderer::new(20, 20, 1.0);
        assert!(!renderer.render(&[]).is_empty());
        assert!(renderer.render(&[]).is_empty());
        renderer.invalidate();
        assert!(!renderer.render(&[]).is_empty());
        assert!(renderer.render(&[]).is_empty());
    }

    #[test]
    fn zero_height_text_damage_covers_its_minimum_raster_height() {
        let mut renderer = SoftwareRenderer::new(160, 40, 1.0);
        let command = |text: &str| PaintCommand::Text {
            bounds: Rect::new(4.0, 6.0, 150.0, 0.0),
            text: text.into(),
            scale: 1.0,
            color: 0xffff_ffff,
            align: TextAlign::Start,
            bold: false,
            wrap: false,
        };
        renderer.render(&[command("Before")]);
        let before = renderer.pixels().to_vec();
        let damage = renderer.render(&[command("After")]);

        for (index, (before, after)) in before.iter().zip(renderer.pixels()).enumerate() {
            if before != after {
                let x = index as u32 % 160;
                let y = index as u32 / 160;
                assert!(damage.rects.iter().any(|rect| {
                    x as f32 + 0.5 >= rect.origin.x
                        && x as f32 + 0.5 < rect.origin.x + rect.size.width
                        && y as f32 + 0.5 >= rect.origin.y
                        && y as f32 + 0.5 < rect.origin.y + rect.size.height
                }));
            }
        }
    }

    #[test]
    fn clipped_command_change_cannot_reuse_the_previous_raster() {
        let mut renderer = SoftwareRenderer::new(160, 80, 1.0);
        let full = Rect::new(0.0, 0.0, 160.0, 80.0);
        let mut commands = vec![
            PaintCommand::PushClip(full),
            label(false, 1.0),
            PaintCommand::PopClip,
        ];
        renderer.render(&commands);
        assert!(renderer.text_rasters[1].is_some());
        commands[0] = PaintCommand::PushClip(Rect::new(150.0, 70.0, 1.0, 1.0));
        if let PaintCommand::Text { text, .. } = &mut commands[1] {
            *text = "Changed".into();
        }
        renderer.render(&commands);
        commands[0] = PaintCommand::PushClip(full);
        renderer.render(&commands);
        let mut fresh = SoftwareRenderer::new(160, 80, 1.0);
        fresh.render(&commands);
        assert_eq!(renderer.pixels(), fresh.pixels());
    }

    #[test]
    fn styled_decorations_and_raster_identity_survive_full_repaints() {
        for underline in [
            super::TextUnderlineStyle::Single,
            super::TextUnderlineStyle::Double,
            super::TextUnderlineStyle::Curly,
            super::TextUnderlineStyle::Dotted,
            super::TextUnderlineStyle::Dashed,
        ] {
            let mut command = label(true, 1.0);
            if let PaintCommand::StyledText { spans, .. } = &mut command {
                spans.push(super::StyledTextSpan {
                    range: 0..5,
                    bold: true,
                    italic: true,
                    monospace: true,
                    font_family: None,
                    strikethrough: true,
                    underline,
                    color: Some(0xff9900),
                    background: None,
                });
            }
            let mut renderer = SoftwareRenderer::new(160, 80, 1.25);
            renderer.render(std::slice::from_ref(&command));
            let cold = renderer.pixels().to_vec();
            let allocated = renderer.raster_stats.candidate_allocated_bytes;
            assert!(
                !renderer.text_rasters[0]
                    .as_ref()
                    .unwrap()
                    .strikes
                    .is_empty()
            );
            renderer.invalidate();
            assert!(!renderer.render(std::slice::from_ref(&command)).is_empty());
            assert_eq!(renderer.pixels(), cold);
            assert_eq!(renderer.raster_stats.candidate_allocated_bytes, allocated);
        }
    }

    // Repeatable release microbenchmark. Targets chosen before implementation:
    // 40% less sample payload, <=2 MiB candidates, <=25% cold regression.
    // Timings are observations, not flaky CI assertions.
    #[test]
    #[ignore = "release memory/timing evidence"]
    fn software_raster_memory_and_timing_evidence() {
        use std::time::Instant;
        let mut renderer = SoftwareRenderer::new(640, 200, 1.25);
        let mut commands = vec![
            PaintCommand::Fill {
                rect: Rect::new(0.0, 0.0, 640.0, 200.0),
                color: 0x112233,
            },
            label(true, 1.0),
        ];
        let cold = Instant::now();
        renderer.render(&commands);
        let cold = cold.elapsed();
        let samples = renderer.text_rasters[1].as_ref().unwrap().pixels.len();
        let capacity = renderer.text_rasters[1].as_ref().unwrap().pixels.capacity();
        let warm = Instant::now();
        for iteration in 0..500 {
            if let PaintCommand::Fill { color, .. } = &mut commands[0] {
                *color = 0x112233 + iteration % 2;
            }
            renderer.render(&commands);
        }
        let warm = warm.elapsed();
        let churn = Instant::now();
        for iteration in 0..500 {
            if let PaintCommand::StyledText { text, .. } = &mut commands[1] {
                *text = format!("Churn {iteration} 世界 🦀");
            }
            renderer.render(&commands);
        }
        eprintln!(
            "software raster evidence: cold={cold:?}; warm500={warm:?}; churn500={:?}; samples={samples}; legacy_same_capacity={} compact_capacity={}; diagnostics={:?}; cache={:?}",
            churn.elapsed(),
            capacity * 20,
            capacity * 12,
            renderer.software_raster_diagnostics(),
            renderer.cache_diagnostics()
        );
        // Compare the old 20-byte tuple collection/drawing algorithm with the
        // compact admission path using identical already-shaped text and warm
        // glyph caches. This excludes shared font loading and shaping.
        use cosmic_text::{Attrs, Buffer, Color, Metrics, Shaping, SwashCache};
        let font_owner = nickel_render_assets::ProcessFontSystem::new();
        let mut fonts = font_owner.lock();
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(15.0, 19.5));
        buffer.set_size(Some(640.0), Some(200.0));
        buffer.set_text(
            "The quick brown fox 世界 🦀",
            &Attrs::new(),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut fonts, false);
        let bounds = Rect::new(0.0, 0.0, 640.0, 200.0);
        let mut legacy = SoftwareRenderer::new(640, 200, 1.0);
        let mut glyph_cache = SwashCache::new();
        let mut legacy_samples = Vec::new();
        let baseline = Instant::now();
        for _ in 0..1000 {
            let mut samples = Vec::new();
            buffer.draw(
                &mut fonts,
                &mut glyph_cache,
                Color::rgb(250, 240, 230),
                |x, y, w, h, color| samples.push((x, y, w, h, color)),
            );
            legacy.pixels.fill(Pixel::TRANSPARENT);
            for &(x, y, w, h, color) in &samples {
                if let Some(rect) =
                    super::intersection(Rect::new(x as f32, y as f32, w as f32, h as f32), bounds)
                {
                    legacy.for_pixels(rect, |renderer, x, y| {
                        renderer.blend(
                            x,
                            y,
                            Pixel::rgba(color.r(), color.g(), color.b(), color.a()),
                        )
                    });
                }
            }
            legacy_samples = samples;
        }
        let baseline = baseline.elapsed();
        let mut compact = SoftwareRenderer::new(640, 200, 1.0);
        compact.text_rasters = vec![None];
        let optimized = Instant::now();
        for _ in 0..1000 {
            compact.text_rasters[0] = None;
            compact.pixels.fill(Pixel::TRANSPARENT);
            compact.raster_text(
                0,
                &buffer,
                &mut fonts,
                Color::rgb(250, 240, 230),
                bounds,
                bounds,
                Vec::new(),
            );
        }
        let optimized = optimized.elapsed();
        assert_eq!(legacy.pixels(), compact.pixels());
        eprintln!(
            "pre-shaped uncached-label1000: legacy={baseline:?} compact={optimized:?} ratio={:.3}; legacy_capacity={} compact_capacity={}",
            optimized.as_secs_f64() / baseline.as_secs_f64(),
            legacy_samples.capacity() * 20,
            compact.text_rasters[0].as_ref().unwrap().retained_bytes()
        );
    }

    #[test]
    fn scale_only_resize_repaints_identical_commands_like_a_fresh_renderer() {
        for styled in [false, true] {
            let commands = [
                PaintCommand::Fill {
                    rect: Rect::new(1.0, 1.0, 10.0, 10.0),
                    color: 0x336699,
                },
                label(styled, 1.0),
            ];
            let mut renderer = SoftwareRenderer::new(160, 80, 1.0);
            renderer.render(&commands);
            renderer.resize(160, 80, 1.0);
            assert!(renderer.render(&commands).is_empty());
            for scale in [1.25, 2.0, 0.75] {
                renderer.resize(160, 80, scale);
                assert!(!renderer.render(&commands).is_empty());
                let mut fresh = SoftwareRenderer::new(160, 80, scale);
                fresh.render(&commands);
                assert_eq!(renderer.pixels(), fresh.pixels());
                assert!(renderer.render(&commands).is_empty());
            }
            renderer.suspend();
            renderer.resize(160, 80, 1.25);
            assert!(!renderer.render(&commands).is_empty());
        }
    }

    #[test]
    fn size_only_resize_reuses_text_rasters_and_matches_fresh_pixels() {
        for styled in [false, true] {
            let commands = [
                PaintCommand::PushClip(Rect::new(2.5, 2.5, 80.25, 24.25)),
                label(styled, 1.0),
                PaintCommand::PopClip,
            ];
            let mut renderer = SoftwareRenderer::new(160, 80, 1.0);
            renderer.render(&commands);
            assert!(renderer.text_rasters[1].is_some());
            let allocated = renderer.raster_stats.candidate_allocated_bytes;
            for (width, height) in [(180, 90), (120, 70), (160, 80)] {
                renderer.resize(width, height, 1.0);
                assert!(!renderer.render(&commands).is_empty());
                let mut fresh = SoftwareRenderer::new(width, height, 1.0);
                fresh.render(&commands);
                assert_eq!(renderer.pixels(), fresh.pixels());
                assert_eq!(renderer.raster_stats.candidate_allocated_bytes, allocated);
            }
        }
    }

    #[test]
    fn moved_plain_text_reuses_relative_raster_and_matches_fresh_pixels() {
        let mut renderer = SoftwareRenderer::new(180, 90, 1.0);
        let mut command = label(false, 1.0);
        renderer.render(std::slice::from_ref(&command));
        let allocated = renderer.raster_stats.candidate_allocated_bytes;
        if let PaintCommand::Text { bounds, .. } = &mut command {
            *bounds = Rect::new(19.3, 11.7, bounds.size.width, bounds.size.height);
        }
        renderer.render(std::slice::from_ref(&command));
        let mut fresh = SoftwareRenderer::new(180, 90, 1.0);
        fresh.render(std::slice::from_ref(&command));
        assert_eq!(renderer.pixels(), fresh.pixels());
        assert_eq!(renderer.raster_stats.candidate_allocated_bytes, allocated);
    }

    #[test]
    fn compact_raster_matches_cosmic_reference_and_bounds_oversized_candidates() {
        use cosmic_text::{Attrs, Buffer, Color, Metrics, Shaping, SwashCache};
        let mut renderer = SoftwareRenderer::new(320, 160, 1.0);
        let mut reference = SoftwareRenderer::new(320, 160, 1.0);
        let fonts = nickel_render_assets::ProcessFontSystem::new();
        let mut fonts = fonts.lock();
        for size in [13.25, 32.0, 120.0] {
            let mut buffer = Buffer::new(&mut fonts, Metrics::new(size, size * 1.3));
            buffer.set_size(Some(320.0), Some(160.0));
            buffer.set_text(
                "Dense WWW sparse iii 世界 العربية 🦀",
                &Attrs::new(),
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(&mut fonts, false);
            let physical = Rect::new(0.4, 0.6, 320.0, 160.0);
            let clip = Rect::new(3.25, 4.5, 300.0, 150.0);
            renderer.pixels.fill(Pixel::TRANSPARENT);
            reference.pixels.fill(Pixel::TRANSPARENT);
            renderer.text_rasters = vec![None];
            renderer.raster_text(
                0,
                &buffer,
                &mut fonts,
                Color::rgb(10, 150, 200),
                physical,
                clip,
                Vec::new(),
            );
            buffer.draw(
                &mut fonts,
                &mut SwashCache::new(),
                Color::rgb(10, 150, 200),
                |x, y, w, h, color| {
                    let rect = Rect::new(
                        physical.origin.x.round() + x as f32,
                        physical.origin.y.round() + y as f32,
                        w as f32,
                        h as f32,
                    );
                    if let Some(rect) = super::intersection(rect, clip) {
                        reference.for_pixels(rect, |renderer, x, y| {
                            renderer.blend(
                                x,
                                y,
                                Pixel::rgba(color.r(), color.g(), color.b(), color.a()),
                            )
                        });
                    }
                },
            );
            assert_eq!(renderer.pixels(), reference.pixels());
        }
        assert_eq!(std::mem::size_of::<(i32, i32, Color)>(), 12);
        // Exercise the same admission path used by arbitrarily long labels:
        // after rejection, subsequent samples draw directly without allocation.
        let mut candidate = Some(Vec::new());
        for _ in 0..300_000 {
            renderer.text_sample(
                &mut candidate,
                super::SOFTWARE_TEXT_RASTER_BUDGET,
                Rect::new(0.0, 0.0, 320.0, 160.0),
                Rect::new(0.0, 0.0, 320.0, 160.0),
                10,
                10,
                Color::rgb(255, 0, 0),
            );
        }
        assert!(candidate.is_none());
        assert!(renderer.raster_stats.candidate_peak_bytes <= super::SOFTWARE_TEXT_RASTER_BUDGET);
        assert_eq!(renderer.pixels()[3210], Pixel::rgba(255, 0, 0, 255));
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(120.0, 156.0));
        buffer.set_size(Some(20_000.0), Some(200.0));
        buffer.set_text(&"W".repeat(1000), &Attrs::new(), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut fonts, false);
        renderer.text_rasters = vec![None];
        renderer.rejected_text_rasters = vec![false];
        let rect = Rect::new(0.0, 0.0, 320.0, 160.0);
        renderer.raster_text(
            0,
            &buffer,
            &mut fonts,
            Color::rgb(255, 0, 0),
            rect,
            rect,
            Vec::new(),
        );
        assert!(renderer.rejected_text_rasters[0]);
        let allocated = renderer.raster_stats.candidate_allocated_bytes;
        renderer.raster_text(
            0,
            &buffer,
            &mut fonts,
            Color::rgb(255, 0, 0),
            rect,
            rect,
            Vec::new(),
        );
        assert_eq!(renderer.raster_stats.candidate_allocated_bytes, allocated);
    }

    #[test]
    fn plain_styled_and_mixed_churn_share_glyph_budget_and_suspend_release() {
        for mode in 0..3 {
            let mut renderer = SoftwareRenderer::new(160, 80, 1.0);
            for iteration in 0..2200 {
                // Preserve the owner while churning physical glyph sizes;
                // resize's independent lifecycle reset is covered above.
                renderer.scale = 0.75 + iteration as f32 / 3000.0;
                let mut command = label(mode == 1 || mode == 2 && iteration % 2 == 0, 1.0);
                match &mut command {
                    PaintCommand::Text { text, .. } | PaintCommand::StyledText { text, .. } => {
                        *text = format!("Ab{iteration}");
                    }
                    _ => unreachable!(),
                }
                renderer.invalidate();
                renderer.render(&[command]);
                assert!(renderer.glyph_bytes <= super::SOFTWARE_GLYPH_BYTE_BUDGET);
                assert!(
                    renderer.swash_cache.as_ref().unwrap().image_cache.len()
                        <= super::SOFTWARE_GLYPH_ENTRY_BUDGET
                );
            }
            assert!(renderer.raster_stats.glyph_resets > 0);
            renderer.invalidate();
            renderer.render(&[label(true, 1.0)]);
            renderer.invalidate();
            renderer.render(&[label(true, 1.0)]);
            assert!(renderer.raster_stats.glyph_hits > 0);
            renderer.suspend();
            assert_eq!(renderer.cache_diagnostics().live_bytes, 0);
            assert_eq!(renderer.swash_cache.as_ref().unwrap().image_cache.len(), 0);
            assert_eq!(renderer.pixel_capacity_bytes(), 4);
        }
    }

    #[test]
    fn rectangular_fill_fast_path_preserves_opaque_and_translucent_blending() {
        let mut renderer = SoftwareRenderer::new_pixel_buffer(4, 3, 1.0);
        renderer.render(&[
            PaintCommand::Fill {
                rect: Rect::new(0.0, 0.0, 4.0, 3.0),
                color: 0x20_40_60,
            },
            PaintCommand::Fill {
                rect: Rect::new(1.0, 1.0, 2.0, 1.0),
                color: 0x80_ff_00_00,
            },
        ]);

        assert_eq!(renderer.pixels()[0], Pixel::rgba(0x20, 0x40, 0x60, 255));
        assert_eq!(renderer.pixels()[5], Pixel::rgba(0x8f, 0x1f, 0x2f, 255));
        assert_eq!(renderer.pixels()[6], Pixel::rgba(0x8f, 0x1f, 0x2f, 255));
        assert_eq!(renderer.pixels()[7], Pixel::rgba(0x20, 0x40, 0x60, 255));
    }

    #[test]
    fn software_renderer_churn_respects_process_font_system_bound() {
        drop(nickel_render_assets::ProcessFontSystem::new().lock());
        for _ in 0..8 {
            let renderers = (0..4)
                .map(|_| SoftwareRenderer::new_pixel_buffer(2, 2, 1.0))
                .collect::<Vec<_>>();
            let during = dependency_owner_diagnostics(DependencyOwnerKind::CosmicTextFontSystem);
            assert_eq!(during.active_owners, 1);
            assert_eq!(during.peak_owners, 1);
            drop(renderers);
        }
        let after = dependency_owner_diagnostics(DependencyOwnerKind::CosmicTextFontSystem);
        assert_eq!(after.active_owners, 1);
        assert_eq!(after.peak_owners, 1);
    }

    #[test]
    fn aggregate_presenter_diagnostics_saturate_across_surfaces() {
        let aggregate = super::AggregatePresenterCacheDiagnostics::from_presenters([
            super::PresenterCacheDiagnostics {
                text_layouts: 2,
                text_layout_bytes: 80,
                image_textures: 1,
                glyphs: 4,
                glyph_atlas_width: 8,
                glyph_atlas_height: 8,
                glyph_atlas_bytes: 256,
                live_bytes: 120,
                peak_bytes: 180,
                hits: 8,
                misses: 2,
                insertions: 6,
                evictions: 1,
                invalidations: 1,
                recomputation_nanos: 90,
            },
            super::PresenterCacheDiagnostics {
                text_layouts: 1,
                text_layout_bytes: 40,
                image_textures: 2,
                glyphs: 3,
                glyph_atlas_width: 8,
                glyph_atlas_height: 8,
                glyph_atlas_bytes: 256,
                live_bytes: 100,
                peak_bytes: 140,
                hits: 5,
                misses: 3,
                insertions: 4,
                evictions: 2,
                invalidations: 2,
                recomputation_nanos: 70,
            },
        ]);
        assert_eq!(aggregate.presenters, 2);
        assert_eq!(aggregate.live_entries, 13);
        assert_eq!(aggregate.live_bytes, 220);
        assert_eq!(aggregate.peak_cache_bytes, 320);
        assert_eq!((aggregate.hits, aggregate.misses), (13, 5));
        assert_eq!((aggregate.insertions, aggregate.evictions), (10, 3));
        assert_eq!(aggregate.invalidations, 3);
        assert_eq!(aggregate.recomputation_nanos, 160);
    }

    #[test]
    fn suspended_pixel_renderer_releases_frame_storage_and_can_render_again() {
        let mut renderer = SoftwareRenderer::new_pixel_buffer(1920, 1080, 1.0);
        assert_eq!(renderer.pixels().len(), 1920 * 1080);

        renderer.suspend();
        assert_eq!(renderer.size(), (1, 1));
        assert_eq!(renderer.pixels().len(), 1);

        renderer.resize(64, 32, 1.0);
        let damage = renderer.render(&[PaintCommand::Fill {
            rect: Rect::new(0.0, 0.0, 64.0, 32.0),
            color: 0x010203,
        }]);
        assert!(!damage.is_empty());
        assert_eq!(renderer.pixels().len(), 64 * 32);
    }

    #[test]
    fn idle_and_lifecycle_churn_reach_a_bounded_resource_plateau() {
        const MAX_WIDTH: u32 = 256;
        const MAX_HEIGHT: u32 = 128;
        const MAX_FRAME_BYTES: usize = MAX_WIDTH as usize * MAX_HEIGHT as usize * 4;
        // Vec growth may reserve the next geometric capacity, but repeated
        // churn must not grow beyond that one bounded allocation step.
        const MAX_CAPACITY_BYTES: usize = MAX_FRAME_BYTES * 2;
        let command = |width, height| PaintCommand::Fill {
            rect: Rect::new(0.0, 0.0, width as f32, height as f32),
            color: 0x102030,
        };

        for mount in 0..16 {
            let mut renderer = SoftwareRenderer::new_pixel_buffer(1, 1, 1.0);
            for resize in 0..32 {
                let width = 64 + ((mount + resize) % 4) * 64;
                let height = 32 + ((mount + resize) % 4) * 32;
                renderer.resize(width, height, 1.0 + (resize % 2) as f32);
                let commands = [command(width, height)];
                assert!(!renderer.render(&commands).is_empty());
                for _ in 0..8 {
                    assert!(renderer.render(&commands).is_empty());
                }
                let diagnostics = renderer.software_raster_diagnostics();
                assert!(diagnostics.framebuffer_live_bytes <= MAX_CAPACITY_BYTES);
                assert!(diagnostics.framebuffer_peak_bytes <= MAX_CAPACITY_BYTES);
                assert_eq!(renderer.cache_diagnostics().live_bytes, 0);
            }
            let clean_before = renderer.software_raster_diagnostics().clean_frames;
            assert!(clean_before >= 32 * 8);
            renderer.suspend();
            let diagnostics = renderer.software_raster_diagnostics();
            assert_eq!(diagnostics.framebuffer_live_bytes, 4);
            assert!(diagnostics.framebuffer_peak_bytes <= MAX_CAPACITY_BYTES);
            assert_eq!(renderer.cache_diagnostics().live_bytes, 0);
        }
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
}
