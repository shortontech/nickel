use cosmic_text::{
    Align, Attrs, Buffer, Color as TextColor, Family, FontSystem, Metrics, Shaping, SwashCache,
    Weight,
};
use sdl3::{pixels::PixelFormat, rect::Rect as SdlRect, render::BlendMode, surface::SurfaceRef};

use crate::{Color, GradientAxis, PaintCommand, Rect, TextAlign};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pixel {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Pixel {
    const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);

    const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

/// Physical pixels changed by the latest component frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DamageRegion {
    pub rects: Vec<Rect>,
}

impl DamageRegion {
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }
}

/// SDL presentation backend for the platform-neutral component tree.
///
/// Component layout and hit testing remain in `UiTree`; this type only rasterizes
/// its paint list. The retained pixel buffer makes an eventual SDL GPU upload or
/// Wayland damage submission independent of component semantics.
pub struct SdlComponentRenderer {
    width: u32,
    height: u32,
    scale: f32,
    pixels: Vec<Pixel>,
    previous_commands: Vec<PaintCommand>,
    font_system: FontSystem,
    swash_cache: SwashCache,
}

/// Transitional name retained while callers move from the old wgpu backend.
pub type ComponentGpu = SdlComponentRenderer;

impl SdlComponentRenderer {
    pub fn new(width: u32, height: u32, scale: f32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        Self {
            width,
            height,
            scale: scale.max(0.25),
            pixels: vec![Pixel::TRANSPARENT; (width * height) as usize],
            previous_commands: Vec::new(),
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        }
    }

    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        let width = width.max(1);
        let height = height.max(1);
        self.scale = scale.max(0.25);
        if (width, height) != (self.width, self.height) {
            self.width = width;
            self.height = height;
            self.pixels
                .resize((self.width * self.height) as usize, Pixel::TRANSPARENT);
            self.previous_commands.clear();
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn pixels(&self) -> &[Pixel] {
        &self.pixels
    }

    pub fn invalidate(&mut self) {
        self.previous_commands.clear();
    }

    /// Rasterize a component display list and return its conservative damage.
    pub fn render(&mut self, commands: &[PaintCommand]) -> DamageRegion {
        let damage = self.damage(commands);
        if damage.is_empty() {
            return damage;
        }

        let full = physical_rect(
            Rect::new(0.0, 0.0, self.width as f32, self.height as f32),
            1.0,
        );
        let repaint = damage
            .rects
            .iter()
            .copied()
            .reduce(union_rect)
            .and_then(|rect| intersection(full, rect))
            .unwrap_or(full);
        self.clear(repaint);
        let mut clips = vec![repaint];
        for command in commands {
            self.draw_command(command, &mut clips);
        }
        self.previous_commands.clear();
        self.previous_commands.extend_from_slice(commands);
        damage
    }

    fn clear(&mut self, rect: Rect) {
        let left = rect.origin.x.floor().max(0.0) as u32;
        let top = rect.origin.y.floor().max(0.0) as u32;
        let right = (rect.origin.x + rect.size.width)
            .ceil()
            .clamp(0.0, self.width as f32) as u32;
        let bottom = (rect.origin.y + rect.size.height)
            .ceil()
            .clamp(0.0, self.height as f32) as u32;
        for y in top..bottom {
            let start = (y * self.width + left) as usize;
            let end = (y * self.width + right) as usize;
            self.pixels[start..end].fill(Pixel::TRANSPARENT);
        }
    }

    /// Rasterize and copy the frame into an SDL window surface.
    pub fn present(
        &mut self,
        surface: &mut SurfaceRef,
        commands: &[PaintCommand],
    ) -> Result<DamageRegion, String> {
        let damage = self.render(commands);
        if damage.is_empty() {
            return Ok(damage);
        }
        let mut upload =
            sdl3::surface::Surface::new(self.width, self.height, PixelFormat::ABGR8888)
                .map_err(|error| error.to_string())?;
        upload
            .set_blend_mode(BlendMode::None)
            .map_err(|error| error.to_string())?;
        upload.with_lock_mut(|bytes| {
            for (color, pixel) in self.pixels.iter().zip(bytes.chunks_exact_mut(4)) {
                pixel.copy_from_slice(&[color.r, color.g, color.b, color.a]);
            }
        });
        upload
            .blit(None, surface, SdlRect::new(0, 0, self.width, self.height))
            .map_err(|error| error.to_string())?;
        Ok(damage)
    }

    fn damage(&self, commands: &[PaintCommand]) -> DamageRegion {
        if self.previous_commands.is_empty() {
            return DamageRegion {
                rects: vec![Rect::new(0.0, 0.0, self.width as f32, self.height as f32)],
            };
        }
        let mut union = None;
        let count = commands.len().max(self.previous_commands.len());
        for index in 0..count {
            let old = self.previous_commands.get(index);
            let new = commands.get(index);
            if old != new {
                for command in [old, new].into_iter().flatten() {
                    if let Some(rect) = command_bounds(command) {
                        let rect = physical_rect(rect, self.scale);
                        union = Some(match union {
                            Some(current) => union_rect(current, rect),
                            None => rect,
                        });
                    }
                }
            }
        }
        if union.is_none() && commands != self.previous_commands {
            union = Some(Rect::new(0.0, 0.0, self.width as f32, self.height as f32));
        }
        DamageRegion {
            rects: union.into_iter().collect(),
        }
    }

    fn draw_command(&mut self, command: &PaintCommand, clips: &mut Vec<Rect>) {
        let clip = *clips.last().expect("clip stack always has the surface");
        match command {
            PaintCommand::Fill { rect, color } | PaintCommand::OverlayFill { rect, color } => {
                self.fill_round(physical_rect(*rect, self.scale), 0.0, 0b1111, *color, clip);
            }
            PaintCommand::TopRoundedFill {
                rect,
                color,
                radius,
            } => self.fill_round(
                physical_rect(*rect, self.scale),
                radius * self.scale,
                0b0011,
                *color,
                clip,
            ),
            PaintCommand::RoundedFill {
                rect,
                color,
                radius,
            } => self.fill_round(
                physical_rect(*rect, self.scale),
                radius * self.scale,
                0b1111,
                *color,
                clip,
            ),
            PaintCommand::Gradient { rect, gradient } => {
                self.fill_gradient(physical_rect(*rect, self.scale), *gradient, clip);
            }
            PaintCommand::Stroke { rect, color, width }
            | PaintCommand::OverlayStroke { rect, color, width } => {
                self.stroke(
                    physical_rect(*rect, self.scale),
                    width * self.scale,
                    *color,
                    clip,
                );
            }
            PaintCommand::Text { .. } => self.text(command, clip),
            PaintCommand::Image { bounds, image, .. } => {
                self.image(physical_rect(*bounds, self.scale), image, clip);
            }
            PaintCommand::PushClip(rect) => {
                let rect = physical_rect(*rect, self.scale);
                clips.push(intersection(clip, rect).unwrap_or_default());
            }
            PaintCommand::PopClip => {
                if clips.len() > 1 {
                    clips.pop();
                }
            }
        }
    }

    fn fill_round(&mut self, rect: Rect, radius: f32, corners: u8, color: Color, clip: Rect) {
        let Some(bounds) = intersection(rect, clip) else {
            return;
        };
        let radius = radius
            .max(0.0)
            .min(rect.size.width / 2.0)
            .min(rect.size.height / 2.0);
        self.for_pixels(bounds, |renderer, x, y| {
            let left = x as f32 + 0.5 - rect.origin.x;
            let top = y as f32 + 0.5 - rect.origin.y;
            let right = rect.size.width - left;
            let bottom = rect.size.height - top;
            let rounded = ((corners & 0b0001 != 0 && left < radius && top < radius)
                .then_some((left - radius, top - radius)))
            .or_else(|| {
                (corners & 0b0010 != 0 && right < radius && top < radius)
                    .then_some((right - radius, top - radius))
            })
            .or_else(|| {
                (corners & 0b0100 != 0 && right < radius && bottom < radius)
                    .then_some((right - radius, bottom - radius))
            })
            .or_else(|| {
                (corners & 0b1000 != 0 && left < radius && bottom < radius)
                    .then_some((left - radius, bottom - radius))
            });
            if rounded.is_none_or(|(dx, dy)| dx * dx + dy * dy <= radius * radius) {
                renderer.blend(x, y, pixel(color));
            }
        });
    }

    fn fill_gradient(&mut self, rect: Rect, gradient: crate::LinearGradient, clip: Rect) {
        let Some(bounds) = intersection(rect, clip) else {
            return;
        };
        self.for_pixels(bounds, |renderer, x, y| {
            let progress = match gradient.axis {
                GradientAxis::Horizontal => (x as f32 + 0.5 - rect.origin.x) / rect.size.width,
                GradientAxis::Vertical => (y as f32 + 0.5 - rect.origin.y) / rect.size.height,
            }
            .clamp(0.0, 1.0);
            renderer.blend(x, y, lerp_color(gradient.start, gradient.end, progress));
        });
    }

    fn stroke(&mut self, rect: Rect, width: f32, color: Color, clip: Rect) {
        let width = width.max(1.0);
        for edge in [
            Rect::new(rect.origin.x, rect.origin.y, rect.size.width, width),
            Rect::new(
                rect.origin.x,
                rect.origin.y + rect.size.height - width,
                rect.size.width,
                width,
            ),
            Rect::new(rect.origin.x, rect.origin.y, width, rect.size.height),
            Rect::new(
                rect.origin.x + rect.size.width - width,
                rect.origin.y,
                width,
                rect.size.height,
            ),
        ] {
            self.fill_round(edge, 0.0, 0, color, clip);
        }
    }

    fn text(&mut self, command: &PaintCommand, clip: Rect) {
        let PaintCommand::Text {
            bounds,
            text,
            scale,
            color,
            align,
            bold,
        } = command
        else {
            return;
        };
        let font_size = text_size(*scale) * self.scale;
        let physical = physical_rect(*bounds, self.scale);
        let mut buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(font_size, font_size * 1.3),
        );
        buffer.set_size(
            Some(physical.size.width.max(1.0)),
            Some(physical.size.height.max(font_size * 1.4)),
        );
        let mut attrs = Attrs::new().family(Family::SansSerif);
        if *bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        for line in &mut buffer.lines {
            line.set_align(Some(match align {
                TextAlign::Start => Align::Left,
                TextAlign::Center => Align::Center,
                TextAlign::End => Align::Right,
            }));
        }
        buffer.shape_until_scroll(&mut self.font_system, false);
        let origin = physical.origin;
        let pixel = pixel(*color);
        let text_color = TextColor::rgba(pixel.r, pixel.g, pixel.b, pixel.a);
        let mut glyph_pixels = Vec::new();
        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            text_color,
            |x, y, width, height, glyph_color| {
                glyph_pixels.push((x, y, width, height, glyph_color));
            },
        );
        for (x, y, width, height, glyph_color) in glyph_pixels {
            let glyph = Rect::new(
                origin.x + x as f32,
                origin.y + y as f32,
                width as f32,
                height as f32,
            );
            let Some(glyph) = intersection(glyph, clip) else {
                continue;
            };
            self.for_pixels(glyph, |renderer, px, py| {
                renderer.blend(
                    px,
                    py,
                    Pixel::rgba(
                        glyph_color.r(),
                        glyph_color.g(),
                        glyph_color.b(),
                        glyph_color.a(),
                    ),
                );
            });
        }
    }

    fn image(&mut self, rect: Rect, image: &image::RgbaImage, clip: Rect) {
        if image.width() == 0 || image.height() == 0 {
            return;
        }
        let scale =
            (rect.size.width / image.width() as f32).min(rect.size.height / image.height() as f32);
        let width = image.width() as f32 * scale;
        let height = image.height() as f32 * scale;
        let rect = Rect::new(
            rect.origin.x + (rect.size.width - width) * 0.5,
            rect.origin.y + (rect.size.height - height) * 0.5,
            width,
            height,
        );
        let Some(bounds) = intersection(rect, clip) else {
            return;
        };
        self.for_pixels(bounds, |renderer, x, y| {
            let source_x =
                (((x as f32 + 0.5 - rect.origin.x) / rect.size.width) * image.width() as f32)
                    .floor()
                    .clamp(0.0, image.width().saturating_sub(1) as f32) as u32;
            let source_y =
                (((y as f32 + 0.5 - rect.origin.y) / rect.size.height) * image.height() as f32)
                    .floor()
                    .clamp(0.0, image.height().saturating_sub(1) as f32) as u32;
            let pixel = image.get_pixel(source_x, source_y).0;
            renderer.blend(
                px(x),
                px(y),
                Pixel::rgba(pixel[0], pixel[1], pixel[2], pixel[3]),
            );
        });
    }

    fn for_pixels(&mut self, rect: Rect, mut draw: impl FnMut(&mut Self, u32, u32)) {
        let x_start = rect.origin.x.floor().max(0.0) as u32;
        let y_start = rect.origin.y.floor().max(0.0) as u32;
        let x_end = (rect.origin.x + rect.size.width)
            .ceil()
            .min(self.width as f32) as u32;
        let y_end = (rect.origin.y + rect.size.height)
            .ceil()
            .min(self.height as f32) as u32;
        for y in y_start..y_end {
            for x in x_start..x_end {
                draw(self, x, y);
            }
        }
    }

    fn blend(&mut self, x: u32, y: u32, source: Pixel) {
        let target = &mut self.pixels[(y * self.width + x) as usize];
        let alpha = u16::from(source.a);
        let inverse = 255 - alpha;
        target.r = ((u16::from(source.r) * alpha + u16::from(target.r) * inverse) / 255) as u8;
        target.g = ((u16::from(source.g) * alpha + u16::from(target.g) * inverse) / 255) as u8;
        target.b = ((u16::from(source.b) * alpha + u16::from(target.b) * inverse) / 255) as u8;
        target.a = (alpha + u16::from(target.a) * inverse / 255).min(255) as u8;
    }
}

