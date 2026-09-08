use super::*;

pub struct PreviewFrame {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

pub const PREVIEW_WIDTH: usize = 240;
pub const PREVIEW_HEIGHT: usize = 135;
pub const PREVIEW_FRAME_BYTES: usize = PREVIEW_WIDTH * PREVIEW_HEIGHT * 4;
pub const PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER: usize = 7;
pub const PREVIEW_ENTRY_CAPACITY: usize = PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER * 2;
pub const PREVIEW_BYTE_CAPACITY: usize = PREVIEW_ENTRY_CAPACITY * PREVIEW_FRAME_BYTES;

#[derive(Clone, Copy, Default)]
pub(super) struct PreviewFailure {
    attempts: u8,
    retry_at: Option<Instant>,
}

impl PreviewFailure {
    fn record(&mut self, now: Instant) {
        // Four delayed retries (100, 200, 400, 800 ms), then stop until a
        // new visible admission or source identity resets this failure state.
        // Ordinary animated commits must not reopen an unsupported capture path.
        self.attempts = self.attempts.saturating_add(1).min(5);
        self.retry_at =
            (self.attempts < 5).then(|| now + Duration::from_millis(100 << (self.attempts - 1)));
    }

    fn ready(self, now: Instant) -> bool {
        self.retry_at.is_some_and(|deadline| now >= deadline)
    }
}

pub(crate) fn preview_capture_dimensions(width: i32, height: i32) -> Option<(u16, u16)> {
    if width <= 0 || height <= 0 {
        return None;
    }
    let width = u64::try_from(width).ok()?;
    let height = u64::try_from(height).ok()?;
    let max_width = PREVIEW_WIDTH as u64;
    let max_height = PREVIEW_HEIGHT as u64;
    let (fitted_width, fitted_height) = if width * max_height > max_width * height {
        (max_width, (height * max_width / width).max(1))
    } else {
        ((width * max_height / height).max(1), max_height)
    };
    Some((
        u16::try_from(fitted_width).ok()?,
        u16::try_from(fitted_height).ok()?,
    ))
}

pub(crate) fn bounded_preview_ids(ids: Vec<WindowId>, selected: usize) -> Vec<WindowId> {
    let start = selected
        .saturating_sub(PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER / 2)
        .min(
            ids.len()
                .saturating_sub(PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER),
        );
    ids.into_iter()
        .skip(start)
        .take(PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER)
        .collect()
}

pub(crate) fn admitted_preview_ids(
    switcher: &[WindowId],
    overlay: &[WindowId],
) -> HashSet<WindowId> {
    switcher
        .iter()
        .chain(overlay)
        .copied()
        .fold(Vec::new(), |mut ids, id| {
            if ids.len() < PREVIEW_ENTRY_CAPACITY && !ids.contains(&id) {
                ids.push(id);
            }
            ids
        })
        .into_iter()
        .collect()
}

pub(super) fn retired_preview_ids(
    current: &HashSet<WindowId>,
    admitted: &HashSet<WindowId>,
) -> Vec<WindowId> {
    current.difference(admitted).copied().collect()
}

pub(crate) fn advance_preview_content_generation(
    generations: &mut HashMap<WindowId, u64>,
    attempted: &mut HashMap<WindowId, (u64, u64)>,
    id: WindowId,
) -> u64 {
    let generation = generations
        .entry(id)
        .and_modify(|generation| *generation = generation.wrapping_add(1).max(1))
        .or_insert(1);
    attempted.remove(&id);
    *generation
}

pub(crate) fn protocol_preview_from_cached(
    window: nickel_session_protocol::WindowId,
    frame: Option<&PreviewFrame>,
) -> Option<ProtocolPreview> {
    let frame = frame?;
    Some(ProtocolPreview {
        window,
        width: frame.width,
        height: frame.height,
        rgba: frame.rgba.clone(),
    })
}

pub(crate) fn reuse_preview_pixels(mut rgba: Vec<u8>, mapped: &[u8]) -> Vec<u8> {
    rgba.clear();
    rgba.extend_from_slice(mapped);
    rgba
}

pub(crate) fn preview_mapping_has_exact_size(mapped: &[u8], width: u16, height: u16) -> bool {
    usize::from(width)
        .checked_mul(usize::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .is_some_and(|expected| mapped.len() == expected && expected <= PREVIEW_FRAME_BYTES)
}

pub(crate) fn record_preview_capture_attempt(
    attempted: &mut HashMap<WindowId, (u64, u64)>,
    id: WindowId,
    content_generation: u64,
    render_wave: u64,
) -> bool {
    if attempted
        .get(&id)
        .is_some_and(|(generation, _)| *generation == content_generation)
    {
        return false;
    }
    attempted.insert(id, (content_generation, render_wave));
    true
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PreviewCacheCounters {
    pub(super) peak_bytes: u64,
    pub(super) admissions: u64,
    pub(super) evictions: u64,
    pub(super) invalidations: u64,
    pub(super) captures: u64,
    pub(super) skipped_unchanged: u64,
    pub(super) readback_bytes: u64,
    pub(super) protocol_copy_bytes: u64,
    pub(super) protocol_raw_copy_bytes: u64,
    pub(super) protocol_base64_bytes: u64,
    pub(super) protocol_json_payload_bytes: u64,
    pub(super) protocol_framed_copy_bytes: u64,
    pub(super) capture_failures: u64,
    // Presentation changes only when visible pixels arrive or retire. Source
    // commits request capture independently, without rebuilding the old overlay.
    pub(super) presentation_generation: u64,
}

impl NickelSession {
    pub(super) fn update_preview_peak_bytes(&mut self) {
        self.preview_counters.peak_bytes = self
            .preview_counters
            .peak_bytes
            .max(self.preview_bytes() as u64);
    }

    pub(crate) fn preview_bytes(&self) -> usize {
        self.preview_frames
            .values()
            .map(|frame| frame.rgba.len())
            .sum::<usize>()
            + self
                .preview_spares
                .values()
                .map(Vec::capacity)
                .sum::<usize>()
    }

    #[cfg(feature = "backend-udev")]
    pub(crate) fn preview_generation(&self) -> u64 {
        self.preview_counters.presentation_generation
    }

    pub(super) fn drop_preview_frame(&mut self, id: &WindowId) {
        self.preview_dirty.remove(id);
        self.preview_content_generation.remove(id);
        self.preview_attempted.remove(id);
        self.preview_retry_pending.remove(id);
        self.preview_failures.remove(id);
        let spare = self.preview_spares.remove(id).is_some();
        let frame = self.preview_frames.remove(id).is_some();
        if spare || frame {
            self.preview_counters.evictions += 1;
        }
        if frame {
            self.preview_counters.presentation_generation = self
                .preview_counters
                .presentation_generation
                .wrapping_add(1)
                .max(1);
        }
    }

    pub(super) fn reconcile_preview_admission(&mut self) {
        let admitted = admitted_preview_ids(
            &self.preview_switcher_interest,
            &self.preview_overlay_interest,
        );
        if admitted != self.preview_admitted {
            self.preview_retry_epoch = self.preview_retry_epoch.wrapping_add(1).max(1);
            self.cancel_preview_retry_timer();
            self.preview_retry_pending
                .retain(|id| admitted.contains(id));
        }
        let retired = retired_preview_ids(&self.preview_admitted, &admitted);
        for id in retired {
            self.drop_preview_frame(&id);
        }
        for id in admitted.difference(&self.preview_admitted) {
            self.preview_counters.admissions += 1;
            self.preview_dirty.insert(*id);
            advance_preview_content_generation(
                &mut self.preview_content_generation,
                &mut self.preview_attempted,
                *id,
            );
        }
        self.preview_admitted = admitted;
        #[cfg(feature = "backend-udev")]
        self.reconcile_native_preview_interest();
        self.schedule_preview_retry();
    }

    pub(super) fn set_switcher_preview_interest(&mut self, ids: Vec<WindowId>) {
        self.preview_switcher_interest = ids;
        self.reconcile_preview_admission();
    }

    pub(super) fn set_overlay_preview_interest(&mut self, ids: Vec<WindowId>) {
        self.preview_overlay_interest = ids;
        self.reconcile_preview_admission();
    }

    pub(super) fn clear_switcher_preview_interest(&mut self) {
        self.preview_switcher_interest.clear();
        self.reconcile_preview_admission();
    }

    pub(super) fn clear_overlay_preview_interest(&mut self) {
        self.preview_overlay_interest.clear();
        self.reconcile_preview_admission();
    }

    pub(crate) fn reassociate_preview_surface(&mut self, id: WindowId) {
        if self.preview_admitted.contains(&id) {
            self.preview_failures.remove(&id);
            self.preview_retry_pending.remove(&id);
            if self.preview_frames.remove(&id).is_some() {
                self.preview_counters.presentation_generation = self
                    .preview_counters
                    .presentation_generation
                    .wrapping_add(1)
                    .max(1);
            }
            self.preview_counters.invalidations += 1;
            self.preview_dirty.insert(id);
            advance_preview_content_generation(
                &mut self.preview_content_generation,
                &mut self.preview_attempted,
                id,
            );
        }
    }

    pub(crate) fn invalidate_preview_for_surface(&mut self, surface: &WlSurface) {
        let mut root = surface.clone();
        while let Some(parent) = smithay::wayland::compositor::get_parent(&root) {
            root = parent;
        }
        if let Some(id) = self.surface_windows.get(&root.id()).copied() {
            self.invalidate_preview_content(id);
        }
    }

    pub(super) fn invalidate_preview_content(&mut self, id: WindowId) {
        if self.preview_admitted.contains(&id) {
            // Keep the last completed pixels presentable while replacement work
            // is pending. Dirty content is not a change to the displayed image.
            self.preview_dirty.insert(id);
            advance_preview_content_generation(
                &mut self.preview_content_generation,
                &mut self.preview_attempted,
                id,
            );
            self.preview_counters.invalidations += 1;
        }
    }

    pub(crate) fn begin_preview_render_wave(&mut self) -> u64 {
        self.preview_render_wave = self.preview_render_wave.wrapping_add(1).max(1);
        self.preview_render_wave
    }

    #[cfg(feature = "backend-udev")]
    fn reconcile_native_preview_interest(&mut self) {
        if let Some(native) = self.native.as_mut()
            && let Some(token) = native.retain_native_preview_interest(&self.preview_admitted)
        {
            self.event_loop_handle.remove(token);
        }
    }

    #[cfg(feature = "backend-udev")]
    pub(crate) fn preview_capture_work_pending(&self) -> bool {
        let now = Instant::now();
        self.preview_admitted.iter().any(|id| {
            (self.preview_dirty.contains(id) || !self.preview_frames.contains_key(id))
                && self.preview_retry_ready(*id, now)
                && self
                    .preview_attempted
                    .get(id)
                    .is_none_or(|(generation, _)| {
                        *generation
                            != self
                                .preview_content_generation
                                .get(id)
                                .copied()
                                .unwrap_or(1)
                    })
        })
    }

    #[cfg(feature = "backend-udev")]
    pub(crate) fn preview_capture_tag(&self, id: WindowId) -> Option<(u64, u64)> {
        self.preview_admitted.contains(&id).then(|| {
            (
                self.preview_retry_epoch,
                self.preview_content_generation
                    .get(&id)
                    .copied()
                    .unwrap_or(1),
            )
        })
    }

    #[cfg(feature = "backend-udev")]
    pub(crate) fn defer_preview_capture(&mut self, id: WindowId) {
        self.preview_attempted.remove(&id);
    }

    pub(crate) fn preview_capture_candidates(&mut self, wave: u64) -> Vec<(WindowId, Window)> {
        let now = Instant::now();
        let admitted = self.preview_admitted.clone();
        let dirty = self.preview_dirty.clone();
        let mut candidates = Vec::new();
        let windows = self.space.elements().cloned().collect::<Vec<_>>();
        for window in windows {
            let Some(id) = window
                .wl_surface()
                .and_then(|surface| self.surface_windows.get(&surface.id()))
                .copied()
            else {
                continue;
            };
            if !admitted.contains(&id) {
                continue;
            }
            if !self.preview_retry_ready(id, now) {
                continue;
            }
            if dirty.contains(&id) || !self.preview_frames.contains_key(&id) {
                let generation = self
                    .preview_content_generation
                    .get(&id)
                    .copied()
                    .unwrap_or(1);
                if !record_preview_capture_attempt(
                    &mut self.preview_attempted,
                    id,
                    generation,
                    wave,
                ) {
                    continue;
                }
                candidates.push((id, window));
            } else {
                self.preview_counters.skipped_unchanged += 1;
            }
        }
        candidates
    }

    pub(crate) fn take_preview_capture_buffer(
        &mut self,
        id: WindowId,
    ) -> (Vec<u8>, Option<(u16, u16)>) {
        self.preview_frames.remove(&id).map_or_else(
            || {
                (
                    self.preview_spares
                        .remove(&id)
                        .unwrap_or_else(|| vec![0; PREVIEW_FRAME_BYTES]),
                    None,
                )
            },
            |frame| (frame.rgba, Some((frame.width, frame.height))),
        )
    }

    pub(crate) fn preview_capture_failed(
        &mut self,
        id: WindowId,
        rgba: Vec<u8>,
        previous_dimensions: Option<(u16, u16)>,
    ) {
        self.preview_counters.capture_failures += 1;
        if let Some((width, height)) = previous_dimensions {
            self.preview_frames.insert(
                id,
                PreviewFrame {
                    width,
                    height,
                    rgba,
                },
            );
        } else {
            self.preview_spares.insert(id, rgba);
        }
        self.record_preview_failure(id, Instant::now());
        self.update_preview_peak_bytes();
    }

    pub(crate) fn preview_renderer_failed(&mut self, id: WindowId) {
        self.preview_counters.capture_failures += 1;
        self.record_preview_failure(id, Instant::now());
    }

    pub(super) fn record_preview_failure(&mut self, id: WindowId, now: Instant) {
        // Like capture storage, failure metadata belongs only to visible admitted
        // IDs; it cannot grow with the number of windows ever seen by the shell.
        if !self.preview_admitted.contains(&id) {
            return;
        }
        let failure = self.preview_failures.entry(id).or_default();
        failure.record(now);
        if failure.retry_at.is_some() {
            self.preview_retry_pending.insert(id);
        } else {
            self.preview_retry_pending.remove(&id);
            if self.preview_retry_pending.is_empty() {
                self.cancel_preview_retry_timer();
            }
        }
    }

    pub(super) fn preview_retry_ready(&self, id: WindowId, now: Instant) -> bool {
        self.preview_failures
            .get(&id)
            .is_none_or(|failure| failure.ready(now))
    }

    pub(super) fn ready_preview_retries(&mut self, now: Instant) -> bool {
        let mut ready = false;
        self.preview_retry_pending.retain(|id| {
            if self
                .preview_failures
                .get(id)
                .is_some_and(|failure| failure.ready(now))
            {
                // Retry the newest source generation, not a fabricated commit.
                self.preview_attempted.remove(id);
                ready = true;
                false
            } else {
                true
            }
        });
        ready
    }

    fn cancel_preview_retry_timer(&mut self) {
        if let Some((_, token)) = self.preview_retry_scheduled.take() {
            self.event_loop_handle.remove(token);
        }
    }

    pub(crate) fn schedule_preview_retry(&mut self) {
        self.schedule_preview_retry_after(Duration::ZERO);
    }

    pub(crate) fn schedule_preview_retry_after(&mut self, delay: std::time::Duration) {
        if self.preview_retry_pending.is_empty() || self.preview_retry_scheduled.is_some() {
            return;
        }
        let Some(deadline) = self
            .preview_retry_pending
            .iter()
            .filter_map(|id| self.preview_failures.get(id)?.retry_at)
            .min()
        else {
            return;
        };
        let epoch = self.preview_retry_epoch;
        let timer =
            Timer::from_duration(delay.max(deadline.saturating_duration_since(Instant::now())));
        match self
            .event_loop_handle
            .insert_source(timer, move |_, _, data| {
                if data.preview_retry_epoch == epoch
                    && data
                        .preview_retry_scheduled
                        .as_ref()
                        .is_some_and(|(scheduled, _)| *scheduled == epoch)
                {
                    data.preview_retry_scheduled = None;
                    if data.ready_preview_retries(Instant::now()) {
                        data.request_output_redraw();
                        #[cfg(feature = "backend-udev")]
                        if data.native.is_some() {
                            data.render_all_outputs_once();
                        }
                    }
                    data.schedule_preview_retry();
                }
                TimeoutAction::Drop
            }) {
            Ok(token) => self.preview_retry_scheduled = Some((epoch, token)),
            Err(error) => tracing::warn!(?error, "failed to schedule preview capture retry"),
        }
    }

    pub(super) fn record_preview_protocol_encoding(
        &mut self,
        json_payload_bytes: usize,
        framed_bytes: usize,
    ) {
        self.preview_counters.protocol_json_payload_bytes += json_payload_bytes as u64;
        self.preview_counters.protocol_framed_copy_bytes += framed_bytes as u64;
        self.preview_counters.protocol_copy_bytes += (json_payload_bytes + framed_bytes) as u64;
    }

    pub(crate) fn store_preview(&mut self, id: WindowId, frame: PreviewFrame) {
        // Capture leases are created only for admitted IDs after the byte/entry ceiling has
        // already been reconciled. No event dispatch can change admission while the synchronous
        // renderer call owns the lease, so commit is intentionally infallible.
        assert!(self.preview_admitted.contains(&id));
        assert!(preview_mapping_has_exact_size(
            &frame.rgba,
            frame.width,
            frame.height
        ));
        self.preview_counters.captures += 1;
        self.preview_counters.presentation_generation = self
            .preview_counters
            .presentation_generation
            .wrapping_add(1)
            .max(1);
        self.preview_counters.readback_bytes += frame.rgba.len() as u64;
        self.preview_frames.insert(id, frame);
        self.preview_spares.remove(&id);
        self.preview_failures.remove(&id);
        self.preview_retry_pending.remove(&id);
        if self.preview_retry_pending.is_empty() {
            self.cancel_preview_retry_timer();
        }
        self.preview_dirty.remove(&id);
        self.preview_attempted.remove(&id);
        self.update_preview_peak_bytes();
    }

    pub(super) fn clear_all_previews(&mut self) {
        if !self.preview_frames.is_empty() {
            self.preview_counters.presentation_generation = self
                .preview_counters
                .presentation_generation
                .wrapping_add(1)
                .max(1);
        }
        self.preview_counters.evictions +=
            (self.preview_frames.len() + self.preview_spares.len()) as u64;
        self.preview_switcher_interest.clear();
        self.preview_overlay_interest.clear();
        self.preview_admitted.clear();
        self.preview_dirty.clear();
        self.preview_content_generation.clear();
        self.preview_attempted.clear();
        self.preview_frames.clear();
        self.preview_spares.clear();
        self.preview_retry_pending.clear();
        self.preview_failures.clear();
        self.preview_retry_epoch = self.preview_retry_epoch.wrapping_add(1).max(1);
        self.cancel_preview_retry_timer();
        self.preview_frames.shrink_to_fit();
        self.preview_spares.shrink_to_fit();
        self.preview_switcher_interest.shrink_to_fit();
        self.preview_overlay_interest.shrink_to_fit();
        self.preview_admitted.shrink_to_fit();
        self.preview_dirty.shrink_to_fit();
        self.preview_content_generation.shrink_to_fit();
        self.preview_attempted.shrink_to_fit();
        self.preview_retry_pending.shrink_to_fit();
        self.preview_failures.shrink_to_fit();
        #[cfg(feature = "backend-udev")]
        self.reconcile_native_preview_interest();
        #[cfg(feature = "backend-udev")]
        if let Some(native) = self.native.as_mut() {
            native.clear_task_switcher_cache();
        }
    }
}
