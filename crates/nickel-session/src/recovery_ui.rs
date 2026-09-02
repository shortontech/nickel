use nickel_ui::{
    ActionKind, Application, Border, Button, Column, Insets, Point, Row, SdlComponentRenderer,
    Shortcut, Text, UiEvent, UiHost, ViewContext,
};

use crate::shell_layout::Geometry;
use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    utils::Transform,
};
use std::cell::RefCell;

pub const WIDTH: u32 = 560;
pub const HEIGHT: u32 = 144;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryAction {
    Retry,
    Exit,
}

#[derive(Default)]
struct RecoveryApplication {
    pending: Option<RecoveryAction>,
}

impl Application for RecoveryApplication {
    type Message = RecoveryAction;

    fn update(&mut self, message: Self::Message) {
        self.pending = Some(message);
    }

    fn shortcut(&mut self, shortcut: Shortcut) -> bool {
        self.pending = match shortcut {
            Shortcut::Submit => Some(RecoveryAction::Retry),
            Shortcut::Escape => Some(RecoveryAction::Exit),
            _ => None,
        };
        self.pending.is_some()
    }

    fn view(&self, _context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        Column::new()
            .width(WIDTH as f32)
            .height(HEIGHT as f32)
            .padding(Insets {
                top: 22.0,
                right: 28.0,
                bottom: 20.0,
                left: 28.0,
            })
            .gap(8.0)
            .background(0x24191c)
            .border_value(Border::new(0xd05a68, 1.0))
            .radius(14.0)
            .child(
                Text::new("Nickel shell needs attention")
                    .scale(1.25)
                    .bold(true)
                    .color(0xfff4f5),
            )
            .child(
                Text::new("The compositor is still running and your applications are safe.")
                    .color(0xe8c9cd),
            )
            .child(
                Row::new()
                    .gap(14.0)
                    .child(
                        Button::new(RecoveryAction::Retry, "Enter  Retry now")
                            .id("recovery-retry")
                            .width(156.0)
                            .height(30.0)
                            .padding(Insets::symmetric(5.0, 12.0))
                            .radius(7.0)
                            .background(0x9d3444)
                            .color(0xffffff)
                            .focus_border(0xff9aa7),
                    )
                    .child(
                        Button::new(RecoveryAction::Exit, "Esc  Log out safely")
                            .id("recovery-exit")
                            .width(180.0)
                            .height(30.0)
                            .padding(Insets::symmetric(5.0, 12.0))
                            .radius(7.0)
                            .background(0x37272b)
                            .border(0x6b4b52, 1.0)
                            .color(0xe8c9cd)
                            .focus_border(0xff9aa7),
                    ),
            )
    }
}

pub struct RecoveryUi {
    host: UiHost<RecoveryApplication>,
    raster: RefCell<RecoveryRasterCache>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RecoveryRasterDiagnostics {
    pub live_bytes: usize,
    pub peak_bytes: usize,
    pub entries: usize,
    pub generation: u64,
    pub rasterizations: u64,
    pub avoided_rasterizations: u64,
    pub evictions: u64,
    /// Smithay owns renderer imports; their byte cost is not exposed by its API.
    pub renderer_bytes: Option<usize>,
}

#[derive(Default)]
struct RecoveryRasterCache {
    generation: u64,
    entry: Option<(u64, MemoryRenderBuffer)>,
    peak_bytes: usize,
    rasterizations: u64,
    avoided_rasterizations: u64,
    evictions: u64,
}

const RECOVERY_RASTER_BYTES: usize = WIDTH as usize * HEIGHT as usize * 4;

impl RecoveryUi {
    pub fn new() -> Self {
        Self {
            host: UiHost::new(RecoveryApplication::default(), WIDTH, HEIGHT),
            raster: RefCell::new(RecoveryRasterCache::default()),
        }
    }

    pub fn panel_geometry(output: Geometry) -> Geometry {
        let width = output.width.clamp(1, WIDTH as i32);
        let height = output.height.clamp(1, HEIGHT as i32);
        Geometry {
            x: output.x + (output.width - width) / 2,
            y: output.y + (output.height - height) / 2,
            width,
            height,
        }
    }

