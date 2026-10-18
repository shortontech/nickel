//! Owns preview identity, hover deadlines, and retained pixels independently of native effects.

use crate::model::{OpenWindow, WindowId};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) struct PreviewState {
    group: Option<usize>,
    generation: u64,
    pending: Option<(usize, Instant)>,
    focus_requested: bool,
    pointer_inside: bool,
    leave_deadline: Option<Instant>,
    hovered: Option<WindowId>,
    images: HashMap<WindowId, Arc<image::RgbaImage>>,
    refresh_deadline: Option<Instant>,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            group: None,
            generation: 1,
            pending: None,
            focus_requested: false,
            pointer_inside: false,
            leave_deadline: None,
            hovered: None,
            images: HashMap::new(),
            refresh_deadline: None,
        }
    }
}
impl PreviewState {
    pub(super) fn group(&self) -> Option<usize> {
        self.group
    }
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }
    pub(super) fn pending(&self) -> Option<(usize, Instant)> {
        self.pending
    }
    pub(super) fn leave_deadline(&self) -> Option<Instant> {
        self.leave_deadline
    }
    pub(super) fn refresh_deadline(&self) -> Option<Instant> {
        self.refresh_deadline
    }
    pub(super) fn images(&self) -> &HashMap<WindowId, Arc<image::RgbaImage>> {
        &self.images
    }
    pub(super) fn cancel_pending(&mut self) {
        self.pending = None;
    }
    pub(super) fn cancel_leave(&mut self) -> bool {
        self.leave_deadline.take().is_some()
    }
    pub(super) fn take_focus_request(&mut self) -> bool {
        std::mem::take(&mut self.focus_requested)
    }
    pub(super) fn advance_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
    }
    pub(super) fn clear_images(&mut self) {
        self.images.clear();
    }
    pub(super) fn invalidate_refresh(&mut self) {
        self.refresh_deadline = None;
    }
    pub(super) fn refreshed_at(&mut self, now: Instant) {
        self.refresh_deadline = Some(now + PREVIEW_REFRESH_INTERVAL);
    }
    pub(super) fn set_hovered(&mut self, hovered: Option<WindowId>) -> bool {
        let changed = self.hovered != hovered;
        self.hovered = hovered;
        changed
    }
    pub(super) fn open(&mut self, index: usize) -> bool {
        self.cancel_pending();
        if self.group == Some(index) {
            return false;
        }
        self.group = Some(index);
        self.advance_generation();
        self.clear_images();
        self.invalidate_refresh();
        self.hovered = None;
        true
    }
    pub(super) fn close(&mut self) {
        self.advance_generation();
        self.group = None;
        self.pending = None;
        self.focus_requested = false;
        self.pointer_inside = false;
        self.leave_deadline = None;
        self.hovered = None;
        self.clear_images();
        self.invalidate_refresh();
    }
    /// Returns whether the host must clear its native window highlight.
    pub(super) fn pointer_entered(&mut self, entered: bool, now: Instant) -> Option<bool> {
        if self.pointer_inside == entered {
            return None;
        }
        self.pointer_inside = entered;
        if entered {
            self.leave_deadline = None;
            Some(false)
        } else {
            self.leave_deadline = Some(now + PREVIEW_LEAVE_DELAY);
            Some(self.hovered.take().is_some())
        }
    }
    pub(super) fn retain_windows(&mut self, windows: &[OpenWindow]) {
        retain_preview_generation(&mut self.images, windows);
    }
    pub(super) fn update_image(&mut self, id: WindowId, image: image::RgbaImage) -> bool {
        update_preview_image(&mut self.images, id, image)
    }
    #[cfg(target_os = "linux")]
    pub(super) fn sync_native_pixels<'a>(
        &mut self,
        windows: &[WindowId],
        mut frame_for: impl FnMut(WindowId) -> Option<(u16, u16, &'a [u8])>,
    ) -> bool {
        let previous = self.images.len();
        self.images
            .retain(|id, _| windows.contains(id) && frame_for(*id).is_some());
        let mut changed = previous != self.images.len();
        for &id in windows {
            let Some((width, height, rgba)) = frame_for(id) else {
                continue;
            };
            if self.images.get(&id).is_some_and(|image| {
                image.dimensions() == (u32::from(width), u32::from(height))
                    && image.as_raw().as_slice() == rgba
            }) {
                continue;
            }
            // The session retains its completed frame for other consumers. Copy
            // once into the existing UI owner, never through JSON or self-RPC.
            if let Some(image) =
                image::RgbaImage::from_raw(u32::from(width), u32::from(height), rgba.to_vec())
            {
                changed |= update_preview_image(&mut self.images, id, image);
            }
        }
        changed
    }
}

const PREVIEW_LEAVE_DELAY: Duration = Duration::from_millis(500);
pub(super) const PREVIEW_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
pub(super) const PREVIEW_CACHE_CAPACITY: usize = 32;

pub(super) fn preview_refresh_due(deadline: Option<Instant>, now: Instant) -> bool {
    deadline.is_none_or(|deadline| now >= deadline)
}

pub(super) fn update_preview_image(
    images: &mut HashMap<crate::model::WindowId, Arc<image::RgbaImage>>,
    window: crate::model::WindowId,
    image: image::RgbaImage,
) -> bool {
    if images
        .get(&window)
        .is_some_and(|current| **current == image)
    {
        return false;
    }
    images.insert(window, Arc::new(image));
    true
}

pub(super) fn retain_preview_generation(
    images: &mut HashMap<crate::model::WindowId, Arc<image::RgbaImage>>,
    windows: &[OpenWindow],
) {
    images.retain(|window, _| {
        windows
            .iter()
            .take(PREVIEW_CACHE_CAPACITY)
            .any(|candidate| candidate.id == *window)
    });
}
