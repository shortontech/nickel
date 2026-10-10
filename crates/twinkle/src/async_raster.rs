//! CPU raster work owned by a worker; native surfaces remain on their event loop.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
};

use crate::{DamageRegion, Pixel, Rect, SoftwareRenderer, backend::PaintCommand};

/// An owned display-list snapshot. The producer may immediately continue to
/// handle input and replace the pending frame for the same surface.
pub struct RasterRequest {
    pub surface: u64,
    pub revision: u64,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub commands: Vec<PaintCommand>,
    pub damage: Option<Vec<Rect>>,
}

pub struct RasterResult {
    pub surface: u64,
    pub revision: u64,
    pub width: u32,
    pub height: u32,
    pub damage: DamageRegion,
    pub pixels: Vec<Pixel>,
}

#[derive(Default)]
struct State {
    pending: HashMap<u64, RasterRequest>,
    order: VecDeque<u64>,
    retired: HashSet<u64>,
    completed: HashMap<u64, RasterResult>,
    acknowledged: HashMap<u64, u64>,
    stopped: bool,
}

struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

/// One worker retains a renderer per surface, preserving text and image caches
/// without giving native window or compositor graphics handles to the worker.
pub struct AsyncRasterWorker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl AsyncRasterWorker {
    pub fn new(name: &str, wake: impl Fn() + Send + 'static) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            ready: Condvar::new(),
        });
        let worker_shared = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name(name.into())
            .spawn(move || worker_loop(worker_shared, wake))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// Replace an unstarted frame for this surface. At most one frame per
    /// surface waits behind an in-progress raster job.
    pub fn submit(&self, request: RasterRequest) {
        let mut state = self.shared.state.lock().expect("raster queue poisoned");
        let surface = request.surface;
        state.retired.remove(&surface);
        if state.pending.insert(surface, request).is_none() {
            state.order.push_back(surface);
        }
        self.shared.ready.notify_one();
    }

    pub fn take_completed(&self, surface: u64) -> Option<RasterResult> {
        self.shared
            .state
            .lock()
            .expect("raster queue poisoned")
            .completed
            .remove(&surface)
    }

    /// Confirm the native presenter accepted a result. A skipped result makes
    /// the next partial damage relative to pixels that were never presented.
    pub fn acknowledge(&self, surface: u64, revision: u64) {
        self.shared
            .state
            .lock()
            .expect("raster queue poisoned")
            .acknowledged
            .insert(surface, revision);
    }

    /// Drop queued work and retained raster caches for a closed surface.
    pub fn retire(&self, surface: u64) {
        let mut state = self.shared.state.lock().expect("raster queue poisoned");
        state.pending.remove(&surface);
        state.completed.remove(&surface);
        state.acknowledged.remove(&surface);
        state.retired.insert(surface);
        self.shared.ready.notify_one();
    }
}

impl Drop for AsyncRasterWorker {
    fn drop(&mut self) {
        {
            let mut state = self.shared.state.lock().expect("raster queue poisoned");
            state.stopped = true;
            state.pending.clear();
            state.order.clear();
            self.shared.ready.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn worker_loop(shared: Arc<Shared>, wake: impl Fn()) {
    let mut renderers = HashMap::<u64, SoftwareRenderer>::new();
    let mut rendered_revisions = HashMap::<u64, u64>::new();
    loop {
        let request = {
            let mut state = shared.state.lock().expect("raster queue poisoned");
            while !state.stopped && state.order.is_empty() && state.retired.is_empty() {
                state = shared.ready.wait(state).expect("raster queue poisoned");
            }
            if state.stopped {
                return;
            }
            for surface in state.retired.drain() {
                renderers.remove(&surface);
                rendered_revisions.remove(&surface);
            }
            state.order.pop_front().and_then(|id| {
                state.pending.remove(&id).map(|request| {
                    let acknowledged = state.acknowledged.get(&id).copied();
                    (request, acknowledged)
                })
            })
        };
        let Some((request, acknowledged)) = request else {
            continue;
        };
        let renderer = renderers.entry(request.surface).or_insert_with(|| {
            SoftwareRenderer::new_pixel_buffer(request.width, request.height, request.scale)
        });
        renderer.resize(request.width, request.height, request.scale);
        if rendered_revisions.get(&request.surface).copied() != acknowledged {
            renderer.invalidate();
        }
        let damage = renderer.render_frame_with_damage_hint(
            &request.commands,
            request.revision,
            request.damage.as_deref(),
        );
        let pixels = if damage.is_empty() {
            Vec::new()
        } else {
            renderer.pixels().to_vec()
        };
        let result = RasterResult {
            surface: request.surface,
            revision: request.revision,
            width: request.width,
            height: request.height,
            damage,
            pixels,
        };
        rendered_revisions.insert(request.surface, request.revision);
        let mut state = shared.state.lock().expect("raster queue poisoned");
        if !state.retired.contains(&request.surface) && !state.stopped {
            state.completed.insert(request.surface, result);
            drop(state);
            wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use crate::{Rect, backend::PaintCommand};

    use super::{AsyncRasterWorker, RasterRequest};

    #[test]
    fn worker_rasters_owned_frames_and_retires_surface_state() {
        let (wake_tx, wake_rx) = mpsc::channel();
        let worker = AsyncRasterWorker::new("raster-test", move || {
            wake_tx.send(()).unwrap();
        })
        .unwrap();
        worker.submit(RasterRequest {
            surface: 7,
            revision: 1,
            width: 8,
            height: 6,
            scale: 1.0,
            commands: vec![PaintCommand::Fill {
                rect: Rect::new(0.0, 0.0, 8.0, 6.0),
                color: 0x123456,
            }],
            damage: None,
        });
        wake_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let result = worker.take_completed(7).unwrap();
        assert_eq!((result.width, result.height, result.revision), (8, 6, 1));
        assert!(!result.damage.is_empty());
        assert_eq!((result.pixels[0].r, result.pixels[0].g), (0x12, 0x34));
        // The first result was consumed but never presented. The next frame
        // must repaint its entire buffer, even if its command delta is small.
        worker.submit(RasterRequest {
            surface: 7,
            revision: 2,
            width: 8,
            height: 6,
            scale: 1.0,
            commands: vec![
                PaintCommand::Fill {
                    rect: Rect::new(0.0, 0.0, 8.0, 6.0),
                    color: 0x123456,
                },
                PaintCommand::Fill {
                    rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                    color: 0x654321,
                },
            ],
            damage: None,
        });
        wake_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let result = worker.take_completed(7).unwrap();
        assert_eq!(
            result.damage.rects.as_slice(),
            &[Rect::new(0.0, 0.0, 8.0, 6.0)]
        );
        worker.retire(7);
        assert!(worker.take_completed(7).is_none());
    }
}
