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
            BufferConstraints, CaptureFailureReason, Frame, FrameRef, ImageCopyCaptureHandler,
            ImageCopyCaptureState, Session, SessionRef,
        },
    },
};

use crate::NickelSession;

const MAX_CAPTURE_SESSIONS: usize = 16;
const MAX_PENDING_CAPTURE_FRAMES: usize = 32;
const MAX_PENDING_CAPTURE_FRAMES_PER_OUTPUT: usize = 16;
const MAX_PENDING_CAPTURE_FRAMES_PER_SESSION: usize = 4;
const MAX_PENDING_CAPTURE_BYTES: usize = 256 * 1024 * 1024;
const MAX_PENDING_CAPTURE_BYTES_PER_OUTPUT: usize = 128 * 1024 * 1024;
const MAX_PENDING_CAPTURE_BYTES_PER_SESSION: usize = 64 * 1024 * 1024;
const _: () = assert!(MAX_CAPTURE_SESSIONS > 0);
const _: () = assert!(MAX_PENDING_CAPTURE_FRAMES >= MAX_CAPTURE_SESSIONS);
const _: () = assert!(MAX_PENDING_CAPTURE_FRAMES <= 64);

pub(crate) struct PendingImageCopyFrame {
    output: Output,
    session: SessionRef,
    frame: Frame,
    shared_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImageCopyCaptureDiagnostics {
    pub frames: usize,
    pub sessions: usize,
    pub outputs: usize,
    pub client_owned_shared_bytes: usize,
    pub session_owned_cpu_bytes: usize,
    pub retained_gpu_readback_bytes: usize,
}

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

fn capture_transform_supported(transform: Transform) -> bool {
    // Native outputs are Normal. The nested backend deliberately presents Flipped180 and its
    // readback path restores top-down rows. Other transforms require column/axis conversion that
    // the direct-copy path does not perform, so do not advertise invalid constraints for them.
    matches!(transform, Transform::Normal | Transform::Flipped180)
}

impl NickelSession {
    pub(crate) fn has_pending_image_copy_frames(&self, output: &Output) -> bool {
        self.pending_image_copy_frames
            .iter()
            .any(|pending| pending.output == *output)
    }

    pub(crate) fn image_copy_capture_diagnostics(&self) -> ImageCopyCaptureDiagnostics {
        let mut sessions = Vec::<SessionRef>::new();
        let mut outputs = Vec::<String>::new();
        for pending in &self.pending_image_copy_frames {
            if !sessions.contains(&pending.session) {
                sessions.push(pending.session.clone());
            }
            let output = pending.output.name();
            if !outputs.contains(&output) {
                outputs.push(output);
            }
        }
        ImageCopyCaptureDiagnostics {
            frames: self.pending_image_copy_frames.len(),
            sessions: sessions.len(),
            outputs: outputs.len(),
            client_owned_shared_bytes: self
                .pending_image_copy_frames
                .iter()
                .map(|pending| pending.shared_bytes)
                .sum(),
            // Pending frames retain protocol handles only. Readback mappings are request-scoped,
            // and portal delivery does not retain a normalized CPU image.
            session_owned_cpu_bytes: 0,
            retained_gpu_readback_bytes: 0,
        }
    }

    pub(crate) fn complete_image_copy_frames(
        &mut self,
        output: &Output,
        mapped: &[u8],
        width: usize,
        height: usize,
        flipped: bool,
    ) {
        let ready = retire_matching(&mut self.pending_image_copy_frames, |pending| {
            pending.output == *output
        });
        let diagnostics = self.image_copy_capture_diagnostics();
        tracing::debug!(
            pending_frames = diagnostics.frames,
            pending_sessions = diagnostics.sessions,
            pending_outputs = diagnostics.outputs,
            client_owned_shared_bytes = diagnostics.client_owned_shared_bytes,
            session_owned_cpu_bytes = diagnostics.session_owned_cpu_bytes,
            retained_gpu_readback_bytes = diagnostics.retained_gpu_readback_bytes,
            output = %output.name(),
            "retired completed image-copy frames"
        );

        for pending in ready {
            let frame = pending.frame;
            if copy_mapped_rgba_to_shm(&frame.buffer(), mapped, width, height, flipped).is_err() {
                frame.fail(CaptureFailureReason::BufferConstraints);
                continue;
            }
            let damage = Rectangle::<i32, Buffer>::from_size((width as i32, height as i32).into());
            frame.success(Transform::Normal, vec![damage], self.start_time.elapsed());
        }
    }

