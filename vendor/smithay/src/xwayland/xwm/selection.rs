use std::{
    collections::HashMap,
    fmt,
    os::fd::{BorrowedFd, OwnedFd},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use calloop::{LoopHandle, RegistrationToken};
use tracing::{debug, trace, warn};
use x11rb::{
    connection::Connection as _,
    errors::ReplyOrIdError,
    protocol::{
        xfixes::{ConnectionExt as _, SelectionEventMask},
        xproto::{
            Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux,
            EventMask, GetPropertyReply, PropMode, SELECTION_NOTIFY_EVENT, Screen,
            SelectionNotifyEvent, SelectionRequestEvent, Window as X11Window, WindowClass,
        },
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

use crate::{
    wayland::selection::SelectionTarget,
    xwayland::xwm::{Atoms, OwnedX11Window},
};

// copied from wlroots - docs say "maximum size can vary widely depending on the implementation"
// and there is no way to query the maximum size, you just get a non-descriptive `Length` error...
pub const INCR_CHUNK_SIZE: usize = 64 * 1024;
pub const MAX_OUTGOING_TRANSFERS: usize = 32;
pub const MAX_OUTGOING_BUFFER: usize = INCR_CHUNK_SIZE * 2;
pub const OUTGOING_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(5);
pub const OUTGOING_TOTAL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OutgoingTransferKey {
    pub requestor: X11Window,
    pub property: Atom,
}

#[derive(Debug)]
pub struct OutgoingAdmission(Arc<AtomicUsize>);

impl OutgoingAdmission {
    pub fn acquire(count: &Arc<AtomicUsize>) -> Option<Self> {
        count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < MAX_OUTGOING_TRANSFERS).then_some(current + 1)
            })
            .ok()?;
        Some(Self(Arc::clone(count)))
    }
}

impl Drop for OutgoingAdmission {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug)]
pub struct RequestorObservation {
    conn: Arc<RustConnection>,
    requestor: X11Window,
    observations: Arc<Mutex<HashMap<X11Window, (EventMask, usize)>>>,
    pub class: WindowClass,
}

impl RequestorObservation {
    pub fn acquire(
        conn: &Arc<RustConnection>,
        observations: &Arc<Mutex<HashMap<X11Window, (EventMask, usize)>>>,
        requestor: X11Window,
    ) -> Result<Self, ReplyOrIdError> {
        let mut observations_guard = observations.lock().unwrap();
        let attributes = conn.get_window_attributes(requestor)?.reply()?;
        if let Some((_, references)) = observations_guard.get_mut(&requestor) {
            *references += 1;
        } else {
            let original = attributes.your_event_mask;
            if !original.contains(EventMask::PROPERTY_CHANGE) {
                conn.change_window_attributes(
                    requestor,
                    &ChangeWindowAttributesAux::new()
                        .event_mask(original | EventMask::PROPERTY_CHANGE),
                )?
                .check()?;
                conn.flush()?;
            }
            observations_guard.insert(requestor, (original, 1));
        }
        drop(observations_guard);
        Ok(Self {
            conn: Arc::clone(conn),
            requestor,
            observations: Arc::clone(observations),
            class: attributes.class,
        })
    }
}

impl Drop for RequestorObservation {
    fn drop(&mut self) {
        let original = {
            let mut observations = self.observations.lock().unwrap();
            let Some((original, references)) = observations.get_mut(&self.requestor) else {
                return;
            };
            *references -= 1;
            if *references != 0 {
                return;
            }
            let original = *original;
            observations.remove(&self.requestor);
            original
        };
        if !original.contains(EventMask::PROPERTY_CHANGE)
            && let Ok(cookie) = self.conn.get_window_attributes(self.requestor)
            && let Ok(attributes) = cookie.reply()
        {
            // The registry is shared by clipboard, primary and DnD, so reaching
            // zero proves no selection transfer on this connection still owns
            // PROPERTY_CHANGE. Preserve every unrelated bit currently selected.
            let retained = EventMask::from(
                attributes.your_event_mask.bits() & !EventMask::PROPERTY_CHANGE.bits(),
            );
            let _ = self.conn.change_window_attributes(
                self.requestor,
                &ChangeWindowAttributesAux::new().event_mask(retained),
            );
            let _ = self.conn.flush();
        }
    }
}

#[derive(Debug)]
pub struct PendingTransfer {
    pub window: OwnedX11Window,
    pub fd: OwnedFd,
    pub mime_type: String,
    pub started: Instant,
}

