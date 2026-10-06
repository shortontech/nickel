use super::PANEL_TRAY_ICON_SIZE;
use crate::model::TrayItem;
use std::sync::Arc;

pub(super) fn plugin_tray_images(
    items: &[TrayItem],
    icons: &[Arc<image::RgbaImage>],
) -> crate::plugin_panel::PluginImages {
    items
        .iter()
        .zip(icons)
        .enumerate()
        .map(|(index, (item, icon))| {
            (
                format!("tray:{}", item.id),
                (0x7100 + index as u16, Arc::clone(icon)),
            )
        })
        .collect()
}

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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tray_artwork_uses_stable_item_keys_and_native_pixels() {
        let items = vec![TrayItem {
            id: "org.nickel.Test/Item".into(),
            title: "Test".into(),
            icon: image::RgbaImage::from_pixel(32, 32, image::Rgba([55, 200, 255, 255])),
        }];
        let icons = panel_tray_icons(&items);
        let images = plugin_tray_images(&items, &icons);
        let (id, image) = &images["tray:org.nickel.Test/Item"];
        assert_eq!(*id, 0x7100);
        assert!(Arc::ptr_eq(image, &icons[0]));
        assert_eq!(image.get_pixel(0, 0).0, [55, 200, 255, 255]);
        assert!(plugin_tray_images(&[], &[]).is_empty());
    }
}