    pub(crate) fn fail_image_copy_frames(&mut self, output: &Output, reason: CaptureFailureReason) {
        let failed = retire_matching(&mut self.pending_image_copy_frames, |pending| {
            pending.output == *output
        });
        let retired_frames = failed.len();
        let retired_client_owned_shared_bytes = failed
            .iter()
            .map(|pending| pending.shared_bytes)
            .sum::<usize>();
        for pending in failed {
            pending.frame.fail(reason);
        }
        let diagnostics = self.image_copy_capture_diagnostics();
        tracing::debug!(
            pending_frames = diagnostics.frames,
            client_owned_shared_bytes = diagnostics.client_owned_shared_bytes,
            output = %output.name(),
            retired_frames,
            retired_client_owned_shared_bytes,
            "retired failed image-copy frames"
        );
    }

    pub(crate) fn fail_all_image_copy_frames(&mut self, reason: CaptureFailureReason) {
        for pending in self.pending_image_copy_frames.drain(..) {
            pending.frame.fail(reason);
        }
        tracing::debug!(
            pending_frames = 0,
            client_owned_shared_bytes = 0,
            session_owned_cpu_bytes = 0,
            retained_gpu_readback_bytes = 0,
            "retired all image-copy frames"
        );
    }
}

fn copy_mapped_rgba_to_shm(
    buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    mapped: &[u8],
    width: usize,
    height: usize,
    flipped: bool,
) -> Result<(), ()> {
    let row_bytes = width.checked_mul(4).ok_or(())?;
    let source_bytes = row_bytes.checked_mul(height).ok_or(())?;
    if mapped.len() < source_bytes {
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
        visit_mapped_rows(
            mapped,
            width,
            height,
            flipped,
            data.format,
            |destination_y, source| {
                let destination = pointer.wrapping_add(offset + destination_y * stride);
                // SAFETY: both ranges were bounds-checked above, they do not overlap,
                // and no Rust reference is created to client-owned shared memory.
                unsafe { std::ptr::copy_nonoverlapping(source.as_ptr(), destination, row_bytes) };
                Ok(())
            },
        )
    })
    .map_err(|_| ())?
}

