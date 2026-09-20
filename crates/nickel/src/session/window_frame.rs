use crate::session::shell_layout::Geometry;
#[cfg(test)]
use sha2::{Digest, Sha256};
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowFrameMetrics {
    pub titlebar_height: i32,
    pub visual_border: i32,
    pub resize_margin: i32,
    pub corner_extent: i32,
    pub corner_radius: i32,
    pub shadow_extent: i32,
}

pub const FRAME_METRICS: WindowFrameMetrics = WindowFrameMetrics {
    titlebar_height: 40,
    visual_border: 1,
    resize_margin: 5,
    corner_extent: 14,
    corner_radius: 10,
    shadow_extent: 16,
};

pub const TITLEBAR_HEIGHT: i32 = FRAME_METRICS.titlebar_height;
pub const RESIZE_BORDER: i32 = FRAME_METRICS.resize_margin;
pub const BUTTON_WIDTH: i32 = 46;
// One current raster per ordinary window fits comfortably: 16 MiB holds more
// than fifty 1920 px titlebars or twenty-five 4K titlebars. Owner retirement
// prevents that budget from becoming historical title storage.
const TITLEBAR_CACHE_MAX_ENTRIES: usize = 64;
const TITLEBAR_CACHE_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Nickel-owned window shadow tint. Keep distinct from terminal/content color.
const WINDOW_SHADOW_RGB: [f32; 3] = [0.035, 0.043, 0.055];

