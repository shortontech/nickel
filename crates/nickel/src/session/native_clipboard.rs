//! Session-owned text clipboard and bounded asynchronous transfer admission.

use super::clipboard_transfer::TransferGate;
use smithay::reexports::calloop::RegistrationToken;
use std::{sync::Arc, time::Duration};

/// One authority supplies the product limit. Keeping it optional permits a
/// policy decision without silently choosing a cap or truncating the user's text.
#[derive(Default)]
pub(crate) struct NativeClipboardState {
    pub(super) text_limit: Option<usize>,
    pub(super) reads: TransferGate,
    pub(super) writes: TransferGate,
    pub(super) pending_read: Option<(u64, RegistrationToken)>,
    next_read: u64,
    pub(super) last_failure: Option<String>,
    /// MIME types advertised by the compositor's current clipboard owner.
    /// Menu construction reads this cache instead of connecting back through
    /// the Wayland socket from the compositor event-loop thread.
    pub(super) mime_types: Vec<String>,
}

pub(super) const TRANSFER_TIMEOUT: Duration = Duration::from_secs(2);
const IMAGE_TRANSFER_LIMIT: usize = 16 * 1024 * 1024;
const IMAGE_DIMENSION_LIMIT: u32 = 8192;
const IMAGE_DECODE_LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy)]
enum NativePasteAuthority {
    OnScreenKeyboard(u64),
    Direct,
}

fn decode_clipboard_png(bytes: Vec<u8>) -> Result<(u32, u32, Vec<u8>), &'static str> {
    let cursor = std::io::Cursor::new(bytes);
    let mut reader = image::ImageReader::with_format(cursor, image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(IMAGE_DIMENSION_LIMIT);
    limits.max_image_height = Some(IMAGE_DIMENSION_LIMIT);
    limits.max_alloc = Some(IMAGE_DECODE_LIMIT);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| "clipboard PNG could not be decoded")?
        .to_rgba8();
    Ok((image.width(), image.height(), image.into_raw()))
}

