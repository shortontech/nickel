//! Compositor ownership for Nickel UI applications which do not have a Wayland surface.

use std::{
    collections::{BTreeMap, HashMap},
    hash::Hash,
    time::Instant,
};

use nickel_ui::{
    Application, DamageRegion, GradientAxis, HostBatch, HostEvent, InternalSurfaceId,
    InternalSurfaceSet, LinearGradient, Point as UiPoint, SoftwareRenderer, Text, UiEvent, View,
    ViewContext,
    backend::{FrameRenderer, PaintCommand, RenderFrame},
};

use super::backend::InternalUiRendererMode;
use sha2::{Digest, Sha256};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Color32F, ImportMem, Renderer,
            element::{
                Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::{SolidColorBuffer, SolidColorRenderElement},
            },
        },
    },
    utils::{Logical, Point, Rectangle, Transform},
};

smithay::backend::renderer::element::render_elements! {
    /// Smithay elements emitted by the compositor-owned Nickel UI presenter.
    pub InternalUiRenderElement<R> where R: Renderer + ImportMem;
    Memory=MemoryRenderBufferRenderElement<R>,
    Solid=SolidColorRenderElement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternalSurfaceRole {
    Desktop,
    Panel,
    Overlay,
    Application,
}

/// The compositor scene boundary an internal surface occupies.
///
/// Background surfaces are composed behind every Wayland client. Overlay
/// surfaces are composed in front of the client scene.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternalSurfaceLayer {
    Background,
    Overlay,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InternalSurfacePlacement {
    pub role: InternalSurfaceRole,
    pub geometry: (i32, i32, u32, u32),
    pub output: Option<String>,
}

struct PresentedSurface {
    placement: InternalSurfacePlacement,
    renderer: SmithayFrameRenderer,
    dirty: bool,
    external_scene: Option<Vec<PaintCommand>>,
    scale_factor: f32,
}

struct SceneSlot;

impl Application for SceneSlot {
    type Message = ();
    fn update(&mut self, (): ()) {}
    fn view(&self, _: ViewContext) -> impl View<Self::Message> {
        Text::new("")
    }
}

fn output_local_location(
    geometry: (i32, i32, u32, u32),
    output_origin: Point<i32, Logical>,
) -> (f64, f64) {
    (
        f64::from(geometry.0 - output_origin.x),
        f64::from(geometry.1 - output_origin.y),
    )
}

/// How the most recently prepared UI frame will reach Smithay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternalUiPresentationMode {
    /// Every primitive maps to a Smithay solid or independently imported
    /// texture element; there is no full-surface upload.
    GpuSolid,
    /// The display list contains a primitive not yet handled natively and is
    /// rasterized once into an importable texture to preserve exact ordering.
    RasterFallback,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InternalUiRendererDiagnostics {
    pub gpu_frames: u64,
    pub fallback_frames: u64,
    pub fallback_primitive_count: usize,
    pub fallback_text_count: usize,
    pub fallback_image_count: usize,
    /// Host-side texture buffers allocated for image resources.
    pub image_allocations: u64,
    /// Image buffers made available for a renderer upload. A cache hit reuses
    /// the same Smithay buffer id and therefore does not increment this value.
    pub image_uploads: u64,
    pub image_cache_hits: u64,
    pub image_cache_misses: u64,
    pub image_cache_evictions: u64,
    pub image_cache_entries: usize,
    pub image_cache_bytes: usize,
    /// Host-side texture buffers allocated for rasterized text resources.
    pub text_allocations: u64,
    /// Rasterized text buffers made available for a renderer upload.
    pub text_uploads: u64,
    pub text_cache_hits: u64,
    pub text_cache_misses: u64,
    pub text_cache_evictions: u64,
    pub text_cache_entries: usize,
    pub text_cache_bytes: usize,
}

/// Nickel display-list adapter for Smithay's renderer element API.
///
/// Geometry and gradients become solid render elements. Images are imported as
/// their own textures, while text is rasterized into tightly bounded glyph
/// textures; neither causes a full-surface software upload.
pub struct SmithayFrameRenderer {
    software: SoftwareRenderer,
    text_software: SoftwareRenderer,
    primitives: Vec<GpuPrimitive>,
    raster: Option<MemoryRenderBuffer>,
    mode: InternalUiPresentationMode,
    diagnostics: InternalUiRendererDiagnostics,
    renderer_mode: InternalUiRendererMode,
    image_cache: TextureCache<ImageTextureKey>,
    text_cache: TextureCache<TextTextureKey>,
}

#[derive(Clone)]
struct CachedTexture {
    buffer: MemoryRenderBuffer,
    width: u32,
    height: u32,
}

struct TextureCacheEntry {
    texture: CachedTexture,
    bytes: usize,
    last_used: u64,
}

struct TextureCache<K> {
    entries: HashMap<K, TextureCacheEntry>,
    bytes: usize,
    clock: u64,
    entry_limit: usize,
    byte_limit: usize,
}

impl<K: Clone + Eq + Hash> TextureCache<K> {
    fn new(entry_limit: usize, byte_limit: usize) -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            clock: 0,
            entry_limit,
            byte_limit,
        }
    }

    fn get(&mut self, key: &K) -> Option<CachedTexture> {
        self.clock = self.clock.saturating_add(1);
        let entry = self.entries.get_mut(key)?;
        entry.last_used = self.clock;
        Some(entry.texture.clone())
    }

    /// Inserts a texture and returns the number of least-recently-used entries
    /// evicted to honor both resource and byte bounds.
    fn insert(&mut self, key: K, texture: CachedTexture, bytes: usize) -> u64 {
        self.clock = self.clock.saturating_add(1);
        if bytes > self.byte_limit {
            return 0;
        }
        if let Some(replaced) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(replaced.bytes);
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.insert(
            key,
            TextureCacheEntry {
                texture,
                bytes,
                last_used: self.clock,
            },
        );
        let mut evictions = 0_u64;
        while self.entries.len() > self.entry_limit || self.bytes > self.byte_limit {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            let removed = self.entries.remove(&oldest);
            self.bytes = self
                .bytes
                .saturating_sub(removed.map_or(0, |entry| entry.bytes));
            evictions = evictions.saturating_add(1);
        }
        evictions
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ImageTextureKey {
    id: u16,
    generation: u64,
    high_density: bool,
    frame_scale: u32,
    width: u32,
    height: u32,
    content_hash: [u8; 32],
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum TextTextureKind {
    Plain {
        text: String,
        command_scale: u32,
        color: u32,
        align: u8,
        bold: bool,
        wrap: bool,
    },
    Styled {
        text: String,
        spans: Vec<nickel_ui::StyledTextSpan>,
        command_scale: u32,
        font_size: Option<u32>,
        color: u32,
        align: u8,
    },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TextTextureKey {
    width: u32,
    height: u32,
    frame_scale: u32,
    kind: TextTextureKind,
}

enum GpuPrimitive {
    Solid(nickel_ui::Rect, SolidColorBuffer),
    Texture {
        rect: nickel_ui::Rect,
        source: Rectangle<f64, Logical>,
        buffer: MemoryRenderBuffer,
    },
}

// Smithay render elements are intentionally simple, but expanding curved or
// gradient geometry into thousands of individual elements makes importing and
// submitting one frame slower than the bounded software fallback. Keep the GPU
// path as the default and fail over before a complex scene can monopolize the
// compositor event loop.
const MAX_GPU_ELEMENTS_PER_SURFACE: usize = 1_024;
const IMAGE_CACHE_ENTRY_LIMIT: usize = 256;
const IMAGE_CACHE_BYTE_LIMIT: usize = 32 * 1024 * 1024;
const TEXT_CACHE_ENTRY_LIMIT: usize = 1_024;
const TEXT_CACHE_BYTE_LIMIT: usize = 8 * 1024 * 1024;

impl SmithayFrameRenderer {
    fn new(width: u32, height: u32, scale: f32, renderer_mode: InternalUiRendererMode) -> Self {
        Self {
            software: SoftwareRenderer::new(width, height, scale),
            // Text commands are rasterized into bounded textures. Reuse one
            // renderer so its process font database and shaping cache survive
            // across every label in a scene; constructing a font system per
            // command can stall the compositor event loop for many seconds.
            text_software: SoftwareRenderer::new(1, 1, scale),
            primitives: Vec::new(),
            raster: None,
            mode: InternalUiPresentationMode::RasterFallback,
            diagnostics: InternalUiRendererDiagnostics::default(),
            renderer_mode,
            image_cache: TextureCache::new(IMAGE_CACHE_ENTRY_LIMIT, IMAGE_CACHE_BYTE_LIMIT),
            text_cache: TextureCache::new(TEXT_CACHE_ENTRY_LIMIT, TEXT_CACHE_BYTE_LIMIT),
        }
    }

    pub fn mode(&self) -> InternalUiPresentationMode {
        self.mode
    }

    pub fn diagnostics(&self) -> InternalUiRendererDiagnostics {
        self.diagnostics
    }

    fn supports_gpu(commands: &[PaintCommand]) -> bool {
        commands.iter().all(|command| {
            matches!(
                command,
                PaintCommand::Fill { .. }
                    | PaintCommand::OverlayFill { .. }
                    | PaintCommand::TopRoundedFill { .. }
                    | PaintCommand::RoundedFill { .. }
                    | PaintCommand::Gradient { .. }
                    | PaintCommand::Stroke { .. }
                    | PaintCommand::OverlayStroke { .. }
                    | PaintCommand::Image { .. }
                    | PaintCommand::Text { .. }
                    | PaintCommand::StyledText { .. }
                    | PaintCommand::PushClip(_)
                    | PaintCommand::PopClip
            )
        })
    }

    fn estimated_gpu_elements(commands: &[PaintCommand]) -> usize {
        commands.iter().fold(0_usize, |total, command| {
            let elements = match command {
                PaintCommand::Fill { .. }
                | PaintCommand::OverlayFill { .. }
                | PaintCommand::Image { .. }
                | PaintCommand::Text { .. }
                | PaintCommand::StyledText { .. } => 1,
                PaintCommand::TopRoundedFill { rect, .. }
                | PaintCommand::RoundedFill { rect, .. } => {
                    rect.size.height.ceil().max(1.0) as usize
                }
                PaintCommand::Gradient { rect, gradient } => match gradient.axis {
                    GradientAxis::Horizontal => rect.size.width.ceil().max(1.0) as usize,
                    GradientAxis::Vertical => rect.size.height.ceil().max(1.0) as usize,
                },
                PaintCommand::Stroke { .. } | PaintCommand::OverlayStroke { .. } => 4,
                PaintCommand::PushClip(_) | PaintCommand::PopClip => 0,
            };
            total.saturating_add(elements)
        })
    }

    fn push_solid(&mut self, rect: nickel_ui::Rect, color: u32, clip: nickel_ui::Rect) {
        let Some(rect) = intersect(rect, clip) else {
            return;
        };
        let size = (
            rect.size.width.ceil().max(1.0) as i32,
            rect.size.height.ceil().max(1.0) as i32,
        );
        self.primitives.push(GpuPrimitive::Solid(
            rect,
            SolidColorBuffer::new(size, color32f(color)),
        ));
    }

    fn push_rounded_solid(
        &mut self,
        rect: nickel_ui::Rect,
        color: u32,
        radius: f32,
        top_only: bool,
        clip: nickel_ui::Rect,
    ) {
        let radius = radius
            .max(0.0)
            .min(rect.size.width / 2.0)
            .min(rect.size.height / 2.0);
        if radius < 0.5 {
            self.push_solid(rect, color, clip);
            return;
        }
        // One logical-pixel strip per row gives the same pixel-centre circle
        // rule as SoftwareRenderer while keeping the result renderer-native.
        let rows = rect.size.height.ceil().max(1.0) as u32;
        for row in 0..rows {
            let y = row as f32;
            let height = (rect.size.height - y).clamp(0.0, 1.0);
            if height == 0.0 {
                continue;
            }
            let sample_y = y + height / 2.0;
            let corner_y = if sample_y < radius {
                Some(radius - sample_y)
            } else if !top_only && sample_y > rect.size.height - radius {
                Some(sample_y - (rect.size.height - radius))
            } else {
                None
            };
            let inset = corner_y
                .map(|dy| radius - (radius * radius - dy * dy).max(0.0).sqrt())
                .unwrap_or(0.0);
            self.push_solid(
                nickel_ui::Rect::new(
                    rect.origin.x + inset,
                    rect.origin.y + y,
                    (rect.size.width - inset * 2.0).max(0.0),
                    height,
                ),
                color,
                clip,
            );
        }
    }

    fn push_gradient(
        &mut self,
        rect: nickel_ui::Rect,
        gradient: LinearGradient,
        clip: nickel_ui::Rect,
    ) {
        let (steps, horizontal) = match gradient.axis {
            GradientAxis::Horizontal => (rect.size.width.ceil().max(1.0) as u32, true),
            GradientAxis::Vertical => (rect.size.height.ceil().max(1.0) as u32, false),
        };
        for step in 0..steps {
            let extent = if horizontal {
                rect.size.width
            } else {
                rect.size.height
            };
            let progress = ((step as f32 + 0.5) / extent.max(1.0)).clamp(0.0, 1.0);
            let color = interpolate_color(gradient.start, gradient.end, progress);
            let strip = if horizontal {
                nickel_ui::Rect::new(
                    rect.origin.x + step as f32,
                    rect.origin.y,
                    (rect.size.width - step as f32).min(1.0),
                    rect.size.height,
                )
            } else {
                nickel_ui::Rect::new(
                    rect.origin.x,
                    rect.origin.y + step as f32,
                    rect.size.width,
                    (rect.size.height - step as f32).min(1.0),
                )
            };
            self.push_solid(strip, color, clip);
        }
    }

    fn prepare_gpu(&mut self, frame: RenderFrame<'_>) {
        self.primitives.clear();
        let viewport = nickel_ui::Rect::new(
            0.0,
            0.0,
            frame.logical_size.0 as f32,
            frame.logical_size.1 as f32,
        );
        let mut clips = vec![viewport];
        for command in frame.commands {
            let clip = *clips.last().unwrap_or(&viewport);
            match command {
                PaintCommand::Fill { rect, color } | PaintCommand::OverlayFill { rect, color } => {
                    self.push_solid(*rect, *color, clip)
                }
                PaintCommand::TopRoundedFill {
                    rect,
                    color,
                    radius,
                } => self.push_rounded_solid(*rect, *color, *radius, true, clip),
                PaintCommand::RoundedFill {
                    rect,
                    color,
                    radius,
                } => self.push_rounded_solid(*rect, *color, *radius, false, clip),
                PaintCommand::Gradient { rect, gradient } => {
                    self.push_gradient(*rect, *gradient, clip)
                }
                PaintCommand::Stroke { rect, color, width }
                | PaintCommand::OverlayStroke { rect, color, width } => {
                    let width = width.max(0.0).min(rect.size.width).min(rect.size.height);
                    self.push_solid(
                        nickel_ui::Rect::new(rect.origin.x, rect.origin.y, rect.size.width, width),
                        *color,
                        clip,
                    );
                    self.push_solid(
                        nickel_ui::Rect::new(
                            rect.origin.x,
                            rect.origin.y + rect.size.height - width,
                            rect.size.width,
                            width,
                        ),
                        *color,
                        clip,
                    );
                    self.push_solid(
                        nickel_ui::Rect::new(
                            rect.origin.x,
                            rect.origin.y + width,
                            width,
                            (rect.size.height - width * 2.0).max(0.0),
                        ),
                        *color,
                        clip,
                    );
                    self.push_solid(
                        nickel_ui::Rect::new(
                            rect.origin.x + rect.size.width - width,
                            rect.origin.y + width,
                            width,
                            (rect.size.height - width * 2.0).max(0.0),
                        ),
                        *color,
                        clip,
                    );
                }
                PaintCommand::PushClip(rect) => clips.push(
                    intersect(clip, *rect)
                        .unwrap_or_else(|| nickel_ui::Rect::new(0.0, 0.0, 0.0, 0.0)),
                ),
                PaintCommand::PopClip => {
                    if clips.len() > 1 {
                        clips.pop();
                    }
                }
                PaintCommand::Text { bounds, .. } | PaintCommand::StyledText { bounds, .. } => {
                    self.push_text_texture(command, *bounds, clip, frame.scale_factor);
                }
                PaintCommand::Image {
                    bounds,
                    id,
                    generation,
                    image,
                    high_density,
                    ..
                } => {
                    let selected_high_density = high_density.is_some() && frame.scale_factor >= 1.5;
                    let image = high_density
                        .as_ref()
                        .filter(|_| frame.scale_factor >= 1.5)
                        .unwrap_or(image);
                    let Some(rect) = intersect(*bounds, clip) else {
                        continue;
                    };
                    if bounds.size.width <= 0.0
                        || bounds.size.height <= 0.0
                        || image.width() == 0
                        || image.height() == 0
                    {
                        continue;
                    }
                    let scale_x = f64::from(image.width()) / f64::from(bounds.size.width);
                    let scale_y = f64::from(image.height()) / f64::from(bounds.size.height);
                    let source = Rectangle::new(
                        (
                            f64::from(rect.origin.x - bounds.origin.x) * scale_x,
                            f64::from(rect.origin.y - bounds.origin.y) * scale_y,
                        )
                            .into(),
                        (
                            f64::from(rect.size.width) * scale_x,
                            f64::from(rect.size.height) * scale_y,
                        )
                            .into(),
                    );
                    let key = ImageTextureKey {
                        id: *id,
                        generation: *generation,
                        high_density: selected_high_density,
                        frame_scale: frame.scale_factor.to_bits(),
                        width: image.width(),
                        height: image.height(),
                        content_hash: content_hash(image.as_raw()),
                    };
                    let texture = if let Some(texture) = self.image_cache.get(&key) {
                        self.diagnostics.image_cache_hits =
                            self.diagnostics.image_cache_hits.saturating_add(1);
                        texture
                    } else {
                        self.diagnostics.image_cache_misses =
                            self.diagnostics.image_cache_misses.saturating_add(1);
                        self.diagnostics.image_allocations =
                            self.diagnostics.image_allocations.saturating_add(1);
                        self.diagnostics.image_uploads =
                            self.diagnostics.image_uploads.saturating_add(1);
                        let texture = CachedTexture {
                            buffer: MemoryRenderBuffer::from_slice(
                                image.as_raw(),
                                Fourcc::Abgr8888,
                                (image.width() as i32, image.height() as i32),
                                1,
                                Transform::Normal,
                                None,
                            ),
                            width: image.width(),
                            height: image.height(),
                        };
                        let bytes = texture_bytes(texture.width, texture.height);
                        self.diagnostics.image_cache_evictions = self
                            .diagnostics
                            .image_cache_evictions
                            .saturating_add(self.image_cache.insert(key, texture.clone(), bytes));
                        texture
                    };
                    self.primitives.push(GpuPrimitive::Texture {
                        rect,
                        source,
                        buffer: texture.buffer,
                    });
                }
            }
        }
        self.refresh_cache_diagnostics();
        self.raster = None;
    }

    fn push_text_texture(
        &mut self,
        command: &PaintCommand,
        bounds: nickel_ui::Rect,
        clip: nickel_ui::Rect,
        scale: f32,
    ) {
        let Some(rect) = intersect(bounds, clip) else {
            return;
        };
        if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
            return;
        }
        let key = text_texture_key(command, bounds, scale);
        if let Some(texture) = self.text_cache.get(&key) {
            self.diagnostics.text_cache_hits = self.diagnostics.text_cache_hits.saturating_add(1);
            let source = text_source_rect(rect, bounds, scale);
            self.primitives.push(GpuPrimitive::Texture {
                rect,
                source,
                buffer: texture.buffer,
            });
            return;
        }
        self.diagnostics.text_cache_misses = self.diagnostics.text_cache_misses.saturating_add(1);
        let mut local = command.clone();
        match &mut local {
            PaintCommand::Text { bounds, .. } | PaintCommand::StyledText { bounds, .. } => {
                bounds.origin = nickel_ui::Point { x: 0.0, y: 0.0 };
            }
            _ => unreachable!("text texture receives a text command"),
        }
        let width = (bounds.size.width * scale).ceil().max(1.0) as u32;
        let height = (bounds.size.height * scale).ceil().max(1.0) as u32;
        self.text_software.resize(width, height, scale);
        self.text_software.invalidate();
        self.text_software.render(&[local]);
        let mut bytes = Vec::with_capacity(self.text_software.pixels().len() * 4);
        for pixel in self.text_software.pixels() {
            bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
        }
        let (physical_width, physical_height) = self.text_software.size();
        let source = text_source_rect(rect, bounds, scale);
        let texture = CachedTexture {
            buffer: MemoryRenderBuffer::from_slice(
                &bytes,
                Fourcc::Abgr8888,
                (physical_width as i32, physical_height as i32),
                1,
                Transform::Normal,
                None,
            ),
            width: physical_width,
            height: physical_height,
        };
        self.diagnostics.text_allocations = self.diagnostics.text_allocations.saturating_add(1);
        self.diagnostics.text_uploads = self.diagnostics.text_uploads.saturating_add(1);
        self.diagnostics.text_cache_evictions = self
            .diagnostics
            .text_cache_evictions
            .saturating_add(self.text_cache.insert(
                key,
                texture.clone(),
                texture_bytes(physical_width, physical_height),
            ));
        self.primitives.push(GpuPrimitive::Texture {
            rect,
            source,
            buffer: texture.buffer,
        });
    }

    fn refresh_cache_diagnostics(&mut self) {
        self.diagnostics.image_cache_entries = self.image_cache.entries.len();
        self.diagnostics.image_cache_bytes = self.image_cache.bytes;
        self.diagnostics.text_cache_entries = self.text_cache.entries.len();
        self.diagnostics.text_cache_bytes = self.text_cache.bytes;
    }

    fn prepare_fallback(&mut self, frame: RenderFrame<'_>) -> DamageRegion {
        let damage = self.software.render(frame.commands);
        let mut bytes = Vec::with_capacity(self.software.pixels().len() * 4);
        for pixel in self.software.pixels() {
            bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
        }
        let (width, height) = self.software.size();
        self.raster = Some(MemoryRenderBuffer::from_slice(
            &bytes,
            Fourcc::Abgr8888,
            (width as i32, height as i32),
            1,
            Transform::Normal,
            None,
        ));
        self.primitives.clear();
        damage
    }

    fn elements<R: Renderer + ImportMem>(
        &self,
        renderer: &mut R,
        location: Point<i32, Logical>,
        logical_size: (u32, u32),
    ) -> Vec<InternalUiRenderElement<R>>
    where
        R::TextureId: Send + Clone + 'static,
    {
        match self.mode {
            InternalUiPresentationMode::GpuSolid => self
                .primitives
                .iter()
                .rev()
                .filter_map(|primitive| match primitive {
                    GpuPrimitive::Solid(rect, buffer) => Some(
                        SolidColorRenderElement::from_buffer(
                            buffer,
                            (
                                location.x + rect.origin.x.round() as i32,
                                location.y + rect.origin.y.round() as i32,
                            ),
                            1.0,
                            1.0,
                            Kind::Unspecified,
                        )
                        .into(),
                    ),
                    GpuPrimitive::Texture {
                        rect,
                        source,
                        buffer,
                    } => MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        (
                            f64::from(location.x) + f64::from(rect.origin.x),
                            f64::from(location.y) + f64::from(rect.origin.y),
                        ),
                        buffer,
                        None,
                        Some(*source),
                        Some(
                            (
                                rect.size.width.ceil() as i32,
                                rect.size.height.ceil() as i32,
                            )
                                .into(),
                        ),
                        Kind::Unspecified,
                    )
                    .ok()
                    .map(Into::into),
                })
                .collect(),
            InternalUiPresentationMode::RasterFallback => self
                .raster
                .as_ref()
                .and_then(|buffer| {
                    MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        (f64::from(location.x), f64::from(location.y)),
                        buffer,
                        None,
                        None,
                        Some((logical_size.0 as i32, logical_size.1 as i32).into()),
                        Kind::Unspecified,
                    )
                    .ok()
                })
                .map(|element| vec![element.into()])
                .unwrap_or_default(),
        }
    }
}

