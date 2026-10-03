//! Hardware presentation for Windows shell surfaces.
//!
//! Geometry is drawn by the GPU. Text is rasterized into bounded, cached
//! textures, as in the Linux compositor; images are uploaded once per revision.
//! A softbuffer presenter remains available if the graphics device is lost.

use std::{cell::RefCell, collections::HashMap, sync::Arc};

use nickel_ui::backend::PaintCommand;
use nickel_ui::{DamageRegion, GradientAxis, PresenterCacheDiagnostics, Rect, SoftwareRenderer};
use raw_window_handle::{DisplayHandle, WindowHandle};

use crate::softbuffer_presenter::{
    PresentationGeometry, SharedGraphics as SoftwareGraphics,
    SoftbufferPresenter as SoftwarePresenter,
};

const SHADER: &str = r#"
struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};
struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
@vertex fn vs_main(input: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(input.position, 0.0, 1.0);
    out.uv = input.uv;
    out.color = input.color;
    return out;
}
@fragment fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    return textureSample(image, image_sampler, input.uv) * input.color;
}
"#;
const MAX_TEXTURE_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_RETAINED_FRAMEBUFFER_BYTES: usize = 128 * 1024 * 1024;

fn retained_framebuffer_bytes(width: u32, height: u32) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)
        .filter(|bytes| *bytes <= MAX_RETAINED_FRAMEBUFFER_BYTES)
}

fn retained_framebuffer_configuration(
    width: u32,
    height: u32,
    scale: f32,
    format: wgpu::TextureFormat,
) -> (u32, u32, u32, wgpu::TextureFormat) {
    (width, height, scale.to_bits(), format)
}

#[derive(Default)]
struct TextureCache {
    entries: HashMap<String, Arc<wgpu::BindGroup>>,
    bytes: usize,
    peak_bytes: usize,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

struct GpuGraphics {
    instance: wgpu::Instance,
    display: raw_window_handle::RawDisplayHandle,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    shader: wgpu::ShaderModule,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    white: Arc<wgpu::BindGroup>,
    textures: RefCell<TextureCache>,
    text_renderer: RefCell<SoftwareRenderer>,
}

pub struct SharedGraphics {
    gpu: Option<GpuGraphics>,
    software: SoftwareGraphics,
}

impl SharedGraphics {
    /// # Safety
    /// The display and window remain valid for the lifetime of all presenters.
    pub unsafe fn new(
        display: DisplayHandle<'_>,
        window: WindowHandle<'_>,
    ) -> Result<Self, String> {
        let software = unsafe { SoftwareGraphics::new(display) }?;
        let gpu = match unsafe { GpuGraphics::new(display, window) } {
            Ok(gpu) => {
                tracing::info!("Windows shell GPU renderer initialized");
                Some(gpu)
            }
            Err(error) => {
                tracing::warn!(%error, "Windows shell GPU unavailable; using shared memory");
                None
            }
        };
        Ok(Self { gpu, software })
    }

    pub fn cache_diagnostics(&self) -> PresenterCacheDiagnostics {
        let mut diagnostics = self.software.cache_diagnostics();
        if let Some(gpu) = &self.gpu {
            let cache = gpu.textures.borrow();
            diagnostics.image_textures += cache.entries.len();
            diagnostics.live_bytes = diagnostics.live_bytes.saturating_add(cache.bytes);
            diagnostics.peak_bytes = diagnostics.peak_bytes.saturating_add(cache.peak_bytes);
        }
        diagnostics
    }
}

impl GpuGraphics {
    unsafe fn new(display: DisplayHandle<'_>, window: WindowHandle<'_>) -> Result<Self, String> {
        let instance = wgpu::Instance::default();
        // SAFETY: the caller guarantees the native window and display outlive
        // this temporary compatibility surface and subsequent presenters.
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(display.as_raw()),
                raw_window_handle: window.as_raw(),
            })
        }
        .map_err(|error| error.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: true,
        }))
        .map_err(|error| error.to_string())?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Nickel shell GPU"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Nickel shell primitives"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Nickel shell texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Nickel shell texture sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let white = upload_texture(&device, &queue, &texture_layout, &sampler, 1, 1, &[255; 4]);
        Ok(Self {
            instance,
            display: display.as_raw(),
            adapter,
            device,
            queue,
            shader,
            texture_layout,
            sampler,
            white,
            textures: RefCell::new(TextureCache::default()),
            text_renderer: RefCell::new(SoftwareRenderer::new_pixel_buffer(1, 1, 1.0)),
        })
    }

    fn texture(&self, key: String, width: u32, height: u32, pixels: &[u8]) -> Arc<wgpu::BindGroup> {
        if let Some(texture) = self.textures.borrow().entries.get(&key) {
            return texture.clone();
        }
        let texture = upload_texture(
            &self.device,
            &self.queue,
            &self.texture_layout,
            &self.sampler,
            width,
            height,
            pixels,
        );
        let texture_bytes = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        if texture_bytes > MAX_TEXTURE_CACHE_BYTES {
            return texture;
        }
        let mut cache = self.textures.borrow_mut();
        if cache.bytes.saturating_add(texture_bytes) > MAX_TEXTURE_CACHE_BYTES {
            cache.entries.clear();
            cache.bytes = 0;
        }
        cache.bytes += texture_bytes;
        cache.peak_bytes = cache.peak_bytes.max(cache.bytes);
        cache.entries.insert(key, texture.clone());
        texture
    }

    fn text_texture(
        &self,
        command: &PaintCommand,
        bounds: Rect,
        scale: f32,
    ) -> Arc<wgpu::BindGroup> {
        let mut local = command.clone();
        match &mut local {
            PaintCommand::Text { bounds, .. } | PaintCommand::StyledText { bounds, .. } => {
                bounds.origin.x = 0.0;
                bounds.origin.y = 0.0;
            }
            _ => unreachable!(),
        }
        let key = format!("text:{scale:?}:{local:?}");
        if let Some(texture) = self.textures.borrow().entries.get(&key) {
            return texture.clone();
        }
        let width = (bounds.size.width * scale).ceil().max(1.0) as u32;
        let height = (bounds.size.height * scale).ceil().max(1.0) as u32;
        let mut renderer = self.text_renderer.borrow_mut();
        renderer.resize(width, height, scale);
        renderer.invalidate();
        renderer.render(&[local]);
        let mut bytes = Vec::with_capacity(renderer.pixels().len() * 4);
        for pixel in renderer.pixels() {
            bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
        }
        if renderer.pixel_capacity_bytes() > 8 * 1024 * 1024 {
            renderer.suspend();
        }
        drop(renderer);
        self.texture(key, width, height, &bytes)
    }
}

fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Arc<wgpu::BindGroup> {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Nickel shell cached texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Nickel shell texture"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    }))
}

pub struct SoftbufferPresenter {
    gpu: Option<GpuPresenter>,
    software: SoftwarePresenter,
}

impl SoftbufferPresenter {
    pub fn retained_gpu_bytes(&self) -> Option<u64> {
        self.retained_framebuffer_diagnostics()
            .map(|diagnostics| diagnostics.live_bytes as u64)
    }

    /// # Safety
    /// The native window remains valid until this presenter is dropped.
    pub unsafe fn new(window: WindowHandle<'_>, graphics: &SharedGraphics) -> Result<Self, String> {
        let software = unsafe { SoftwarePresenter::new(window, &graphics.software) }?;
        let gpu = graphics.gpu.as_ref().and_then(|gpu| {
            // SAFETY: same window lifetime contract as this constructor.
            match unsafe { GpuPresenter::new(window, gpu) } {
                Ok(presenter) => Some(presenter),
                Err(error) => {
                    tracing::warn!(%error, "GPU surface unavailable; using shared memory");
                    None
                }
            }
        });
        Ok(Self { gpu, software })
    }

    pub fn present_with_damage(
        &mut self,
        geometry: PresentationGeometry,
        graphics: &SharedGraphics,
        commands: &[PaintCommand],
        damage: Option<&[Rect]>,
    ) -> Result<DamageRegion, String> {
        if let (Some(gpu), Some(shared)) = (&mut self.gpu, &graphics.gpu) {
            match gpu.present(geometry, shared, commands, damage) {
                Ok(damage) => return Ok(damage),
                Err(error) => {
                    tracing::warn!(%error, "GPU shell presentation failed; switching surface to shared memory");
                    self.gpu = None;
                }
            }
        }
        self.software
            .present(geometry, &graphics.software, commands)
    }

    pub fn retained_framebuffer_diagnostics(&self) -> Option<RetainedFramebufferDiagnostics> {
        self.gpu
            .as_ref()
            .map(|presenter| presenter.retained_diagnostics)
    }
}

struct GpuPresenter {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    configured: bool,
    pipeline: wgpu::RenderPipeline,
    clear_pipeline: wgpu::RenderPipeline,
    retained: Option<RetainedFramebuffer>,
    retained_diagnostics: RetainedFramebufferDiagnostics,
    vertex_buffer: Option<wgpu::Buffer>,
    vertex_capacity: usize,
}

