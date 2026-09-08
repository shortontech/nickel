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
}

pub(super) const TRANSFER_TIMEOUT: Duration = Duration::from_secs(2);

impl super::state::NickelSession {
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
            return self.dispatch_native_key(epoch, event, Some(text.to_string()));
        }
        let permit = self
            .native_clipboard
            .reads
            .acquire(1)
            .ok_or("clipboard paste is already pending")?;
        let recipient = self
            .internal_ui
            .focused()
            .ok_or("native keyboard recipient is unavailable")?;
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
        self.begin_native_paste_read(reader.into(), epoch, event, recipient, maximum, permit)
    }

    pub(super) fn begin_native_paste_read(
        &mut self,
        reader: std::os::fd::OwnedFd,
        epoch: u64,
        event: nickel_input::KeyEvent,
        recipient: nickel_ui::InternalSurfaceId,
        maximum: usize,
        permit: super::clipboard_transfer::TransferPermit,
    ) -> Result<(), &'static str> {
        use smithay::reexports::calloop::channel;
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
                    // A delayed source cannot target whichever field happens to have
                    // focus later. Both the native identity and authority epoch must match.
                    let result = result.and_then(|text| {
                        if session.internal_ui.focused() != Some(recipient) {
                            return Err("clipboard paste recipient changed");
                        }
                        session.dispatch_native_key(epoch, event.clone(), Some(text))
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
                let _permit = permit;
                let _ = sender.send(super::clipboard_transfer::read_text(
                    reader,
                    maximum,
                    TRANSFER_TIMEOUT,
                ));
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
        smithay::wayland::selection::data_device::set_data_device_selection(
            &self.display_handle,
            &self.seat,
            vec!["text/plain;charset=utf-8".into(), "text/plain".into()],
            // Move the existing text allocation; Arc<str> would copy its bytes.
            super::handlers::SelectionOwner::NativeText(Arc::new(text)),
        );
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
