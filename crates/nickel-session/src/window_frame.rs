use crate::shell_layout::Geometry;
#[cfg(feature = "backend-udev")]
use smithay::backend::renderer::element::solid::SolidColorBuffer;
use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    utils::Transform,
};
use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

pub const TITLEBAR_HEIGHT: i32 = 40;
pub const RESIZE_BORDER: i32 = 5;
pub const BUTTON_WIDTH: i32 = 46;
pub const MINIMIZE_GLYPH: char = '\u{f2d1}';
pub const MAXIMIZE_GLYPH: char = '\u{f2d0}';
pub const RESTORE_GLYPH: char = '\u{f2d2}';
pub const CLOSE_GLYPH: char = '\u{f2d3}';
const TITLEBAR_CACHE_MAX_ENTRIES: usize = 16;
const TITLEBAR_CACHE_MAX_BYTES: usize = 512 * 1024;

#[derive(Clone)]
struct TitlebarCacheEntry {
    owner: Option<u64>,
    width: i32,
    title: String,
    background: u32,
    foreground: u32,
    buffer: MemoryRenderBuffer,
    bytes: usize,
}

#[derive(Default)]
struct TitlebarCache {
    entries: VecDeque<TitlebarCacheEntry>,
    live_bytes: usize,
    peak_bytes: usize,
    hits: u64,
    misses: u64,
    insertions: u64,
    evictions: u64,
    invalidations: u64,
    rasterizations: u64,
    avoided_rasterizations: u64,
    generation: u64,
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
    pub rasterizations: u64,
    pub avoided_rasterizations: u64,
    pub generation: u64,
    pub font_database_loads: u64,
    /// Smithay owns renderer imports; their byte cost is not exposed by its API.
    pub renderer_bytes: Option<usize>,
}

/// Selects whether server-decoration rasters may be retained and reused.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TitlebarCacheMode {
    #[default]
    Enabled,
    Bypass,
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
                rasterizations: cache.rasterizations,
                avoided_rasterizations: cache.avoided_rasterizations,
                generation: cache.generation,
                font_database_loads: TITLEBAR_FONT_DATABASE_LOADS.load(Ordering::Relaxed),
                renderer_bytes: None,
            }
        })
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

#[cfg(test)]
pub fn render_titlebar(
    width: i32,
    title: &str,
    background: u32,
    foreground: u32,
) -> Option<MemoryRenderBuffer> {
    render_titlebar_for(None, width, title, background, foreground)
}

pub fn render_titlebar_for(
    owner: Option<u64>,
    width: i32,
    title: &str,
    background: u32,
    foreground: u32,
) -> Option<MemoryRenderBuffer> {
    render_titlebar_with_mode(
        owner,
        width,
        title,
        background,
        foreground,
        TitlebarCacheMode::Enabled,
    )
}

