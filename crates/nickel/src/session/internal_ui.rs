//! Compositor ownership for Nickel UI applications which do not have a Wayland surface.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    hash::Hash,
    rc::Rc,
    time::Instant,
};

use nickel_ui::{
    Application, DamageRegion, GradientAxis, HostBatch, HostEvent, InternalSurfaceId,
    InternalSurfaceSet, LinearGradient, Point as UiPoint, SoftwareRenderer, Text, UiEvent, View,
    ViewContext,
    backend::{FrameRenderer, PaintCommand, RenderFrame},
};

use super::backend::InternalUiRendererMode;
mod desktop_input;
pub(crate) use desktop_input::DesktopPointerAction;
use sha2::{Digest, Sha256};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Color32F, ImportMem, Renderer,
            element::{
                Element, Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::{SolidColorBuffer, SolidColorRenderElement},
                utils::RescaleRenderElement,
            },
        },
    },
    utils::{Logical, Point, Rectangle, Transform},
};

#[cfg(test)]
mod memory_test_renderer;

smithay::backend::renderer::element::render_elements! {
    /// Smithay elements emitted by the compositor-owned Nickel UI presenter.
    pub InternalUiRenderElement<R> where R: Renderer + ImportMem;
    Memory=MemoryRenderBufferRenderElement<R>,
    Text=RescaleRenderElement<MemoryRenderBufferRenderElement<R>>,
    Solid=SolidColorRenderElement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternalSurfaceRole {
    Desktop,
    Panel,
    Overlay,
    /// Foremost compositor paint that never participates in hit testing or focus.
    PassiveOverlay,
    /// Interactive overlay that must preserve the text recipient's seat focus.
    OnScreenKeyboard,
    Application,
}

/// The compositor scene boundary an internal surface occupies.
///
/// Background surfaces are composed behind every Wayland client. Overlay
/// surfaces are composed in front of the client scene.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternalSurfaceLayer {
    Background,
    /// Application windows, composed in the ordinary client scene boundary.
    Application,
    Overlay,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InternalSurfacePlacement {
    pub role: InternalSurfaceRole,
    pub geometry: (i32, i32, u32, u32),
    pub output: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InternalWindowDecoration {
    pub owner: u64,
    pub title: String,
    pub active: bool,
    pub maximized: bool,
    pub background: u32,
    pub foreground: u32,
}

struct PresentedSurface {
    placement: InternalSurfacePlacement,
    renderer: SmithayFrameRenderer,
    dirty: bool,
    external_scene: Option<Vec<PaintCommand>>,
    scale_factor: f32,
    visible: bool,
    z_order: u64,
    decoration: Option<InternalWindowDecoration>,
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
    /// Individual GPU texture imports which failed and forced the complete
    /// surface through the software compatibility path.
    pub texture_import_failures: u64,
    /// Full-surface compatibility buffers which also failed to import.
    pub fallback_import_failures: u64,
    /// Full-surface CPU pixels currently retained for software fallback.
    pub software_frame_bytes: usize,
    /// Full-surface CPU pixels currently retained by the importable fallback buffer.
    pub fallback_raster_bytes: usize,
    pub fallback_buffer_creations: u64,
    pub fallback_buffer_reuses: u64,
    pub fallback_converted_bytes: u64,
    /// Bounding damage submitted to Smithay, excluding context-specific initial
    /// uploads and opaque driver storage. Smithay may combine pending damage.
    pub fallback_upload_damage_bytes: u64,
    pub fallback_full_repaints: u64,
    pub fallback_partial_repaints: u64,
    pub text_scratch_bytes: usize,
    pub text_private_cache_bytes: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AggregateInternalUiRendererDiagnostics {
    pub surfaces: usize,
    pub gpu_frames: u64,
    pub fallback_frames: u64,
    pub software_frame_bytes: usize,
    pub fallback_raster_bytes: usize,
    pub fallback_buffer_creations: u64,
    pub fallback_buffer_reuses: u64,
    pub fallback_converted_bytes: u64,
    pub fallback_upload_damage_bytes: u64,
    pub fallback_full_repaints: u64,
    pub fallback_partial_repaints: u64,
    pub text_scratch_bytes: usize,
    pub text_private_cache_bytes: usize,
    pub image_cache_entries: usize,
    pub image_cache_bytes: usize,
    pub text_cache_entries: usize,
    pub text_cache_bytes: usize,
    pub texture_import_failures: u64,
    pub fallback_import_failures: u64,
}

/// Nickel display-list adapter for Smithay's renderer element API.
///
/// Geometry and gradients become solid render elements. Images are imported as
/// their own textures, while text is rasterized into tightly bounded glyph
/// textures; neither causes a full-surface software upload.
pub struct SmithayFrameRenderer {
    software: Option<SoftwareRenderer>,
    text_software: SoftwareRenderer,
    primitives: Vec<GpuPrimitive>,
    raster: Option<MemoryRenderBuffer>,
    raster_configuration: Option<(u32, u32, u32)>,
    mode: InternalUiPresentationMode,
    diagnostics: InternalUiRendererDiagnostics,
    renderer_mode: InternalUiRendererMode,
    image_cache: Rc<RefCell<TextureCache<ImageTextureKey>>>,
    image_hashes: ImageHashes,
    text_cache: Rc<RefCell<TextureCache<TextTextureKey>>>,
    /// Retained until all renderer-specific texture imports succeed. Without
    /// this, an import error can only omit the affected icon from the frame.
    import_fallback: Option<ImportFallbackFrame>,
}

#[derive(Clone)]
struct ImportFallbackFrame {
    commands: Vec<PaintCommand>,
    logical_size: (u32, u32),
    scale_factor: f32,
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
    width: u32,
    height: u32,
    content_hash: [u8; 32],
}

#[derive(Default)]
struct ImageHashes {
    entries: HashMap<usize, (std::sync::Weak<image::RgbaImage>, [u8; 32])>,
    hashed_bytes: u64,
}

impl ImageHashes {
    fn get(&mut self, image: &std::sync::Arc<image::RgbaImage>) -> [u8; 32] {
        let address = std::sync::Arc::as_ptr(image) as usize;
        if let Some((owner, hash)) = self.entries.get(&address)
            && owner
                .upgrade()
                .is_some_and(|owner| std::sync::Arc::ptr_eq(&owner, image))
        {
            return *hash;
        }
        // Weak ownership neither pins pixel buffers nor aliases a reused address.
        // Arc mutation with a weak observer detaches, so changed pixels rehash.
        if self.entries.len() >= IMAGE_CACHE_ENTRY_LIMIT {
            self.entries
                .retain(|_, (owner, _)| owner.strong_count() != 0);
            if self.entries.len() >= IMAGE_CACHE_ENTRY_LIMIT {
                self.entries.clear();
            }
        }
        let hash = content_hash(image.as_raw());
        self.hashed_bytes = self
            .hashed_bytes
            .saturating_add(image.as_raw().len() as u64);
        self.entries
            .insert(address, (std::sync::Arc::downgrade(image), hash));
        hash
    }
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
        text_scale: Option<f32>,
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

fn estimated_rounded_elements(rect: nickel_ui::Rect, radius: f32, top_only: bool) -> usize {
    let rows = rect.size.height.ceil().max(1.0) as usize;
    let radius = radius
        .max(0.0)
        .min(rect.size.width / 2.0)
        .min(rect.size.height / 2.0);
    if radius < 0.5 {
        return 1;
    }
    let corner_rows = radius.ceil() as usize;
    let corner_rows = if top_only {
        corner_rows
    } else {
        corner_rows.saturating_mul(2)
    };
    // Corner strips plus the coalesced rectangular middle. This is a
    // conservative upper bound because pixel-centre sampling can turn an
    // edge row into part of the middle.
    rows.min(corner_rows.saturating_add(1))
}

struct SharedTextureCaches {
    images: Rc<RefCell<TextureCache<ImageTextureKey>>>,
    text: Rc<RefCell<TextureCache<TextTextureKey>>>,
}

impl Default for SharedTextureCaches {
    fn default() -> Self {
        Self {
            images: Rc::new(RefCell::new(TextureCache::new(
                IMAGE_CACHE_ENTRY_LIMIT,
                IMAGE_CACHE_BYTE_LIMIT,
            ))),
            text: Rc::new(RefCell::new(TextureCache::new(
                TEXT_CACHE_ENTRY_LIMIT,
                TEXT_CACHE_BYTE_LIMIT,
            ))),
        }
    }
}

impl SmithayFrameRenderer {
    #[cfg(test)]
    fn new(_width: u32, _height: u32, scale: f32, renderer_mode: InternalUiRendererMode) -> Self {
        Self::with_caches(scale, renderer_mode, &SharedTextureCaches::default())
    }

    fn with_caches(
        scale: f32,
        renderer_mode: InternalUiRendererMode,
        caches: &SharedTextureCaches,
    ) -> Self {
        Self {
            // The healthy GPU path has no reason to commit a full-surface CPU
            // framebuffer. Software presentation remains available and is
            // allocated on demand by `prepare_fallback`.
            software: None,
            // Text commands are rasterized into bounded textures. Reuse one
            // renderer so its process font database and shaping cache survive
            // across every label in a scene; constructing a font system per
            // command can stall the compositor event loop for many seconds.
            text_software: SoftwareRenderer::new(1, 1, scale),
            primitives: Vec::new(),
            raster: None,
            raster_configuration: None,
            mode: InternalUiPresentationMode::RasterFallback,
            diagnostics: InternalUiRendererDiagnostics::default(),
            renderer_mode,
            image_cache: Rc::clone(&caches.images),
            image_hashes: Default::default(),
            text_cache: Rc::clone(&caches.text),
            import_fallback: None,
        }
    }

    pub fn mode(&self) -> InternalUiPresentationMode {
        self.mode
    }

    pub fn diagnostics(&self) -> InternalUiRendererDiagnostics {
        InternalUiRendererDiagnostics {
            text_scratch_bytes: self.text_software.pixel_capacity_bytes(),
            text_private_cache_bytes: self.text_software.cache_diagnostics().live_bytes,
            ..self.diagnostics
        }
    }

    /// Release regenerable frame-sized state while the owning surface cannot
    /// be presented. Shared texture caches remain available to other surfaces.
    fn suspend(&mut self) {
        self.image_hashes.entries.clear();
        self.text_software.suspend();
        if let Some(mut software) = self.software.take() {
            software.suspend();
        }
        self.raster = None;
        self.raster_configuration = None;
        self.primitives.clear();
        self.import_fallback = None;
        self.mode = InternalUiPresentationMode::GpuSolid;
        self.diagnostics.software_frame_bytes = 0;
        self.diagnostics.fallback_raster_bytes = 0;
        self.diagnostics.fallback_primitive_count = 0;
        self.diagnostics.fallback_text_count = 0;
        self.diagnostics.fallback_image_count = 0;
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
                PaintCommand::TopRoundedFill { rect, radius, .. } => {
                    estimated_rounded_elements(*rect, *radius, true)
                }
                PaintCommand::RoundedFill { rect, radius, .. } => {
                    estimated_rounded_elements(*rect, *radius, false)
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
        // Only corner rows need individual strips. Coalesce the rectangular
        // middle into one element so a large rounded launcher background does
        // not exceed the GPU element budget merely because it is tall. That
        // otherwise activates two full-output software fallback buffers and
        // leaves the process allocator's resident high-water mark behind when
        // the transient surface closes.
        let rows = rect.size.height.ceil().max(1.0) as u32;
        let mut middle_start = None::<f32>;
        let mut middle_end = 0.0_f32;
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
            if inset == 0.0 {
                middle_start.get_or_insert(y);
                middle_end = y + height;
                continue;
            }
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
        if let Some(y) = middle_start {
            self.push_solid(
                nickel_ui::Rect::new(
                    rect.origin.x,
                    rect.origin.y + y,
                    rect.size.width,
                    middle_end - y,
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
        self.import_fallback = Some(ImportFallbackFrame {
            commands: frame.commands.to_vec(),
            logical_size: frame.logical_size,
            scale_factor: frame.scale_factor,
        });
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
                        width: image.width(),
                        height: image.height(),
                        content_hash: self.image_hashes.get(image),
                    };
                    let cached = self.image_cache.borrow_mut().get(&key);
                    let texture = if let Some(texture) = cached {
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
                            buffer: premultiplied_memory_buffer(
                                image.as_raw(),
                                image.width(),
                                image.height(),
                            ),
                            width: image.width(),
                            height: image.height(),
                        };
                        let bytes = texture_bytes(texture.width, texture.height);
                        let evictions =
                            self.image_cache
                                .borrow_mut()
                                .insert(key, texture.clone(), bytes);
                        self.diagnostics.image_cache_evictions = self
                            .diagnostics
                            .image_cache_evictions
                            .saturating_add(evictions);
                        texture
                    };
                    self.primitives.push(GpuPrimitive::Texture {
                        rect,
                        source,
                        text_scale: None,
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
        let cached = self.text_cache.borrow_mut().get(&key);
        if let Some(texture) = cached {
            self.diagnostics.text_cache_hits = self.diagnostics.text_cache_hits.saturating_add(1);
            let source = text_source_rect(rect, bounds, scale, texture.width, texture.height);
            if source.is_empty() {
                return;
            }
            self.primitives.push(GpuPrimitive::Texture {
                rect: text_destination_rect(bounds, source, scale),
                source,
                text_scale: Some(scale),
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
            // SoftwareRenderer already blends into premultiplied RGBA.
            // Multiplying coverage again darkens antialiased glyph edges.
            bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
        }
        let (physical_width, physical_height) = self.text_software.size();
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
        let source = text_source_rect(rect, bounds, scale, texture.width, texture.height);
        if source.is_empty() {
            return;
        }
        // Large one-off labels must not pin a peak-sized private framebuffer
        // for the remaining lifetime of a visible host. Shared textures own
        // their pixels independently and remain valid after suspension.
        if self.text_software.pixel_capacity_bytes() > 8 * 1024 * 1024 {
            self.text_software.suspend();
        }
        self.diagnostics.text_allocations = self.diagnostics.text_allocations.saturating_add(1);
        self.diagnostics.text_uploads = self.diagnostics.text_uploads.saturating_add(1);
        let evictions = self.text_cache.borrow_mut().insert(
            key,
            texture.clone(),
            texture_bytes(physical_width, physical_height),
        );
        self.diagnostics.text_cache_evictions = self
            .diagnostics
            .text_cache_evictions
            .saturating_add(evictions);
        self.primitives.push(GpuPrimitive::Texture {
            rect: text_destination_rect(bounds, source, scale),
            source,
            text_scale: Some(scale),
            buffer: texture.buffer,
        });
    }

    fn refresh_cache_diagnostics(&mut self) {
        let images = self.image_cache.borrow();
        self.diagnostics.image_cache_entries = images.entries.len();
        self.diagnostics.image_cache_bytes = images.bytes;
        let text = self.text_cache.borrow();
        self.diagnostics.text_cache_entries = text.entries.len();
        self.diagnostics.text_cache_bytes = text.bytes;
    }

    fn prepare_fallback(&mut self, frame: RenderFrame<'_>) -> DamageRegion {
        let width = ((frame.logical_size.0 as f32) * frame.scale_factor)
            .round()
            .max(1.0) as u32;
        let height = ((frame.logical_size.1 as f32) * frame.scale_factor)
            .round()
            .max(1.0) as u32;
        let configuration = (width, height, frame.scale_factor.to_bits());
        let full_repaint =
            self.raster.is_none() || self.raster_configuration != Some(configuration);
        let software = self
            .software
            .get_or_insert_with(|| SoftwareRenderer::new(width, height, frame.scale_factor));
        software.resize(width, height, frame.scale_factor);
        if full_repaint {
            software.invalidate();
        }
        let mut damage = software.render(frame.commands);
        let (width, height) = software.size();
        if full_repaint {
            self.raster = Some(MemoryRenderBuffer::new(
                Fourcc::Abgr8888,
                (width as i32, height as i32),
                1,
                Transform::Normal,
                None,
            ));
            self.raster_configuration = Some(configuration);
            self.diagnostics.fallback_buffer_creations =
                self.diagnostics.fallback_buffer_creations.saturating_add(1);
            damage.rects.clear();
            damage
                .rects
                .push(nickel_ui::Rect::new(0.0, 0.0, width as f32, height as f32));
        }
        self.primitives.clear();
        self.import_fallback = None;
        self.diagnostics.software_frame_bytes = software.pixel_capacity_bytes();
        self.diagnostics.fallback_raster_bytes = texture_bytes(width, height);
        if damage.is_empty() {
            return damage;
        }
        let regions = fallback_damage_regions(&damage, width, height);
        let full = Rectangle::from_size((width as i32, height as i32).into());
        if regions.contains(&full) {
            self.diagnostics.fallback_full_repaints =
                self.diagnostics.fallback_full_repaints.saturating_add(1);
        } else {
            self.diagnostics.fallback_partial_repaints =
                self.diagnostics.fallback_partial_repaints.saturating_add(1);
        }
        if !full_repaint {
            self.diagnostics.fallback_buffer_reuses =
                self.diagnostics.fallback_buffer_reuses.saturating_add(1);
        }
        let converted = regions
            .iter()
            .map(|rect| rect.size.w as u64 * rect.size.h as u64 * 4)
            .sum::<u64>();
        let upload_damage = regions.iter().copied().reduce(|a, b| a.merge(b));
        self.diagnostics.fallback_converted_bytes = self
            .diagnostics
            .fallback_converted_bytes
            .saturating_add(converted);
        self.diagnostics.fallback_upload_damage_bytes = self
            .diagnostics
            .fallback_upload_damage_bytes
            .saturating_add(
                upload_damage.map_or(0, |rect| rect.size.w as u64 * rect.size.h as u64 * 4),
            );
        // Smithay serializes access and gives retained render elements immutable
        // CPU snapshots through Arc::make_mut. Reusing this handle preserves its
        // per-renderer texture imports and damage history. Snapshot lifetimes and
        // driver allocations are Smithay-owned, not a second private buffer pool.
        self.raster
            .as_mut()
            .expect("fallback buffer initialized")
            .render()
            .draw(|bytes| {
                for region in &regions {
                    for y in region.loc.y..region.loc.y + region.size.h {
                        let start = y as usize * width as usize + region.loc.x as usize;
                        let end = start + region.size.w as usize;
                        for (target, pixel) in bytes[start * 4..end * 4]
                            .chunks_exact_mut(4)
                            .zip(&software.pixels()[start..end])
                        {
                            target.copy_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
                        }
                    }
                }
                Ok::<_, std::convert::Infallible>(regions)
            })
            .unwrap();
        damage
    }

    fn activate_import_fallback(&mut self) -> bool {
        let Some(frame) = self.import_fallback.take() else {
            return false;
        };
        self.mode = InternalUiPresentationMode::RasterFallback;
        self.diagnostics.fallback_frames = self.diagnostics.fallback_frames.saturating_add(1);
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
        self.diagnostics.fallback_primitive_count = frame.commands.len();
        self.prepare_fallback(RenderFrame {
            commands: &frame.commands,
            logical_size: frame.logical_size,
            scale_factor: frame.scale_factor,
            generation: 0,
        });
        true
    }

    fn elements<R: Renderer + ImportMem>(
        &mut self,
        renderer: &mut R,
        location: Point<i32, Logical>,
        logical_size: (u32, u32),
    ) -> Vec<InternalUiRenderElement<R>>
    where
        R::TextureId: Send + Clone + 'static,
    {
        if self.mode == InternalUiPresentationMode::GpuSolid {
            let mut elements = Vec::with_capacity(self.primitives.len());
            let mut import_failed = false;
            for primitive in self.primitives.iter().rev() {
                let element = match primitive {
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
                        text_scale,
                        buffer,
                    } => match MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        (
                            (f64::from(location.x) + f64::from(rect.origin.x))
                                * f64::from(text_scale.unwrap_or(1.0)),
                            (f64::from(location.y) + f64::from(rect.origin.y))
                                * f64::from(text_scale.unwrap_or(1.0)),
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
                    ) {
                        Ok(element) => Some(if let Some(scale) = text_scale {
                            // Smithay sizes ordinary textures in integer logical units.
                            // Text is already rasterized at output resolution: preserve
                            // its physical extent instead of fitting it to that rounding.
                            let geometry = element.geometry(f64::from(*scale).into());
                            RescaleRenderElement::from_element(
                                element,
                                geometry.loc,
                                (
                                    source.size.w / f64::from(geometry.size.w),
                                    source.size.h / f64::from(geometry.size.h),
                                ),
                            )
                            .into()
                        } else {
                            element.into()
                        }),
                        Err(error) => {
                            tracing::warn!(
                                ?error,
                                "failed to import compositor-owned UI texture; falling back to the complete software surface"
                            );
                            None
                        }
                    },
                };
                if let Some(element) = element {
                    elements.push(element);
                } else {
                    import_failed = true;
                    break;
                }
            }
            if !import_failed {
                return elements;
            }
            self.diagnostics.texture_import_failures =
                self.diagnostics.texture_import_failures.saturating_add(1);
            let _ = self.activate_import_fallback();
        }

        let Some(buffer) = self.raster.as_ref() else {
            return Vec::new();
        };
        match MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            (f64::from(location.x), f64::from(location.y)),
            buffer,
            None,
            None,
            Some((logical_size.0 as i32, logical_size.1 as i32).into()),
            Kind::Unspecified,
        ) {
            Ok(element) => vec![element.into()],
            Err(error) => {
                self.diagnostics.fallback_import_failures =
                    self.diagnostics.fallback_import_failures.saturating_add(1);
                tracing::error!(
                    ?error,
                    "failed to import compositor-owned UI software fallback"
                );
                Vec::new()
            }
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
            if let Some(mut software) = self.software.take() {
                software.suspend();
            }
            self.diagnostics.software_frame_bytes = 0;
            self.diagnostics.fallback_raster_bytes = 0;
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

fn premultiplied_memory_buffer(bytes: &[u8], width: u32, height: u32) -> MemoryRenderBuffer {
    debug_assert_eq!(bytes.len(), texture_bytes(width, height));
    let mut buffer = MemoryRenderBuffer::new(
        Fourcc::Abgr8888,
        (width as i32, height as i32),
        1,
        Transform::Normal,
        None,
    );
    buffer
        .render()
        .draw(|target| {
            for (target, source) in target.chunks_exact_mut(4).zip(bytes.chunks_exact(4)) {
                target.copy_from_slice(&premultiplied_pixel([
                    source[0], source[1], source[2], source[3],
                ]));
            }
            Ok::<_, std::convert::Infallible>(vec![Rectangle::from_size(
                (width as i32, height as i32).into(),
            )])
        })
        .unwrap();
    buffer
}

fn premultiplied_pixel([red, green, blue, alpha]: [u8; 4]) -> [u8; 4] {
    let scale = u16::from(alpha);
    let channel = |value: u8| ((u16::from(value) * scale + 127) / 255) as u8;
    [channel(red), channel(green), channel(blue), alpha]
}

fn fallback_damage_regions(
    damage: &DamageRegion,
    width: u32,
    height: u32,
) -> Vec<Rectangle<i32, smithay::utils::Buffer>> {
    damage
        .rects
        .iter()
        .filter_map(|rect| {
            let left = rect.origin.x.floor().clamp(0.0, width as f32) as i32;
            let top = rect.origin.y.floor().clamp(0.0, height as f32) as i32;
            let right = (rect.origin.x + rect.size.width)
                .ceil()
                .clamp(0.0, width as f32) as i32;
            let bottom = (rect.origin.y + rect.size.height)
                .ceil()
                .clamp(0.0, height as f32) as i32;
            (right > left && bottom > top)
                .then(|| Rectangle::new((left, top).into(), (right - left, bottom - top).into()))
        })
        .collect()
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
    texture_width: u32,
    texture_height: u32,
) -> Rectangle<f64, Logical> {
    // Crop on the raster's pixel grid. A proportional crop based on rounded
    // allocation dimensions changes the glyph scale whenever layout is fractional.
    let x = ((rect.origin.x - bounds.origin.x) * scale).round().max(0.0);
    let y = ((rect.origin.y - bounds.origin.y) * scale).round().max(0.0);
    let right = if rect.origin.x + rect.size.width >= bounds.origin.x + bounds.size.width {
        texture_width as f32
    } else {
        ((rect.origin.x + rect.size.width - bounds.origin.x) * scale)
            .round()
            .min(texture_width as f32)
    };
    let bottom = if rect.origin.y + rect.size.height >= bounds.origin.y + bounds.size.height {
        texture_height as f32
    } else {
        ((rect.origin.y + rect.size.height - bounds.origin.y) * scale)
            .round()
            .min(texture_height as f32)
    };
    Rectangle::new(
        (f64::from(x), f64::from(y)).into(),
        (
            f64::from((right - x).max(0.0)),
            f64::from((bottom - y).max(0.0)),
        )
            .into(),
    )
}

fn text_destination_rect(
    bounds: nickel_ui::Rect,
    source: Rectangle<f64, Logical>,
    scale: f32,
) -> nickel_ui::Rect {
    nickel_ui::Rect::new(
        bounds.origin.x + source.loc.x as f32 / scale,
        bounds.origin.y + source.loc.y as f32 / scale,
        source.size.w as f32 / scale,
        source.size.h as f32 / scale,
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
    clipboard_limit: usize,
    clipboard_result: Option<Result<String, String>>,
    surfaces: InternalSurfaceSet,
    presentation: BTreeMap<InternalSurfaceId, PresentedSurface>,
    focused: Option<InternalSurfaceId>,
    hovered: Option<InternalSurfaceId>,
    touches: BTreeMap<u64, (InternalSurfaceId, UiPoint)>,
    // Keep the complete batch for coordinator-owned scenes: normalized input and
    // focus lifecycle facts must survive the same handoff as semantic UI actions.
    routed_events: Vec<(
        InternalSurfaceId,
        HostBatch,
        Option<nickel_input::ModifierState>,
    )>,
    desktop_input: desktop_input::DesktopInputState,
    renderer_mode: InternalUiRendererMode,
    next_z_order: u64,
    texture_caches: SharedTextureCaches,
    frame_icons: Option<crate::session::window_frame::FrameIcons>,
}

impl Default for InternalUiRuntime {
    fn default() -> Self {
        Self {
            clipboard_limit: 0,
            clipboard_result: None,
            surfaces: InternalSurfaceSet::default(),
            presentation: BTreeMap::new(),
            focused: None,
            hovered: None,
            touches: BTreeMap::new(),
            routed_events: Vec::new(),
            desktop_input: Default::default(),
            renderer_mode: InternalUiRendererMode::Gpu,
            next_z_order: 0,
            texture_caches: SharedTextureCaches::default(),
            frame_icons: crate::session::window_frame::FrameIcons::load(),
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
        let id = self.surfaces.insert_boxed(surface);
        self.next_z_order = self.next_z_order.saturating_add(1);
        self.presentation.insert(
            id,
            PresentedSurface {
                placement,
                renderer: SmithayFrameRenderer::with_caches(
                    scale,
                    self.renderer_mode,
                    &self.texture_caches,
                ),
                dirty: true,
                external_scene: None,
                scale_factor: scale,
                visible: true,
                z_order: self.next_z_order,
                decoration: None,
            },
        );
        id
    }

    pub fn application<T: 'static>(&self, id: InternalSurfaceId) -> Option<&T> {
        self.surfaces.get(id)?.application().downcast_ref()
    }

    /// Current application-owned title for a compositor-hosted surface.
    pub fn title(&self, id: InternalSurfaceId) -> Option<&str> {
        self.surfaces.get(id).map(|surface| surface.title())
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

    /// Relinquish compositor-owned keyboard focus.
    ///
    /// Native focus transitions call this before assigning the Smithay seat so
    /// the hosted UI and protocol client cannot both believe they own input.
    pub fn clear_focus(&mut self) -> Option<InternalSurfaceId> {
        let previous = self.focused.take()?;
        self.step(
            previous,
            HostBatch {
                window_focused: Some(false),
                ..Default::default()
            },
        );
        Some(previous)
    }
    pub fn insert<A: Application + 'static>(
        &mut self,
        application: A,
        placement: InternalSurfacePlacement,
        scale: f32,
    ) -> InternalSurfaceId {
        let (_, _, width, height) = placement.geometry;
        let id = self.surfaces.insert(application, width, height);
        self.next_z_order = self.next_z_order.saturating_add(1);
        self.presentation.insert(
            id,
            PresentedSurface {
                placement,
                renderer: SmithayFrameRenderer::with_caches(
                    scale,
                    self.renderer_mode,
                    &self.texture_caches,
                ),
                dirty: true,
                external_scene: None,
                scale_factor: scale,
                visible: true,
                z_order: self.next_z_order,
                decoration: None,
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
        self.retire_normalized_touch_surface(id);
        if self.focused == Some(id) {
            self.clear_focus();
        }
        let removed = self.surfaces.remove(id).is_some();
        self.presentation.remove(&id);
        if self.hovered == Some(id) {
            self.hovered = None;
        }
        self.touches.retain(|_, (target, _)| *target != id);
        removed
    }

    pub fn placement(&self, id: InternalSurfaceId) -> Option<&InternalSurfacePlacement> {
        self.presentation.get(&id).map(|surface| &surface.placement)
    }

    pub(crate) fn set_window_decoration(
        &mut self,
        id: InternalSurfaceId,
        decoration: InternalWindowDecoration,
    ) -> bool {
        let Some(surface) = self.presentation.get_mut(&id) else {
            return false;
        };
        if surface.placement.role != InternalSurfaceRole::Application
            || surface.decoration.as_ref() == Some(&decoration)
        {
            return false;
        }
        surface.decoration = Some(decoration);
        surface.dirty = true;
        true
    }

    pub(crate) fn internal_frame_target(
        &self,
        point: (f64, f64),
    ) -> Option<(InternalSurfaceId, crate::session::window_frame::FramePart)> {
        self.presentation
            .iter()
            .filter(|(_, surface)| {
                surface.visible
                    && surface.placement.role == InternalSurfaceRole::Application
                    && surface.decoration.is_some()
            })
            .filter_map(|(id, surface)| {
                let (x, y, width, height) = surface.placement.geometry;
                crate::session::window_frame::hit_test(
                    crate::session::shell_layout::Geometry {
                        x,
                        y,
                        width: i32::try_from(width).unwrap_or(i32::MAX),
                        height: i32::try_from(height).unwrap_or(i32::MAX),
                    },
                    point.0.round() as i32,
                    point.1.round() as i32,
                )
                .map(|part| (surface.z_order, *id, part))
            })
            .max_by_key(|(z, _, _)| *z)
            .map(|(_, id, part)| (id, part))
    }

    pub(crate) fn configure_application(
        &mut self,
        id: InternalSurfaceId,
        placement: InternalSurfacePlacement,
    ) -> bool {
        let Some(surface) = self.presentation.get_mut(&id) else {
            return false;
        };
        if surface.placement.role != InternalSurfaceRole::Application {
            return false;
        }
        let resized = surface.placement.geometry.2 != placement.geometry.2
            || surface.placement.geometry.3 != placement.geometry.3;
        let changed = surface.placement != placement;
        surface.placement = placement;
        if resized {
            surface.renderer.suspend();
            surface.dirty = true;
            if let Some(host) = self.surfaces.get_mut(id) {
                host.step(HostBatch {
                    surface_size: Some((
                        surface.placement.geometry.2,
                        surface.placement.geometry.3,
                    )),
                    ..HostBatch::default()
                });
            }
        }
        changed
    }

    /// Show or hide a hosted surface without destroying its application state.
    pub fn set_visible(&mut self, id: InternalSurfaceId, visible: bool) -> bool {
        if !visible {
            self.retire_normalized_touch_surface(id);
        }
        let changed = {
            let Some(surface) = self.presentation.get_mut(&id) else {
                return false;
            };
            let changed = surface.visible != visible;
            surface.visible = visible;
            if visible {
                surface.dirty = true;
            } else {
                surface.renderer.suspend();
            }
            changed
        };
        if !visible {
            if self.focused == Some(id) {
                self.clear_focus();
            }
            if self.hovered == Some(id) {
                self.hovered = None;
            }
            self.touches.retain(|_, (target, _)| *target != id);
        }
        changed
    }

    pub fn is_visible(&self, id: InternalSurfaceId) -> bool {
        self.presentation
            .get(&id)
            .is_some_and(|surface| surface.visible)
    }

    /// Bring a compositor-hosted application to the front of its role layer.
    pub fn raise(&mut self, id: InternalSurfaceId) -> bool {
        let Some(surface) = self.presentation.get_mut(&id) else {
            return false;
        };
        self.next_z_order = self.next_z_order.saturating_add(1);
        surface.z_order = self.next_z_order;
        true
    }

    /// Move an existing surface in compositor-global logical coordinates.
    ///
    /// Shell overlays retain their identity while their invoking output changes.
    /// A size change still requires reinsertion because it also resizes the UI host
    /// and renderer; callers use this method for relocation-only updates.
    pub fn relocate(&mut self, id: InternalSurfaceId, placement: InternalSurfacePlacement) -> bool {
        let Some(surface) = self.presentation.get_mut(&id) else {
            return false;
        };
        if surface.placement.geometry.2 != placement.geometry.2
            || surface.placement.geometry.3 != placement.geometry.3
        {
            return false;
        }
        let changed = surface.placement != placement;
        surface.placement = placement;
        changed
    }

    #[cfg(test)]
    pub(crate) fn scale_factor(&self, id: InternalSurfaceId) -> Option<f32> {
        self.presentation
            .get(&id)
            .map(|surface| surface.scale_factor)
    }

    /// Resize an externally produced scene without replacing its runtime ID.
    /// In particular, a keyboard resize gesture must keep its captured target
    /// while scene layout, host geometry and raster scale change together.
    pub(crate) fn configure_scene(
        &mut self,
        id: InternalSurfaceId,
        placement: InternalSurfacePlacement,
        scale: f32,
    ) -> bool {
        let Some(surface) = self.presentation.get_mut(&id) else {
            return false;
        };
        if surface.external_scene.is_none() {
            return false;
        }
        let resized = surface.placement.geometry.2 != placement.geometry.2
            || surface.placement.geometry.3 != placement.geometry.3
            || surface.scale_factor != scale;
        let changed = resized || surface.placement != placement;
        surface.placement = placement;
        surface.scale_factor = scale;
        if resized {
            surface.renderer.suspend();
            surface.dirty = true;
            if let Some(host) = self.surfaces.get_mut(id) {
                host.step(HostBatch {
                    surface_size: Some((
                        surface.placement.geometry.2,
                        surface.placement.geometry.3,
                    )),
                    scale_factor: Some(scale),
                    ..Default::default()
                });
            }
        }
        changed
    }

    pub(crate) fn set_clipboard_limit(&mut self, limit: usize) {
        self.clipboard_limit = limit;
    }

    pub(crate) fn paste_clipboard_image(
        &mut self,
        id: InternalSurfaceId,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> bool {
        let Some(surface) = self.surfaces.get_mut(id) else {
            return false;
        };
        if surface.paste_clipboard_image(width, height, rgba) {
            if let Some(presentation) = self.presentation.get_mut(&id) {
                presentation.dirty = true;
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn take_clipboard_result(&mut self) -> Option<Result<String, String>> {
        self.clipboard_result.take()
    }

    pub fn step(&mut self, id: InternalSurfaceId, mut batch: HostBatch) -> bool {
        batch.clipboard_text_limit = Some(self.clipboard_limit);
        if batch.window_focused == Some(false) {
            self.clear_desktop_pressed_keys();
        }
        if self
            .presentation
            .get(&id)
            .is_some_and(|surface| surface.external_scene.is_some())
        {
            // The SceneSlot supplies identity only. Its reducer cannot apply input
            // or focus changes to the LiveShell authority owned by the coordinator.
            self.routed_events.push((id, batch, None));
            return true;
        }
        let Some(surface) = self.surfaces.get_mut(id) else {
            return false;
        };
        let mut outcome = surface.step(batch);
        crate::session_host::record_clipboard_outcome(&mut self.clipboard_result, &mut outcome);
        let changed = outcome.changed;
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

    pub fn drain_routed_events(
        &mut self,
    ) -> Vec<(
        InternalSurfaceId,
        HostBatch,
        Option<nickel_input::ModifierState>,
    )> {
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

    pub fn aggregate_renderer_diagnostics(&self) -> AggregateInternalUiRendererDiagnostics {
        let mut total = self.presentation.values().fold(
            AggregateInternalUiRendererDiagnostics::default(),
            |mut total, surface| {
                let item = surface.renderer.diagnostics();
                total.surfaces = total.surfaces.saturating_add(1);
                total.gpu_frames = total.gpu_frames.saturating_add(item.gpu_frames);
                total.fallback_frames = total.fallback_frames.saturating_add(item.fallback_frames);
                total.software_frame_bytes = total
                    .software_frame_bytes
                    .saturating_add(item.software_frame_bytes);
                total.fallback_raster_bytes = total
                    .fallback_raster_bytes
                    .saturating_add(item.fallback_raster_bytes);
                total.fallback_buffer_creations = total
                    .fallback_buffer_creations
                    .saturating_add(item.fallback_buffer_creations);
                total.fallback_buffer_reuses = total
                    .fallback_buffer_reuses
                    .saturating_add(item.fallback_buffer_reuses);
                total.fallback_converted_bytes = total
                    .fallback_converted_bytes
                    .saturating_add(item.fallback_converted_bytes);
                total.fallback_upload_damage_bytes = total
                    .fallback_upload_damage_bytes
                    .saturating_add(item.fallback_upload_damage_bytes);
                total.fallback_full_repaints = total
                    .fallback_full_repaints
                    .saturating_add(item.fallback_full_repaints);
                total.fallback_partial_repaints = total
                    .fallback_partial_repaints
                    .saturating_add(item.fallback_partial_repaints);
                total.text_scratch_bytes = total
                    .text_scratch_bytes
                    .saturating_add(item.text_scratch_bytes);
                total.text_private_cache_bytes = total
                    .text_private_cache_bytes
                    .saturating_add(item.text_private_cache_bytes);
                total.texture_import_failures = total
                    .texture_import_failures
                    .saturating_add(item.texture_import_failures);
                total.fallback_import_failures = total
                    .fallback_import_failures
                    .saturating_add(item.fallback_import_failures);
                total
            },
        );
        let images = self.texture_caches.images.borrow();
        total.image_cache_entries = images.entries.len();
        total.image_cache_bytes = images.bytes;
        let text = self.texture_caches.text.borrow();
        total.text_cache_entries = text.entries.len();
        total.text_cache_bytes = text.bytes;
        total
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
        self.presentation
            .values()
            .any(|surface| surface.visible && surface.dirty)
    }

    pub fn focused(&self) -> Option<InternalSurfaceId> {
        self.focused
    }

    pub(crate) fn focused_field_lease(
        &self,
        id: InternalSurfaceId,
    ) -> Option<(nickel_ui::UiId, u64)> {
        let inspection = self.surfaces.get(id)?.inspect();
        Some((
            inspection.keyboard_focus?,
            inspection.keyboard_focus_generation,
        ))
    }

    fn role_order(role: InternalSurfaceRole) -> u8 {
        match role {
            InternalSurfaceRole::Desktop => 0,
            InternalSurfaceRole::Application => 1,
            InternalSurfaceRole::Panel => 2,
            InternalSurfaceRole::Overlay
            | InternalSurfaceRole::PassiveOverlay
            | InternalSurfaceRole::OnScreenKeyboard => 3,
        }
    }

    fn role_layer(role: InternalSurfaceRole) -> InternalSurfaceLayer {
        match role {
            InternalSurfaceRole::Desktop => InternalSurfaceLayer::Background,
            InternalSurfaceRole::Application => InternalSurfaceLayer::Application,
            InternalSurfaceRole::Panel
            | InternalSurfaceRole::Overlay
            | InternalSurfaceRole::PassiveOverlay
            | InternalSurfaceRole::OnScreenKeyboard => InternalSurfaceLayer::Overlay,
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
                (surface.visible
                    && surface.placement.role != InternalSurfaceRole::PassiveOverlay
                    && !(client_present
                        && matches!(
                            surface.placement.role,
                            InternalSurfaceRole::Desktop | InternalSurfaceRole::Application
                        ))
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

    /// Return an application surface irrespective of the external client scene.
    ///
    /// This is used only to begin compositor gestures such as Super+drag; normal
    /// pointer routing continues to respect the client scene above it.
    pub fn application_surface_at(
        &self,
        point: (f64, f64),
    ) -> Option<(InternalSurfaceId, UiPoint)> {
        self.presentation
            .iter()
            .filter(|(_, surface)| surface.placement.role == InternalSurfaceRole::Application)
            .filter_map(|(id, surface)| {
                let (x, y, width, height) = surface.placement.geometry;
                (point.0 >= f64::from(x)
                    && point.1 >= f64::from(y)
                    && point.0 < f64::from(x) + f64::from(width)
                    && point.1 < f64::from(y) + f64::from(height))
                .then_some((
                    *id,
                    UiPoint {
                        x: (point.0 - f64::from(x)) as f32,
                        y: (point.1 - f64::from(y)) as f32,
                    },
                ))
            })
            .max_by_key(|(id, _)| *id)
    }

    fn dispatch_ui(&mut self, id: InternalSurfaceId, event: UiEvent) -> bool {
        if matches!(event, UiEvent::PointerCancelled) && self.desktop_pointer_leave(id) {
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

    fn surface_accepts_keyboard_focus(&self, id: InternalSurfaceId) -> bool {
        self.presentation.get(&id).is_some_and(|surface| {
            matches!(
                surface.placement.role,
                InternalSurfaceRole::Desktop
                    | InternalSurfaceRole::Overlay
                    | InternalSurfaceRole::Application
            )
        })
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
            if self.surface_accepts_keyboard_focus(id) && self.focused != Some(id) {
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
                if self.surface_accepts_keyboard_focus(id) && self.focused != Some(id) {
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

    pub fn submit_or_activate(&mut self) -> bool {
        self.focused.is_some_and(|id| {
            let submitted = self.step(
                id,
                HostBatch {
                    events: vec![HostEvent::Shortcut(nickel_ui::Shortcut::Submit)],
                    ..Default::default()
                },
            );
            if !submitted {
                self.dispatch_ui(id, UiEvent::KeyboardActivate);
            }
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
            (surface.visible
                && surface
                    .placement
                    .output
                    .as_deref()
                    .is_none_or(|name| name == output))
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
            let surface = self.presentation.get(id).unwrap();
            (
                std::cmp::Reverse(Self::role_order(surface.placement.role)),
                std::cmp::Reverse(surface.z_order),
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
        let frame_icons = self.frame_icons.clone();
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
                let presentation = self.presentation.get_mut(&id)?;
                let local = output_local_location(placement.geometry, output_origin);
                let mut content = presentation.renderer.elements(
                    renderer,
                    (local.0.round() as i32, local.1.round() as i32).into(),
                    (placement.geometry.2, placement.geometry.3),
                );
                let Some(decoration) = presentation.decoration.as_ref() else {
                    return Some(content);
                };
                let width = i32::try_from(placement.geometry.2).unwrap_or(i32::MAX);
                let titlebar = crate::session::window_frame::render_titlebar_for(
                    Some(decoration.owner),
                    width,
                    &decoration.title,
                    decoration.background,
                    decoration.foreground,
                );
                let mut framed = Vec::new();
                let titlebar_y =
                    local.1.round() as i32 - crate::session::window_frame::TITLEBAR_HEIGHT;
                if let Some(icons) = frame_icons.as_ref() {
                    let icon_y = titlebar_y + 8;
                    let icon_x = local.0.round() as i32 + width;
                    for (buffer, offset) in [
                        (&icons.close, 35),
                        (
                            if decoration.maximized {
                                &icons.restore
                            } else {
                                &icons.maximize
                            },
                            81,
                        ),
                        (&icons.minimize, 127),
                    ] {
                        if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                            renderer,
                            ((icon_x - offset) as f64, icon_y as f64),
                            buffer,
                            None,
                            None,
                            None,
                            Kind::Unspecified,
                        ) {
                            framed.push(element.into());
                        }
                    }
                }
                if let Some(titlebar) = titlebar
                    && let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        (local.0, f64::from(titlebar_y)),
                        &titlebar,
                        None,
                        None,
                        Some((width, crate::session::window_frame::TITLEBAR_HEIGHT).into()),
                        Kind::Unspecified,
                    )
                {
                    framed.push(element.into());
                }
                framed.append(&mut content);
                Some(framed)
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

    #[test]
    fn immutable_image_hash_reuse_skips_pixels_without_retaining_them() {
        use std::sync::Arc;
        let mut cache = ImageHashes::default();
        let mut image = Arc::new(image::RgbaImage::from_pixel(
            32,
            32,
            image::Rgba([1, 2, 3, 255]),
        ));
        let first = cache.get(&image);
        let bytes = cache.hashed_bytes;
        for _ in 0..100 {
            assert_eq!(cache.get(&image), first);
        }
        assert_eq!(
            cache.hashed_bytes, bytes,
            "unchanged drag frames must not scan image pixels"
        );
        Arc::make_mut(&mut image).put_pixel(0, 0, image::Rgba([9, 8, 7, 255]));
        assert_ne!(cache.get(&image), first);
        assert_eq!(cache.hashed_bytes, bytes * 2);
        let weak = Arc::downgrade(&image);
        drop(image);
        assert!(
            weak.upgrade().is_none(),
            "hash metadata must not pin CPU image buffers"
        );
    }

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
    fn hiding_fallback_surface_releases_frame_sized_storage_until_shown() {
        let mut runtime = InternalUiRuntime::default();
        runtime.set_renderer_mode(InternalUiRendererMode::Software);
        let id = runtime.insert(Label, placement(Some("DP-1")), 1.0);

        assert!(runtime.render_buffer(id).is_some());
        let live = runtime.renderer_diagnostics(id).unwrap();
        assert_eq!(live.software_frame_bytes, 120 * 32 * 4);
        assert_eq!(live.fallback_raster_bytes, 120 * 32 * 4);

        assert!(runtime.set_visible(id, false));
        let hidden = runtime.renderer_diagnostics(id).unwrap();
        assert_eq!(hidden.software_frame_bytes, 0);
        assert_eq!(hidden.fallback_raster_bytes, 0);
        assert_eq!(hidden.fallback_primitive_count, 0);

        assert!(runtime.set_visible(id, true));
        assert!(runtime.render_buffer(id).is_some());
        let shown = runtime.renderer_diagnostics(id).unwrap();
        assert_eq!(shown.software_frame_bytes, 120 * 32 * 4);
        assert_eq!(shown.fallback_raster_bytes, 120 * 32 * 4);
    }

    #[test]
    fn configuring_external_scene_resizes_host_and_scale_without_replacing_identity() {
        let mut runtime = InternalUiRuntime::default();
        let id = runtime.insert_scene(Vec::new(), placement(Some("DP-1")), 1.0);
        let target = InternalSurfacePlacement {
            geometry: (-1920, 592, 1920, 368),
            ..placement(Some("DP-1"))
        };
        assert!(runtime.configure_scene(id, target.clone(), 1.5));
        assert_eq!(runtime.placement(id), Some(&target));
        assert_eq!(runtime.scale_factor(id), Some(1.5));
        assert_eq!(
            runtime.surfaces.get(id).unwrap().logical_size(),
            (1920, 368)
        );
        assert_eq!(runtime.surfaces.get(id).unwrap().scale_factor(), 1.5);
        assert_eq!(runtime.surfaces.ids().collect::<Vec<_>>(), vec![id]);
        assert!(!runtime.configure_scene(id, target, 1.5));
        assert!(runtime.drain_routed_events().is_empty());
    }

    #[test]
    fn output_agnostic_surfaces_are_visible_on_every_output() {
        let mut runtime = InternalUiRuntime::default();
        let id = runtime.insert(Label, placement(None), 1.0);
        assert_eq!(runtime.ids_for_output("DP-1").next(), Some(id));
        assert_eq!(runtime.ids_for_output("HDMI-A-1").next(), Some(id));
    }

    #[test]
    fn relocation_preserves_surface_identity_across_outputs() {
        let mut runtime = InternalUiRuntime::default();
        let id = runtime.insert(Label, placement(Some("DP-1")), 1.0);
        let moved = InternalSurfacePlacement {
            role: InternalSurfaceRole::Panel,
            geometry: (-1910, 220, 120, 32),
            output: Some("HDMI-A-1".into()),
        };

        assert!(runtime.relocate(id, moved.clone()));
        assert_eq!(runtime.placement(id), Some(&moved));
        assert_eq!(
            runtime.ids_for_output("HDMI-A-1").collect::<Vec<_>>(),
            vec![id]
        );
        assert!(runtime.ids_for_output("DP-1").next().is_none());
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

    struct SubmitCounter(usize);
    impl Application for SubmitCounter {
        type Message = ();
        fn update(&mut self, (): ()) {}
        fn shortcut(&mut self, shortcut: nickel_ui::Shortcut) -> bool {
            if shortcut != nickel_ui::Shortcut::Submit {
                return false;
            }
            self.0 += 1;
            true
        }
        fn view(&self, _: ViewContext) -> impl View<Self::Message> {
            Text::new("submit")
        }
    }

    #[test]
    fn focused_internal_surface_receives_submit_before_activation_fallback() {
        let mut runtime = InternalUiRuntime::default();
        let id = runtime.insert(
            SubmitCounter(0),
            InternalSurfacePlacement {
                role: InternalSurfaceRole::Application,
                geometry: (0, 0, 320, 200),
                output: None,
            },
            1.0,
        );
        assert!(runtime.focus_surface(id));

        assert!(runtime.submit_or_activate());
        assert_eq!(runtime.application::<SubmitCounter>(id).unwrap().0, 1);
    }

    #[test]
    fn application_decoration_owns_titlebar_and_window_buttons() {
        let mut runtime = InternalUiRuntime::default();
        let id = runtime.insert(
            Counter(0),
            InternalSurfacePlacement {
                role: InternalSurfaceRole::Application,
                geometry: (100, 80, 460, 240),
                output: None,
            },
            1.0,
        );
        assert!(runtime.set_window_decoration(
            id,
            InternalWindowDecoration {
                owner: 7,
                title: "Hosted app".into(),
                active: true,
                maximized: false,
                background: 0xff20_2020,
                foreground: 0xffff_ffff,
            }
        ));

        assert_eq!(
            runtime.internal_frame_target((120.0, 60.0)),
            Some((id, crate::session::window_frame::FramePart::Titlebar))
        );
        assert_eq!(
            runtime.internal_frame_target((550.0, 60.0)),
            Some((id, crate::session::window_frame::FramePart::Close))
        );
        assert_eq!(runtime.internal_frame_target((120.0, 100.0)), None);
    }

    #[test]
    fn coordinator_scene_preserves_normalized_input_and_focus_batches() {
        let mut runtime = InternalUiRuntime::default();
        let desktop = runtime.insert_scene(
            Vec::new(),
            InternalSurfacePlacement {
                role: InternalSurfaceRole::Desktop,
                geometry: (-800, -120, 800, 600),
                output: Some("left".into()),
            },
            1.5,
        );
        let input = nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
            device: nickel_input::DeviceId(7),
            order: nickel_input::EventOrder(29),
            button: nickel_input::PointerButton::Secondary,
            edge: nickel_input::KeyEdge::Released,
            position: Some(nickel_input::Point { x: 13.0, y: 71.0 }),
        });
        assert!(runtime.step(
            desktop,
            HostBatch {
                events: vec![HostEvent::Normalized {
                    input: input.clone(),
                    clipboard_text: None
                }],
                ..Default::default()
            }
        ));
        runtime.focus_surface(desktop);
        runtime.clear_focus();
        let batches = runtime.drain_routed_events();
        assert_eq!(batches.len(), 3);
        assert!(batches.iter().all(|(id, _, _)| *id == desktop));
        assert!(
            matches!(&batches[0].1.events[..], [HostEvent::Normalized { input: actual, .. }] if actual == &input)
        );
        assert_eq!(batches[1].1.window_focused, Some(true));
        assert_eq!(batches[2].1.window_focused, Some(false));
        assert!(runtime.drain_routed_events().is_empty());
    }

    #[test]
    fn hosted_application_input_is_not_duplicated_into_coordinator_queue() {
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
        runtime.focus_surface(application);
        runtime.keyboard(UiEvent::KeyboardActivate);
        runtime.clear_focus();
        assert!(runtime.drain_routed_events().is_empty());
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
        assert!(runtime.surface_at((10.0, 60.0), true).is_none());
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
        assert!(runtime.pointer_button((10.0, 10.0), true));
        assert_eq!(runtime.focused(), Some(application));
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
    fn focus_transfer_and_hide_blur_the_previous_host() {
        let mut runtime = InternalUiRuntime::default();
        let placement = |x| InternalSurfacePlacement {
            role: InternalSurfaceRole::Application,
            geometry: (x, 0, 100, 100),
            output: None,
        };
        let first = runtime.insert(Counter(0), placement(0), 1.0);
        let second = runtime.insert(Counter(0), placement(100), 1.0);

        assert!(runtime.focus_surface(first));
        assert!(
            runtime
                .surfaces
                .get(first)
                .unwrap()
                .inspect()
                .window_focused
        );

        assert!(runtime.focus_surface(second));
        assert!(
            !runtime
                .surfaces
                .get(first)
                .unwrap()
                .inspect()
                .window_focused
        );
        assert!(
            runtime
                .surfaces
                .get(second)
                .unwrap()
                .inspect()
                .window_focused
        );

        assert_eq!(runtime.clear_focus(), Some(second));
        assert_eq!(runtime.focused(), None);
        assert!(
            !runtime
                .surfaces
                .get(second)
                .unwrap()
                .inspect()
                .window_focused
        );

        assert!(runtime.focus_surface(first));
        assert!(runtime.set_visible(first, false));
        assert_eq!(runtime.focused(), None);
        assert!(
            !runtime
                .surfaces
                .get(first)
                .unwrap()
                .inspect()
                .window_focused
        );
    }

    #[test]
    fn desktop_is_below_the_complete_client_scene_but_system_surfaces_remain_above_it() {
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
        // `client_present` includes compositor-owned server decorations, not
        // only the client's wl_surface. A titlebar click must therefore pass
        // through the desktop and reach the frame dispatcher.
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
            vec![overlay, panel]
        );
        assert_eq!(
            runtime.ordered_ids_for_layer("nested", Some(InternalSurfaceLayer::Application)),
            vec![application]
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
        assert!(renderer.software.is_none());
        assert_eq!(renderer.diagnostics().software_frame_bytes, 0);
        assert_eq!(renderer.diagnostics().gpu_frames, 1);
    }

    #[test]
    fn gpu_renderer_does_not_eagerly_allocate_a_software_framebuffer() {
        let renderer = SmithayFrameRenderer::new(3840, 2160, 1.0, InternalUiRendererMode::Gpu);

        assert!(renderer.software.is_none());
        assert_eq!(renderer.diagnostics().software_frame_bytes, 0);
        assert_eq!(renderer.diagnostics().fallback_raster_bytes, 0);
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
        assert!(renderer.software.is_some());
        assert_eq!(renderer.diagnostics().software_frame_bytes, 40 * 30 * 4);
        assert_eq!(renderer.diagnostics().fallback_raster_bytes, 40 * 30 * 4);
        assert_eq!(renderer.diagnostics().gpu_frames, 0);
        assert_eq!(renderer.diagnostics().fallback_frames, 1);
    }

    #[test]
    fn successful_gpu_frame_releases_software_fallback_storage() {
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

        renderer.renderer_mode = InternalUiRendererMode::Gpu;
        renderer
            .render_frame(RenderFrame {
                commands: &commands,
                logical_size: (40, 30),
                scale_factor: 1.0,
                generation: 2,
            })
            .unwrap();

        assert_eq!(renderer.mode(), InternalUiPresentationMode::GpuSolid);
        assert!(renderer.software.is_none());
        assert!(renderer.raster.is_none());
        assert_eq!(renderer.diagnostics().software_frame_bytes, 0);
        assert_eq!(renderer.diagnostics().fallback_raster_bytes, 0);
    }

    #[test]
    fn full_output_rounded_fill_coalesces_middle_and_stays_on_gpu() {
        let commands = [PaintCommand::RoundedFill {
            rect: nickel_ui::Rect::new(0.0, 0.0, 1920.0, 1080.0),
            color: 0x336699,
            radius: 8.0,
        }];
        let mut renderer = SmithayFrameRenderer::new(1920, 1080, 1.0, InternalUiRendererMode::Gpu);

        renderer
            .render_frame(RenderFrame {
                commands: &commands,
                logical_size: (1920, 1080),
                scale_factor: 1.0,
                generation: 1,
            })
            .unwrap();

        assert_eq!(renderer.mode(), InternalUiPresentationMode::GpuSolid);
        assert!(renderer.raster.is_none());
        assert_eq!(renderer.primitives.len(), 17);
        let GpuPrimitive::Solid(first, _) = &renderer.primitives[0] else {
            panic!("rounded row should be a solid")
        };
        assert!(first.size.width < 1920.0);
        assert!(renderer.primitives.iter().any(|primitive| matches!(
            primitive,
            GpuPrimitive::Solid(rect, _) if rect.size.width == 1920.0 && rect.size.height > 1.0
        )));
    }

    #[test]
    fn element_heavy_scene_uses_bounded_fallback() {
        let commands = [PaintCommand::Gradient {
            rect: nickel_ui::Rect::new(0.0, 0.0, 20.0, (MAX_GPU_ELEMENTS_PER_SURFACE + 1) as f32),
            gradient: LinearGradient {
                start: 0xff336699,
                end: 0xff112233,
                axis: GradientAxis::Vertical,
            },
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
    fn text_maps_the_complete_pixel_rounded_texture_at_fractional_bounds() {
        let commands = [PaintCommand::Text {
            bounds: nickel_ui::Rect::new(7.25, 3.5, 153.6, 17.2),
            text: "Fractional label".into(),
            scale: 1.0,
            color: 0xff336699,
            align: nickel_ui::TextAlign::Start,
            bold: false,
            wrap: false,
        }];
        let mut renderer = SmithayFrameRenderer::new(200, 40, 1.0, InternalUiRendererMode::Gpu);

        renderer
            .render_frame(RenderFrame {
                commands: &commands,
                logical_size: (200, 40),
                scale_factor: 1.0,
                generation: 1,
            })
            .unwrap();

        let GpuPrimitive::Texture { source, .. } = &renderer.primitives[0] else {
            panic!("text should produce a texture")
        };
        assert_eq!(source.loc, (0.0, 0.0).into());
        assert_eq!(source.size, (154.0, 18.0).into());
    }

    #[test]
    fn text_upload_preserves_antialiased_glyph_coverage() {
        use super::memory_test_renderer::MemoryTestRenderer;
        use smithay::backend::renderer::element::{RenderElement, UnderlyingStorage};
        let commands = [PaintCommand::Text {
            bounds: nickel_ui::Rect::new(0.0, 0.0, 160.0, 32.0),
            text: "catering.html".into(),
            scale: 1.0,
            color: 0xffffff,
            align: nickel_ui::TextAlign::Start,
            bold: false,
            wrap: false,
        }];
        for mode in [
            InternalUiRendererMode::Gpu,
            InternalUiRendererMode::Software,
        ] {
            let mut owner = SmithayFrameRenderer::new(160, 32, 1.0, mode);
            owner
                .render_frame(RenderFrame {
                    commands: &commands,
                    logical_size: (160, 32),
                    scale_factor: 1.0,
                    generation: 1,
                })
                .unwrap();
            let mut backend = MemoryTestRenderer::default();
            let elements = owner.elements(&mut backend, (0, 0).into(), (160, 32));
            let UnderlyingStorage::Memory(bytes) =
                elements[0].underlying_storage(&mut backend).unwrap()
            else {
                panic!("expected uploaded glyph pixels")
            };
            let mut antialiased = 0;
            for pixel in bytes.chunks_exact(4) {
                if pixel[3] > 0 && pixel[3] < 255 {
                    antialiased += 1;
                    // White glyphs in premultiplied storage have RGB equal to
                    // coverage, not coverage squared.
                    assert_eq!(
                        &pixel[..3],
                        &[pixel[3]; 3],
                        "glyph coverage changed in {mode:?}"
                    );
                }
            }
            assert!(antialiased > 0, "exercise partially covered glyph edges");
        }
    }

    #[test]
    fn presented_text_keeps_one_texel_per_output_pixel_when_clipped_or_scaled() {
        use super::memory_test_renderer::MemoryTestRenderer;
        for scale in [1.0_f32, 1.25, 1.5, 2.0] {
            for clipped in [false, true] {
                let mut commands = Vec::new();
                if clipped {
                    commands.push(PaintCommand::PushClip(nickel_ui::Rect::new(
                        12.75, 5.25, 120.3, 13.2,
                    )));
                }
                commands.push(PaintCommand::Text {
                    bounds: nickel_ui::Rect::new(7.25, 3.5, 153.6, 17.2),
                    text: "Google Chrome".into(),
                    scale: 1.0,
                    color: 0xffffff,
                    align: nickel_ui::TextAlign::Center,
                    bold: false,
                    wrap: false,
                });
                if clipped {
                    commands.push(PaintCommand::PopClip);
                }
                let mut owner =
                    SmithayFrameRenderer::new(200, 40, scale, InternalUiRendererMode::Gpu);
                let mut renderer = MemoryTestRenderer::default();
                for generation in [1, 2] {
                    owner
                        .render_frame(RenderFrame {
                            commands: &commands,
                            logical_size: (200, 40),
                            scale_factor: scale,
                            generation,
                        })
                        .unwrap();
                    let elements = owner.elements(&mut renderer, (19, 11).into(), (200, 40));
                    assert_eq!(elements.len(), 1);
                    let source = elements[0].src();
                    let destination = elements[0].geometry(f64::from(scale).into());
                    assert_eq!(source.loc.x.fract(), 0.0);
                    assert_eq!(source.loc.y.fract(), 0.0);
                    assert_eq!(
                        source.size.w,
                        f64::from(destination.size.w),
                        "horizontal resampling at {scale}, clipped={clipped}"
                    );
                    assert_eq!(
                        source.size.h,
                        f64::from(destination.size.h),
                        "vertical resampling at {scale}, clipped={clipped}"
                    );
                }
            }
        }
    }

    #[test]
    fn clipped_text_crops_on_the_raster_pixel_grid() {
        let bounds = nickel_ui::Rect::new(10.0, 5.0, 10.5, 8.5);
        let rect = nickel_ui::Rect::new(12.0, 7.0, 5.0, 4.0);

        assert_eq!(
            text_source_rect(rect, bounds, 1.25, 14, 11),
            Rectangle::new((3.0, 3.0).into(), (6.0, 5.0).into(),)
        );
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
    fn failed_gpu_texture_import_can_restore_the_complete_image_frame() {
        use std::sync::Arc;

        let image = Arc::new(image::RgbaImage::from_pixel(
            4,
            4,
            image::Rgba([220, 70, 40, 255]),
        ));
        let commands = [
            PaintCommand::Fill {
                rect: nickel_ui::Rect::new(0.0, 0.0, 12.0, 12.0),
                color: 0x101010,
            },
            PaintCommand::Image {
                bounds: nickel_ui::Rect::new(4.0, 4.0, 4.0, 4.0),
                id: 99,
                generation: 1,
                image,
                high_density: None,
            },
        ];
        let mut renderer = SmithayFrameRenderer::new(12, 12, 1.0, InternalUiRendererMode::Gpu);
        renderer
            .render_frame(RenderFrame {
                commands: &commands,
                logical_size: (12, 12),
                scale_factor: 1.0,
                generation: 1,
            })
            .unwrap();

        assert_eq!(renderer.mode(), InternalUiPresentationMode::GpuSolid);
        assert!(renderer.activate_import_fallback());
        assert_eq!(renderer.mode(), InternalUiPresentationMode::RasterFallback);
        assert!(renderer.raster.is_some());
        assert!(renderer.primitives.is_empty());
        assert_eq!(renderer.diagnostics().fallback_image_count, 1);
        assert_eq!(renderer.diagnostics().fallback_primitive_count, 2);
    }

    #[test]
    fn compositor_surfaces_share_one_image_texture_cache() {
        use std::sync::Arc;

        let caches = SharedTextureCaches::default();
        let image = Arc::new(image::RgbaImage::from_pixel(
            4,
            4,
            image::Rgba([20, 40, 60, 255]),
        ));
        let commands = [PaintCommand::Image {
            bounds: nickel_ui::Rect::new(0.0, 0.0, 4.0, 4.0),
            id: 7,
            generation: 3,
            image,
            high_density: None,
        }];
        let mut desktop =
            SmithayFrameRenderer::with_caches(1.0, InternalUiRendererMode::Gpu, &caches);
        let mut panel =
            SmithayFrameRenderer::with_caches(1.0, InternalUiRendererMode::Gpu, &caches);

        for renderer in [&mut desktop, &mut panel] {
            renderer
                .render_frame(RenderFrame {
                    commands: &commands,
                    logical_size: (4, 4),
                    scale_factor: 1.0,
                    generation: 1,
                })
                .unwrap();
        }

        assert_eq!(desktop.diagnostics().image_cache_misses, 1);
        assert_eq!(desktop.diagnostics().image_uploads, 1);
        assert_eq!(panel.diagnostics().image_cache_hits, 1);
        assert_eq!(panel.diagnostics().image_uploads, 0);
        assert_eq!(caches.images.borrow().entries.len(), 1);
        assert_eq!(caches.images.borrow().bytes, 4 * 4 * 4);
    }

    #[test]
    fn smithay_texture_pixels_are_premultiplied_without_changing_alpha() {
        assert_eq!(premultiplied_pixel([200, 100, 50, 128]), [100, 50, 25, 128]);
        assert_eq!(premultiplied_pixel([20, 30, 40, 0]), [0, 0, 0, 0]);
        assert_eq!(premultiplied_pixel([20, 30, 40, 255]), [20, 30, 40, 255]);
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
    fn image_cache_key_rejects_changed_content_but_shares_destination_scales() {
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
        for (commands, scale) in [make(1, 1.0), make(2, 1.0), make(2, 1.25), make(2, 2.0)] {
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
        assert_eq!(diagnostics.image_cache_misses, 2);
        assert_eq!(diagnostics.image_uploads, 2);
        assert_eq!(diagnostics.image_cache_entries, 2);
        assert_eq!(diagnostics.image_cache_hits, 2);
    }

    #[test]
    fn fallback_reuses_storage_and_preserves_imported_snapshot_pixels() {
        use smithay::backend::renderer::element::{Element, RenderElement, UnderlyingStorage};

        let mut renderer =
            SmithayFrameRenderer::new(80, 60, 1.25, InternalUiRendererMode::Software);
        let mut backend = memory_test_renderer::MemoryTestRenderer::default();
        let mut commands = vec![
            PaintCommand::Fill {
                rect: nickel_ui::Rect::new(0.0, 0.0, 80.0, 60.0),
                color: 0xff112233,
            },
            PaintCommand::Fill {
                rect: nickel_ui::Rect::new(2.25, 3.5, 4.5, 5.25),
                color: 0x80446688,
            },
        ];
        fn frame(commands: &[PaintCommand]) -> RenderFrame<'_> {
            RenderFrame {
                commands,
                logical_size: (80, 60),
                scale_factor: 1.25,
                generation: 1,
            }
        }
        renderer.prepare_fallback(frame(&commands));
        let snapshot = MemoryRenderBufferRenderElement::from_buffer(
            &mut backend,
            (0.0, 0.0),
            renderer.raster.as_ref().unwrap(),
            None,
            None,
            None,
            Kind::Unspecified,
        )
        .unwrap();
        let original = match snapshot.underlying_storage(&mut backend).unwrap() {
            UnderlyingStorage::Memory(bytes) => bytes.to_vec(),
            _ => panic!("expected CPU snapshot"),
        };
        let before = renderer.diagnostics();
        assert!(renderer.prepare_fallback(frame(&commands)).is_empty());
        assert_eq!(
            renderer.diagnostics().fallback_converted_bytes,
            before.fallback_converted_bytes
        );
        assert_eq!(renderer.diagnostics().fallback_buffer_creations, 1);
        let unchanged = MemoryRenderBufferRenderElement::from_buffer(
            &mut backend,
            (0.0, 0.0),
            renderer.raster.as_ref().unwrap(),
            None,
            None,
            None,
            Kind::Unspecified,
        )
        .unwrap();
        assert_eq!(snapshot.id(), unchanged.id());
        assert_eq!(backend.imports, 1);
        assert!(backend.updates.is_empty(), "unchanged frames do not upload");
        for step in 0..12 {
            commands[1] = PaintCommand::Fill {
                rect: nickel_ui::Rect::new(2.25 + step as f32, 3.5, 4.5, 5.25),
                color: 0x40336699 + step,
            };
            renderer.prepare_fallback(frame(&commands));
            let mut reference = SoftwareRenderer::new(100, 75, 1.25);
            reference.render(&commands);
            let expected: Vec<_> = reference
                .pixels()
                .iter()
                .flat_map(|pixel| [pixel.r, pixel.g, pixel.b, pixel.a])
                .collect();
            renderer
                .raster
                .as_mut()
                .unwrap()
                .render()
                .draw(|bytes| {
                    assert_eq!(bytes, expected.as_slice());
                    Ok::<_, std::convert::Infallible>(Vec::new())
                })
                .unwrap();
            let presented = MemoryRenderBufferRenderElement::from_buffer(
                &mut backend,
                (0.0, 0.0),
                renderer.raster.as_ref().unwrap(),
                None,
                None,
                None,
                Kind::Unspecified,
            )
            .unwrap();
            assert_eq!(presented.id(), snapshot.id());
            let upload = backend
                .updates
                .last()
                .expect("changed frame updates texture");
            assert!(
                upload.size.w * upload.size.h < 100 * 75,
                "partial frames avoid full texture uploads"
            );
        }
        let updated = MemoryRenderBufferRenderElement::from_buffer(
            &mut backend,
            (0.0, 0.0),
            renderer.raster.as_ref().unwrap(),
            None,
            None,
            None,
            Kind::Unspecified,
        )
        .unwrap();
        assert_eq!(
            snapshot.id(),
            updated.id(),
            "same import resource survives partial updates"
        );
        match snapshot.underlying_storage(&mut backend).unwrap() {
            UnderlyingStorage::Memory(bytes) => assert_eq!(&**bytes, original.as_slice()),
            _ => panic!("expected CPU snapshot"),
        }
        assert_eq!(renderer.diagnostics().fallback_buffer_creations, 1);
        assert_eq!(renderer.diagnostics().fallback_buffer_reuses, 12);
        assert_eq!(backend.imports, 1);
        assert_eq!(backend.updates.len(), 12);
        let mut replacement = memory_test_renderer::MemoryTestRenderer::default();
        let replaced = MemoryRenderBufferRenderElement::from_buffer(
            &mut replacement,
            (0.0, 0.0),
            renderer.raster.as_ref().unwrap(),
            None,
            None,
            None,
            Kind::Unspecified,
        )
        .unwrap();
        assert_eq!(replaced.id(), updated.id());
        assert_eq!(
            replacement.imports, 1,
            "replacement context imports complete current pixels"
        );
        assert!(replacement.updates.is_empty());
        assert_eq!(renderer.diagnostics().fallback_partial_repaints, 12);
        assert!(
            renderer.diagnostics().fallback_converted_bytes < before.fallback_converted_bytes * 2
        );
    }

    #[test]
    fn mixed_dpi_image_owners_share_selected_sources() {
        use std::sync::Arc;
        let caches = SharedTextureCaches::default();
        let commands = [PaintCommand::Image {
            bounds: nickel_ui::Rect::new(1.25, 2.5, 4.0, 4.0),
            id: 7,
            generation: 1,
            image: Arc::new(image::RgbaImage::from_pixel(
                4,
                4,
                image::Rgba([255, 0, 0, 255]),
            )),
            high_density: Some(Arc::new(image::RgbaImage::from_pixel(
                8,
                8,
                image::Rgba([0, 255, 0, 255]),
            ))),
        }];
        for scale in [1.0, 1.25, 2.0, 2.5] {
            let mut owner =
                SmithayFrameRenderer::with_caches(scale, InternalUiRendererMode::Gpu, &caches);
            owner
                .render_frame(RenderFrame {
                    commands: &commands,
                    logical_size: (20, 20),
                    scale_factor: scale,
                    generation: 1,
                })
                .unwrap();
            let GpuPrimitive::Texture { buffer, source, .. } = &mut owner.primitives[0] else {
                panic!("expected image texture")
            };
            let (dimension, pixel) = if scale < 1.5 {
                (4.0, [255, 0, 0, 255])
            } else {
                (8.0, [0, 255, 0, 255])
            };
            assert_eq!(source.size, (dimension, dimension).into());
            buffer
                .render()
                .draw(|bytes| {
                    assert!(bytes.chunks_exact(4).all(|actual| actual == pixel));
                    Ok::<_, std::convert::Infallible>(Vec::new())
                })
                .unwrap();
        }
        assert_eq!(caches.images.borrow().entries.len(), 2);
        assert_eq!(caches.images.borrow().bytes, (4 * 4 + 8 * 8) * 4);
    }

    #[test]
    fn fallback_damage_clips_and_rounds_disjoint_regions_outward() {
        let damage = DamageRegion {
            rects: [
                nickel_ui::Rect::new(-1.2, 2.2, 4.4, 2.1),
                nickel_ui::Rect::new(8.5, 7.2, 5.0, 5.0),
            ]
            .into_iter()
            .collect(),
        };
        assert_eq!(
            fallback_damage_regions(&damage, 10, 10),
            vec![
                Rectangle::new((0, 2).into(), (4, 3).into()),
                Rectangle::new((8, 7).into(), (2, 3).into()),
            ]
        );
    }

    #[test]
    fn fallback_scale_resize_and_suspend_require_complete_repaint() {
        let commands = [PaintCommand::Fill {
            rect: nickel_ui::Rect::new(1.0, 1.0, 2.0, 2.0),
            color: 0x804488cc,
        }];
        let mut renderer = SmithayFrameRenderer::new(10, 10, 1.0, InternalUiRendererMode::Software);
        for (size, scale) in [((10, 10), 1.0), ((5, 5), 2.0), ((20, 20), 2.0)] {
            renderer.prepare_fallback(RenderFrame {
                commands: &commands,
                logical_size: size,
                scale_factor: scale,
                generation: 1,
            });
        }
        assert_eq!(renderer.diagnostics().fallback_buffer_creations, 3);
        assert_eq!(renderer.diagnostics().fallback_full_repaints, 3);
        renderer.suspend();
        assert_eq!(renderer.diagnostics().fallback_raster_bytes, 0);
        renderer.prepare_fallback(RenderFrame {
            commands: &commands,
            logical_size: (20, 20),
            scale_factor: 2.0,
            generation: 1,
        });
        assert_eq!(renderer.diagnostics().fallback_full_repaints, 4);
    }

    #[test]
    fn hiding_text_owner_releases_private_scratch_and_preserves_shared_textures() {
        let caches = SharedTextureCaches::default();
        let mut owner =
            SmithayFrameRenderer::with_caches(1.0, InternalUiRendererMode::Gpu, &caches);
        let mut peer = SmithayFrameRenderer::with_caches(1.0, InternalUiRendererMode::Gpu, &caches);
        let commands = [PaintCommand::Text {
            bounds: nickel_ui::Rect::new(0.0, 0.0, 1024.0, 256.0),
            text: "A persistent menu".into(),
            scale: 1.0,
            color: 0xffffffff,
            align: nickel_ui::TextAlign::Start,
            bold: false,
            wrap: false,
        }];
        for _ in 0..8 {
            owner
                .render_frame(RenderFrame {
                    commands: &commands,
                    logical_size: (1024, 256),
                    scale_factor: 1.0,
                    generation: 1,
                })
                .unwrap();
            peer.render_frame(RenderFrame {
                commands: &commands,
                logical_size: (1024, 256),
                scale_factor: 1.0,
                generation: 1,
            })
            .unwrap();
            assert!(!peer.primitives.is_empty());
            let shared_bytes = caches.text.borrow().bytes;
            owner.suspend();
            assert_eq!(owner.diagnostics().text_scratch_bytes, 4);
            assert_eq!(owner.diagnostics().text_private_cache_bytes, 0);
            assert_eq!(caches.text.borrow().bytes, shared_bytes);
            assert!(!peer.primitives.is_empty());
        }
        assert_eq!(peer.diagnostics().text_uploads, 0);
    }

    #[test]
    fn unusually_large_text_scratch_is_retired_while_visible() {
        let mut renderer = SmithayFrameRenderer::new(2048, 1025, 1.0, InternalUiRendererMode::Gpu);
        let commands = [PaintCommand::Text {
            bounds: nickel_ui::Rect::new(0.0, 0.0, 2048.0, 1025.0),
            text: "One unusually large label".into(),
            scale: 1.0,
            color: 0xffffffff,
            align: nickel_ui::TextAlign::Start,
            bold: false,
            wrap: false,
        }];
        renderer
            .render_frame(RenderFrame {
                commands: &commands,
                logical_size: (2048, 1025),
                scale_factor: 1.0,
                generation: 1,
            })
            .unwrap();
        assert_eq!(renderer.diagnostics().text_scratch_bytes, 4);
        assert_eq!(renderer.diagnostics().text_private_cache_bytes, 0);
        assert_eq!(
            renderer.primitives.len(),
            1,
            "uploaded texture survives scratch retirement"
        );
    }

    #[test]
    #[ignore = "release workload measurement; run with --release --ignored --nocapture"]
    fn measure_fallback_damage_workloads() {
        for (width, height) in [(1920, 1080), (3840, 2160)] {
            let mut renderer =
                SmithayFrameRenderer::new(width, height, 1.0, InternalUiRendererMode::Software);
            let mut commands = [
                PaintCommand::Fill {
                    rect: nickel_ui::Rect::new(0.0, 0.0, width as f32, height as f32),
                    color: 0xff112233,
                },
                PaintCommand::Fill {
                    rect: nickel_ui::Rect::new(10.0, 10.0, 16.0, 16.0),
                    color: 0xff445566,
                },
            ];
            for full in [false, true] {
                let start = std::time::Instant::now();
                let before = renderer.diagnostics();
                for index in 0..120 {
                    if let PaintCommand::Fill { color, .. } = &mut commands[usize::from(!full)] {
                        *color = 0xff112200 + index;
                    }
                    renderer.prepare_fallback(RenderFrame {
                        commands: &commands,
                        logical_size: (width, height),
                        scale_factor: 1.0,
                        generation: index as u64,
                    });
                }
                let after = renderer.diagnostics();
                eprintln!(
                    "{width}x{height} full={full} frames=120 elapsed={:?} owned_cpu_bytes={} creations={} converted_bytes={} submitted_damage_bytes={}",
                    start.elapsed(),
                    after.software_frame_bytes + after.fallback_raster_bytes,
                    after.fallback_buffer_creations - before.fallback_buffer_creations,
                    after.fallback_converted_bytes - before.fallback_converted_bytes,
                    after.fallback_upload_damage_bytes - before.fallback_upload_damage_bytes
                );
            }
        }
    }
}
