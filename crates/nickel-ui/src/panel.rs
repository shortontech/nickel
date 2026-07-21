use std::sync::Arc;

use glyphon::{
    Attrs, Buffer, Cache, Color, ContentType, CustomGlyph, Family, FontSystem, Metrics,
    RasterizeCustomGlyphRequest, RasterizedCustomGlyph, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport, cosmic_text::Align,
};
use winit::window::Window;

use crate::{icons, rectangles::RectangleRenderer};

const TASKS_LEFT: f64 = 208.0;
const TASK_WIDTH: f64 = 48.0;

pub struct PanelTask {
    pub id: u64,
    pub active: bool,
    pub icon: image::RgbaImage,
}

pub struct PanelGpu {
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
    clock: Buffer,
    clock_text: String,
    icon_buffer: Buffer,
    tasks: Vec<PanelTask>,
    rectangles: RectangleRenderer,
}

impl PanelGpu {
    pub async fn new(window: Arc<Window>) -> Result<Self, String> {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
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
                label: Some("Nickel panel device"),
                ..Default::default()
            })
            .await
            .map_err(|error| error.to_string())?;
        let size = window.inner_size();
        let config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| "panel surface has no supported configuration".to_owned())?;
        surface.configure(&device, &config);

        let mut font_system = FontSystem::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, config.format);
        let renderer =
            TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let mut label = Buffer::new(&mut font_system, Metrics::new(22.0, 44.0));
        label.set_size(Some(config.width as f32), Some(config.height as f32));
        label.set_text(
            "◆  Nickel",
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        label.shape_until_scroll(&mut font_system, false);
        let clock_text = local_time_text();
        let mut clock = Buffer::new(&mut font_system, Metrics::new(20.0, 44.0));
        clock.set_size(
            Some(config.width.saturating_sub(24) as f32),
            Some(config.height as f32),
        );
        clock.set_text(
            &clock_text,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            Some(Align::Right),
        );
        clock.shape_until_scroll(&mut font_system, false);
        let mut icon_buffer = Buffer::new(&mut font_system, Metrics::new(1.0, 1.0));
        icon_buffer.set_size(Some(config.width as f32), Some(config.height as f32));
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
            clock,
            clock_text,
            icon_buffer,
            tasks: Vec::new(),
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
        self.label.set_size(Some(width as f32), Some(height as f32));
        self.clock
            .set_size(Some(width.saturating_sub(24) as f32), Some(height as f32));
        self.icon_buffer
            .set_size(Some(width as f32), Some(height as f32));
    }

    pub fn update_clock(&mut self) -> bool {
        let text = local_time_text();
        if text == self.clock_text {
            return false;
        }
        self.clock_text = text;
        self.clock.set_text(
            &self.clock_text,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            Some(Align::Right),
        );
        self.clock.shape_until_scroll(&mut self.font_system, false);
        true
    }

    pub fn set_tasks(&mut self, tasks: Vec<PanelTask>) {
        self.tasks = tasks;
    }

    pub fn render(&mut self, launcher_hovered: bool, task_hovered: Option<usize>) {
        let custom_glyphs = self
            .tasks
            .iter()
            .enumerate()
            .map(|(index, task)| CustomGlyph {
                id: u16::try_from(task.id).unwrap_or(u16::MAX),
                left: (TASKS_LEFT + index as f64 * TASK_WIDTH + 8.0) as f32,
                top: 12.0,
                width: 32.0,
                height: 32.0,
                color: None,
                snap_to_physical_pixel: true,
                metadata: 0,
            })
            .collect::<Vec<_>>();
        let mut rectangles = Vec::new();
        if let Some(index) = task_hovered {
            let left = TASKS_LEFT as f32 + index as f32 * TASK_WIDTH as f32;
            rectangles.push((
                [left + 2.0, 4.0, left + 46.0, 52.0],
                [0.12, 0.15, 0.21, 1.0],
            ));
        }
        for (index, task) in self.tasks.iter().enumerate() {
            if task.active {
                let left = TASKS_LEFT as f32 + index as f32 * TASK_WIDTH as f32;
                rectangles.push((
                    [left + 12.0, 52.0, left + 36.0, 55.0],
                    [0.3, 0.62, 1.0, 1.0],
                ));
            }
        }
        self.rectangles.update_raw(
            &self.queue,
            (self.config.width, self.config.height),
            &rectangles,
        );
        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        self.renderer
            .prepare_with_custom(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [
                    TextArea {
                        buffer: &self.label,
                        left: 48.0,
                        top: 6.0,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: 0,
                            top: 0,
                            right: 200,
                            bottom: self.config.height as i32,
                        },
                        default_color: if launcher_hovered {
                            Color::rgb(120, 180, 255)
                        } else {
                            Color::rgb(238, 241, 248)
                        },
                        custom_glyphs: &[],
                    },
                    TextArea {
                        buffer: &self.clock,
                        left: 0.0,
                        top: 6.0,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: 200,
                            top: 0,
                            right: self.config.width as i32,
                            bottom: self.config.height as i32,
                        },
                        default_color: Color::rgb(238, 241, 248),
                        custom_glyphs: &[],
                    },
                    TextArea {
                        buffer: &self.icon_buffer,
                        left: 0.0,
                        top: 0.0,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: TASKS_LEFT as i32,
                            top: 0,
                            right: self.config.width.saturating_sub(100) as i32,
                            bottom: self.config.height as i32,
                        },
                        default_color: Color::rgb(238, 241, 248),
                        custom_glyphs: &custom_glyphs,
                    },
                ],
                &mut self.swash_cache,
                &|request: RasterizeCustomGlyphRequest| {
                    let source = &self
                        .tasks
                        .iter()
                        .find(|task| u16::try_from(task.id).ok() == Some(request.id))?
                        .icon;
                    let image = icons::resized(source, request.width.into(), request.height.into());
                    Some(RasterizedCustomGlyph {
                        data: image.into_raw(),
                        content_type: ContentType::Color,
                    })
                },
            )
            .expect("panel text preparation succeeds");

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Nickel panel encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Nickel panel pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.035,
                            g: 0.045,
                            b: 0.065,
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
                .expect("panel text rendering succeeds");
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
    }
}

