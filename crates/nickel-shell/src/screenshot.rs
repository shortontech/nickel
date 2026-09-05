use std::{
    env, fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use image::RgbaImage;
#[cfg(any(target_os = "linux", test))]
use nickel_ui::ActionKind;
use nickel_ui::backend::PaintCommand;
use nickel_ui::{
    Align, Application, Button, ButtonPresentation, Column, Completion, CompletionFailure,
    CompletionFailureKind, Container, EffectEvidence, FrameOverlay, HostBatch, HostEvent, Image,
    ImageFit, Insets, Point, Rect, Row, SemanticTheme, Spacer, Text, Tone, UiEvent, UiHost,
    ViewContext,
};

use crate::platform;

const TOOLBAR_HEIGHT: f32 = 70.0;
const PREVIEW_PADDING: f32 = 20.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolbarAction {
    Copy,
    Save,
    TemporaryPath,
    Cancel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScreenshotMessage {
    Toolbar(ToolbarAction),
}

#[derive(Clone, Debug)]
enum ScreenshotCompletion {
    PointerMoved {
        x: f32,
        y: f32,
    },
    PointerPressed {
        x: f32,
        y: f32,
    },
    PointerReleased,
    #[cfg(any(target_os = "linux", test))]
    Semantic {
        action: nickel_session_protocol::ScreenshotTargetAction,
    },
    Platform {
        effect: ScreenshotEffect,
        result: Result<String, String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScreenshotEffect {
    Copy,
    Save,
    TemporaryPath,
    Cancel,
}

pub struct ScreenshotApp {
    image: Option<Arc<RgbaImage>>,
    cursor: (f32, f32),
    drag_start: Option<(f32, f32)>,
    selection: Option<Rect>,
    confirmed: bool,
    last_click: Option<(Instant, f32, f32)>,
    status: String,
    error_visible: bool,
    save_after_confirmation: bool,
    palette: nickel_core::theme::ThemePalette,
    viewport: (u32, u32),
    effects: Vec<ScreenshotEffect>,
    effect_evidence: Vec<EffectEvidence>,
    dirty: bool,
}

impl ScreenshotApp {
    fn new(width: u32, height: u32) -> Self {
        Self {
            image: None,
            cursor: (0.0, 0.0),
            drag_start: None,
            selection: None,
            confirmed: false,
            last_click: None,
            status: instructions(),
            error_visible: false,
            save_after_confirmation: false,
            palette: nickel_core::theme::ThemePalette::from_appearance(Default::default()),
            viewport: (width, height),
            effects: Vec::new(),
            effect_evidence: Vec::new(),
            dirty: false,
        }
    }

    #[cfg(any(test, feature = "workbench-fixtures"))]
    #[allow(dead_code)] // Binary and fixture library compile this shared module separately.
    pub fn fixture(width: u32, height: u32, state: &str) -> Self {
        let mut app = Self::new(width, height);
        match state {
            "idle" => {}
            "selecting" => {
                app.image = Some(Arc::new(RgbaImage::from_pixel(
                    width.max(1),
                    height.max(1),
                    image::Rgba([38, 46, 66, 255]),
                )));
                app.selection = Some(Rect {
                    origin: nickel_ui::Point { x: 64.0, y: 72.0 },
                    size: nickel_ui::Size {
                        width: 240.0,
                        height: 160.0,
                    },
                });
            }
            "confirmed" => {
                app = Self::fixture(width, height, "selecting");
                app.confirmed = true;
                app.status = "SELECTION CONFIRMED".into();
            }
            "error" => {
                app.status = "SCREENSHOT FAILED · fixture failure · ESC TO CLOSE".into();
                app.error_visible = true;
            }
            other => panic!("unknown screenshot fixture state `{other}`"),
        }
        app
    }

    fn image_rect(&self) -> Rect {
        image_rect(self.image.as_deref(), self.viewport.0, self.viewport.1)
    }

    fn pointer_moved(&mut self, x: f32, y: f32) -> bool {
        self.cursor = (x, y);
        let Some(start) = self.drag_start else {
            return false;
        };
        self.selection = Some(normalized(
            start,
            clamp_to_rect(self.cursor, self.image_rect()),
        ));
        true
    }

    fn pointer_pressed(&mut self, x: f32, y: f32) -> bool {
        self.cursor = (x, y);
        let image_rect = self.image_rect();
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
                self.status = "SELECTION CONFIRMED".into();
                if self.save_after_confirmation {
                    self.push_effect(ScreenshotEffect::Save);
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

    fn pointer_released(&mut self) -> bool {
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

    fn push_effect(&mut self, effect: ScreenshotEffect) {
        self.effects.push(effect);
        self.effect_evidence.push(EffectEvidence {
            type_name: std::any::type_name::<ScreenshotEffect>(),
            label: Some(format!("{effect:?}")),
        });
    }

    #[cfg(any(target_os = "linux", test))]
    fn perform_semantic_action(
        &mut self,
        action: nickel_session_protocol::ScreenshotTargetAction,
    ) -> bool {
        use nickel_session_protocol::ScreenshotTargetAction;

        let Some(_) = self.image else {
            return false;
        };
        let preview = self.image_rect();
        match action {
            ScreenshotTargetAction::SelectionStart => {
                let start = (
                    preview.origin.x + preview.size.width * 0.25,
                    preview.origin.y + preview.size.height * 0.25,
                );
                self.cursor = start;
                self.drag_start = Some(start);
                self.selection = Some(normalized(start, start));
                self.confirmed = false;
                self.last_click = None;
                true
            }
            ScreenshotTargetAction::SelectionEnd => {
                let Some(start) = self.drag_start.take() else {
                    return false;
                };
                let end = (
                    preview.origin.x + preview.size.width * 0.75,
                    preview.origin.y + preview.size.height * 0.75,
                );
                self.cursor = end;
                self.selection = Some(normalized(start, end));
                self.last_click = None;
                true
            }
            ScreenshotTargetAction::Confirm if self.selection.is_some() => {
                self.confirmed = true;
                self.drag_start = None;
                self.last_click = None;
                self.status = "SELECTION CONFIRMED".into();
                if self.save_after_confirmation {
                    self.push_effect(ScreenshotEffect::Save);
                }
                true
            }
            ScreenshotTargetAction::Confirm
            | ScreenshotTargetAction::CopyImage
            | ScreenshotTargetAction::SaveImage
            | ScreenshotTargetAction::CopyTemporaryPath
            | ScreenshotTargetAction::Cancel => false,
        }
    }
}

impl Application for ScreenshotApp {
    type Message = ScreenshotMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            ScreenshotMessage::Toolbar(action) => self.push_effect(match action {
                ToolbarAction::Copy => ScreenshotEffect::Copy,
                ToolbarAction::Save => ScreenshotEffect::Save,
                ToolbarAction::TemporaryPath => ScreenshotEffect::TemporaryPath,
                ToolbarAction::Cancel => ScreenshotEffect::Cancel,
            }),
        }
    }

    fn complete(&mut self, completion: Completion) -> Result<bool, CompletionFailure> {
        let id = completion.id;
        let input =
            completion
                .downcast::<ScreenshotCompletion>()
                .map_err(|_| CompletionFailure {
                    id,
                    kind: CompletionFailureKind::TypeMismatch,
                    detail: "screenshot completion payload type mismatch".into(),
                })?;
        Ok(match input {
            ScreenshotCompletion::PointerMoved { x, y } => self.pointer_moved(x, y),
            ScreenshotCompletion::PointerPressed { x, y } => self.pointer_pressed(x, y),
            ScreenshotCompletion::PointerReleased => self.pointer_released(),
            #[cfg(any(target_os = "linux", test))]
            ScreenshotCompletion::Semantic { action } => self.perform_semantic_action(action),
            ScreenshotCompletion::Platform { effect, result } => {
                match (effect, result) {
                    (_, Ok(status)) => {
                        self.image = None;
                        self.selection = None;
                        self.confirmed = false;
                        self.status = status;
                    }
                    (_, Err(failure)) => self.status = failure,
                }
                true
            }
        })
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        screenshot_view(self, context)
    }

    fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        self.selection
            .into_iter()
            .map(|rect| FrameOverlay::SelectionMarquee {
                rect,
                fill: None,
                stroke: self.palette.accent,
                width: if self.confirmed { 5.0 } else { 3.0 },
            })
            .collect()
    }

    fn take_effect_evidence(&mut self) -> Vec<EffectEvidence> {
        std::mem::take(&mut self.effect_evidence)
    }

    fn poll(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    fn shortcut(&mut self, shortcut: nickel_ui::Shortcut) -> bool {
        if shortcut == nickel_ui::Shortcut::Escape && (self.image.is_some() || self.error_visible) {
            self.push_effect(ScreenshotEffect::Cancel);
            true
        } else {
            false
        }
    }
}

pub struct ScreenshotTool {
    host: UiHost<ScreenshotApp>,
    capture_deadline: Option<Instant>,
    pending_pointer: Option<(f32, f32, u32, u32)>,
    pointer_deadline: Option<Instant>,
}

impl Default for ScreenshotTool {
    fn default() -> Self {
        Self {
            host: UiHost::new(ScreenshotApp::new(1, 1), 1, 1),
            capture_deadline: None,
            pending_pointer: None,
            pointer_deadline: None,
        }
    }
}

impl ScreenshotTool {
    pub fn change_token(&self) -> nickel_ui::HostChangeToken {
        let inspection = self.host.inspect();
        nickel_ui::HostChangeToken {
            frame_generation: inspection.frame_generation,
            semantic_generation: inspection.semantic_generation,
        }
    }

    pub fn request_capture(&mut self) {
        self.hide();
        self.host.application_mut().save_after_confirmation = false;
        self.capture_deadline = Some(Instant::now() + Duration::from_millis(75));
    }

    pub fn request_capture_to_file(&mut self) {
        self.hide();
        self.host.application_mut().save_after_confirmation = true;
        self.capture_deadline = Some(Instant::now() + Duration::from_millis(75));
    }

    pub fn capture_ready_at(&mut self, now: Instant) -> bool {
        if self
            .capture_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.capture_deadline = None;
            true
        } else {
            false
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        [self.capture_deadline, self.pointer_deadline]
            .into_iter()
            .flatten()
            .min()
    }

    pub fn show(&mut self, image: RgbaImage) {
        let app = self.host.application_mut();
        app.image = Some(Arc::new(image));
        app.status = instructions();
        app.error_visible = false;
        app.dirty = true;
        self.host.poll();
    }

    pub fn show_error(&mut self, error: impl Into<String>) {
        self.hide();
        let app = self.host.application_mut();
        app.status = format!("SCREENSHOT FAILED · {} · ESC TO CLOSE", error.into());
        app.error_visible = true;
        app.dirty = true;
        self.host.poll();
    }

    pub fn visible(&self) -> bool {
        self.host.application().image.is_some() || self.host.application().error_visible
    }

    #[cfg(test)]
    pub fn confirmed(&self) -> bool {
        self.host.application().confirmed
    }

    pub fn hide(&mut self) {
        self.pending_pointer = None;
        self.pointer_deadline = None;
        let app = self.host.application_mut();
        app.image = None;
        app.drag_start = None;
        app.selection = None;
        app.confirmed = false;
        app.last_click = None;
        app.error_visible = false;
        app.save_after_confirmation = false;
        app.dirty = true;
        self.host.poll();
    }

    pub fn pointer_moved(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
        self.resize(width, height, None);
        let outcome = self.host.step(HostBatch {
            completions: vec![Completion::new(
                "screenshot-pointer",
                ScreenshotCompletion::PointerMoved { x, y },
            )],
            events: vec![HostEvent::Ui(UiEvent::PointerMoved(Point { x, y }))],
            ..HostBatch::default()
        });
        outcome.changed
    }

    pub fn queue_pointer_moved(&mut self, x: f32, y: f32, width: u32, height: u32) {
        self.pending_pointer = Some((x, y, width, height));
        self.pointer_deadline
            .get_or_insert_with(|| Instant::now() + Duration::from_millis(10));
    }

    pub fn poll_pointer_deadline(&mut self, now: Instant) -> bool {
        if self.pointer_deadline.is_none_or(|deadline| now < deadline) {
            return false;
        }
        self.pointer_deadline = None;
        let Some((x, y, width, height)) = self.pending_pointer.take() else {
            return false;
        };
        self.pointer_moved(x, y, width, height)
    }

    fn flush_pointer(&mut self) -> bool {
        self.pointer_deadline = None;
        let Some((x, y, width, height)) = self.pending_pointer.take() else {
            return false;
        };
        self.pointer_moved(x, y, width, height)
    }

    pub fn pointer_pressed(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
        self.flush_pointer();
        self.resize(width, height, None);
        let outcome = self.host.step(HostBatch {
            completions: vec![Completion::new(
                "screenshot-pointer",
                ScreenshotCompletion::PointerPressed { x, y },
            )],
            events: vec![HostEvent::Ui(UiEvent::PointerPressed(Point { x, y }))],
            ..HostBatch::default()
        });
        outcome.changed | self.apply_effects()
    }

    pub fn pointer_released(&mut self) -> bool {
        self.flush_pointer();
        let cursor = self.host.application().cursor;
        let outcome = self.host.step(HostBatch {
            completions: vec![Completion::new(
                "screenshot-pointer",
                ScreenshotCompletion::PointerReleased,
            )],
            events: vec![HostEvent::Ui(UiEvent::PointerReleased(Point {
                x: cursor.0,
                y: cursor.1,
            }))],
            ..HostBatch::default()
        });
        outcome.changed | self.apply_effects()
    }

    pub fn escape(&mut self) -> bool {
        let outcome = self.host.step(HostBatch {
            events: vec![HostEvent::Shortcut(nickel_ui::Shortcut::Escape)],
            ..HostBatch::default()
        });
        outcome.changed | self.apply_effects()
    }

    pub fn controller_action(&mut self, action: nickel_ui::ControllerAction) -> bool {
        let outcome = self.host.step(HostBatch {
            events: vec![HostEvent::Controller(action)],
            ..HostBatch::default()
        });
        outcome.changed | self.apply_effects()
    }

    #[cfg(any(target_os = "linux", test))]
    pub fn perform_semantic_action(
        &mut self,
        action: nickel_session_protocol::ScreenshotTargetAction,
    ) -> bool {
        use nickel_session_protocol::ScreenshotTargetAction;

        let toolbar = match action {
            ScreenshotTargetAction::CopyImage => Some(ToolbarAction::Copy),
            ScreenshotTargetAction::SaveImage => Some(ToolbarAction::Save),
            ScreenshotTargetAction::CopyTemporaryPath => Some(ToolbarAction::TemporaryPath),
            ScreenshotTargetAction::Cancel => Some(ToolbarAction::Cancel),
            ScreenshotTargetAction::SelectionStart
            | ScreenshotTargetAction::SelectionEnd
            | ScreenshotTargetAction::Confirm => None,
        };
        let outcome = if let Some(toolbar) = toolbar {
            if !self.host.application().confirmed {
                return false;
            }
            let Ok(target) = self
                .host
                .unique_semantic_target_for_message(&ScreenshotMessage::Toolbar(toolbar))
            else {
                return false;
            };
            self.host.perform_semantic_action(
                target.id,
                nickel_ui::SemanticAction::Invoke(ActionKind::Activate),
            )
        } else {
            self.host.step(HostBatch {
                completions: vec![Completion::new(
                    "screenshot-semantic-action",
                    ScreenshotCompletion::Semantic { action },
                )],
                ..HostBatch::default()
            })
        };
        outcome.changed | self.apply_effects()
    }

    pub fn scene(
        &mut self,
        width: u32,
        height: u32,
        palette: nickel_core::theme::ThemePalette,
    ) -> Vec<PaintCommand> {
        self.resize(width, height, Some(palette));
        self.host.commands().to_vec()
    }

    fn cropped(&self, width: u32, height: u32) -> Option<RgbaImage> {
        let app = self.host.application();
        let image = app.image.as_ref()?;
        let selection = app.selection?;
        let preview = image_rect(Some(image), width, height);
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

    fn copy(&self, width: u32, height: u32) -> Result<String, String> {
        self.cropped(width, height)
            .ok_or_else(|| "COPY FAILED · NO SELECTION".to_owned())
            .and_then(|image| {
                platform::copy_image_to_clipboard(&image)
                    .map(|()| "IMAGE COPIED".to_owned())
                    .map_err(|error| format!("COPY FAILED · {error}"))
            })
    }

    fn apply_effects(&mut self) -> bool {
        let (width, height) = self.host.application().viewport;
        let effects = std::mem::take(&mut self.host.application_mut().effects);
        let mut changed = false;
        for effect in effects {
            if effect == ScreenshotEffect::Cancel {
                self.hide();
                changed = true;
                continue;
            }
            let result = match effect {
                ScreenshotEffect::Copy => self.copy(width, height),
                ScreenshotEffect::Save => self.save(width, height),
                ScreenshotEffect::TemporaryPath => self.temp(width, height),
                ScreenshotEffect::Cancel => unreachable!(),
            };
            changed |= self
                .host
                .step(HostBatch {
                    completions: vec![Completion::new(
                        "screenshot-platform",
                        ScreenshotCompletion::Platform { effect, result },
                    )],
                    ..HostBatch::default()
                })
                .changed;
        }
        changed
    }

    fn temp(&self, width: u32, height: u32) -> Result<String, String> {
        self.cropped(width, height)
            .ok_or_else(|| "TEMP SAVE FAILED · NO SELECTION".to_owned())
            .and_then(|image| {
                platform::copy_temp_image_path(&image)
                    .map(|path| format!("TEMP PATH COPIED · {}", path.display()))
                    .map_err(|error| format!("TEMP SAVE FAILED · {error}"))
            })
    }

    fn save(&self, width: u32, height: u32) -> Result<String, String> {
        let Some(image) = self.cropped(width, height) else {
            return Err("SAVE FAILED · NO SELECTION".into());
        };
        let Some(home) = env::var_os("USERPROFILE")
            .or_else(|| env::var_os("HOME"))
            .map(PathBuf::from)
        else {
            return Err("SAVE FAILED · HOME DIRECTORY IS UNKNOWN".into());
        };
        let directory = home.join("Pictures").join("Screenshots");
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let path = directory.join(format!("Nickel Screenshot {stamp}.png"));
        if fs::create_dir_all(&directory)
            .and_then(|_| image.save(&path).map_err(std::io::Error::other))
            .is_ok()
        {
            Ok(format!("SAVED · {}", path.display()))
        } else {
            Err("SAVE FAILED".into())
        }
    }

    fn resize(
        &mut self,
        width: u32,
        height: u32,
        palette: Option<nickel_core::theme::ThemePalette>,
    ) {
        let app = self.host.application_mut();
        if app.viewport != (width, height) {
            app.viewport = (width, height);
            app.dirty = true;
        }
        if let Some(palette) = palette
            && app.palette != palette
        {
            app.palette = palette;
            app.dirty = true;
        }
        let _ = self.host.step(HostBatch {
            surface_size: Some((width, height)),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
    }
}

fn screenshot_view(
    app: &ScreenshotApp,
    context: ViewContext,
) -> impl nickel_ui::View<ScreenshotMessage> {
    let width = context.viewport.size.width as u32;
    let palette = app.palette;
    let theme = SemanticTheme::from_tokens(nickel_ui::SemanticTokenSet::standard(
        palette.background,
        palette.panel,
        palette.surface,
        palette.surface_hover,
        palette.surface_hover,
        palette.text,
        palette.muted,
        palette.accent,
        palette.accent_soft,
        palette.complement,
        palette.complement,
    ));
    let mut toolbar = Row::new()
        .height(TOOLBAR_HEIGHT)
        .padding(Insets {
            top: 14.0,
            right: 16.0,
            bottom: 14.0,
            left: 16.0,
        })
        .gap(8.0)
        .align_items(Align::Center)
        .background(palette.panel)
        .child(Text::new(&app.status).tone(Tone::Muted).ellipsis(true))
        .child(Spacer::flex());
    if app.confirmed {
        toolbar = toolbar
            .child(
                Button::semantic(
                    theme,
                    ScreenshotMessage::Toolbar(ToolbarAction::Copy),
                    "Copy",
                    ButtonPresentation::Primary,
                )
                .id(toolbar_key(ToolbarAction::Copy))
                .width(112.0),
            )
            .child(
                Button::semantic(
                    theme,
                    ScreenshotMessage::Toolbar(ToolbarAction::Save),
                    "Save",
                    ButtonPresentation::Secondary,
                )
                .id(toolbar_key(ToolbarAction::Save))
                .width(104.0),
            )
            .child(
                Button::semantic(
                    theme,
                    ScreenshotMessage::Toolbar(ToolbarAction::TemporaryPath),
                    "Copy file path",
                    ButtonPresentation::Secondary,
                )
                .id(toolbar_key(ToolbarAction::TemporaryPath))
                .width(158.0),
            )
            .child(
                Button::semantic(
                    theme,
                    ScreenshotMessage::Toolbar(ToolbarAction::Cancel),
                    "Cancel",
                    ButtonPresentation::Quiet,
                )
                .id(toolbar_key(ToolbarAction::Cancel))
                .width(96.0),
            );
    }
    let image_rect = app.image_rect();
    let mut preview = Column::new()
        .width(width as f32)
        .height(context.viewport.size.height)
        .child(toolbar)
        .child(Spacer::vertical(
            (image_rect.origin.y - TOOLBAR_HEIGHT).max(0.0),
        ));
    if let Some(image) = &app.image {
        preview = preview.child(
            Row::new()
                .height(image_rect.size.height)
                .child(Spacer::fixed(image_rect.origin.x))
                .child(
                    Image::new(65_000, image.clone())
                        .width(image_rect.size.width)
                        .height(image_rect.size.height)
                        .fit(ImageFit::Stretch),
                ),
        );
    }
    Container::new()
        .background(palette.background)
        .width(width as f32)
        .height(context.viewport.size.height)
        .child(preview)
}

fn instructions() -> String {
    "DRAG CORNER TO CORNER · DOUBLE-CLICK TO CONFIRM · ESC TO CANCEL".into()
}

fn image_rect(image: Option<&RgbaImage>, width: u32, height: u32) -> Rect {
    let Some(image) = image else {
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

fn toolbar_key(action: ToolbarAction) -> &'static str {
    match action {
        ToolbarAction::Copy => "screenshot-toolbar-copy",
        ToolbarAction::Save => "screenshot-toolbar-save",
        ToolbarAction::TemporaryPath => "screenshot-toolbar-temporary-path",
        ToolbarAction::Cancel => "screenshot-toolbar-cancel",
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

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::{ScreenshotApp, ScreenshotMessage, ScreenshotTool, ToolbarAction, normalized};

    fn toolbar_host() -> nickel_ui::UiHost<ScreenshotApp> {
        let mut app = ScreenshotApp::new(1200, 760);
        app.confirmed = true;
        app.status = "SELECTION CONFIRMED".into();
        nickel_ui::UiHost::new(app, 1200, 760)
    }

    #[test]
    fn toolbar_frame_exposes_typed_actions_to_controller_input() {
        let mut toolbar = toolbar_host();
        for action in [
            ToolbarAction::Copy,
            ToolbarAction::Save,
            ToolbarAction::TemporaryPath,
            ToolbarAction::Cancel,
        ] {
            assert_eq!(
                toolbar
                    .perform_controller_semantic_action(
                        toolbar
                            .unique_semantic_target_for_message(&ScreenshotMessage::Toolbar(action))
                            .expect("toolbar action target")
                            .id,
                        nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
                    )
                    .messages,
                vec![nickel_ui::MessageEvidence {
                    type_name: std::any::type_name::<ScreenshotMessage>(),
                    label: None,
                }]
            );
        }
    }

    #[test]
    fn toolbar_frame_publishes_accessible_button_authority() {
        let toolbar = toolbar_host();
        let nodes = toolbar.semantic_nodes();

        for (action, label) in [
            (ToolbarAction::Copy, "Copy"),
            (ToolbarAction::Save, "Save"),
            (ToolbarAction::TemporaryPath, "Copy file path"),
            (ToolbarAction::Cancel, "Cancel"),
        ] {
            let target = toolbar
                .unique_semantic_target_for_message(&ScreenshotMessage::Toolbar(action))
                .expect("toolbar action target");
            let node = nodes
                .iter()
                .find(|node| node.id == target.id)
                .expect("toolbar action has a semantic node");
            assert_eq!(node.name.as_deref(), Some(label));
            assert_eq!(node.role, Some(nickel_ui::SemanticRole::Button));
            assert_eq!(node.actions, vec![nickel_ui::ActionKind::Activate]);
        }
    }

    #[test]
    fn normalization_accepts_reverse_corner_drag() {
        let rect = normalized((30.0, 40.0), (10.0, 15.0));
        assert_eq!(rect.origin.x, 10.0);
        assert_eq!(rect.origin.y, 15.0);
        assert_eq!(rect.size.width, 20.0);
        assert_eq!(rect.size.height, 25.0);
    }

    #[test]
    fn production_pointer_path_selects_from_bottom_right_to_top_left() {
        let mut tool = ScreenshotTool::default();
        tool.show(RgbaImage::new(800, 450));
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let _ = tool.scene(1280, 720, palette);
        let preview = tool.host.application().image_rect();
        let start = (
            preview.origin.x + preview.size.width - 12.0,
            preview.origin.y + preview.size.height - 12.0,
        );
        let end = (preview.origin.x + 18.0, preview.origin.y + 16.0);

        assert!(tool.pointer_pressed(start.0, start.1, 1280, 720));
        tool.queue_pointer_moved(end.0, end.1, 1280, 720);
        assert!(tool.pointer_released());

        let selection = tool
            .host
            .application()
            .selection
            .expect("reverse selection");
        assert_eq!(selection.origin, nickel_ui::Point { x: end.0, y: end.1 });
        assert!((selection.size.width - (start.0 - end.0)).abs() < 0.01);
        assert!((selection.size.height - (start.1 - end.1)).abs() < 0.01);
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
        tool.host.application_mut().viewport = (1200, 760);
        let preview = tool.host.application().image_rect();
        tool.host.application_mut().selection = Some(nickel_ui::Rect::new(
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
        assert!(tool.host.application().status.contains("permission denied"));
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
        assert!(tool.host.application().save_after_confirmation);
        assert!(tool.capture_deadline.is_some());
        tool.request_capture();
        assert!(!tool.host.application().save_after_confirmation);
        assert!(tool.next_deadline().is_some());
    }

    #[test]
    fn host_batch_owns_selection_completions_effects_failures_and_escape() {
        let mut app = ScreenshotApp::new(800, 600);
        app.image = Some(std::sync::Arc::new(RgbaImage::new(400, 200)));
        let mut host = nickel_ui::UiHost::new(app, 800, 600);
        let preview = host.application().image_rect();
        let outcome = host.step(nickel_ui::HostBatch {
            completions: vec![nickel_ui::Completion::new(
                "screenshot-pointer",
                super::ScreenshotCompletion::PointerPressed {
                    x: preview.origin.x + 10.0,
                    y: preview.origin.y + 10.0,
                },
            )],
            ..nickel_ui::HostBatch::default()
        });
        assert!(outcome.changed);
        assert!(outcome.telemetry.rebuilt);
        assert_eq!(outcome.telemetry.completions_processed, 1);

        let failure = host.step(nickel_ui::HostBatch {
            completions: vec![nickel_ui::Completion::new("screenshot-pointer", 42_u32)],
            ..nickel_ui::HostBatch::default()
        });
        assert_eq!(failure.completion_failures.len(), 1);
        assert_eq!(
            failure.completion_failures[0].kind,
            nickel_ui::CompletionFailureKind::TypeMismatch
        );

        let escaped = host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Shortcut(nickel_ui::Shortcut::Escape)],
            ..nickel_ui::HostBatch::default()
        });
        assert!(escaped.changed);
        assert_eq!(escaped.effects.len(), 1);
        assert_eq!(
            host.application_mut().effects,
            [super::ScreenshotEffect::Cancel]
        );
    }

    #[test]
    fn every_successful_capture_action_closes_the_screenshot_surface_model() {
        for effect in [
            super::ScreenshotEffect::Copy,
            super::ScreenshotEffect::Save,
            super::ScreenshotEffect::TemporaryPath,
        ] {
            let mut app = ScreenshotApp::new(800, 600);
            app.image = Some(std::sync::Arc::new(RgbaImage::new(400, 200)));
            app.selection = Some(nickel_ui::Rect::new(0.0, 0.0, 100.0, 100.0));
            let mut host = nickel_ui::UiHost::new(app, 800, 600);
            host.step(nickel_ui::HostBatch {
                completions: vec![nickel_ui::Completion::new(
                    "screenshot-platform",
                    super::ScreenshotCompletion::Platform {
                        effect,
                        result: Ok("done".into()),
                    },
                )],
                ..nickel_ui::HostBatch::default()
            });

            assert!(host.application().image.is_none());
            assert!(host.application().selection.is_none());
            assert!(!host.application().confirmed);
        }
    }

    #[test]
    fn semantic_selection_actions_drive_application_state_without_coordinates() {
        use nickel_session_protocol::ScreenshotTargetAction;

        let mut tool = ScreenshotTool::default();
        tool.show(RgbaImage::new(400, 200));
        let _ = tool.scene(
            800,
            600,
            nickel_core::theme::ThemePalette::from_appearance(Default::default()),
        );
        assert!(tool.perform_semantic_action(ScreenshotTargetAction::SelectionStart));
        assert!(tool.perform_semantic_action(ScreenshotTargetAction::SelectionEnd));
        assert!(tool.perform_semantic_action(ScreenshotTargetAction::Confirm));
        assert!(tool.host.application().confirmed);
        let _ = tool.scene(
            800,
            600,
            nickel_core::theme::ThemePalette::from_appearance(Default::default()),
        );
        assert!(tool.perform_semantic_action(ScreenshotTargetAction::Cancel));
        assert!(!tool.visible());
    }

    #[test]
    fn semantic_actions_reject_unavailable_state_without_synthesizing_input() {
        use nickel_session_protocol::ScreenshotTargetAction;

        let mut tool = ScreenshotTool::default();
        assert!(!tool.perform_semantic_action(ScreenshotTargetAction::SelectionStart));
        tool.show(RgbaImage::new(400, 200));
        assert!(!tool.perform_semantic_action(ScreenshotTargetAction::Confirm));
        assert!(!tool.perform_semantic_action(ScreenshotTargetAction::CopyImage));
    }

    #[test]
    fn pointer_motion_is_coalesced_to_the_latest_sample_before_one_frame() {
        let mut tool = ScreenshotTool::default();
        tool.show(RgbaImage::new(800, 600));
        let before = tool.change_token();
        for x in 0..100 {
            tool.queue_pointer_moved(x as f32, 240.0, 800, 600);
        }
        let deadline = tool.pointer_deadline.expect("motion schedules one frame");

        assert_eq!(tool.change_token(), before);
        assert_eq!(tool.pending_pointer.map(|sample| sample.0), Some(99.0));
        assert!(tool.poll_pointer_deadline(deadline));
        assert_eq!(tool.host.application().cursor, (99.0, 240.0));
        assert!(tool.pending_pointer.is_none());
        assert!(tool.pointer_deadline.is_none());
    }
}