fn command_bounds(command: &PaintCommand) -> Option<Rect> {
    match command {
        PaintCommand::Fill { rect, .. }
        | PaintCommand::TopRoundedFill { rect, .. }
        | PaintCommand::RoundedFill { rect, .. }
        | PaintCommand::Gradient { rect, .. }
        | PaintCommand::Stroke { rect, .. }
        | PaintCommand::OverlayFill { rect, .. }
        | PaintCommand::OverlayStroke { rect, .. } => Some(*rect),
        PaintCommand::Text { bounds, .. } | PaintCommand::Image { bounds, .. } => Some(*bounds),
        PaintCommand::PushClip(_) | PaintCommand::PopClip => None,
    }
}

fn physical_rect(rect: Rect, scale: f32) -> Rect {
    Rect::new(
        rect.origin.x * scale,
        rect.origin.y * scale,
        rect.size.width * scale,
        rect.size.height * scale,
    )
}

fn intersection(left: Rect, right: Rect) -> Option<Rect> {
    let x = left.origin.x.max(right.origin.x);
    let y = left.origin.y.max(right.origin.y);
    let right_edge = (left.origin.x + left.size.width).min(right.origin.x + right.size.width);
    let bottom_edge = (left.origin.y + left.size.height).min(right.origin.y + right.size.height);
    (right_edge > x && bottom_edge > y).then(|| Rect::new(x, y, right_edge - x, bottom_edge - y))
}

fn union_rect(left: Rect, right: Rect) -> Rect {
    let x = left.origin.x.min(right.origin.x);
    let y = left.origin.y.min(right.origin.y);
    let right_edge = (left.origin.x + left.size.width).max(right.origin.x + right.size.width);
    let bottom_edge = (left.origin.y + left.size.height).max(right.origin.y + right.size.height);
    Rect::new(x, y, right_edge - x, bottom_edge - y)
}

fn lerp_color(start: Color, end: Color, progress: f32) -> Pixel {
    let start = pixel(start);
    let end = pixel(end);
    let channel =
        |start: u8, end: u8| (start as f32 + (end as f32 - start as f32) * progress).round() as u8;
    Pixel::rgba(
        channel(start.r, end.r),
        channel(start.g, end.g),
        channel(start.b, end.b),
        channel(start.a, end.a),
    )
}

fn pixel(color: Color) -> Pixel {
    let encoded_alpha = ((color >> 24) & 0xff) as u8;
    Pixel::rgba(
        ((color >> 16) & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        (color & 0xff) as u8,
        if color <= 0x00ff_ffff {
            255
        } else {
            encoded_alpha
        },
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

fn px(value: u32) -> u32 {
    value
}
