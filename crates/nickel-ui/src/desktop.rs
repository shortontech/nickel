use std::sync::Arc;

use image::{Rgba, RgbaImage, imageops::FilterType};
use winit::window::Window;

use crate::graphics::SharedGraphics;

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
    wallpaper: Wallpaper,
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

impl DesktopGpu {
    pub fn new(
        window: Arc<Window>,
        graphics: Arc<SharedGraphics>,
        wallpaper: Wallpaper,
    ) -> Result<Self, String> {
        let surface = graphics.create_surface(window.clone())?;
        let size = window.inner_size();
        let config = surface
            .get_default_config(&graphics.adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| "desktop surface has no supported configuration".to_owned())?;
        surface.configure(&graphics.device, &config);
        let composed = compose_wallpaper(&wallpaper, config.width, config.height);
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
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    let uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 2.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(2.0, 0.0),
    );
    var output: VertexOutput;
    output.position = vec4<f32>(positions[index], 0.0, 1.0);
    output.uv = uvs[index];
    return output;
}

@group(0) @binding(0) var wallpaper: texture_2d<f32>;
@group(0) @binding(1) var wallpaper_sampler: sampler;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(wallpaper, wallpaper_sampler, input.uv);
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
        let (texture, bind_group) = upload_wallpaper(&graphics, &bind_group_layout, &composed);
        Ok(Self {
            surface,
            graphics,
            config,
            wallpaper,
            texture,
            bind_group,
            pipeline,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.graphics.device, &self.config);
        let composed = compose_wallpaper(&self.wallpaper, width, height);
        let layout = self.pipeline.get_bind_group_layout(0);
        (self.texture, self.bind_group) = upload_wallpaper(&self.graphics, &layout, &composed);
    }

    pub fn render(&mut self) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.graphics.device, &self.config);
                return;
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
            pass.draw(0..3, 0..1);
        }
        self.graphics.queue.submit([encoder.finish()]);
        self.graphics.queue.present(frame);
    }
}

fn upload_wallpaper(
    graphics: &SharedGraphics,
    layout: &wgpu::BindGroupLayout,
    image: &RgbaImage,
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
            ],
        });
    (texture, bind_group)
}

fn compose_wallpaper(wallpaper: &Wallpaper, width: u32, height: u32) -> RgbaImage {
    let mut target = RgbaImage::from_pixel(
        width.max(1),
        height.max(1),
        Rgba([
            wallpaper.color[0],
            wallpaper.color[1],
            wallpaper.color[2],
            255,
        ]),
    );
    let Some(source) = wallpaper.image.as_ref() else {
        return target;
    };
    match wallpaper.position {
        WallpaperPosition::Stretch => {
            target = image::imageops::resize(source, width, height, FilterType::Lanczos3);
        }
        WallpaperPosition::Tile => {
            for y in (0..height).step_by(source.height().max(1) as usize) {
                for x in (0..width).step_by(source.width().max(1) as usize) {
                    image::imageops::overlay(&mut target, source, i64::from(x), i64::from(y));
                }
            }
        }
        WallpaperPosition::Center => overlay_centered(&mut target, source),
        WallpaperPosition::Fit => {
            let resized = scaled(source, width, height, false);
            overlay_centered(&mut target, &resized);
        }
        WallpaperPosition::Fill | WallpaperPosition::Span => {
            let resized = scaled(source, width, height, true);
            overlay_centered(&mut target, &resized);
        }
    }
    target
}

fn scaled(source: &RgbaImage, width: u32, height: u32, cover: bool) -> RgbaImage {
    let x_scale = width as f64 / source.width().max(1) as f64;
    let y_scale = height as f64 / source.height().max(1) as f64;
    let scale = if cover {
        x_scale.max(y_scale)
    } else {
        x_scale.min(y_scale)
    };
    image::imageops::resize(
        source,
        (source.width() as f64 * scale).round().max(1.0) as u32,
        (source.height() as f64 * scale).round().max(1.0) as u32,
        FilterType::Lanczos3,
    )
}

fn overlay_centered(target: &mut RgbaImage, source: &RgbaImage) {
    let x = (i64::from(target.width()) - i64::from(source.width())) / 2;
    let y = (i64::from(target.height()) - i64::from(source.height())) / 2;
    image::imageops::overlay(target, source, x, y);
}

#[cfg(test)]
mod tests {
    use super::{Wallpaper, WallpaperPosition, compose_wallpaper};
    use image::{Rgba, RgbaImage};

    #[test]
    fn fit_preserves_background_bars() {
        let wallpaper = Wallpaper {
            image: Some(RgbaImage::from_pixel(4, 2, Rgba([255, 0, 0, 255]))),
            color: [0, 0, 255],
            position: WallpaperPosition::Fit,
        };
        let result = compose_wallpaper(&wallpaper, 4, 4);
        assert_eq!(result.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(result.get_pixel(2, 2).0, [255, 0, 0, 255]);
    }

    #[test]
    fn tile_repeats_source_pixels() {
        let mut image = RgbaImage::new(2, 1);
        image.put_pixel(0, 0, Rgba([10, 0, 0, 255]));
        image.put_pixel(1, 0, Rgba([20, 0, 0, 255]));
        let result = compose_wallpaper(
            &Wallpaper {
                image: Some(image),
                color: [0, 0, 0],
                position: WallpaperPosition::Tile,
            },
            4,
            1,
        );
        assert_eq!(result.get_pixel(2, 0).0, [10, 0, 0, 255]);
        assert_eq!(result.get_pixel(3, 0).0, [20, 0, 0, 255]);
    }
}
