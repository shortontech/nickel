use std::{
    fs,
    path::Path,
    sync::{Arc, OnceLock},
};

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

pub fn nickel_application(identity_or_name: &str) -> Option<(u16, Arc<RgbaImage>)> {
    static SETTINGS: OnceLock<Option<Arc<RgbaImage>>> = OnceLock::new();
    static FILE: OnceLock<Option<Arc<RgbaImage>>> = OnceLock::new();
    let (id, bytes, cache): (u16, &[u8], &OnceLock<Option<Arc<RgbaImage>>>) =
        if identity_or_name == "nickel-settings" || identity_or_name.starts_with("Nickel Settings")
        {
            (
                0x3000,
                include_bytes!("../../../assets/icons/nickel-settings.png"),
                &SETTINGS,
            )
        } else if identity_or_name == "nickel-file" || identity_or_name.starts_with("Nickel File") {
            (
                0x3001,
                include_bytes!("../../../assets/icons/nickel-file.png"),
                &FILE,
            )
        } else {
            return None;
        };
    cache
        .get_or_init(|| {
            image::load_from_memory(bytes)
                .ok()
                .map(DynamicImage::into_rgba8)
                .map(Arc::new)
        })
        .as_ref()
        .map(|image| (id, Arc::clone(image)))
}

const CODEX_PET_BYTES: [&[u8]; 6] = [
    include_bytes!("../../../assets/icons/codex-pets/duck.png"),
    include_bytes!("../../../assets/icons/codex-pets/elephant.png"),
    include_bytes!("../../../assets/icons/codex-pets/tiger.png"),
    include_bytes!("../../../assets/icons/codex-pets/giraffe.png"),
    include_bytes!("../../../assets/icons/codex-pets/cat.png"),
    include_bytes!("../../../assets/icons/codex-pets/golden-retriever.png"),
];

pub fn codex_pet(application_id: &str, frame: u8) -> Option<(u16, Arc<RgbaImage>)> {
    let project = application_id.strip_prefix("io.nickel.codex.project.")?;
    static FRAMES: OnceLock<Vec<Vec<Arc<RgbaImage>>>> = OnceLock::new();
    let frames = FRAMES.get_or_init(|| {
        CODEX_PET_BYTES
            .iter()
            .map(|bytes| {
                let original = image::load_from_memory(bytes)
                    .expect("embedded Codex pet image remains valid")
                    .into_rgba8();
                let mut bounds = (original.width(), original.height(), 0, 0);
                for (x, y, pixel) in original.enumerate_pixels() {
                    if pixel[3] > 8 {
                        bounds.0 = bounds.0.min(x);
                        bounds.1 = bounds.1.min(y);
                        bounds.2 = bounds.2.max(x + 1);
                        bounds.3 = bounds.3.max(y + 1);
                    }
                }
                let face = if bounds.2 > bounds.0 && bounds.3 > bounds.1 {
                    image::imageops::crop_imm(
                        &original,
                        bounds.0,
                        bounds.1,
                        bounds.2 - bounds.0,
                        bounds.3 - bounds.1,
                    )
                    .to_image()
                } else {
                    original
                };
                let face = resized(&face, 88, 88);
                (0..4)
                    .map(|phase| {
                        let mut canvas = RgbaImage::new(96, 96);
                        let top = match phase {
                            0 => 5,
                            1 | 3 => 3,
                            _ => 1,
                        };
                        image::imageops::overlay(&mut canvas, &face, 4, top);
                        Arc::new(canvas)
                    })
                    .collect()
            })
            .collect()
    });
    let hash = project.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    let pet = hash as usize % frames.len();
    let phase = usize::from(frame % 4);
    Some((
        0x3100 + (pet as u16 * 4) + phase as u16,
        Arc::clone(&frames[pet][phase]),
    ))
}

#[cfg(test)]
mod codex_pet_tests {
    use super::codex_pet;

    #[test]
    fn project_face_is_stable_across_windows_and_animation_frames() {
        let project = "io.nickel.codex.project.abc123";
        let (first_id, first) = codex_pet(project, 0).unwrap();
        let (same_id, same) = codex_pet(project, 0).unwrap();
        assert_eq!(first_id, same_id);
        assert!(std::sync::Arc::ptr_eq(&first, &same));
        for frame in 1..4 {
            let (animated_id, animated) = codex_pet(project, frame).unwrap();
            assert_eq!((animated_id - 0x3100) / 4, (first_id - 0x3100) / 4);
            assert_eq!(animated.dimensions(), first.dimensions());
        }
        let (_, raised) = codex_pet(project, 1).unwrap();
        for y in 0..94 {
            for x in 0..96 {
                assert_eq!(first.get_pixel(x, y + 2), raised.get_pixel(x, y));
            }
        }
        assert!(codex_pet("unrelated.application", 0).is_none());
    }

    #[test]
    fn separate_projects_can_use_all_six_faces() {
        let faces = (0..256)
            .map(|project| {
                let id = format!("io.nickel.codex.project.{project:016x}");
                (codex_pet(&id, 0).unwrap().0 - 0x3100) / 4
            })
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(faces.len(), 6);
    }
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
    // tiny-skia stores premultiplied RGBA, while `RgbaImage` and every Nickel
    // UI image API use straight-alpha pixels. Keeping the two representations
    // distinct matters when an icon is tinted and later premultiplied for a
    // Smithay texture upload.
    let mut pixels = pixmap.data().to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        if alpha == 0 {
            pixel[..3].fill(0);
        } else if alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    RgbaImage::from_raw(width, height, pixels)
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
        assert!(std::sync::Arc::ptr_eq(
            &settings,
            &nickel_application("Nickel Settings — Display").unwrap().1
        ));
        assert!(std::sync::Arc::ptr_eq(
            &file,
            &nickel_application("Nickel File — Home").unwrap().1
        ));
        assert!(std::sync::Arc::ptr_eq(
            &file,
            &nickel_application("nickel-file").unwrap().1
        ));
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

    #[test]
    fn svg_loader_exposes_straight_alpha_pixels() {
        let icon = load_svg_bytes(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#c86432" fill-opacity="0.5"/></svg>"##,
            2,
        )
        .unwrap();
        let [red, green, blue, alpha] = icon.get_pixel(0, 0).0;
        assert!((red as i16 - 200).abs() <= 2, "red={red}");
        assert!((green as i16 - 100).abs() <= 2, "green={green}");
        assert!((blue as i16 - 50).abs() <= 2, "blue={blue}");
        assert!((alpha as i16 - 128).abs() <= 1, "alpha={alpha}");
    }
}