    fn local_point(output: Geometry, x: f64, y: f64) -> Option<Point> {
        let panel = Self::panel_geometry(output);
        if x < f64::from(panel.x)
            || x >= f64::from(panel.x + panel.width)
            || y < f64::from(panel.y)
            || y >= f64::from(panel.y + panel.height)
        {
            return None;
        }
        Some(Point {
            x: ((x - f64::from(panel.x)) * f64::from(WIDTH) / f64::from(panel.width)) as f32,
            y: ((y - f64::from(panel.y)) * f64::from(HEIGHT) / f64::from(panel.height)) as f32,
        })
    }

    pub fn pointer(
        &mut self,
        output: Geometry,
        x: f64,
        y: f64,
        pressed: bool,
    ) -> Option<RecoveryAction> {
        let point = Self::local_point(output, x, y)?;
        let event = if pressed {
            UiEvent::PointerPressed(point)
        } else {
            UiEvent::PointerReleased(point)
        };
        self.host.handle_event(event);
        self.invalidate_raster();
        self.take_action()
    }

    pub fn touch(&mut self, output: Geometry, x: f64, y: f64) -> Option<RecoveryAction> {
        let point = Self::local_point(output, x, y)?;
        self.host.handle_event(UiEvent::PointerPressed(point));
        self.host.handle_event(UiEvent::PointerReleased(point));
        self.invalidate_raster();
        self.take_action()
    }

    pub fn shortcut(&mut self, shortcut: Shortcut) -> Option<RecoveryAction> {
        self.host.shortcut(shortcut);
        self.invalidate_raster();
        self.take_action()
    }

    pub fn activate(&mut self, action: RecoveryAction) -> Option<RecoveryAction> {
        let target = self
            .host
            .unique_semantic_target_for_message(&action)
            .ok()?
            .id;
        self.host.perform_semantic_action(
            target,
            nickel_ui::SemanticAction::Invoke(ActionKind::Activate),
        );
        self.invalidate_raster();
        self.take_action()
    }

    fn take_action(&mut self) -> Option<RecoveryAction> {
        self.host.application_mut().pending.take()
    }

    pub fn render_pixels(&self) -> Vec<u8> {
        let mut renderer = SdlComponentRenderer::new_pixel_buffer(WIDTH, HEIGHT, 1.0);
        self.host.render_software(&mut renderer);
        renderer
            .pixels()
            .iter()
            .flat_map(|pixel| [pixel.r, pixel.g, pixel.b, pixel.a])
            .collect()
    }

    fn invalidate_raster(&self) {
        let mut cache = self.raster.borrow_mut();
        cache.generation = cache.generation.saturating_add(1);
        cache.evictions = cache
            .evictions
            .saturating_add(u64::from(cache.entry.is_some()));
        cache.entry = None;
    }

    pub fn raster_diagnostics(&self) -> RecoveryRasterDiagnostics {
        let cache = self.raster.borrow();
        RecoveryRasterDiagnostics {
            live_bytes: usize::from(cache.entry.is_some()) * RECOVERY_RASTER_BYTES,
            peak_bytes: cache.peak_bytes,
            entries: usize::from(cache.entry.is_some()),
            generation: cache.generation,
            rasterizations: cache.rasterizations,
            avoided_rasterizations: cache.avoided_rasterizations,
            evictions: cache.evictions,
            renderer_bytes: None,
        }
    }

    pub fn release_raster(&self) {
        let mut cache = self.raster.borrow_mut();
        if cache.entry.is_none() {
            return;
        }
        cache.evictions = cache.evictions.saturating_add(1);
        cache.entry = None;
        drop(cache);
        tracing::trace!(diagnostics = ?self.raster_diagnostics(), "recovery raster retired");
    }

