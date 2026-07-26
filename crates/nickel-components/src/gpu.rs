use std::{collections::HashMap, mem, sync::Arc};

use bytemuck::{Pod, Zeroable};
use glyphon::{
    Attrs, Buffer, Cache, Color as TextColor, ContentType, CustomGlyph, Family, FontSystem,
    Metrics, RasterizeCustomGlyphRequest, RasterizedCustomGlyph, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, cosmic_text::Align,
};
use winit::window::Window;

use crate::{GradientAxis, PaintCommand, Rect, TextAlign};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 4],
}

pub struct ComponentGpu {
    _instance: Option<wgpu::Instance>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    rectangle_pipeline: wgpu::RenderPipeline,
    rectangle_vertices: wgpu::Buffer,
    rectangle_capacity: usize,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    text_buffers: Vec<Buffer>,
    image_buffer: Buffer,
}

impl ComponentGpu {
    pub fn new(window: Arc<Window>, width: u32, height: u32) -> Result<Self, String> {
        pollster::block_on(Self::new_async(window, width, height))
    }

    async fn new_async(window: Arc<Window>, width: u32, height: u32) -> Result<Self, String> {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .map_err(|error| format!("failed to create component surface: {error}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
            .map_err(|error| format!("failed to find a graphics adapter: {error}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Nickel component device"),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                ..Default::default()
            })
            .await
            .map_err(|error| format!("failed to create component device: {error}"))?;
        Self::from_graphics(
            Some(instance),
            surface,
            &adapter,
            &device,
            &queue,
            width,
            height,
        )
    }

    pub fn with_shared_graphics(
        surface: wgpu::Surface<'static>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        Self::from_graphics(None, surface, adapter, device, queue, width, height)
    }

    fn from_graphics(
        instance: Option<wgpu::Instance>,
        surface: wgpu::Surface<'static>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let mut config = surface
            .get_default_config(&adapter, width.max(1), height.max(1))
            .ok_or_else(|| "component surface has no supported configuration".to_owned())?;
        config.desired_maximum_frame_latency = 1;
        surface.configure(device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Nickel component rectangle shader"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};
@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(input.position, 0.0, 1.0);
    output.color = input.color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
"#
                .into(),
            ),
        });
        let rectangle_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Nickel component rectangle pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let rectangle_capacity = 256;
        let rectangle_vertices = rectangle_buffer(&device, rectangle_capacity);

