use std::sync::Arc;

use image::{Rgba, RgbaImage};
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::graphics::SharedGraphics;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WallpaperUniform {
    position: u32,
    has_image: u32,
    _padding: [u32; 2],
    source_size: [f32; 2],
    target_size: [f32; 2],
    background: [f32; 4],
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WallpaperPosition {
    Center,
    Tile,
    Stretch,
    Fit,
    Span,
    #[default]
    Fill,
}

#[derive(Clone, Debug)]
pub struct Wallpaper {
    pub image: Option<RgbaImage>,
    pub color: [u8; 3],
    pub position: WallpaperPosition,
}

impl Default for Wallpaper {
    fn default() -> Self {
        Self {
            image: None,
            color: [14, 18, 26],
            position: WallpaperPosition::Fill,
        }
    }
}

pub struct DesktopGpu {
    surface: wgpu::Surface<'static>,
    graphics: Arc<SharedGraphics>,
    config: wgpu::SurfaceConfiguration,
    _texture: wgpu::Texture,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform: WallpaperUniform,
}

impl DesktopGpu {
    pub fn new(
        window: Arc<Window>,
        graphics: Arc<SharedGraphics>,
        mut wallpaper: Wallpaper,
    ) -> Result<Self, String> {
        let surface = graphics.create_surface(window.clone())?;
        let size = crate::platform::surface_size(&window);
        let mut config = surface
            .get_default_config(&graphics.adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| "desktop surface has no supported configuration".to_owned())?;
        config.desired_maximum_frame_latency = 1;
        surface.configure(&graphics.device, &config);
        let has_image = wallpaper.image.is_some();
        let upload_image = wallpaper
            .image
            .take()
            .unwrap_or_else(|| RgbaImage::from_pixel(1, 1, Rgba([255, 255, 255, 255])));
        let bind_group_layout =
            graphics
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Nickel desktop wallpaper bind group layout"),
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
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
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
        let pipeline_layout =
            graphics
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Nickel desktop wallpaper pipeline layout"),
                    bind_group_layouts: &[Some(&bind_group_layout)],
                    immediate_size: 0,
                });
        let shader = graphics
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Nickel desktop wallpaper shader"),
                source: wgpu::ShaderSource::Wgsl(
                    r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
    );
    var output: VertexOutput;
    output.position = vec4<f32>(positions[index], 0.0, 1.0);
    let uv = positions[index] * 0.5 + vec2<f32>(0.5);
    output.uv = vec2<f32>(uv.x, 1.0 - uv.y);
    return output;
}

@group(0) @binding(0) var wallpaper: texture_2d<f32>;
@group(0) @binding(1) var wallpaper_sampler: sampler;
struct WallpaperSettings {
    position: u32,
    has_image: u32,
    padding: vec2<u32>,
    source_size: vec2<f32>,
    target_size: vec2<f32>,
    background: vec4<f32>,
};
@group(0) @binding(2) var<uniform> settings: WallpaperSettings;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if settings.has_image == 0u {
        return settings.background;
    }
    var uv = input.uv;
    var inside = true;
    if settings.position == 1u {
        uv = fract(input.uv * settings.target_size / settings.source_size);
    } else if settings.position != 2u {
        var displayed = settings.source_size;
        if settings.position == 3u {
                let scale = min(
                    settings.target_size.x / settings.source_size.x,
                    settings.target_size.y / settings.source_size.y,
                );
                displayed *= scale;
            } else if settings.position == 4u || settings.position == 5u {
                let scale = max(
                    settings.target_size.x / settings.source_size.x,
                    settings.target_size.y / settings.source_size.y,
                );
                displayed *= scale;
            }
            let pixel = input.uv * settings.target_size;
            uv = (pixel - (settings.target_size - displayed) * 0.5) / displayed;
            inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
    }
    if !inside {
        return settings.background;
    }
    return textureSample(wallpaper, wallpaper_sampler, uv);
}
"#
                    .into(),
                ),
            });
        let pipeline = graphics
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Nickel desktop wallpaper pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format,
                        blend: None,
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
        let uniform = wallpaper_uniform(
            &wallpaper,
            &upload_image,
            config.width,
            config.height,
            has_image,
        );
        let uniform_buffer =
            graphics
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Nickel desktop wallpaper settings"),
                    contents: bytemuck::bytes_of(&uniform),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });
        let (texture, bind_group) = upload_wallpaper(
            &graphics,
            &bind_group_layout,
            &upload_image,
            &uniform_buffer,
        );
        Ok(Self {
            surface,
            graphics,
            config,
            _texture: texture,
            bind_group_layout,
            bind_group,
            pipeline,
            uniform_buffer,
            uniform,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.graphics.device, &self.config);
        self.uniform.target_size = [width as f32, height as f32];
    }

    pub fn set_wallpaper(&mut self, mut wallpaper: Wallpaper) {
        let has_image = wallpaper.image.is_some();
        let image = wallpaper
            .image
            .take()
            .unwrap_or_else(|| RgbaImage::from_pixel(1, 1, Rgba([255, 255, 255, 255])));
        self.uniform = wallpaper_uniform(
            &wallpaper,
            &image,
            self.config.width,
            self.config.height,
            has_image,
        );
        let (texture, bind_group) = upload_wallpaper(
            &self.graphics,
            &self.bind_group_layout,
            &image,
            &self.uniform_buffer,
        );
        self._texture = texture;
        self.bind_group = bind_group;
    }

    pub fn render(&mut self) {
        self.graphics.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&self.uniform),
        );
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.graphics.device, &self.config);
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    _ => return,
                }
            }
            _ => return,
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self
            .graphics
            .device
            .create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Nickel desktop wallpaper pass"),
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
        self.graphics.queue.submit([encoder.finish()]);
        self.graphics.queue.present(frame);
    }
}