    pub fn render_buffer(&self) -> MemoryRenderBuffer {
        let generation = self.raster.borrow().generation;
        let cached = {
            let cache = self.raster.borrow();
            cache
                .entry
                .as_ref()
                .filter(|(entry_generation, _)| *entry_generation == generation)
                .map(|(_, buffer)| buffer.clone())
        };
        if let Some(buffer) = cached {
            let mut cache = self.raster.borrow_mut();
            cache.avoided_rasterizations = cache.avoided_rasterizations.saturating_add(1);
            return buffer;
        }
        let buffer = MemoryRenderBuffer::from_slice(
            &self.render_pixels(),
            Fourcc::Abgr8888,
            (WIDTH as i32, HEIGHT as i32),
            1,
            Transform::Normal,
            None,
        );
        let mut cache = self.raster.borrow_mut();
        cache.rasterizations = cache.rasterizations.saturating_add(1);
        if cache.entry.is_some() {
            cache.evictions = cache.evictions.saturating_add(1);
        }
        cache.entry = Some((generation, buffer.clone()));
        cache.peak_bytes = cache.peak_bytes.max(RECOVERY_RASTER_BYTES);
        drop(cache);
        tracing::trace!(diagnostics = ?self.raster_diagnostics(), "recovery raster cached");
        buffer
    }
}

#[cfg(test)]
mod tests {
    use nickel_ui::{SemanticRole, SemanticSelector};

    use super::*;

    #[test]
    fn host_declares_typed_retry_and_exit_buttons() {
        let ui = RecoveryUi::new();
        let buttons = ui.host.query(&SemanticSelector::Role(SemanticRole::Button));
        assert_eq!(buttons.len(), 2);
        assert_eq!(
            ui.host
                .semantic_targets_for_message(&RecoveryAction::Retry)
                .len(),
            1
        );
        assert_eq!(
            ui.host
                .semantic_targets_for_message(&RecoveryAction::Exit)
                .len(),
            1
        );
    }

    #[test]
    fn pointer_touch_keyboard_and_semantic_routes_converge() {
        let output = Geometry {
            x: 50,
            y: 30,
            width: 800,
            height: 600,
        };
        let mut ui = RecoveryUi::new();
        let retry = ui
            .host
            .unique_semantic_target_for_message(&RecoveryAction::Retry)
            .unwrap()
            .bounds;
        let panel = RecoveryUi::panel_geometry(output);
        let map = |point: Point| {
            (
                f64::from(panel.x) + f64::from(point.x) * f64::from(panel.width) / f64::from(WIDTH),
                f64::from(panel.y)
                    + f64::from(point.y) * f64::from(panel.height) / f64::from(HEIGHT),
            )
        };
        let (x, y) = map(Point {
            x: retry.origin.x + retry.size.width / 2.0,
            y: retry.origin.y + retry.size.height / 2.0,
        });
        assert_eq!(ui.pointer(output, x, y, true), None);
        assert_eq!(ui.pointer(output, x, y, false), Some(RecoveryAction::Retry));
        assert_eq!(ui.touch(output, x, y), Some(RecoveryAction::Retry));
        assert_eq!(ui.shortcut(Shortcut::Submit), Some(RecoveryAction::Retry));
        assert_eq!(ui.shortcut(Shortcut::Escape), Some(RecoveryAction::Exit));
        assert_eq!(
            ui.activate(RecoveryAction::Exit),
            Some(RecoveryAction::Exit)
        );
    }

    #[test]
    fn host_raster_is_visible_and_has_exact_size() {
        let pixels = RecoveryUi::new().render_pixels();
        assert_eq!(pixels.len(), WIDTH as usize * HEIGHT as usize * 4);
        assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] != 0));
    }

    #[test]
    fn unchanged_recovery_visual_reuses_one_scale_independent_raster() {
        let ui = RecoveryUi::new();
        ui.render_buffer();
        ui.render_buffer();
        ui.render_buffer();
        ui.render_buffer();

        let diagnostics = ui.raster_diagnostics();
        assert_eq!(diagnostics.entries, 1);
        assert_eq!(diagnostics.live_bytes, RECOVERY_RASTER_BYTES);
        assert_eq!(diagnostics.rasterizations, 1);
        assert_eq!(diagnostics.avoided_rasterizations, 3);
    }

    #[test]
    fn recovery_visual_changes_and_hiding_retire_rasters() {
        let mut ui = RecoveryUi::new();
        ui.render_buffer();
        let generation = ui.raster_diagnostics().generation;
        ui.shortcut(Shortcut::Submit);
        let invalidated = ui.raster_diagnostics();
        assert_eq!(invalidated.generation, generation + 1);
        assert_eq!(invalidated.live_bytes, 0);

        ui.render_buffer();
        ui.release_raster();
        let released = ui.raster_diagnostics();
        assert_eq!(released.live_bytes, 0);
        assert_eq!(released.entries, 0);
        assert!(released.evictions >= 2);
    }
}
