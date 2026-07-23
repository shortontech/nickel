use std::sync::Arc;

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use winit::{dpi::PhysicalPosition, window::Window};

use crate::{graphics::SharedGraphics, rectangles::RectangleRenderer};

pub const WIDTH: u32 = 520;
pub const HEIGHT: u32 = 240;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Run,
    Cancel,
    Browse,
}

pub fn action_at(position: PhysicalPosition<f64>) -> Option<Action> {
    if !(174.0..=218.0).contains(&position.y) {
        return None;
    }
    match position.x {
        192.0..=292.0 => Some(Action::Run),
        300.0..=400.0 => Some(Action::Cancel),
        408.0..=508.0 => Some(Action::Browse),
        _ => None,
    }
}

pub struct RunDialogGpu {
    surface: wgpu::Surface<'static>,
    graphics: Arc<SharedGraphics>,
    config: wgpu::SurfaceConfiguration,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderer: TextRenderer,
    heading: Buffer,
    prompt: Buffer,
    input: Buffer,
    buttons: Vec<Buffer>,
    rectangles: RectangleRenderer,
}

impl RunDialogGpu {
    pub fn new(window: Arc<Window>, graphics: Arc<SharedGraphics>) -> Result<Self, String> {
        let surface = graphics.create_surface(window)?;
        let config = surface
            .get_default_config(&graphics.adapter, WIDTH, HEIGHT)
            .ok_or_else(|| "run dialog surface has no supported configuration".to_owned())?;
        let format = config.format;
        surface.configure(&graphics.device, &config);
        let mut font_system = FontSystem::new();
        let cache = Cache::new(&graphics.device);
        let viewport = Viewport::new(&graphics.device, &cache);
        let mut atlas = TextAtlas::new(&graphics.device, &graphics.queue, &cache, config.format);
        let renderer = TextRenderer::new(
            &mut atlas,
            &graphics.device,
            wgpu::MultisampleState::default(),
            None,
        );
        let heading = text_buffer(&mut font_system, "Run", 24.0, 32.0, 460.0);
        let prompt = text_buffer(
            &mut font_system,
            "Type the name of a program, folder, document, or internet resource.",
            16.0,
            23.0,
            456.0,
        );
        let input = text_buffer(&mut font_system, "", 18.0, 30.0, 430.0);
        let buttons = ["OK", "Cancel", "Browse…"]
            .into_iter()
            .map(|label| text_buffer(&mut font_system, label, 16.0, 24.0, 90.0))
            .collect();
        Ok(Self {
            surface,
            graphics: graphics.clone(),
            config,
            font_system,
            swash_cache: SwashCache::new(),
            viewport,
            atlas,
            renderer,
            heading,
            prompt,
            input,
            buttons,
            rectangles: RectangleRenderer::new(&graphics.device, format),
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

    pub fn render(&mut self, command: &str, hovered: Option<Action>) {
        self.input.set_text(
            &format!("{command}▏"),
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        self.input.shape_until_scroll(&mut self.font_system, false);
        self.viewport.update(
            &self.graphics.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        let mut rectangles = vec![
            ([28.0, 116.0, 492.0, 158.0], [0.03, 0.04, 0.06, 1.0]),
            ([28.0, 116.0, 492.0, 118.0], [0.28, 0.48, 0.82, 1.0]),
        ];
        for (index, action) in [Action::Run, Action::Cancel, Action::Browse]
            .into_iter()
            .enumerate()
        {
            let left = 192.0 + index as f32 * 108.0;
            rectangles.push((
                [left, 174.0, left + 100.0, 218.0],
                if hovered == Some(action) {
                    [0.2, 0.28, 0.4, 1.0]
                } else {
                    [0.12, 0.15, 0.21, 1.0]
                },
            ));
        }
        self.rectangles.update_raw(
            &self.graphics.queue,
            (self.config.width, self.config.height),
            &rectangles,
        );
        let mut areas = vec![
            area(
                &self.heading,
                28.0,
                18.0,
                self.config.width,
                self.config.height,
            ),
            area(
                &self.prompt,
                28.0,
                62.0,
                self.config.width,
                self.config.height,
            ),
            area(
                &self.input,
                42.0,
                122.0,
                self.config.width,
                self.config.height,
            ),
        ];
        for (index, button) in self.buttons.iter().enumerate() {
            areas.push(area(
                button,
                210.0 + index as f32 * 108.0,
                184.0,
                self.config.width,
                self.config.height,
            ));
        }
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
            .expect("run dialog text preparation succeeds");
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
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.055,
                            g: 0.07,
                            b: 0.10,
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
                .expect("run dialog rendering succeeds");
        }
        self.graphics.queue.submit([encoder.finish()]);
        self.graphics.queue.present(frame);
    }
}

fn text_buffer(
    font_system: &mut FontSystem,
    text: &str,
    size: f32,
    line_height: f32,
    width: f32,
) -> Buffer {
    let mut buffer = Buffer::new(font_system, Metrics::new(size, line_height));
    buffer.set_size(Some(width), Some(line_height));
    buffer.set_text(
        text,
        &Attrs::new().family(Family::SansSerif),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);
    buffer
}

fn area<'a>(buffer: &'a Buffer, left: f32, top: f32, width: u32, height: u32) -> TextArea<'a> {
    TextArea {
        buffer,
        left,
        top,
        scale: 1.0,
        bounds: TextBounds {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        },
        default_color: Color::rgb(232, 237, 247),
        custom_glyphs: &[],
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, action_at};
    use winit::dpi::PhysicalPosition;

    #[test]
    fn buttons_have_independent_hit_targets() {
        assert_eq!(
            action_at(PhysicalPosition::new(240.0, 190.0)),
            Some(Action::Run)
        );
        assert_eq!(
            action_at(PhysicalPosition::new(350.0, 190.0)),
            Some(Action::Cancel)
        );
        assert_eq!(
            action_at(PhysicalPosition::new(460.0, 190.0)),
            Some(Action::Browse)
        );
    }
}
