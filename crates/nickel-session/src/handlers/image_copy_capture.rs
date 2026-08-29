use smithay::{
    output::{Output, WeakOutput},
    reexports::wayland_server::{Client, protocol::wl_shm},
    utils::{Buffer, Rectangle, Transform},
    wayland::{
        image_capture_source::{
            ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler,
            OutputCaptureSourceState,
        },
        image_copy_capture::{
            BufferConstraints, CaptureFailureReason, Frame, ImageCopyCaptureHandler,
            ImageCopyCaptureState, Session, SessionRef,
        },
    },
};

use crate::NickelSession;

const MAX_CAPTURE_SESSIONS: usize = 16;
const MAX_PENDING_CAPTURE_FRAMES: usize = 32;
const _: () = assert!(MAX_CAPTURE_SESSIONS > 0);
const _: () = assert!(MAX_PENDING_CAPTURE_FRAMES >= MAX_CAPTURE_SESSIONS);
const _: () = assert!(MAX_PENDING_CAPTURE_FRAMES <= 64);

pub(crate) fn is_portal_capture_client(client: &Client) -> bool {
    client
        .get_data::<crate::state::ClientState>()
        .is_some_and(|state| state.portal_capture_allowed)
}

pub(crate) fn portal_capture_pid_allowed(pid: i32) -> bool {
    u32::try_from(pid)
        .ok()
        .and_then(|pid| std::fs::read_link(format!("/proc/{pid}/exe")).ok())
        .is_some_and(|executable| portal_capture_executable(&executable))
}

fn portal_capture_executable(executable: &std::path::Path) -> bool {
    executable.file_name().is_some_and(|name| {
        name.as_encoded_bytes() == b"xdg-desktop-portal-wlr"
            || name.as_encoded_bytes() == b"xdg-desktop-portal-wlr (deleted)"
    })
}

impl NickelSession {
    pub(crate) fn has_pending_image_copy_frames(&self, output: &Output) -> bool {
        self.pending_image_copy_frames
            .iter()
            .any(|(candidate, _)| candidate == output)
    }

    pub(crate) fn complete_image_copy_frames(
        &mut self,
        output: &Output,
        rgba: &[u8],
        width: usize,
        height: usize,
    ) {
        let mut remaining = Vec::with_capacity(self.pending_image_copy_frames.len());
        let mut ready = Vec::new();
        for (candidate, frame) in self.pending_image_copy_frames.drain(..) {
            if candidate == *output {
                ready.push(frame);
            } else {
                remaining.push((candidate, frame));
            }
        }
        self.pending_image_copy_frames = remaining;

        for frame in ready {
            if copy_rgba_to_shm(&frame.buffer(), rgba, width, height).is_err() {
                frame.fail(CaptureFailureReason::BufferConstraints);
                continue;
            }
            let damage = Rectangle::<i32, Buffer>::from_size((width as i32, height as i32).into());
            frame.success(Transform::Normal, vec![damage], self.start_time.elapsed());
        }
    }

    pub(crate) fn fail_image_copy_frames(&mut self, output: &Output, reason: CaptureFailureReason) {
        let mut remaining = Vec::with_capacity(self.pending_image_copy_frames.len());
        for (candidate, frame) in self.pending_image_copy_frames.drain(..) {
            if candidate == *output {
                frame.fail(reason);
            } else {
                remaining.push((candidate, frame));
            }
        }
        self.pending_image_copy_frames = remaining;
    }
}

fn copy_rgba_to_shm(
    buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    rgba: &[u8],
    width: usize,
    height: usize,
) -> Result<(), ()> {
    let row_bytes = width.checked_mul(4).ok_or(())?;
    let source_bytes = row_bytes.checked_mul(height).ok_or(())?;
    if rgba.len() < source_bytes {
        return Err(());
    }
    smithay::wayland::shm::with_buffer_contents_mut(buffer, |pointer, pool_len, data| {
        if usize::try_from(data.width).ok() != Some(width)
            || usize::try_from(data.height).ok() != Some(height)
            || usize::try_from(data.stride)
                .ok()
                .is_none_or(|stride| stride < row_bytes)
        {
            return Err(());
        }
        let offset = usize::try_from(data.offset).map_err(|_| ())?;
        let stride = usize::try_from(data.stride).map_err(|_| ())?;
        let required = offset
            .checked_add(stride.checked_mul(height.saturating_sub(1)).ok_or(())?)
            .and_then(|end| end.checked_add(row_bytes))
            .ok_or(())?;
        if required > pool_len {
            return Err(());
        }
        let mut converted = vec![0; row_bytes];
        for row in 0..height {
            let source = &rgba[row * row_bytes..(row + 1) * row_bytes];
            let source = match data.format {
                wl_shm::Format::Abgr8888 => source,
                wl_shm::Format::Xbgr8888 => {
                    convert_rgba_row(source, &mut converted, false, false);
                    &converted
                }
                wl_shm::Format::Argb8888 => {
                    convert_rgba_row(source, &mut converted, true, true);
                    &converted
                }
                wl_shm::Format::Xrgb8888 => {
                    convert_rgba_row(source, &mut converted, true, false);
                    &converted
                }
                _ => return Err(()),
            };
            let destination = pointer.wrapping_add(offset + row * stride);
            // SAFETY: both ranges were bounds-checked above, they do not overlap,
            // and no Rust reference is created to client-owned shared memory.
            unsafe { std::ptr::copy_nonoverlapping(source.as_ptr(), destination, row_bytes) };
        }
        Ok(())
    })
    .map_err(|_| ())?
}