impl PendingTransfer {
    pub fn timed_out(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.started) >= OUTGOING_INACTIVITY_TIMEOUT
    }
}

#[derive(Debug)]
pub struct XWmSelection {
    pub atom: Atom,

    pub conn: Arc<RustConnection>,
    pub atoms: Atoms,
    pub window: OwnedX11Window,
    pub owner: X11Window,
    pub mime_types: Vec<String>,
    pub timestamp: u32,

    pub pending_transfers: Arc<Mutex<HashMap<X11Window, PendingTransfer>>>,
    pub incoming: HashMap<X11Window, IncomingTransfer>,
    pub outgoing: HashMap<OutgoingTransferKey, OutgoingTransfer>,
}

pub struct IncomingTransfer {
    pub token: Option<RegistrationToken>,
    pub window: OwnedX11Window,

    pub incr: bool,
    pub source_data: Vec<u8>,
    pub incr_done: bool,
    pub started: Instant,
    pub last_progress: Instant,
    pub mime_type: String,
    pub bytes_received: usize,
}

impl fmt::Debug for IncomingTransfer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IncomingTransfer")
            .field("token", &self.token)
            .field("window", &self.window)
            .field("incr", &self.incr)
            .field("source_data", &self.source_data)
            .field("incr_done", &self.incr_done)
            .finish()
    }
}

impl IncomingTransfer {
    pub fn read_selection_prop(&mut self, reply: GetPropertyReply) {
        if !reply.value.is_empty() {
            self.last_progress = Instant::now();
        }
        self.bytes_received = self.bytes_received.saturating_add(reply.value.len());
        self.source_data.extend(&reply.value)
    }

    pub fn write_selection(&mut self, fd: BorrowedFd<'_>) -> std::io::Result<bool> {
        if self.source_data.is_empty() {
            return Ok(true);
        }

        let len = rustix::io::write(fd, &self.source_data)?;
        if len > 0 {
            self.last_progress = Instant::now();
        }
        self.source_data = self.source_data.split_off(len);

        Ok(self.source_data.is_empty())
    }

    pub fn destroy<D>(mut self, handle: &LoopHandle<'_, D>) {
        if let Some(token) = self.token.take() {
            handle.remove(token);
        }
    }

    pub fn timeout_reason(&self, now: Instant) -> Option<&'static str> {
        if now.saturating_duration_since(self.started) >= OUTGOING_TOTAL_TIMEOUT {
            Some("total-deadline")
        } else if now.saturating_duration_since(self.last_progress) >= OUTGOING_INACTIVITY_TIMEOUT {
            Some("inactivity-deadline")
        } else {
            None
        }
    }
}

impl Drop for IncomingTransfer {
    fn drop(&mut self) {
        if self.token.is_some() {
            tracing::warn!(
                ?self,
                "IncomingTransfer freed before being removed from EventLoop"
            );
        }
    }
}

pub struct OutgoingTransfer {
    pub conn: Arc<RustConnection>,
    pub token: Option<RegistrationToken>,

    pub incr: bool,
    pub source_data: Vec<u8>,
    pub request: SelectionRequestEvent,
    pub mime_type: String,
    pub _observation: RequestorObservation,
    pub _admission: OutgoingAdmission,
    pub started: Instant,
    pub last_progress: Instant,
    pub bytes_read: usize,

    pub property_set: bool,
    pub flush_property_on_delete: bool,
    /// The final 0-byte data chunk has been sent, denoting the completion of this transfer
    pub sent_finished: bool,
}

impl fmt::Debug for OutgoingTransfer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutgoingTransfer")
            .field("conn", &"...")
            .field("token", &self.token)
            .field("incr", &self.incr)
            .field("source_data", &self.source_data)
            .field("request", &self.request)
            .field("property_set", &self.property_set)
            .field("flush_property_on_delete", &self.flush_property_on_delete)
            .finish()
    }
}

