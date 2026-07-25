use std::sync::Arc;

use glyphon::{
    Attrs, Buffer, Cache, Color, ContentType, CustomGlyph, Family, FontSystem, Metrics,
    RasterizeCustomGlyphRequest, RasterizedCustomGlyph, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport,
};
use winit::{dpi::PhysicalPosition, window::Window};

use crate::{graphics::SharedGraphics, icons, rectangles::RectangleRenderer};

pub const WIDTH: u32 = 320;
pub const HEIGHT: u32 = 52;
const ROW_HEIGHT: u32 = 44;
pub const PREVIEW_CARD_WIDTH: u32 = 260;
pub const PREVIEW_HEIGHT: u32 = 190;
const PREVIEW_GLYPH_BASE: u16 = 50_000;

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
    preview_images: Vec<image::RgbaImage>,
    close_label: Buffer,
    rectangles: RectangleRenderer,
}

impl ContextMenuGpu {
    pub fn new(window: Arc<Window>, graphics: Arc<SharedGraphics>) -> Result<Self, String> {
        let surface = graphics.create_surface(window)?;
        let mut config = surface
            .get_default_config(&graphics.adapter, WIDTH, HEIGHT)
            .ok_or_else(|| "context menu surface has no supported configuration".to_owned())?;
        config.desired_maximum_frame_latency = 1;
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
        let mut close_label = Buffer::new(&mut font_system, Metrics::new(22.0, 28.0));
        close_label.set_size(Some(24.0), Some(28.0));
        close_label.set_text(
            "×",
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        close_label.shape_until_scroll(&mut font_system, false);
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
            labels: Vec::new(),
            preview_images: Vec::new(),
            close_label,
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
        self.preview_images.clear();
        self.labels = make_labels(&mut self.font_system, labels);
    }

    pub fn set_previews(&mut self, labels: &[String], images: Vec<image::RgbaImage>) {
        self.labels = make_labels(&mut self.font_system, labels);
        self.preview_images = images;
    }

    pub fn render(&mut self, hovered: Option<usize>) {
        let preview_mode = !self.preview_images.is_empty();
        self.viewport.update(
            &self.graphics.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        let mut rectangles = Vec::new();
        if preview_mode {
            for index in 0..self.preview_images.len() {
                let left = 4.0 + index as f32 * PREVIEW_CARD_WIDTH as f32;
                rectangles.push((
                    [
                        left,
                        4.0,
                        left + PREVIEW_CARD_WIDTH as f32 - 4.0,
                        PREVIEW_HEIGHT as f32 - 4.0,
                    ],
                    [0.08, 0.1, 0.14, 1.0],
                ));
                rectangles.push((
                    [
                        left + PREVIEW_CARD_WIDTH as f32 - 34.0,
                        8.0,
                        left + PREVIEW_CARD_WIDTH as f32 - 8.0,
                        34.0,
                    ],
                    [0.5, 0.12, 0.14, 1.0],
                ));
            }
        }
        if let Some(index) = hovered {
            if preview_mode {
                let left = 4.0 + index as f32 * PREVIEW_CARD_WIDTH as f32;
                rectangles.push((
                    [
                        left,
                        4.0,
                        left + PREVIEW_CARD_WIDTH as f32 - 4.0,
                        PREVIEW_HEIGHT as f32 - 4.0,
                    ],
                    [0.14, 0.18, 0.25, 1.0],
                ));
            } else {
                let top = 4.0 + index as f32 * ROW_HEIGHT as f32;
                rectangles.push((
                    [4.0, top, WIDTH as f32 - 4.0, top + ROW_HEIGHT as f32],
                    [0.14, 0.18, 0.25, 1.0],
                ));
            }
        }
        self.rectangles.update_raw(
            &self.graphics.queue,
            (self.config.width, self.config.height),
            &rectangles,
        );
        let mut areas: Vec<_> = self
            .labels
            .iter()
            .enumerate()
            .map(|(index, label)| TextArea {
                buffer: label,
                left: if preview_mode {
                    12.0 + index as f32 * PREVIEW_CARD_WIDTH as f32
                } else {
                    16.0
                },
                top: if preview_mode {
                    4.0
                } else {
                    8.0 + index as f32 * ROW_HEIGHT as f32
                },
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
        if preview_mode {
            areas.extend(
                self.preview_images
                    .iter()
                    .enumerate()
                    .map(|(index, _)| TextArea {
                        buffer: &self.close_label,
                        left: (index as f32 + 1.0) * PREVIEW_CARD_WIDTH as f32 - 30.0,
                        top: 3.0,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: 0,
                            top: 0,
                            right: self.config.width as i32,
                            bottom: self.config.height as i32,
                        },
                        default_color: Color::rgb(255, 255, 255),
                        custom_glyphs: &[],
                    }),
            );
        }
        let custom_glyphs: Vec<_> = self
            .preview_images
            .iter()
            .enumerate()
            .filter_map(|(index, _)| {
                Some(CustomGlyph {
                    id: PREVIEW_GLYPH_BASE.checked_add(u16::try_from(index).ok()?)?,
                    left: 10.0 + index as f32 * PREVIEW_CARD_WIDTH as f32,
                    top: 44.0,
                    width: 240.0,
                    height: 135.0,
                    color: None,
                    snap_to_physical_pixel: true,
                    metadata: 0,
                })
            })
            .collect();
        if let Some(area) = areas.first_mut() {
            area.custom_glyphs = &custom_glyphs;
        }
        self.renderer
            .prepare_with_custom(
                &self.graphics.device,
                &self.graphics.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash_cache,
                &|request: RasterizeCustomGlyphRequest| {
                    let index = usize::from(request.id.checked_sub(PREVIEW_GLYPH_BASE)?);
                    let image = icons::resized(
                        self.preview_images.get(index)?,
                        request.width.into(),
                        request.height.into(),
                    );
                    Some(RasterizedCustomGlyph {
                        data: image.into_raw(),
                        content_type: ContentType::Color,
                    })
                },
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

fn make_labels(font_system: &mut FontSystem, labels: &[String]) -> Vec<Buffer> {
    labels
        .iter()
        .map(|label| {
            let mut buffer = Buffer::new(font_system, Metrics::new(18.0, 36.0));
            buffer.set_size(Some(WIDTH as f32 - 24.0), Some(ROW_HEIGHT as f32));
            buffer.set_text(
                label,
                &Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(font_system, false);
            buffer
        })
        .collect()
}

pub fn height_for(count: usize) -> u32 {
    8 + ROW_HEIGHT * u32::try_from(count.max(1)).unwrap_or(u32::MAX / ROW_HEIGHT)
}
pub fn preview_width(count: usize) -> u32 {
    8 + PREVIEW_CARD_WIDTH * u32::try_from(count.max(1)).unwrap_or(u32::MAX / PREVIEW_CARD_WIDTH)
}

pub fn item_at(position: PhysicalPosition<f64>, count: usize) -> Option<usize> {
    if position.x < 4.0 || position.x >= f64::from(WIDTH - 4) || position.y < 4.0 {
        return None;
    }
    let index = ((position.y - 4.0) / f64::from(ROW_HEIGHT)) as usize;
    (index < count).then_some(index)
}

pub fn preview_at(position: PhysicalPosition<f64>, count: usize) -> Option<usize> {
    if position.y < 4.0 || position.y >= f64::from(PREVIEW_HEIGHT - 4) || position.x < 4.0 {
        return None;
    }
    let index = ((position.x - 4.0) / f64::from(PREVIEW_CARD_WIDTH)) as usize;
    (index < count).then_some(index)
}

pub fn preview_close_at(position: PhysicalPosition<f64>, count: usize) -> Option<usize> {
    let index = preview_at(position, count)?;
    let left = 4.0 + index as f64 * f64::from(PREVIEW_CARD_WIDTH);
    (position.x >= left + f64::from(PREVIEW_CARD_WIDTH) - 34.0
        && position.x < left + f64::from(PREVIEW_CARD_WIDTH) - 8.0
        && position.y >= 8.0
        && position.y < 34.0)
        .then_some(index)
}

#[cfg(test)]
mod tests {
    use super::{HEIGHT, PREVIEW_CARD_WIDTH, WIDTH, item_at, preview_at, preview_close_at};
    use winit::dpi::PhysicalPosition;

    #[test]
    fn menu_item_respects_padding() {
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

    #[test]
    fn preview_cards_and_close_buttons_are_independent() {
        assert_eq!(preview_at(PhysicalPosition::new(20.0, 80.0), 2), Some(0));
        assert_eq!(
            preview_at(
                PhysicalPosition::new(f64::from(PREVIEW_CARD_WIDTH) + 20.0, 80.0),
                2
            ),
            Some(1)
        );
        assert_eq!(
            preview_close_at(
                PhysicalPosition::new(f64::from(PREVIEW_CARD_WIDTH) - 20.0, 20.0),
                2
            ),
            Some(0)
        );
    }
}
