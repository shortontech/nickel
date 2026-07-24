use std::sync::Arc;

use glyphon::{
    Attrs, Buffer, Cache, Color, ContentType, CustomGlyph, Family, FontSystem, Metrics,
    RasterizeCustomGlyphRequest, RasterizedCustomGlyph, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport, cosmic_text::Align,
};
use winit::window::Window;

use crate::{graphics::SharedGraphics, icons, rectangles::RectangleRenderer};

const LAUNCHER_BUTTON_WIDTH: f64 = 56.0;
const TASKS_LEFT: f64 = 64.0;
const TASK_WIDTH: f64 = 48.0;
const PANEL_ICON_GLYPH_ID: u16 = u16::MAX - 1;
const TRAY_GLYPH_BASE: u16 = 60_000;
const TRAY_WIDTH: f64 = 40.0;
const CLOCK_WIDTH: f64 = 100.0;

pub struct PanelTask {
    pub glyph_id: u16,
    pub active: bool,
    pub icon: image::RgbaImage,
}

pub struct PanelTrayItem {
    pub icon: image::RgbaImage,
}

pub struct PanelGpu {
    surface: wgpu::Surface<'static>,
    graphics: Arc<SharedGraphics>,
    config: wgpu::SurfaceConfiguration,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderer: TextRenderer,
    clock: Buffer,
    clock_text: String,
    icon_buffer: Buffer,
    tasks: Vec<PanelTask>,
    tray_items: Vec<PanelTrayItem>,
    rectangles: RectangleRenderer,
    panel_icon: image::RgbaImage,
}