impl OutgoingTransfer {
    pub fn timeout_reason(&self, now: Instant) -> Option<&'static str> {
        if now.saturating_duration_since(self.started) >= OUTGOING_TOTAL_TIMEOUT {
            Some("total-deadline")
        } else if now.saturating_duration_since(self.last_progress) >= OUTGOING_INACTIVITY_TIMEOUT {
            Some("inactivity-deadline")
        } else {
            None
        }
    }

    pub fn flush_data(&mut self) -> Result<usize, ReplyOrIdError> {
        let len = std::cmp::min(self.source_data.len(), INCR_CHUNK_SIZE);

        if len == 0 {
            // This flush will complete the transfer
            self.sent_finished = true;
        }

        let mut data = self.source_data.split_off(len);
        std::mem::swap(&mut data, &mut self.source_data);

        self.conn.change_property8(
            PropMode::REPLACE,
            self.request.requestor,
            self.request.property,
            self.request.target,
            &data,
        )?;
        self.conn.flush()?;

        let remaining = self.source_data.len();
        self.property_set = true;
        Ok(remaining)
    }

    pub fn abort(&self) {
        if self.incr {
            // ICCCM has no failure SelectionNotify after INCR acknowledgement.
            // Remove the outstanding handshake property so the requestor is not
            // left waiting for a chunk from an owner which retired the transfer.
            let _ = self
                .conn
                .delete_property(self.request.requestor, self.request.property);
            let _ = self.conn.flush();
        } else {
            let _ = send_selection_notify_resp(&self.conn, &self.request, false);
        }
    }

    pub fn destroy<D>(mut self, handle: &LoopHandle<'_, D>) {
        if let Some(token) = self.token.take() {
            handle.remove(token);
        }
    }
}

impl Drop for OutgoingTransfer {
    fn drop(&mut self) {
        if self.token.is_some() {
            tracing::warn!(
                ?self,
                "OutgoingTransfer freed before being removed from EventLoop"
            );
        }
    }
}

