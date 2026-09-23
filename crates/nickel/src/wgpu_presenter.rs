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

    pub fn present(
        &mut self,
        geometry: PresentationGeometry,
        graphics: &SharedGraphics,
        commands: &[PaintCommand],
    ) -> Result<DamageRegion, String> {
        if let (Some(gpu), Some(shared)) = (&mut self.gpu, &graphics.gpu) {
            match gpu.present(geometry, shared, commands) {
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
}

struct GpuPresenter {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    configured: bool,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: Option<wgpu::Buffer>,
    vertex_capacity: usize,
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
        Ok(Self {
            surface,
            config,
            configured: false,
            pipeline,
            vertex_buffer: None,
            vertex_capacity: 0,
        })
    }

    fn present(
        &mut self,
        geometry: PresentationGeometry,
        graphics: &GpuGraphics,
        commands: &[PaintCommand],
    ) -> Result<DamageRegion, String> {
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
        let mut frame = Frame::new(width, height, scale, graphics.white.clone());
        frame.prepare(commands, graphics);
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
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Nickel shell surface"),
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
            for draw in &frame.draws {
                pass.set_scissor_rect(
                    draw.scissor.0,
                    draw.scissor.1,
                    draw.scissor.2,
                    draw.scissor.3,
                );
                pass.set_bind_group(0, draw.texture.as_ref(), &[]);
                pass.draw(draw.start..draw.end, 0..1);
            }
        }
        graphics.queue.submit(Some(encoder.finish()));
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
            if let Some(last) = self.draws.last_mut() {
                if last.end == start
                    && last.scissor == scissor
                    && Arc::ptr_eq(&last.texture, &texture)
                {
                    last.end += 6;
                    return;
                }
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
        let radius = radius
            .max(0.0)
            .min(rect.size.width / 2.0)
            .min(rect.size.height / 2.0);
        if radius < 0.5 {
            self.solid(rect, color);
            return;
        }
        let rows = rect.size.height.ceil().max(1.0) as u32;
        let mut middle = None;
        let mut middle_end = 0.0;
        for row in 0..rows {
            let y = row as f32;
            let h = (rect.size.height - y).clamp(0.0, 1.0);
            if h <= 0.0 {
                continue;
            }
            let sample_y = y + h / 2.0;
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
                middle.get_or_insert(y);
                middle_end = y + h;
                continue;
            }
            self.solid(
                Rect::new(
                    rect.origin.x + inset,
                    rect.origin.y + y,
                    (rect.size.width - inset * 2.0).max(0.0),
                    h,
                ),
                color,
            );
        }
        if let Some(y) = middle {
            self.solid(
                Rect::new(
                    rect.origin.x,
                    rect.origin.y + y,
                    rect.size.width,
                    middle_end - y,
                ),
                color,
            );
        }
    }

    fn prepare(&mut self, commands: &[PaintCommand], graphics: &GpuGraphics) {
        for command in commands {
            match command {
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
                        for pixel in bytes.chunks_exact_mut(4) {
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
