use crate::shell_layout::Geometry;
#[cfg(feature = "backend-udev")]
use smithay::backend::renderer::element::solid::SolidColorBuffer;
use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    utils::Transform,
};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

pub const TITLEBAR_HEIGHT: i32 = 40;
pub const RESIZE_BORDER: i32 = 5;
pub const BUTTON_WIDTH: i32 = 46;
pub const MINIMIZE_GLYPH: char = '\u{f2d1}';
pub const MAXIMIZE_GLYPH: char = '\u{f2d0}';
pub const RESTORE_GLYPH: char = '\u{f2d2}';
pub const CLOSE_GLYPH: char = '\u{f2d3}';

#[derive(Clone)]
#[cfg(feature = "backend-udev")]
pub struct ShadowLayer {
    pub buffer: SolidColorBuffer,
    pub offset: (i32, i32),
}

#[cfg(feature = "backend-udev")]
pub fn shadow_layers(width: i32, height: i32) -> Vec<ShadowLayer> {
    type ShadowCache = HashMap<(i32, i32), Vec<ShadowLayer>>;
    static CACHE: OnceLock<Mutex<ShadowCache>> = OnceLock::new();
    let key = (width.max(1), height.max(1));
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cache) = cache.lock()
        && let Some(layers) = cache.get(&key)
    {
        return layers.clone();
    }
    let layers = [(16, 7, 0.08), (10, 5, 0.12), (5, 3, 0.18)]
        .into_iter()
        .map(|(spread, vertical_offset, alpha)| ShadowLayer {
            buffer: SolidColorBuffer::new(
                (key.0 + spread * 2, key.1 + spread * 2),
                [0.0, 0.0, 0.0, alpha],
            ),
            offset: (-spread, -spread + vertical_offset),
        })
        .collect::<Vec<_>>();
    if let Ok(mut cache) = cache.lock() {
        if cache.len() >= 128 {
            cache.clear();
        }
        cache.insert(key, layers.clone());
    }
    layers
}

#[derive(Clone)]
pub struct FrameIcons {
    pub minimize: MemoryRenderBuffer,
    pub maximize: MemoryRenderBuffer,
    pub restore: MemoryRenderBuffer,
    pub close: MemoryRenderBuffer,
}

impl FrameIcons {
    pub fn load() -> Option<Self> {
        Some(Self {
            minimize: render_glyph(MINIMIZE_GLYPH)?,
            maximize: render_glyph(MAXIMIZE_GLYPH)?,
            restore: render_glyph(RESTORE_GLYPH)?,
            close: render_glyph(CLOSE_GLYPH)?,
        })
    }
}

fn render_glyph(glyph: char) -> Option<MemoryRenderBuffer> {
    const SIZE: u32 = 24;
    let mut options = resvg::usvg::Options::default();
    options
        .fontdb_mut()
        .load_font_file("/usr/share/fonts/opentype/font-awesome/FontAwesome.otf")
        .ok()?;
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{SIZE}" height="{SIZE}">
<text x="12" y="18" text-anchor="middle" font-family="FontAwesome" font-size="16" fill="white">{glyph}</text>
</svg>"#
    );
    let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &options).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(SIZE, SIZE)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    Some(MemoryRenderBuffer::from_slice(
        pixmap.data(),
        Fourcc::Abgr8888,
        (SIZE as i32, SIZE as i32),
        1,
        Transform::Normal,
        None,
    ))
}

pub fn render_titlebar(
    width: i32,
    title: &str,
    background: u32,
    foreground: u32,
) -> Option<MemoryRenderBuffer> {
    // Resizing can produce a new width for every pointer event. Quantizing the
    // raster width keeps the expensive SVG/text asset stable while the render
    // element scales it by only a few pixels.
    let width = ((width.max(1) + 31) / 32) * 32;
    type CacheKey = (i32, String, u32, u32);
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, MemoryRenderBuffer>>> = OnceLock::new();
    let key = (width, title.to_owned(), background, foreground);
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(buffer) = cache.lock().ok()?.get(&key).cloned() {
        return Some(buffer);
    }
    let buffer = render_titlebar_uncached(width, title, background, foreground)?;
    let mut cache = cache.lock().ok()?;
    if cache.len() >= 128 {
        cache.clear();
    }
    cache.insert(key, buffer.clone());
    Some(buffer)
}

