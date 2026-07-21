use std::sync::Arc;

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, cosmic_text::Align,
};
use winit::window::Window;

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

    pub fn render(&mut self, hovered: bool) {
        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        self.renderer
            .prepare(
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
                        default_color: if hovered {
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
                ],
                &mut self.swash_cache,
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
            self.renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("panel text rendering succeeds");
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
    }
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

    use super::{launcher_button_contains, local_time_text};

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
}