impl super::state::NickelSession {
    pub(super) fn request_native_image_paste(
        &mut self,
        recipient: nickel_ui::InternalSurfaceId,
    ) -> Result<bool, &'static str> {
        use smithay::wayland::selection::data_device::{
            current_data_device_selection_userdata, request_data_device_client_selection,
        };
        if !self
            .native_clipboard
            .mime_types
            .iter()
            .any(|mime| mime.eq_ignore_ascii_case("image/png"))
        {
            return Ok(false);
        }
        if self.native_clipboard.pending_read.is_some() {
            return Err("clipboard paste is already pending");
        }
        let permit = self
            .native_clipboard
            .reads
            .acquire(1)
            .ok_or("clipboard paste is already pending")?;
        let owner = current_data_device_selection_userdata(&self.seat).map(|owner| owner.clone());
        let (reader, writer) =
            std::os::unix::net::UnixStream::pair().map_err(|_| "clipboard pipe unavailable")?;
        if let Some(super::handlers::SelectionOwner::XWayland(owner)) = owner {
            let Some((_, xwm)) = self.xwm.as_mut().filter(|(id, _)| *id == owner) else {
                return Err("XWayland selection owner is unavailable");
            };
            xwm.send_selection(
                smithay::wayland::selection::SelectionTarget::Clipboard,
                "image/png".into(),
                writer.into(),
            )
            .map_err(|_| "XWayland image selection request failed")?;
        } else if let Some(super::handlers::SelectionOwner::NativeImage(png)) = owner {
            self.send_native_image_clipboard(writer.into(), png)?;
        } else {
            request_data_device_client_selection(&self.seat, "image/png".into(), writer.into())
                .map_err(|_| "clipboard PNG is unavailable")?;
        }
        self.begin_native_image_read(reader.into(), recipient, permit)?;
        Ok(true)
    }

    pub(super) fn begin_native_image_read(
        &mut self,
        reader: std::os::fd::OwnedFd,
        recipient: nickel_ui::InternalSurfaceId,
        permit: super::clipboard_transfer::TransferPermit,
    ) -> Result<(), &'static str> {
        use smithay::reexports::calloop::channel;
        let field = self
            .native_field_lease(recipient)
            .ok_or("native image paste field is unavailable")?;
        let (sender, receiver) = channel::channel::<Result<(u32, u32, Vec<u8>), &'static str>>();
        self.native_clipboard.next_read = self.native_clipboard.next_read.wrapping_add(1);
        let transaction = self.native_clipboard.next_read;
        let token = self
            .event_loop_handle
            .insert_source(receiver, move |message, _, session| {
                if session
                    .native_clipboard
                    .pending_read
                    .as_ref()
                    .map(|(id, _)| *id)
                    != Some(transaction)
                {
                    return;
                }
                if matches!(message, channel::Event::Closed) {
                    if let Some((_, token)) = session.native_clipboard.pending_read.take() {
                        session.event_loop_handle.remove(token);
                    }
                    session.native_clipboard.last_failure =
                        Some("clipboard image worker closed without a result".into());
                    return;
                }
                if let channel::Event::Msg(result) = message {
                    if let Some((_, token)) = session.native_clipboard.pending_read.take() {
                        session.event_loop_handle.remove(token);
                    }
                    let result = result.and_then(|(width, height, rgba)| {
                        if session.internal_ui.focused() != Some(recipient) {
                            return Err("clipboard paste recipient changed");
                        }
                        if session.native_field_lease(recipient).as_ref() != Some(&field) {
                            return Err("clipboard paste field changed");
                        }
                        if !session
                            .internal_ui
                            .paste_clipboard_image(recipient, width, height, &rgba)
                        {
                            return Err("clipboard image recipient rejected the image");
                        }
                        session.schedule_internal_ui_frame();
                        Ok(())
                    });
                    if let Err(error) = result {
                        tracing::warn!(error, "native clipboard image paste rejected");
                        session.native_clipboard.last_failure = Some(error.into());
                    }
                }
            })
            .map_err(|_| "clipboard image completion source unavailable")?;
        self.native_clipboard.pending_read = Some((transaction, token));
        if std::thread::Builder::new()
            .name("nickel-clipboard-image-read".into())
            .spawn(move || {
                let result = super::clipboard_transfer::read_bytes(
                    reader,
                    IMAGE_TRANSFER_LIMIT,
                    TRANSFER_TIMEOUT,
                )
                .and_then(decode_clipboard_png);
                // Completion permits another read immediately.
                drop(permit);
                let _ = sender.send(result);
            })
            .is_err()
        {
            self.native_clipboard.pending_read = None;
            self.event_loop_handle.remove(token);
            return Err("clipboard image transfer worker unavailable");
        }
        let _ = self.display_handle.flush_clients();
        Ok(())
    }

    pub(super) fn native_file_clipboard_available(&self) -> bool {
        self.native_clipboard.mime_types.iter().any(|mime| {
            matches!(
                mime.as_str(),
                "x-special/gnome-copied-files" | "text/uri-list"
            )
        })
    }

    fn native_field_lease(
        &self,
        recipient: nickel_ui::InternalSurfaceId,
    ) -> Option<(nickel_ui::UiId, u64)> {
        if let Some((shell_id, _)) = self
            .internal_shell_surfaces
            .iter()
            .find(|(_, runtime)| **runtime == recipient)
        {
            return self.internal_shell.as_ref()?.focused_field_lease(*shell_id);
        }
        self.internal_ui.focused_field_lease(recipient)
    }

    pub(super) fn dispatch_native_key(
        &mut self,
        epoch: u64,
        event: nickel_input::KeyEvent,
        clipboard: Option<String>,
    ) -> Result<(), &'static str> {
        self.reconcile_keyboard_internal_recipient();
        let snapshot = self.on_screen_keyboard_snapshot();
        if snapshot.epoch != epoch
            || !snapshot.enabled
            || !snapshot.visible
            || snapshot.internal_recipient.is_none()
        {
            return Err("on-screen keyboard recipient is no longer available");
        }
        let id = self
            .internal_ui
            .focused()
            .ok_or("native keyboard recipient is unavailable")?;
        self.dispatch_native_key_to(id, event, clipboard)
    }

    fn dispatch_native_key_to(
        &mut self,
        id: nickel_ui::InternalSurfaceId,
        event: nickel_input::KeyEvent,
        clipboard: Option<String>,
    ) -> Result<(), &'static str> {
        if self.internal_ui.focused() != Some(id) {
            return Err("native keyboard recipient is unavailable");
        }
        self.native_clipboard.last_failure = None;
        self.internal_ui
            .set_clipboard_limit(self.native_clipboard.text_limit.unwrap_or(0));
        let mut released = event.clone();
        released.edge = nickel_input::KeyEdge::Released;
        self.internal_ui.step(
            id,
            nickel_ui::HostBatch {
                events: vec![
                    nickel_ui::HostEvent::Normalized {
                        input: nickel_input::InputEvent::Key(event),
                        clipboard_text: clipboard,
                    },
                    nickel_ui::HostEvent::Normalized {
                        input: nickel_input::InputEvent::Key(released),
                        clipboard_text: None,
                    },
                ],
                ..Default::default()
            },
        );
        self.flush_internal_shell_input();
        self.reconcile_internal_application_focus();
        self.note_input_activity();
        self.wake_internal_shell();
        self.schedule_internal_ui_frame();
        if self.native_clipboard.last_failure.is_some() {
            Err("native clipboard operation rejected")
        } else {
            Ok(())
        }
    }

    pub(super) fn request_native_paste(
        &mut self,
        epoch: u64,
        event: nickel_input::KeyEvent,
    ) -> Result<(), &'static str> {
        let recipient = self
            .internal_ui
            .focused()
            .ok_or("native keyboard recipient is unavailable")?;
        self.request_native_text_paste(
            NativePasteAuthority::OnScreenKeyboard(epoch),
            event,
            recipient,
        )
    }

    pub(super) fn request_native_direct_text_paste(
        &mut self,
        recipient: nickel_ui::InternalSurfaceId,
        event: nickel_input::KeyEvent,
    ) -> Result<(), &'static str> {
        self.request_native_text_paste(NativePasteAuthority::Direct, event, recipient)
    }

    fn request_native_text_paste(
        &mut self,
        authority: NativePasteAuthority,
        event: nickel_input::KeyEvent,
        recipient: nickel_ui::InternalSurfaceId,
    ) -> Result<(), &'static str> {
        use smithay::wayland::selection::data_device::{
            current_data_device_selection_userdata, request_data_device_client_selection,
        };
        let maximum = self
            .native_clipboard
            .text_limit
            .ok_or("native clipboard text limit is not configured")?;
        if self.native_clipboard.pending_read.is_some() {
            return Err("clipboard paste is already pending");
        }
        let owner = current_data_device_selection_userdata(&self.seat).map(|owner| owner.clone());
        if let Some(super::handlers::SelectionOwner::NativeText(text)) = &owner {
            if text.len() > maximum {
                return Err("clipboard text exceeds transfer limit");
            }
            return match authority {
                NativePasteAuthority::OnScreenKeyboard(epoch) => {
                    self.dispatch_native_key(epoch, event, Some(text.to_string()))
                }
                NativePasteAuthority::Direct => {
                    self.dispatch_native_key_to(recipient, event, Some(text.to_string()))
                }
            };
        }
        let permit = self
            .native_clipboard
            .reads
            .acquire(1)
            .ok_or("clipboard paste is already pending")?;
        let (reader, writer) =
            std::os::unix::net::UnixStream::pair().map_err(|_| "clipboard pipe unavailable")?;
        if let Some(super::handlers::SelectionOwner::XWayland(owner)) = owner {
            let Some((_, xwm)) = self.xwm.as_mut().filter(|(id, _)| *id == owner) else {
                return Err("XWayland selection owner is unavailable");
            };
            xwm.send_selection(
                smithay::wayland::selection::SelectionTarget::Clipboard,
                "text/plain;charset=utf-8".into(),
                writer.into(),
            )
            .map_err(|_| "XWayland selection request failed")?;
        } else {
            let mut requested = false;
            for mime in ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"] {
                let fd = writer
                    .try_clone()
                    .map_err(|_| "clipboard descriptor duplication failed")?;
                if request_data_device_client_selection(&self.seat, mime.into(), fd.into()).is_ok()
                {
                    requested = true;
                    break;
                }
            }
            if !requested {
                return Err("clipboard UTF-8 text is unavailable");
            }
        }
        self.begin_native_text_read(reader.into(), authority, event, recipient, maximum, permit)
    }

    #[cfg(test)]
    pub(super) fn begin_native_paste_read(
        &mut self,
        reader: std::os::fd::OwnedFd,
        epoch: u64,
        event: nickel_input::KeyEvent,
        recipient: nickel_ui::InternalSurfaceId,
        maximum: usize,
        permit: super::clipboard_transfer::TransferPermit,
    ) -> Result<(), &'static str> {
        self.begin_native_text_read(
            reader,
            NativePasteAuthority::OnScreenKeyboard(epoch),
            event,
            recipient,
            maximum,
            permit,
        )
    }

    #[cfg(test)]
    pub(super) fn begin_native_direct_paste_read(
        &mut self,
        reader: std::os::fd::OwnedFd,
        event: nickel_input::KeyEvent,
        recipient: nickel_ui::InternalSurfaceId,
        maximum: usize,
        permit: super::clipboard_transfer::TransferPermit,
    ) -> Result<(), &'static str> {
        self.begin_native_text_read(
            reader,
            NativePasteAuthority::Direct,
            event,
            recipient,
            maximum,
            permit,
        )
    }

    fn begin_native_text_read(
        &mut self,
        reader: std::os::fd::OwnedFd,
        authority: NativePasteAuthority,
        event: nickel_input::KeyEvent,
        recipient: nickel_ui::InternalSurfaceId,
        maximum: usize,
        permit: super::clipboard_transfer::TransferPermit,
    ) -> Result<(), &'static str> {
        use smithay::reexports::calloop::channel;
        let field = self
            .native_field_lease(recipient)
            .ok_or("native paste field is unavailable")?;
        let (sender, receiver) = channel::channel::<Result<String, &'static str>>();
        self.native_clipboard.next_read = self.native_clipboard.next_read.wrapping_add(1);
        let transaction = self.native_clipboard.next_read;
        let token = self
            .event_loop_handle
            .insert_source(receiver, move |message, _, session| {
                // calloop can report Closed after the message in the same turn.
                // Only this transaction may retire its registration or report failure.
                if session
                    .native_clipboard
                    .pending_read
                    .as_ref()
                    .map(|(id, _)| *id)
                    != Some(transaction)
                {
                    return;
                }
                if matches!(message, channel::Event::Closed) {
                    if let Some((_, token)) = session.native_clipboard.pending_read.take() {
                        session.event_loop_handle.remove(token);
                    }
                    session.native_clipboard.last_failure =
                        Some("clipboard worker closed without a result".into());
                    return;
                }
                if let channel::Event::Msg(result) = message {
                    if let Some((_, token)) = session.native_clipboard.pending_read.take() {
                        session.event_loop_handle.remove(token);
                    }
                    // A delayed paste belongs to the original field and focus
                    // generation, not whichever editor receives focus later.
                    let result = result.and_then(|text| {
                        if session.internal_ui.focused() != Some(recipient) {
                            return Err("clipboard paste recipient changed");
                        }
                        if session.native_field_lease(recipient).as_ref() != Some(&field) {
                            return Err("clipboard paste field changed");
                        }
                        match authority {
                            NativePasteAuthority::OnScreenKeyboard(epoch) => {
                                session.dispatch_native_key(epoch, event.clone(), Some(text))
                            }
                            NativePasteAuthority::Direct => {
                                session.dispatch_native_key_to(recipient, event.clone(), Some(text))
                            }
                        }
                    });
                    if let Err(error) = result {
                        tracing::warn!(error, "native clipboard paste rejected");
                        session.native_clipboard.last_failure = Some(error.into());
                    }
                }
            })
            .map_err(|_| "clipboard completion source unavailable")?;
        self.native_clipboard.pending_read = Some((transaction, token));
        if std::thread::Builder::new()
            .name("nickel-clipboard-read".into())
            .spawn(move || {
                let result =
                    super::clipboard_transfer::read_text(reader, maximum, TRANSFER_TIMEOUT);
                // Release admission before the event loop observes completion.
                drop(permit);
                let _ = sender.send(result);
            })
            .is_err()
        {
            self.native_clipboard.pending_read = None;
            self.event_loop_handle.remove(token);
            return Err("clipboard transfer worker unavailable");
        }
        let _ = self.display_handle.flush_clients();
        Ok(())
    }
    pub(super) fn flush_native_clipboard_results(&mut self) {
        let results = [
            self.internal_ui.take_clipboard_result(),
            self.internal_shell
                .as_mut()
                .and_then(|shell| shell.take_clipboard_result()),
        ];
        for result in results.into_iter().flatten() {
            let result =
                result.and_then(|text| self.publish_native_clipboard(text).map_err(str::to_owned));
            if let Err(error) = result {
                tracing::warn!(error, "native clipboard operation rejected");
                self.native_clipboard.last_failure = Some(error);
            }
        }
    }
    pub(super) fn publish_native_image_clipboard(
        &mut self,
        png: Arc<Vec<u8>>,
    ) -> Result<(), &'static str> {
        if self.locked {
            return Err("clipboard is unavailable while locked");
        }
        let mime_types = vec!["image/png".into()];
        smithay::wayland::selection::data_device::set_data_device_selection(
            &self.display_handle,
            &self.seat,
            mime_types.clone(),
            super::handlers::SelectionOwner::NativeImage(png),
        );
        self.native_clipboard.mime_types = mime_types.clone();
        if let Some((_, xwm)) = self.xwm.as_mut() {
            xwm.new_selection(
                smithay::wayland::selection::SelectionTarget::Clipboard,
                Some(mime_types),
            )
            .map_err(|_| "native image clipboard could not be mirrored to XWayland")?;
        }
        Ok(())
    }

    pub(super) fn send_native_image_clipboard(
        &self,
        fd: std::os::fd::OwnedFd,
        png: Arc<Vec<u8>>,
    ) -> Result<(), &'static str> {
        let permit = self
            .native_clipboard
            .writes
            .acquire(4)
            .ok_or("clipboard transfers are busy")?;
        std::thread::Builder::new()
            .name("nickel-image-clipboard-write".into())
            .spawn(move || {
                let _permit = permit;
                if let Err(error) =
                    super::clipboard_transfer::write_bytes(fd, &png, TRANSFER_TIMEOUT)
                {
                    tracing::debug!(error, "image clipboard transfer failed");
                }
            })
            .map_err(|_| "clipboard transfer worker unavailable")?;
        Ok(())
    }

    pub(super) fn publish_native_clipboard(&mut self, text: String) -> Result<(), &'static str> {
        if self.locked {
            return Err("clipboard is unavailable while locked");
        }
        let maximum = self
            .native_clipboard
            .text_limit
            .ok_or("native clipboard text limit is not configured")?;
        if text.len() > maximum {
            return Err("clipboard text exceeds transfer limit");
        }
        self.publish_native_text_selection(text)
    }

    pub(super) fn publish_native_text_selection(
        &mut self,
        text: String,
    ) -> Result<(), &'static str> {
        if self.locked {
            return Err("clipboard is unavailable while locked");
        }
        smithay::wayland::selection::data_device::set_data_device_selection(
            &self.display_handle,
            &self.seat,
            vec!["text/plain;charset=utf-8".into(), "text/plain".into()],
            // Move the existing text allocation; Arc<str> would copy its bytes.
            super::handlers::SelectionOwner::NativeText(Arc::new(text)),
        );
        self.native_clipboard.mime_types =
            vec!["text/plain;charset=utf-8".into(), "text/plain".into()];
        if let Some((_, xwm)) = self.xwm.as_mut() {
            xwm.new_selection(
                smithay::wayland::selection::SelectionTarget::Clipboard,
                Some(vec!["text/plain;charset=utf-8".into(), "text/plain".into()]),
            )
            .map_err(|_| "native clipboard could not be mirrored to XWayland")?;
        }
        Ok(())
    }

    pub(super) fn send_native_clipboard(
        &self,
        fd: std::os::fd::OwnedFd,
        text: Arc<String>,
    ) -> Result<(), &'static str> {
        let permit = self
            .native_clipboard
            .writes
            .acquire(4)
            .ok_or("clipboard transfers are busy")?;
        std::thread::Builder::new()
            .name("nickel-clipboard-write".into())
            .spawn(move || {
                let _permit = permit;
                if let Err(error) =
                    super::clipboard_transfer::write_text(fd, text, TRANSFER_TIMEOUT)
                {
                    tracing::debug!(error, "clipboard transfer failed");
                }
            })
            .map_err(|_| "clipboard transfer worker unavailable")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    #[test]
    fn png_decode_is_rgba_and_dimension_bounded() {
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[1, 2, 3, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert_eq!(
            decode_clipboard_png(png).unwrap(),
            (1, 1, vec![1, 2, 3, 255])
        );

        let oversized_header = {
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(
                    &vec![0; (IMAGE_DIMENSION_LIMIT as usize + 1) * 4],
                    IMAGE_DIMENSION_LIMIT + 1,
                    1,
                    image::ExtendedColorType::Rgba8,
                )
                .unwrap();
            png
        };
        assert_eq!(
            decode_clipboard_png(oversized_header),
            Err("clipboard PNG could not be decoded")
        );
    }
}