#[derive(Clone)]
struct TitlebarCacheEntry {
    owner: Option<u64>,
    width: i32,
    title: String,
    background: u32,
    foreground: u32,
    border: u32,
    buffer: MemoryRenderBuffer,
    bytes: usize,
    #[cfg(test)]
    pixel_digest: [u8; 32],
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
static SHADOW_ASSETS: OnceLock<[FrameShadowAssets; 2]> = OnceLock::new();

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
pub struct FrameSolidLayer {
    pub buffer: SolidColorBuffer,
    pub offset: (i32, i32),
}

#[derive(Clone)]
pub struct FrameImageLayer {
    pub buffer: MemoryRenderBuffer,
    pub offset: (i32, i32),
    pub size: (i32, i32),
}

pub struct FrameShadowLayers {
    pub images: Vec<FrameImageLayer>,
}

struct FrameShadowAssets {
    top: MemoryRenderBuffer,
    bottom: MemoryRenderBuffer,
    left: MemoryRenderBuffer,
    right: MemoryRenderBuffer,
    top_left: MemoryRenderBuffer,
    top_right: MemoryRenderBuffer,
    bottom_left: MemoryRenderBuffer,
    bottom_right: MemoryRenderBuffer,
}

pub fn frame_border_color(background: u32, foreground: u32, active: bool) -> u32 {
    let weight = if active { 0.28 } else { 0.16 };
    let channel = |shift| {
        let background = ((background >> shift) & 0xff_u32) as f32;
        let foreground = ((foreground >> shift) & 0xff_u32) as f32;
        (background + (foreground - background) * weight).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn solid_color(color: u32, alpha: f32) -> [f32; 4] {
    [
        ((color >> 16) & 0xff) as f32 / 255.0,
        ((color >> 8) & 0xff) as f32 / 255.0,
        (color & 0xff) as f32 / 255.0,
        alpha,
    ]
}

pub fn content_border_layers(width: i32, height: i32, color: u32) -> Vec<FrameSolidLayer> {
    let width = width.max(1);
    let height = height.max(1);
    let border = FRAME_METRICS.visual_border.max(1);
    vec![
        FrameSolidLayer {
            buffer: SolidColorBuffer::new((border, height), solid_color(color, 1.0)),
            offset: (0, 0),
        },
        FrameSolidLayer {
            buffer: SolidColorBuffer::new((border, height), solid_color(color, 1.0)),
            offset: (width - border, 0),
        },
        FrameSolidLayer {
            buffer: SolidColorBuffer::new((width, border), solid_color(color, 1.0)),
            offset: (0, height - border),
        },
    ]
}

/// Border for client-decorated windows. Their own title chrome supplies no
/// compositor-owned top edge, so retain all four sides without adding a second
/// titlebar.
pub fn client_border_layers(width: i32, height: i32, color: u32) -> Vec<FrameSolidLayer> {
    let width = width.max(1);
    let border = FRAME_METRICS.visual_border.max(1);
    let mut layers = content_border_layers(width, height, color);
    layers.push(FrameSolidLayer {
        buffer: SolidColorBuffer::new((width, border), solid_color(color, 1.0)),
        offset: (0, 0),
    });
    layers
}

pub fn shadow_layers(width: i32, height: i32, active: bool) -> FrameShadowLayers {
    let extent = FRAME_METRICS.shadow_extent;
    let width = width.max(1);
    let height = height.max(1);
    let radius = FRAME_METRICS.corner_radius.min(width / 2).max(1);
    let assets = &SHADOW_ASSETS.get_or_init(|| [shadow_assets(false), shadow_assets(true)])
        [usize::from(active)];
    FrameShadowLayers {
        images: vec![
            FrameImageLayer {
                buffer: assets.top.clone(),
                offset: (radius, -extent),
                size: ((width - radius * 2).max(1), extent),
            },
            FrameImageLayer {
                buffer: assets.bottom.clone(),
                offset: (0, height),
                size: (width, extent),
            },
            FrameImageLayer {
                buffer: assets.left.clone(),
                offset: (-extent, radius),
                size: (extent, (height - radius).max(1)),
            },
            FrameImageLayer {
                buffer: assets.right.clone(),
                offset: (width, radius),
                size: (extent, (height - radius).max(1)),
            },
            FrameImageLayer {
                buffer: assets.top_left.clone(),
                offset: (-extent, -extent),
                size: (extent + radius, extent + radius),
            },
            FrameImageLayer {
                buffer: assets.top_right.clone(),
                offset: (width - radius, -extent),
                size: (extent + radius, extent + radius),
            },
            FrameImageLayer {
                buffer: assets.bottom_left.clone(),
                offset: (-extent, height),
                size: (extent, extent),
            },
            FrameImageLayer {
                buffer: assets.bottom_right.clone(),
                offset: (width, height),
                size: (extent, extent),
            },
        ],
    }
}

fn shadow_assets(active: bool) -> FrameShadowAssets {
    let extent = FRAME_METRICS.shadow_extent;
    let radius = FRAME_METRICS.corner_radius;
    let raster = |width, height, distance: &dyn Fn(f32, f32) -> f32| {
        let mut pixels = vec![0_u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let distance = distance(x as f32 + 0.5, y as f32 + 0.5).max(0.0);
                let falloff = (1.0 - distance / extent as f32).clamp(0.0, 1.0);
                let alpha = falloff * falloff * if active { 0.20 } else { 0.13 };
                let offset = ((y * width + x) * 4) as usize;
                pixels[offset] = (WINDOW_SHADOW_RGB[0] * alpha * 255.0).round() as u8;
                pixels[offset + 1] = (WINDOW_SHADOW_RGB[1] * alpha * 255.0).round() as u8;
                pixels[offset + 2] = (WINDOW_SHADOW_RGB[2] * alpha * 255.0).round() as u8;
                pixels[offset + 3] = (alpha * 255.0).round() as u8;
            }
        }
        MemoryRenderBuffer::from_slice(
            &pixels,
            Fourcc::Abgr8888,
            (width, height),
            1,
            Transform::Normal,
            None,
        )
    };
    let corner = extent + radius;
    FrameShadowAssets {
        top: raster(1, extent, &|_, y| (extent as f32 - y).max(0.0)),
        bottom: raster(1, extent, &|_, y| y),
        left: raster(extent, 1, &|x, _| (extent as f32 - x).max(0.0)),
        right: raster(extent, 1, &|x, _| x),
        top_left: raster(corner, corner, &|x, y| {
            ((x - corner as f32).powi(2) + (y - corner as f32).powi(2)).sqrt() - radius as f32
        }),
        top_right: raster(corner, corner, &|x, y| {
            (x.powi(2) + (y - corner as f32).powi(2)).sqrt() - radius as f32
        }),
        bottom_left: raster(extent, extent, &|x, y| {
            ((extent as f32 - x).powi(2) + y.powi(2)).sqrt()
        }),
        bottom_right: raster(extent, extent, &|x, y| (x.powi(2) + y.powi(2)).sqrt()),
    }
}

#[derive(Clone)]
pub struct FrameIcons {
    pub minimize: MemoryRenderBuffer,
    pub maximize: MemoryRenderBuffer,
    pub restore: MemoryRenderBuffer,
    pub close: MemoryRenderBuffer,
}

/// Places the 24 logical-pixel control glyph canvas on the titlebar's optical
/// baseline. The text baseline is raised by one pixel as well, so pure
/// mathematical centering (`+ 8` in a 40-pixel bar) looks subtly low beside it.
pub const fn frame_icon_y(titlebar_y: i32) -> i32 {
    titlebar_y + 7
}

impl FrameIcons {
    pub fn load() -> Option<Self> {
        Some(Self {
            minimize: render_frame_icon(FrameIcon::Minimize)?,
            maximize: render_frame_icon(FrameIcon::Maximize)?,
            restore: render_frame_icon(FrameIcon::Restore)?,
            close: render_frame_icon(FrameIcon::Close)?,
        })
    }
}

#[derive(Clone, Copy)]
enum FrameIcon {
    Minimize,
    Maximize,
    Restore,
    Close,
}

fn render_frame_icon(icon: FrameIcon) -> Option<MemoryRenderBuffer> {
    const SIZE: u32 = 24;
    let mut pixels = vec![0_u8; (SIZE * SIZE * 4) as usize];
    let mut rect = |x: u32, y: u32, width: u32, height: u32| {
        for row in y..(y + height).min(SIZE) {
            for column in x..(x + width).min(SIZE) {
                let offset = ((row * SIZE + column) * 4) as usize;
                pixels[offset..offset + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    };
    let mut outline = |x: u32, y: u32, width: u32, height: u32| {
        rect(x, y, width, 2);
        rect(x, y + height.saturating_sub(2), width, 2);
        rect(x, y, 2, height);
        rect(x + width.saturating_sub(2), y, 2, height);
    };
    match icon {
        FrameIcon::Minimize => rect(6, 11, 12, 2),
        FrameIcon::Maximize => outline(6, 6, 12, 12),
        FrameIcon::Restore => {
            outline(8, 5, 11, 11);
            rect(5, 8, 2, 11);
            rect(5, 17, 11, 2);
            rect(14, 10, 2, 9);
            rect(7, 8, 9, 2);
        }
        FrameIcon::Close => {
            for step in 0..10 {
                rect(7 + step, 7 + step, 2, 2);
                rect(16 - step, 7 + step, 2, 2);
            }
        }
    }
    Some(MemoryRenderBuffer::from_slice(
        &pixels,
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

#[cfg(test)]
pub fn render_titlebar_for(
    owner: Option<u64>,
    width: i32,
    title: &str,
    background: u32,
    foreground: u32,
) -> Option<MemoryRenderBuffer> {
    render_titlebar_for_state(owner, width, title, background, foreground, true)
}

pub fn render_titlebar_for_state(
    owner: Option<u64>,
    width: i32,
    title: &str,
    background: u32,
    foreground: u32,
    active: bool,
) -> Option<MemoryRenderBuffer> {
    render_titlebar_with_mode(
        owner,
        width,
        title,
        background,
        foreground,
        frame_border_color(background, foreground, active),
        TitlebarCacheMode::Enabled,
    )
}

pub fn render_titlebar_with_mode(
    owner: Option<u64>,
    width: i32,
    title: &str,
    background: u32,
    foreground: u32,
    border: u32,
    mode: TitlebarCacheMode,
) -> Option<MemoryRenderBuffer> {
    // The compositor uses this buffer as a viewport rather than scaling it.
    // Retain the exact width so the right rounded corner is never cropped.
    let width = width.max(1);
    if mode == TitlebarCacheMode::Bypass {
        return render_titlebar_uncached(width, title, background, foreground, border);
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
                && entry.border == border
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
    let (buffer, raster_pixels) =
        render_titlebar_raster(width, title, background, foreground, border)?;
    #[cfg(test)]
    let pixel_digest = Sha256::digest(&raster_pixels).into();
    #[cfg(not(test))]
    let _ = raster_pixels;
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
        border,
        buffer: buffer.clone(),
        bytes: retained_bytes,
        #[cfg(test)]
        pixel_digest,
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

#[cfg(test)]
fn cached_titlebar_pixel_digest(
    owner: Option<u64>,
    width: i32,
    title: &str,
    background: u32,
    foreground: u32,
) -> Option<[u8; 32]> {
    let border = frame_border_color(background, foreground, true);
    let cache = TITLEBAR_CACHE.get()?.lock().ok()?;
    cache
        .entries
        .iter()
        .find(|entry| {
            entry.owner == owner
                && entry.width == width
                && entry.title == title
                && entry.background == background
                && entry.foreground == foreground
                && entry.border == border
        })
        .map(|entry| entry.pixel_digest)
}

fn render_titlebar_uncached(
    width: i32,
    title: &str,
    background: u32,
    foreground: u32,
    border: u32,
) -> Option<MemoryRenderBuffer> {
    render_titlebar_raster(width, title, background, foreground, border).map(|(buffer, _)| buffer)
}

fn render_titlebar_raster(
    width: i32,
    title: &str,
    background: u32,
    foreground: u32,
    border: u32,
) -> Option<(MemoryRenderBuffer, Vec<u8>)> {
    let (pixels, width) = render_titlebar_pixels(width, title, background, foreground, border)?;
    let buffer = MemoryRenderBuffer::from_slice(
        &pixels,
        Fourcc::Abgr8888,
        (width as i32, TITLEBAR_HEIGHT),
        1,
        Transform::Normal,
        None,
    );
    Some((buffer, pixels))
}

fn render_titlebar_pixels(
    width: i32,
    title: &str,
    background: u32,
    foreground: u32,
    border: u32,
) -> Option<(Vec<u8>, u32)> {
    let width = u32::try_from(width).ok()?.max(1);
    let title_width = width.saturating_sub((BUTTON_WIDTH * 3 + 20) as u32);
    let mut pixels = rounded_titlebar_pixels(width, background, border);
    let color = [
        ((foreground >> 16) & 0xff) as u8,
        ((foreground >> 8) & 0xff) as u8,
        (foreground & 0xff) as u8,
        255,
    ];
    let asset = title_text_cache()
        .lock()
        .ok()?
        .get(nickel_render_assets::TextRequest {
            text: title,
            size: 14.0,
            line_height: 19.0,
            max_width: None,
            color,
            weight: nickel_render_assets::TextWeight::Normal,
        });
    let text_y = ((TITLEBAR_HEIGHT - i32::try_from(asset.height()).ok()?) / 2 - 1).max(0);
    composite_text_asset(
        &mut pixels,
        width,
        &asset,
        17,
        text_y + 1,
        title_width,
        Some(0.28),
    );
    composite_text_asset(&mut pixels, width, &asset, 16, text_y, title_width, None);
    Some((pixels, width))
}

fn title_text_cache() -> &'static Mutex<nickel_render_assets::TextAssetCache> {
    static CACHE: OnceLock<Mutex<nickel_render_assets::TextAssetCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        TITLEBAR_FONT_DATABASE_LOADS.fetch_add(1, Ordering::Relaxed);
        Mutex::new(nickel_render_assets::TextAssetCache::new())
    })
}

fn rounded_titlebar_pixels(width: u32, background: u32, border: u32) -> Vec<u8> {
    const SAMPLES: i32 = 4;
    let height = TITLEBAR_HEIGHT.max(1) as u32;
    let radius = FRAME_METRICS.corner_radius.max(1) as f32;
    let inner_radius = (radius - FRAME_METRICS.visual_border as f32).max(0.0);
    let background = [
        ((background >> 16) & 0xff) as f32,
        ((background >> 8) & 0xff) as f32,
        (background & 0xff) as f32,
    ];
    let border = [
        ((border >> 16) & 0xff) as f32,
        ((border >> 8) & 0xff) as f32,
        (border & 0xff) as f32,
    ];
    let mut pixels = vec![0_u8; width as usize * height as usize * 4];
    for y in 0..height {
        for x in 0..width {
            let mut outer_samples = 0_i32;
            let mut inner_samples = 0_i32;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let px = x as f32 + (sx as f32 + 0.5) / SAMPLES as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SAMPLES as f32;
                    outer_samples += i32::from(inside_top_rounded_rect(
                        px,
                        py,
                        width as f32,
                        height as f32,
                        radius,
                        0.0,
                    ));
                    inner_samples += i32::from(inside_top_rounded_rect(
                        px,
                        py,
                        width as f32,
                        height as f32,
                        inner_radius,
                        FRAME_METRICS.visual_border as f32,
                    ));
                }
            }
            let samples = (SAMPLES * SAMPLES) as f32;
            let outer = outer_samples as f32 / samples;
            let inner = inner_samples as f32 / samples;
            let outline = (outer - inner).max(0.0);
            let offset = ((y * width + x) * 4) as usize;
            for channel in 0..3 {
                pixels[offset + channel] =
                    (background[channel] * inner + border[channel] * outline).round() as u8;
            }
            pixels[offset + 3] = (outer * 255.0).round() as u8;
        }
    }
    pixels
}

fn inside_top_rounded_rect(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    inset: f32,
) -> bool {
    if x < inset || x >= width - inset || y < inset || y >= height {
        return false;
    }
    if radius <= 0.0 || y >= inset + radius {
        return true;
    }
    let left_center = inset + radius;
    let right_center = width - inset - radius;
    if x >= left_center && x < right_center {
        return true;
    }
    let center_x = if x < left_center {
        left_center
    } else {
        right_center
    };
    let center_y = inset + radius;
    let dx = x - center_x;
    let dy = y - center_y;
    dx * dx + dy * dy <= radius * radius
}

#[allow(clippy::too_many_arguments)]
fn composite_text_asset(
    destination: &mut [u8],
    destination_width: u32,
    asset: &nickel_render_assets::RgbaAsset,
    destination_x: i32,
    destination_y: i32,
    clip_width: u32,
    opacity: Option<f32>,
) {
    let destination_height = destination.len() as u32 / 4 / destination_width.max(1);
    for source_y in 0..asset.height() {
        let target_y = destination_y + source_y as i32;
        if !(0..destination_height as i32).contains(&target_y) {
            continue;
        }
        for source_x in 0..asset.width().min(clip_width) {
            let target_x = destination_x + source_x as i32;
            if !(0..destination_width as i32).contains(&target_x) {
                continue;
            }
            let source_offset = ((source_y * asset.width() + source_x) * 4) as usize;
            let target_offset =
                ((target_y as u32 * destination_width + target_x as u32) * 4) as usize;
            let source = &asset.pixels()[source_offset..source_offset + 4];
            let alpha = f32::from(source[3]) / 255.0 * opacity.unwrap_or(1.0);
            let inverse = 1.0 - alpha;
            for channel in 0..3 {
                let source_channel = if opacity.is_some() {
                    0.0
                } else {
                    f32::from(source[channel])
                };
                destination[target_offset + channel] = (source_channel * alpha
                    + f32::from(destination[target_offset + channel]) * inverse)
                    .round() as u8;
            }
            destination[target_offset + 3] =
                ((alpha + f32::from(destination[target_offset + 3]) / 255.0 * inverse) * 255.0)
                    .round() as u8;
        }
    }
}

fn svg_text_options() -> &'static Mutex<resvg::usvg::Options<'static>> {
    static OPTIONS: OnceLock<Mutex<resvg::usvg::Options<'static>>> = OnceLock::new();
    OPTIONS.get_or_init(|| {
        let mut options = resvg::usvg::Options::default();
        options.fontdb_mut().load_system_fonts();
        Mutex::new(options)
    })
}

pub(crate) fn render_task_switcher_label(
    width: u32,
    height: u32,
    title: &str,
    background: u32,
    foreground: u32,
) -> Option<Vec<u8>> {
    let escaped = task_switcher_label_text(width, title)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;");
    let baseline = height.saturating_sub(5);
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}">
<rect width="{width}" height="{height}" fill="#{background:06x}"/>
<defs><clipPath id="label"><rect x="4" y="0" width="{}" height="{height}"/></clipPath></defs>
<text x="4" y="{baseline}" clip-path="url(#label)" font-family="sans-serif" font-size="14" font-weight="500" fill="#{foreground:06x}">{escaped}</text>
</svg>"##,
        width.saturating_sub(8),
    );
    let options = svg_text_options().lock().ok()?;
    let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &options).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width.max(1), height.max(1))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    Some(pixmap.data().to_vec())
}

fn task_switcher_label_text(width: u32, title: &str) -> String {
    let maximum = ((width.saturating_sub(8)) / 8).max(1) as usize;
    let mut label = title.trim().chars().collect::<Vec<_>>();
    if label.len() > maximum {
        label.truncate(maximum.saturating_sub(1));
        label.push('…');
    }
    label.into_iter().collect()
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

impl FrameCursor {
    pub const fn cursor_icon(self) -> smithay::input::pointer::CursorIcon {
        use smithay::input::pointer::CursorIcon;
        match self {
            Self::Arrow => CursorIcon::Default,
            Self::North => CursorIcon::NResize,
            Self::NorthEast => CursorIcon::NeResize,
            Self::East => CursorIcon::EResize,
            Self::SouthEast => CursorIcon::SeResize,
            Self::South => CursorIcon::SResize,
            Self::SouthWest => CursorIcon::SwResize,
            Self::West => CursorIcon::WResize,
            Self::NorthWest => CursorIcon::NwResize,
        }
    }
}

impl FramePart {
    pub const fn is_resize(self) -> bool {
        matches!(
            self,
            Self::ResizeNorth
                | Self::ResizeNorthEast
                | Self::ResizeEast
                | Self::ResizeSouthEast
                | Self::ResizeSouth
                | Self::ResizeSouthWest
                | Self::ResizeWest
                | Self::ResizeNorthWest
        )
    }

    /// Whether this resize affordance accurately describes the dimensions an
    /// operation may change. A diagonal cursor is only advertised when both
    /// axes are resizable; otherwise its visual promise would be misleading.
    pub const fn resize_allowed(self, width_resizable: bool, height_resizable: bool) -> bool {
        match self {
            Self::ResizeNorth | Self::ResizeSouth => height_resizable,
            Self::ResizeEast | Self::ResizeWest => width_resizable,
            Self::ResizeNorthEast
            | Self::ResizeSouthEast
            | Self::ResizeSouthWest
            | Self::ResizeNorthWest => width_resizable && height_resizable,
            Self::Titlebar | Self::Minimize | Self::Maximize | Self::Close => true,
        }
    }

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

/// Reports whether a point is occupied by the ordinary client scene rather
/// than the compositor's background desktop.
///
/// A server-side frame is part of that scene even though it has no Wayland
/// input surface of its own.
pub fn client_scene_occupies(
    client_surface_present: bool,
    server_frame_part: Option<FramePart>,
) -> bool {
    client_surface_present || server_frame_part.is_some()
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
    let frame_top = content.y - TITLEBAR_HEIGHT;
    let frame_right = content.x + content.width;
    let frame_bottom = content.y + content.height;
    let left = x < content.x + FRAME_METRICS.resize_margin;
    let right = x >= frame_right - FRAME_METRICS.resize_margin;
    let top = y < frame_top + FRAME_METRICS.resize_margin;
    let bottom = y >= frame_bottom - FRAME_METRICS.resize_margin;
    let near_left = x < content.x + FRAME_METRICS.corner_extent;
    let near_right = x >= frame_right - FRAME_METRICS.corner_extent;
    let near_top = y < frame_top + FRAME_METRICS.corner_extent;
    let near_bottom = y >= frame_bottom - FRAME_METRICS.corner_extent;
    if near_left && near_top {
        return Some(FramePart::ResizeNorthWest);
    }
    if near_right && near_top {
        return Some(FramePart::ResizeNorthEast);
    }
    if near_left && near_bottom {
        return Some(FramePart::ResizeSouthWest);
    }
    if near_right && near_bottom {
        return Some(FramePart::ResizeSouthEast);
    }
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
    use sha2::{Digest, Sha256};
    use std::sync::{Mutex, MutexGuard};

    use super::{
        BUTTON_WIDTH, FramePart, RESIZE_BORDER, TITLEBAR_CACHE, TITLEBAR_CACHE_MAX_BYTES,
        TITLEBAR_CACHE_MAX_ENTRIES, TITLEBAR_HEIGHT, TitlebarCacheMode,
        cached_titlebar_pixel_digest, client_border_layers, client_scene_occupies,
        frame_border_color, hit_test, outer_geometry, render_task_switcher_label, render_titlebar,
        render_titlebar_for, render_titlebar_pixels, render_titlebar_with_mode,
        retain_titlebars_for_windows, task_switcher_label_text, titlebar_cache_diagnostics,
        titlebar_geometry,
    };
    use crate::session::shell_layout::Geometry;

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
    fn client_decorated_windows_receive_all_four_compositor_border_edges() {
        let layers = client_border_layers(320, 180, 0x445566);
        assert_eq!(layers.len(), 4);
        assert_eq!(
            layers.iter().filter(|layer| layer.offset == (0, 0)).count(),
            2,
            "left and top edges share the origin"
        );
        assert!(layers.iter().any(|layer| layer.offset == (319, 0)));
        assert!(layers.iter().any(|layer| layer.offset == (0, 179)));
    }

    #[test]
    fn titlebar_raster_has_symmetric_rounded_alpha_corners_and_an_outline() {
        let border = frame_border_color(0x20242c, 0xe8edf4, true);
        let (pixels, width) =
            render_titlebar_pixels(320, "Nickel File", 0x20242c, 0xe8edf4, border)
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
        assert_ne!(rgba(width / 2, 0), rgba(width / 2, 2));
    }

    #[test]
    fn titlebar_glyph_bounds_are_optically_centered_unclipped_and_exclude_controls() {
        let background = 0x20242c;
        let foreground = 0xe8edf4;
        let border = frame_border_color(background, foreground, true);
        let (blank, width) =
            render_titlebar_pixels(420, "", background, foreground, border).unwrap();
        for title in ["Agpq,!?", "Nickel — 猫 🙂"] {
            let (pixels, rendered_width) =
                render_titlebar_pixels(420, title, background, foreground, border).unwrap();
            assert_eq!(rendered_width, width);
            let changed = pixels
                .chunks_exact(4)
                .zip(blank.chunks_exact(4))
                .enumerate()
                .filter_map(|(index, (pixel, empty))| (pixel != empty).then_some(index as u32))
                .collect::<Vec<_>>();
            assert!(
                !changed.is_empty(),
                "representative title must paint glyphs"
            );
            let min_y = changed.iter().map(|index| index / width).min().unwrap();
            let max_y = changed.iter().map(|index| index / width).max().unwrap();
            let max_x = changed.iter().map(|index| index % width).max().unwrap();
            assert!(min_y > 0, "ascenders must not clip the top border");
            assert!(
                max_y < TITLEBAR_HEIGHT as u32 - 1,
                "descenders and their restrained shadow must not clip"
            );
            let optical_center = (min_y + max_y) as f32 / 2.0;
            assert!(
                (18.0..=21.0).contains(&optical_center),
                "glyph ink center {optical_center} must track the 20px titlebar centerline"
            );
            assert!(
                max_x < width - (BUTTON_WIDTH * 3 + 20) as u32 + 17,
                "title ink must remain outside the control exclusion region"
            );
        }
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
        render_titlebar(1024, "Window 79", 0x20242c, 0xe8edf4).expect("cached titlebar raster");

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

    #[test]
    fn realistic_multi_window_steady_frames_do_not_thrash() {
        let _test_lock = cache_test_lock();
        if let Some(cache) = TITLEBAR_CACHE.get() {
            *cache.lock().unwrap() = super::TitlebarCache::default();
        }
        let windows = (0_u64..12)
            .map(|owner| {
                let width = if owner % 4 == 0 { 3840 } else { 1920 };
                (owner, width, format!("Nickel window {owner}"))
            })
            .collect::<Vec<_>>();
        for (owner, width, title) in &windows {
            render_titlebar_for(Some(*owner), *width, title, 0x20242c, 0xe8edf4).unwrap();
        }
        let warm = titlebar_cache_diagnostics();
        assert_eq!(warm.entries, windows.len());
        assert!(warm.live_bytes <= TITLEBAR_CACHE_MAX_BYTES);

        for _ in 0..120 {
            for (owner, width, title) in &windows {
                render_titlebar_for(Some(*owner), *width, title, 0x20242c, 0xe8edf4).unwrap();
            }
        }
        let steady = titlebar_cache_diagnostics();
        assert_eq!(steady.misses, warm.misses);
        assert_eq!(steady.rasterizations, warm.rasterizations);
        assert_eq!(steady.hits - warm.hits, (windows.len() * 120) as u64);
        assert_eq!(steady.font_database_loads, 1);

        retain_titlebars_for_windows(windows.iter().take(3).map(|(owner, _, _)| *owner));
        assert_eq!(titlebar_cache_diagnostics().entries, 3);
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
        for owner in 0..8 {
            render_titlebar_for(
                Some(owner),
                if owner % 3 == 0 { 3840 } else { 1920 },
                &format!("Nickel window {owner}"),
                0x20242c,
                0xe8edf4,
            )
            .expect("warm production titlebar");
        }

        for workload in ["cold", "warm", "churn", "low_reuse"] {
            if workload == "warm" {
                for owner in 0..8 {
                    render_titlebar_for(
                        Some(owner),
                        if owner % 3 == 0 { 3840 } else { 1920 },
                        &format!("Nickel window {owner}"),
                        0x20242c,
                        0xe8edf4,
                    )
                    .expect("rewarm multi-owner production cache");
                }
            }
            let mut cached_samples = Vec::with_capacity(SAMPLES);
            let mut bypass_samples = Vec::with_capacity(SAMPLES);
            for sample in 0..SAMPLES {
                if workload == "cold"
                    && let Some(cache) = TITLEBAR_CACHE.get()
                {
                    *cache.lock().unwrap() = super::TitlebarCache::default();
                }
                let owner = match workload {
                    "low_reuse" => sample as u64 + 100,
                    _ => (sample % 8) as u64,
                };
                let width = if owner % 3 == 0 { 3840 } else { 1920 };
                let title = match workload {
                    "churn" => format!("Nickel window {owner}, generation {sample}"),
                    "low_reuse" => format!("Nickel document {sample}"),
                    _ => format!("Nickel window {owner}"),
                };
                let started = std::time::Instant::now();
                let cached = render_titlebar_with_mode(
                    Some(owner),
                    width,
                    &title,
                    0x20242c,
                    0xe8edf4,
                    frame_border_color(0x20242c, 0xe8edf4, true),
                    TitlebarCacheMode::Enabled,
                )
                .expect("cached titlebar");
                std::hint::black_box(cached);
                cached_samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);

                let started = std::time::Instant::now();
                let bypass = render_titlebar_with_mode(
                    Some(owner),
                    width,
                    &title,
                    0x20242c,
                    0xe8edf4,
                    frame_border_color(0x20242c, 0xe8edf4, true),
                    TitlebarCacheMode::Bypass,
                )
                .expect("bypassed titlebar");
                std::hint::black_box(bypass);
                bypass_samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);

                let cached_digest =
                    cached_titlebar_pixel_digest(Some(owner), width, &title, 0x20242c, 0xe8edf4)
                        .expect("digest retained by production cache path");
                let bypass_pixels = render_titlebar_pixels(
                    width,
                    &title,
                    0x20242c,
                    0xe8edf4,
                    frame_border_color(0x20242c, 0xe8edf4, true),
                )
                .expect("bypass authoritative pixels")
                .0;
                let bypass_digest: [u8; 32] = Sha256::digest(&bypass_pixels).into();
                assert_eq!(cached_digest, bypass_digest);
            }
            let cached = admission_stats(cached_samples);
            let bypass = admission_stats(bypass_samples);
            let diagnostics = titlebar_cache_diagnostics();
            let retained_bytes = diagnostics.live_bytes;
            let oracle_bytes = diagnostics.entries * std::mem::size_of::<[u8; 32]>();
            println!(
                "{{\"schema\":\"nickel-cache-admission-v1\",\"cache\":\"window_titlebar_rasters\",\"workload\":\"{workload}\",\"fixture\":\"multi_owner_mixed_1920_3840\",\"profile\":\"release\",\"samples\":{SAMPLES},\"cached_median_us\":{:.3},\"cached_p95_us\":{:.3},\"bypass_median_us\":{:.3},\"bypass_p95_us\":{:.3},\"retained_bytes\":{retained_bytes},\"test_oracle_bytes\":{oracle_bytes},\"complexity\":{{\"key_fields\":6,\"invalidation_triggers\":5,\"storage_collections\":1}},\"output_equivalence\":\"sha256_cached_source_rgba\"}}",
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
    fn task_switcher_title_is_unicode_safe_and_visibly_truncated() {
        assert_eq!(task_switcher_label_text(88, "Short"), "Short");
        assert_eq!(task_switcher_label_text(48, "猫猫猫猫猫猫猫"), "猫猫猫猫…");
    }

    #[test]
    fn task_switcher_title_is_rasterized_onto_its_opaque_card_background() {
        let pixels = render_task_switcher_label(88, 24, "gyp", 0x002b3852, 0x00e8edf4)
            .expect("task switcher label should rasterize");
        assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }

    #[test]
    fn compositor_frame_controls_occupy_the_scene_above_the_desktop() {
        assert!(client_scene_occupies(false, Some(FramePart::Close)));
        assert!(client_scene_occupies(false, Some(FramePart::Titlebar)));
        assert!(!client_scene_occupies(false, None));
        assert!(client_scene_occupies(true, None));
    }

    #[test]
    fn corners_win_over_edges() {
        assert_eq!(hit_test(CONTENT, 96, 96), Some(FramePart::ResizeNorthWest));
        assert_eq!(
            hit_test(CONTENT, 105, 105),
            Some(FramePart::ResizeNorthWest)
        );
        assert_eq!(
            hit_test(CONTENT, 595, 105),
            Some(FramePart::ResizeNorthEast)
        );
        assert_eq!(
            hit_test(CONTENT, 604, 444),
            Some(FramePart::ResizeSouthEast)
        );
    }

    #[test]
    fn shadow_uses_bounded_nine_slice_assets_instead_of_full_window_bands() {
        let layers = super::shadow_layers(640, 480, true);
        assert_eq!(layers.images.len(), 8);
        assert!(layers.images.iter().all(|layer| {
            layer.size.0 <= 640 + super::FRAME_METRICS.shadow_extent
                && layer.size.1 <= 480 + super::FRAME_METRICS.shadow_extent
                && (layer.size.0 <= super::FRAME_METRICS.shadow_extent
                    || layer.size.1 <= super::FRAME_METRICS.shadow_extent
                    || layer.size.0
                        <= super::FRAME_METRICS.shadow_extent + super::FRAME_METRICS.corner_radius)
        }));
    }

    #[test]
    fn resize_parts_select_directional_cursors() {
        assert_eq!(
            FramePart::ResizeNorthWest.cursor(),
            super::FrameCursor::NorthWest
        );
        assert_eq!(FramePart::ResizeEast.cursor(), super::FrameCursor::East);
        assert_eq!(FramePart::Titlebar.cursor(), super::FrameCursor::Arrow);
        assert_eq!(
            FramePart::ResizeNorthWest.cursor().cursor_icon(),
            smithay::input::pointer::CursorIcon::NwResize
        );
        assert_eq!(
            FramePart::ResizeSouth.cursor().cursor_icon(),
            smithay::input::pointer::CursorIcon::SResize
        );
        assert_eq!(
            FramePart::Titlebar.cursor().cursor_icon(),
            smithay::input::pointer::CursorIcon::Default
        );
    }

    #[test]
    fn resize_parts_only_advertise_dimensions_that_can_change() {
        assert!(FramePart::ResizeEast.resize_allowed(true, false));
        assert!(!FramePart::ResizeEast.resize_allowed(false, true));
        assert!(FramePart::ResizeNorth.resize_allowed(false, true));
        assert!(!FramePart::ResizeNorth.resize_allowed(true, false));
        assert!(FramePart::ResizeNorthEast.resize_allowed(true, true));
        assert!(!FramePart::ResizeNorthEast.resize_allowed(true, false));
        assert!(FramePart::Titlebar.resize_allowed(false, false));
    }

    #[test]
    fn straight_edges_use_the_resize_margin_not_only_the_visual_border() {
        assert_eq!(hit_test(CONTENT, 350, 103), Some(FramePart::ResizeNorth));
        assert_eq!(hit_test(CONTENT, 597, 250), Some(FramePart::ResizeEast));
        assert_eq!(hit_test(CONTENT, 350, 437), Some(FramePart::ResizeSouth));
        assert_eq!(hit_test(CONTENT, 103, 250), Some(FramePart::ResizeWest));
        assert_eq!(hit_test(CONTENT, 350, 106), Some(FramePart::Titlebar));
        assert_eq!(hit_test(CONTENT, 594, 250), None);
        assert_eq!(hit_test(CONTENT, 350, 434), None);
        assert_eq!(hit_test(CONTENT, 106, 250), None);
    }
}