fn visit_mapped_rows(
    mapped: &[u8],
    width: usize,
    height: usize,
    flipped: bool,
    format: wl_shm::Format,
    mut visit: impl FnMut(usize, &[u8]) -> Result<(), ()>,
) -> Result<(), ()> {
    let row_bytes = width.checked_mul(4).ok_or(())?;
    if mapped.len() < row_bytes.checked_mul(height).ok_or(())? {
        return Err(());
    }
    let mut converted = vec![0; row_bytes];
    for destination_y in 0..height {
        let source_y = if flipped {
            destination_y
        } else {
            height - 1 - destination_y
        };
        let source = &mapped[source_y * row_bytes..(source_y + 1) * row_bytes];
        let source = match format {
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
        visit(destination_y, source)?;
    }
    Ok(())
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
        if !capture_transform_supported(output.current_transform()) {
            return None;
        }
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
        if !capture_transform_supported(output.current_transform()) {
            frame.fail(CaptureFailureReason::Stopped);
            return;
        }
        let shared_bytes = estimated_shm_bytes(&frame.buffer()).unwrap_or(usize::MAX);
        let diagnostics = self.image_copy_capture_diagnostics();
        let output_bytes = self
            .pending_image_copy_frames
            .iter()
            .filter(|pending| pending.output == output)
            .map(|pending| pending.shared_bytes)
            .sum::<usize>();
        let output_frames = self
            .pending_image_copy_frames
            .iter()
            .filter(|pending| pending.output == output)
            .count();
        let session_bytes = self
            .pending_image_copy_frames
            .iter()
            .filter(|pending| pending.session == *session)
            .map(|pending| pending.shared_bytes)
            .sum::<usize>();
        let session_frames = self
            .pending_image_copy_frames
            .iter()
            .filter(|pending| pending.session == *session)
            .count();
        let over_limit = !capture_admitted(
            diagnostics.frames,
            diagnostics.client_owned_shared_bytes,
            output_frames,
            output_bytes,
            session_frames,
            session_bytes,
            shared_bytes,
        );
        if over_limit {
            tracing::warn!(
                frames = diagnostics.frames,
                client_owned_shared_bytes = diagnostics.client_owned_shared_bytes,
                output = %output.name(),
                output_frames,
                output_client_owned_shared_bytes = output_bytes,
                session_frames,
                session_client_owned_shared_bytes = session_bytes,
                requested_shared_bytes = shared_bytes,
                "rejecting image-copy frame at count or byte limit"
            );
            frame.fail(CaptureFailureReason::Unknown);
            return;
        }
        self.pending_image_copy_frames.push(PendingImageCopyFrame {
            output,
            session: session.clone(),
            frame,
            shared_bytes,
        });
        let diagnostics = self.image_copy_capture_diagnostics();
        tracing::debug!(
            pending_frames = diagnostics.frames,
            client_owned_shared_bytes = diagnostics.client_owned_shared_bytes,
            output = %self.pending_image_copy_frames.last().expect("just admitted").output.name(),
            session_frames = session_frames + 1,
            session_client_owned_shared_bytes = session_bytes + shared_bytes,
            "admitted image-copy frame"
        );
        self.request_output_redraw();
    }

    fn session_destroyed(&mut self, session: SessionRef) {
        let retired = retire_matching(&mut self.pending_image_copy_frames, |pending| {
            pending.session == session
        });
        let retired_frames = retired.len();
        let retired_client_owned_shared_bytes = retired
            .iter()
            .map(|pending| pending.shared_bytes)
            .sum::<usize>();
        for pending in retired {
            pending.frame.fail(CaptureFailureReason::Stopped);
        }
        let diagnostics = self.image_copy_capture_diagnostics();
        tracing::debug!(
            pending_frames = diagnostics.frames,
            client_owned_shared_bytes = diagnostics.client_owned_shared_bytes,
            retired_frames,
            retired_client_owned_shared_bytes,
            "retired destroyed image-copy session"
        );
        self.image_copy_sessions
            .retain(|candidate| candidate != &session);
    }

    fn frame_aborted(&mut self, frame: FrameRef) {
        let retired = retire_matching(&mut self.pending_image_copy_frames, |pending| {
            pending.frame == frame
        });
        let retired_client_owned_shared_bytes = retired
            .iter()
            .map(|pending| pending.shared_bytes)
            .sum::<usize>();
        let diagnostics = self.image_copy_capture_diagnostics();
        tracing::debug!(
            pending_frames = diagnostics.frames,
            client_owned_shared_bytes = diagnostics.client_owned_shared_bytes,
            retired_frames = retired.len(),
            retired_client_owned_shared_bytes,
            "retired aborted image-copy frame"
        );
    }
}

fn retire_matching<T>(items: &mut Vec<T>, mut predicate: impl FnMut(&T) -> bool) -> Vec<T> {
    let mut remaining = Vec::with_capacity(items.len());
    let mut retired = Vec::new();
    for item in items.drain(..) {
        if predicate(&item) {
            retired.push(item);
        } else {
            remaining.push(item);
        }
    }
    *items = remaining;
    retired
}