impl FrameRenderer for SmithayFrameRenderer {
    type Error = std::convert::Infallible;

    fn render_frame(&mut self, frame: RenderFrame<'_>) -> Result<DamageRegion, Self::Error> {
        let estimated_gpu_elements = Self::estimated_gpu_elements(frame.commands);
        let damage = if self.renderer_mode == InternalUiRendererMode::Gpu
            && Self::supports_gpu(frame.commands)
            && estimated_gpu_elements <= MAX_GPU_ELEMENTS_PER_SURFACE
        {
            self.mode = InternalUiPresentationMode::GpuSolid;
            self.diagnostics.gpu_frames += 1;
            self.diagnostics.fallback_primitive_count = 0;
            self.diagnostics.fallback_text_count = 0;
            self.diagnostics.fallback_image_count = 0;
            self.prepare_gpu(frame);
            DamageRegion {
                rects: [nickel_ui::Rect::new(
                    0.0,
                    0.0,
                    frame.logical_size.0 as f32,
                    frame.logical_size.1 as f32,
                )]
                .into_iter()
                .collect(),
            }
        } else {
            self.mode = InternalUiPresentationMode::RasterFallback;
            self.diagnostics.fallback_frames += 1;
            self.diagnostics.fallback_text_count = frame
                .commands
                .iter()
                .filter(|command| {
                    matches!(
                        command,
                        PaintCommand::Text { .. } | PaintCommand::StyledText { .. }
                    )
                })
                .count();
            self.diagnostics.fallback_image_count = frame
                .commands
                .iter()
                .filter(|command| matches!(command, PaintCommand::Image { .. }))
                .count();
            self.diagnostics.fallback_primitive_count =
                if estimated_gpu_elements > MAX_GPU_ELEMENTS_PER_SURFACE {
                    estimated_gpu_elements
                } else {
                    self.diagnostics.fallback_text_count + self.diagnostics.fallback_image_count
                };
            self.prepare_fallback(frame)
        };
        Ok(damage)
    }
}