        let mut font_system = FontSystem::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, config.format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let mut image_buffer = Buffer::new(&mut font_system, Metrics::new(1.0, 1.0));
        image_buffer.set_size(Some(width.max(1) as f32), Some(height.max(1) as f32));
        Ok(Self {
            _instance: instance,
            surface,
            device: device.clone(),
            queue: queue.clone(),
            config,
            rectangle_pipeline,
            rectangle_vertices,
            rectangle_capacity,
            font_system,
            swash_cache: SwashCache::new(),
            viewport,
            atlas,
            text_renderer,
            text_buffers: Vec::new(),
            image_buffer,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn render(&mut self, commands: &[PaintCommand]) -> Result<(), String> {
        let mut vertices = Vec::new();
        let mut overlay_vertices = Vec::new();
        self.text_buffers.clear();
        let mut text_bounds = Vec::new();
        let mut text_colors = Vec::new();
        let mut image_commands = Vec::new();
        for command in commands {
            match command {
                PaintCommand::Fill { rect, color } => {
                    add_rectangle(
                        &mut vertices,
                        (self.config.width, self.config.height),
                        *rect,
                        color_rgba(*color),
                    );
                }
                PaintCommand::Gradient { rect, gradient } => {
                    add_gradient(
                        &mut vertices,
                        (self.config.width, self.config.height),
                        *rect,
                        color_rgba(gradient.start),
                        color_rgba(gradient.end),
                        gradient.axis,
                    );
                }
                PaintCommand::Stroke { rect, color, width } => {
                    add_stroke(
                        &mut vertices,
                        (self.config.width, self.config.height),
                        *rect,
                        *width,
                        color_rgba(*color),
                    );
                }
                PaintCommand::OverlayFill { rect, color } => {
                    add_rectangle(
                        &mut overlay_vertices,
                        (self.config.width, self.config.height),
                        *rect,
                        color_rgba(*color),
                    );
                }
                PaintCommand::OverlayStroke { rect, color, width } => {
                    add_stroke(
                        &mut overlay_vertices,
                        (self.config.width, self.config.height),
                        *rect,
                        *width,
                        color_rgba(*color),
                    );
                }
                PaintCommand::Text {
                    bounds,
                    text,
                    scale,
                    color,
                    align,
                } => {
                    let font_size = text_size(*scale);
                    let mut buffer = Buffer::new(
                        &mut self.font_system,
                        Metrics::new(font_size, font_size * 1.3),
                    );
                    buffer.set_size(
                        Some(bounds.size.width.max(1.0)),
                        Some(bounds.size.height.max(font_size * 1.4)),
                    );
                    buffer.set_text(
                        text,
                        &Attrs::new().family(Family::SansSerif),
                        Shaping::Advanced,
                        None,
                    );
                    for line in &mut buffer.lines {
                        line.set_align(Some(match align {
                            TextAlign::Start => Align::Left,
                            TextAlign::Center => Align::Center,
                            TextAlign::End => Align::Right,
                        }));
                    }
                    buffer.shape_until_scroll(&mut self.font_system, false);
                    self.text_buffers.push(buffer);
                    text_bounds.push(*bounds);
                    text_colors.push(text_color(*color));
                }
                PaintCommand::Image { bounds, id, image } => {
                    image_commands.push((*bounds, *id, image.clone()));
                }
            }
        }
        let overlay_start = vertices.len();
        vertices.extend_from_slice(&overlay_vertices);
        self.ensure_rectangle_capacity(vertices.len());
        if !vertices.is_empty() {
            self.queue
                .write_buffer(&self.rectangle_vertices, 0, bytemuck::cast_slice(&vertices));
        }
        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        let text_areas = self
            .text_buffers
            .iter()
            .zip(text_bounds)
            .zip(text_colors)
            .map(|((buffer, bounds), color)| TextArea {
                buffer,
                left: bounds.origin.x,
                top: bounds.origin.y,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: (bounds.origin.x + bounds.size.width).ceil() as i32,
                    bottom: (bounds.origin.y + bounds.size.height).ceil() as i32,
                },
                default_color: color,
                custom_glyphs: &[],
            })
            .collect::<Vec<_>>();
        let image_glyphs = image_commands
            .iter()
            .map(|(bounds, id, _)| {
                vec![CustomGlyph {
                    id: *id,
                    left: 0.0,
                    top: 0.0,
                    width: bounds.size.width.round().max(1.0),
                    height: bounds.size.height.round().max(1.0),
                    color: None,
                    snap_to_physical_pixel: true,
                    metadata: 0,
                }]
            })
            .collect::<Vec<_>>();
        let mut areas = Vec::new();
        for ((bounds, _, _), glyphs) in image_commands.iter().zip(&image_glyphs) {
            areas.push(TextArea {
                buffer: &self.image_buffer,
                left: bounds.origin.x,
                top: bounds.origin.y,
                scale: 1.0,
                bounds: TextBounds {
                    left: bounds.origin.x.floor() as i32,
                    top: bounds.origin.y.floor() as i32,
                    right: (bounds.origin.x + bounds.size.width).ceil() as i32,
                    bottom: (bounds.origin.y + bounds.size.height).ceil() as i32,
                },
                default_color: TextColor::rgba(255, 255, 255, 255),
                custom_glyphs: glyphs,
            });
        }
        areas.extend(text_areas);
        let images = image_commands
            .iter()
            .map(|(_, id, image)| (*id, image))
            .collect::<HashMap<_, _>>();
        self.text_renderer
            .prepare_with_custom(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash_cache,
                &|request: RasterizeCustomGlyphRequest| {
                    let source = images.get(&request.id)?;
                    let image = resize_rgba(source, request.width.into(), request.height.into());
                    Some(RasterizedCustomGlyph {
                        data: image.into_raw(),
                        content_type: ContentType::Color,
                    })
                },
            )
            .map_err(|error| format!("failed to prepare component text: {error}"))?;

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("component surface validation failed".to_owned());
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                ..Default::default()
            });
            if !vertices.is_empty() {
                pass.set_pipeline(&self.rectangle_pipeline);
                pass.set_vertex_buffer(0, self.rectangle_vertices.slice(..));
                pass.draw(0..vertices.len() as u32, 0..1);
            }
            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .map_err(|error| format!("failed to render component text: {error}"))?;
            if overlay_start < vertices.len() {
                pass.set_pipeline(&self.rectangle_pipeline);
                pass.set_vertex_buffer(0, self.rectangle_vertices.slice(..));
                pass.draw(overlay_start as u32..vertices.len() as u32, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        self.atlas.trim();
        Ok(())
    }

    fn ensure_rectangle_capacity(&mut self, required: usize) {
        if required <= self.rectangle_capacity {
            return;
        }
        self.rectangle_capacity = required.next_power_of_two();
        self.rectangle_vertices = rectangle_buffer(&self.device, self.rectangle_capacity);
    }
}

fn resize_rgba(source: &image::RgbaImage, width: u32, height: u32) -> image::RgbaImage {
    let width = width.max(1);
    let height = height.max(1);
    let scale = (width as f32 / source.width().max(1) as f32)
        .min(height as f32 / source.height().max(1) as f32);
    let fitted_width = (source.width() as f32 * scale).round().max(1.0) as u32;
    let fitted_height = (source.height() as f32 * scale).round().max(1.0) as u32;
    let fitted = image::imageops::resize(
        source,
        fitted_width,
        fitted_height,
        image::imageops::FilterType::Lanczos3,
    );
    let mut output = image::RgbaImage::new(width, height);
    image::imageops::overlay(
        &mut output,
        &fitted,
        i64::from((width - fitted_width) / 2),
        i64::from((height - fitted_height) / 2),
    );
    output
}

fn rectangle_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Nickel component rectangle vertices"),
        size: (capacity * mem::size_of::<Vertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn add_stroke(
    vertices: &mut Vec<Vertex>,
    surface: (u32, u32),
    rect: Rect,
    width: f32,
    color: [f32; 4],
) {
    let width = width.max(1.0);
    add_rectangle(
        vertices,
        surface,
        Rect::new(rect.origin.x, rect.origin.y, rect.size.width, width),
        color,
    );
    add_rectangle(
        vertices,
        surface,
        Rect::new(
            rect.origin.x,
            rect.origin.y + rect.size.height - width,
            rect.size.width,
            width,
        ),
        color,
    );
    add_rectangle(
        vertices,
        surface,
        Rect::new(rect.origin.x, rect.origin.y, width, rect.size.height),
        color,
    );
    add_rectangle(
        vertices,
        surface,
        Rect::new(
            rect.origin.x + rect.size.width - width,
            rect.origin.y,
            width,
            rect.size.height,
        ),
        color,
    );
}

fn add_rectangle(vertices: &mut Vec<Vertex>, surface: (u32, u32), rect: Rect, color: [f32; 4]) {
    let x1 = rect.origin.x / surface.0 as f32 * 2.0 - 1.0;
    let x2 = (rect.origin.x + rect.size.width) / surface.0 as f32 * 2.0 - 1.0;
    let y1 = 1.0 - rect.origin.y / surface.1 as f32 * 2.0;
    let y2 = 1.0 - (rect.origin.y + rect.size.height) / surface.1 as f32 * 2.0;
    for position in [[x1, y1], [x1, y2], [x2, y2], [x1, y1], [x2, y2], [x2, y1]] {
        vertices.push(Vertex { position, color });
    }
}

fn add_gradient(
    vertices: &mut Vec<Vertex>,
    surface: (u32, u32),
    rect: Rect,
    start: [f32; 4],
    end: [f32; 4],
    axis: GradientAxis,
) {
    let x1 = rect.origin.x / surface.0 as f32 * 2.0 - 1.0;
    let x2 = (rect.origin.x + rect.size.width) / surface.0 as f32 * 2.0 - 1.0;
    let y1 = 1.0 - rect.origin.y / surface.1 as f32 * 2.0;
    let y2 = 1.0 - (rect.origin.y + rect.size.height) / surface.1 as f32 * 2.0;
    let (top_left, top_right, bottom_left, bottom_right) = match axis {
        GradientAxis::Vertical => (start, start, end, end),
        GradientAxis::Horizontal => (start, end, start, end),
    };
    for (position, color) in [
        ([x1, y1], top_left),
        ([x1, y2], bottom_left),
        ([x2, y2], bottom_right),
        ([x1, y1], top_left),
        ([x2, y2], bottom_right),
        ([x2, y1], top_right),
    ] {
        vertices.push(Vertex { position, color });
    }
}

fn color_rgba(color: u32) -> [f32; 4] {
    [
        srgb_to_linear(((color >> 16) & 0xff) as u8),
        srgb_to_linear(((color >> 8) & 0xff) as u8),
        srgb_to_linear((color & 0xff) as u8),
        1.0,
    ]
}

fn srgb_to_linear(channel: u8) -> f32 {
    let channel = channel as f32 / 255.0;
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn text_color(color: u32) -> TextColor {
    TextColor::rgb(
        ((color >> 16) & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        (color & 0xff) as u8,
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
