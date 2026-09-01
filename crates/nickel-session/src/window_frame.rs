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
const RECOVERY_PANEL_WIDTH: i32 = 560;
const RECOVERY_PANEL_HEIGHT: i32 = 144;
const TITLEBAR_CACHE_MAX_ENTRIES: usize = 128;
const TITLEBAR_CACHE_MAX_BYTES: usize = 16 * 1024 * 1024;

type TitlebarCacheKey = (i32, String, u32, u32);

#[derive(Clone)]
struct TitlebarCacheEntry {
    buffer: MemoryRenderBuffer,
}

#[derive(Default)]
struct TitlebarCache {
    entries: HashMap<TitlebarCacheKey, TitlebarCacheEntry>,
    live_bytes: usize,
    peak_bytes: usize,
    hits: u64,
    misses: u64,
    insertions: u64,
    evictions: u64,
    invalidations: u64,
}

static TITLEBAR_CACHE: OnceLock<Mutex<TitlebarCache>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TitlebarCacheDiagnostics {
    pub entries: usize,
    pub live_bytes: usize,
    pub peak_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub insertions: u64,
    pub evictions: u64,
    pub invalidations: u64,
}

pub fn titlebar_cache_diagnostics() -> TitlebarCacheDiagnostics {
    TITLEBAR_CACHE
        .get()
        .and_then(|cache| cache.lock().ok())
        .map_or_else(TitlebarCacheDiagnostics::default, |cache| {
            TitlebarCacheDiagnostics {
                entries: cache.entries.len(),
                live_bytes: cache.live_bytes,
                peak_bytes: cache.peak_bytes,
                hits: cache.hits,
                misses: cache.misses,
                insertions: cache.insertions,
                evictions: cache.evictions,
                invalidations: cache.invalidations,
            }
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryAction {
    Retry,
    Exit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryLayout {
    pub panel: Geometry,
    pub retry: Geometry,
    pub exit: Geometry,
}

pub fn recovery_layout(output: Geometry) -> RecoveryLayout {
    let width = output.width.clamp(1, RECOVERY_PANEL_WIDTH);
    let height = output.height.clamp(1, RECOVERY_PANEL_HEIGHT);
    let panel = Geometry {
        x: output.x + (output.width - width) / 2,
        y: output.y + (output.height - height) / 2,
        width,
        height,
    };
    let scaled = |x: i32, y: i32, width: i32, height: i32| Geometry {
        x: panel.x + x * panel.width / RECOVERY_PANEL_WIDTH,
        y: panel.y + y * panel.height / RECOVERY_PANEL_HEIGHT,
        width: (width * panel.width / RECOVERY_PANEL_WIDTH).max(1),
        height: (height * panel.height / RECOVERY_PANEL_HEIGHT).max(1),
    };
    RecoveryLayout {
        panel,
        retry: scaled(28, 94, 156, 30),
        exit: scaled(198, 94, 180, 30),
    }
}

pub fn recovery_action_at(output: Geometry, x: f64, y: f64) -> Option<RecoveryAction> {
    let contains = |geometry: Geometry| {
        x >= f64::from(geometry.x)
            && x < f64::from(geometry.x + geometry.width)
            && y >= f64::from(geometry.y)
            && y < f64::from(geometry.y + geometry.height)
    };
    let layout = recovery_layout(output);
    if contains(layout.retry) {
        Some(RecoveryAction::Retry)
    } else if contains(layout.exit) {
        Some(RecoveryAction::Exit)
    } else {
        None
    }
}

#[derive(Clone)]
#[cfg(feature = "backend-udev")]
pub struct ShadowLayer {
    pub buffer: SolidColorBuffer,
    pub offset: (i32, i32),
}

#[cfg(feature = "backend-udev")]
pub fn shadow_layers(width: i32, height: i32) -> Vec<ShadowLayer> {
    let key = (width.max(1), height.max(1));
    [(16, 7, 0.08), (10, 5, 0.12), (5, 3, 0.18)]
        .into_iter()
        .map(|(spread, vertical_offset, alpha)| ShadowLayer {
            buffer: SolidColorBuffer::new(
                (key.0 + spread * 2, key.1 + spread * 2),
                [0.0, 0.0, 0.0, alpha],
            ),
            offset: (-spread, -spread + vertical_offset),
        })
        .collect()
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

pub fn render_recovery_panel() -> Option<MemoryRenderBuffer> {
    static PANEL: OnceLock<Option<MemoryRenderBuffer>> = OnceLock::new();
    PANEL
        .get_or_init(render_recovery_panel_uncached)
        .as_ref()
        .cloned()
}

fn render_recovery_panel_uncached() -> Option<MemoryRenderBuffer> {
    const WIDTH: u32 = RECOVERY_PANEL_WIDTH as u32;
    const HEIGHT: u32 = RECOVERY_PANEL_HEIGHT as u32;
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}">
<rect width="{WIDTH}" height="{HEIGHT}" rx="14" fill="#24191c" stroke="#d05a68"/>
<text x="28" y="42" font-family="sans-serif" font-size="20" font-weight="600" fill="#fff4f5">Nickel shell needs attention</text>
<text x="28" y="72" font-family="sans-serif" font-size="14" fill="#e8c9cd">The compositor is still running and your applications are safe.</text>
<rect x="28" y="94" width="156" height="30" rx="7" fill="#9d3444"/>
<text x="45" y="114" font-family="sans-serif" font-size="13" font-weight="600" fill="white">Enter  Retry now</text>
<rect x="198" y="94" width="180" height="30" rx="7" fill="#37272b" stroke="#6b4b52"/>
<text x="212" y="114" font-family="sans-serif" font-size="13" fill="#e8c9cd">Esc  Log out safely</text>
</svg>"##
    );
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &options).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(WIDTH, HEIGHT)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    Some(MemoryRenderBuffer::from_slice(
        pixmap.data(),
        Fourcc::Abgr8888,
        (WIDTH as i32, HEIGHT as i32),
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
    let key = (width, title.to_owned(), background, foreground);
    let cache = TITLEBAR_CACHE.get_or_init(|| Mutex::new(TitlebarCache::default()));
    {
        let mut cache = cache.lock().ok()?;
        if let Some(buffer) = cache.entries.get(&key).map(|entry| entry.buffer.clone()) {
            cache.hits = cache.hits.saturating_add(1);
            return Some(buffer);
        }
        cache.misses = cache.misses.saturating_add(1);
    }
    let buffer = render_titlebar_uncached(width, title, background, foreground)?;
    let retained_bytes = usize::try_from(width)
        .unwrap_or(usize::MAX)
        .saturating_mul(TITLEBAR_HEIGHT as usize)
        .saturating_mul(4)
        .saturating_add(key.1.len());
    let mut cache = cache.lock().ok()?;
    if retained_bytes > TITLEBAR_CACHE_MAX_BYTES {
        return Some(buffer);
    }
    if cache.entries.len() >= TITLEBAR_CACHE_MAX_ENTRIES
        || cache.live_bytes.saturating_add(retained_bytes) > TITLEBAR_CACHE_MAX_BYTES
    {
        cache.evictions = cache.evictions.saturating_add(cache.entries.len() as u64);
        cache.invalidations = cache.invalidations.saturating_add(1);
        cache.entries.clear();
        cache.live_bytes = 0;
    }
    cache.entries.insert(
        key,
        TitlebarCacheEntry {
            buffer: buffer.clone(),
        },
    );
    cache.live_bytes = cache.live_bytes.saturating_add(retained_bytes);
    cache.peak_bytes = cache.peak_bytes.max(cache.live_bytes);
    cache.insertions = cache.insertions.saturating_add(1);
    drop(cache);
    let diagnostics = titlebar_cache_diagnostics();
    tracing::trace!(
        entries = diagnostics.entries,
        live_bytes = diagnostics.live_bytes,
        peak_bytes = diagnostics.peak_bytes,
        hits = diagnostics.hits,
        misses = diagnostics.misses,
        insertions = diagnostics.insertions,
        evictions = diagnostics.evictions,
        invalidations = diagnostics.invalidations,
        "server-decoration titlebar cache updated"
    );
    Some(buffer)
}

fn render_titlebar_uncached(
    width: i32,
    title: &str,
    background: u32,
    foreground: u32,
) -> Option<MemoryRenderBuffer> {
    let (pixels, width) = render_titlebar_pixels(width, title, background, foreground)?;
    Some(MemoryRenderBuffer::from_slice(
        &pixels,
        Fourcc::Abgr8888,
        (width as i32, TITLEBAR_HEIGHT),
        1,
        Transform::Normal,
        None,
    ))
}

fn render_titlebar_pixels(
    width: i32,
    title: &str,
    background: u32,
    foreground: u32,
) -> Option<(Vec<u8>, u32)> {
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
<path d="M 10 .5 H {right} Q {width_minus_half} .5 {width_minus_half} 10 V {TITLEBAR_HEIGHT} H .5 V 10 Q .5 .5 10 .5 Z" fill="#{background:06x}" stroke="#{foreground:06x}" stroke-opacity=".38"/>
<text x="16" y="26" clip-path="url(#title)" font-family="sans-serif" font-size="14" font-weight="500" fill="#{foreground:06x}">{escaped_title}</text>
</svg>"##,
        width_minus_half = width as f32 - 0.5
    );
    let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &options).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, TITLEBAR_HEIGHT as u32)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    Some((pixmap.data().to_vec(), width))
}

pub fn titlebar_geometry(content: Geometry) -> Geometry {
    Geometry {
        x: content.x,
        y: content.y - TITLEBAR_HEIGHT,
        width: content.width,
        height: TITLEBAR_HEIGHT,
    }
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

pub fn topmost_frame_target<T>(
    candidates: impl IntoIterator<Item = (T, bool, Option<FramePart>)>,
) -> Option<(T, FramePart)> {
    for (target, surface_accepts_input, frame_part) in candidates {
        if surface_accepts_input {
            return None;
        }
        if let Some(frame_part) = frame_part {
            return Some((target, frame_part));
        }
    }
    None
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
    use super::{
        FramePart, RESIZE_BORDER, RecoveryAction, TITLEBAR_CACHE, TITLEBAR_CACHE_MAX_BYTES,
        TITLEBAR_CACHE_MAX_ENTRIES, TITLEBAR_HEIGHT, hit_test, outer_geometry, recovery_action_at,
        recovery_layout, render_recovery_panel, render_titlebar, render_titlebar_pixels,
        titlebar_cache_diagnostics, titlebar_geometry, topmost_frame_target,
    };
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
    fn titlebar_visual_geometry_has_no_resize_hit_region_overhang() {
        let titlebar = titlebar_geometry(CONTENT);
        let outer = outer_geometry(CONTENT);
        assert_eq!(titlebar.x, CONTENT.x);
        assert_eq!(titlebar.y, CONTENT.y - TITLEBAR_HEIGHT);
        assert_eq!(titlebar.width, CONTENT.width);
        assert_eq!(titlebar.height, TITLEBAR_HEIGHT);
        assert_eq!(titlebar.y + titlebar.height, CONTENT.y);
        assert_eq!(outer.x, titlebar.x - RESIZE_BORDER);
        assert_eq!(outer.width, titlebar.width + RESIZE_BORDER * 2);
    }

    #[test]
    fn titlebar_raster_has_rounded_alpha_corners_and_continuous_top_border() {
        let (pixels, width) = render_titlebar_pixels(320, "Nickel File", 0x20242c, 0xe8edf4)
            .expect("titlebar raster");
        let alpha = |x: u32, y: u32| pixels[((y * width + x) * 4 + 3) as usize];
        assert_eq!(alpha(0, 0), 0, "left outer corner must remain transparent");
        assert_eq!(
            alpha(width - 1, 0),
            0,
            "right outer corner must remain transparent"
        );
        assert!(
            (10..width - 10).all(|x| alpha(x, 0) > 0),
            "outer top border must have no transparent gap"
        );
        assert!(alpha(0, 10) > 0 && alpha(width - 1, 10) > 0);
    }

    #[test]
    fn titlebar_cache_is_byte_bounded_and_reports_churn() {
        if let Some(cache) = TITLEBAR_CACHE.get() {
            *cache.lock().unwrap() = super::TitlebarCache::default();
        }
        for index in 0..(TITLEBAR_CACHE_MAX_ENTRIES + 16) {
            render_titlebar(4096, &format!("Window {index}"), 0x20242c, 0xe8edf4)
                .expect("titlebar raster");
        }
        render_titlebar(4096, "Window 143", 0x20242c, 0xe8edf4).expect("cached titlebar raster");

        let diagnostics = titlebar_cache_diagnostics();
        assert!(diagnostics.entries <= TITLEBAR_CACHE_MAX_ENTRIES);
        assert!(diagnostics.live_bytes <= TITLEBAR_CACHE_MAX_BYTES);
        assert!(diagnostics.peak_bytes <= TITLEBAR_CACHE_MAX_BYTES);
        assert_eq!(diagnostics.misses, (TITLEBAR_CACHE_MAX_ENTRIES + 16) as u64);
        assert_eq!(diagnostics.hits, 1);
        assert_eq!(diagnostics.insertions, diagnostics.misses);
        assert!(diagnostics.evictions > 0);
        assert!(diagnostics.invalidations > 0);
    }

    #[test]
    fn titlebar_buttons_have_stable_right_aligned_targets() {
        assert_eq!(hit_test(CONTENT, 590, 120), Some(FramePart::Close));
        assert_eq!(hit_test(CONTENT, 550, 120), Some(FramePart::Maximize));
        assert_eq!(hit_test(CONTENT, 500, 120), Some(FramePart::Minimize));
        assert_eq!(hit_test(CONTENT, 200, 120), Some(FramePart::Titlebar));
    }

    #[test]
    fn visible_surface_blocks_a_lower_window_frame() {
        let candidates = [
            ("foreground", true, None),
            ("background", false, Some(FramePart::ResizeSouth)),
        ];

        assert_eq!(topmost_frame_target(candidates), None);
    }

    #[test]
    fn input_region_pass_through_reaches_an_exposed_lower_frame() {
        let candidates = [
            ("foreground", false, None),
            ("background", false, Some(FramePart::ResizeSouth)),
        ];

        assert_eq!(
            topmost_frame_target(candidates),
            Some(("background", FramePart::ResizeSouth))
        );
    }

    #[test]
    fn topmost_exposed_frame_wins_over_lower_surfaces() {
        let candidates = [
            ("foreground", false, Some(FramePart::Titlebar)),
            ("background", true, None),
        ];

        assert_eq!(
            topmost_frame_target(candidates),
            Some(("foreground", FramePart::Titlebar))
        );
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

    #[test]
    fn compositor_recovery_panel_rasterizes_without_the_shell() {
        assert!(render_recovery_panel().is_some());
    }

    #[test]
    fn recovery_actions_use_the_renderers_production_layout() {
        let output = Geometry {
            x: 100,
            y: 40,
            width: 320,
            height: 120,
        };
        let layout = recovery_layout(output);
        let center = |geometry: Geometry| {
            (
                f64::from(geometry.x) + f64::from(geometry.width) / 2.0,
                f64::from(geometry.y) + f64::from(geometry.height) / 2.0,
            )
        };
        let retry = center(layout.retry);
        let exit = center(layout.exit);
        assert_eq!(
            recovery_action_at(output, retry.0, retry.1),
            Some(RecoveryAction::Retry)
        );
        assert_eq!(
            recovery_action_at(output, exit.0, exit.1),
            Some(RecoveryAction::Exit)
        );
        assert_eq!(
            recovery_action_at(output, output.x.into(), output.y.into()),
            None
        );
    }
}