fn content_hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn texture_bytes(width: u32, height: u32) -> usize {
    (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4)
}

fn text_align_key(align: nickel_ui::TextAlign) -> u8 {
    match align {
        nickel_ui::TextAlign::Start => 0,
        nickel_ui::TextAlign::Center => 1,
        nickel_ui::TextAlign::End => 2,
    }
}

fn text_texture_key(
    command: &PaintCommand,
    bounds: nickel_ui::Rect,
    frame_scale: f32,
) -> TextTextureKey {
    let kind = match command {
        PaintCommand::Text {
            text,
            scale,
            color,
            align,
            bold,
            wrap,
            ..
        } => TextTextureKind::Plain {
            text: text.clone(),
            command_scale: scale.to_bits(),
            color: *color,
            align: text_align_key(*align),
            bold: *bold,
            wrap: *wrap,
        },
        PaintCommand::StyledText {
            text,
            spans,
            scale,
            font_size,
            color,
            align,
            ..
        } => TextTextureKind::Styled {
            text: text.clone(),
            spans: spans.clone(),
            command_scale: scale.to_bits(),
            font_size: font_size.map(f32::to_bits),
            color: *color,
            align: text_align_key(*align),
        },
        _ => unreachable!("text texture key receives a text command"),
    };
    TextTextureKey {
        width: bounds.size.width.to_bits(),
        height: bounds.size.height.to_bits(),
        frame_scale: frame_scale.to_bits(),
        kind,
    }
}