impl XWmSelection {
    pub fn new(
        conn: &Arc<RustConnection>,
        screen: &Screen,
        atoms: &Atoms,
        atom: Atom,
    ) -> Result<Self, ReplyOrIdError> {
        let window = conn.generate_id()?;
        conn.create_window(
            screen.root_depth,
            window,
            screen.root,
            0,
            0,
            10,
            10,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?;

        if atom == atoms.CLIPBOARD {
            conn.set_selection_owner(window, atoms.CLIPBOARD_MANAGER, x11rb::CURRENT_TIME)?;
        }
        conn.xfixes_select_selection_input(
            window,
            atom,
            SelectionEventMask::SET_SELECTION_OWNER
                | SelectionEventMask::SELECTION_WINDOW_DESTROY
                | SelectionEventMask::SELECTION_CLIENT_CLOSE,
        )?;
        conn.flush()?;

        debug!(
            selection_window = ?window,
            ?atom,
            "Selection init",
        );

        Ok(XWmSelection {
            atom,
            conn: conn.clone(),
            atoms: *atoms,
            window: OwnedX11Window::new(window, conn),
            owner: x11rb::NONE,
            mime_types: Vec::new(),
            timestamp: x11rb::CURRENT_TIME,
            pending_transfers: Arc::new(Mutex::new(HashMap::new())),
            incoming: HashMap::new(),
            outgoing: HashMap::new(),
        })
    }

    pub fn window_destroyed<D>(&mut self, window: &X11Window, loop_handle: &LoopHandle<'_, D>) -> bool {
        let mut removed = if let Some(transfer) = self.incoming.remove(window) {
            transfer.destroy(loop_handle);
            true
        } else {
            false
        };
        let outgoing = self
            .outgoing
            .keys()
            .filter(|key| key.requestor == *window)
            .copied()
            .collect::<Vec<_>>();
        for key in outgoing {
            if let Some(transfer) = self.outgoing.remove(&key) {
                transfer.destroy(loop_handle);
                removed = true;
            }
        }
        removed || self.pending_transfers.lock().unwrap().remove(window).is_some()
    }

    pub fn has_window(&self, window: &X11Window) -> bool {
        self.window == *window
            || self.incoming.contains_key(window)
            || self.outgoing.keys().any(|key| key.requestor == *window)
            || self.pending_transfers.lock().unwrap().contains_key(window)
    }

    pub fn type_(&self) -> Option<SelectionTarget> {
        match self.atom {
            x if x == self.atoms.CLIPBOARD => Some(SelectionTarget::Clipboard),
            x if x == self.atoms.PRIMARY => Some(SelectionTarget::Primary),
            _ => None,
        }
    }

    pub fn expire_outgoing<D>(
        &mut self,
        now: Instant,
        loop_handle: &LoopHandle<'_, D>,
        delete_fences: &mut HashMap<OutgoingTransferKey, usize>,
    ) -> usize {
        let expired = self
            .outgoing
            .iter()
            .filter_map(|(key, transfer)| transfer.timeout_reason(now).map(|reason| (*key, reason)))
            .collect::<Vec<_>>();
        for (key, reason) in &expired {
            let Some(transfer) = self.outgoing.remove(key) else {
                continue;
            };
            if transfer.incr && transfer.property_set {
                *delete_fences.entry(*key).or_default() += 1;
            }
            transfer.abort();
            warn!(
                direction = "wayland-to-x11",
                mime_type = transfer.mime_type,
                requestor_class = ?transfer._observation.class,
                requestor = transfer.request.requestor,
                bytes = transfer.bytes_read,
                elapsed_ms = now.saturating_duration_since(transfer.started).as_millis(),
                terminal_reason = *reason,
                "selection transfer timed out"
            );
            transfer.destroy(loop_handle);
        }
        expired.len()
    }

    pub fn expire_incoming<D>(&mut self, now: Instant, loop_handle: &LoopHandle<'_, D>) -> usize {
        let expired = self
            .incoming
            .iter()
            .filter_map(|(window, transfer)| transfer.timeout_reason(now).map(|reason| (*window, reason)))
            .collect::<Vec<_>>();
        for (window, reason) in &expired {
            let Some(transfer) = self.incoming.remove(window) else {
                continue;
            };
            if transfer.incr {
                if let Some(conn) = transfer.window.conn.upgrade() {
                    let _ = conn.delete_property(*transfer.window, self.atoms._WL_SELECTION);
                    let _ = conn.flush();
                }
            }
            warn!(
                direction = "x11-to-wayland",
                mime_type = transfer.mime_type,
                requestor = *window,
                bytes = transfer.bytes_received,
                elapsed_ms = now.saturating_duration_since(transfer.started).as_millis(),
                terminal_reason = *reason,
                "selection transfer timed out"
            );
            transfer.destroy(loop_handle);
        }
        expired.len()
    }

    pub fn expire_pending(&mut self, now: Instant) -> usize {
        let mut pending = self.pending_transfers.lock().unwrap();
        let expired = pending
            .iter()
            .filter_map(|(window, transfer)| transfer.timed_out(now).then_some(*window))
            .collect::<Vec<_>>();
        for window in &expired {
            if let Some(transfer) = pending.remove(window) {
                warn!(
                    direction = "x11-to-wayland",
                    mime_type = transfer.mime_type,
                    requestor = *window,
                    elapsed_ms = now.saturating_duration_since(transfer.started).as_millis(),
                    terminal_reason = "selection-notify-deadline",
                    "pending selection transfer timed out"
                );
            }
        }
        expired.len()
    }

    pub fn destroy_all<D>(
        &mut self,
        loop_handle: &LoopHandle<'_, D>,
        delete_fences: &mut HashMap<OutgoingTransferKey, usize>,
    ) {
        for (_, transfer) in self.incoming.drain() {
            transfer.destroy(loop_handle);
        }
        for (key, transfer) in self.outgoing.drain() {
            if transfer.incr && transfer.property_set {
                *delete_fences.entry(key).or_default() += 1;
            }
            transfer.abort();
            transfer.destroy(loop_handle);
        }
        self.pending_transfers.lock().unwrap().clear();
    }
}

pub enum OutgoingAction {
    Done,
    DoneReading,
    Backpressured,
    WaitForReadable,
    Abort,
}

pub fn read_selection_callback(
    conn: &RustConnection,
    atoms: &Atoms,
    fd: BorrowedFd<'_>,
    transfer: &mut OutgoingTransfer,
) -> Result<OutgoingAction, ReplyOrIdError> {
    if transfer.source_data.len() >= MAX_OUTGOING_BUFFER {
        return Ok(OutgoingAction::Backpressured);
    }
    let mut buf = [0; INCR_CHUNK_SIZE];
    let Ok(len) = rustix::io::read(fd, &mut buf) else {
        debug!(
            requestor = transfer.request.requestor,
            "File descriptor closed, aborting transfer."
        );
        return Ok(OutgoingAction::Abort);
    };
    trace!(
        requestor = transfer.request.requestor,
        "Transfer became readable, read {} bytes", len
    );

    transfer.source_data.extend_from_slice(&buf[..len]);
    transfer.bytes_read = transfer.bytes_read.saturating_add(len);
    if len > 0 {
        transfer.last_progress = Instant::now();
    }
    if transfer.source_data.len() >= INCR_CHUNK_SIZE {
        if !transfer.incr {
            // start incr transfer
            trace!(
                requestor = transfer.request.requestor,
                "Transfer became incremental",
            );
            conn.change_property32(
                PropMode::REPLACE,
                transfer.request.requestor,
                transfer.request.property,
                atoms.INCR,
                &[INCR_CHUNK_SIZE as u32],
            )?;
            conn.flush()?;
            transfer.incr = true;
            transfer.property_set = true;
            transfer.flush_property_on_delete = true;
            send_selection_notify_resp(conn, &transfer.request, true)?;
        } else if transfer.property_set {
            // got more bytes, waiting for property delete
            transfer.flush_property_on_delete = true;
        } else {
            // got more bytes, property deleted
            let len = transfer.flush_data()?;
            trace!(
                requestor = transfer.request.requestor,
                "Send data chunk: {} bytes", len
            );
        }
    }

    if len == 0 {
        if transfer.incr {
            debug!("Incr transfer completed");
            if !transfer.property_set {
                let len = transfer.flush_data()?;
                trace!(
                    requestor = transfer.request.requestor,
                    "Send data chunk: {} bytes", len
                );
            }
            transfer.flush_property_on_delete = true;
            Ok(OutgoingAction::DoneReading)
        } else {
            let len = transfer.flush_data()?;
            debug!("Non-Incr transfer completed with {} bytes", len);
            send_selection_notify_resp(conn, &transfer.request, true)?;
            Ok(OutgoingAction::Done)
        }
    } else if transfer.source_data.len() >= MAX_OUTGOING_BUFFER && transfer.property_set {
        Ok(OutgoingAction::Backpressured)
    } else {
        Ok(OutgoingAction::WaitForReadable)
    } // nothing to be done, buffered the bytes
}

pub enum IncomingAction {
    Done,
    WaitForProperty,
    WaitForWritable,
}

pub fn write_selection_callback(
    fd: BorrowedFd<'_>,
    conn: &RustConnection,
    atoms: &Atoms,
    transfer: &mut IncomingTransfer,
) -> Result<IncomingAction, ReplyOrIdError> {
    match transfer.write_selection(fd) {
        Ok(true) => {
            if transfer.incr {
                conn.delete_property(*transfer.window, atoms._WL_SELECTION)?;
                Ok(IncomingAction::WaitForProperty)
            } else {
                debug!(?transfer, "Non-Incr Transfer complete!");
                Ok(IncomingAction::Done)
            }
        }
        Ok(false) => Ok(IncomingAction::WaitForWritable),
        Err(err) => {
            warn!(?err, "Transfer errored");
            if transfer.incr {
                // even if it failed, we still need to drain the incr transfer
                conn.delete_property(*transfer.window, atoms._WL_SELECTION)?;
                Ok(IncomingAction::WaitForProperty)
            } else {
                Ok(IncomingAction::Done)
            }
        }
    }
}

pub fn send_selection_notify_resp(
    conn: &RustConnection,
    req: &SelectionRequestEvent,
    success: bool,
) -> Result<(), ReplyOrIdError> {
    conn.send_event(
        false,
        req.requestor,
        EventMask::NO_EVENT,
        SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: req.time,
            requestor: req.requestor,
            selection: req.selection,
            target: req.target,
            property: if success {
                req.property
            } else {
                AtomEnum::NONE.into()
            },
        },
    )?;
    conn.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outgoing_admission_is_bounded_and_released_exactly_once() {
        let count = Arc::new(AtomicUsize::new(0));
        let permits = (0..MAX_OUTGOING_TRANSFERS)
            .map(|_| OutgoingAdmission::acquire(&count).expect("within admission bound"))
            .collect::<Vec<_>>();
        assert_eq!(count.load(Ordering::Acquire), MAX_OUTGOING_TRANSFERS);
        assert!(OutgoingAdmission::acquire(&count).is_none());

        drop(permits);
        assert_eq!(count.load(Ordering::Acquire), 0);
        assert!(OutgoingAdmission::acquire(&count).is_some());
    }

    #[test]
    fn request_property_identity_does_not_alias_simultaneous_transfers() {
        let first = OutgoingTransferKey {
            requestor: 7,
            property: 11,
        };
        let second_property = OutgoingTransferKey {
            requestor: 7,
            property: 12,
        };
        let second_requestor = OutgoingTransferKey {
            requestor: 8,
            property: 11,
        };
        assert_ne!(first, second_property);
        assert_ne!(first, second_requestor);
    }
}
