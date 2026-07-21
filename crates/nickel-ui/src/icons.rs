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

fn load_svg(path: &Path) -> Option<RgbaImage> {
    let data = fs::read(path).ok()?;
    let tree = resvg::usvg::Tree::from_data(&data, &Default::default()).ok()?;
    let size = tree.size();
    let scale = (RASTER_SIZE as f32 / size.width()).min(RASTER_SIZE as f32 / size.height());
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

    use super::resized;

    #[test]
    fn resize_preserves_aspect_ratio_and_centers_icon() {
        let source = RgbaImage::from_pixel(8, 4, Rgba([255, 0, 0, 255]));
        let output = resized(&source, 8, 8);
        assert_eq!(output.dimensions(), (8, 8));
        assert_eq!(output.get_pixel(0, 0).0[3], 0);
        assert_eq!(output.get_pixel(0, 2).0, [255, 0, 0, 255]);
    }
}