fn text_source_rect(
    rect: nickel_ui::Rect,
    bounds: nickel_ui::Rect,
    scale: f32,
) -> Rectangle<f64, Logical> {
    Rectangle::new(
        (
            f64::from(rect.origin.x - bounds.origin.x) * f64::from(scale),
            f64::from(rect.origin.y - bounds.origin.y) * f64::from(scale),
        )
            .into(),
        (
            f64::from(rect.size.width) * f64::from(scale),
            f64::from(rect.size.height) * f64::from(scale),
        )
            .into(),
    )
}

fn intersect(left: nickel_ui::Rect, right: nickel_ui::Rect) -> Option<nickel_ui::Rect> {
    let x = left.origin.x.max(right.origin.x);
    let y = left.origin.y.max(right.origin.y);
    let right_edge = (left.origin.x + left.size.width).min(right.origin.x + right.size.width);
    let bottom = (left.origin.y + left.size.height).min(right.origin.y + right.size.height);
    (right_edge > x && bottom > y).then(|| nickel_ui::Rect::new(x, y, right_edge - x, bottom - y))
}

fn color32f(color: u32) -> Color32F {
    let alpha = if color <= 0x00ff_ffff {
        0xff
    } else {
        (color >> 24) & 0xff
    };
    Color32F::new(
        ((color >> 16) & 0xff) as f32 / 255.0,
        ((color >> 8) & 0xff) as f32 / 255.0,
        (color & 0xff) as f32 / 255.0,
        alpha as f32 / 255.0,
    )
}

fn interpolate_color(start: u32, end: u32, progress: f32) -> u32 {
    let channels = |color: u32| {
        let alpha = if color <= 0x00ff_ffff {
            0xff
        } else {
            (color >> 24) & 0xff
        };
        [
            alpha,
            (color >> 16) & 0xff,
            (color >> 8) & 0xff,
            color & 0xff,
        ]
    };
    let start = channels(start);
    let end = channels(end);
    let mut result = 0;
    for (shift, channel) in [24, 16, 8, 0].into_iter().zip(0..4) {
        let value =
            start[channel] as f32 + (end[channel] as f32 - start[channel] as f32) * progress;
        result |= (value.round() as u32) << shift;
    }
    result
}

/// Session-owned applications and their compositor presentation state.
pub struct InternalUiRuntime {
    surfaces: InternalSurfaceSet,
    presentation: BTreeMap<InternalSurfaceId, PresentedSurface>,
    focused: Option<InternalSurfaceId>,
    hovered: Option<InternalSurfaceId>,
    touches: BTreeMap<u64, (InternalSurfaceId, UiPoint)>,
    routed_events: Vec<(InternalSurfaceId, UiEvent)>,
    renderer_mode: InternalUiRendererMode,
}

impl Default for InternalUiRuntime {
    fn default() -> Self {
        Self {
            surfaces: InternalSurfaceSet::default(),
            presentation: BTreeMap::new(),
            focused: None,
            hovered: None,
            touches: BTreeMap::new(),
            routed_events: Vec::new(),
            renderer_mode: InternalUiRendererMode::Gpu,
        }
    }
}

impl InternalUiRuntime {
    pub fn insert_boxed(
        &mut self,
        surface: Box<dyn nickel_ui::InternalUiSurface>,
        placement: InternalSurfacePlacement,
        scale: f32,
    ) -> InternalSurfaceId {
        let (_, _, width, height) = placement.geometry;
        let id = self.surfaces.insert_boxed(surface);
        self.presentation.insert(
            id,
            PresentedSurface {
                placement,
                renderer: SmithayFrameRenderer::new(width, height, scale, self.renderer_mode),
                dirty: true,
                external_scene: None,
                scale_factor: scale,
            },
        );
        id
    }

