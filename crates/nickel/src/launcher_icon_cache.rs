//! Bounded icon cache shared by the JSX launcher and legacy visual fixtures.

use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};

use crate::{icons, launcher::Application, platform};
use image::RgbaImage;

pub(crate) const PLACE_ICON_SIZE: u32 = 20;

/// Bounded CPU image cache with stable renderer resource IDs.
#[derive(Clone)]
pub struct LauncherIconCache {
    shared: Arc<LauncherIconCacheShared>,
}

struct LauncherIconCacheShared {
    state: Mutex<LauncherIconCacheState>,
    requests: mpsc::Sender<LauncherIconRequest>,
    revision: AtomicU64,
}

struct LauncherIconCacheState {
    icons: HashMap<String, CachedIcon>,
    insertion_order: VecDeque<String>,
    next_id: u16,
    evictions: u64,
    generation: u64,
}

pub(crate) const LAUNCHER_ICON_CACHE_CAPACITY: usize = 512;
pub(crate) const LAUNCHER_ICON_MAX_SIDE: u32 = 96;
const LAUNCHER_ICON_CACHE_MAX_BYTES: usize = LAUNCHER_ICON_CACHE_CAPACITY
    * LAUNCHER_ICON_MAX_SIDE as usize
    * LAUNCHER_ICON_MAX_SIDE as usize
    * 4;

#[derive(Clone)]
struct CachedIcon {
    id: u16,
    image: Option<Arc<RgbaImage>>,
    pending: bool,
}

struct LauncherIconRequest {
    key: String,
    id: u16,
    generation: u64,
    icon_path: Option<PathBuf>,
    icon_reference: Option<String>,
}

impl LauncherIconCache {
    pub fn new() -> Self {
        let (requests, receiver) = mpsc::channel();
        let shared = Arc::new(LauncherIconCacheShared {
            state: Mutex::new(LauncherIconCacheState {
                icons: HashMap::new(),
                insertion_order: VecDeque::new(),
                // Keep launcher IDs away from wallpaper and panel fixed IDs.
                next_id: 0x4000,
                evictions: 0,
                generation: 0,
            }),
            requests,
            revision: AtomicU64::new(0),
        });
        let worker_shared = Arc::downgrade(&shared);
        std::thread::Builder::new()
            .name("nickel-launcher-icons".into())
            .spawn(move || launcher_icon_worker(worker_shared, receiver))
            .expect("launcher icon worker must start");
        Self { shared }
    }

    pub fn diagnostics(&self) -> LauncherIconCacheDiagnostics {
        let state = self.shared.state.lock().expect("launcher icon cache lock");
        LauncherIconCacheDiagnostics {
            entries: state.icons.len(),
            capacity: LAUNCHER_ICON_CACHE_CAPACITY,
            retained_pixel_bytes: state
                .icons
                .values()
                .filter_map(|cached| cached.image.as_ref())
                .map(|image| image.as_raw().len())
                .sum(),
            byte_capacity: LAUNCHER_ICON_CACHE_MAX_BYTES,
            evictions: state.evictions,
        }
    }

    pub(crate) fn revision(&self) -> u64 {
        self.shared.revision.load(Ordering::Acquire)
    }

    pub fn begin_visual_generation(&mut self) {
        let mut state = self.shared.state.lock().expect("launcher icon cache lock");
        state.icons.retain(|key, _| !key.starts_with("structural:"));
        state
            .insertion_order
            .retain(|key| !key.starts_with("structural:"));
    }

    pub(crate) fn invalidate_application_inventory(&mut self) {
        let mut state = self.shared.state.lock().expect("launcher icon cache lock");
        state.generation = state.generation.wrapping_add(1);
        state.icons.clear();
        state.insertion_order.clear();
    }

    pub(crate) fn resolve(&mut self, application: &Application) -> Option<(u16, Arc<RgbaImage>)> {
        let mut state = self.shared.state.lock().expect("launcher icon cache lock");
        if let Some(cached) = state.icons.get(application.id()) {
            return cached
                .image
                .as_ref()
                .map(|image| (cached.id, Arc::clone(image)));
        }
        let id = state.next_id;
        state.next_id = state.next_id.checked_add(1).unwrap_or(0x4000);
        let key = application.id().to_owned();
        let generation = state.generation;
        insert_launcher_icon(
            &mut state,
            key.clone(),
            CachedIcon {
                id,
                image: None,
                pending: true,
            },
        );
        drop(state);
        if self
            .shared
            .requests
            .send(LauncherIconRequest {
                key: key.clone(),
                id,
                generation,
                icon_path: application.icon_path().map(PathBuf::from),
                icon_reference: application.icon().map(str::to_owned),
            })
            .is_err()
        {
            let mut state = self.shared.state.lock().expect("launcher icon cache lock");
            if let Some(cached) = state.icons.get_mut(&key) {
                cached.pending = false;
            }
        }
        None
    }

    pub(crate) fn resolve_window_icon(
        &mut self,
        window: crate::model::WindowId,
        image: Arc<RgbaImage>,
    ) -> (u16, Arc<RgbaImage>) {
        let key = format!("window:{}", window.0);
        let mut state = self.shared.state.lock().expect("launcher icon cache lock");
        if let Some(cached) = state.icons.get(&key)
            && cached
                .image
                .as_ref()
                .is_some_and(|cached_image| cached_image.as_raw() == image.as_raw())
        {
            return (cached.id, Arc::clone(cached.image.as_ref().unwrap()));
        }
        let id = state.next_id;
        state.next_id = state.next_id.checked_add(1).unwrap_or(0x4000);
        insert_launcher_icon(
            &mut state,
            key,
            CachedIcon {
                id,
                image: Some(Arc::clone(&image)),
                pending: false,
            },
        );
        (id, image)
    }