fn upload_wallpaper(
    graphics: &SharedGraphics,
    layout: &wgpu::BindGroupLayout,
    image: &RgbaImage,
    uniform_buffer: &wgpu::Buffer,
) -> (wgpu::Texture, wgpu::BindGroup) {
    let size = wgpu::Extent3d {
        width: image.width(),
        height: image.height(),
        depth_or_array_layers: 1,
    };
    let texture = graphics.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Nickel desktop wallpaper texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    graphics.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        image.as_raw(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * image.width()),
            rows_per_image: Some(image.height()),
        },
        size,
    );
    let view = texture.create_view(&Default::default());
    let sampler = graphics.device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Nickel desktop wallpaper sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let bind_group = graphics
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Nickel desktop wallpaper bind group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });
    (texture, bind_group)
}

fn wallpaper_uniform(
    wallpaper: &Wallpaper,
    image: &RgbaImage,
    width: u32,
    height: u32,
    has_image: bool,
) -> WallpaperUniform {
    let position = match wallpaper.position {
        WallpaperPosition::Center => 0,
        WallpaperPosition::Tile => 1,
        WallpaperPosition::Stretch => 2,
        WallpaperPosition::Fit => 3,
        WallpaperPosition::Span => 4,
        WallpaperPosition::Fill => 5,
    };
    WallpaperUniform {
        position,
        has_image: u32::from(has_image),
        _padding: [0; 2],
        source_size: [image.width().max(1) as f32, image.height().max(1) as f32],
        target_size: [width.max(1) as f32, height.max(1) as f32],
        background: [
            f32::from(wallpaper.color[0]) / 255.0,
            f32::from(wallpaper.color[1]) / 255.0,
            f32::from(wallpaper.color[2]) / 255.0,
            1.0,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::{Wallpaper, WallpaperPosition, wallpaper_uniform};
    use image::{Rgba, RgbaImage};

    #[test]
    fn fit_is_encoded_for_shader_positioning() {
        let wallpaper = Wallpaper {
            image: Some(RgbaImage::from_pixel(4, 2, Rgba([255, 0, 0, 255]))),
            color: [0, 0, 255],
            position: WallpaperPosition::Fit,
        };
        let uniform = wallpaper_uniform(&wallpaper, wallpaper.image.as_ref().unwrap(), 4, 4, true);
        assert_eq!(uniform.position, 3);
        assert_eq!(uniform.source_size, [4.0, 2.0]);
        assert_eq!(uniform.target_size, [4.0, 4.0]);
        assert_eq!(uniform.background, [0.0, 0.0, 1.0, 1.0]);
    }
}