struct RetainedFramebuffer {
    view: wgpu::TextureView,
    texture: Arc<wgpu::BindGroup>,
    configuration: (u32, u32, u32, wgpu::TextureFormat),
    valid: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetainedFramebufferDiagnostics {
    pub live_bytes: usize,
    pub peak_bytes: usize,
    pub creations: u64,
    pub releases: u64,
    pub budget_fallbacks: u64,
    pub composited_frames: u64,
    pub direct_full_frames: u64,
    pub full_initializations: u64,
    pub partial_redraws: u64,
    pub full_redraws: u64,
    pub unchanged_reuses: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum OffscreenRedraw {
    Full,
    Partial(Rect),
    Reuse,
}

fn union_rect(left: Rect, right: Rect) -> Rect {
    let x = left.origin.x.min(right.origin.x);
    let y = left.origin.y.min(right.origin.y);
    let right_edge = (left.origin.x + left.size.width).max(right.origin.x + right.size.width);
    let bottom_edge = (left.origin.y + left.size.height).max(right.origin.y + right.size.height);
    Rect::new(x, y, right_edge - x, bottom_edge - y)
}

fn offscreen_redraw(
    commands: &[PaintCommand],
    damage: Option<&[Rect]>,
    valid: bool,
) -> OffscreenRedraw {
    if !valid
        || commands
            .iter()
            .any(|command| matches!(command, PaintCommand::BackdropBlur { .. }))
    {
        return OffscreenRedraw::Full;
    }
    let Some(damage) = damage else {
        return OffscreenRedraw::Full;
    };
    damage
        .iter()
        .copied()
        .reduce(union_rect)
        .map_or(OffscreenRedraw::Reuse, OffscreenRedraw::Partial)
}

fn physical_scissor(
    rect: Rect,
    width: u32,
    height: u32,
    scale: f32,
) -> Option<(u32, u32, u32, u32)> {
    let left = (rect.origin.x * scale).floor().clamp(0.0, width as f32) as u32;
    let top = (rect.origin.y * scale).floor().clamp(0.0, height as f32) as u32;
    let right = ((rect.origin.x + rect.size.width) * scale)
        .ceil()
        .clamp(0.0, width as f32) as u32;
    let bottom = ((rect.origin.y + rect.size.height) * scale)
        .ceil()
        .clamp(0.0, height as f32) as u32;
    (right > left && bottom > top).then_some((left, top, right - left, bottom - top))
}

fn intersect_scissor(
    left: (u32, u32, u32, u32),
    right: (u32, u32, u32, u32),
) -> Option<(u32, u32, u32, u32)> {
    let x = left.0.max(right.0);
    let y = left.1.max(right.1);
    let right_edge = left
        .0
        .saturating_add(left.2)
        .min(right.0.saturating_add(right.2));
    let bottom = left
        .1
        .saturating_add(left.3)
        .min(right.1.saturating_add(right.3));
    (right_edge > x && bottom > y).then_some((x, y, right_edge - x, bottom - y))
}

impl GpuPresenter {
    unsafe fn new(window: WindowHandle<'_>, graphics: &GpuGraphics) -> Result<Self, String> {
        // SAFETY: ShellSurface drops the presenter before the native window.
        let surface = unsafe {
            graphics
                .instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(graphics.display),
                    raw_window_handle: window.as_raw(),
                })
        }
        .map_err(|error| error.to_string())?;
        let mut config = surface
            .get_default_config(&graphics.adapter, 1, 1)
            .ok_or_else(|| "GPU adapter cannot present to this window".to_owned())?;
        let capabilities = surface.get_capabilities(&graphics.adapter);
        config.format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| *format == wgpu::TextureFormat::Bgra8Unorm)
            .unwrap_or(config.format);
        if capabilities
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            config.alpha_mode = wgpu::CompositeAlphaMode::PreMultiplied;
        }
        let layout = graphics
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Nickel shell pipeline"),
                bind_group_layouts: &[Some(&graphics.texture_layout)],
                immediate_size: 0,
            });
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];
        let pipeline = graphics
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Nickel shell textured rectangles"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &graphics.shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &graphics.shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        let clear_pipeline =
            graphics
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Nickel shell damage clear"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &graphics.shader,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<Vertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &attributes,
                        })],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &graphics.shader,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: config.format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
        Ok(Self {
            surface,
            config,
            configured: false,
            pipeline,
            clear_pipeline,
            retained: None,
            retained_diagnostics: RetainedFramebufferDiagnostics::default(),
            vertex_buffer: None,
            vertex_capacity: 0,
        })
    }

    fn ensure_retained_framebuffer(
        &mut self,
        graphics: &GpuGraphics,
        width: u32,
        height: u32,
        scale: f32,
    ) -> bool {
        let Some(bytes) = retained_framebuffer_bytes(width, height) else {
            self.release_retained_framebuffer();
            self.retained_diagnostics.budget_fallbacks =
                self.retained_diagnostics.budget_fallbacks.saturating_add(1);
            return false;
        };
        let configuration =
            retained_framebuffer_configuration(width, height, scale, self.config.format);
        if self
            .retained
            .as_ref()
            .is_some_and(|target| target.configuration == configuration)
        {
            return true;
        }
        self.release_retained_framebuffer();
        let texture = graphics.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Nickel retained shell framebuffer"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampled = Arc::new(
            graphics
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Nickel retained shell framebuffer"),
                    layout: &graphics.texture_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&graphics.sampler),
                        },
                    ],
                }),
        );
        self.retained = Some(RetainedFramebuffer {
            view,
            texture: sampled,
            configuration,
            valid: false,
        });
        self.retained_diagnostics.live_bytes = bytes;
        self.retained_diagnostics.peak_bytes = self.retained_diagnostics.peak_bytes.max(bytes);
        self.retained_diagnostics.creations = self.retained_diagnostics.creations.saturating_add(1);
        true
    }

    fn release_retained_framebuffer(&mut self) {
        if self.retained.take().is_some() {
            self.retained_diagnostics.releases =
                self.retained_diagnostics.releases.saturating_add(1);
        }
        self.retained_diagnostics.live_bytes = 0;
    }

    fn present(
        &mut self,
        geometry: PresentationGeometry,
        graphics: &GpuGraphics,
        commands: &[PaintCommand],
        damage: Option<&[Rect]>,
    ) -> Result<DamageRegion, String> {
        if geometry.pixel_width == 0 || geometry.pixel_height == 0 {
            self.release_retained_framebuffer();
            self.configured = false;
            return Ok(DamageRegion::default());
        }
        let width = geometry.pixel_width.max(1);
        let height = geometry.pixel_height.max(1);
        let changed = self.config.width != width || self.config.height != height;
        if changed || !self.configured {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&graphics.device, &self.config);
            self.configured = true;
        }
        let scale = width as f32 / geometry.logical_width.max(1) as f32;
        let retained = self.ensure_retained_framebuffer(graphics, width, height, scale);
        let retained_valid = self.retained.as_ref().is_some_and(|target| target.valid);
        let mut redraw = if retained {
            offscreen_redraw(commands, damage, retained_valid)
        } else {
            OffscreenRedraw::Full
        };
        let mut frame = Frame::new(width, height, scale, graphics.white.clone());
        frame.prepare(commands, graphics);
        let clear_start = frame.vertices.len() as u32;
        let mut damage_scissor = match redraw {
            OffscreenRedraw::Partial(damage) => physical_scissor(damage, width, height, scale),
            OffscreenRedraw::Full | OffscreenRedraw::Reuse => None,
        };
        if damage_scissor == Some((0, 0, width, height)) {
            redraw = OffscreenRedraw::Full;
            damage_scissor = None;
        } else if matches!(redraw, OffscreenRedraw::Partial(_)) && damage_scissor.is_none() {
            redraw = OffscreenRedraw::Reuse;
        }
        if damage_scissor.is_some() {
            frame
                .vertices
                .extend_from_slice(&fullscreen_vertices([0.0; 4]));
        }
        let composite_start = frame.vertices.len() as u32;
        if retained {
            frame
                .vertices
                .extend_from_slice(&fullscreen_vertices([1.0; 4]));
        }
        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(output) => output,
            wgpu::CurrentSurfaceTexture::Suboptimal(output) => {
                self.configured = false;
                output
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&graphics.device, &self.config);
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(output)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(output) => output,
                    other => {
                        return Err(format!(
                            "cannot acquire GPU surface after reconfigure: {other:?}"
                        ));
                    }
                }
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(DamageRegion::default());
            }
            other => return Err(format!("cannot acquire GPU surface: {other:?}")),
        };
        let vertex_bytes = bytemuck::cast_slice(&frame.vertices);
        let required = vertex_bytes.len().max(std::mem::size_of::<Vertex>());
        if required > self.vertex_capacity {
            self.vertex_capacity = required.next_power_of_two();
            self.vertex_buffer = Some(graphics.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Nickel shell frame vertices"),
                size: self.vertex_capacity as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let vertex_buffer = self.vertex_buffer.as_ref().expect("frame buffer allocated");
        if !vertex_bytes.is_empty() {
            graphics.queue.write_buffer(vertex_buffer, 0, vertex_bytes);
        }
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = graphics
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Nickel shell frame"),
            });
        {
            let target = self
                .retained
                .as_ref()
                .filter(|_| retained)
                .map_or(&view, |retained| &retained.view);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Nickel retained shell frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if matches!(redraw, OffscreenRedraw::Full) {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            if let Some(scissor) = damage_scissor {
                pass.set_pipeline(&self.clear_pipeline);
                pass.set_scissor_rect(scissor.0, scissor.1, scissor.2, scissor.3);
                pass.set_bind_group(0, graphics.white.as_ref(), &[]);
                pass.draw(clear_start..clear_start + 6, 0..1);
            }
            pass.set_pipeline(&self.pipeline);
            for draw in &frame.draws {
                let scissor = match redraw {
                    OffscreenRedraw::Full => Some(draw.scissor),
                    OffscreenRedraw::Partial(_) => {
                        damage_scissor.and_then(|damage| intersect_scissor(draw.scissor, damage))
                    }
                    OffscreenRedraw::Reuse => None,
                };
                let Some(scissor) = scissor else { continue };
                pass.set_scissor_rect(scissor.0, scissor.1, scissor.2, scissor.3);
                pass.set_bind_group(0, draw.texture.as_ref(), &[]);
                pass.draw(draw.start..draw.end, 0..1);
            }
        }
        if let Some(retained) = self.retained.as_ref().filter(|_| retained) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Nickel shell surface composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            pass.set_scissor_rect(0, 0, width, height);
            pass.set_bind_group(0, retained.texture.as_ref(), &[]);
            pass.draw(composite_start..composite_start + 6, 0..1);
            self.retained_diagnostics.composited_frames = self
                .retained_diagnostics
                .composited_frames
                .saturating_add(1);
        } else {
            self.retained_diagnostics.direct_full_frames = self
                .retained_diagnostics
                .direct_full_frames
                .saturating_add(1);
        }
        graphics.queue.submit(Some(encoder.finish()));
        match redraw {
            OffscreenRedraw::Full => {
                self.retained_diagnostics.full_redraws =
                    self.retained_diagnostics.full_redraws.saturating_add(1);
            }
            OffscreenRedraw::Partial(_) => {
                self.retained_diagnostics.partial_redraws =
                    self.retained_diagnostics.partial_redraws.saturating_add(1);
            }
            OffscreenRedraw::Reuse => {
                self.retained_diagnostics.unchanged_reuses =
                    self.retained_diagnostics.unchanged_reuses.saturating_add(1);
            }
        }
        if let Some(retained) = self.retained.as_mut().filter(|_| retained)
            && !retained.valid
        {
            retained.valid = true;
            self.retained_diagnostics.full_initializations = self
                .retained_diagnostics
                .full_initializations
                .saturating_add(1);
        }
        graphics.queue.present(output);
        Ok(DamageRegion {
            rects: vec![Rect::new(0.0, 0.0, width as f32, height as f32)].into(),
        })
    }
}