    pub fn application<T: 'static>(&self, id: InternalSurfaceId) -> Option<&T> {
        self.surfaces.get(id)?.application().downcast_ref()
    }

    /// Mutably access an application hosted by the compositor.
    pub fn application_mut<T: 'static>(&mut self, id: InternalSurfaceId) -> Option<&mut T> {
        self.surfaces.get_mut(id)?.application_mut().downcast_mut()
    }

    /// Earliest application-owned wakeup across compositor surfaces.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.surfaces.next_deadline()
    }

    pub fn surface_deadline(&self, id: InternalSurfaceId) -> Option<Instant> {
        self.surfaces.get(id)?.next_deadline()
    }

    /// Poll one surface when its application deadline has elapsed.
    pub fn poll_surface(&mut self, id: InternalSurfaceId, now: Instant) -> bool {
        if self
            .surface_deadline(id)
            .is_none_or(|deadline| deadline > now)
        {
            return false;
        }
        self.step(
            id,
            HostBatch {
                now: Some(now),
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            },
        )
    }

    /// Poll every due surface and return those which produced a new frame.
    pub fn poll_due(&mut self, now: Instant) -> Vec<InternalSurfaceId> {
        let ids = self.surfaces.ids().collect::<Vec<_>>();
        ids.into_iter()
            .filter(|id| self.poll_surface(*id, now))
            .collect()
    }

    pub fn focus_surface(&mut self, id: InternalSurfaceId) -> bool {
        if !self.presentation.contains_key(&id) {
            return false;
        }
        if self.focused != Some(id) {
            if let Some(previous) = self.focused {
                self.step(
                    previous,
                    HostBatch {
                        window_focused: Some(false),
                        ..Default::default()
                    },
                );
            }
            self.focused = Some(id);
            self.step(
                id,
                HostBatch {
                    window_focused: Some(true),
                    ..Default::default()
                },
            );
        }
        true
    }
    pub fn insert<A: Application + 'static>(
        &mut self,
        application: A,
        placement: InternalSurfacePlacement,
        scale: f32,
    ) -> InternalSurfaceId {
        let (_, _, width, height) = placement.geometry;
        let id = self.surfaces.insert(application, width, height);
        let physical_width = ((width as f32) * scale).round().max(1.0) as u32;
        let physical_height = ((height as f32) * scale).round().max(1.0) as u32;
        self.presentation.insert(
            id,
            PresentedSurface {
                placement,
                renderer: SmithayFrameRenderer::new(
                    physical_width,
                    physical_height,
                    scale,
                    self.renderer_mode,
                ),
                dirty: true,
                external_scene: None,
                scale_factor: scale,
            },
        );
        id
    }

    /// Insert a renderer-neutral scene produced by compositor-owned shell state.
    pub fn insert_scene(
        &mut self,
        commands: Vec<PaintCommand>,
        placement: InternalSurfacePlacement,
        scale: f32,
    ) -> InternalSurfaceId {
        let id = self.insert(SceneSlot, placement, scale);
        self.presentation.get_mut(&id).unwrap().external_scene = Some(commands);
        id
    }

    pub fn remove(&mut self, id: InternalSurfaceId) -> bool {
        let removed = self.surfaces.remove(id).is_some();
        self.presentation.remove(&id);
        if self.focused == Some(id) {
            self.focused = None;
        }
        if self.hovered == Some(id) {
            self.hovered = None;
        }
        self.touches.retain(|_, (target, _)| *target != id);
        removed
    }

    pub fn placement(&self, id: InternalSurfaceId) -> Option<&InternalSurfacePlacement> {
        self.presentation.get(&id).map(|surface| &surface.placement)
    }

    pub fn step(&mut self, id: InternalSurfaceId, batch: HostBatch) -> bool {
        let Some(surface) = self.surfaces.get_mut(id) else {
            return false;
        };
        let changed = surface.step(batch).changed;
        if changed && let Some(presentation) = self.presentation.get_mut(&id) {
            presentation.dirty = true;
        }
        changed
    }

    pub fn update_scene(&mut self, id: InternalSurfaceId, commands: Vec<PaintCommand>) -> bool {
        let Some(surface) = self.presentation.get_mut(&id) else {
            return false;
        };
        surface.external_scene = Some(commands);
        surface.dirty = true;
        true
    }

    pub fn drain_routed_events(&mut self) -> Vec<(InternalSurfaceId, UiEvent)> {
        std::mem::take(&mut self.routed_events)
    }

    pub fn mark_dirty(&mut self, id: InternalSurfaceId) {
        if let Some(surface) = self.presentation.get_mut(&id) {
            surface.dirty = true;
        }
    }

    pub fn presentation_mode(&self, id: InternalSurfaceId) -> Option<InternalUiPresentationMode> {
        self.presentation
            .get(&id)
            .map(|surface| surface.renderer.mode())
    }

    pub fn renderer_diagnostics(
        &self,
        id: InternalSurfaceId,
    ) -> Option<InternalUiRendererDiagnostics> {
        self.presentation
            .get(&id)
            .map(|surface| surface.renderer.diagnostics())
    }

    /// Force the compositor-owned UI renderer used by both native and nested backends.
    /// Existing surfaces are invalidated so the new mode is visible on the next frame.
    pub fn set_renderer_mode(&mut self, mode: InternalUiRendererMode) {
        self.renderer_mode = mode;
        for surface in self.presentation.values_mut() {
            surface.renderer.renderer_mode = mode;
            surface.dirty = true;
        }
    }

    pub fn has_damage(&self) -> bool {
        self.presentation.values().any(|surface| surface.dirty)
    }

    pub fn focused(&self) -> Option<InternalSurfaceId> {
        self.focused
    }

    fn role_order(role: InternalSurfaceRole) -> u8 {
        match role {
            InternalSurfaceRole::Desktop => 0,
            InternalSurfaceRole::Application => 1,
            InternalSurfaceRole::Panel => 2,
            InternalSurfaceRole::Overlay => 3,
        }
    }

    fn role_layer(role: InternalSurfaceRole) -> InternalSurfaceLayer {
        match role {
            InternalSurfaceRole::Desktop => InternalSurfaceLayer::Background,
            InternalSurfaceRole::Application
            | InternalSurfaceRole::Panel
            | InternalSurfaceRole::Overlay => InternalSurfaceLayer::Overlay,
        }
    }

    /// Return the foremost internal surface at `point`.
    ///
    /// A desktop is below the Wayland client scene, so it is excluded when a
    /// client occupies the point. Other internal roles are compositor-owned
    /// foreground surfaces and retain priority over clients.
    pub fn surface_at(
        &self,
        point: (f64, f64),
        client_present: bool,
    ) -> Option<(InternalSurfaceId, UiPoint)> {
        self.presentation
            .iter()
            .filter_map(|(id, surface)| {
                let (x, y, width, height) = surface.placement.geometry;
                (!(client_present && surface.placement.role == InternalSurfaceRole::Desktop)
                    && point.0 >= f64::from(x)
                    && point.1 >= f64::from(y)
                    && point.0 < f64::from(x) + f64::from(width)
                    && point.1 < f64::from(y) + f64::from(height))
                .then_some((
                    Self::role_order(surface.placement.role),
                    *id,
                    UiPoint {
                        x: (point.0 - f64::from(x)) as f32,
                        y: (point.1 - f64::from(y)) as f32,
                    },
                ))
            })
            .max_by_key(|(role, id, _)| (*role, *id))
            .map(|(_, id, local)| (id, local))
    }

    fn dispatch_ui(&mut self, id: InternalSurfaceId, event: UiEvent) -> bool {
        self.routed_events.push((id, event.clone()));
        if self
            .presentation
            .get(&id)
            .is_some_and(|surface| surface.external_scene.is_some())
        {
            // Renderer-neutral scenes are owned by the shell coordinator. It
            // consumes this event after routing and supplies the next scene;
            // the identity-only SceneSlot must never reduce input itself.
            return true;
        }
        self.step(
            id,
            HostBatch {
                events: vec![HostEvent::Ui(event)],
                ..Default::default()
            },
        )
    }

    pub fn pointer_motion(&mut self, point: (f64, f64)) -> bool {
        self.pointer_motion_with_client(point, false)
    }

    pub fn pointer_motion_with_client(&mut self, point: (f64, f64), client_present: bool) -> bool {
        let target = self.surface_at(point, client_present);
        if self.hovered != target.map(|(id, _)| id) {
            if let Some(previous) = self.hovered {
                self.dispatch_ui(previous, UiEvent::PointerCancelled);
            }
            self.hovered = target.map(|(id, _)| id);
        }
        if let Some((id, local)) = target {
            self.dispatch_ui(id, UiEvent::PointerMoved(local));
            true
        } else {
            false
        }
    }

    pub fn pointer_button(&mut self, point: (f64, f64), pressed: bool) -> bool {
        self.pointer_button_with_client(point, pressed, false)
    }

    pub fn pointer_button_with_client(
        &mut self,
        point: (f64, f64),
        pressed: bool,
        client_present: bool,
    ) -> bool {
        let Some((id, local)) = self.surface_at(point, client_present) else {
            if pressed && let Some(previous) = self.focused.take() {
                self.step(
                    previous,
                    HostBatch {
                        window_focused: Some(false),
                        ..Default::default()
                    },
                );
            }
            return false;
        };
        if pressed {
            if self.focused != Some(id) {
                if let Some(previous) = self.focused {
                    self.step(
                        previous,
                        HostBatch {
                            window_focused: Some(false),
                            ..Default::default()
                        },
                    );
                }
                self.focused = Some(id);
                self.step(
                    id,
                    HostBatch {
                        window_focused: Some(true),
                        ..Default::default()
                    },
                );
            }
            self.dispatch_ui(id, UiEvent::PointerPressed(local));
        } else {
            self.dispatch_ui(id, UiEvent::PointerReleased(local));
        }
        true
    }

    pub fn scroll(&mut self, point: (f64, f64), horizontal: f32, vertical: f32) -> bool {
        self.scroll_with_client(point, horizontal, vertical, false)
    }

    pub fn scroll_with_client(
        &mut self,
        point: (f64, f64),
        horizontal: f32,
        vertical: f32,
        client_present: bool,
    ) -> bool {
        let Some((id, local)) = self.surface_at(point, client_present) else {
            return false;
        };
        if horizontal != 0.0 {
            self.dispatch_ui(
                id,
                UiEvent::ScrollHorizontal {
                    point: local,
                    delta_x: horizontal,
                },
            );
        }
        if vertical != 0.0 {
            self.dispatch_ui(
                id,
                UiEvent::Scroll {
                    point: local,
                    delta_y: vertical,
                },
            );
        }
        true
    }

    pub fn touch(&mut self, contact: u64, point: (f64, f64), phase: TouchPhase) -> bool {
        self.touch_with_client(contact, point, phase, false)
    }

    pub fn touch_with_client(
        &mut self,
        contact: u64,
        point: (f64, f64),
        phase: TouchPhase,
        client_present: bool,
    ) -> bool {
        match phase {
            TouchPhase::Started => {
                let Some((id, local)) = self.surface_at(point, client_present) else {
                    return false;
                };
                self.touches.insert(contact, (id, local));
                if self.focused != Some(id) {
                    if let Some(previous) = self.focused {
                        self.step(
                            previous,
                            HostBatch {
                                window_focused: Some(false),
                                ..Default::default()
                            },
                        );
                    }
                    self.focused = Some(id);
                    self.step(
                        id,
                        HostBatch {
                            window_focused: Some(true),
                            ..Default::default()
                        },
                    );
                }
                self.dispatch_ui(id, UiEvent::PointerPressed(local));
                true
            }
            TouchPhase::Moved => self.touches.get(&contact).copied().is_some_and(|(id, _)| {
                let Some(surface) = self.presentation.get(&id) else {
                    return false;
                };
                let (x, y, _, _) = surface.placement.geometry;
                let local = UiPoint {
                    x: (point.0 - f64::from(x)) as f32,
                    y: (point.1 - f64::from(y)) as f32,
                };
                self.touches.insert(contact, (id, local));
                self.dispatch_ui(id, UiEvent::PointerMoved(local));
                true
            }),
            TouchPhase::Ended => self.touches.remove(&contact).is_some_and(|(id, local)| {
                self.dispatch_ui(id, UiEvent::PointerReleased(local));
                true
            }),
            TouchPhase::Cancelled => self
                .touches
                .remove(&contact)
                .is_some_and(|(id, _)| self.dispatch_ui(id, UiEvent::PointerCancelled)),
        }
    }

    pub fn keyboard(&mut self, event: UiEvent) -> bool {
        self.focused.is_some_and(|id| {
            self.dispatch_ui(id, event);
            true
        })
    }

    pub fn cancel_touches(&mut self) -> bool {
        let targets = std::mem::take(&mut self.touches);
        let mut handled = false;
        for (_, (id, _)) in targets {
            handled |= self.dispatch_ui(id, UiEvent::PointerCancelled);
        }
        handled
    }

    pub fn ids_for_output<'a>(
        &'a self,
        output: &'a str,
    ) -> impl Iterator<Item = InternalSurfaceId> + 'a {
        self.presentation.iter().filter_map(move |(id, surface)| {
            surface
                .placement
                .output
                .as_deref()
                .is_none_or(|name| name == output)
                .then_some(*id)
        })
    }

    fn ordered_ids_for_layer(
        &self,
        output: &str,
        layer: Option<InternalSurfaceLayer>,
    ) -> Vec<InternalSurfaceId> {
        let mut ids = self
            .ids_for_output(output)
            .filter(|id| {
                layer.is_none_or(|layer| {
                    self.presentation
                        .get(id)
                        .is_some_and(|surface| Self::role_layer(surface.placement.role) == layer)
                })
            })
            .collect::<Vec<_>>();
        ids.sort_by_key(|id| {
            let role = self.presentation.get(id).unwrap().placement.role;
            (
                std::cmp::Reverse(Self::role_order(role)),
                std::cmp::Reverse(*id),
            )
        });
        ids
    }

    /// Prepare a dirty surface and return its Smithay-importable fallback buffer.
    ///
    /// `None` after preparation means the frame is represented by GPU-native
    /// solid elements and should be obtained through [`Self::render_elements`].
    pub fn render_buffer(&mut self, id: InternalSurfaceId) -> Option<MemoryRenderBuffer> {
        let presentation = self.presentation.get_mut(&id)?;
        if presentation.dirty {
            if let Some(commands) = &presentation.external_scene {
                let (_, _, width, height) = presentation.placement.geometry;
                let _ = presentation.renderer.render_frame(RenderFrame {
                    commands,
                    logical_size: (width, height),
                    scale_factor: presentation.scale_factor,
                    generation: 0,
                });
            } else {
                let surface = self.surfaces.get(id)?;
                let _ = presentation.renderer.render_frame(surface.render_frame());
            }
            presentation.dirty = false;
        }
        presentation.renderer.raster.clone()
    }

    /// Build render elements in output-local coordinates for the backend's current renderer.
    pub fn render_elements<R: Renderer + ImportMem>(
        &mut self,
        renderer: &mut R,
        output: &str,
        output_origin: Point<i32, Logical>,
    ) -> Vec<InternalUiRenderElement<R>>
    where
        R::TextureId: Send + Clone + 'static,
    {
        self.render_elements_for_layer(renderer, output, output_origin, None)
    }

    /// Build front-to-back render elements for one compositor scene layer.
    pub fn render_elements_for_layer<R: Renderer + ImportMem>(
        &mut self,
        renderer: &mut R,
        output: &str,
        output_origin: Point<i32, Logical>,
        layer: Option<InternalSurfaceLayer>,
    ) -> Vec<InternalUiRenderElement<R>>
    where
        R::TextureId: Send + Clone + 'static,
    {
        self.ordered_ids_for_layer(output, layer)
            .into_iter()
            .filter_map(|id| {
                let placement = self.presentation.get(&id)?.placement.clone();
                if self.presentation.get(&id)?.dirty {
                    let presentation = self.presentation.get_mut(&id)?;
                    if let Some(commands) = &presentation.external_scene {
                        let (_, _, width, height) = placement.geometry;
                        let _ = presentation.renderer.render_frame(RenderFrame {
                            commands,
                            logical_size: (width, height),
                            scale_factor: presentation.scale_factor,
                            generation: 0,
                        });
                    } else {
                        let surface = self.surfaces.get(id)?;
                        let _ = presentation.renderer.render_frame(surface.render_frame());
                    }
                    presentation.dirty = false;
                }
                let presentation = self.presentation.get(&id)?;
                let local = output_local_location(placement.geometry, output_origin);
                Some(presentation.renderer.elements(
                    renderer,
                    (local.0.round() as i32, local.1.round() as i32).into(),
                    (placement.geometry.2, placement.geometry.3),
                ))
            })
            .flatten()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.presentation.len()
    }
    pub fn is_empty(&self) -> bool {
        self.presentation.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TouchPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_ui::{Button, Text, View, ViewContext};

    struct Label;
    impl Application for Label {
        type Message = ();
        fn update(&mut self, (): ()) {}
        fn view(&self, _: ViewContext) -> impl View<Self::Message> {
            Text::new("owned")
        }
    }

    fn placement(output: Option<&str>) -> InternalSurfacePlacement {
        InternalSurfacePlacement {
            role: InternalSurfaceRole::Panel,
            geometry: (10, 20, 120, 32),
            output: output.map(str::to_owned),
        }
    }

    #[test]
    fn lifecycle_keeps_application_and_presentation_identity_together() {
        let mut runtime = InternalUiRuntime::default();
        let id = runtime.insert(Label, placement(Some("DP-1")), 1.0);
        assert_eq!(runtime.ids_for_output("DP-1").collect::<Vec<_>>(), vec![id]);
        assert!(runtime.ids_for_output("HDMI-A-1").next().is_none());
        assert!(runtime.has_damage());
        assert!(runtime.render_buffer(id).is_none());
        assert_eq!(
            runtime.presentation.get(&id).unwrap().renderer.mode(),
            InternalUiPresentationMode::GpuSolid
        );
        assert!(!runtime.has_damage());
        assert!(runtime.remove(id));
        assert!(runtime.is_empty());
    }

    #[test]
    fn output_agnostic_surfaces_are_visible_on_every_output() {
        let mut runtime = InternalUiRuntime::default();
        let id = runtime.insert(Label, placement(None), 1.0);
        assert_eq!(runtime.ids_for_output("DP-1").next(), Some(id));
        assert_eq!(runtime.ids_for_output("HDMI-A-1").next(), Some(id));
    }

    #[test]
    fn global_placement_is_translated_to_output_local_logical_coordinates() {
        let origin = Point::<i32, Logical>::from((1920, -120));

        assert_eq!(
            output_local_location((1936, -88, 480, 64), origin),
            (16.0, 32.0)
        );
    }

    struct Counter(usize);
    impl Application for Counter {
        type Message = ();
        fn update(&mut self, (): ()) {
            self.0 += 1;
        }
        fn view(&self, _: ViewContext) -> impl View<Self::Message> {
            Button::new((), "count")
        }
    }

    #[test]
    fn hit_testing_respects_system_role_stack_and_focus_routes_keyboard() {
        let mut runtime = InternalUiRuntime::default();
        let application = runtime.insert(
            Counter(0),
            InternalSurfacePlacement {
                role: InternalSurfaceRole::Application,
                geometry: (0, 0, 100, 100),
                output: None,
            },
            1.0,
        );
        let panel = runtime.insert(
            Counter(0),
            InternalSurfacePlacement {
                role: InternalSurfaceRole::Panel,
                geometry: (0, 0, 100, 40),
                output: None,
            },
            1.0,
        );

        assert_eq!(
            runtime.surface_at((10.0, 10.0), false).map(|hit| hit.0),
            Some(panel)
        );
        assert!(runtime.pointer_button((10.0, 60.0), true));
        assert_eq!(runtime.focused(), Some(application));
        assert!(runtime.keyboard(UiEvent::KeyboardActivate));
        assert_eq!(
            runtime
                .surfaces
                .get(application)
                .unwrap()
                .application()
                .downcast_ref::<Counter>()
                .unwrap()
                .0,
            1
        );
        assert_eq!(
            runtime
                .surfaces
                .get(panel)
                .unwrap()
                .application()
                .downcast_ref::<Counter>()
                .unwrap()
                .0,
            0
        );
    }

    #[test]
    fn desktop_is_below_clients_but_system_surfaces_remain_above_them() {
        let mut runtime = InternalUiRuntime::default();
        let desktop = runtime.insert(
            Label,
            InternalSurfacePlacement {
                role: InternalSurfaceRole::Desktop,
                geometry: (0, 0, 200, 200),
                output: None,
            },
            1.0,
        );
        let panel = runtime.insert(
            Label,
            InternalSurfacePlacement {
                role: InternalSurfaceRole::Panel,
                geometry: (0, 0, 200, 32),
                output: None,
            },
            1.0,
        );

        assert_eq!(runtime.surface_at((50.0, 80.0), false).unwrap().0, desktop);
        assert!(runtime.surface_at((50.0, 80.0), true).is_none());
        assert_eq!(runtime.surface_at((50.0, 16.0), true).unwrap().0, panel);
    }

    #[test]
    fn render_layers_are_front_to_back_and_match_hit_test_roles() {
        let mut runtime = InternalUiRuntime::default();
        let desktop = runtime.insert(
            Label,
            InternalSurfacePlacement {
                role: InternalSurfaceRole::Desktop,
                geometry: (0, 0, 100, 100),
                output: Some("nested".into()),
            },
            1.0,
        );
        let application = runtime.insert(
            Label,
            InternalSurfacePlacement {
                role: InternalSurfaceRole::Application,
                geometry: (0, 0, 100, 100),
                output: Some("nested".into()),
            },
            1.0,
        );
        let panel = runtime.insert(
            Label,
            InternalSurfacePlacement {
                role: InternalSurfaceRole::Panel,
                geometry: (0, 0, 100, 100),
                output: Some("nested".into()),
            },
            1.0,
        );
        let overlay = runtime.insert(
            Label,
            InternalSurfacePlacement {
                role: InternalSurfaceRole::Overlay,
                geometry: (0, 0, 100, 100),
                output: Some("nested".into()),
            },
            1.0,
        );

        assert_eq!(
            runtime.ordered_ids_for_layer("nested", Some(InternalSurfaceLayer::Background)),
            vec![desktop]
        );
        assert_eq!(
            runtime.ordered_ids_for_layer("nested", Some(InternalSurfaceLayer::Overlay)),
            vec![overlay, panel, application]
        );
        assert_eq!(runtime.surface_at((10.0, 10.0), true).unwrap().0, overlay);
    }

    #[test]
    fn renderer_neutral_scene_uses_the_same_presentation_lifecycle() {
        let mut runtime = InternalUiRuntime::default();
        let id = runtime.insert_scene(Vec::new(), placement(Some("nested")), 1.0);
        assert!(runtime.has_damage());
        assert!(runtime.render_buffer(id).is_none());
        assert_eq!(
            runtime.presentation_mode(id),
            Some(InternalUiPresentationMode::GpuSolid)
        );
        assert!(!runtime.has_damage());
    }

    #[test]
    fn input_outside_internal_surfaces_is_left_for_wayland_clients() {
        let mut runtime = InternalUiRuntime::default();
        runtime.insert(Label, placement(Some("DP-1")), 1.0);
        assert!(!runtime.pointer_motion((500.0, 500.0)));
        assert!(!runtime.pointer_button((500.0, 500.0), true));
        assert!(!runtime.scroll((500.0, 500.0), 0.0, 1.0));
    }

    #[test]
    fn rectangular_display_list_selects_gpu_solids_and_honors_clip() {
        let commands = [
            PaintCommand::PushClip(nickel_ui::Rect::new(5.0, 4.0, 20.0, 10.0)),
            PaintCommand::Fill {
                rect: nickel_ui::Rect::new(0.0, 0.0, 40.0, 30.0),
                color: 0xff336699,
            },
            PaintCommand::PopClip,
        ];
        let mut renderer = SmithayFrameRenderer::new(40, 30, 1.0, InternalUiRendererMode::Gpu);

        renderer
            .render_frame(RenderFrame {
                commands: &commands,
                logical_size: (40, 30),
                scale_factor: 1.0,
                generation: 1,
            })
            .unwrap();

        assert_eq!(renderer.mode(), InternalUiPresentationMode::GpuSolid);
        assert_eq!(renderer.primitives.len(), 1);
        let GpuPrimitive::Solid(rect, _) = &renderer.primitives[0] else {
            panic!("fill should produce a solid")
        };
        assert_eq!(*rect, nickel_ui::Rect::new(5.0, 4.0, 20.0, 10.0));
        assert!(renderer.raster.is_none());
        assert_eq!(renderer.diagnostics().gpu_frames, 1);
    }

    #[test]
    fn explicit_software_mode_rasterizes_a_gpu_supported_frame() {
        let commands = [PaintCommand::Fill {
            rect: nickel_ui::Rect::new(0.0, 0.0, 40.0, 30.0),
            color: 0xff336699,
        }];
        let mut renderer = SmithayFrameRenderer::new(40, 30, 1.0, InternalUiRendererMode::Software);

        renderer
            .render_frame(RenderFrame {
                commands: &commands,
                logical_size: (40, 30),
                scale_factor: 1.0,
                generation: 1,
            })
            .unwrap();

        assert_eq!(renderer.mode(), InternalUiPresentationMode::RasterFallback);
        assert!(renderer.primitives.is_empty());
        assert!(renderer.raster.is_some());
        assert_eq!(renderer.diagnostics().gpu_frames, 0);
        assert_eq!(renderer.diagnostics().fallback_frames, 1);
    }

    #[test]
    fn rounded_fill_stays_on_gpu_as_scanline_solids() {
        let commands = [PaintCommand::RoundedFill {
            rect: nickel_ui::Rect::new(0.0, 0.0, 20.0, 12.0),
            color: 0x336699,
            radius: 4.0,
        }];
        let mut renderer = SmithayFrameRenderer::new(20, 12, 1.0, InternalUiRendererMode::Gpu);

        renderer
            .render_frame(RenderFrame {
                commands: &commands,
                logical_size: (20, 12),
                scale_factor: 1.0,
                generation: 1,
            })
            .unwrap();

        assert_eq!(renderer.mode(), InternalUiPresentationMode::GpuSolid);
        assert!(renderer.raster.is_none());
        assert_eq!(renderer.primitives.len(), 12);
        let GpuPrimitive::Solid(first, _) = &renderer.primitives[0] else {
            panic!("rounded row should be a solid")
        };
        let GpuPrimitive::Solid(middle, _) = &renderer.primitives[6] else {
            panic!("rounded row should be a solid")
        };
        assert!(first.size.width < 20.0);
        assert_eq!(middle.size.width, 20.0);
    }

    #[test]
    fn element_heavy_scene_uses_bounded_fallback() {
        let commands = [PaintCommand::RoundedFill {
            rect: nickel_ui::Rect::new(0.0, 0.0, 20.0, (MAX_GPU_ELEMENTS_PER_SURFACE + 1) as f32),
            color: 0x336699,
            radius: 4.0,
        }];
        let mut renderer = SmithayFrameRenderer::new(
            20,
            (MAX_GPU_ELEMENTS_PER_SURFACE + 1) as u32,
            1.0,
            InternalUiRendererMode::Gpu,
        );

        renderer
            .render_frame(RenderFrame {
                commands: &commands,
                logical_size: (20, (MAX_GPU_ELEMENTS_PER_SURFACE + 1) as u32),
                scale_factor: 1.0,
                generation: 1,
            })
            .unwrap();

        assert_eq!(renderer.mode(), InternalUiPresentationMode::RasterFallback);
        assert!(renderer.primitives.is_empty());
        assert!(renderer.raster.is_some());
        assert_eq!(renderer.diagnostics().fallback_frames, 1);
        assert_eq!(
            renderer.diagnostics().fallback_primitive_count,
            MAX_GPU_ELEMENTS_PER_SURFACE + 1
        );
    }

    #[test]
    fn text_uses_a_bounded_texture_without_full_surface_fallback() {
        let commands = [PaintCommand::Text {
            bounds: nickel_ui::Rect::new(0.0, 0.0, 20.0, 12.0),
            text: "Nickel".into(),
            scale: 1.0,
            color: 0x336699,
            align: nickel_ui::TextAlign::Start,
            bold: false,
            wrap: false,
        }];
        let mut renderer = SmithayFrameRenderer::new(40, 24, 2.0, InternalUiRendererMode::Gpu);

        renderer
            .render_frame(RenderFrame {
                commands: &commands,
                logical_size: (20, 12),
                scale_factor: 2.0,
                generation: 1,
            })
            .unwrap();

        assert_eq!(renderer.mode(), InternalUiPresentationMode::GpuSolid);
        let GpuPrimitive::Texture { source, .. } = &renderer.primitives[0] else {
            panic!("text should produce a texture")
        };
        assert_eq!(source.size, (40.0, 24.0).into());
        assert!(renderer.raster.is_none());
        assert_eq!(renderer.diagnostics().fallback_text_count, 0);
        assert_eq!(renderer.diagnostics().fallback_image_count, 0);
    }

    #[test]
    fn clipped_image_uses_cropped_texture_coordinates_without_surface_fallback() {
        use std::sync::Arc;

        let image = Arc::new(image::RgbaImage::new(40, 20));
        let commands = [
            PaintCommand::PushClip(nickel_ui::Rect::new(15.0, 8.0, 10.0, 5.0)),
            PaintCommand::Image {
                bounds: nickel_ui::Rect::new(10.0, 5.0, 20.0, 10.0),
                id: 3,
                generation: 1,
                image,
                high_density: None,
            },
            PaintCommand::PopClip,
        ];
        let mut renderer = SmithayFrameRenderer::new(40, 20, 1.0, InternalUiRendererMode::Gpu);
        renderer
            .render_frame(RenderFrame {
                commands: &commands,
                logical_size: (40, 20),
                scale_factor: 1.0,
                generation: 1,
            })
            .unwrap();

        assert_eq!(renderer.mode(), InternalUiPresentationMode::GpuSolid);
        let GpuPrimitive::Texture { rect, source, .. } = &renderer.primitives[0] else {
            panic!("image should produce a texture")
        };
        assert_eq!(*rect, nickel_ui::Rect::new(15.0, 8.0, 10.0, 5.0));
        assert_eq!(
            *source,
            Rectangle::new((10.0, 6.0).into(), (20.0, 10.0).into())
        );
        assert!(renderer.raster.is_none());
        assert_eq!(renderer.diagnostics().fallback_image_count, 0);
    }

    #[test]
    fn unchanged_text_and_image_frames_reuse_buffers_without_uploads() {
        use std::sync::Arc;

        let commands = [
            PaintCommand::Text {
                bounds: nickel_ui::Rect::new(0.0, 0.0, 30.0, 12.0),
                text: "Nickel".into(),
                scale: 1.0,
                color: 0x336699,
                align: nickel_ui::TextAlign::Start,
                bold: false,
                wrap: false,
            },
            PaintCommand::Image {
                bounds: nickel_ui::Rect::new(30.0, 0.0, 8.0, 8.0),
                id: 7,
                generation: 3,
                image: Arc::new(image::RgbaImage::from_pixel(
                    8,
                    8,
                    image::Rgba([1, 2, 3, 255]),
                )),
                high_density: None,
            },
        ];
        let mut renderer = SmithayFrameRenderer::new(40, 16, 1.0, InternalUiRendererMode::Gpu);
        for generation in 1..=2 {
            renderer
                .render_frame(RenderFrame {
                    commands: &commands,
                    logical_size: (40, 16),
                    scale_factor: 1.0,
                    generation,
                })
                .unwrap();
        }

        let diagnostics = renderer.diagnostics();
        assert_eq!(diagnostics.text_allocations, 1);
        assert_eq!(diagnostics.text_uploads, 1);
        assert_eq!(diagnostics.text_cache_misses, 1);
        assert_eq!(diagnostics.text_cache_hits, 1);
        assert_eq!(diagnostics.image_allocations, 1);
        assert_eq!(diagnostics.image_uploads, 1);
        assert_eq!(diagnostics.image_cache_misses, 1);
        assert_eq!(diagnostics.image_cache_hits, 1);
        assert_eq!(diagnostics.text_cache_entries, 1);
        assert_eq!(diagnostics.image_cache_entries, 1);
    }

    #[test]
    fn resource_cache_eviction_bounds_entry_and_byte_churn() {
        fn texture(width: u32, height: u32) -> CachedTexture {
            CachedTexture {
                buffer: MemoryRenderBuffer::from_slice(
                    &vec![0; texture_bytes(width, height)],
                    Fourcc::Abgr8888,
                    (width as i32, height as i32),
                    1,
                    Transform::Normal,
                    None,
                ),
                width,
                height,
            }
        }

        let mut cache = TextureCache::new(2, 8);
        assert_eq!(cache.insert(1_u8, texture(1, 1), 4), 0);
        assert_eq!(cache.insert(2, texture(1, 1), 4), 0);
        assert!(cache.get(&1).is_some(), "access updates LRU order");
        assert_eq!(cache.insert(3, texture(1, 1), 4), 1);
        assert!(cache.get(&2).is_none(), "least recently used entry evicted");
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(cache.bytes, 8);

        assert_eq!(cache.insert(4, texture(2, 1), 8), 2);
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.bytes, 8);
        assert!(cache.get(&4).is_some());

        // Oversized resources remain usable for the current frame but cannot
        // displace the bounded reusable working set.
        assert_eq!(cache.insert(5, texture(3, 1), 12), 0);
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.bytes, 8);
    }

    #[test]
    fn image_cache_key_rejects_same_identity_with_changed_content_or_scale() {
        use std::sync::Arc;

        let make = |value, scale| {
            (
                [PaintCommand::Image {
                    bounds: nickel_ui::Rect::new(0.0, 0.0, 2.0, 2.0),
                    id: 1,
                    generation: 1,
                    image: Arc::new(image::RgbaImage::from_pixel(
                        2,
                        2,
                        image::Rgba([value, 0, 0, 255]),
                    )),
                    high_density: None,
                }],
                scale,
            )
        };
        let mut renderer = SmithayFrameRenderer::new(4, 4, 1.0, InternalUiRendererMode::Gpu);
        for (commands, scale) in [make(1, 1.0), make(2, 1.0), make(2, 2.0)] {
            renderer
                .render_frame(RenderFrame {
                    commands: &commands,
                    logical_size: (4, 4),
                    scale_factor: scale,
                    generation: 1,
                })
                .unwrap();
        }
        let diagnostics = renderer.diagnostics();
        assert_eq!(diagnostics.image_cache_misses, 3);
        assert_eq!(diagnostics.image_uploads, 3);
        assert_eq!(diagnostics.image_cache_entries, 3);
    }
}
