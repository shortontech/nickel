use std::{
    env, fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use image::RgbaImage;
use nickel_components::{ComponentGpu, PaintCommand, Rect, TextAlign};
use nickel_core::theme::{Appearance, ThemePalette};
use winit::{dpi::PhysicalPosition, window::Window};

use crate::platform;

const TOOLBAR_HEIGHT: f32 = 70.0;
const PREVIEW_PADDING: f32 = 20.0;

pub struct ScreenshotTool {
    pub window: Arc<Window>,
    gpu: ComponentGpu,
    image: Option<Arc<RgbaImage>>,
    cursor: (f32, f32),
    drag_start: Option<(f32, f32)>,
    selection: Option<Rect>,
    confirmed: bool,
    last_click: Option<(Instant, f32, f32)>,
    status: String,
    appearance: Appearance,
}

impl ScreenshotTool {
    pub fn new(window: Arc<Window>, appearance: Appearance) -> Result<Self, String> {
        let size = window.inner_size();
        Ok(Self {
            gpu: ComponentGpu::new(window.clone(), size.width, size.height)?,
            window,
            image: None,
            cursor: (0.0, 0.0),
            drag_start: None,
            selection: None,
            confirmed: false,
            last_click: None,
            status: "DRAG CORNER TO CORNER · DOUBLE-CLICK TO CONFIRM · ESC TO CANCEL".into(),
            appearance,
        })
    }

    pub fn show(&mut self, image: RgbaImage) {
        self.image = Some(Arc::new(image));
        self.drag_start = None;
        self.selection = None;
        self.confirmed = false;
        self.status = "DRAG CORNER TO CORNER · DOUBLE-CLICK TO CONFIRM · ESC TO CANCEL".into();
        self.window.set_visible(true);
        self.window.focus_window();
        self.window.request_redraw();
    }

    pub fn hide(&mut self) {
        self.window.set_visible(false);
        self.image = None;
        self.drag_start = None;
        self.selection = None;
        self.confirmed = false;
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.gpu.resize(width, height);
    }

    pub fn cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        self.cursor = (position.x as f32, position.y as f32);
        if let Some(start) = self.drag_start {
            let current = clamp_to_rect(self.cursor, self.image_rect());
            self.selection = Some(normalized(start, current));
            tracing::debug!(
                start_x = start.0,
                start_y = start.1,
                x = self.cursor.0,
                y = self.cursor.1,
                "screenshot selection dragged"
            );
            self.window.request_redraw();
        }
    }

    pub fn pointer_pressed(&mut self) -> bool {
        tracing::info!(
            x = self.cursor.0,
            y = self.cursor.1,
            confirmed = self.confirmed,
            "screenshot pointer pressed"
        );
        let y = self.cursor.1;
        if self.confirmed && y <= TOOLBAR_HEIGHT {
            let x = self.cursor.0;
            if x < 170.0 {
                self.copy();
                return true;
            } else if x < 340.0 {
                self.save();
                return true;
            } else if x < 580.0 {
                self.temp();
                return true;
            } else if x < 720.0 {
                return true;
            }
            self.window.request_redraw();
            return false;
        }
        let image_rect = self.image_rect();
        if !contains(image_rect, self.cursor) {
            return false;
        }
        if let Some(selection) = self.selection
            && contains(selection, self.cursor)
        {
            if self.last_click.is_some_and(|(at, x, y)| {
                at.elapsed() <= Duration::from_millis(420)
                    && (x - self.cursor.0).abs() < 6.0
                    && (y - self.cursor.1).abs() < 6.0
            }) {
                self.confirmed = true;
                self.drag_start = None;
                self.last_click = None;
                self.status = "SELECTION CONFIRMED".into();
                self.window.request_redraw();
            } else {
                // The first click of a double-click arms confirmation without replacing the
                // selection. A second nearby click confirms it.
                self.last_click = Some((Instant::now(), self.cursor.0, self.cursor.1));
            }
            return false;
        }
        self.confirmed = false;
        let cursor = clamp_to_rect(self.cursor, image_rect);
        self.drag_start = Some(cursor);
        self.selection = Some(normalized(cursor, cursor));
        self.last_click = Some((Instant::now(), self.cursor.0, self.cursor.1));
        false
    }

    pub fn pointer_released(&mut self) {
        let was_dragging = self.drag_start.is_some();
        self.drag_start = None;
        if was_dragging {
            self.last_click = None;
        }
        if self
            .selection
            .is_some_and(|rect| rect.size.width < 3.0 || rect.size.height < 3.0)
        {
            self.selection = None;
        }
        tracing::info!(selection = ?self.selection, "screenshot pointer released");
        self.window.request_redraw();
    }

    fn cropped(&self) -> Option<RgbaImage> {
        let image = self.image.as_ref()?;
        let selection = self.selection?;
        let preview = self.image_rect();
        let sx = image.width() as f32 / preview.size.width.max(1.0);
        let sy = image.height() as f32 / preview.size.height.max(1.0);
        let x = ((selection.origin.x - preview.origin.x) * sx)
            .round()
            .max(0.0) as u32;
        let y = ((selection.origin.y - preview.origin.y) * sy)
            .round()
            .max(0.0) as u32;
        let width = (selection.size.width * sx).round().max(1.0) as u32;
        let height = (selection.size.height * sy).round().max(1.0) as u32;
        Some(
            image::imageops::crop_imm(
                image.as_ref(),
                x.min(image.width() - 1),
                y.min(image.height() - 1),
                width.min(image.width() - x.min(image.width() - 1)),
                height.min(image.height() - y.min(image.height() - 1)),
            )
            .to_image(),
        )
    }

    fn image_rect(&self) -> Rect {
        let size = self.window.inner_size();
        let Some(image) = &self.image else {
            return Rect::new(0.0, TOOLBAR_HEIGHT, 1.0, 1.0);
        };
        let available_width = (size.width as f32 - PREVIEW_PADDING * 2.0).max(1.0);
        let available_height =
            (size.height as f32 - TOOLBAR_HEIGHT - PREVIEW_PADDING * 2.0).max(1.0);
        let scale =
            (available_width / image.width() as f32).min(available_height / image.height() as f32);
        let width = image.width() as f32 * scale;
        let height = image.height() as f32 * scale;
        Rect::new(
            (size.width as f32 - width) / 2.0,
            TOOLBAR_HEIGHT + PREVIEW_PADDING + (available_height - height) / 2.0,
            width,
            height,
        )
    }

    fn copy(&mut self) {
        self.status = match self.cropped().and_then(|image| {
            platform::copy_image_to_clipboard(&image)
                .ok()
                .map(|_| image)
        }) {
            Some(_) => "IMAGE COPIED".into(),
            None => "COPY FAILED".into(),
        };
    }

    fn temp(&mut self) {
        self.status = match self
            .cropped()
            .and_then(|image| platform::copy_temp_image_path(&image).ok())
        {
            Some(path) => format!("TEMP PATH COPIED · {}", path.display()),
            None => "TEMP SAVE FAILED".into(),
        };
    }

    fn save(&mut self) {
        let Some(image) = self.cropped() else { return };
        let directory = env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir)
            .join("Pictures")
            .join("Screenshots");
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let path = directory.join(format!("Nickel Screenshot {stamp}.png"));
        self.status = if fs::create_dir_all(&directory)
            .and_then(|_| image.save(&path).map_err(std::io::Error::other))
            .is_ok()
        {
            format!("SAVED · {}", path.display())
        } else {
            "SAVE FAILED".into()
        };
    }

    pub fn render(&mut self) {
        let Some(image) = &self.image else { return };
        let size = self.window.inner_size();
        let palette = ThemePalette::from_appearance(self.appearance);
        let mut commands = vec![PaintCommand::Image {
            bounds: self.image_rect(),
            id: 65000,
            image: image.clone(),
        }];
        commands.push(PaintCommand::Fill {
            rect: Rect::new(0.0, 0.0, size.width as f32, TOOLBAR_HEIGHT),
            color: palette.panel,
        });
        if let Some(rect) = self.selection {
            commands.push(PaintCommand::OverlayStroke {
                rect,
                color: palette.accent,
                width: if self.confirmed { 5.0 } else { 3.0 },
            });
        }
        let labels = if self.confirmed {
            ["⧉ COPY", "↓ SAVE", "⌗ TEMP PATH", "× CLOSE"]
        } else {
            ["", "", "", ""]
        };
        let _ = labels;
        let labels = if self.confirmed {
            [
                "\u{29c9} COPY IMAGE",
                "\u{2193} SAVE IMAGE",
                "\u{2317} COPY TEMPORARY FILENAME",
                "\u{00d7} CANCEL",
            ]
        } else {
            ["", "", "", ""]
        };
        for (index, label) in labels.iter().enumerate() {
            let (x, w) = match index {
                0 => (0.0, 170.0),
                1 => (170.0, 170.0),
                2 => (340.0, 240.0),
                _ => (580.0, 140.0),
            };
            commands.push(PaintCommand::Text {
                bounds: Rect::new(x, 18.0, w, 34.0),
                text: (*label).into(),
                scale: 1.0,
                color: palette.text,
                align: TextAlign::Center,
                bold: false,
            });
        }
        commands.push(PaintCommand::Text {
            bounds: Rect::new(730.0, 18.0, (size.width as f32 - 740.0).max(10.0), 34.0),
            text: self.status.clone(),
            scale: 1.0,
            color: palette.muted,
            align: TextAlign::Start,
            bold: false,
        });
        if let Err(error) = self.gpu.render(&commands) {
            tracing::warn!(%error, "failed to render screenshot crop surface");
        }
    }
}

fn normalized(a: (f32, f32), b: (f32, f32)) -> Rect {
    Rect::new(
        a.0.min(b.0),
        a.1.min(b.1),
        (a.0 - b.0).abs(),
        (a.1 - b.1).abs(),
    )
}

fn contains(rect: Rect, point: (f32, f32)) -> bool {
    point.0 >= rect.origin.x
        && point.0 <= rect.origin.x + rect.size.width
        && point.1 >= rect.origin.y
        && point.1 <= rect.origin.y + rect.size.height
}

fn clamp_to_rect(point: (f32, f32), rect: Rect) -> (f32, f32) {
    (
        point
            .0
            .clamp(rect.origin.x, rect.origin.x + rect.size.width),
        point
            .1
            .clamp(rect.origin.y, rect.origin.y + rect.size.height),
    )
}
