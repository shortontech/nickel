use std::{
    cell::Cell,
    env, fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use image::RgbaImage;
use nickel_ui::{PaintCommand, Rect, TextAlign};

use crate::platform;

const TOOLBAR_HEIGHT: f32 = 70.0;
const PREVIEW_PADDING: f32 = 20.0;

pub struct ScreenshotTool {
    image: Option<Arc<RgbaImage>>,
    cursor: (f32, f32),
    drag_start: Option<(f32, f32)>,
    selection: Option<Rect>,
    confirmed: bool,
    last_click: Option<(Instant, f32, f32)>,
    status: String,
    error_visible: bool,
    capture_deadline: Option<Instant>,
    save_after_confirmation: bool,
    viewport: Cell<Option<(u32, u32)>>,
}

impl Default for ScreenshotTool {
    fn default() -> Self {
        Self {
            image: None,
            cursor: (0.0, 0.0),
            drag_start: None,
            selection: None,
            confirmed: false,
            last_click: None,
            status: instructions(),
            error_visible: false,
            capture_deadline: None,
            save_after_confirmation: false,
            viewport: Cell::new(None),
        }
    }
}

impl ScreenshotTool {
    pub fn request_capture(&mut self) {
        self.hide();
        self.save_after_confirmation = false;
        self.capture_deadline = Some(Instant::now() + Duration::from_millis(75));
    }

    pub fn request_capture_to_file(&mut self) {
        self.hide();
        self.save_after_confirmation = true;
        self.capture_deadline = Some(Instant::now() + Duration::from_millis(75));
    }

    pub fn capture_ready(&mut self) -> bool {
        if self
            .capture_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.capture_deadline = None;
            true
        } else {
            false
        }
    }

    pub fn show(&mut self, image: RgbaImage) {
        self.image = Some(Arc::new(image));
        self.status = instructions();
        self.error_visible = false;
    }

    pub fn show_error(&mut self, error: impl Into<String>) {
        self.hide();
        self.status = format!("SCREENSHOT FAILED · {} · ESC TO CLOSE", error.into());
        self.error_visible = true;
    }

    pub fn visible(&self) -> bool {
        self.image.is_some() || self.error_visible
    }

    pub fn hide(&mut self) {
        self.image = None;
        self.drag_start = None;
        self.selection = None;
        self.confirmed = false;
        self.last_click = None;
        self.error_visible = false;
        self.save_after_confirmation = false;
    }

    pub fn pointer_moved(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
        self.cursor = (x, y);
        let Some(start) = self.drag_start else {
            return false;
        };
        self.selection = Some(normalized(
            start,
            clamp_to_rect(self.cursor, self.image_rect(width, height)),
        ));
        true
    }

    pub fn pointer_pressed(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
        self.cursor = (x, y);
        if self.confirmed && self.cursor.1 <= TOOLBAR_HEIGHT {
            match self.cursor.0 {
                x if x < 170.0 => self.copy(width, height),
                x if x < 340.0 => self.save(width, height),
                x if x < 580.0 => self.temp(width, height),
                _ => self.hide(),
            }
            return true;
        }
        let image_rect = self.image_rect(width, height);
        if !contains(image_rect, self.cursor) {
            return false;
        }
        if self
            .selection
            .is_some_and(|selection| contains(selection, self.cursor))
        {
            if self.last_click.is_some_and(|(at, x, y)| {
                at.elapsed() <= Duration::from_millis(420)
                    && (x - self.cursor.0).abs() < 6.0
                    && (y - self.cursor.1).abs() < 6.0
            }) {
                self.confirmed = true;
                self.drag_start = None;
                self.last_click = None;
                if self.save_after_confirmation {
                    self.save(width, height);
                } else {
                    self.status = "SELECTION CONFIRMED".into();
                }
            } else {
                self.last_click = Some((Instant::now(), self.cursor.0, self.cursor.1));
            }
            return true;
        }
        self.confirmed = false;
        let cursor = clamp_to_rect(self.cursor, image_rect);
        self.drag_start = Some(cursor);
        self.selection = Some(normalized(cursor, cursor));
        self.last_click = Some((Instant::now(), self.cursor.0, self.cursor.1));
        true
    }

    pub fn pointer_released(&mut self) -> bool {
        let changed = self.drag_start.take().is_some();
        if changed {
            self.last_click = None;
        }
        if self
            .selection
            .is_some_and(|rect| rect.size.width < 3.0 || rect.size.height < 3.0)
        {
            self.selection = None;
        }
        changed
    }

    pub fn semantic_target(
        &self,
        action: nickel_session_protocol::ScreenshotTargetAction,
    ) -> Option<(i32, i32, nickel_session_protocol::PointerInteraction)> {
        use nickel_session_protocol::{PointerInteraction, ScreenshotTargetAction};

        let (width, height) = self.viewport.get()?;
        let point = match action {
            ScreenshotTargetAction::SelectionStart => {
                let preview = self.image_rect(width, height);
                (
                    preview.origin.x + preview.size.width * 0.25,
                    preview.origin.y + preview.size.height * 0.25,
                    PointerInteraction::LeftPress,
                )
            }
            ScreenshotTargetAction::SelectionEnd => {
                let preview = self.image_rect(width, height);
                (
                    preview.origin.x + preview.size.width * 0.75,
                    preview.origin.y + preview.size.height * 0.75,
                    PointerInteraction::LeftRelease,
                )
            }
            ScreenshotTargetAction::Confirm => {
                let selection = self.selection?;
                (
                    selection.origin.x + selection.size.width / 2.0,
                    selection.origin.y + selection.size.height / 2.0,
                    PointerInteraction::LeftDoubleClick,
                )
            }
            ScreenshotTargetAction::CopyImage if self.confirmed => {
                (85.0, TOOLBAR_HEIGHT / 2.0, PointerInteraction::LeftClick)
            }
            ScreenshotTargetAction::SaveImage if self.confirmed => {
                (255.0, TOOLBAR_HEIGHT / 2.0, PointerInteraction::LeftClick)
            }
            ScreenshotTargetAction::CopyTemporaryPath if self.confirmed => {
                (460.0, TOOLBAR_HEIGHT / 2.0, PointerInteraction::LeftClick)
            }
            ScreenshotTargetAction::Cancel if self.confirmed => {
                (650.0, TOOLBAR_HEIGHT / 2.0, PointerInteraction::LeftClick)
            }
            ScreenshotTargetAction::CopyImage
            | ScreenshotTargetAction::SaveImage
            | ScreenshotTargetAction::CopyTemporaryPath
            | ScreenshotTargetAction::Cancel => return None,
        };
        Some((point.0.round() as i32, point.1.round() as i32, point.2))
    }

    pub fn scene(
        &self,
        width: u32,
        height: u32,
        palette: nickel_core::theme::ThemePalette,
    ) -> Vec<PaintCommand> {
        self.viewport.set(Some((width, height)));
        let mut commands = vec![PaintCommand::Fill {
            rect: Rect::new(0.0, 0.0, width as f32, height as f32),
            color: palette.background,
        }];
        if let Some(image) = &self.image {
            commands.push(PaintCommand::Image {
                bounds: self.image_rect(width, height),
                id: 65_000,
                image: image.clone(),
                high_density: None,
            });
        }
        commands.push(PaintCommand::Fill {
            rect: Rect::new(0.0, 0.0, width as f32, TOOLBAR_HEIGHT),
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
            [
                "⧉ COPY IMAGE",
                "↓ SAVE IMAGE",
                "⌗ COPY TEMPORARY FILENAME",
                "× CANCEL",
            ]
        } else {
            ["", "", "", ""]
        };
        for (index, label) in labels.iter().enumerate() {
            let (x, item_width) = match index {
                0 => (0.0, 170.0),
                1 => (170.0, 170.0),
                2 => (340.0, 240.0),
                _ => (580.0, 140.0),
            };
            commands.push(PaintCommand::Text {
                bounds: Rect::new(x, 18.0, item_width, 34.0),
                text: (*label).into(),
                scale: 1.0,
                color: palette.text,
                align: TextAlign::Center,
                bold: false,
                wrap: false,
            });
        }
        commands.push(PaintCommand::Text {
            bounds: Rect::new(730.0, 18.0, (width as f32 - 740.0).max(10.0), 34.0),
            text: self.status.clone(),
            scale: 1.0,
            color: palette.muted,
            align: TextAlign::Start,
            bold: false,
            wrap: false,
        });
        commands
    }

    fn image_rect(&self, width: u32, height: u32) -> Rect {
        let Some(image) = &self.image else {
            return Rect::new(0.0, TOOLBAR_HEIGHT, 1.0, 1.0);
        };
        let available_width = (width as f32 - PREVIEW_PADDING * 2.0).max(1.0);
        let available_height = (height as f32 - TOOLBAR_HEIGHT - PREVIEW_PADDING * 2.0).max(1.0);
        let scale =
            (available_width / image.width() as f32).min(available_height / image.height() as f32);
        let image_width = image.width() as f32 * scale;
        let image_height = image.height() as f32 * scale;
        Rect::new(
            (width as f32 - image_width) / 2.0,
            TOOLBAR_HEIGHT + PREVIEW_PADDING + (available_height - image_height) / 2.0,
            image_width,
            image_height,
        )
    }

    fn cropped(&self, width: u32, height: u32) -> Option<RgbaImage> {
        let image = self.image.as_ref()?;
        let selection = self.selection?;
        let preview = self.image_rect(width, height);
        let sx = image.width() as f32 / preview.size.width.max(1.0);
        let sy = image.height() as f32 / preview.size.height.max(1.0);
        let x = ((selection.origin.x - preview.origin.x) * sx)
            .round()
            .max(0.0) as u32;
        let y = ((selection.origin.y - preview.origin.y) * sy)
            .round()
            .max(0.0) as u32;
        let crop_width = (selection.size.width * sx).round().max(1.0) as u32;
        let crop_height = (selection.size.height * sy).round().max(1.0) as u32;
        let x = x.min(image.width() - 1);
        let y = y.min(image.height() - 1);
        Some(
            image::imageops::crop_imm(
                image.as_ref(),
                x,
                y,
                crop_width.min(image.width() - x),
                crop_height.min(image.height() - y),
            )
            .to_image(),
        )
    }

    fn copy(&mut self, width: u32, height: u32) {
        self.status = if self
            .cropped(width, height)
            .is_some_and(|image| platform::copy_image_to_clipboard(&image).is_ok())
        {
            "IMAGE COPIED".into()
        } else {
            "COPY FAILED".into()
        };
    }

    fn temp(&mut self, width: u32, height: u32) {
        self.status = match self
            .cropped(width, height)
            .and_then(|image| platform::copy_temp_image_path(&image).ok())
        {
            Some(path) => format!("TEMP PATH COPIED · {}", path.display()),
            None => "TEMP SAVE FAILED".into(),
        };
    }

    fn save(&mut self, width: u32, height: u32) {
        let Some(image) = self.cropped(width, height) else {
            return;
        };
        let Some(home) = env::var_os("USERPROFILE")
            .or_else(|| env::var_os("HOME"))
            .map(PathBuf::from)
        else {
            self.status = "SAVE FAILED · HOME DIRECTORY IS UNKNOWN".into();
            return;
        };
        let directory = home.join("Pictures").join("Screenshots");
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
}

fn instructions() -> String {
    "DRAG CORNER TO CORNER · DOUBLE-CLICK TO CONFIRM · ESC TO CANCEL".into()
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

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::{ScreenshotTool, normalized};

    #[test]
    fn normalization_accepts_reverse_corner_drag() {
        let rect = normalized((30.0, 40.0), (10.0, 15.0));
        assert_eq!(rect.origin.x, 10.0);
        assert_eq!(rect.origin.y, 15.0);
        assert_eq!(rect.size.width, 20.0);
        assert_eq!(rect.size.height, 25.0);
    }

    #[test]
    fn crop_maps_preview_selection_back_to_source_pixels() {
        let mut image = RgbaImage::new(4, 2);
        for y in 0..2 {
            for x in 0..4 {
                image.put_pixel(x, y, Rgba([x as u8, y as u8, 0, 255]));
            }
        }
        let mut tool = ScreenshotTool::default();
        tool.show(image);
        let preview = tool.image_rect(1200, 760);
        tool.selection = Some(nickel_ui::Rect::new(
            preview.origin.x + preview.size.width / 4.0,
            preview.origin.y,
            preview.size.width / 2.0,
            preview.size.height,
        ));

        let crop = tool.cropped(1200, 760).expect("selection is cropable");
        assert_eq!(crop.dimensions(), (2, 2));
        assert_eq!(crop.get_pixel(0, 0).0, [1, 0, 0, 255]);
        assert_eq!(crop.get_pixel(1, 1).0, [2, 1, 0, 255]);
    }

    #[test]
    fn capture_failure_is_visible_and_escape_can_clear_it() {
        let mut tool = ScreenshotTool::default();
        tool.show_error("permission denied");
        assert!(tool.visible());
        assert!(tool.status.contains("permission denied"));
        assert!(
            !tool
                .scene(
                    800,
                    600,
                    nickel_core::theme::ThemePalette::from_appearance(Default::default()),
                )
                .is_empty()
        );
        tool.hide();
        assert!(!tool.visible());
    }

    #[test]
    fn interactive_file_capture_is_an_explicit_mode() {
        let mut tool = ScreenshotTool::default();
        tool.request_capture_to_file();
        assert!(tool.save_after_confirmation);
        assert!(tool.capture_deadline.is_some());
        tool.request_capture();
        assert!(!tool.save_after_confirmation);
    }

    #[test]
    fn semantic_selection_targets_drive_production_hit_testing() {
        use nickel_session_protocol::{PointerInteraction, ScreenshotTargetAction};

        let mut tool = ScreenshotTool::default();
        tool.show(RgbaImage::new(400, 200));
        let _ = tool.scene(
            800,
            600,
            nickel_core::theme::ThemePalette::from_appearance(Default::default()),
        );
        let (start_x, start_y, start_edge) = tool
            .semantic_target(ScreenshotTargetAction::SelectionStart)
            .unwrap();
        assert_eq!(start_edge, PointerInteraction::LeftPress);
        assert!(tool.pointer_pressed(start_x as f32, start_y as f32, 800, 600));
        let (end_x, end_y, end_edge) = tool
            .semantic_target(ScreenshotTargetAction::SelectionEnd)
            .unwrap();
        assert_eq!(end_edge, PointerInteraction::LeftRelease);
        assert!(tool.pointer_moved(end_x as f32, end_y as f32, 800, 600));
        assert!(tool.pointer_released());
        let (confirm_x, confirm_y, confirm_edge) = tool
            .semantic_target(ScreenshotTargetAction::Confirm)
            .unwrap();
        assert_eq!(confirm_edge, PointerInteraction::LeftDoubleClick);
        assert!(tool.pointer_pressed(confirm_x as f32, confirm_y as f32, 800, 600));
        assert!(tool.pointer_pressed(confirm_x as f32, confirm_y as f32, 800, 600));
        assert!(tool.confirmed);
        assert!(
            tool.semantic_target(ScreenshotTargetAction::CopyImage)
                .is_some()
        );
    }
}