pub fn render_titlebar_with_mode(
    owner: Option<u64>,
    width: i32,
    title: &str,
    background: u32,
    foreground: u32,
    mode: TitlebarCacheMode,
) -> Option<MemoryRenderBuffer> {
    // The compositor uses this buffer as a viewport rather than scaling it.
    // Retain the exact width so the right rounded corner is never cropped.
    let width = width.max(1);
    if mode == TitlebarCacheMode::Bypass {
        return render_titlebar_uncached(width, title, background, foreground);
    }
    let cache = TITLEBAR_CACHE.get_or_init(|| Mutex::new(TitlebarCache::default()));
    {
        let mut cache = cache.lock().ok()?;
        if let Some(index) = cache.entries.iter().position(|entry| {
            entry.width == width
                && entry.owner == owner
                && entry.title == title
                && entry.background == background
                && entry.foreground == foreground
        }) {
            let entry = cache.entries.remove(index)?;
            let buffer = entry.buffer.clone();
            cache.entries.push_back(entry);
            cache.hits = cache.hits.saturating_add(1);
            cache.avoided_rasterizations = cache.avoided_rasterizations.saturating_add(1);
            return Some(buffer);
        }
        cache.misses = cache.misses.saturating_add(1);
    }
    let buffer = render_titlebar_uncached(width, title, background, foreground)?;
    let title = title.to_owned();
    let retained_bytes = usize::try_from(width)
        .unwrap_or(usize::MAX)
        .saturating_mul(TITLEBAR_HEIGHT as usize)
        .saturating_mul(4)
        .saturating_add(title.len());
    let mut cache = cache.lock().ok()?;
    cache.rasterizations = cache.rasterizations.saturating_add(1);
    if retained_bytes > TITLEBAR_CACHE_MAX_BYTES {
        return Some(buffer);
    }
    if let Some(owner) = owner {
        let mut index = 0;
        while index < cache.entries.len() {
            if cache.entries[index].owner == Some(owner) {
                let obsolete = cache.entries.remove(index)?;
                cache.live_bytes = cache.live_bytes.saturating_sub(obsolete.bytes);
                cache.evictions = cache.evictions.saturating_add(1);
                cache.invalidations = cache.invalidations.saturating_add(1);
                cache.generation = cache.generation.saturating_add(1);
            } else {
                index += 1;
            }
        }
    }
    while cache.entries.len() >= TITLEBAR_CACHE_MAX_ENTRIES
        || cache.live_bytes.saturating_add(retained_bytes) > TITLEBAR_CACHE_MAX_BYTES
    {
        let Some(evicted) = cache.entries.pop_front() else {
            break;
        };
        cache.live_bytes = cache.live_bytes.saturating_sub(evicted.bytes);
        cache.evictions = cache.evictions.saturating_add(1);
        cache.invalidations = cache.invalidations.saturating_add(1);
        cache.generation = cache.generation.saturating_add(1);
    }
    cache.entries.push_back(TitlebarCacheEntry {
        owner,
        width,
        title,
        background,
        foreground,
        buffer: buffer.clone(),
        bytes: retained_bytes,
    });
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

pub fn retain_titlebars_for_windows(owners: impl IntoIterator<Item = u64>) {
    let owners = owners.into_iter().collect::<HashSet<_>>();
    let Some(cache) = TITLEBAR_CACHE.get() else {
        return;
    };
    let Ok(mut cache) = cache.lock() else {
        return;
    };
    let mut index = 0;
    while index < cache.entries.len() {
        let retire = cache.entries[index]
            .owner
            .is_some_and(|owner| !owners.contains(&owner));
        if retire {
            let obsolete = cache.entries.remove(index).expect("known titlebar index");
            cache.live_bytes = cache.live_bytes.saturating_sub(obsolete.bytes);
            cache.evictions = cache.evictions.saturating_add(1);
            cache.invalidations = cache.invalidations.saturating_add(1);
            cache.generation = cache.generation.saturating_add(1);
        } else {
            index += 1;
        }
    }
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
    static OPTIONS: OnceLock<Mutex<resvg::usvg::Options<'static>>> = OnceLock::new();
    let options = OPTIONS.get_or_init(|| {
        let mut options = resvg::usvg::Options::default();
        options.fontdb_mut().load_system_fonts();
        TITLEBAR_FONT_DATABASE_LOADS.fetch_add(1, Ordering::Relaxed);
        Mutex::new(options)
    });
    let right = width.saturating_sub(10);
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{TITLEBAR_HEIGHT}">
<defs><clipPath id="title"><rect x="16" y="0" width="{title_width}" height="{TITLEBAR_HEIGHT}"/></clipPath></defs>
<path d="M 10 0 H {right} Q {width} 0 {width} 10 V {TITLEBAR_HEIGHT} H 0 V 10 Q 0 0 10 0 Z" fill="#{background:06x}"/>
<text x="16" y="26" clip-path="url(#title)" font-family="sans-serif" font-size="14" font-weight="500" fill="#{foreground:06x}">{escaped_title}</text>
</svg>"##,
    );
    let options = options.lock().ok()?;
    let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &options).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, TITLEBAR_HEIGHT as u32)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    Some((pixmap.data().to_vec(), width))
}

