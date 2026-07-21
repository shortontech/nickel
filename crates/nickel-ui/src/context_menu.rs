use std::sync::Arc;

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use winit::{dpi::PhysicalPosition, window::Window};

use crate::{graphics::SharedGraphics, rectangles::RectangleRenderer};

pub const WIDTH: u32 = 320;
pub const HEIGHT: u32 = 52;
const ROW_HEIGHT: u32 = 44;

pub struct ContextMenuGpu {
    surface: wgpu::Surface<'static>,
    graphics: Arc<SharedGraphics>,
    config: wgpu::SurfaceConfiguration,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderer: TextRenderer,
    labels: Vec<Buffer>,
    rectangles: RectangleRenderer,
}

impl ContextMenuGpu {
    pub fn new(window: Arc<Window>, graphics: Arc<SharedGraphics>) -> Result<Self, String> {
        let surface = graphics.create_surface(window)?;
        let config = surface
            .get_default_config(&graphics.adapter, WIDTH, HEIGHT)
            .ok_or_else(|| "context menu surface has no supported configuration".to_owned())?;
        surface.configure(&graphics.device, &config);
        let font_system = FontSystem::new();
        let cache = Cache::new(&graphics.device);
        let viewport = Viewport::new(&graphics.device, &cache);
        let mut atlas = TextAtlas::new(&graphics.device, &graphics.queue, &cache, config.format);
        let renderer = TextRenderer::new(
            &mut atlas,
            &graphics.device,
            wgpu::MultisampleState::default(),
            None,
        );
        let labels = Vec::new();
        let rectangles = RectangleRenderer::new(&graphics.device, config.format);
        Ok(Self {
            surface,
            graphics,
            config,
            font_system,
            swash_cache: SwashCache::new(),
            viewport,
            atlas,
            renderer,
            labels,
            rectangles,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.graphics.device, &self.config);
    }

    pub fn set_labels(&mut self, labels: &[String]) {
        self.labels = labels
            .iter()
            .map(|label| {
                let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(18.0, 36.0));
                buffer.set_size(Some(WIDTH as f32 - 24.0), Some(ROW_HEIGHT as f32));
                buffer.set_text(
                    label,
                    &Attrs::new().family(Family::SansSerif),
                    Shaping::Advanced,
                    None,
                );
                buffer.shape_until_scroll(&mut self.font_system, false);
                buffer
            })
            .collect();
    }

    pub fn render(&mut self, hovered: Option<usize>) {
        self.viewport.update(
            &self.graphics.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        self.rectangles.update_raw(
            &self.graphics.queue,
            (self.config.width, self.config.height),
            &hovered
                .map(|index| {
                    let top = 4.0 + index as f32 * ROW_HEIGHT as f32;
                    vec![(
                        [4.0, top, WIDTH as f32 - 4.0, top + ROW_HEIGHT as f32],
                        [0.14, 0.18, 0.25, 1.0],
                    )]
                })
                .unwrap_or_default(),
        );
        let areas: Vec<_> = self
            .labels
            .iter()
            .enumerate()
            .map(|(index, label)| TextArea {
                buffer: label,
                left: 16.0,
                top: 8.0 + index as f32 * ROW_HEIGHT as f32,
                scale: 1.0,
                bounds: TextBounds {
                    left: 4,
                    top: 4,
                    right: self.config.width as i32 - 4,
                    bottom: self.config.height as i32 - 4,
                },
                default_color: Color::rgb(238, 241, 248),
                custom_glyphs: &[],
            })
            .collect();
        self.renderer
            .prepare(
                &self.graphics.device,
                &self.graphics.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash_cache,
            )
            .expect("context menu text preparation succeeds");
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
        self.graphics.queue.submit([encoder.finish()]);
        self.graphics.queue.present(frame);
    }
}

pub fn height_for(count: usize) -> u32 {
    8 + ROW_HEIGHT * u32::try_from(count.max(1)).unwrap_or(u32::MAX / ROW_HEIGHT)
}

pub fn item_at(position: PhysicalPosition<f64>, count: usize) -> Option<usize> {
    if position.x < 4.0 || position.x >= f64::from(WIDTH - 4) || position.y < 4.0 {
        return None;
    }
    let index = ((position.y - 4.0) / f64::from(ROW_HEIGHT)) as usize;
    (index < count).then_some(index)
}

#[cfg(test)]
mod tests {
    use super::{HEIGHT, WIDTH, item_at};
    use winit::dpi::PhysicalPosition;

    #[test]
    fn close_item_respects_menu_padding() {
        assert_eq!(item_at(PhysicalPosition::new(20.0, 20.0), 1), Some(0));
        assert_eq!(item_at(PhysicalPosition::new(0.0, 20.0), 1), None);
        assert_eq!(
            item_at(
                PhysicalPosition::new(f64::from(WIDTH), f64::from(HEIGHT)),
                1
            ),
            None
        );
    }
}