fn render_titlebar_uncached(
    width: i32,
    title: &str,
    background: u32,
    foreground: u32,
) -> Option<MemoryRenderBuffer> {
    let width = u32::try_from(width).ok()?.max(1);
    let title_width = width.saturating_sub((BUTTON_WIDTH * 3 + 20) as u32);
    let escaped_title = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;");
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let right = width.saturating_sub(10);
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{TITLEBAR_HEIGHT}">
<defs><clipPath id="title"><rect x="16" y="0" width="{title_width}" height="{TITLEBAR_HEIGHT}"/></clipPath></defs>
<path d="M 10 0 H {right} Q {width} 0 {width} 10 V {TITLEBAR_HEIGHT} H 0 V 10 Q 0 0 10 0 Z" fill="#{background:06x}"/>
<text x="16" y="26" clip-path="url(#title)" font-family="sans-serif" font-size="14" font-weight="500" fill="#{foreground:06x}">{escaped_title}</text>
</svg>"##
    );
    let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &options).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, TITLEBAR_HEIGHT as u32)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    Some(MemoryRenderBuffer::from_slice(
        pixmap.data(),
        Fourcc::Abgr8888,
        (width as i32, TITLEBAR_HEIGHT),
        1,
        Transform::Normal,
        None,
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramePart {
    Titlebar,
    Minimize,
    Maximize,
    Close,
    ResizeNorth,
    ResizeNorthEast,
    ResizeEast,
    ResizeSouthEast,
    ResizeSouth,
    ResizeSouthWest,
    ResizeWest,
    ResizeNorthWest,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum FrameCursor {
    #[default]
    Arrow,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

impl FramePart {
    pub fn cursor(self) -> FrameCursor {
        match self {
            Self::ResizeNorth => FrameCursor::North,
            Self::ResizeNorthEast => FrameCursor::NorthEast,
            Self::ResizeEast => FrameCursor::East,
            Self::ResizeSouthEast => FrameCursor::SouthEast,
            Self::ResizeSouth => FrameCursor::South,
            Self::ResizeSouthWest => FrameCursor::SouthWest,
            Self::ResizeWest => FrameCursor::West,
            Self::ResizeNorthWest => FrameCursor::NorthWest,
            Self::Titlebar | Self::Minimize | Self::Maximize | Self::Close => FrameCursor::Arrow,
        }
    }
}

pub fn outer_geometry(content: Geometry) -> Geometry {
    Geometry {
        x: content.x - RESIZE_BORDER,
        y: content.y - TITLEBAR_HEIGHT - RESIZE_BORDER,
        width: content.width + RESIZE_BORDER * 2,
        height: content.height + TITLEBAR_HEIGHT + RESIZE_BORDER * 2,
    }
}

pub fn hit_test(content: Geometry, x: i32, y: i32) -> Option<FramePart> {
    let outer = outer_geometry(content);
    if x < outer.x || y < outer.y || x >= outer.x + outer.width || y >= outer.y + outer.height {
        return None;
    }
    let left = x < content.x;
    let right = x >= content.x + content.width;
    let top = y < outer.y + RESIZE_BORDER;
    let bottom = y >= content.y + content.height;
    match (left, right, top, bottom) {
        (true, false, true, false) => return Some(FramePart::ResizeNorthWest),
        (false, true, true, false) => return Some(FramePart::ResizeNorthEast),
        (true, false, false, true) => return Some(FramePart::ResizeSouthWest),
        (false, true, false, true) => return Some(FramePart::ResizeSouthEast),
        (true, false, false, false) => return Some(FramePart::ResizeWest),
        (false, true, false, false) => return Some(FramePart::ResizeEast),
        (false, false, true, false) => return Some(FramePart::ResizeNorth),
        (false, false, false, true) => return Some(FramePart::ResizeSouth),
        _ => {}
    }
    if y >= content.y {
        return None;
    }
    let from_right = content.x + content.width - x;
    Some(if from_right <= BUTTON_WIDTH {
        FramePart::Close
    } else if from_right <= BUTTON_WIDTH * 2 {
        FramePart::Maximize
    } else if from_right <= BUTTON_WIDTH * 3 {
        FramePart::Minimize
    } else {
        FramePart::Titlebar
    })
}

#[cfg(test)]
mod tests {
    use super::{FramePart, hit_test, outer_geometry};
    use crate::shell_layout::Geometry;

    const CONTENT: Geometry = Geometry {
        x: 100,
        y: 140,
        width: 500,
        height: 300,
    };

    #[test]
    fn frame_extends_outside_client_content() {
        assert_eq!(
            outer_geometry(CONTENT),
            Geometry {
                x: 95,
                y: 95,
                width: 510,
                height: 350
            }
        );
    }

    #[test]
    fn titlebar_buttons_have_stable_right_aligned_targets() {
        assert_eq!(hit_test(CONTENT, 590, 120), Some(FramePart::Close));
        assert_eq!(hit_test(CONTENT, 550, 120), Some(FramePart::Maximize));
        assert_eq!(hit_test(CONTENT, 500, 120), Some(FramePart::Minimize));
        assert_eq!(hit_test(CONTENT, 200, 120), Some(FramePart::Titlebar));
    }

    #[test]
    fn corners_win_over_edges() {
        assert_eq!(hit_test(CONTENT, 96, 96), Some(FramePart::ResizeNorthWest));
        assert_eq!(
            hit_test(CONTENT, 604, 444),
            Some(FramePart::ResizeSouthEast)
        );
    }

    #[test]
    fn resize_parts_select_directional_cursors() {
        assert_eq!(
            FramePart::ResizeNorthWest.cursor(),
            super::FrameCursor::NorthWest
        );
        assert_eq!(FramePart::ResizeEast.cursor(), super::FrameCursor::East);
        assert_eq!(FramePart::Titlebar.cursor(), super::FrameCursor::Arrow);
    }
}