struct Draw {
    start: u32,
    end: u32,
    scissor: (u32, u32, u32, u32),
    texture: Arc<wgpu::BindGroup>,
}

struct Frame {
    width: u32,
    height: u32,
    scale: f32,
    white: Arc<wgpu::BindGroup>,
    vertices: Vec<Vertex>,
    draws: Vec<Draw>,
    clips: Vec<Rect>,
}

fn fullscreen_vertices(color: [f32; 4]) -> [Vertex; 6] {
    let top_left = Vertex {
        position: [-1.0, 1.0],
        uv: [0.0, 0.0],
        color,
    };
    let top_right = Vertex {
        position: [1.0, 1.0],
        uv: [1.0, 0.0],
        color,
    };
    let bottom_left = Vertex {
        position: [-1.0, -1.0],
        uv: [0.0, 1.0],
        color,
    };
    let bottom_right = Vertex {
        position: [1.0, -1.0],
        uv: [1.0, 1.0],
        color,
    };
    [
        top_left,
        bottom_left,
        top_right,
        top_right,
        bottom_left,
        bottom_right,
    ]
}

impl Frame {
    fn new(width: u32, height: u32, scale: f32, white: Arc<wgpu::BindGroup>) -> Self {
        Self {
            width,
            height,
            scale,
            white,
            vertices: Vec::new(),
            draws: Vec::new(),
            clips: vec![Rect::new(
                0.0,
                0.0,
                width as f32 / scale,
                height as f32 / scale,
            )],
        }
    }