fn convert_rgba_row(
    source: &[u8],
    converted: &mut [u8],
    swap_red_blue: bool,
    preserve_alpha: bool,
) {
    for (pixel, target) in source.chunks_exact(4).zip(converted.chunks_exact_mut(4)) {
        if swap_red_blue {
            target[..3].copy_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        } else {
            target[..3].copy_from_slice(&pixel[..3]);
        }
        target[3] = if preserve_alpha { pixel[3] } else { u8::MAX };
    }
}

impl ImageCaptureSourceHandler for NickelSession {}

impl OutputCaptureSourceHandler for NickelSession {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.output_capture_source_state
    }

    fn output_source_created(&mut self, source: ImageCaptureSource, output: &Output) {
        source.user_data().insert_if_missing(|| output.downgrade());
    }
}

impl ImageCopyCaptureHandler for NickelSession {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.image_copy_capture_state
    }

    fn capture_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        let output = source.user_data().get::<WeakOutput>()?.upgrade()?;
        let size = output
            .current_transform()
            .transform_size(output.current_mode()?.size);
        Some(BufferConstraints {
            size: (size.w, size.h).into(),
            shm: vec![
                wl_shm::Format::Xrgb8888,
                wl_shm::Format::Argb8888,
                wl_shm::Format::Xbgr8888,
                wl_shm::Format::Abgr8888,
            ],
            #[cfg(feature = "backend-udev")]
            dma: None,
        })
    }

    fn new_session(&mut self, session: Session) {
        if self.image_copy_sessions.len() >= MAX_CAPTURE_SESSIONS {
            tracing::warn!(
                limit = MAX_CAPTURE_SESSIONS,
                "rejecting image-copy session at resource limit"
            );
            session.stop();
            return;
        }
        self.image_copy_sessions.push(session);
    }

    fn frame(&mut self, session: &SessionRef, frame: Frame) {
        let output = session
            .source()
            .user_data()
            .get::<WeakOutput>()
            .and_then(WeakOutput::upgrade);
        let Some(output) = output else {
            frame.fail(CaptureFailureReason::Stopped);
            return;
        };
        if self.pending_image_copy_frames.len() >= MAX_PENDING_CAPTURE_FRAMES {
            tracing::warn!(
                limit = MAX_PENDING_CAPTURE_FRAMES,
                "rejecting image-copy frame at resource limit"
            );
            frame.fail(CaptureFailureReason::Unknown);
            return;
        }
        self.pending_image_copy_frames.push((output, frame));
        self.request_output_redraw();
    }

    fn session_destroyed(&mut self, session: SessionRef) {
        self.image_copy_sessions
            .retain(|candidate| candidate != &session);
    }
}

#[cfg(test)]
mod tests {
    use super::{convert_rgba_row, portal_capture_executable};
    use std::path::Path;

    #[test]
    fn renderer_rgba_converts_to_portal_shm_channel_orders() {
        let rgba = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut converted = [0; 8];
        convert_rgba_row(&rgba, &mut converted, false, true);
        assert_eq!(converted, rgba);
        convert_rgba_row(&rgba, &mut converted, false, false);
        assert_eq!(converted, [1, 2, 3, 255, 5, 6, 7, 255]);
        convert_rgba_row(&rgba, &mut converted, true, true);
        assert_eq!(converted, [3, 2, 1, 4, 7, 6, 5, 8]);
        convert_rgba_row(&rgba, &mut converted, true, false);
        assert_eq!(converted, [3, 2, 1, 255, 7, 6, 5, 255]);
    }

    #[test]
    fn capture_globals_are_reserved_for_the_portal_backend_executable() {
        assert!(portal_capture_executable(Path::new(
            "/usr/libexec/xdg-desktop-portal-wlr"
        )));
        assert!(portal_capture_executable(Path::new(
            "/usr/libexec/xdg-desktop-portal-wlr (deleted)"
        )));
        assert!(!portal_capture_executable(Path::new(
            "/tmp/xdg-desktop-portal-wlr-helper"
        )));
        assert!(!portal_capture_executable(Path::new(
            "/usr/bin/wayland-info"
        )));
    }
}
