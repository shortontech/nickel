use super::PANEL_TRAY_ICON_SIZE;
use crate::model::TrayItem;
use std::sync::Arc;

pub(super) fn panel_tray_icons(items: &[TrayItem]) -> Vec<Arc<image::RgbaImage>> {
    items
        .iter()
        .rev()
        .take(64)
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
    let keep_from = items.len().saturating_sub(64);
    items.into_iter().skip(keep_from).collect()
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