    fn rect(&mut self, rect: Rect, color: [f32; 4], texture: Arc<wgpu::BindGroup>) {
        let clip = *self.clips.last().expect("frame clip");
        if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
            return;
        }
        if intersect(rect, clip).is_none() {
            return;
        }
        let x0 = (rect.origin.x * self.scale / self.width as f32) * 2.0 - 1.0;
        let y0 = 1.0 - (rect.origin.y * self.scale / self.height as f32) * 2.0;
        let x1 = ((rect.origin.x + rect.size.width) * self.scale / self.width as f32) * 2.0 - 1.0;
        let y1 = 1.0 - ((rect.origin.y + rect.size.height) * self.scale / self.height as f32) * 2.0;
        let corners = [
            Vertex {
                position: [x0, y0],
                uv: [0.0, 0.0],
                color,
            },
            Vertex {
                position: [x1, y0],
                uv: [1.0, 0.0],
                color,
            },
            Vertex {
                position: [x0, y1],
                uv: [0.0, 1.0],
                color,
            },
            Vertex {
                position: [x1, y1],
                uv: [1.0, 1.0],
                color,
            },
        ];
        let start = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&[
            corners[0], corners[2], corners[1], corners[1], corners[2], corners[3],
        ]);
        let sx = (clip.origin.x * self.scale)
            .floor()
            .clamp(0.0, self.width as f32) as u32;
        let sy = (clip.origin.y * self.scale)
            .floor()
            .clamp(0.0, self.height as f32) as u32;
        let ex = ((clip.origin.x + clip.size.width) * self.scale)
            .ceil()
            .clamp(0.0, self.width as f32) as u32;
        let ey = ((clip.origin.y + clip.size.height) * self.scale)
            .ceil()
            .clamp(0.0, self.height as f32) as u32;
        if ex > sx && ey > sy {
            let scissor = (sx, sy, ex - sx, ey - sy);
            if let Some(last) = self.draws.last_mut()
                && last.end == start
                && last.scissor == scissor
                && Arc::ptr_eq(&last.texture, &texture)
            {
                last.end += 6;
                return;
            }
            self.draws.push(Draw {
                start,
                end: start + 6,
                scissor,
                texture,
            });
        }
    }

    fn solid(&mut self, rect: Rect, color: u32) {
        self.rect(rect, rgb(color), self.white.clone());
    }

    fn rounded(&mut self, rect: Rect, color: u32, radius: f32, top_only: bool) {
        for (span, shaded) in nickel_ui::backend::rounded_coverage_spans(
            rect, color, radius, None, top_only, self.scale,
        ) {
            self.solid(span, shaded);
        }
    }

    fn prepare(&mut self, commands: &[PaintCommand], graphics: &GpuGraphics) {
        for command in commands {
            match command {
                PaintCommand::BackdropBlur { .. } => {}
                PaintCommand::Fill { rect, color } | PaintCommand::OverlayFill { rect, color } => {
                    self.solid(*rect, *color)
                }
                PaintCommand::RoundedFill {
                    rect,
                    color,
                    radius,
                } => self.rounded(*rect, *color, *radius, false),
                PaintCommand::TopRoundedFill {
                    rect,
                    color,
                    radius,
                } => self.rounded(*rect, *color, *radius, true),
                PaintCommand::RoundedStroke {
                    rect,
                    color,
                    width,
                    radius,
                } => {
                    for (span, shaded) in nickel_ui::backend::rounded_coverage_spans(
                        *rect,
                        *color,
                        *radius,
                        Some(*width),
                        false,
                        self.scale,
                    ) {
                        self.solid(span, shaded);
                    }
                }
                PaintCommand::Gradient { rect, gradient } => {
                    let horizontal = gradient.axis == GradientAxis::Horizontal;
                    let extent = if horizontal {
                        rect.size.width
                    } else {
                        rect.size.height
                    };
                    for step in 0..extent.ceil().max(1.0) as u32 {
                        let at = step as f32;
                        let color = mix(
                            gradient.start,
                            gradient.end,
                            ((at + 0.5) / extent.max(1.0)).clamp(0.0, 1.0),
                        );
                        let strip = if horizontal {
                            Rect::new(
                                rect.origin.x + at,
                                rect.origin.y,
                                (extent - at).min(1.0),
                                rect.size.height,
                            )
                        } else {
                            Rect::new(
                                rect.origin.x,
                                rect.origin.y + at,
                                rect.size.width,
                                (extent - at).min(1.0),
                            )
                        };
                        self.solid(strip, color);
                    }
                }
                PaintCommand::Stroke { rect, color, width }
                | PaintCommand::OverlayStroke { rect, color, width } => {
                    let w = width.max(0.0).min(rect.size.width).min(rect.size.height);
                    self.solid(
                        Rect::new(rect.origin.x, rect.origin.y, rect.size.width, w),
                        *color,
                    );
                    self.solid(
                        Rect::new(
                            rect.origin.x,
                            rect.origin.y + rect.size.height - w,
                            rect.size.width,
                            w,
                        ),
                        *color,
                    );
                    self.solid(
                        Rect::new(
                            rect.origin.x,
                            rect.origin.y + w,
                            w,
                            (rect.size.height - 2.0 * w).max(0.0),
                        ),
                        *color,
                    );
                    self.solid(
                        Rect::new(
                            rect.origin.x + rect.size.width - w,
                            rect.origin.y + w,
                            w,
                            (rect.size.height - 2.0 * w).max(0.0),
                        ),
                        *color,
                    );
                }
                PaintCommand::Text { bounds, .. } | PaintCommand::StyledText { bounds, .. } => {
                    if bounds.size.width > 0.0
                        && bounds.size.height > 0.0
                        && intersect(*bounds, *self.clips.last().unwrap()).is_some()
                    {
                        let texture = graphics.text_texture(command, *bounds, self.scale);
                        self.rect(*bounds, [1.0; 4], texture);
                    }
                }
                PaintCommand::Image {
                    bounds,
                    id,
                    generation,
                    image,
                    high_density,
                } => {
                    if bounds.size.width <= 0.0
                        || bounds.size.height <= 0.0
                        || intersect(*bounds, *self.clips.last().expect("frame clip")).is_none()
                    {
                        continue;
                    }
                    let selected = high_density
                        .as_ref()
                        .filter(|_| self.scale >= 1.5)
                        .unwrap_or(image);
                    if selected.width() == 0 || selected.height() == 0 {
                        continue;
                    }
                    let key = format!(
                        "image:{id}:{generation}:{}:{}:{}:{:p}",
                        self.scale >= 1.5,
                        selected.width(),
                        selected.height(),
                        Arc::as_ptr(selected),
                    );
                    let cached = graphics.textures.borrow().entries.get(&key).cloned();
                    let texture = if let Some(cached) = cached {
                        cached
                    } else {
                        let mut bytes = selected.as_raw().clone();
                        for pixel in bytes.as_chunks_mut::<4>().0.iter_mut() {
                            let alpha = pixel[3] as u16;
                            for channel in &mut pixel[..3] {
                                *channel = ((*channel as u16 * alpha + 127) / 255) as u8;
                            }
                        }
                        graphics.texture(key, selected.width(), selected.height(), &bytes)
                    };
                    self.rect(*bounds, [1.0; 4], texture);
                }
                PaintCommand::PushClip(rect) => {
                    let clip = *self.clips.last().unwrap();
                    self.clips
                        .push(intersect(clip, *rect).unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0)));
                }
                PaintCommand::PopClip => {
                    if self.clips.len() > 1 {
                        self.clips.pop();
                    }
                }
            }
        }
    }
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.origin.x.max(b.origin.x);
    let y = a.origin.y.max(b.origin.y);
    let right = (a.origin.x + a.size.width).min(b.origin.x + b.size.width);
    let bottom = (a.origin.y + a.size.height).min(b.origin.y + b.size.height);
    (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
}

