//! Primary-GPU preview work, independent of output presentation and target GPUs.

use super::*;
use crate::session::window_registry::WindowId;
use smithay::backend::renderer::{gles::GlesMapping, sync::SyncPoint};

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const REFRESH_INTERVAL: Duration = Duration::from_millis(100);
const PENDING_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, PartialEq, Eq)]
enum Readiness {
    Stale,
    Pending,
    Ready,
    TimedOut,
}

fn preview_readiness(
    current: bool,
    elapsed: Duration,
    signaled: impl FnOnce() -> bool,
) -> Readiness {
    if signaled() {
        return if current {
            Readiness::Ready
        } else {
            Readiness::Stale
        };
    }
    if elapsed >= PENDING_TIMEOUT {
        Readiness::TimedOut
    } else {
        Readiness::Pending
    }
}

pub(super) struct SubmittedPreview {
    // Keep the draw target alive until the readback has completed or retired.
    pub(super) texture: GlesTexture,
    pub(super) mapping: GlesMapping,
    pub(super) fence: SyncPoint,
    pub(super) dimensions: (u16, u16),
}

struct PendingPreview {
    id: WindowId,
    tag: (u64, u64),
    gpu: DrmNode,
    renderer_generation: (u64, u64),
    context: usize,
    started: Instant,
    submission: SubmittedPreview,
}

#[derive(Default)]
pub(super) struct NativePreviewWork {
    // One global slot is also at most one slot per renderer context. Never keep
    // a queue of client commits or move a thread-affine renderer to a worker.
    pending: Option<PendingPreview>,
    timer: Option<RegistrationToken>,
    last_capture: HashMap<WindowId, Instant>,
    diagnostics: nickel_session_protocol::NativePreviewWorkDiagnostics,
}

impl UdevData {
    pub(crate) fn native_preview_diagnostics(
        &self,
    ) -> nickel_session_protocol::NativePreviewWorkDiagnostics {
        let mut result = self.preview_work.diagnostics;
        if let Some(pending) = &self.preview_work.pending {
            let (width, height) = pending.submission.dimensions;
            let payload = u64::from(width) * u64::from(height) * 4;
            result.pending_count = 1;
            result.pending_texture_bytes = payload;
            result.pending_readback_bytes = payload;
        }
        result
    }

    pub(crate) fn retain_native_preview_interest(
        &mut self,
        admitted: &HashSet<WindowId>,
    ) -> Option<RegistrationToken> {
        self.preview_work
            .last_capture
            .retain(|id, _| admitted.contains(id));
        // Obsolete work still owns its slot until readiness/timeout. Replacing
        // it on every client commit would queue GPU work behind a slow fence.
        if admitted.is_empty() && self.preview_work.pending.is_none() {
            self.preview_work.timer.take()
        } else {
            None
        }
    }
}

impl NickelSession {
    pub(super) fn schedule_native_preview_work(&mut self) {
        let Some(native) = self.native.as_ref() else {
            return;
        };
        if native.preview_work.timer.is_some() {
            return;
        }
        if self.locked
            || !native.activity.is_active()
            || (native.preview_work.pending.is_none() && !self.preview_capture_work_pending())
        {
            return;
        }
        let timer = Timer::from_duration(Duration::ZERO);
        match self.event_loop_handle.insert_source(timer, |_, _, data| {
            if data.native_preview_turn() {
                TimeoutAction::ToDuration(POLL_INTERVAL)
            } else {
                if let Some(native) = data.native.as_mut() {
                    native.preview_work.timer = None;
                }
                TimeoutAction::Drop
            }
        }) {
            Ok(token) => self.native.as_mut().unwrap().preview_work.timer = Some(token),
            Err(error) => tracing::warn!(?error, "failed to schedule native preview work"),
        }
    }

