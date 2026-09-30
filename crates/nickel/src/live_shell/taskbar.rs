use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use jiff::Zoned;
#[cfg(test)]
use nickel_ui::Rect;

use super::PANEL_TRAY_ICON_SIZE;
#[cfg(test)]
use super::{
    PANEL_CLOCK_WIDTH, PANEL_CODEX_ICON_SIZE, PANEL_CODEX_WIDTH, PANEL_CONTROL_GAP,
    PANEL_TRAY_WIDTH,
};
use crate::model::TrayItem;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PanelStatusLayout {
    pub(super) control_start: f32,
    pub(super) tray_start: f32,
    pub(super) codex_start: f32,
}

#[cfg(test)]
impl PanelStatusLayout {
    pub(super) fn codex_icon_bounds(self) -> Rect {
        Rect::new(
            self.codex_start + (PANEL_CODEX_WIDTH - PANEL_CODEX_ICON_SIZE) / 2.0,
            (56.0 - PANEL_CODEX_ICON_SIZE) / 2.0,
            PANEL_CODEX_ICON_SIZE,
            PANEL_CODEX_ICON_SIZE,
        )
    }
}

#[cfg(test)]
pub(super) fn panel_status_layout(
    width: u32,
    tray_count: usize,
    codex_available: bool,
) -> PanelStatusLayout {
    let control_start = panel_control_start(width);
    let tray_start = control_start - tray_count.min(4) as f32 * PANEL_TRAY_WIDTH;
    PanelStatusLayout {
        control_start,
        tray_start,
        codex_start: tray_start
            - if codex_available {
                PANEL_CODEX_WIDTH
            } else {
                0.0
            },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TaskbarHover {
    Launcher,
    Task(usize),
    Tray(usize),
    Control,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TaskbarAction {
    OnScreenKeyboard,
    Launcher,
    Task(usize),
    TaskContext(usize),
    MoveTaskPinLeft(String),
    MoveTaskPinRight(String),
    Codex,
    Tray(String),
    TrayContext(String),
    Control,
}

pub(super) fn panel_clock_text() -> (String, String) {
    let now = Zoned::now();
    (
        now.strftime("%-I:%M %p").to_string(),
        now.strftime("%-m/%-d/%Y").to_string(),
    )
}

pub(super) fn duration_until_next_minute() -> Duration {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let elapsed_in_minute = Duration::new(elapsed.as_secs() % 60, elapsed.subsec_nanos());
    Duration::from_secs(60)
        .saturating_sub(elapsed_in_minute)
        .max(Duration::from_millis(1))
}

#[cfg(test)]
fn panel_control_start(width: u32) -> f32 {
    width as f32 - PANEL_CLOCK_WIDTH - PANEL_CONTROL_GAP
}

pub(super) fn panel_tray_icons(items: &[TrayItem]) -> Vec<Arc<image::RgbaImage>> {
    items
        .iter()
        .rev()
        .take(4)
        .rev()
        .map(|item| {
            Arc::new(crate::icons::resized(
                &item.icon,
                PANEL_TRAY_ICON_SIZE,
                PANEL_TRAY_ICON_SIZE,
            ))
        })
        .collect()
}

pub(super) fn normalize_tray_items(items: Vec<TrayItem>) -> Vec<TrayItem> {
    let keep_from = items.len().saturating_sub(4);
    items.into_iter().skip(keep_from).collect()
}

#[cfg(test)]
pub(super) fn visible_tray_item(items: &[TrayItem], visual_index: usize) -> Option<&TrayItem> {
    items.get(items.len().saturating_sub(4).checked_add(visual_index)?)
}

pub(super) fn tint_panel_icon(mut icon: image::RgbaImage, color: u32) -> image::RgbaImage {
    let tint = [
        ((color >> 16) & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        (color & 0xff) as u8,
    ];
    for pixel in icon.pixels_mut() {
        let coverage = u8::MAX - pixel.0[0];
        pixel.0[3] = ((u16::from(pixel.0[3]) * u16::from(coverage)) / u16::from(u8::MAX)) as u8;
        pixel.0[..3].copy_from_slice(&tint);
    }
    icon
}
