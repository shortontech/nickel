//! Native demand-driven wallpaper thumbnails. One worker, one in-flight batch,
//! and one replaceable desired batch keep scrolling from building a decode queue.

use crate::{plugin_panel::PluginImages, wallpaper_selection::Catalog};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

struct Job {
    catalog: Arc<Catalog>,
    ids: Vec<String>,
    cancelled: Arc<AtomicBool>,
}

struct Completion {
    cancelled: Arc<AtomicBool>,
    attempted: Vec<String>,
    images: PluginImages,
}

struct Worker {
    jobs: mpsc::SyncSender<Job>,
    results: mpsc::Receiver<Completion>,
}

impl Worker {
    fn start() -> std::io::Result<Self> {
        let (jobs, requests) = mpsc::sync_channel::<Job>(1);
        let (results, completed) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("wallpaper-previews".into())
            .spawn(move || {
                while let Ok(job) = requests.recv() {
                    let images =
                        job.catalog
                            .previews_for(job.ids.iter().map(String::as_str), || {
                                if job.cancelled.load(Ordering::Acquire) {
                                    Err("preview demand retired".into())
                                } else {
                                    Ok(())
                                }
                            });
                    if results
                        .send(Completion {
                            cancelled: job.cancelled,
                            attempted: job.ids,
                            images,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self {
            jobs,
            results: completed,
        })
    }
}

pub(crate) struct WallpaperPreviews {
    surfaces:
        std::collections::BTreeMap<nickel_core::plugins::PluginSurfaceKey, (u64, Arc<Vec<String>>)>,
    observation: u64,
    surfaces_dirty: bool,
    catalog: Option<Arc<Catalog>>,
    demand: Vec<String>,
    images: PluginImages,
    attempted: std::collections::BTreeSet<String>,
    cancelled: Arc<AtomicBool>,
    worker: Option<Worker>,
    in_flight: bool,
    settled: bool,
    deadline: Option<Instant>,
}

impl Default for WallpaperPreviews {
    fn default() -> Self {
        Self {
            surfaces: Default::default(),
            observation: 0,
            surfaces_dirty: false,
            catalog: None,
            demand: Vec::new(),
            images: Default::default(),
            attempted: Default::default(),
            cancelled: Arc::new(AtomicBool::new(false)),
            worker: None,
            in_flight: false,
            settled: false,
            deadline: None,
        }
    }
}

impl Drop for WallpaperPreviews {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        // Dropping both channels wakes an idle worker or releases a completion
        // sender. Destruction never waits for an image decoder on the UI thread.
    }
}

impl WallpaperPreviews {
    pub(crate) fn sync_surfaces<'a>(
        &mut self,
        surfaces: impl Iterator<Item = (&'a nickel_core::plugins::PluginSurfaceKey, Arc<Vec<String>>)>,
        now: Instant,
    ) {
        self.observation = self.observation.wrapping_add(1);
        let mut changed = std::mem::take(&mut self.surfaces_dirty);
        for (key, assets) in surfaces {
            if assets.is_empty() {
                continue;
            }
            if let Some((seen, previous)) = self.surfaces.get_mut(key) {
                *seen = self.observation;
                if !Arc::ptr_eq(previous, &assets) && **previous != *assets {
                    changed = true;
                }
                *previous = assets;
            } else {
                self.surfaces
                    .insert(key.clone(), (self.observation, assets));
                changed = true;
            }
        }
        self.surfaces.retain(|_, (seen, _)| {
            let keep = *seen == self.observation;
            changed |= !keep;
            keep
        });
        if changed {
            let ids = self
                .surfaces
                .values()
                .flat_map(|(_, assets)| assets.iter())
                .filter_map(|asset| asset.strip_prefix("wallpaper:"))
                .map(str::to_owned)
                .collect();
            self.set_demand(ids, now);
        } else {
            self.start_latest(now);
        }
    }

    fn invalidate(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.cancelled = Arc::new(AtomicBool::new(false));
        self.settled = false;
    }

    pub(crate) fn set_catalog(&mut self, catalog: Catalog) {
        self.invalidate();
        self.catalog = Some(Arc::new(catalog));
        self.surfaces_dirty = true;
        self.images.clear();
        self.attempted.clear();
        // Metadata observation alone must never start file or decoder work.
    }

    pub(crate) fn clear_catalog(&mut self) {
        if self.catalog.take().is_some() {
            self.surfaces_dirty = true;
            self.invalidate();
            self.images.clear();
            self.attempted.clear();
        }
    }

    pub(crate) fn set_demand(&mut self, mut ids: Vec<String>, now: Instant) {
        // Only approved visible identities consume cache space. The catalog's
        // existing 128-entry bound limits retained 160×90 thumbnails to 7.1 MiB;
        // the eight-image limit applies to a decode batch, not visible content.
        ids.retain(|id| {
            self.catalog
                .as_ref()
                .is_some_and(|catalog| catalog.approves_preview_id(id))
        });
        ids.sort();
        ids.dedup();
        if ids != self.demand {
            self.invalidate();
            self.demand = ids;
            self.attempted.retain(|id| self.demand.contains(id));
            self.images.retain(|asset, _| {
                asset
                    .strip_prefix("wallpaper:")
                    .is_some_and(|id| self.demand.iter().any(|wanted| wanted == id))
            });
        }
        self.start_latest(now);
    }

    fn start_latest(&mut self, now: Instant) {
        if self.surfaces_dirty || self.in_flight || self.settled || self.demand.is_empty() {
            return;
        }
        let Some(catalog) = &self.catalog else {
            return;
        };
        let ids = self
            .demand
            .iter()
            .filter(|id| !self.attempted.contains(*id))
            .take(8)
            .cloned()
            .collect::<Vec<_>>();
        if ids.is_empty() {
            self.settled = true;
            return;
        }
        if self.worker.is_none() {
            match Worker::start() {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    tracing::warn!(%error, "wallpaper preview worker unavailable");
                    self.settled = true;
                    return;
                }
            }
        }
        let job = Job {
            catalog: Arc::clone(catalog),
            ids,
            cancelled: Arc::clone(&self.cancelled),
        };
        match self.worker.as_ref().unwrap().jobs.try_send(job) {
            Ok(()) => {
                self.in_flight = true;
                self.deadline = Some(now + Duration::from_millis(16));
            }
            Err(_) => {
                self.worker = None;
                self.settled = true;
            }
        }
    }

    pub(crate) fn images(&self) -> &PluginImages {
        &self.images
    }
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub(crate) fn poll(&mut self, now: Instant) -> bool {
        if !self.deadline.is_some_and(|deadline| now >= deadline) {
            return false;
        }
        let result = self.worker.as_ref().map(|worker| worker.results.try_recv());
        match result {
            Some(Ok(completion)) => {
                self.in_flight = false;
                self.deadline = None;
                let current = Arc::ptr_eq(&completion.cancelled, &self.cancelled)
                    && !completion.cancelled.load(Ordering::Acquire);
                let changed = current && (!self.images.is_empty() || !completion.images.is_empty());
                if current {
                    self.images.extend(completion.images);
                    self.attempted.extend(completion.attempted);
                    self.settled = false;
                }
                self.start_latest(now);
                changed
            }
            Some(Err(mpsc::TryRecvError::Empty)) => {
                self.deadline = Some(now + Duration::from_millis(16));
                false
            }
            _ => {
                self.in_flight = false;
                self.deadline = None;
                self.worker = None;
                self.settled = true;
                false
            }
        }
    }
}
