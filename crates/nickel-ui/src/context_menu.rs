use std::sync::Arc;

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use winit::{dpi::PhysicalPosition, window::Window};

use crate::rectangles::RectangleRenderer;

pub const WIDTH: u32 = 200;
pub const HEIGHT: u32 = 52;

pub struct ContextMenuGpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderer: TextRenderer,
    label: Buffer,
    rectangles: RectangleRenderer,
}

impl ContextMenuGpu {
    pub async fn new(window: Arc<Window>) -> Result<Self, String> {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .map_err(|error| error.to_string())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
            .map_err(|error| error.to_string())?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Nickel context menu device"),
                ..Default::default()
            })
            .await
            .map_err(|error| error.to_string())?;
        let config = surface
            .get_default_config(&adapter, WIDTH, HEIGHT)
            .ok_or_else(|| "context menu surface has no supported configuration".to_owned())?;
        surface.configure(&device, &config);
        let mut font_system = FontSystem::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, config.format);
        let renderer =
            TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let mut label = Buffer::new(&mut font_system, Metrics::new(18.0, 36.0));
        label.set_size(Some(WIDTH as f32 - 24.0), Some(HEIGHT as f32));
        label.set_text(
            "Close window",
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        label.shape_until_scroll(&mut font_system, false);
        let rectangles = RectangleRenderer::new(&device, config.format);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            font_system,
            swash_cache: SwashCache::new(),
            viewport,
            atlas,
            renderer,
            label,
            rectangles,
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

    pub fn render(&mut self, hovered: bool) {
        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        self.rectangles.update_raw(
            &self.queue,
            (self.config.width, self.config.height),
            if hovered {
                &[([4.0, 4.0, 196.0, 48.0], [0.14, 0.18, 0.25, 1.0])]
            } else {
                &[]
            },
        );
        self.renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [TextArea {
                    buffer: &self.label,
                    left: 16.0,
                    top: 8.0,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 4,
                        top: 4,
                        right: self.config.width as i32 - 4,
                        bottom: self.config.height as i32 - 4,
                    },
                    default_color: Color::rgb(238, 241, 248),
                    custom_glyphs: &[],
                }],
                &mut self.swash_cache,
            )
            .expect("context menu text preparation succeeds");
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Nickel context menu pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.045,
                            g: 0.055,
                            b: 0.075,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                ..Default::default()
            });
            self.rectangles.render(&mut pass);
            self.renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("context menu text rendering succeeds");
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
    }
}

pub fn item_contains(position: PhysicalPosition<f64>) -> bool {
    position.x >= 4.0
        && position.x < f64::from(WIDTH - 4)
        && position.y >= 4.0
        && position.y < f64::from(HEIGHT - 4)
}

#[cfg(test)]
mod tests {
    use super::{HEIGHT, WIDTH, item_contains};
    use winit::dpi::PhysicalPosition;

    #[test]
    fn close_item_respects_menu_padding() {
        assert!(item_contains(PhysicalPosition::new(20.0, 20.0)));
        assert!(!item_contains(PhysicalPosition::new(0.0, 20.0)));
        assert!(!item_contains(PhysicalPosition::new(
            f64::from(WIDTH),
            f64::from(HEIGHT)
        )));
    }
}