pub fn task_at(position: winit::dpi::PhysicalPosition<f64>, task_count: usize) -> Option<usize> {
    if position.y < 0.0 || position.y >= 56.0 || position.x < TASKS_LEFT {
        return None;
    }
    let index = ((position.x - TASKS_LEFT) / TASK_WIDTH) as usize;
    (index < task_count).then_some(index)
}

pub fn fallback_icon() -> image::RgbaImage {
    image::RgbaImage::from_fn(32, 32, |x, y| {
        let border = x <= 2 || y <= 2 || x >= 29 || y >= 29;
        image::Rgba(if border {
            [155, 169, 194, 255]
        } else {
            [70, 80, 100, 255]
        })
    })
}

pub fn launcher_button_contains(position: winit::dpi::PhysicalPosition<f64>) -> bool {
    position.x >= 0.0 && position.x < 200.0 && position.y >= 0.0 && position.y < 56.0
}

fn local_time_text() -> String {
    jiff::Zoned::now().strftime("%H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use winit::dpi::PhysicalPosition;

    use super::{launcher_button_contains, local_time_text, task_at};

    #[test]
    fn local_clock_uses_zero_padded_hour_and_minute() {
        let text = local_time_text();
        assert_eq!(text.len(), 5);
        assert_eq!(text.as_bytes()[2], b':');
        assert!(text.chars().enumerate().all(|(index, character)| {
            index == 2 && character == ':' || index != 2 && character.is_ascii_digit()
        }));
    }

    #[test]
    fn launcher_button_does_not_include_clock_area() {
        assert!(launcher_button_contains(PhysicalPosition::new(100.0, 28.0)));
        assert!(!launcher_button_contains(PhysicalPosition::new(
            900.0, 28.0
        )));
    }

    #[test]
    fn task_hit_testing_starts_after_launcher_button() {
        assert_eq!(task_at(PhysicalPosition::new(208.0, 28.0), 2), Some(0));
        assert_eq!(task_at(PhysicalPosition::new(255.0, 28.0), 2), Some(0));
        assert_eq!(task_at(PhysicalPosition::new(256.0, 28.0), 2), Some(1));
        assert_eq!(task_at(PhysicalPosition::new(304.0, 28.0), 2), None);
    }
}