fn rgb(color: u32) -> [f32; 4] {
    let alpha = if color <= 0x00ff_ffff {
        1.0
    } else {
        ((color >> 24) & 255) as f32 / 255.0
    };
    [
        ((color >> 16) & 255) as f32 / 255.0 * alpha,
        ((color >> 8) & 255) as f32 / 255.0 * alpha,
        (color & 255) as f32 / 255.0 * alpha,
        alpha,
    ]
}

fn mix(start: u32, end: u32, at: f32) -> u32 {
    let channel = |shift: u32| {
        let a = if shift == 24 && start <= 0x00ff_ffff {
            255
        } else {
            (start >> shift) & 255
        };
        let b = if shift == 24 && end <= 0x00ff_ffff {
            255
        } else {
            (end >> shift) & 255
        };
        (a as f32 + (b as f32 - a as f32) * at).round() as u32
    };
    channel(24) << 24 | channel(16) << 16 | channel(8) << 8 | channel(0)
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_RETAINED_FRAMEBUFFER_BYTES, OffscreenRedraw, fullscreen_vertices, offscreen_redraw,
        retained_framebuffer_bytes, retained_framebuffer_configuration,
    };
    use nickel_ui::{Rect, backend::PaintCommand};

    #[cfg(target_os = "windows")]
    use {
        super::{GpuGraphics, GpuPresenter},
        crate::softbuffer_presenter::PresentationGeometry,
        nickel_ui::{
            SoftwareRenderer,
            backend::{FrameRenderer, RenderFrame},
        },
        raw_window_handle::{HasDisplayHandle, HasWindowHandle},
        serde_json::json,
        std::time::{Duration, Instant},
        winit::{
            event_loop::EventLoop, platform::windows::EventLoopBuilderExtWindows, window::Window,
        },
    };

    #[cfg(target_os = "windows")]
    fn admission_distribution(samples: &[Duration]) -> serde_json::Value {
        assert!(!samples.is_empty());
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let at = |percentile: usize| {
            let rank = sorted.len().saturating_mul(percentile).div_ceil(100);
            sorted[rank.saturating_sub(1).min(sorted.len() - 1)].as_nanos() as u64
        };
        json!({
            "p50_ns": at(50),
            "p95_ns": at(95),
            "p99_ns": at(99),
            "max_ns": sorted.last().expect("samples are non-empty").as_nanos() as u64,
        })
    }

    #[cfg(target_os = "windows")]
    fn admission_commands(selected: usize) -> Vec<PaintCommand> {
        const COLUMNS: usize = 8;
        const ROWS: usize = 6;
        const CELL: f32 = 16.0;
        let mut commands = Vec::with_capacity(COLUMNS * ROWS + 1);
        commands.push(PaintCommand::Fill {
            rect: Rect::new(0.0, 0.0, COLUMNS as f32 * CELL, ROWS as f32 * CELL),
            color: 0xff10_1820,
        });
        for index in 0..COLUMNS * ROWS {
            let column = index % COLUMNS;
            let row = index / COLUMNS;
            commands.push(PaintCommand::Fill {
                rect: Rect::new(
                    column as f32 * CELL + 2.0,
                    row as f32 * CELL + 2.0,
                    CELL - 4.0,
                    CELL - 4.0,
                ),
                color: if index == selected {
                    0xffe0_8240
                } else {
                    0xff30_4858
                },
            });
        }
        commands
    }

    #[cfg(target_os = "windows")]
    fn admission_damage(previous: usize, selected: usize) -> [Rect; 2] {
        let cell = |index: usize| {
            Rect::new(
                (index % 8) as f32 * 16.0 + 2.0,
                (index / 8) as f32 * 16.0 + 2.0,
                12.0,
                12.0,
            )
        };
        [cell(previous), cell(selected)]
    }

    #[cfg(target_os = "windows")]
    fn emit_wgpu_admission_skip(
        reason: impl std::fmt::Display,
        software_work: &serde_json::Value,
        software_changed: &[Duration],
        software_unchanged: &[Duration],
    ) {
        eprintln!(
            "nickel_release_admission={}",
            json!({
                "schema": 1,
                "suite": "production_presenters",
                "workload": "software_vs_wgpu_retained",
                "status": "skipped",
                "skip_reason": reason.to_string(),
                "metadata": {
                    "software_renderer": "nickel_ui::SoftwareRenderer",
                    "wgpu_renderer": "production GpuPresenter",
                },
                "work": software_work,
                "timings": {
                    "software_changed": admission_distribution(software_changed),
                    "software_unchanged": admission_distribution(software_unchanged),
                },
            })
        );
    }

    #[test]
    #[ignore = "release admission requires a visible Windows WGPU surface"]
    #[cfg(target_os = "windows")]
    fn production_software_and_wgpu_release_admission() {
        const WIDTH: u32 = 128;
        const HEIGHT: u32 = 96;
        const SAMPLES: usize = 31;

        // Always exercise the real software renderer, even if the native WGPU
        // adapter or presentation surface is unavailable and this admission is
        // reported as skipped below.
        let mut software = SoftwareRenderer::new_pixel_buffer(WIDTH, HEIGHT, 1.0);
        let initial = admission_commands(0);
        software
            .render_frame(RenderFrame {
                commands: &initial,
                logical_size: (WIDTH, HEIGHT),
                scale_factor: 1.0,
                generation: 0,
            })
            .expect("software rendering is infallible");
        let software_baseline = software.software_raster_diagnostics();
        let mut software_changed = Vec::with_capacity(SAMPLES);
        let mut software_unchanged = Vec::with_capacity(SAMPLES);
        let mut paired_frames = Vec::with_capacity(SAMPLES);
        let mut previous = 0;
        for sample in 0..SAMPLES {
            let selected = (sample + 1) % 48;
            let commands = admission_commands(selected);
            let damage = admission_damage(previous, selected);
            let frame = RenderFrame {
                commands: &commands,
                logical_size: (WIDTH, HEIGHT),
                scale_factor: 1.0,
                generation: sample as u64 + 1,
            };
            let start = Instant::now();
            let changed_damage = software
                .render_frame_with_damage(frame, Some(&damage))
                .expect("software rendering is infallible");
            software_changed.push(start.elapsed());
            assert_eq!(changed_damage.rects.as_ref(), damage.as_slice());

            let start = Instant::now();
            let unchanged_damage = software
                .render_frame_with_damage(frame, Some(&[]))
                .expect("software rendering is infallible");
            software_unchanged.push(start.elapsed());
            assert!(unchanged_damage.is_empty());

            let mut cold = SoftwareRenderer::new_pixel_buffer(WIDTH, HEIGHT, 1.0);
            cold.render(&commands);
            assert_eq!(software.pixels(), cold.pixels());
            previous = selected;
            paired_frames.push(commands);
        }
        let software_work = software.software_raster_diagnostics();
        assert_eq!(
            software_work.partial_repaints - software_baseline.partial_repaints,
            SAMPLES as u64
        );
        assert_eq!(
            software_work.clean_frames - software_baseline.clean_frames,
            SAMPLES as u64
        );
        assert_eq!(
            software_work.damage_hints_accepted - software_baseline.damage_hints_accepted,
            SAMPLES as u64
        );
        assert_eq!(
            software_work.framebuffer_live_bytes,
            WIDTH as usize * HEIGHT as usize * 4
        );
        let software_report = json!({
            "commands_per_frame": paired_frames[0].len(),
            "damage_rects_per_changed_frame": 2,
            "software_partial_repaints": software_work.partial_repaints - software_baseline.partial_repaints,
            "software_clean_frames": software_work.clean_frames - software_baseline.clean_frames,
            "software_damage_hints_accepted": software_work.damage_hints_accepted - software_baseline.damage_hints_accepted,
            "software_framebuffer_bytes": software_work.framebuffer_live_bytes,
            "software_cold_raster_equivalence_samples": SAMPLES,
        });

        let mut builder = EventLoop::builder();
        builder.with_any_thread(true);
        let events = match builder.build() {
            Ok(events) => events,
            Err(error) => {
                emit_wgpu_admission_skip(
                    format!("cannot create Windows event loop: {error}"),
                    &software_report,
                    &software_changed,
                    &software_unchanged,
                );
                return;
            }
        };
        #[allow(deprecated)]
        let window = match events.create_window(
            Window::default_attributes()
                .with_title("Nickel WGPU release admission")
                .with_inner_size(winit::dpi::PhysicalSize::new(WIDTH, HEIGHT))
                .with_resizable(false)
                .with_visible(true),
        ) {
            Ok(window) => window,
            Err(error) => {
                emit_wgpu_admission_skip(
                    format!("cannot create Windows surface: {error}"),
                    &software_report,
                    &software_changed,
                    &software_unchanged,
                );
                return;
            }
        };
        let display = window.display_handle().expect("window display handle");
        let handle = window.window_handle().expect("window handle");
        // SAFETY: the window remains alive until both GPU values are dropped.
        let graphics = match unsafe { GpuGraphics::new(display, handle) } {
            Ok(graphics) => graphics,
            Err(error) => {
                emit_wgpu_admission_skip(
                    format!("production WGPU adapter unavailable: {error}"),
                    &software_report,
                    &software_changed,
                    &software_unchanged,
                );
                return;
            }
        };
        let adapter = graphics.adapter.get_info();
        // SAFETY: the window remains alive until the presenter is dropped.
        let mut gpu = match unsafe { GpuPresenter::new(handle, &graphics) } {
            Ok(gpu) => gpu,
            Err(error) => {
                emit_wgpu_admission_skip(
                    format!("production WGPU surface unavailable: {error}"),
                    &software_report,
                    &software_changed,
                    &software_unchanged,
                );
                return;
            }
        };
        let geometry = PresentationGeometry {
            pixel_width: WIDTH,
            pixel_height: HEIGHT,
            logical_width: WIDTH,
            logical_height: HEIGHT,
        };
        gpu.present(geometry, &graphics, &admission_commands(0), None)
            .expect("initial WGPU presentation");
        let gpu_baseline = gpu.retained_diagnostics;
        let mut gpu_changed = Vec::with_capacity(SAMPLES);
        let mut gpu_unchanged = Vec::with_capacity(SAMPLES);
        previous = 0;
        for (sample, commands) in paired_frames.iter().enumerate() {
            let selected = (sample + 1) % 48;
            let damage = admission_damage(previous, selected);
            let start = Instant::now();
            gpu.present(geometry, &graphics, commands, Some(&damage))
                .expect("changed WGPU presentation");
            gpu_changed.push(start.elapsed());
            let start = Instant::now();
            gpu.present(geometry, &graphics, commands, Some(&[]))
                .expect("unchanged WGPU presentation");
            gpu_unchanged.push(start.elapsed());
            previous = selected;
        }
        let gpu_work = gpu.retained_diagnostics;
        assert_eq!(
            gpu_work.partial_redraws - gpu_baseline.partial_redraws,
            SAMPLES as u64
        );
        assert_eq!(
            gpu_work.unchanged_reuses - gpu_baseline.unchanged_reuses,
            SAMPLES as u64
        );
        assert_eq!(gpu_work.full_redraws - gpu_baseline.full_redraws, 0);
        assert_eq!(
            gpu_work.composited_frames - gpu_baseline.composited_frames,
            (SAMPLES * 2) as u64
        );
        assert_eq!(gpu_work.live_bytes, WIDTH as usize * HEIGHT as usize * 4);
        assert_eq!(gpu_work.creations, 1);
        assert_eq!(gpu_work.releases, 0);
        assert_eq!(gpu_work.budget_fallbacks, 0);
        assert_eq!(gpu.vertex_capacity, 16_384);
        assert_eq!(graphics.textures.borrow().entries.len(), 0);

        eprintln!(
            "nickel_release_admission={}",
            json!({
                "schema": 1,
                "suite": "production_presenters",
                "workload": "software_vs_wgpu_retained",
                "status": "measured",
                "metadata": {
                    "samples": SAMPLES,
                    "width": WIDTH,
                    "height": HEIGHT,
                    "adapter_name": adapter.name,
                    "adapter_backend": format!("{:?}", adapter.backend),
                    "adapter_device_type": format!("{:?}", adapter.device_type),
                    "adapter_vendor": adapter.vendor,
                    "adapter_device": adapter.device,
                    "driver": adapter.driver,
                    "driver_info": adapter.driver_info,
                    "wgpu_renderer": "production GpuPresenter with native surface",
                    "software_renderer": "nickel_ui::SoftwareRenderer",
                    "wgpu_surface_readback_available": false,
                    "software_cold_raster_equivalence_samples": SAMPLES,
                },
                "work": {
                    "commands_per_frame": paired_frames[0].len(),
                    "damage_rects_per_changed_frame": 2,
                    "software_partial_repaints": software_work.partial_repaints - software_baseline.partial_repaints,
                    "software_clean_frames": software_work.clean_frames - software_baseline.clean_frames,
                    "software_damage_hints_accepted": software_work.damage_hints_accepted - software_baseline.damage_hints_accepted,
                    "software_framebuffer_bytes": software_work.framebuffer_live_bytes,
                    "wgpu_partial_redraws": gpu_work.partial_redraws - gpu_baseline.partial_redraws,
                    "wgpu_unchanged_reuses": gpu_work.unchanged_reuses - gpu_baseline.unchanged_reuses,
                    "wgpu_full_redraws": gpu_work.full_redraws - gpu_baseline.full_redraws,
                    "wgpu_composited_frames": gpu_work.composited_frames - gpu_baseline.composited_frames,
                    "wgpu_retained_framebuffer_bytes": gpu_work.live_bytes,
                    "wgpu_vertex_buffer_bytes": gpu.vertex_capacity,
                    "wgpu_cached_textures": graphics.textures.borrow().entries.len(),
                },
                "timings": {
                    "software_changed": admission_distribution(&software_changed),
                    "software_unchanged": admission_distribution(&software_unchanged),
                    "wgpu_changed": admission_distribution(&gpu_changed),
                    "wgpu_unchanged": admission_distribution(&gpu_unchanged),
                }
            })
        );
    }

    #[test]
    fn retained_framebuffer_admission_is_bounded_and_overflow_safe() {
        assert_eq!(retained_framebuffer_bytes(1920, 1080), Some(8_294_400));
        assert_eq!(retained_framebuffer_bytes(0, 1080), Some(0));
        assert_eq!(retained_framebuffer_bytes(u32::MAX, u32::MAX), None);
        let square = ((MAX_RETAINED_FRAMEBUFFER_BYTES / 4) as f64).sqrt().floor() as u32;
        assert!(retained_framebuffer_bytes(square, square).is_some());
        assert!(retained_framebuffer_bytes(square + 1, square + 1).is_none());
    }

    #[test]
    fn retained_framebuffer_composite_covers_the_cold_frame_exactly() {
        let vertices = fullscreen_vertices([1.0; 4]);
        assert_eq!(vertices.len(), 6);
        let positions = vertices.map(|vertex| vertex.position);
        assert!(positions.contains(&[-1.0, 1.0]));
        assert!(positions.contains(&[1.0, 1.0]));
        assert!(positions.contains(&[-1.0, -1.0]));
        assert!(positions.contains(&[1.0, -1.0]));
        assert!(vertices.iter().all(|vertex| vertex.color == [1.0; 4]));
    }

    #[test]
    fn retained_framebuffer_identity_changes_on_resize_scale_and_format() {
        let original =
            retained_framebuffer_configuration(800, 600, 1.0, wgpu::TextureFormat::Bgra8Unorm);
        assert_ne!(
            original,
            retained_framebuffer_configuration(801, 600, 1.0, wgpu::TextureFormat::Bgra8Unorm)
        );
        assert_ne!(
            original,
            retained_framebuffer_configuration(800, 600, 2.0, wgpu::TextureFormat::Bgra8Unorm)
        );
        assert_ne!(
            original,
            retained_framebuffer_configuration(800, 600, 1.0, wgpu::TextureFormat::Rgba8Unorm)
        );
    }

    #[test]
    fn keyed_front_insertion_uses_identity_damage_without_invalidating_trailing_work() {
        let previous = (0..64)
            .map(|index| PaintCommand::Fill {
                rect: Rect::new(index as f32 * 10.0, 40.0, 8.0, 8.0),
                color: index,
            })
            .collect::<Vec<_>>();
        let inserted_bounds = Rect::new(4.0, 4.0, 20.0, 20.0);
        let mut commands = vec![PaintCommand::Fill {
            rect: inserted_bounds,
            color: 0xff00ff,
        }];
        commands.extend(previous.iter().cloned());
        assert_eq!(&commands[1..], previous);

        assert_eq!(
            offscreen_redraw(&commands, Some(&[inserted_bounds]), true),
            OffscreenRedraw::Partial(inserted_bounds)
        );
        assert_eq!(
            offscreen_redraw(&commands, Some(&[]), true),
            OffscreenRedraw::Reuse
        );
    }

    #[test]
    fn missing_damage_invalid_storage_and_backdrop_effects_force_full_redraw() {
        let fill = PaintCommand::Fill {
            rect: Rect::new(0.0, 0.0, 20.0, 20.0),
            color: 0xff0000,
        };
        assert_eq!(
            offscreen_redraw(&[fill.clone()], None, true),
            OffscreenRedraw::Full
        );
        assert_eq!(
            offscreen_redraw(&[fill.clone()], Some(&[]), false),
            OffscreenRedraw::Full
        );
        assert_eq!(
            offscreen_redraw(
                &[
                    PaintCommand::BackdropBlur {
                        rect: Rect::new(0.0, 0.0, 20.0, 20.0),
                        radius: 4.0,
                        blur: 8.0,
                    },
                    fill,
                ],
                Some(&[Rect::new(0.0, 0.0, 20.0, 20.0)]),
                true,
            ),
            OffscreenRedraw::Full
        );
    }
}