    fn native_preview_turn(&mut self) -> bool {
        let Some(mut native) = self.native.take() else {
            return false;
        };
        let now = Instant::now();
        native.preview_work.diagnostics.turns += 1;
        // Owner generations prevent a recycled EGL handle from accepting a
        // completion from a removed/recreated renderer or topology generation.
        let renderer_generation = (
            native.renderer_lifecycle.activations,
            native.renderer_lifecycle.retirements,
        );
        let mut changed = false;
        let keep_polling = (|| {
            if self.locked || !native.activity.is_active() {
                native.preview_work.diagnostics.cancellations +=
                    u64::from(native.preview_work.pending.is_some());
                native.preview_work.pending = None;
                native.preview_work.last_capture.clear();
                return false;
            }
            native
                .preview_work
                .last_capture
                .retain(|id, _| self.preview_capture_tag(*id).is_some());
            if let Some(pending) = native.preview_work.pending.as_ref() {
                match preview_readiness(
                    self.preview_capture_tag(pending.id) == Some(pending.tag)
                        && pending.gpu == native.primary_gpu
                        && pending.renderer_generation == renderer_generation,
                    now.duration_since(pending.started),
                    || pending.submission.fence.is_reached(),
                ) {
                    Readiness::Stale => {
                        native.preview_work.diagnostics.cancellations += 1;
                        self.defer_preview_capture(pending.id);
                        native.preview_work.pending = None;
                    }
                    Readiness::Pending => {
                        native.preview_work.diagnostics.pending_polls += 1;
                        // Readiness polling must not render outputs or call map.
                        return true;
                    }
                    Readiness::TimedOut => {
                        native.preview_work.diagnostics.timeouts += 1;
                        let id = pending.id;
                        tracing::trace!(
                            ?id,
                            elapsed_us = elapsed_micros(pending.started),
                            "preview readback timed out"
                        );
                        native.preview_work.pending = None;
                        self.preview_renderer_failed(id);
                    }
                    Readiness::Ready => {}
                }
            }

            // Always use the primary renderer alone. An output's MultiRenderer
            // may copy to another GPU during finish, including CPU-copy fallback.
            let mut primary = match native.gpus.single_renderer(&native.primary_gpu) {
                Ok(renderer) => renderer,
                Err(error) => {
                    if let Some(pending) = native.preview_work.pending.take() {
                        native.preview_work.diagnostics.cancellations += 1;
                        self.preview_renderer_failed(pending.id);
                    }
                    self.preview_renderer_unavailable(now);
                    tracing::debug!(?error, "primary preview renderer unavailable");
                    return false;
                }
            };
            let renderer: &mut GlesRenderer = primary.as_mut();
            let context = renderer.egl_context().get_context_handle() as usize;
            if let Some(pending) = native.preview_work.pending.take() {
                if pending.context == context {
                    // Only a signaled post-readback fence reaches this map. Keep
                    // old CPU pixels owned by the cache until mapping succeeds.
                    let map_started = Instant::now();
                    let result = renderer.map_texture(&pending.submission.mapping);
                    if let Ok(mapped) = result {
                        let (width, height) = pending.submission.dimensions;
                        if crate::session::state::preview_mapping_has_exact_size(
                            mapped, width, height,
                        ) {
                            let (pixels, _) = self.take_preview_capture_buffer(pending.id);
                            let rgba = crate::session::state::reuse_preview_pixels(pixels, mapped);
                            self.store_preview(
                                pending.id,
                                PreviewFrame {
                                    width,
                                    height,
                                    rgba,
                                },
                            );
                            changed = true;
                            native.preview_work.diagnostics.completions += 1;
                            native.preview_work.diagnostics.completion_age_us +=
                                elapsed_micros(pending.started);
                            tracing::trace!(id = ?pending.id, map_copy_us = elapsed_micros(map_started), submitted_elapsed_us = elapsed_micros(pending.started), "preview readback installed");
                        } else {
                            native.preview_work.diagnostics.readback_failures += 1;
                            self.preview_renderer_failed(pending.id);
                        }
                    } else {
                        native.preview_work.diagnostics.readback_failures += 1;
                        self.preview_renderer_failed(pending.id);
                    }
                    native.preview_work.diagnostics.map_copy_cpu_us += elapsed_micros(map_started);
                } else {
                    native.preview_work.diagnostics.cancellations += 1;
                    self.defer_preview_capture(pending.id);
                }
                // Texture and mapping drops enqueue renderer-owned cleanup; no
                // explicit drain or fence wait is performed during retirement.
                drop(pending.submission.texture);
            }

            let wave = self.begin_preview_render_wave();
            let mut candidates = self.preview_capture_candidates(wave);
            // Least recently attempted first prevents an animated first window
            // starving its neighbours. Only the bounded admitted set is sorted.
            candidates.sort_by_key(|(id, _)| native.preview_work.last_capture.get(id).copied());
            let mut chosen = None;
            let mut deferred = false;
            for (id, window) in candidates {
                let ready = native
                    .preview_work
                    .last_capture
                    .get(&id)
                    .is_none_or(|last| now.duration_since(*last) >= REFRESH_INTERVAL);
                if chosen.is_none() && ready {
                    chosen = Some((id, window));
                } else {
                    self.defer_preview_capture(id);
                    deferred = true;
                }
            }
            let Some((id, window)) = chosen else {
                return deferred;
            };
            let Some(tag) = self.preview_capture_tag(id) else {
                return deferred;
            };
            native.preview_work.last_capture.insert(id, now);
            let submitted_at = Instant::now();
            let submission = submit_preview(renderer, &window);
            native.preview_work.diagnostics.submit_cpu_us += elapsed_micros(submitted_at);
            match submission {
                Some(submission) => {
                    // Logical payload, not driver allocation size: one texture
                    // and one PBO, each no larger than 240 * 135 * 4 bytes.
                    let payload = usize::from(submission.dimensions.0)
                        * usize::from(submission.dimensions.1)
                        * 4;
                    native.preview_work.diagnostics.submissions += 1;
                    native.preview_work.diagnostics.peak_pending_payload_bytes = native
                        .preview_work
                        .diagnostics
                        .peak_pending_payload_bytes
                        .max(payload as u64 * 2);
                    tracing::trace!(
                        ?id,
                        submit_us = elapsed_micros(submitted_at),
                        pending_count = 1,
                        texture_texel_bytes = payload,
                        readback_payload_bytes = payload,
                        "preview readback submitted"
                    );
                    native.preview_work.pending = Some(PendingPreview {
                        id,
                        tag,
                        gpu: native.primary_gpu,
                        renderer_generation,
                        context,
                        started: now,
                        submission,
                    });
                    true
                }
                None => {
                    native.preview_work.diagnostics.submission_failures += 1;
                    self.preview_renderer_failed(id);
                    deferred
                }
            }
        })();
        self.native = Some(native);
        self.schedule_preview_retry();
        if changed {
            self.refresh_internal_preview_pixels();
            self.request_output_redraw();
            self.render_all_outputs_once();
        }
        keep_polling
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct NeverReady;

    impl smithay::backend::renderer::sync::Fence for NeverReady {
        fn is_signaled(&self) -> bool {
            false
        }
        fn wait(&self) -> Result<(), smithay::backend::renderer::sync::Interrupted> {
            panic!("preview readiness must not wait")
        }
        fn is_exportable(&self) -> bool {
            false
        }
        fn export(&self) -> Option<std::os::fd::OwnedFd> {
            None
        }
    }

    #[test]
    fn never_ready_preview_fence_is_only_queried_until_timeout() {
        let fence = SyncPoint::from(NeverReady);
        for millis in 0..1000 {
            assert_eq!(
                preview_readiness(true, Duration::from_millis(millis), || fence.is_reached()),
                Readiness::Pending
            );
        }
        assert_eq!(
            preview_readiness(true, PENDING_TIMEOUT, || fence.is_reached()),
            Readiness::TimedOut
        );
    }

    #[test]
    fn pending_preview_requires_readiness_even_when_its_pixels_are_obsolete() {
        assert_eq!(
            preview_readiness(false, Duration::ZERO, || true),
            Readiness::Stale
        );
        assert_eq!(
            preview_readiness(false, Duration::ZERO, || false),
            Readiness::Pending
        );
        assert_eq!(
            preview_readiness(true, Duration::ZERO, || false),
            Readiness::Pending
        );
        assert_eq!(
            preview_readiness(true, PENDING_TIMEOUT, || false),
            Readiness::TimedOut
        );
        assert_eq!(
            preview_readiness(true, PENDING_TIMEOUT, || true),
            Readiness::Ready
        );
    }
}