    #[cfg(any(test, feature = "workbench-fixtures"))]
    pub(crate) fn structural(
        &mut self,
        name: &str,
        bytes: &[u8],
        color: u32,
    ) -> Option<(u16, Arc<RgbaImage>)> {
        let key = format!("structural:{name}:{color:06x}");
        let mut state = self.shared.state.lock().expect("launcher icon cache lock");
        if let Some(cached) = state.icons.get(&key) {
            return cached
                .image
                .as_ref()
                .map(|image| (cached.id, Arc::clone(image)));
        }
        let image = icons::load_svg_bytes(bytes, 48).map(|mut image| {
            let red = ((color >> 16) & 0xff) as u8;
            let green = ((color >> 8) & 0xff) as u8;
            let blue = (color & 0xff) as u8;
            for pixel in image.pixels_mut() {
                if pixel[3] != 0 {
                    pixel[0] = red;
                    pixel[1] = green;
                    pixel[2] = blue;
                }
            }
            Arc::new(image)
        });
        let id = state.next_id;
        state.next_id = state.next_id.checked_add(1).unwrap_or(0x4000);
        insert_launcher_icon(
            &mut state,
            key,
            CachedIcon {
                id,
                image: image.clone(),
                pending: false,
            },
        );
        image.map(|image| (id, image))
    }
}

fn insert_launcher_icon(state: &mut LauncherIconCacheState, key: String, cached: CachedIcon) {
    while state.icons.len() >= LAUNCHER_ICON_CACHE_CAPACITY {
        let Some(oldest) = state.insertion_order.pop_front() else {
            break;
        };
        if state.icons.remove(&oldest).is_some() {
            state.evictions = state.evictions.saturating_add(1);
        }
    }
    state.insertion_order.push_back(key.clone());
    state.icons.insert(key, cached);
}

fn launcher_icon_worker(
    shared: std::sync::Weak<LauncherIconCacheShared>,
    receiver: mpsc::Receiver<LauncherIconRequest>,
) {
    while let Ok(request) = receiver.recv() {
        let image = if request.key.starts_with("place:") {
            request
                .icon_reference
                .as_deref()
                .map(|path| place_icon(std::path::Path::new(path)))
        } else {
            request
                .icon_path
                .as_deref()
                .and_then(icons::load)
                .or_else(|| {
                    request
                        .icon_reference
                        .as_deref()
                        .and_then(platform::application_icon)
                })
        }
        .filter(has_visible_pixel)
        .map(normalize_launcher_icon)
        .map(Arc::new);
        let Some(shared) = shared.upgrade() else {
            return;
        };
        let mut state = shared.state.lock().expect("launcher icon cache lock");
        if state.generation != request.generation {
            continue;
        }
        let Some(cached) = state.icons.get_mut(&request.key) else {
            continue;
        };
        if cached.id != request.id || !cached.pending {
            continue;
        }
        cached.image = image;
        cached.pending = false;
        drop(state);
        shared.revision.fetch_add(1, Ordering::Release);
    }
}

fn place_icon(path: &std::path::Path) -> RgbaImage {
    use nickel_file::icons::{ArtworkAppearance, ArtworkRequest, SemanticIconKind};

    let settings = nickel_core::shell_settings::ShellSettings::load_default();
    let is_home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .is_some_and(|home| path == std::path::Path::new(&home));
    let kind = if is_home {
        SemanticIconKind::HomeFolder
    } else {
        nickel_file::icons::semantic_kind(path, true)
    };
    let request = ArtworkRequest {
        path,
        kind,
        logical_size: PLACE_ICON_SIZE as u16,
        scale_milli: 1000,
        appearance: ArtworkAppearance::Dark,
    };
    if is_home {
        return nickel_file::icons::resolve_artwork(
            nickel_core::shell_settings::FileIconPreference::Nickel,
            &request,
        )
        .pixels
        .as_ref()
        .clone();
    }
    nickel_platform::path_icon_with_theme_at_size(
        path,
        settings.file_icon_theme.as_deref(),
        PLACE_ICON_SIZE,
    )
    .unwrap_or_else(|| {
        nickel_file::icons::resolve_artwork(
            nickel_core::shell_settings::FileIconPreference::Nickel,
            &request,
        )
        .pixels
        .as_ref()
        .clone()
    })
}

pub(crate) fn normalize_launcher_icon(image: RgbaImage) -> RgbaImage {
    let bounds = image
        .enumerate_pixels()
        .filter(|(_, _, pixel)| pixel[3] != 0)
        .fold(None::<(u32, u32, u32, u32)>, |bounds, (x, y, _)| {
            Some(match bounds {
                None => (x, y, x, y),
                Some((left, top, right, bottom)) => {
                    (left.min(x), top.min(y), right.max(x), bottom.max(y))
                }
            })
        });
    let image = if let Some((left, top, right, bottom)) = bounds {
        image::imageops::crop_imm(&image, left, top, right - left + 1, bottom - top + 1).to_image()
    } else {
        image
    };
    if image.width() > LAUNCHER_ICON_MAX_SIDE || image.height() > LAUNCHER_ICON_MAX_SIDE {
        icons::resized(&image, LAUNCHER_ICON_MAX_SIDE, LAUNCHER_ICON_MAX_SIDE)
    } else {
        image
    }
}

impl Default for LauncherIconCache {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LauncherIconCacheDiagnostics {
    pub entries: usize,
    pub capacity: usize,
    pub retained_pixel_bytes: usize,
    pub byte_capacity: usize,
    pub evictions: u64,
}

fn has_visible_pixel(image: &RgbaImage) -> bool {
    image.pixels().any(|pixel| pixel[3] != 0)
}