fn capture_admitted(
    frames: usize,
    total_bytes: usize,
    output_frames: usize,
    output_bytes: usize,
    session_frames: usize,
    session_bytes: usize,
    requested_bytes: usize,
) -> bool {
    frames < MAX_PENDING_CAPTURE_FRAMES
        && output_frames < MAX_PENDING_CAPTURE_FRAMES_PER_OUTPUT
        && session_frames < MAX_PENDING_CAPTURE_FRAMES_PER_SESSION
        && total_bytes.saturating_add(requested_bytes) <= MAX_PENDING_CAPTURE_BYTES
        && output_bytes.saturating_add(requested_bytes) <= MAX_PENDING_CAPTURE_BYTES_PER_OUTPUT
        && session_bytes.saturating_add(requested_bytes) <= MAX_PENDING_CAPTURE_BYTES_PER_SESSION
}

fn estimated_shm_bytes(
    buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
) -> Option<usize> {
    smithay::wayland::shm::with_buffer_contents(buffer, |_, pool_len, _| {
        Some(retained_shm_bytes(pool_len))
    })
    .ok()
    .flatten()
}

fn retained_shm_bytes(pool_len: usize) -> usize {
    // A live wl_buffer keeps its wl_shm_pool mapping alive, including bytes outside this image.
    pool_len
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_PENDING_CAPTURE_BYTES, MAX_PENDING_CAPTURE_BYTES_PER_OUTPUT,
        MAX_PENDING_CAPTURE_BYTES_PER_SESSION, MAX_PENDING_CAPTURE_FRAMES,
        MAX_PENDING_CAPTURE_FRAMES_PER_OUTPUT, MAX_PENDING_CAPTURE_FRAMES_PER_SESSION,
        capture_admitted, capture_transform_supported, convert_rgba_row, portal_capture_executable,
        retained_shm_bytes, retire_matching, visit_mapped_rows,
    };
    use smithay::{reexports::wayland_server::protocol::wl_shm, utils::Transform};
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
    fn direct_mapped_rows_apply_orientation_format_and_destination_stride() {
        let mapped = [
            1, 2, 3, 4, 5, 6, 7, 8, // renderer row zero
            9, 10, 11, 12, 13, 14, 15, 16, // renderer row one
        ];
        let mut destination = [0x55; 24];
        visit_mapped_rows(
            &mapped,
            2,
            2,
            false,
            wl_shm::Format::Xrgb8888,
            |row, bytes| {
                destination[row * 12..row * 12 + bytes.len()].copy_from_slice(bytes);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(&destination[..8], &[11, 10, 9, 255, 15, 14, 13, 255]);
        assert_eq!(&destination[8..12], &[0x55; 4]);
        assert_eq!(&destination[12..20], &[3, 2, 1, 255, 7, 6, 5, 255]);
        assert_eq!(&destination[20..], &[0x55; 4]);

        let mut flipped = Vec::new();
        visit_mapped_rows(&mapped, 2, 2, true, wl_shm::Format::Abgr8888, |_, bytes| {
            flipped.extend_from_slice(bytes);
            Ok(())
        })
        .unwrap();
        assert_eq!(flipped, mapped);
    }

    #[test]
    fn capture_only_advertises_transforms_implemented_by_direct_copy() {
        assert!(capture_transform_supported(Transform::Normal));
        assert!(capture_transform_supported(Transform::Flipped180));
        for unsupported in [
            Transform::_90,
            Transform::_180,
            Transform::_270,
            Transform::Flipped,
            Transform::Flipped90,
            Transform::Flipped270,
        ] {
            assert!(!capture_transform_supported(unsupported));
        }
    }

    #[test]
    fn retained_buffer_accounting_charges_the_whole_shm_pool() {
        let oversized_pool = MAX_PENDING_CAPTURE_BYTES_PER_SESSION + 1;
        assert_eq!(retained_shm_bytes(oversized_pool), oversized_pool);
        assert!(!capture_admitted(0, 0, 0, 0, 0, 0, oversized_pool));
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

    #[test]
    fn pending_capture_admission_enforces_every_independent_limit() {
        assert!(capture_admitted(0, 0, 0, 0, 0, 0, 4096));
        assert!(!capture_admitted(
            MAX_PENDING_CAPTURE_FRAMES,
            0,
            0,
            0,
            0,
            0,
            1
        ));
        assert!(!capture_admitted(
            0,
            MAX_PENDING_CAPTURE_BYTES,
            0,
            0,
            0,
            0,
            1
        ));
        assert!(!capture_admitted(
            0,
            0,
            MAX_PENDING_CAPTURE_FRAMES_PER_OUTPUT,
            0,
            0,
            0,
            1
        ));
        assert!(!capture_admitted(
            0,
            0,
            0,
            0,
            MAX_PENDING_CAPTURE_FRAMES_PER_SESSION,
            0,
            1
        ));
        assert!(!capture_admitted(
            0,
            0,
            0,
            MAX_PENDING_CAPTURE_BYTES_PER_OUTPUT,
            0,
            0,
            1
        ));
        assert!(!capture_admitted(
            0,
            0,
            0,
            0,
            0,
            MAX_PENDING_CAPTURE_BYTES_PER_SESSION,
            1
        ));
        assert!(capture_admitted(
            MAX_PENDING_CAPTURE_FRAMES - 1,
            MAX_PENDING_CAPTURE_BYTES - 1,
            MAX_PENDING_CAPTURE_FRAMES_PER_OUTPUT - 1,
            MAX_PENDING_CAPTURE_BYTES_PER_OUTPUT - 1,
            MAX_PENDING_CAPTURE_FRAMES_PER_SESSION - 1,
            MAX_PENDING_CAPTURE_BYTES_PER_SESSION - 1,
            1
        ));
    }

    #[test]
    fn per_session_fairness_does_not_prevent_another_session_from_admission() {
        assert!(!capture_admitted(
            MAX_PENDING_CAPTURE_FRAMES_PER_SESSION,
            4096,
            MAX_PENDING_CAPTURE_FRAMES_PER_SESSION,
            4096,
            MAX_PENDING_CAPTURE_FRAMES_PER_SESSION,
            4096,
            1,
        ));
        assert!(capture_admitted(
            MAX_PENDING_CAPTURE_FRAMES_PER_SESSION,
            4096,
            MAX_PENDING_CAPTURE_FRAMES_PER_SESSION,
            4096,
            0,
            0,
            1,
        ));
    }

    #[test]
    fn session_output_and_abort_retirement_return_accounting_to_zero() {
        #[derive(Clone, Copy)]
        struct Entry {
            frame: u8,
            session: u8,
            output: u8,
            bytes: usize,
        }

        let mut pending = vec![
            Entry {
                frame: 1,
                session: 1,
                output: 1,
                bytes: 10,
            },
            Entry {
                frame: 2,
                session: 2,
                output: 1,
                bytes: 20,
            },
            Entry {
                frame: 3,
                session: 2,
                output: 2,
                bytes: 30,
            },
        ];
        let session = retire_matching(&mut pending, |entry| entry.session == 1);
        assert_eq!(session.iter().map(|entry| entry.bytes).sum::<usize>(), 10);
        let output = retire_matching(&mut pending, |entry| entry.output == 1);
        assert_eq!(output.iter().map(|entry| entry.bytes).sum::<usize>(), 20);
        let abort = retire_matching(&mut pending, |entry| entry.frame == 3);
        assert_eq!(abort.iter().map(|entry| entry.bytes).sum::<usize>(), 30);
        assert!(pending.is_empty());
        assert_eq!(pending.iter().map(|entry| entry.bytes).sum::<usize>(), 0);
    }
}
