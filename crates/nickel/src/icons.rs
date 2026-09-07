use std::{fs, path::Path};

use image::{DynamicImage, RgbaImage};

const RASTER_SIZE: u32 = 96;

pub fn load(path: &Path) -> Option<RgbaImage> {
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
    {
        load_svg(path)
    } else {
        image::open(path).ok().map(DynamicImage::into_rgba8)
    }
}

pub fn resized(source: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    if width == 0 || height == 0 {
        return RgbaImage::new(width, height);
    }
    let scale = (width as f32 / source.width() as f32).min(height as f32 / source.height() as f32);
    let fitted_width = (source.width() as f32 * scale).round().max(1.0) as u32;
    let fitted_height = (source.height() as f32 * scale).round().max(1.0) as u32;
    let fitted = image::imageops::resize(
        source,
        fitted_width,
        fitted_height,
        image::imageops::FilterType::Lanczos3,
    );
    let mut output = RgbaImage::new(width, height);
    let x = i64::from((width - fitted.width()) / 2);
    let y = i64::from((height - fitted.height()) / 2);
    image::imageops::overlay(&mut output, &fitted, x, y);
    output
}

pub fn nickel_application(name: &str) -> Option<(u16, RgbaImage)> {
    let (id, bytes): (u16, &[u8]) = if name.starts_with("Nickel Settings") {
        (
            0x3000,
            include_bytes!("../../../assets/icons/nickel-settings.png"),
        )
    } else if name.starts_with("Nickel File") {
        (
            0x3001,
            include_bytes!("../../../assets/icons/nickel-file.png"),
        )
    } else {
        return None;
    };
    image::load_from_memory(bytes)
        .ok()
        .map(DynamicImage::into_rgba8)
        .map(|image| (id, image))
}

fn load_svg(path: &Path) -> Option<RgbaImage> {
    let data = fs::read(path).ok()?;
    load_svg_bytes(&data, RASTER_SIZE)
}

pub fn load_svg_bytes(data: &[u8], raster_size: u32) -> Option<RgbaImage> {
    let tree = resvg::usvg::Tree::from_data(&data, &Default::default()).ok()?;
    let size = tree.size();
    let scale = (raster_size as f32 / size.width()).min(raster_size as f32 / size.height());
    let width = (size.width() * scale).round().max(1.0) as u32;
    let height = (size.height() * scale).round().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    RgbaImage::from_raw(width, height, pixmap.data().to_vec())
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::{load_svg_bytes, nickel_application, resized};

    #[test]
    fn resize_preserves_aspect_ratio_and_centers_icon() {
        let source = RgbaImage::from_pixel(8, 4, Rgba([255, 0, 0, 255]));
        let output = resized(&source, 8, 8);
        assert_eq!(output.dimensions(), (8, 8));
        assert_eq!(output.get_pixel(0, 0).0[3], 0);
        assert_eq!(output.get_pixel(0, 2).0, [255, 0, 0, 255]);
    }

    #[test]
    fn built_in_nickel_applications_keep_icons_without_desktop_entries() {
        let (settings_id, settings) = nickel_application("Nickel Settings").unwrap();
        let (file_id, file) = nickel_application("Nickel File").unwrap();

        assert_ne!(settings_id, file_id);
        assert!(settings.pixels().any(|pixel| pixel.0[3] != 0));
        assert!(file.pixels().any(|pixel| pixel.0[3] != 0));
        assert!(nickel_application("Other application").is_none());
    }

    #[test]
    fn embedded_chat_icon_is_a_compact_alpha_mask() {
        let icon =
            load_svg_bytes(include_bytes!("../../../assets/icons/nickel-chat.svg"), 24).unwrap();

        assert_eq!(icon.dimensions(), (24, 24));
        assert!(icon.pixels().any(|pixel| pixel.0[3] == 0));
        assert!(icon.pixels().any(|pixel| pixel.0[3] != 0));
    }
}