static TITLEBAR_FONT_DATABASE_LOADS: AtomicU64 = AtomicU64::new(0);

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
    use std::sync::{Mutex, MutexGuard};

    use super::{
        FramePart, RESIZE_BORDER, TITLEBAR_CACHE, TITLEBAR_CACHE_MAX_BYTES,
        TITLEBAR_CACHE_MAX_ENTRIES, TITLEBAR_HEIGHT, TitlebarCacheMode, hit_test, outer_geometry,
        render_titlebar, render_titlebar_for, render_titlebar_pixels, render_titlebar_with_mode,
        retain_titlebars_for_windows, titlebar_cache_diagnostics, titlebar_geometry,
        topmost_frame_target,
    };
    use crate::shell_layout::Geometry;

    const CONTENT: Geometry = Geometry {
        x: 100,
        y: 140,
        width: 500,
        height: 300,
    };

    fn cache_test_lock() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

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
    fn titlebar_raster_has_symmetric_rounded_alpha_corners_without_an_outline() {
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
        let rgba = |x: u32, y: u32| {
            let offset = ((y * width + x) * 4) as usize;
            &pixels[offset..offset + 4]
        };
        assert_eq!(rgba(width / 2, 1), rgba(width / 2, 2));
    }

    #[test]
    fn titlebar_cache_is_byte_bounded_and_reports_churn() {
        let _test_lock = cache_test_lock();
        if let Some(cache) = TITLEBAR_CACHE.get() {
            *cache.lock().unwrap() = super::TitlebarCache::default();
        }
        for index in 0..(TITLEBAR_CACHE_MAX_ENTRIES + 16) {
            render_titlebar(1024, &format!("Window {index}"), 0x20242c, 0xe8edf4)
                .expect("titlebar raster");
        }
        render_titlebar(1024, "Window 31", 0x20242c, 0xe8edf4).expect("cached titlebar raster");

        let diagnostics = titlebar_cache_diagnostics();
        assert!(diagnostics.entries <= TITLEBAR_CACHE_MAX_ENTRIES);
        assert!(diagnostics.live_bytes <= TITLEBAR_CACHE_MAX_BYTES);
        assert!(diagnostics.peak_bytes <= TITLEBAR_CACHE_MAX_BYTES);
        assert_eq!(diagnostics.misses, (TITLEBAR_CACHE_MAX_ENTRIES + 16) as u64);
        assert_eq!(diagnostics.hits, 1);
        assert_eq!(diagnostics.insertions, diagnostics.misses);
        assert!(diagnostics.evictions > 0);
        assert!(diagnostics.invalidations > 0);
        assert_eq!(diagnostics.rasterizations, diagnostics.misses);
        assert_eq!(diagnostics.avoided_rasterizations, diagnostics.hits);
    }

    #[test]
    fn titlebar_owner_changes_and_closed_windows_retire_obsolete_rasters() {
        let _test_lock = cache_test_lock();
        if let Some(cache) = TITLEBAR_CACHE.get() {
            *cache.lock().unwrap() = super::TitlebarCache::default();
        }
        render_titlebar_for(Some(7), 640, "Before", 0x20242c, 0xe8edf4).unwrap();
        render_titlebar_for(Some(7), 640, "After", 0x20242c, 0xe8edf4).unwrap();
        render_titlebar_for(Some(9), 640, "Other", 0x20242c, 0xe8edf4).unwrap();
        let renamed = titlebar_cache_diagnostics();
        assert_eq!(renamed.entries, 2);
        assert!(renamed.invalidations >= 1);

        retain_titlebars_for_windows([9]);
        let closed = titlebar_cache_diagnostics();
        assert_eq!(closed.entries, 1);
        assert!(closed.invalidations >= 2);
    }

    #[derive(Clone, Copy)]
    struct AdmissionStats {
        median_us: f64,
        p95_us: f64,
    }

    fn admission_stats(mut samples: Vec<f64>) -> AdmissionStats {
        samples.sort_by(f64::total_cmp);
        let nearest_rank =
            |percent: usize| samples[(samples.len() * percent).div_ceil(100).saturating_sub(1)];
        AdmissionStats {
            median_us: nearest_rank(50),
            p95_us: nearest_rank(95),
        }
    }

    #[test]
    #[ignore = "release-mode cache admission benchmark"]
    fn window_titlebar_rasters_admission_workloads() {
        let _test_lock = cache_test_lock();
        const SAMPLES: usize = 31;
        const WARM_P95_BENEFIT_US: f64 = 100.0;
        if let Some(cache) = TITLEBAR_CACHE.get() {
            *cache.lock().unwrap() = super::TitlebarCache::default();
        }
        render_titlebar(1280, "Nickel File", 0x20242c, 0xe8edf4).expect("warm titlebar");

        for workload in ["cold", "warm", "churn", "low_reuse"] {
            let mut cached_samples = Vec::with_capacity(SAMPLES);
            let mut bypass_samples = Vec::with_capacity(SAMPLES);
            for sample in 0..SAMPLES {
                if workload == "cold"
                    && let Some(cache) = TITLEBAR_CACHE.get()
                {
                    *cache.lock().unwrap() = super::TitlebarCache::default();
                }
                let title = match workload {
                    "churn" => format!("Nickel workspace {}", sample % 4),
                    "low_reuse" => format!("Nickel document {sample}"),
                    _ => "Nickel File".to_owned(),
                };
                let started = std::time::Instant::now();
                let cached = render_titlebar_with_mode(
                    None,
                    1280,
                    &title,
                    0x20242c,
                    0xe8edf4,
                    TitlebarCacheMode::Enabled,
                )
                .expect("cached titlebar");
                std::hint::black_box(cached);
                cached_samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);

                let started = std::time::Instant::now();
                let bypass = render_titlebar_with_mode(
                    None,
                    1280,
                    &title,
                    0x20242c,
                    0xe8edf4,
                    TitlebarCacheMode::Bypass,
                )
                .expect("bypassed titlebar");
                std::hint::black_box(bypass);
                bypass_samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);

                let cached_pixels = render_titlebar_pixels(1280, &title, 0x20242c, 0xe8edf4)
                    .expect("cached-path authoritative pixels");
                let bypass_pixels = render_titlebar_pixels(1280, &title, 0x20242c, 0xe8edf4)
                    .expect("bypass authoritative pixels");
                assert_eq!(cached_pixels, bypass_pixels);
            }
            let cached = admission_stats(cached_samples);
            let bypass = admission_stats(bypass_samples);
            let retained_bytes = titlebar_cache_diagnostics().live_bytes;
            println!(
                "{{\"schema\":\"nickel-cache-admission-v1\",\"cache\":\"window_titlebar_rasters\",\"workload\":\"{workload}\",\"fixture\":\"deterministic_headless\",\"profile\":\"release\",\"samples\":{SAMPLES},\"cached_median_us\":{:.3},\"cached_p95_us\":{:.3},\"bypass_median_us\":{:.3},\"bypass_p95_us\":{:.3},\"retained_bytes\":{retained_bytes},\"complexity\":{{\"key_fields\":4,\"invalidation_triggers\":3,\"storage_collections\":1}},\"output_equivalence\":\"exact_rgba\"}}",
                cached.median_us, cached.p95_us, bypass.median_us, bypass.p95_us
            );
            if workload == "warm" {
                assert!(
                    bypass.p95_us - cached.p95_us > WARM_P95_BENEFIT_US,
                    "predeclared warm p95 benefit was not met"
                );
            }
        }
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
}