impl PanelGpu {
    pub fn new(window: Arc<Window>, graphics: Arc<SharedGraphics>) -> Result<Self, String> {
        let surface = graphics.create_surface(window.clone())?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&graphics.adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| "panel surface has no supported configuration".to_owned())?;
        let capabilities = surface.get_capabilities(&graphics.adapter);
        if capabilities
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            config.alpha_mode = wgpu::CompositeAlphaMode::PreMultiplied;
        } else if capabilities
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PostMultiplied)
        {
            config.alpha_mode = wgpu::CompositeAlphaMode::PostMultiplied;
        }
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
        let clock_text = local_clock_text();
        let mut clock = Buffer::new(&mut font_system, Metrics::new(14.0, 22.0));
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
        let rectangles = RectangleRenderer::new(&graphics.device, config.format);
        let panel_icon = tinted_panel_icon(
            image::load_from_memory(include_bytes!("../../../assets/icons/nickel-panel.png"))
                .map_err(|error| format!("failed to decode Nickel panel icon: {error}"))?
                .into_rgba8(),
        );

        Ok(Self {
            surface,
            graphics,
            config,
            font_system,
            swash_cache: SwashCache::new(),
            viewport,
            atlas,
            renderer,
            clock,
            clock_text,
            icon_buffer,
            tasks: Vec::new(),
            tray_items: Vec::new(),
            rectangles,
            panel_icon,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.graphics.device, &self.config);
        self.clock
            .set_size(Some(width.saturating_sub(24) as f32), Some(height as f32));
        self.icon_buffer
            .set_size(Some(width as f32), Some(height as f32));
    }

    pub fn update_clock(&mut self) -> bool {
        let text = local_clock_text();
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

    pub fn set_tray_items(&mut self, items: Vec<PanelTrayItem>) {
        self.tray_items = items;
    }

    pub fn render(&mut self, launcher_hovered: bool, task_hovered: Option<usize>) {
        let mut custom_glyphs = vec![CustomGlyph {
            id: PANEL_ICON_GLYPH_ID,
            left: 12.0,
            top: 12.0,
            width: 32.0,
            height: 32.0,
            color: None,
            snap_to_physical_pixel: true,
            metadata: 0,
        }];
        custom_glyphs.extend(
            self.tasks
                .iter()
                .enumerate()
                .map(|(index, task)| CustomGlyph {
                    id: task.glyph_id,
                    left: (TASKS_LEFT + index as f64 * TASK_WIDTH + 8.0) as f32,
                    top: 12.0,
                    width: 32.0,
                    height: 32.0,
                    color: None,
                    snap_to_physical_pixel: true,
                    metadata: 0,
                }),
        );
        custom_glyphs.extend(self.tray_items.iter().enumerate().filter_map(|(index, _)| {
            Some(CustomGlyph {
                id: TRAY_GLYPH_BASE.checked_add(u16::try_from(index).ok()?)?,
                left: tray_left(self.config.width, self.tray_items.len(), index) as f32 + 4.0,
                top: 12.0,
                width: 32.0,
                height: 32.0,
                color: None,
                snap_to_physical_pixel: true,
                metadata: 0,
            })
        }));
        let mut rectangles = Vec::new();
        if launcher_hovered {
            rectangles.push(([4.0, 4.0, 52.0, 52.0], [0.12, 0.15, 0.21, 1.0]));
        }
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
            &self.graphics.queue,
            (self.config.width, self.config.height),
            &rectangles,
        );
        self.viewport.update(
            &self.graphics.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        self.renderer
            .prepare_with_custom(
                &self.graphics.device,
                &self.graphics.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [
                    TextArea {
                        buffer: &self.clock,
                        left: 0.0,
                        top: 6.0,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: TASKS_LEFT as i32,
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
                            left: 0,
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
                    let source = if request.id == PANEL_ICON_GLYPH_ID {
                        &self.panel_icon
                    } else if request.id >= TRAY_GLYPH_BASE {
                        &self
                            .tray_items
                            .get(usize::from(request.id - TRAY_GLYPH_BASE))?
                            .icon
                    } else {
                        &self
                            .tasks
                            .iter()
                            .find(|task| task.glyph_id == request.id)?
                            .icon
                    };
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
                self.surface.configure(&self.graphics.device, &self.config);
                return;
            }
            _ => return,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            self.graphics
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
        self.graphics.queue.submit([encoder.finish()]);
        self.graphics.queue.present(frame);
    }
}

pub fn task_at(position: winit::dpi::PhysicalPosition<f64>, task_count: usize) -> Option<usize> {
    if position.y < 0.0 || position.y >= 56.0 || position.x < TASKS_LEFT {
        return None;
    }
    let index = ((position.x - TASKS_LEFT) / TASK_WIDTH) as usize;
    (index < task_count).then_some(index)
}

pub fn task_menu_x(index: usize) -> i32 {
    (TASKS_LEFT + index as f64 * TASK_WIDTH) as i32
}

fn tray_left(panel_width: u32, count: usize, index: usize) -> f64 {
    f64::from(panel_width) - CLOCK_WIDTH - (count - index) as f64 * TRAY_WIDTH
}

pub fn tray_at(
    position: winit::dpi::PhysicalPosition<f64>,
    panel_width: u32,
    count: usize,
) -> Option<usize> {
    (0..count).find(|index| {
        let left = tray_left(panel_width, count, *index);
        position.x >= left
            && position.x < left + TRAY_WIDTH
            && position.y >= 0.0
            && position.y < 56.0
    })
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

fn tinted_panel_icon(mut icon: image::RgbaImage) -> image::RgbaImage {
    for pixel in icon.pixels_mut() {
        pixel.0[..3].copy_from_slice(&[238, 241, 248]);
    }
    icon
}

pub fn launcher_button_contains(position: winit::dpi::PhysicalPosition<f64>) -> bool {
    position.x >= 0.0
        && position.x < LAUNCHER_BUTTON_WIDTH
        && position.y >= 0.0
        && position.y < 56.0
}

fn local_clock_text() -> String {
    format!("{}\n{}", local_time_text(), local_short_date_text())
}

#[cfg(target_os = "windows")]
fn local_time_text() -> String {
    use windows::{
        Win32::Globalization::{GetTimeFormatEx, TIME_NOSECONDS},
        core::PCWSTR,
    };

    let mut time = [0_u16; 128];
    let written = unsafe {
        GetTimeFormatEx(
            PCWSTR::null(),
            TIME_NOSECONDS,
            None,
            PCWSTR::null(),
            Some(&mut time),
        )
    };
    if written > 1 {
        String::from_utf16_lossy(&time[..written as usize - 1])
    } else {
        jiff::Zoned::now().strftime("%H:%M").to_string()
    }
}

#[cfg(not(target_os = "windows"))]
fn local_time_text() -> String {
    jiff::Zoned::now().strftime("%H:%M").to_string()
}

#[cfg(target_os = "windows")]
fn local_short_date_text() -> String {
    use windows::{
        Win32::Globalization::{DATE_SHORTDATE, GetDateFormatEx},
        core::PCWSTR,
    };

    let mut date = [0_u16; 128];
    let written = unsafe {
        GetDateFormatEx(
            PCWSTR::null(),
            DATE_SHORTDATE,
            None,
            PCWSTR::null(),
            Some(&mut date),
            PCWSTR::null(),
        )
    };
    if written > 1 {
        String::from_utf16_lossy(&date[..written as usize - 1])
    } else {
        jiff::Zoned::now().strftime("%Y-%m-%d").to_string()
    }
}

#[cfg(not(target_os = "windows"))]
fn local_short_date_text() -> String {
    jiff::Zoned::now().strftime("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use winit::dpi::PhysicalPosition;

    use super::{
        launcher_button_contains, local_clock_text, local_short_date_text, local_time_text,
        task_at, tinted_panel_icon, tray_at,
    };

    #[test]
    fn local_clock_has_a_nonempty_single_line_time() {
        let text = local_time_text();
        assert!(!text.is_empty());
        assert!(!text.contains(['\r', '\n']));
    }

    #[test]
    fn local_clock_includes_the_platform_short_date() {
        let date = local_short_date_text();
        let clock = local_clock_text();
        assert!(!date.is_empty());
        assert_eq!(clock.lines().count(), 2);
        assert!(clock.ends_with(&date));
    }

    #[test]
    fn launcher_button_does_not_include_clock_area() {
        assert!(launcher_button_contains(PhysicalPosition::new(28.0, 28.0)));
        assert!(!launcher_button_contains(PhysicalPosition::new(56.0, 28.0)));
        assert!(!launcher_button_contains(PhysicalPosition::new(
            900.0, 28.0
        )));
    }

    #[test]
    fn task_hit_testing_starts_after_launcher_button() {
        assert_eq!(task_at(PhysicalPosition::new(64.0, 28.0), 2), Some(0));
        assert_eq!(task_at(PhysicalPosition::new(111.0, 28.0), 2), Some(0));
        assert_eq!(task_at(PhysicalPosition::new(112.0, 28.0), 2), Some(1));
        assert_eq!(task_at(PhysicalPosition::new(160.0, 28.0), 2), None);
    }

    #[test]
    fn panel_icon_tint_preserves_alpha_mask() {
        let source = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 127]));
        assert_eq!(
            tinted_panel_icon(source).get_pixel(0, 0).0,
            [238, 241, 248, 127]
        );
    }

    #[test]
    fn tray_hit_testing_uses_space_before_clock() {
        assert_eq!(
            tray_at(PhysicalPosition::new(1110.0, 28.0), 1280, 2),
            Some(0)
        );
        assert_eq!(
            tray_at(PhysicalPosition::new(1150.0, 28.0), 1280, 2),
            Some(1)
        );
        assert_eq!(tray_at(PhysicalPosition::new(1190.0, 28.0), 1280, 2), None);
    }
}
