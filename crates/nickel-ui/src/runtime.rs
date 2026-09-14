use std::{
    any::Any,
    error::Error,
    num::NonZeroU32,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

static NEXT_LOCAL_CONTROLLER_LEASE: AtomicU64 = AtomicU64::new(1);
static NEXT_NATIVE_HOST_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ControllerDiscoveryMode {
    Standalone,
    Session,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ControllerRole {
    Local,
    Session,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ControllerRoleLease {
    role: ControllerRole,
    owner_generation: u64,
    generation: u64,
}

impl ControllerRoleLease {
    fn local() -> Self {
        Self {
            role: ControllerRole::Local,
            owner_generation: 0,
            generation: NEXT_LOCAL_CONTROLLER_LEASE.fetch_add(1, Ordering::Relaxed),
        }
    }

    const fn session(owner_generation: u64, generation: u64) -> Self {
        Self {
            role: ControllerRole::Session,
            owner_generation,
            generation,
        }
    }
}

#[cfg(any(unix, windows))]
fn adopt_pending_controller_lease(
    connection: nickel_session_protocol::controller_broker::ConnectionGeneration,
    lease_epoch: Option<nickel_session_protocol::controller_broker::LeaseEpoch>,
    oracle: Option<nickel_session_protocol::ControllerExecutionOracle>,
) -> Option<(
    nickel_session_protocol::controller_broker::LeaseEpoch,
    ControllerRoleLease,
)> {
    let (lease, oracle) = lease_epoch.zip(oracle)?;
    (oracle.lease_epoch == lease && oracle.connection_generation == connection)
        .then_some((lease, ControllerRoleLease::session(connection.0, lease.0)))
}

fn local_controller_poll_lease(
    mode: ControllerDiscoveryMode,
    lease: Option<ControllerRoleLease>,
) -> Option<ControllerRoleLease> {
    lease.filter(|lease| {
        mode == ControllerDiscoveryMode::Standalone && lease.role == ControllerRole::Local
    })
}

use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle},
    window::{CursorIcon, Window, WindowAttributes, WindowId},
};

use crate::{
    AccessibilityNode, ActionKind, Color, ControllerAction, ControllerExecutionAuthority,
    ControllerExecutionBinding, ControllerFamily, ControllerFence, ControllerInput, DamageRegion,
    EffectiveHitRoute, FocusedInputDispatcher, FrameRequest, FrameResourceDiagnostics,
    InputCommand, InputContext, InputModality, InputSource, InteractionIntent, Invalidation,
    LayoutDiagnostic, OverlayId, OverlayMenu, PointerIcon, Rect, SemanticAction,
    SemanticActionError, SemanticNodeSnapshot, SemanticQueryError, SemanticSelector,
    SoftwareRenderer, UiEvent, UiFrame, UiId, UiStateStore, View,
};

#[cfg(any(unix, windows))]
enum SessionControllerSource {
    Absent,
    Connecting {
        connection: nickel_session_protocol::client::AsyncControllerConnection,
    },
    Attached {
        connection: nickel_session_protocol::client::AsyncControllerConnection,
        connection_generation: nickel_session_protocol::controller_broker::ConnectionGeneration,
        lease: Option<nickel_session_protocol::controller_broker::LeaseEpoch>,
        role_lease: Option<ControllerRoleLease>,
        last_event: nickel_session_protocol::controller_broker::EventId,
        pending_overflow: Option<ControllerOverflowReport>,
        phase: SessionControllerPhase,
    },
    Retrying {
        next_attempt: Instant,
    },
}

#[cfg(any(unix, windows))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionControllerPhase {
    Lease,
    PendingLease,
    Poll,
    Acknowledge,
    Reset,
}

#[cfg(any(unix, windows))]
#[derive(Clone, Copy)]
struct ControllerOverflowReport {
    lease_epoch: nickel_session_protocol::controller_broker::LeaseEpoch,
    stream_generation: nickel_session_protocol::controller_broker::StreamGeneration,
    through: nickel_session_protocol::controller_broker::EventId,
}

#[cfg(any(unix, windows))]
impl SessionControllerSource {
    const RETRY_INTERVAL: Duration = Duration::from_millis(250);

    fn discover() -> (
        ControllerDiscoveryMode,
        Option<ControllerInput>,
        Option<ControllerRoleLease>,
        Self,
    ) {
        use nickel_session_protocol::client::AsyncControllerConnection;

        match AsyncControllerConnection::begin_from_environment(Duration::from_millis(100)) {
            Ok(None) => (
                ControllerDiscoveryMode::Standalone,
                Some(ControllerInput::new()),
                Some(ControllerRoleLease::local()),
                Self::Absent,
            ),
            Ok(Some(connection)) => (
                ControllerDiscoveryMode::Session,
                None,
                None,
                Self::Connecting { connection },
            ),
            Err(error) => {
                tracing::warn!(%error, "advertised session controller connection failed closed");
                (
                    ControllerDiscoveryMode::Session,
                    None,
                    None,
                    Self::Retrying {
                        next_attempt: Instant::now() + Self::RETRY_INTERVAL,
                    },
                )
            }
        }
    }

    fn retrying() -> Self {
        Self::Retrying {
            next_attempt: Instant::now() + Self::RETRY_INTERVAL,
        }
    }

    fn relinquish(&mut self) {
        let state = std::mem::replace(self, Self::retrying());
        if let Self::Attached {
            mut connection,
            connection_generation,
            ..
        } = state
        {
            let _ = connection.relinquish(connection_generation);
        }
    }

    fn report_execution_overflow(&mut self, binding: ControllerExecutionBinding) {
        if let Self::Attached {
            connection_generation,
            lease,
            pending_overflow,
            ..
        } = self
            && connection_generation.0 == binding.connection_generation
            && lease.is_some_and(|lease| lease.0 == binding.lease_epoch)
        {
            *pending_overflow = Some(ControllerOverflowReport {
                lease_epoch: nickel_session_protocol::controller_broker::LeaseEpoch(
                    binding.lease_epoch,
                ),
                stream_generation: nickel_session_protocol::controller_broker::StreamGeneration(
                    binding.stream_generation,
                ),
                through: nickel_session_protocol::controller_broker::EventId(binding.event_id),
            });
        }
    }

    fn poll_actions(
        &mut self,
    ) -> Vec<(
        Option<ControllerAction>,
        ControllerFamily,
        ControllerExecutionBinding,
        ControllerExecutionAuthority,
    )> {
        use nickel_session_protocol::{
            ControllerHostRequest, ControllerHostResponse, InputState,
            controller_broker::BrokerMessage,
        };
        let state = std::mem::replace(self, Self::retrying());
        let (next, actions) = match state {
            Self::Retrying { next_attempt } if Instant::now() < next_attempt => {
                (Self::Retrying { next_attempt }, Vec::new())
            }
            Self::Retrying { .. } => {
                match nickel_session_protocol::client::AsyncControllerConnection::begin_from_environment(
                    Duration::from_millis(100),
                ) {
                    Ok(Some(connection)) => (Self::Connecting { connection }, Vec::new()),
                    Ok(None) => (Self::retrying(), Vec::new()),
                    Err(error) => {
                        tracing::warn!(%error, "session controller rediscovery failed closed");
                        (Self::retrying(), Vec::new())
                    }
                }
            }
            Self::Connecting { mut connection } => match connection.receive() {
                Ok(None) => (Self::Connecting { connection }, Vec::new()),
                Ok(Some(ControllerHostResponse::Attached {
                    connection_generation,
                    ..
                })) => {
                    if let Err(error) = connection.send(ControllerHostRequest::RequestLease {
                        connection_generation,
                    }) {
                        tracing::warn!(%error, "session controller lease request failed closed");
                        (Self::retrying(), Vec::new())
                    } else {
                        (
                            Self::Attached {
                                connection,
                                connection_generation,
                                lease: None,
                                role_lease: None,
                                last_event: nickel_session_protocol::controller_broker::EventId(0),
                                pending_overflow: None,
                                phase: SessionControllerPhase::Lease,
                            },
                            Vec::new(),
                        )
                    }
                }
                Ok(Some(_)) => (Self::retrying(), Vec::new()),
                Err(error) => {
                    tracing::warn!(%error, "session controller attachment failed closed");
                    (Self::retrying(), Vec::new())
                }
            },
            Self::Attached {
                mut connection,
                connection_generation,
                mut lease,
                mut role_lease,
                mut last_event,
                mut pending_overflow,
                phase,
            } => match connection.receive() {
                Ok(None) => (
                    Self::Attached {
                        connection,
                        connection_generation,
                        lease,
                        role_lease,
                        last_event,
                        pending_overflow,
                        phase,
                    },
                    Vec::new(),
                ),
                Err(error) => {
                    tracing::warn!(%error, "session controller request failed closed");
                    (Self::retrying(), Vec::new())
                }
                Ok(Some(response)) => {
                    let mut actions = Vec::new();
                    let mut pending_lease_poll = false;
                    let request = match (phase, response) {
                        (
                            SessionControllerPhase::Lease,
                            ControllerHostResponse::LeaseGranted { lease_epoch },
                        ) => {
                            lease = Some(lease_epoch);
                            role_lease = Some(ControllerRoleLease::session(
                                connection_generation.0,
                                lease_epoch.0,
                            ));
                            Some(ControllerHostRequest::Poll {
                                connection_generation,
                            })
                        }
                        (
                            SessionControllerPhase::Lease,
                            ControllerHostResponse::LeasePending { .. },
                        ) => {
                            pending_lease_poll = true;
                            Some(ControllerHostRequest::Poll {
                                connection_generation,
                            })
                        }
                        (
                            SessionControllerPhase::Lease,
                            ControllerHostResponse::LeaseFailed,
                        ) => {
                            *self = Self::retrying();
                            return Vec::new();
                        }
                        (
                            SessionControllerPhase::Poll | SessionControllerPhase::PendingLease,
                            ControllerHostResponse::Messages {
                                lease_epoch,
                                messages,
                                execution_oracle,
                            },
                        ) => {
                            if phase == SessionControllerPhase::PendingLease
                                && let Some((granted, granted_role)) =
                                    adopt_pending_controller_lease(
                                        connection_generation,
                                        lease_epoch,
                                        execution_oracle,
                                    )
                            {
                                lease = Some(granted);
                                role_lease = Some(granted_role);
                            }
                            if let Some(report) = pending_overflow.take() {
                                lease = None;
                                role_lease = None;
                                actions.clear();
                                Some(ControllerHostRequest::ReportExecutionOverflow {
                                    connection_generation,
                                    lease_epoch: report.lease_epoch,
                                    stream_generation: report.stream_generation,
                                    through: report.through,
                                })
                            } else {
                            let revocation = messages.iter().find_map(|message| match message {
                                BrokerMessage::Revoke {
                                    connection_generation: bound,
                                    lease_epoch,
                                    cutoff,
                                } if *bound == connection_generation
                                    && Some(*lease_epoch) == lease =>
                                {
                                    Some((*lease_epoch, *cutoff))
                                }
                                _ => None,
                            });
                            let reset = messages.iter().any(|message| {
                                matches!(message, BrokerMessage::StreamReset { .. })
                            });
                            if reset || revocation.is_some() {
                                lease = None;
                                role_lease = None;
                                if reset {
                                    *self = Self::retrying();
                                    return Vec::new();
                                }
                                revocation.map(|(lease_epoch, cutoff)| {
                                    ControllerHostRequest::AcknowledgeQuiescence {
                                        connection_generation,
                                        lease_epoch,
                                        cutoff,
                                    }
                                })
                            } else {
                                lease = lease_epoch;
                                if let Some((current_lease, oracle)) = lease
                                    .filter(|current| {
                                    role_lease
                                        == Some(ControllerRoleLease::session(
                                            connection_generation.0,
                                            current.0,
                                        ))
                                    })
                                    .zip(execution_oracle)
                                    .filter(|(current, oracle)| {
                                        oracle.lease_epoch == *current
                                            && oracle.connection_generation
                                                == connection_generation
                                    })
                                {
                                    for message in messages {
                                        let BrokerMessage::Deliver(delivery) = message else {
                                            continue;
                                        };
                                        if delivery.connection_generation != connection_generation
                                            || delivery.lease_epoch != current_lease
                                            || delivery.stream_generation
                                                != oracle.stream_generation
                                            || delivery.payload.routing_epoch
                                                != oracle.routing_epoch
                                            || delivery.payload.surface_generation
                                                != oracle.surface_generation
                                            || delivery.event_id.0 <= last_event.0
                                        {
                                            continue;
                                        }
                                        last_event = delivery.event_id;
                                        let binding = ControllerExecutionBinding {
                                            device_generation: delivery.payload.device_generation,
                                            edge: match delivery.payload.edge {
                                                InputState::Pressed => {
                                                    nickel_input::KeyEdge::Pressed
                                                }
                                                InputState::Released => {
                                                    nickel_input::KeyEdge::Released
                                                }
                                            },
                                            routing_epoch: delivery.payload.routing_epoch,
                                            event_id: delivery.event_id.0,
                                            lease_epoch: delivery.lease_epoch.0,
                                            connection_generation: delivery.connection_generation.0,
                                            stream_generation: delivery.stream_generation.0,
                                            cutoff: None,
                                            repeat: delivery.payload.repeat,
                                            surface_generation: delivery
                                                .payload
                                                .surface_generation,
                                        };
                                        actions.push((
                                            delivery
                                                .payload
                                                .action
                                                .map(controller_action_from_message),
                                            controller_family_from_message(delivery.payload.family),
                                            binding,
                                            ControllerExecutionAuthority {
                                                routing_epoch: oracle.routing_epoch,
                                                lease_epoch: oracle.lease_epoch.0,
                                                connection_generation: oracle
                                                    .connection_generation
                                                    .0,
                                                stream_generation: oracle.stream_generation.0,
                                                cutoff: None,
                                                surface_generation: oracle.surface_generation,
                                            },
                                        ));
                                    }
                                }
                                Some(ControllerHostRequest::Poll {
                                    connection_generation,
                                })
                            }
                            }
                        }
                        (
                            SessionControllerPhase::Reset,
                            ControllerHostResponse::ResetAcknowledged { .. },
                        ) => {
                            *self = Self::retrying();
                            return Vec::new();
                        }
                        (SessionControllerPhase::Acknowledge, _) => {
                            *self = Self::retrying();
                            return Vec::new();
                        }
                        _ => None,
                    };
                    if let Some(request) = request {
                        let next_phase = if matches!(
                            request,
                            ControllerHostRequest::AcknowledgeQuiescence { .. }
                        ) {
                            SessionControllerPhase::Acknowledge
                        } else if matches!(
                            request,
                            ControllerHostRequest::ReportExecutionOverflow { .. }
                        ) {
                            SessionControllerPhase::Reset
                        } else if pending_lease_poll
                            || (phase == SessionControllerPhase::PendingLease && lease.is_none())
                        {
                            SessionControllerPhase::PendingLease
                        } else {
                            SessionControllerPhase::Poll
                        };
                        if let Err(error) = connection.send(request) {
                            tracing::warn!(%error, "session controller request failed closed");
                            (Self::retrying(), Vec::new())
                        } else {
                            (
                                Self::Attached {
                                    connection,
                                    connection_generation,
                                    lease,
                                    role_lease,
                                    last_event,
                                    pending_overflow,
                                    phase: next_phase,
                                },
                                actions,
                            )
                        }
                    } else {
                        (Self::retrying(), Vec::new())
                    }
                }
            },
            other => (other, Vec::new()),
        };
        *self = next;
        actions
    }
}

#[cfg(any(unix, windows))]
fn controller_action_from_message(
    action: nickel_session_protocol::ControllerActionMessage,
) -> ControllerAction {
    use nickel_session_protocol::ControllerActionMessage::*;
    match action {
        Launcher => ControllerAction::Launcher,
        Up => ControllerAction::Up,
        Down => ControllerAction::Down,
        Left => ControllerAction::Left,
        Right => ControllerAction::Right,
        Confirm => ControllerAction::Confirm,
        Cancel => ControllerAction::Cancel,
        ContextMenu => ControllerAction::ContextMenu,
        PreviousPane => ControllerAction::PreviousPane,
        NextPane => ControllerAction::NextPane,
    }
}

#[cfg(any(unix, windows))]
fn controller_family_from_message(
    family: nickel_session_protocol::ControllerFamilyMessage,
) -> ControllerFamily {
    use nickel_session_protocol::ControllerFamilyMessage::*;
    match family {
        PlayStation => ControllerFamily::PlayStation,
        Xbox => ControllerFamily::Xbox,
        Switch => ControllerFamily::Switch,
        Generic => ControllerFamily::Generic,
    }
}

#[derive(Debug, Default)]
struct PresentScheduler {
    dirty: bool,
    pending: bool,
}

impl PresentScheduler {
    fn invalidate(&mut self) {
        self.dirty = true;
    }

    fn request_present(&mut self) -> bool {
        if !self.dirty || self.pending {
            return false;
        }
        self.pending = true;
        true
    }

    fn begin_present(&mut self) -> bool {
        self.pending = false;
        std::mem::take(&mut self.dirty)
    }
}

#[derive(Clone, Debug)]
struct AdmittedNormalizedInput {
    envelope: NormalizedInputEnvelope,
    authority: NormalizedIngressAuthority,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContinuousQueueOutcome {
    Queued,
    ResetRequired,
}

fn queue_continuous_input(
    pending: &mut Vec<AdmittedNormalizedInput>,
    mut sample: AdmittedNormalizedInput,
) -> ContinuousQueueOutcome {
    const MAX_PENDING_CONTINUOUS_INPUT: usize = 256;
    if pending.len() >= MAX_PENDING_CONTINUOUS_INPUT {
        let order = sample.envelope.admission.order;
        let source = sample.envelope.source.clone();
        let recipient = sample.envelope.recipient;
        let mut reset = sample.clone();
        reset.envelope.input = nickel_input::InputEvent::FocusLost {
            order: nickel_input::EventOrder(order),
        };
        reset.envelope.clipboard_text = None;
        reset.envelope.source = source;
        reset.envelope.recipient = recipient;
        pending.clear();
        pending.push(reset);
        return ContinuousQueueOutcome::ResetRequired;
    }
    let event = sample.envelope.input;
    let nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Motion {
        device,
        order,
        position,
        delta,
    }) = event
    else {
        sample.envelope.input = event;
        pending.push(sample);
        return ContinuousQueueOutcome::Queued;
    };
    if let Some(queued) = pending.last_mut()
        && let nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Motion {
            device: queued_device,
            order: queued_order,
            position: queued_position,
            delta: queued_delta,
        }) = &mut queued.envelope.input
        && *queued_device == device
        && queued.authority == sample.authority
    {
        *queued_order = order;
        *queued_position = position;
        *queued_delta = match (*queued_delta, delta) {
            (Some(previous), Some(next)) => Some(nickel_input::Vector {
                x: previous.x + next.x,
                y: previous.y + next.y,
            }),
            (None, next) => next,
            (previous, None) => previous,
        };
        queued.envelope.admission = sample.envelope.admission;
        return ContinuousQueueOutcome::Queued;
    }
    sample.envelope.input = nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Motion {
        device,
        order,
        position,
        delta,
    });
    pending.push(sample);
    ContinuousQueueOutcome::Queued
}

fn transform_is_current(sample: &AdmittedNormalizedInput, current_generation: u64) -> bool {
    matches!(
        sample.envelope.input,
        nickel_input::InputEvent::FocusLost { .. }
    ) || (sample.envelope.transform_generation == Some(current_generation)
        && sample.authority.transform_generation == Some(current_generation))
}

fn revoke_native_ingress(
    recipient: &mut NormalizedRecipientBinding,
    stream_generation: &mut u64,
    reset_generation: u64,
) {
    *stream_generation = reset_generation;
    *recipient = NormalizedRecipientBinding {
        lease: 0,
        lifetime: reset_generation,
    };
}

fn grant_native_ingress(recipient: &mut NormalizedRecipientBinding, focus_generation: u64) {
    *recipient = NormalizedRecipientBinding {
        lease: focus_generation,
        lifetime: focus_generation,
    };
}

fn grant_native_ingress_if_focused(
    recipient: &mut NormalizedRecipientBinding,
    focus_observed: bool,
    focus_generation: u64,
) -> bool {
    if !focus_observed {
        return false;
    }
    grant_native_ingress(recipient, focus_generation);
    true
}

fn native_pointer_source_binding(
    stream_generation: u64,
    device_generation: u64,
) -> NormalizedSourceBinding {
    NormalizedSourceBinding {
        seat: 0,
        backend_stream: "winit-pointer-seat-0".into(),
        stream_generation,
        device_generation,
        identity_capability: "backend_generation".into(),
        reconnect_generation: device_generation,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeInputClass {
    Pointer,
    KeyboardText,
    Touch,
    Window,
}

fn native_input_class(input: &nickel_input::InputEvent) -> NativeInputClass {
    match input {
        nickel_input::InputEvent::Pointer(_) => NativeInputClass::Pointer,
        nickel_input::InputEvent::Key(_) | nickel_input::InputEvent::Text(_) => {
            NativeInputClass::KeyboardText
        }
        nickel_input::InputEvent::Touch(_) => NativeInputClass::Touch,
        nickel_input::InputEvent::FocusGained { .. }
        | nickel_input::InputEvent::FocusLost { .. }
        | nickel_input::InputEvent::DeviceRemoved { .. } => NativeInputClass::Window,
    }
}

fn native_input_source_binding(
    class: NativeInputClass,
    stream_generation: u64,
    device_generation: u64,
) -> NormalizedSourceBinding {
    if class == NativeInputClass::Pointer {
        return native_pointer_source_binding(stream_generation, device_generation);
    }
    let backend_stream = match class {
        NativeInputClass::KeyboardText => "winit-keyboard-seat-0",
        NativeInputClass::Touch => "winit-touch-seat-0",
        NativeInputClass::Window => "winit-window-seat-0",
        NativeInputClass::Pointer => unreachable!(),
    };
    NormalizedSourceBinding {
        seat: 0,
        backend_stream: backend_stream.into(),
        stream_generation,
        device_generation,
        identity_capability: "backend_generation".into(),
        reconnect_generation: device_generation,
    }
}

#[cfg(target_os = "linux")]
fn file_uri_list(paths: &[std::path::PathBuf]) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    let mut payload = Vec::new();
    for path in paths {
        payload.extend_from_slice(b"file://");
        for byte in path.as_os_str().as_bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') {
                payload.push(*byte);
            } else {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                payload.extend_from_slice(&[
                    b'%',
                    HEX[(byte >> 4) as usize],
                    HEX[(byte & 15) as usize],
                ]);
            }
        }
        payload.extend_from_slice(b"\r\n");
    }
    payload
}

#[cfg(target_os = "windows")]
fn start_windows_file_drag(paths: &[std::path::PathBuf]) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::{
        System::{
            Com::IDataObject,
            Ole::{DROPEFFECT_COPY, DROPEFFECT_MOVE, IDropSource},
        },
        UI::Shell::{ILCreateFromPathW, ILFree, SHCreateDataObject, SHDoDragDrop},
    };
    use windows::core::PCWSTR;
    let wide = paths
        .iter()
        .map(|path| {
            path.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let pidls = wide
        .iter()
        .map(|path| unsafe { ILCreateFromPathW(PCWSTR(path.as_ptr())) })
        .collect::<Vec<_>>();
    if pidls.iter().any(|pidl| pidl.is_null()) {
        return Err("could not create shell drag items".into());
    }
    let pointers = pidls
        .iter()
        .map(|pidl| *pidl as *const _)
        .collect::<Vec<_>>();
    let result = unsafe {
        let data: IDataObject = SHCreateDataObject(None, Some(&pointers), None::<&IDataObject>)
            .map_err(|error| error.to_string())?;
        SHDoDragDrop(
            None,
            &data,
            None::<&IDropSource>,
            DROPEFFECT_COPY | DROPEFFECT_MOVE,
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    };
    for pidl in pidls {
        unsafe { ILFree(Some(pidl.cast())) }
    }
    result
}

fn wait_duration(
    now: Instant,
    deadlines: impl IntoIterator<Item = Option<Instant>>,
) -> Option<Duration> {
    deadlines
        .into_iter()
        .flatten()
        .min()
        .map(|deadline| deadline.saturating_duration_since(now))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControllerPollSchedule {
    deadline: Instant,
}

impl ControllerPollSchedule {
    pub const CONNECTED_INTERVAL: Duration = Duration::from_millis(16);
    pub const DISCONNECTED_INTERVAL: Duration = Duration::from_millis(250);

    pub fn new(now: Instant) -> Self {
        Self { deadline: now }
    }

    pub fn deadline(self) -> Instant {
        self.deadline
    }

    pub fn is_due(self, now: Instant) -> bool {
        now >= self.deadline
    }

    pub fn mark_polled(&mut self, now: Instant, connected: bool) {
        self.deadline = now
            + if connected {
                Self::CONNECTED_INTERVAL
            } else {
                Self::DISCONNECTED_INTERVAL
            };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Submit,
    Newline,
    Escape,
    Reload,
    Back,
    Forward,
    DocumentStart,
    DocumentEnd,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileDragEvent {
    Hovered(std::path::PathBuf),
    HoverCancelled,
    ActionChanged(FileDragAction),
    Dropped(std::path::PathBuf),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileDragAction {
    Copy,
    Move,
}

/// An application request to begin a native outbound file drag.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutboundFileDrag {
    pub paths: Vec<std::path::PathBuf>,
}

/// Immutable environmental inputs for a declarative application view.
///
/// The host owns this state and supplies it before every resolve, so responsive
/// applications do not need a parallel resize callback or cached window size.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewContext {
    pub viewport: Rect,
    pub modality: InputModality,
    /// Host-owned keyboard/accessibility focus retained across declarative rebuilds.
    pub focused: Option<UiId>,
    /// Host-owned controller selection retained across declarative rebuilds.
    pub controller_target: Option<UiId>,
    /// Semantic actions accepted by the currently selected controller target.
    pub available_semantic_actions: Vec<ActionKind>,
    /// Depth of the active controller navigation scope.
    pub navigation_depth: usize,
    /// The topmost transient overlay, when controller navigation is trapped there.
    pub open_overlay: Option<OverlayId>,
}

/// Declarative content rendered above an application's ordinary view.
///
/// The host remains the sole owner of frame resolution, interaction state,
/// semantics, hit testing, and paint-list construction. Applications only
/// describe transient layers derived from their model.
#[derive(Clone, Debug)]
pub enum FrameOverlay<Message> {
    Menu(OverlayMenu<Message>),
    Surface(crate::TransientSurface),
    ContentSurface {
        surface: crate::TransientSurface,
        content: Box<crate::ui::Element<Message>>,
    },
    SelectionMarquee {
        rect: Rect,
        fill: Option<Color>,
        stroke: Color,
        width: f32,
    },
}

/// A named, interactive transient surface anchored to ordinary declarative UI.
/// Layout, focus trapping, dismissal, and focus return remain host-owned.
#[derive(Clone, Debug)]
pub struct Popover<Message> {
    surface: crate::TransientSurface,
    content: Box<crate::ui::Element<Message>>,
}

impl<Message> Popover<Message> {
    pub fn new(
        id: impl Into<UiId>,
        anchor: crate::OverlayAnchor,
        name: impl Into<String>,
        size: crate::Size,
        style: crate::OverlayStyle,
        content: impl crate::Component<Message>,
    ) -> Self {
        Self {
            surface: crate::TransientSurface::popover(id, anchor, size, style)
                .accessible_name(name),
            content: Box::new(content.into_element()),
        }
    }

    pub fn placement(mut self, placement: crate::OverlayPlacement) -> Self {
        self.surface = self.surface.placement(placement);
        self
    }

    pub fn collision(mut self, collision: crate::CollisionPolicy) -> Self {
        self.surface = self.surface.collision(collision);
        self
    }

    pub fn focus(mut self, focus: crate::OverlayFocusPolicy) -> Self {
        self.surface = self.surface.focus(focus);
        self
    }

    pub fn dismiss(mut self, dismiss: crate::DismissPolicy) -> Self {
        self.surface = self.surface.dismiss(dismiss);
        self
    }

    pub fn focus_return(mut self, target: impl Into<UiId>) -> Self {
        self.surface = self.surface.focus_return(target);
        self
    }

    pub fn direction(mut self, direction: crate::ReadingDirection) -> Self {
        self.surface = self.surface.direction(direction);
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.surface = self.surface.scale(scale);
        self
    }
}

/// A named, non-focus-stealing transient hint. Tooltips share the host's
/// collision and lifecycle machinery without pretending to be popovers.
#[derive(Clone, Debug)]
pub struct Tooltip<Message> {
    surface: crate::TransientSurface,
    content: Box<crate::ui::Element<Message>>,
}

impl<Message> Tooltip<Message> {
    pub fn new(
        id: impl Into<UiId>,
        anchor: crate::OverlayAnchor,
        name: impl Into<String>,
        size: crate::Size,
        style: crate::OverlayStyle,
        content: impl crate::Component<Message>,
    ) -> Self {
        Self {
            surface: crate::TransientSurface::tooltip(id, anchor, size, style)
                .accessible_name(name),
            content: Box::new(content.into_element()),
        }
    }

    pub fn placement(mut self, placement: crate::OverlayPlacement) -> Self {
        self.surface = self.surface.placement(placement);
        self
    }

    pub fn collision(mut self, collision: crate::CollisionPolicy) -> Self {
        self.surface = self.surface.collision(collision);
        self
    }

    pub fn dismiss(mut self, dismiss: crate::DismissPolicy) -> Self {
        self.surface = self.surface.dismiss(dismiss);
        self
    }

    pub fn direction(mut self, direction: crate::ReadingDirection) -> Self {
        self.surface = self.surface.direction(direction);
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.surface = self.surface.scale(scale);
        self
    }
}

impl<Message> From<Popover<Message>> for FrameOverlay<Message> {
    fn from(popover: Popover<Message>) -> Self {
        Self::ContentSurface {
            surface: popover.surface,
            content: popover.content,
        }
    }
}

impl<Message> From<Tooltip<Message>> for FrameOverlay<Message> {
    fn from(tooltip: Tooltip<Message>) -> Self {
        Self::ContentSurface {
            surface: tooltip.surface,
            content: tooltip.content,
        }
    }
}

impl<Message> FrameOverlay<Message> {
    pub fn surface(
        surface: crate::TransientSurface,
        content: impl crate::Component<Message>,
    ) -> Self {
        Self::ContentSurface {
            surface,
            content: Box::new(content.into_element()),
        }
    }
}

impl ViewContext {
    pub const fn new(viewport: Rect, modality: InputModality) -> Self {
        Self {
            viewport,
            modality,
            focused: None,
            controller_target: None,
            available_semantic_actions: Vec::new(),
            navigation_depth: 0,
            open_overlay: None,
        }
    }

    fn from_host(viewport: Rect, state: &UiStateStore, tree: Option<&UiFrame<impl Clone>>) -> Self {
        Self {
            viewport,
            modality: state.input_modality(),
            focused: state.focused().cloned(),
            controller_target: state.navigation().controller_selected().cloned(),
            available_semantic_actions: tree
                .map(|tree| tree.available_semantic_actions(state))
                .unwrap_or_default(),
            navigation_depth: tree.map(|tree| tree.navigation_depth(state)).unwrap_or(0),
            open_overlay: state.open_overlay_id().cloned(),
        }
    }
}

pub trait Application: Sized {
    type Message: Clone;

    fn update(&mut self, message: Self::Message);

    /// Whether current application state contains an authentication or other
    /// protected surface. Compositor adapters must reject remote observation
    /// and control of such a surface, regardless of an agent's lease scope.
    fn remote_access_protected(&self) -> bool {
        false
    }

    /// Application input policy shared by native and compositor-owned hosts.
    /// Runs before ordinary hit testing, once per normalized event; it must not
    /// recursively dispatch input. Native adapters own only window services.
    fn adapt_input(_host: &mut UiHost<Self>, _input: &nickel_input::InputEvent) -> AdapterOutcome {
        AdapterOutcome::default()
    }

    fn message_evidence(&self, _message: &Self::Message) -> MessageEvidence {
        MessageEvidence {
            type_name: std::any::type_name::<Self::Message>(),
            label: None,
        }
    }

    /// Drains application-owned effects produced by updates, completions, or
    /// polling so adapters, scenarios, and telemetry observe the same effects.
    fn take_effect_evidence(&mut self) -> Vec<EffectEvidence> {
        Vec::new()
    }

    /// Drains a semantic focus request produced by a domain update.
    ///
    /// The host resolves the stable application id against the rebuilt tree,
    /// including component-generated ancestor prefixes, and performs focus as
    /// an ordinary UI transition. Applications therefore never need to retain
    /// a frame or mutate [`UiStateStore`] to reconcile focus after rebuilding.
    fn take_focus_request(&mut self) -> Option<UiId> {
        None
    }

    /// Drains text explicitly offered to the system clipboard by an application update.
    fn take_clipboard_write(&mut self) -> Option<String> {
        None
    }

    /// Applies an application/domain completion injected by a host adapter or
    /// deterministic scenario. Implementations downcast the typed payload and
    /// return whether the completion changed declarative state.
    fn complete(&mut self, completion: Completion) -> Result<bool, CompletionFailure> {
        Err(CompletionFailure {
            id: completion.id,
            kind: CompletionFailureKind::Unhandled,
            detail: "application has no matching completion subscription".into(),
        })
    }

    fn view(&self, context: ViewContext) -> impl View<Self::Message>;

    fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        Vec::new()
    }

    /// Poll application-owned background work without introducing another UI runtime.
    /// Return `true` when new state requires a redraw.
    fn poll(&mut self) -> bool {
        false
    }

    /// Declares the cadence for application-owned completion polling.
    /// `None` means the application is event-driven and must not be woken.
    fn poll_interval(&self) -> Option<Duration> {
        None
    }

    /// Explicit application-level keyboard semantics before ordinary component activation.
    fn shortcut_outcome(&mut self, _shortcut: Shortcut) -> ShortcutOutcome {
        ShortcutOutcome::from_changed(false)
    }

    /// Offers normalized RGBA clipboard pixels to the focused application.
    /// Returning true makes image data win over simultaneous clipboard text.
    fn paste_clipboard_image(&mut self, _width: u32, _height: u32, _rgba: &[u8]) -> bool {
        false
    }

    /// Receives native file drag offers without exposing platform MIME or COM
    /// details to application policy.
    fn file_drag_event(&mut self, _event: FileDragEvent) -> bool {
        false
    }

    /// Takes a pending outbound file drag. Hosts call this synchronously while
    /// handling the pointer press/motion that supplied the native drag serial.
    fn take_outbound_file_drag(&mut self) -> Option<OutboundFileDrag> {
        None
    }

    /// Reports the controller presentation currently driving this host.
    /// Applications can retain it when controller-specific legends are part
    /// of their declarative view.
    fn controller_family_changed(&mut self, _family: ControllerFamily) -> bool {
        false
    }

    /// Reports the physical pixels per logical pixel used by the host.
    fn scale_factor_changed(&mut self, _scale_factor: f32) -> bool {
        false
    }

    fn title(&self) -> &str {
        "Nickel UI"
    }

    fn initial_size(&self) -> (u32, u32) {
        (800, 600)
    }
}

fn controller_ui_event(action: ControllerAction) -> Option<UiEvent> {
    match action {
        ControllerAction::Up => Some(UiEvent::ControllerUp),
        ControllerAction::Down => Some(UiEvent::ControllerDown),
        ControllerAction::Left => Some(UiEvent::ControllerLeft),
        ControllerAction::Right => Some(UiEvent::ControllerRight),
        ControllerAction::Confirm => Some(UiEvent::ControllerActivate),
        ControllerAction::Cancel => Some(UiEvent::ControllerBack),
        ControllerAction::PreviousPane => Some(UiEvent::ControllerPreviousPane),
        ControllerAction::NextPane => Some(UiEvent::ControllerNextPane),
        ControllerAction::Launcher => None,
        ControllerAction::ContextMenu => Some(UiEvent::ControllerContextMenu),
    }
}

const MAX_ADMITTED_CONTROLLER_PRESSES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct AdmittedControllerPress {
    device_generation: u64,
    action: ControllerAction,
    routing_epoch: u64,
    lease_epoch: u64,
    connection_generation: u64,
    stream_generation: u64,
    cutoff: Option<u64>,
    surface_generation: Option<u64>,
}

impl AdmittedControllerPress {
    fn new(binding: ControllerExecutionBinding, action: ControllerAction) -> Self {
        Self {
            device_generation: binding.device_generation,
            action,
            routing_epoch: binding.routing_epoch,
            lease_epoch: binding.lease_epoch,
            connection_generation: binding.connection_generation,
            stream_generation: binding.stream_generation,
            cutoff: binding.cutoff,
            surface_generation: binding.surface_generation,
        }
    }
}

pub struct UiHost<A: Application> {
    application: A,
    state: UiStateStore,
    tree: UiFrame<A::Message>,
    tree_remote_access_protected: bool,
    bounds: Rect,
    scale_factor: f32,
    input_dispatcher: FocusedInputDispatcher,
    frame_generation: u64,
    pointer_icon: PointerIcon,
    overlay_failures: Vec<OverlayDeclarationFailure>,
    next_application_deadline: Option<Instant>,
    pending_long_press: Option<PendingLongPress>,
    admitted_controller_presses: std::collections::BTreeSet<AdmittedControllerPress>,
    controller_press_authority: Option<ControllerExecutionAuthority>,
    controller_overflow_fence: Option<ControllerExecutionAuthority>,
    normalized_source_orders: std::collections::BTreeMap<(u64, String), (u64, u64, u64)>,
}

/// Runtime-owned state for one independently presented viewport of an application.
///
/// Embedders with several native surfaces backed by one application can park a
/// viewport here while another surface is active.  The application remains in
/// `UiHost`; layout, hit testing, focus, pointer state, and frame generations do
/// not leak between surfaces.
pub struct UiHostViewport<Message> {
    state: UiStateStore,
    tree: UiFrame<Message>,
    tree_remote_access_protected: bool,
    bounds: Rect,
    scale_factor: f32,
    input_dispatcher: FocusedInputDispatcher,
    frame_generation: u64,
    pointer_icon: PointerIcon,
    overlay_failures: Vec<OverlayDeclarationFailure>,
    next_application_deadline: Option<Instant>,
    pending_long_press: Option<PendingLongPress>,
    admitted_controller_presses: std::collections::BTreeSet<AdmittedControllerPress>,
    controller_press_authority: Option<ControllerExecutionAuthority>,
    controller_overflow_fence: Option<ControllerExecutionAuthority>,
    normalized_source_orders: std::collections::BTreeMap<(u64, String), (u64, u64, u64)>,
}

impl<Message> UiHostViewport<Message> {
    /// Observe a retained viewport without activating it or changing focus.
    /// The application owner must also validate current application protection.
    pub fn bounded_semantics(
        &self,
        max_nodes: usize,
        max_bytes: usize,
    ) -> Result<(u64, Vec<SemanticNodeSnapshot>), crate::BoundedSemanticError>
    where
        Message: Clone,
    {
        if self.remote_access_protected() {
            return Err(crate::BoundedSemanticError::ProtectedSurface);
        }
        Ok((
            self.frame_generation,
            self.tree.bounded_semantic_nodes(max_nodes, max_bytes)?,
        ))
    }

    /// Retired viewport trees retain their protection until the owner rebuilds them.
    pub fn remote_access_protected(&self) -> bool
    where
        Message: Clone,
    {
        self.tree_remote_access_protected || self.tree.has_protected_text()
    }

    pub fn pointer_interaction_active(&self) -> bool {
        self.state.pressed().is_some()
            || self.state.captured().is_some()
            || self.input_dispatcher.touch_active()
            || self.pending_long_press.is_some()
    }
}

#[derive(Clone, Debug)]
struct PendingLongPress {
    contact: nickel_input::TouchContactId,
    origin: crate::Point,
    deadline: Instant,
    source: NormalizedSourceBinding,
    recipient: NormalizedRecipientBinding,
    target: Option<UiId>,
    frame_generation: Option<u64>,
    transform_generation: Option<u64>,
    host_connection_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TouchIntentArbitration {
    Preserve,
    CancelThenDispatch,
    CancelGesture,
    RejectBusy,
}

const TOUCH_LONG_PRESS_DELAY: Duration = Duration::from_millis(500);
const TOUCH_LONG_PRESS_SLOP: f32 = 8.0;

/// Winit window delivery is currently treated as one aggregate device stream.
/// It therefore supports stream-local pairing and reset, but cannot claim
/// per-physical-mouse isolation until the runtime wires native device lifetimes
/// through `nickel_input::winit::DeviceRegistry`.
const STANDALONE_AGGREGATE_DEVICE: nickel_input::DeviceId = nickel_input::DeviceId(0);

#[derive(Clone, Default)]
struct OverlayInteractionSnapshot {
    target: Option<UiId>,
    controller_projection_active: bool,
    hovered: Option<UiId>,
    pressed: Option<UiId>,
    captured: Option<UiId>,
}

impl OverlayInteractionSnapshot {
    fn capture<Message: Clone>(state: &UiStateStore, tree: &UiFrame<Message>) -> Self {
        let owned = |id: Option<&UiId>| id.filter(|id| tree.contains_target(id)).cloned();
        let target = if let Some(overlay) = state.open_overlay_id() {
            state
                .current_target()
                .filter(|id| tree.is_descendant_or_self(overlay.as_ui_id(), id))
                .cloned()
        } else {
            owned(state.current_target())
        };
        Self {
            target,
            controller_projection_active: state.navigation().controller_projection_active(),
            hovered: owned(state.hovered()),
            pressed: owned(state.pressed()),
            captured: owned(state.captured()),
        }
    }

    fn restore<Message: Clone>(self, state: &mut UiStateStore, tree: &UiFrame<Message>) {
        let valid = |id: Option<UiId>| id.filter(|id| tree.contains_target(id));
        if state
            .current_target()
            .is_none_or(|target| !tree.contains_target(target))
            && self.target.is_some()
        {
            state.set_focus(valid(self.target));
        }
        if self.hovered.is_some() {
            state.set_hovered(valid(self.hovered));
        }
        if self.pressed.is_some() {
            state.set_pressed(valid(self.pressed));
        }
        if self.captured.is_some() {
            state.set_capture(valid(self.captured));
        }
        state
            .navigation_mut()
            .set_controller_projection_active(self.controller_projection_active);
    }

    fn restore_before_overlay(&self, state: &mut UiStateStore) {
        if let Some(focused) = &self.target {
            state.set_focus(Some(focused.clone()));
        }
        if let Some(hovered) = &self.hovered {
            state.set_hovered(Some(hovered.clone()));
        }
        if let Some(pressed) = &self.pressed {
            state.set_pressed(Some(pressed.clone()));
        }
        if let Some(captured) = &self.captured {
            state.set_capture(Some(captured.clone()));
        }
        state
            .navigation_mut()
            .set_controller_projection_active(self.controller_projection_active);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverlayDeclarationFailure {
    pub overlay: OverlayId,
    pub anchor: UiId,
    pub error: SemanticActionError,
}

// Normalized ingress is intentionally inline: batches are short-lived and this keeps the
// authority-bearing input transaction in one allocation through validation and dispatch.
#[allow(clippy::large_enum_variant)]
pub enum HostEvent {
    Ui(UiEvent),
    Controller(ControllerAction),
    AdmittedController {
        action: Option<ControllerAction>,
        binding: ControllerExecutionBinding,
    },
    Shortcut(Shortcut),
    Semantic {
        target: UiId,
        action: SemanticAction,
    },
    Accessibility {
        target: UiId,
        action: SemanticAction,
    },
    ControllerSemantic {
        target: UiId,
        action: SemanticAction,
    },
    Normalized {
        input: nickel_input::InputEvent,
        clipboard_text: Option<String>,
    },
    /// Authority-bearing normalized ingress. New production adapters must use
    /// this instead of the compatibility `Normalized` form.
    NormalizedIngress(NormalizedInputEnvelope),
    Poll,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedInputEnvelope {
    pub input: nickel_input::InputEvent,
    pub clipboard_text: Option<String>,
    pub source: NormalizedSourceBinding,
    pub admission: NormalizedAdmissionBinding,
    pub recipient: NormalizedRecipientBinding,
    pub operation: Option<u64>,
    pub transform_generation: Option<u64>,
    pub text_transaction: Option<u64>,
    pub transfer_cutoff: Option<u64>,
    pub broker_event_id: Option<u64>,
    pub host_connection_generation: u64,
    pub operation_epoch: Option<u64>,
    pub role: String,
    pub coordinate_meaning: String,
    pub composition_recipient_epoch: Option<u64>,
}

impl NormalizedInputEnvelope {
    pub fn execution_authority(&self) -> NormalizedIngressAuthority {
        NormalizedIngressAuthority {
            source: self.source.clone(),
            recipient: self.recipient,
            transfer_cutoff: self.transfer_cutoff,
            host_connection_generation: self.host_connection_generation,
            operation_epoch: self.operation_epoch,
            transform_generation: self.transform_generation,
            text_transaction: self.text_transaction,
            composition_recipient_epoch: self.composition_recipient_epoch,
            role: self.role.clone(),
            coordinate_meaning: self.coordinate_meaning.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedIngressAuthority {
    pub source: NormalizedSourceBinding,
    pub recipient: NormalizedRecipientBinding,
    pub transfer_cutoff: Option<u64>,
    pub host_connection_generation: u64,
    pub operation_epoch: Option<u64>,
    pub transform_generation: Option<u64>,
    pub text_transaction: Option<u64>,
    pub composition_recipient_epoch: Option<u64>,
    pub role: String,
    pub coordinate_meaning: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedSourceBinding {
    pub seat: u64,
    pub backend_stream: String,
    pub stream_generation: u64,
    pub device_generation: u64,
    pub identity_capability: String,
    pub reconnect_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NormalizedAdmissionBinding {
    pub order: u64,
    pub monotonic_micros: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NormalizedRecipientBinding {
    pub lease: u64,
    pub lifetime: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlobalAction {
    ToggleLauncher,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShortcutOutcome {
    pub disposition: crate::EventDisposition,
    pub changed: bool,
}

impl ShortcutOutcome {
    pub const fn from_changed(changed: bool) -> Self {
        Self {
            disposition: if changed {
                crate::EventDisposition::Handled
            } else {
                crate::EventDisposition::Unhandled
            },
            changed,
        }
    }

    pub const fn handled(changed: bool) -> Self {
        Self {
            disposition: crate::EventDisposition::Handled,
            changed,
        }
    }

    pub const fn rejected(reason: &'static str) -> Self {
        Self {
            disposition: crate::EventDisposition::Rejected(reason),
            changed: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AdapterOutcome {
    pub changed: bool,
    pub disposition: crate::EventDisposition,
    pub exit: bool,
}

impl AdapterOutcome {
    pub const fn changed() -> Self {
        Self {
            changed: true,
            disposition: crate::EventDisposition::Unhandled,
            exit: false,
        }
    }

    pub const fn consumed(changed: bool) -> Self {
        Self {
            changed,
            disposition: crate::EventDisposition::Handled,
            exit: false,
        }
    }

    pub const fn exit() -> Self {
        Self {
            changed: false,
            disposition: crate::EventDisposition::Handled,
            exit: true,
        }
    }
}

/// Read-only native services supplied to an application-specific host adapter.
/// Rendering, normalized input, clipboard routing, and controller navigation
/// remain owned by [`UiHost`].
pub struct HostServices<'a> {
    window: &'a Window,
}

impl<'a> HostServices<'a> {
    pub fn window(&self) -> &'a Window {
        self.window
    }
}

/// Injects platform-specific effects into the canonical Nickel UI runtime
/// without creating an application-owned event loop.
pub trait HostAdapter<A: Application> {
    /// Returns session-owned controller admission state. Session-aware adapters
    /// should fail closed when ownership cannot be established.
    fn controller_fence(&mut self, _services: HostServices<'_>) -> ControllerFence {
        ControllerFence::default()
    }

    /// Requests session-owned text entry for a controller-activated text field.
    fn request_text_entry(&mut self, _services: HostServices<'_>) {}

    fn poll_interval(&self) -> Option<Duration> {
        None
    }

    /// Declares the next adapter wakeup. Adapters without pending work return
    /// `None`, allowing the platform event loop to sleep indefinitely.
    fn next_deadline(&self, _now: Instant) -> Option<Instant> {
        None
    }

    fn started(
        &mut self,
        _host: &mut UiHost<A>,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn Error>> {
        Ok(AdapterOutcome::default())
    }

    fn event(
        &mut self,
        _host: &mut UiHost<A>,
        _event: &WindowEvent,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn Error>> {
        Ok(AdapterOutcome::default())
    }

    /// Observes canonical normalized input at the same dispatch boundary as
    /// [`UiHost`]. Continuous pointer input is therefore delivered only after
    /// the runtime has coalesced it for the next presentation.
    fn normalized_input(
        &mut self,
        _host: &mut UiHost<A>,
        _input: &nickel_input::InputEvent,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn Error>> {
        Ok(AdapterOutcome::default())
    }

    fn poll(
        &mut self,
        _host: &mut UiHost<A>,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn Error>> {
        Ok(AdapterOutcome::default())
    }

    fn global_action(
        &mut self,
        _host: &mut UiHost<A>,
        _action: GlobalAction,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn Error>> {
        Ok(AdapterOutcome::default())
    }

    fn stopped(
        &mut self,
        _host: &mut UiHost<A>,
        _services: HostServices<'_>,
    ) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
}

#[derive(Default)]
pub struct DefaultHostAdapter;

impl<A: Application> HostAdapter<A> for DefaultHostAdapter {}

#[derive(Default)]
pub struct HostBatch {
    /// Native embedders may bound clipboard ownership without truncating text.
    /// Oversized copy/cut is rejected before a cut can mutate the document.
    pub clipboard_text_limit: Option<usize>,
    /// Monotonic time supplied by an adapter or deterministic harness.
    pub now: Option<Instant>,
    /// The embedding host mutated application-owned view data directly.
    pub application_changed: bool,
    pub surface_size: Option<(u32, u32)>,
    pub scale_factor: Option<f32>,
    pub window_focused: Option<bool>,
    pub completions: Vec<Completion>,
    /// Current session authority revalidated at the final host execution boundary.
    pub controller_authority: Option<ControllerExecutionAuthority>,
    pub normalized_authorities: Vec<NormalizedIngressAuthority>,
    /// Failures observed by the transport while servicing this batch.  They
    /// are evidence, not application input: reporting one must not mutate or
    /// short-circuit the canonical UI transition.
    pub failures: Vec<HostFailure>,
    pub events: Vec<HostEvent>,
}

pub struct Completion {
    pub id: &'static str,
    payload: Box<dyn Any + Send>,
}

impl Completion {
    pub fn new<T: Any + Send>(id: &'static str, payload: T) -> Self {
        Self {
            id,
            payload: Box::new(payload),
        }
    }

    pub fn downcast<T: Any + Send>(self) -> Result<T, Self> {
        match self.payload.downcast::<T>() {
            Ok(value) => Ok(*value),
            Err(payload) => Err(Self {
                id: self.id,
                payload,
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionFailure {
    pub id: &'static str,
    pub kind: CompletionFailureKind,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionFailureKind {
    Unhandled,
    TypeMismatch,
    Rejected,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HostInspection {
    pub frame_generation: u64,
    pub semantic_generation: u64,
    pub input: InputContext,
    pub window_focused: bool,
    pub scale_factor: f32,
    pub pointer_icon: PointerIcon,
    pub keyboard_focus: Option<UiId>,
    /// Changes on focus transfer/loss, including away-and-back to the same ID.
    pub keyboard_focus_generation: u64,
    /// The semantic target currently owned by production pointer hit testing.
    pub pointer_hover: Option<UiId>,
    pub pointer_capture: Option<UiId>,
    pub controller_target: Option<UiId>,
    pub controller_scope: Option<UiId>,
    pub navigation_depth: usize,
    pub available_semantic_actions: Vec<ActionKind>,
    pub controller_editing: bool,
    pub target_mode: crate::WidgetTargetMode,
    pub open_overlay: Option<OverlayId>,
    pub modality: InputModality,
    pub diagnostics: Vec<LayoutDiagnostic>,
    pub resources: FrameResourceDiagnostics,
    pub overlay_failures: Vec<OverlayDeclarationFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageEvidence {
    pub type_name: &'static str,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectEvidence {
    pub type_name: &'static str,
    pub label: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostFailureStage {
    Presenter,
    Clipboard,
    Ime,
    Accessibility,
    Controller,
    DomainService,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostFailure {
    pub surface: String,
    pub stage: HostFailureStage,
    pub optional: bool,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HostChangeToken {
    pub frame_generation: u64,
    pub semantic_generation: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HostEventOutcome {
    pub changed: bool,
    pub disposition: crate::EventDisposition,
    pub invalidation: Invalidation,
    pub messages: Vec<MessageEvidence>,
    pub effects: Vec<EffectEvidence>,
    pub failures: Vec<HostFailure>,
    pub completion_failures: Vec<CompletionFailure>,
    pub pointer_icon: PointerIcon,
    pub text_input_active: bool,
    pub accessibility_generation: u64,
    pub change_token: HostChangeToken,
    pub next_deadline: Option<Instant>,
    pub telemetry: HostTelemetry,
    pub clipboard_text: Option<String>,
    pub semantic_failures: Vec<SemanticActionFailure>,
    pub global_actions: Vec<GlobalAction>,
    pub controller_executions: Vec<ControllerExecutionEvidence>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControllerExecutionDisposition {
    Executed,
    RejectedBusy,
    RejectedStale,
    RejectedUnpairedRelease,
    ResetOverflow,
    RejectedResetFence,
    ResetNeutralized,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControllerExecutionEvidence {
    pub binding: ControllerExecutionBinding,
    pub disposition: ControllerExecutionDisposition,
    pub message_count: usize,
    pub effect_count: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HostTelemetry {
    pub events_processed: usize,
    pub completions_processed: usize,
    pub rebuilt: bool,
    /// Time from beginning the host step through application message dispatch.
    pub input_to_message_us: u64,
    /// Time from beginning the host step through the resolved frame.
    pub input_to_frame_us: u64,
    /// Time spent resolving declarative layout during this step.
    pub layout_us: u64,
    /// Time spent constructing the application's declarative view/paint list.
    pub paint_list_us: u64,
    /// Explicit host/application deadline wakeups processed by this step.
    pub scheduled_wakeups: usize,
    /// Retained bytes owned by the resolved frame after this step.
    pub retained_frame_bytes: usize,
    /// Allocation counting is not available in the shared runtime unless a
    /// concrete adapter installs an allocator-visible counter.
    pub allocation_count: Option<u64>,
}

impl Default for HostEventOutcome {
    fn default() -> Self {
        Self {
            changed: false,
            disposition: crate::EventDisposition::Unhandled,
            invalidation: Invalidation::None,
            messages: Vec::new(),
            effects: Vec::new(),
            failures: Vec::new(),
            completion_failures: Vec::new(),
            pointer_icon: PointerIcon::Default,
            text_input_active: false,
            accessibility_generation: 0,
            change_token: HostChangeToken::default(),
            next_deadline: None,
            telemetry: HostTelemetry::default(),
            clipboard_text: None,
            semantic_failures: Vec::new(),
            global_actions: Vec::new(),
            controller_executions: Vec::new(),
        }
    }
}

impl HostEventOutcome {
    fn merge(&mut self, mut other: Self) {
        self.changed |= other.changed;
        self.disposition = self.disposition.merge(other.disposition);
        self.invalidation = self.invalidation.merge(other.invalidation);
        self.messages.append(&mut other.messages);
        self.effects.append(&mut other.effects);
        self.failures.append(&mut other.failures);
        if other.clipboard_text.is_some() {
            self.clipboard_text = other.clipboard_text;
        }
        self.semantic_failures.append(&mut other.semantic_failures);
        self.completion_failures
            .append(&mut other.completion_failures);
        self.global_actions.append(&mut other.global_actions);
        self.controller_executions
            .append(&mut other.controller_executions);
        self.pointer_icon = other.pointer_icon;
        self.text_input_active = other.text_input_active;
        self.accessibility_generation = self
            .accessibility_generation
            .max(other.accessibility_generation);
        self.change_token.frame_generation = self
            .change_token
            .frame_generation
            .max(other.change_token.frame_generation);
        self.change_token.semantic_generation = self
            .change_token
            .semantic_generation
            .max(other.change_token.semantic_generation);
        self.next_deadline = match (self.next_deadline, other.next_deadline) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (deadline @ Some(_), None) | (None, deadline @ Some(_)) => deadline,
            (None, None) => None,
        };
        self.telemetry.events_processed = self
            .telemetry
            .events_processed
            .saturating_add(other.telemetry.events_processed);
        self.telemetry.completions_processed = self
            .telemetry
            .completions_processed
            .saturating_add(other.telemetry.completions_processed);
        self.telemetry.rebuilt |= other.telemetry.rebuilt;
        self.telemetry.input_to_message_us = self
            .telemetry
            .input_to_message_us
            .saturating_add(other.telemetry.input_to_message_us);
        self.telemetry.input_to_frame_us = self
            .telemetry
            .input_to_frame_us
            .saturating_add(other.telemetry.input_to_frame_us);
        self.telemetry.layout_us = self
            .telemetry
            .layout_us
            .saturating_add(other.telemetry.layout_us);
        self.telemetry.paint_list_us = self
            .telemetry
            .paint_list_us
            .saturating_add(other.telemetry.paint_list_us);
        self.telemetry.scheduled_wakeups = self
            .telemetry
            .scheduled_wakeups
            .saturating_add(other.telemetry.scheduled_wakeups);
        self.telemetry.retained_frame_bytes = self
            .telemetry
            .retained_frame_bytes
            .max(other.telemetry.retained_frame_bytes);
        self.telemetry.allocation_count = match (
            self.telemetry.allocation_count,
            other.telemetry.allocation_count,
        ) {
            (Some(left), Some(right)) => Some(left.saturating_add(right)),
            _ => None,
        };
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundedSemanticActionError {
    InputBusy,
    Snapshot(crate::BoundedSemanticError),
    StaleGeneration,
    MissingTarget,
    ActionUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionFailure {
    pub target: UiId,
    pub error: SemanticActionError,
}

impl<A: Application> UiHost<A> {
    /// Builds fresh runtime state for a new native viewport without cloning the
    /// application or borrowing interaction state from another surface.
    pub fn new_viewport(&self, width: u32, height: u32) -> UiHostViewport<A::Message> {
        let bounds = Rect::new(0.0, 0.0, width as f32, height as f32);
        let mut state = UiStateStore::default();
        let context = ViewContext::from_host(bounds, &state, None::<&UiFrame<A::Message>>);
        let mut tree = UiFrame::resolve(
            self.application.view(context.clone()),
            FrameRequest::new(bounds, &mut state),
        );
        let overlay_failures = apply_frame_overlays(
            &mut tree,
            &mut state,
            self.application.frame_overlays(context),
        );
        UiHostViewport {
            tree_remote_access_protected: self.application.remote_access_protected(),
            state,
            tree,
            bounds,
            scale_factor: 1.0,
            input_dispatcher: FocusedInputDispatcher::default(),
            frame_generation: 1,
            pointer_icon: PointerIcon::Default,
            overlay_failures,
            next_application_deadline: self
                .application
                .poll_interval()
                .map(|interval| Instant::now() + interval),
            pending_long_press: None,
            admitted_controller_presses: std::collections::BTreeSet::new(),
            controller_press_authority: None,
            controller_overflow_fence: None,
            normalized_source_orders: std::collections::BTreeMap::new(),
        }
    }

    /// Activates one viewport and returns the previously active viewport.
    pub fn replace_viewport(
        &mut self,
        viewport: UiHostViewport<A::Message>,
    ) -> UiHostViewport<A::Message> {
        UiHostViewport {
            state: std::mem::replace(&mut self.state, viewport.state),
            tree: std::mem::replace(&mut self.tree, viewport.tree),
            tree_remote_access_protected: std::mem::replace(
                &mut self.tree_remote_access_protected,
                viewport.tree_remote_access_protected,
            ),
            bounds: std::mem::replace(&mut self.bounds, viewport.bounds),
            scale_factor: std::mem::replace(&mut self.scale_factor, viewport.scale_factor),
            input_dispatcher: std::mem::replace(
                &mut self.input_dispatcher,
                viewport.input_dispatcher,
            ),
            frame_generation: std::mem::replace(
                &mut self.frame_generation,
                viewport.frame_generation,
            ),
            pointer_icon: std::mem::replace(&mut self.pointer_icon, viewport.pointer_icon),
            overlay_failures: std::mem::replace(
                &mut self.overlay_failures,
                viewport.overlay_failures,
            ),
            next_application_deadline: std::mem::replace(
                &mut self.next_application_deadline,
                viewport.next_application_deadline,
            ),
            pending_long_press: std::mem::replace(
                &mut self.pending_long_press,
                viewport.pending_long_press,
            ),
            admitted_controller_presses: std::mem::replace(
                &mut self.admitted_controller_presses,
                viewport.admitted_controller_presses,
            ),
            controller_press_authority: std::mem::replace(
                &mut self.controller_press_authority,
                viewport.controller_press_authority,
            ),
            controller_overflow_fence: std::mem::replace(
                &mut self.controller_overflow_fence,
                viewport.controller_overflow_fence,
            ),
            normalized_source_orders: std::mem::replace(
                &mut self.normalized_source_orders,
                viewport.normalized_source_orders,
            ),
        }
    }
    pub fn paste_clipboard_image(&mut self, width: u32, height: u32, rgba: &[u8]) -> bool {
        if self.application.paste_clipboard_image(width, height, rgba) {
            self.rebuild();
            true
        } else {
            false
        }
    }
    pub fn set_controller_family(&mut self, family: ControllerFamily) -> bool {
        if self.application.controller_family_changed(family) {
            self.rebuild();
            true
        } else {
            false
        }
    }
    pub fn set_scale_factor(&mut self, scale_factor: f32) -> bool {
        self.step(HostBatch {
            scale_factor: Some(scale_factor),
            ..HostBatch::default()
        })
        .changed
    }
    pub fn new(application: A, width: u32, height: u32) -> Self {
        Self::new_at(application, width, height, Instant::now())
    }

    /// Constructs a host against an explicit monotonic origin for deterministic
    /// scheduling tests and embedded adapters that already own the clock.
    pub fn new_at(application: A, width: u32, height: u32, now: Instant) -> Self {
        let bounds = Rect::new(0.0, 0.0, width as f32, height as f32);
        let mut state = UiStateStore::default();
        let context = ViewContext::from_host(bounds, &state, None::<&UiFrame<A::Message>>);
        let mut tree = UiFrame::resolve(
            application.view(context.clone()),
            FrameRequest::new(bounds, &mut state),
        );
        let overlay_failures =
            apply_frame_overlays(&mut tree, &mut state, application.frame_overlays(context));
        let next_application_deadline = application.poll_interval().map(|interval| now + interval);
        Self {
            tree_remote_access_protected: application.remote_access_protected(),
            application,
            state,
            tree,
            bounds,
            scale_factor: 1.0,
            input_dispatcher: FocusedInputDispatcher::default(),
            frame_generation: 1,
            pointer_icon: PointerIcon::Default,
            overlay_failures,
            next_application_deadline,
            pending_long_press: None,
            admitted_controller_presses: std::collections::BTreeSet::new(),
            controller_press_authority: None,
            controller_overflow_fence: None,
            normalized_source_orders: std::collections::BTreeMap::new(),
        }
    }

    pub fn application_mut(&mut self) -> &mut A {
        &mut self.application
    }

    pub fn application(&self) -> &A {
        &self.application
    }

    pub fn commands(&self) -> &[crate::PaintCommand] {
        self.tree.commands()
    }

    /// Exports the current resolved display list without copying commands or
    /// shared image pixels. This is the presentation boundary used by native
    /// compositor and software backends alike.
    pub fn render_frame(&self) -> crate::backend::RenderFrame<'_> {
        crate::backend::RenderFrame {
            commands: self.tree.commands(),
            logical_size: (
                self.bounds.size.width as u32,
                self.bounds.size.height as u32,
            ),
            scale_factor: self.scale_factor,
            generation: self.frame_generation,
        }
    }

    pub fn render_with<R: crate::backend::FrameRenderer>(
        &self,
        renderer: &mut R,
    ) -> Result<DamageRegion, R::Error> {
        renderer.render_frame(self.render_frame())
    }

    pub fn render_software(&self, renderer: &mut SoftwareRenderer) -> DamageRegion {
        match self.render_with(renderer) {
            Ok(damage) => damage,
            Err(error) => match error {},
        }
    }

    pub fn pointer_icon_at(&self, point: crate::Point) -> PointerIcon {
        self.tree.pointer_icon_at(point)
    }

    pub fn input_context(&self) -> crate::InputContext {
        crate::InputContext {
            text_focused: self
                .state
                .focused()
                .is_some_and(|id| self.tree.is_text_input(id)),
            navigation_active: self.state.navigation().controller_selected().is_some(),
            selection_owned: self.state.selection_owner().is_some(),
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.tree.selected_text(&self.state)
    }

    /// Generation of the currently resolved tree, not a presentation acknowledgement.
    pub fn resolved_frame_generation(&self) -> u64 {
        self.frame_generation
    }

    /// Bounded, protected-value-free projection. Adapters still own authorization.
    pub fn bounded_semantic_nodes(
        &self,
        max_nodes: usize,
        max_bytes: usize,
    ) -> Result<Vec<SemanticNodeSnapshot>, crate::BoundedSemanticError> {
        if self.application().remote_access_protected() || self.tree_remote_access_protected {
            return Err(crate::BoundedSemanticError::ProtectedSurface);
        }
        self.tree.bounded_semantic_nodes(max_nodes, max_bytes)
    }

    pub fn semantic_nodes(&self) -> Vec<SemanticNodeSnapshot> {
        self.tree.semantic_nodes()
    }

    /// Protection is queried from live application state as well as the tree,
    /// so an authentication transition is protected before its next paint.
    /// A previously protected tree remains protected until it is rebuilt.
    pub fn remote_access_protected(&self) -> bool {
        self.application().remote_access_protected()
            || self.tree_remote_access_protected
            || self.tree.has_protected_text()
    }

    pub fn query(&self, selector: &SemanticSelector) -> Vec<SemanticNodeSnapshot> {
        self.tree.query(selector)
    }

    pub fn query_unique(
        &self,
        selector: &SemanticSelector,
    ) -> Result<SemanticNodeSnapshot, SemanticQueryError> {
        self.tree.query_unique(selector)
    }

    pub fn semantic_targets_for_message(&self, message: &A::Message) -> Vec<crate::SemanticTarget>
    where
        A::Message: PartialEq,
    {
        self.tree.semantic_targets_for_message(message)
    }

    /// Returns the typed application message bound to a semantic target.
    ///
    /// Semantic IDs are opaque identity tokens. Adapters that need to route
    /// hover or other non-activating interactions can use this lookup instead
    /// of recovering application meaning by parsing an ID's text.
    pub fn message_for_semantic_target(&self, target: &UiId) -> Option<&A::Message> {
        self.tree.message_for_id(target)
    }

    /// Returns the typed callback used by a specific semantic invocation.
    /// Activation and context-menu actions can dispatch different messages.
    pub fn message_for_semantic_action(
        &self,
        target: &UiId,
        action: ActionKind,
    ) -> Option<&A::Message> {
        self.tree.message_for_semantic_action(target, action)
    }

    pub fn unique_semantic_target_for_message(
        &self,
        message: &A::Message,
    ) -> Result<crate::SemanticTarget, SemanticQueryError>
    where
        A::Message: PartialEq,
    {
        self.tree.unique_semantic_target_for_message(message)
    }

    pub fn accessibility_nodes(&self) -> &[AccessibilityNode] {
        self.tree.accessibility_nodes()
    }

    pub fn resolved_grid_columns(&self) -> Option<usize> {
        self.tree.resolved_grid_columns()
    }

    /// Scrolls the canonical view state just enough to reveal a message-bound
    /// item. The host owns both geometry and scroll state; applications only
    /// name the item and its scroll surface.
    pub fn ensure_message_visible(&mut self, item: &A::Message, scroll: &A::Message) -> bool
    where
        A::Message: PartialEq,
    {
        let Ok(item_target) = self.tree.unique_semantic_target_for_message(item) else {
            return false;
        };
        let item_rect = item_target.bounds;
        let Some(viewport) = self.tree.scroll_viewport(scroll) else {
            return false;
        };
        let Some(extent) = self.tree.scroll_extent(scroll) else {
            return false;
        };
        let Some(target) = self
            .tree
            .semantic_targets_for_message(scroll)
            .into_iter()
            .next()
        else {
            return false;
        };
        let delta = if item_rect.origin.y < viewport.origin.y {
            item_rect.origin.y - viewport.origin.y
        } else {
            let item_bottom = item_rect.origin.y + item_rect.size.height;
            let viewport_bottom = viewport.origin.y + viewport.size.height;
            (item_bottom - viewport_bottom).max(0.0)
        };
        let maximum = (extent.content.height - extent.viewport.height).max(0.0);
        let changed = self.state.scroll_by(target.id, delta, maximum) != crate::Invalidation::None;
        if changed {
            self.rebuild();
        }
        changed
    }

    pub fn reset_scroll(&mut self, scroll: &A::Message) -> bool
    where
        A::Message: PartialEq,
    {
        let Some(extent) = self.tree.scroll_extent(scroll) else {
            return false;
        };
        let Some(target) = self
            .tree
            .semantic_targets_for_message(scroll)
            .into_iter()
            .next()
        else {
            return false;
        };
        let maximum = (extent.content.height - extent.viewport.height).max(0.0);
        let current = self
            .state
            .state(&target.id)
            .map_or(extent.offset, |state| state.scroll_offset);
        let changed =
            self.state.scroll_by(target.id, -current, maximum) != crate::Invalidation::None;
        if changed {
            self.rebuild();
        }
        changed
    }

    pub fn resolve_effective_target(
        &self,
        target: &UiId,
        action: ActionKind,
    ) -> Result<EffectiveHitRoute, SemanticActionError> {
        self.tree.resolve_effective_target(target, action)
    }

    /// True while this viewport owns a pointer press/capture or touch contact.
    /// This does not describe native seat keyboard or other surface ownership.
    pub fn pointer_interaction_active(&self) -> bool {
        self.state.pressed().is_some()
            || self.state.captured().is_some()
            || self.input_dispatcher.touch_active()
            || self.pending_long_press.is_some()
    }

    /// Resolve an ordinal from the same bounded projection used for observation,
    /// then dispatch through production semantics. Adapters must validate surface
    /// identity and acquire their input reservation before calling this method.
    /// The returned host outcome is for local effect handling, never a remote payload.
    /// An explicit clipboard limit replaces the previous adapter limit; otherwise
    /// the current limit is retained.
    pub fn perform_bounded_semantic_action(
        &mut self,
        expected_generation: u64,
        ordinal: usize,
        action: SemanticAction,
        max_nodes: usize,
        max_bytes: usize,
        clipboard_text_limit: Option<usize>,
    ) -> Result<HostEventOutcome, BoundedSemanticActionError> {
        if self.frame_generation != expected_generation {
            return Err(BoundedSemanticActionError::StaleGeneration);
        }
        if self.pointer_interaction_active() {
            return Err(BoundedSemanticActionError::InputBusy);
        }
        match &action {
            SemanticAction::SetValue(crate::SemanticValueInput::Text(text))
                if text.len() > max_bytes =>
            {
                return Err(BoundedSemanticActionError::Snapshot(
                    crate::BoundedSemanticError::BudgetExceeded,
                ));
            }
            SemanticAction::SetValue(crate::SemanticValueInput::Number(value))
                if !value.is_finite() =>
            {
                return Err(BoundedSemanticActionError::ActionUnavailable);
            }
            _ => {}
        }
        let nodes = self
            .bounded_semantic_nodes(max_nodes, max_bytes)
            .map_err(BoundedSemanticActionError::Snapshot)?;
        let target = nodes
            .into_iter()
            .nth(ordinal)
            .ok_or(BoundedSemanticActionError::MissingTarget)?;
        let required = match &action {
            SemanticAction::Invoke(kind) => *kind,
            SemanticAction::SetValue(_) => ActionKind::SetValue,
        };
        if !target.actions.contains(&required) {
            return Err(BoundedSemanticActionError::ActionUnavailable);
        }
        // This entry point runs outside the adapter's ordinary batch builder.
        // Preserve its clipboard policy when entering the production reducer.
        Ok(self.step(HostBatch {
            events: vec![HostEvent::Semantic {
                target: target.id,
                action,
            }],
            clipboard_text_limit: clipboard_text_limit.or(self.state.clipboard_text_limit),
            ..HostBatch::default()
        }))
    }

    pub fn perform_semantic_action(
        &mut self,
        target: UiId,
        action: SemanticAction,
    ) -> HostEventOutcome {
        self.step(HostBatch {
            events: vec![HostEvent::Semantic { target, action }],
            ..HostBatch::default()
        })
    }

    pub fn perform_accessibility_action(
        &mut self,
        target: UiId,
        action: SemanticAction,
    ) -> HostEventOutcome {
        self.step(HostBatch {
            events: vec![HostEvent::Accessibility { target, action }],
            ..HostBatch::default()
        })
    }

    pub fn perform_controller_semantic_action(
        &mut self,
        target: UiId,
        action: SemanticAction,
    ) -> HostEventOutcome {
        self.step(HostBatch {
            events: vec![HostEvent::ControllerSemantic { target, action }],
            ..HostBatch::default()
        })
    }

    pub fn open_transient(&mut self, id: OverlayId, invocation_target: UiId) -> bool {
        let changed = self.state.open_overlay(id, invocation_target) != Invalidation::None;
        if changed {
            self.rebuild();
        }
        changed
    }

    /// Requests focus through the frame reducer without changing the user's
    /// current input modality. Adapters use this after an application update
    /// resolves a new semantic descendant.
    pub fn request_focus(&mut self, target: UiId) -> HostEventOutcome {
        let transition = self
            .tree
            .transition(
                &mut self.state,
                InputSource::System,
                InteractionIntent::Event(UiEvent::AccessibilityFocus(target)),
            )
            .expect("system focus is an ordinary frame event");
        let changed =
            transition.invalidation != crate::Invalidation::None || !transition.messages.is_empty();
        for message in transition.messages {
            self.application.update(message);
        }
        let mut outcome = HostEventOutcome {
            changed,
            clipboard_text: transition.clipboard_text,
            ..HostEventOutcome::default()
        };
        if changed {
            self.rebuild();
        }
        outcome.effects = self.application.take_effect_evidence();
        outcome.pointer_icon = self.pointer_icon;
        outcome.text_input_active = self.input_context().text_focused;
        outcome.accessibility_generation = self.frame_generation;
        outcome.change_token = HostChangeToken {
            frame_generation: self.frame_generation,
            semantic_generation: self.frame_generation,
        };
        outcome
    }

    /// Whether controller activation currently addresses an editable text field.
    /// Controller adjustment mode is reserved for value controls, not text editors.
    pub fn controller_targets_text_input(&self) -> bool {
        self.state
            .navigation()
            .controller_selected()
            .or_else(|| self.state.focused())
            .is_some_and(|target| self.tree.is_text_input(target))
    }

    pub fn inspect(&self) -> HostInspection {
        HostInspection {
            frame_generation: self.frame_generation,
            semantic_generation: self.frame_generation,
            input: self.input_context(),
            window_focused: self.state.window_focused(),
            scale_factor: self.scale_factor,
            pointer_icon: self.pointer_icon,
            keyboard_focus: self.state.focused().cloned(),
            keyboard_focus_generation: self.state.focus_generation(),
            pointer_hover: self.state.hovered().cloned(),
            pointer_capture: self.state.captured().cloned(),
            controller_target: self.state.navigation().controller_selected().cloned(),
            controller_scope: self.state.navigation().controller_scope().cloned(),
            navigation_depth: self.tree.navigation_depth(&self.state),
            available_semantic_actions: self.tree.available_semantic_actions(&self.state),
            controller_editing: self.state.navigation().controller_editing(),
            target_mode: self.state.navigation().target_mode(),
            open_overlay: self.state.open_overlay_id().cloned(),
            modality: self.state.input_modality(),
            diagnostics: self.tree.diagnostics().to_vec(),
            resources: self.tree.resource_diagnostics(),
            overlay_failures: self.overlay_failures.clone(),
        }
    }

    fn update_admitted_long_press(&mut self, envelope: &NormalizedInputEnvelope, now: Instant) {
        if self.pending_long_press.as_ref().is_some_and(|pending| {
            pending.source.seat == envelope.source.seat
                && pending.source.backend_stream == envelope.source.backend_stream
                && (pending.source.stream_generation != envelope.source.stream_generation
                    || pending.recipient != envelope.recipient
                    || pending.transform_generation != envelope.transform_generation
                    || pending.host_connection_generation != envelope.host_connection_generation)
        }) {
            self.pending_long_press = None;
        }
        match &envelope.input {
            nickel_input::InputEvent::Touch(nickel_input::TouchEvent::Started {
                device,
                contact,
                position,
                ..
            }) if !self.input_dispatcher.pointer_interaction_active() => {
                self.pending_long_press = Some(PendingLongPress {
                    contact: nickel_input::TouchContactId::new(*device, *contact),
                    origin: crate::Point {
                        x: position.x as f32,
                        y: position.y as f32,
                    },
                    deadline: now + TOUCH_LONG_PRESS_DELAY,
                    source: envelope.source.clone(),
                    recipient: envelope.recipient,
                    target: self
                        .tree
                        .id_at(crate::Point {
                            x: position.x as f32,
                            y: position.y as f32,
                        })
                        .cloned(),
                    frame_generation: None,
                    transform_generation: envelope.transform_generation,
                    host_connection_generation: envelope.host_connection_generation,
                });
            }
            nickel_input::InputEvent::Touch(nickel_input::TouchEvent::Moved {
                device,
                contact,
                position,
                ..
            }) if self.pending_long_press.as_ref().is_some_and(|pending| {
                pending.contact == nickel_input::TouchContactId::new(*device, *contact)
            }) =>
            {
                let pending = self
                    .pending_long_press
                    .as_ref()
                    .expect("matched pending touch");
                let dx = position.x as f32 - pending.origin.x;
                let dy = position.y as f32 - pending.origin.y;
                if dx * dx + dy * dy > TOUCH_LONG_PRESS_SLOP * TOUCH_LONG_PRESS_SLOP {
                    self.pending_long_press = None;
                }
            }
            nickel_input::InputEvent::Touch(
                nickel_input::TouchEvent::Ended {
                    device, contact, ..
                }
                | nickel_input::TouchEvent::Cancelled {
                    device, contact, ..
                },
            ) if self.pending_long_press.as_ref().is_some_and(|pending| {
                pending.contact == nickel_input::TouchContactId::new(*device, *contact)
            }) =>
            {
                self.pending_long_press = None;
            }
            nickel_input::InputEvent::FocusLost { .. } => {
                self.pending_long_press = None;
            }
            nickel_input::InputEvent::DeviceRemoved { device, .. }
                if self
                    .pending_long_press
                    .as_ref()
                    .is_some_and(|pending| pending.contact.device == *device) =>
            {
                self.pending_long_press = None;
            }
            _ => {}
        }
    }

    fn arbitrate_touch_ownership(&mut self) -> HostEventOutcome {
        let pending = self.pending_long_press.take().is_some();
        let active = self.input_dispatcher.cancel_touch_ownership();
        let tree_owned = self.state.pressed().is_some() || self.state.captured().is_some();
        if pending || active || tree_owned {
            self.dispatch_ui_event_unarbitrated(UiEvent::PointerCancelled)
        } else {
            HostEventOutcome::default()
        }
    }

    fn touch_owner_target(&self) -> Option<&UiId> {
        self.pending_long_press
            .as_ref()
            .and_then(|pending| pending.target.as_ref())
            .or_else(|| self.state.captured())
    }

    fn arbitrate_activation_target(&self, target: Option<&UiId>) -> TouchIntentArbitration {
        let Some(owner) = self.touch_owner_target() else {
            return TouchIntentArbitration::Preserve;
        };
        match target {
            Some(target) if target == owner => TouchIntentArbitration::RejectBusy,
            Some(_) => TouchIntentArbitration::CancelThenDispatch,
            None => TouchIntentArbitration::Preserve,
        }
    }

    fn touch_arbitration_for_event(&self, event: &HostEvent) -> TouchIntentArbitration {
        let resolved_action = |target: &UiId, action: ActionKind| {
            self.tree
                .resolve_effective_target(target, action)
                .ok()
                .map(|route| route.target)
        };
        match event {
            HostEvent::Semantic { target, action }
            | HostEvent::Accessibility { target, action }
            | HostEvent::ControllerSemantic { target, action } => {
                let kind = match action {
                    SemanticAction::Invoke(kind) => *kind,
                    SemanticAction::SetValue(_) => ActionKind::SetValue,
                };
                resolved_action(target, kind)
                    .as_ref()
                    .map_or(TouchIntentArbitration::Preserve, |target| {
                        self.arbitrate_activation_target(Some(target))
                    })
            }
            _ => TouchIntentArbitration::Preserve,
        }
    }

    fn touch_arbitration_for_ui_event(&self, event: &UiEvent) -> TouchIntentArbitration {
        let Some(owner) = self.touch_owner_target() else {
            return TouchIntentArbitration::Preserve;
        };
        if matches!(event, UiEvent::ControllerBack) {
            return TouchIntentArbitration::CancelGesture;
        }
        if matches!(event, UiEvent::PointerContext(_)) {
            return TouchIntentArbitration::RejectBusy;
        }
        let relevant = matches!(
            event,
            UiEvent::FocusNext
                | UiEvent::FocusPrevious
                | UiEvent::KeyboardNavigateUp
                | UiEvent::KeyboardNavigateDown
                | UiEvent::KeyboardNavigateLeft
                | UiEvent::KeyboardNavigateRight
                | UiEvent::KeyboardNavigateActivate
                | UiEvent::ControllerUp
                | UiEvent::ControllerDown
                | UiEvent::ControllerLeft
                | UiEvent::ControllerRight
                | UiEvent::ControllerNext
                | UiEvent::ControllerPrevious
                | UiEvent::ControllerAdjust(_)
                | UiEvent::ControllerActivate
                | UiEvent::ControllerContextMenu
                | UiEvent::KeyboardContextMenu
                | UiEvent::AccessibilityFocus(_)
                | UiEvent::AccessibilityActivate(_)
                | UiEvent::AccessibilityContextMenu(_)
        );
        if !relevant {
            return TouchIntentArbitration::Preserve;
        }
        let mut preview = self.state.clone();
        let Ok(outcome) = self.tree.transition(
            &mut preview,
            event.input_source(),
            InteractionIntent::Event(event.clone()),
        ) else {
            return TouchIntentArbitration::Preserve;
        };
        let context_target = match event {
            UiEvent::ControllerContextMenu => self
                .tree
                .effective_context_target(&self.state, InputSource::Controller),
            UiEvent::KeyboardContextMenu => self
                .tree
                .effective_context_target(&self.state, InputSource::Keyboard),
            UiEvent::AccessibilityContextMenu(target) => Some(target.clone()),
            _ => None,
        };
        if (outcome.invalidation != Invalidation::None || !outcome.messages.is_empty())
            && let Some(target) = context_target.as_ref()
        {
            return self.arbitrate_activation_target(Some(target));
        }
        let after = preview.current_target();
        if after.is_some_and(|target| target != owner) {
            TouchIntentArbitration::CancelThenDispatch
        } else if after == Some(owner)
            && (!outcome.messages.is_empty()
                || matches!(
                    event,
                    UiEvent::KeyboardNavigateActivate
                        | UiEvent::ControllerAdjust(_)
                        | UiEvent::ControllerActivate
                        | UiEvent::ControllerContextMenu
                        | UiEvent::AccessibilityActivate(_)
                ))
        {
            TouchIntentArbitration::RejectBusy
        } else {
            TouchIntentArbitration::Preserve
        }
    }

    fn finalize_pending_long_press_attachment(&mut self) {
        let Some(pending) = self.pending_long_press.as_mut() else {
            return;
        };
        if pending.frame_generation.is_some() {
            return;
        }
        if self.tree.id_at(pending.origin) == pending.target.as_ref() {
            pending.frame_generation = Some(self.frame_generation);
        } else {
            self.pending_long_press = None;
        }
    }

    pub fn step(&mut self, batch: HostBatch) -> HostEventOutcome {
        self.state.clipboard_text_limit = batch.clipboard_text_limit;
        let controller_authority = batch.controller_authority;
        if let Some(authority) = controller_authority
            && self.controller_press_authority != Some(authority)
        {
            self.admitted_controller_presses.clear();
            self.controller_press_authority = Some(authority);
        }
        if batch.window_focused == Some(false) {
            self.admitted_controller_presses.clear();
            self.controller_press_authority = None;
            self.pending_long_press = None;
        }
        #[cfg(test)]
        let mut normalized_authorities = batch.normalized_authorities;
        #[cfg(not(test))]
        let normalized_authorities = batch.normalized_authorities;
        #[cfg(test)]
        normalized_authorities.extend(batch.events.iter().filter_map(|event| {
            match event {
                HostEvent::NormalizedIngress(envelope)
                    if envelope
                        .source
                        .identity_capability
                        .starts_with("synthetic-") =>
                {
                    Some(envelope.execution_authority())
                }
                _ => None,
            }
        }));
        self.state.clipboard_rejected = false;
        let step_started = Instant::now();
        let now = batch.now.unwrap_or_else(Instant::now);
        let mut combined = HostEventOutcome {
            failures: batch.failures,
            ..HostEventOutcome::default()
        };
        combined.telemetry.events_processed = batch.events.len();
        combined.telemetry.completions_processed = batch.completions.len();
        if batch.application_changed {
            combined.changed = true;
            combined.invalidation = Invalidation::Layout;
        }
        combined.telemetry.scheduled_wakeups = batch
            .events
            .iter()
            .filter(|event| matches!(event, HostEvent::Poll))
            .count();
        if let Some((width, height)) = batch.surface_size {
            let next = Rect::new(0.0, 0.0, width as f32, height as f32);
            if next != self.bounds {
                self.bounds = next;
                combined.changed = true;
                combined.invalidation = Invalidation::Layout;
            }
        }
        if let Some(scale_factor) = batch.scale_factor
            && scale_factor.is_finite()
            && scale_factor > 0.0
            && scale_factor != self.scale_factor
        {
            self.scale_factor = scale_factor;
            combined.changed = true;
            combined.invalidation = combined.invalidation.merge(Invalidation::Layout);
            if self.application.scale_factor_changed(scale_factor) {
                combined.changed = true;
                combined.invalidation = combined.invalidation.merge(Invalidation::Paint);
            }
        }
        if let Some(focused) = batch.window_focused {
            let adapted = A::adapt_input(
                self,
                &if focused {
                    nickel_input::InputEvent::FocusGained {
                        order: nickel_input::EventOrder(0),
                    }
                } else {
                    nickel_input::InputEvent::FocusLost {
                        order: nickel_input::EventOrder(0),
                    }
                },
            );
            if adapted.changed {
                combined.changed = true;
                combined.invalidation = combined.invalidation.merge(Invalidation::Layout);
            }
            let focus = self.dispatch_ui_event(if focused {
                UiEvent::FocusGained
            } else {
                UiEvent::FocusLost
            });
            combined.changed |= focus.changed;
            combined.invalidation = combined.invalidation.merge(focus.invalidation);
        }
        for completion in batch.completions {
            match self.application.complete(completion) {
                Ok(changed) => {
                    combined.changed |= changed;
                    if changed {
                        combined.invalidation = combined.invalidation.merge(Invalidation::Layout);
                    }
                }
                Err(failure) => combined.completion_failures.push(failure),
            }
        }
        for event in batch.events {
            let controller_fenced = if let HostEvent::AdmittedController { binding, .. } = &event {
                let admitted =
                    controller_authority.is_some_and(|authority| authority.admits(*binding));
                if let Some(blocked) = self.controller_overflow_fence
                    && admitted
                    && controller_authority.is_some_and(|authority| authority != blocked)
                {
                    self.controller_overflow_fence = None;
                    self.controller_press_authority = controller_authority;
                    self.admitted_controller_presses.clear();
                }
                self.controller_overflow_fence.is_some()
            } else {
                false
            };
            let admitted_controller_press = match &event {
                HostEvent::AdmittedController {
                    action: Some(action),
                    binding,
                } if !controller_fenced
                    && binding.edge == nickel_input::KeyEdge::Pressed
                    && controller_authority.is_some_and(|authority| authority.admits(*binding)) =>
                {
                    controller_ui_event(*action).map(|event| (*binding, event))
                }
                _ => None,
            };
            let touch_arbitration = if controller_fenced {
                TouchIntentArbitration::Preserve
            } else {
                admitted_controller_press.as_ref().map_or_else(
                    || self.touch_arbitration_for_event(&event),
                    |(_, event)| self.touch_arbitration_for_ui_event(event),
                )
            };
            let normalized_input = match &event {
                HostEvent::Normalized { input, .. } => Some(input),
                HostEvent::NormalizedIngress(envelope) => Some(&envelope.input),
                _ => None,
            };
            if let Some(input) = normalized_input {
                match input {
                    nickel_input::InputEvent::FocusLost { .. } => {
                        self.admitted_controller_presses.clear();
                        self.controller_press_authority = None;
                    }
                    nickel_input::InputEvent::DeviceRemoved { device, .. } => {
                        self.admitted_controller_presses
                            .retain(|press| press.device_generation != device.0);
                    }
                    _ => {}
                }
            }
            let mut cancellation =
                if touch_arbitration == TouchIntentArbitration::CancelThenDispatch {
                    self.arbitrate_touch_ownership()
                } else {
                    HostEventOutcome::default()
                };
            let mut outcome = if controller_fenced {
                let HostEvent::AdmittedController { binding, .. } = &event else {
                    unreachable!();
                };
                let mut outcome = HostEventOutcome::default();
                outcome
                    .controller_executions
                    .push(ControllerExecutionEvidence {
                        binding: *binding,
                        disposition: ControllerExecutionDisposition::RejectedResetFence,
                        message_count: 0,
                        effect_count: 0,
                    });
                outcome
            } else if touch_arbitration == TouchIntentArbitration::RejectBusy {
                let mut outcome = HostEventOutcome {
                    disposition: crate::EventDisposition::Rejected("input busy"),
                    ..HostEventOutcome::default()
                };
                if let Some((binding, _)) = admitted_controller_press {
                    outcome
                        .controller_executions
                        .push(ControllerExecutionEvidence {
                            binding,
                            disposition: ControllerExecutionDisposition::RejectedBusy,
                            message_count: 0,
                            effect_count: 0,
                        });
                }
                outcome
            } else {
                match event {
                    HostEvent::Ui(event) => self.dispatch_ui_event(event),
                    HostEvent::Controller(action) => self.dispatch_controller_action(action),
                    HostEvent::AdmittedController { action, binding } => {
                        let admitted =
                            controller_authority.is_some_and(|authority| authority.admits(binding));
                        if self.controller_overflow_fence.is_some() {
                            let mut outcome = HostEventOutcome::default();
                            outcome
                                .controller_executions
                                .push(ControllerExecutionEvidence {
                                    binding,
                                    disposition: ControllerExecutionDisposition::RejectedResetFence,
                                    message_count: 0,
                                    effect_count: 0,
                                });
                            outcome
                        } else {
                            let mut overflow_reset = false;
                            let paired = match (binding.edge, action) {
                                (nickel_input::KeyEdge::Released, Some(action)) => {
                                    let key = AdmittedControllerPress::new(binding, action);
                                    let paired =
                                        admitted && self.admitted_controller_presses.remove(&key);
                                    if !paired {
                                        self.admitted_controller_presses.retain(|press| {
                                            press.device_generation != binding.device_generation
                                                || press.action != action
                                        });
                                    }
                                    paired
                                }
                                (nickel_input::KeyEdge::Released, None) => {
                                    self.admitted_controller_presses.retain(|press| {
                                        press.device_generation != binding.device_generation
                                    });
                                    false
                                }
                                (nickel_input::KeyEdge::Pressed, Some(action)) if admitted => {
                                    let key = AdmittedControllerPress::new(binding, action);
                                    if !self.admitted_controller_presses.contains(&key)
                                        && self.admitted_controller_presses.len()
                                            >= MAX_ADMITTED_CONTROLLER_PRESSES
                                    {
                                        self.admitted_controller_presses.clear();
                                        self.controller_press_authority = None;
                                        self.controller_overflow_fence = controller_authority;
                                        overflow_reset = true;
                                        false
                                    } else {
                                        self.admitted_controller_presses.insert(key);
                                        true
                                    }
                                }
                                (nickel_input::KeyEdge::Pressed, _) => admitted,
                            };
                            let mut outcome =
                                if paired && binding.edge == nickel_input::KeyEdge::Pressed {
                                    if touch_arbitration == TouchIntentArbitration::CancelGesture {
                                        self.arbitrate_touch_ownership()
                                    } else {
                                        self.dispatch_controller_action(
                                            action.expect("paired press has action"),
                                        )
                                    }
                                } else {
                                    HostEventOutcome::default()
                                };
                            outcome
                                .controller_executions
                                .push(ControllerExecutionEvidence {
                                    binding,
                                    disposition: if overflow_reset {
                                        ControllerExecutionDisposition::ResetOverflow
                                    } else if !admitted {
                                        ControllerExecutionDisposition::RejectedStale
                                    } else if binding.edge == nickel_input::KeyEdge::Released
                                        && !paired
                                    {
                                        ControllerExecutionDisposition::RejectedUnpairedRelease
                                    } else {
                                        ControllerExecutionDisposition::Executed
                                    },
                                    message_count: outcome.messages.len(),
                                    effect_count: 0,
                                });
                            outcome
                        }
                    }
                    HostEvent::Shortcut(shortcut) => {
                        let shortcut = self.application.shortcut_outcome(shortcut);
                        HostEventOutcome {
                            changed: shortcut.changed,
                            disposition: shortcut.disposition,
                            invalidation: if shortcut.changed {
                                Invalidation::Layout
                            } else {
                                Invalidation::None
                            },
                            ..HostEventOutcome::default()
                        }
                    }
                    HostEvent::Semantic { target, action } => {
                        self.dispatch_semantic_action(target, action, InputSource::Programmatic)
                    }
                    HostEvent::Accessibility { target, action } => {
                        self.dispatch_semantic_action(target, action, InputSource::Accessibility)
                    }
                    HostEvent::ControllerSemantic { target, action } => {
                        self.dispatch_semantic_action(target, action, InputSource::Controller)
                    }
                    HostEvent::Normalized {
                        input,
                        clipboard_text,
                    } => self.dispatch_input(&input, clipboard_text.as_deref()),
                    HostEvent::NormalizedIngress(envelope) => {
                        if self.admits_normalized_ingress(&envelope, &normalized_authorities) {
                            self.update_admitted_long_press(&envelope, now);
                            self.dispatch_input(&envelope.input, envelope.clipboard_text.as_deref())
                        } else {
                            HostEventOutcome::default()
                        }
                    }
                    HostEvent::Poll => {
                        let changed = self.application.poll();
                        self.next_application_deadline = self
                            .application
                            .poll_interval()
                            .map(|interval| now + interval);
                        HostEventOutcome {
                            changed,
                            invalidation: if changed {
                                Invalidation::Layout
                            } else {
                                Invalidation::None
                            },
                            ..HostEventOutcome::default()
                        }
                    }
                }
            };
            // Reject a read-only Copy before merging its effect. An oversized
            // later copy cannot erase bytes from an earlier successful Cut.
            if self.state.clipboard_text_limit.is_some_and(|limit| {
                outcome
                    .clipboard_text
                    .as_ref()
                    .is_some_and(|text| text.len() > limit)
            }) {
                outcome.clipboard_text = None;
                self.state.clipboard_rejected = true;
            }
            cancellation.merge(outcome);
            combined.merge(cancellation);
        }
        combined.telemetry.input_to_message_us = elapsed_us(step_started);
        if combined.changed {
            let (paint_list_us, layout_us) = self.rebuild_timed();
            combined.telemetry.paint_list_us = paint_list_us;
            combined.telemetry.layout_us = layout_us;
            combined.telemetry.rebuilt = true;
        }
        if let Some(requested) = self.application.take_focus_request()
            && let Some(target) = self.tree.resolve_stable_target(&requested)
        {
            let focus = self
                .tree
                .transition(
                    &mut self.state,
                    InputSource::System,
                    InteractionIntent::Event(UiEvent::AccessibilityFocus(target)),
                )
                .expect("host focus requests are ordinary frame events");
            if focus.invalidation != Invalidation::None || !focus.messages.is_empty() {
                combined.changed = true;
                combined.invalidation = combined.invalidation.merge(focus.invalidation);
                for message in focus.messages {
                    self.application.update(message);
                }
                let (paint_list_us, layout_us) = self.rebuild_timed();
                combined.telemetry.paint_list_us = combined
                    .telemetry
                    .paint_list_us
                    .saturating_add(paint_list_us);
                combined.telemetry.layout_us =
                    combined.telemetry.layout_us.saturating_add(layout_us);
                combined.telemetry.rebuilt = true;
            }
        }
        self.finalize_pending_long_press_attachment();
        let due_long_press = self.pending_long_press.as_ref().is_some_and(|pending| {
            now >= pending.deadline
                && pending.frame_generation == Some(self.frame_generation)
                && self.tree.id_at(pending.origin) == pending.target.as_ref()
        });
        if due_long_press {
            let pending = self.pending_long_press.take().expect("due long press");
            let mut long_press = self.dispatch_ui_event(UiEvent::PointerCancelled);
            long_press.merge(self.dispatch_ui_event(UiEvent::TouchLongPress(pending.origin)));
            let rebuild = long_press.changed;
            combined.merge(long_press);
            if rebuild {
                let (paint_list_us, layout_us) = self.rebuild_timed();
                combined.telemetry.paint_list_us = combined
                    .telemetry
                    .paint_list_us
                    .saturating_add(paint_list_us);
                combined.telemetry.layout_us =
                    combined.telemetry.layout_us.saturating_add(layout_us);
                combined.telemetry.rebuilt = true;
            }
        }
        combined.effects = self.application.take_effect_evidence();
        if let Some(execution) = combined
            .controller_executions
            .iter_mut()
            .rev()
            .find(|execution| execution.disposition == ControllerExecutionDisposition::Executed)
        {
            execution.effect_count = combined.effects.len();
        }
        if self.state.clipboard_rejected {
            combined.failures.push(HostFailure {
                surface: self.application.title().into(),
                stage: HostFailureStage::Clipboard,
                optional: false,
                detail:
                    "Clipboard operation exceeds the native transfer limit; that operation was rejected"
                        .into(),
            });
        }
        self.next_application_deadline = match (
            self.next_application_deadline,
            self.application.poll_interval(),
        ) {
            (_, None) => None,
            (Some(deadline), Some(_)) => Some(deadline),
            (None, Some(interval)) => Some(now + interval),
        };
        combined.pointer_icon = self.pointer_icon;
        combined.text_input_active = self.input_context().text_focused;
        combined.accessibility_generation = self.frame_generation;
        combined.change_token = HostChangeToken {
            frame_generation: self.frame_generation,
            semantic_generation: self.frame_generation,
        };
        combined.next_deadline = [
            self.next_application_deadline,
            self.pending_long_press
                .as_ref()
                .map(|pending| pending.deadline),
        ]
        .into_iter()
        .flatten()
        .min();
        combined.telemetry.retained_frame_bytes =
            self.tree.resource_diagnostics().estimated_retained_bytes;
        combined.telemetry.input_to_frame_us = elapsed_us(step_started);
        combined
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        [
            self.next_application_deadline,
            self.pending_long_press
                .as_ref()
                .map(|pending| pending.deadline),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.step(HostBatch {
            surface_size: Some((width, height)),
            ..HostBatch::default()
        });
    }

    pub fn poll(&mut self) -> bool {
        self.step(HostBatch {
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        })
        .changed
    }

    pub fn handle_event(&mut self, event: UiEvent) -> HostEventOutcome {
        self.step(HostBatch {
            events: vec![HostEvent::Ui(event)],
            ..HostBatch::default()
        })
    }

    fn dispatch_ui_event(&mut self, event: UiEvent) -> HostEventOutcome {
        match self.touch_arbitration_for_ui_event(&event) {
            TouchIntentArbitration::RejectBusy => {
                return HostEventOutcome {
                    disposition: crate::EventDisposition::Rejected("input busy"),
                    ..HostEventOutcome::default()
                };
            }
            TouchIntentArbitration::CancelThenDispatch => {
                let mut outcome = self.arbitrate_touch_ownership();
                outcome.merge(self.dispatch_ui_event_unarbitrated(event));
                return outcome;
            }
            TouchIntentArbitration::CancelGesture => return self.arbitrate_touch_ownership(),
            TouchIntentArbitration::Preserve => {}
        }
        self.dispatch_ui_event_unarbitrated(event)
    }

    fn dispatch_ui_event_unarbitrated(&mut self, event: UiEvent) -> HostEventOutcome {
        if let UiEvent::PointerMoved(point)
        | UiEvent::PointerPressed(point)
        | UiEvent::PointerReleased(point) = &event
        {
            self.pointer_icon = self.tree.pointer_icon_at(*point);
        }
        let source = event.input_source();
        let outcome = self
            .tree
            .transition(&mut self.state, source, InteractionIntent::Event(event))
            .expect("ordinary UI events cannot fail semantic resolution");
        let invalidation = outcome.invalidation;
        let disposition = outcome.disposition;
        let changed = invalidation != Invalidation::None || !outcome.messages.is_empty();
        let messages = outcome
            .messages
            .iter()
            .map(|message| self.application.message_evidence(message))
            .collect();
        for message in outcome.messages {
            self.application.update(message);
        }
        let clipboard_text = self
            .application
            .take_clipboard_write()
            .or(outcome.clipboard_text);
        HostEventOutcome {
            changed,
            disposition,
            invalidation,
            messages,
            clipboard_text,
            semantic_failures: Vec::new(),
            global_actions: Vec::new(),
            completion_failures: Vec::new(),
            ..HostEventOutcome::default()
        }
    }

    fn dispatch_semantic_action(
        &mut self,
        target: UiId,
        action: SemanticAction,
        source: InputSource,
    ) -> HostEventOutcome {
        match self.tree.transition(
            &mut self.state,
            source,
            InteractionIntent::Invoke {
                target: target.clone(),
                action,
            },
        ) {
            Ok(outcome) => {
                let invalidation = outcome.invalidation;
                let disposition = outcome.disposition;
                let changed = invalidation != Invalidation::None || !outcome.messages.is_empty();
                let messages = outcome
                    .messages
                    .iter()
                    .map(|message| self.application.message_evidence(message))
                    .collect();
                for message in outcome.messages {
                    self.application.update(message);
                }
                HostEventOutcome {
                    changed,
                    disposition,
                    invalidation,
                    messages,
                    clipboard_text: self
                        .application
                        .take_clipboard_write()
                        .or(outcome.clipboard_text),
                    semantic_failures: Vec::new(),
                    global_actions: Vec::new(),
                    completion_failures: Vec::new(),
                    ..HostEventOutcome::default()
                }
            }
            Err(error) => HostEventOutcome {
                disposition: crate::EventDisposition::Rejected(match error {
                    SemanticActionError::MissingTarget => "missing target",
                    SemanticActionError::AmbiguousTarget => "ambiguous target",
                    SemanticActionError::ActionUnavailable => "action unavailable",
                }),
                semantic_failures: vec![SemanticActionFailure { target, error }],
                ..HostEventOutcome::default()
            },
        }
    }

    pub fn handle_controller_action(&mut self, action: ControllerAction) -> HostEventOutcome {
        self.step(HostBatch {
            events: vec![HostEvent::Controller(action)],
            ..HostBatch::default()
        })
    }

    fn dispatch_controller_action(&mut self, action: ControllerAction) -> HostEventOutcome {
        if !self.state.window_focused() {
            return HostEventOutcome::default();
        }
        if action == ControllerAction::Launcher {
            return HostEventOutcome {
                global_actions: vec![GlobalAction::ToggleLauncher],
                ..HostEventOutcome::default()
            };
        }
        controller_ui_event(action)
            .map(|event| self.dispatch_ui_event(event))
            .unwrap_or_default()
    }

    pub fn shortcut(&mut self, shortcut: Shortcut) -> bool {
        self.step(HostBatch {
            events: vec![HostEvent::Shortcut(shortcut)],
            ..HostBatch::default()
        })
        .changed
    }

    /// Dispatch a normalized event through the same focused-input contract used by standalone
    /// Nickel UI applications. Embedded hosts provide clipboard text only when paste is allowed;
    /// copy and cut return replacement clipboard text in the outcome.
    pub fn handle_input(
        &mut self,
        input: &nickel_input::InputEvent,
        clipboard_text: Option<&str>,
    ) -> HostEventOutcome {
        self.step(HostBatch {
            events: vec![HostEvent::Normalized {
                input: input.clone(),
                clipboard_text: clipboard_text.map(ToOwned::to_owned),
            }],
            ..HostBatch::default()
        })
    }

    fn dispatch_input(
        &mut self,
        input: &nickel_input::InputEvent,
        clipboard_text: Option<&str>,
    ) -> HostEventOutcome {
        let adapted = A::adapt_input(self, input);
        let mut combined = HostEventOutcome::default();
        if adapted.changed {
            combined.changed = true;
            combined.invalidation = Invalidation::Layout;
        }
        combined.disposition = combined.disposition.merge(adapted.disposition);
        if adapted.disposition != crate::EventDisposition::Unhandled {
            return combined;
        }
        self.state.set_clipboard_offer(clipboard_text);
        let context = self.input_context();
        let commands = self.input_dispatcher.dispatch_with_context(input, context);
        for command in commands {
            let event = match command {
                InputCommand::Ui(event) => Some(event),
                InputCommand::Application { shortcut, fallback } => {
                    let shortcut = self.application.shortcut_outcome(shortcut);
                    combined.disposition = combined.disposition.merge(shortcut.disposition);
                    if shortcut.changed {
                        combined.changed = true;
                        combined.invalidation = combined.invalidation.merge(Invalidation::Layout);
                    }
                    match shortcut.disposition {
                        crate::EventDisposition::Unhandled => fallback,
                        crate::EventDisposition::Handled | crate::EventDisposition::Rejected(_) => {
                            None
                        }
                    }
                }
                InputCommand::Copy => Some(UiEvent::TextCopy),
                InputCommand::Cut => Some(UiEvent::TextCut),
                InputCommand::Paste => clipboard_text.map(|text| UiEvent::TextPaste(text.into())),
            };
            let Some(event) = event else {
                continue;
            };
            let outcome = self.dispatch_ui_event(event);
            combined.merge(outcome);
        }
        combined
    }

    fn admits_normalized_ingress(
        &mut self,
        envelope: &NormalizedInputEnvelope,
        authorities: &[NormalizedIngressAuthority],
    ) -> bool {
        let authorized = authorities.iter().any(|authority| {
            authority.source == envelope.source
                && authority.recipient == envelope.recipient
                && authority.transfer_cutoff == envelope.transfer_cutoff
                && authority.host_connection_generation == envelope.host_connection_generation
                && authority.operation_epoch == envelope.operation_epoch
                && authority.transform_generation == envelope.transform_generation
                && authority.text_transaction == envelope.text_transaction
                && authority.composition_recipient_epoch == envelope.composition_recipient_epoch
                && authority.role == envelope.role
                && authority.coordinate_meaning == envelope.coordinate_meaning
        });
        if !authorized {
            return false;
        }
        if envelope.recipient.lease == 0
            || envelope
                .transfer_cutoff
                .is_some_and(|cutoff| envelope.broker_event_id.is_none_or(|event| event > cutoff))
        {
            return false;
        }
        let key = (envelope.source.seat, envelope.source.backend_stream.clone());
        let generation = (
            envelope.source.stream_generation,
            envelope.source.reconnect_generation,
        );
        match self.normalized_source_orders.get_mut(&key) {
            Some((stream, reconnect, last_order)) if (*stream, *reconnect) == generation => {
                if envelope.admission.order <= *last_order {
                    return false;
                }
                *last_order = envelope.admission.order;
            }
            slot => {
                let value = (generation.0, generation.1, envelope.admission.order);
                if let Some(slot) = slot {
                    *slot = value;
                } else {
                    self.normalized_source_orders.insert(key, value);
                }
            }
        }
        true
    }

    fn rebuild(&mut self) {
        let _ = self.rebuild_timed();
    }

    fn rebuild_timed(&mut self) -> (u64, u64) {
        let pending_long_press_was_bound = self
            .pending_long_press
            .as_ref()
            .is_some_and(|pending| pending.frame_generation.is_some());
        let context = ViewContext::from_host(self.bounds, &self.state, Some(&self.tree));
        let overlay_interaction = OverlayInteractionSnapshot::capture(&self.state, &self.tree);
        let paint_started = Instant::now();
        let view = self.application.view(context.clone());
        let overlays = self.application.frame_overlays(context);
        let paint_list_us = elapsed_us(paint_started);
        let layout_started = Instant::now();
        self.tree = UiFrame::resolve(view, FrameRequest::new(self.bounds, &mut self.state));
        self.tree_remote_access_protected = self.application.remote_access_protected();
        // Base resolution cannot retain transient descendants because their
        // topology is declared next. Restore interaction ownership before
        // overlay emission so paint and semantics observe the same state.
        overlay_interaction.restore_before_overlay(&mut self.state);
        self.overlay_failures = apply_frame_overlays(&mut self.tree, &mut self.state, overlays);
        overlay_interaction.restore(&mut self.state, &self.tree);
        self.tree.reconcile_transient_focus(&mut self.state);
        self.tree.finalize_transient_layers(&self.state);
        self.frame_generation = self.frame_generation.wrapping_add(1);
        if pending_long_press_was_bound {
            self.pending_long_press = None;
        } else {
            self.finalize_pending_long_press_attachment();
        }
        (paint_list_us, elapsed_us(layout_started))
    }

    pub fn shutdown(&mut self) {
        self.state.destroy();
    }
}

fn elapsed_us(started: Instant) -> u64 {
    started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

fn apply_frame_overlays<Message: Clone>(
    frame: &mut UiFrame<Message>,
    state: &mut UiStateStore,
    overlays: Vec<FrameOverlay<Message>>,
) -> Vec<OverlayDeclarationFailure> {
    let mut failures = Vec::new();
    for overlay in overlays {
        match overlay {
            FrameOverlay::Menu(menu) => {
                let id = menu.id.clone();
                let anchor = menu.anchor.id().clone();
                if let Err(error) = frame.present_menu(state, menu) {
                    failures.push(OverlayDeclarationFailure {
                        overlay: id,
                        anchor,
                        error,
                    });
                }
            }
            FrameOverlay::Surface(surface) => {
                let id = surface.id.clone();
                let anchor = surface.anchor.id().clone();
                if let Err(error) = frame.present_transient_surface(state, surface) {
                    failures.push(OverlayDeclarationFailure {
                        overlay: id,
                        anchor,
                        error,
                    });
                }
            }
            FrameOverlay::ContentSurface { surface, content } => {
                let id = surface.id.clone();
                let anchor = surface.anchor.id().clone();
                if let Err(error) = frame.present_transient_content(state, surface, *content) {
                    failures.push(OverlayDeclarationFailure {
                        overlay: id,
                        anchor,
                        error,
                    });
                }
            }
            FrameOverlay::SelectionMarquee {
                rect,
                fill,
                stroke,
                width,
            } => {
                frame.selection_marquee_layer(rect, fill, stroke, width);
            }
        }
    }
    frame.finalize_transient_layers(state);
    failures
}

pub fn run<A: Application>(application: A) -> Result<(), Box<dyn Error>> {
    run_with_adapter(application, DefaultHostAdapter)
}

pub fn run_with_adapter<A: Application>(
    application: A,
    adapter: impl HostAdapter<A>,
) -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::new()?;
    let display = event_loop.owned_display_handle();
    let mut runtime = ApplicationRuntime::new(application, adapter, display);
    event_loop.run_app(&mut runtime)?;
    if let Some(error) = runtime.error {
        Err(error)
    } else {
        Ok(())
    }
}

struct ApplicationRuntime<A: Application, H: HostAdapter<A>> {
    host: Option<UiHost<A>>,
    application: Option<A>,
    adapter: H,
    display: OwnedDisplayHandle,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<OwnedDisplayHandle, Arc<Window>>>,
    renderer: Option<SoftwareRenderer>,
    input: nickel_input::winit::Adapter,
    clipboard: Option<arboard::Clipboard>,
    controller: Option<ControllerInput>,
    controller_discovery: ControllerDiscoveryMode,
    local_controller_lease: Option<ControllerRoleLease>,
    #[cfg(any(unix, windows))]
    session_controller: SessionControllerSource,
    controller_schedule: ControllerPollSchedule,
    next_caret_blink: Instant,
    next_adapter_poll: Option<Instant>,
    scheduler: PresentScheduler,
    pointer_icon: PointerIcon,
    scale: f32,
    transform_generation: u64,
    stopped: bool,
    error: Option<Box<dyn Error>>,
    pending_continuous_input: Vec<AdmittedNormalizedInput>,
    normalized_admission_order: u64,
    normalized_ingress_epoch: Instant,
    native_host_generation: u64,
    standalone_recipient: NormalizedRecipientBinding,
    native_pointer_recipient: NormalizedRecipientBinding,
    native_pointer_stream_generation: u64,
    native_touch_recipient: NormalizedRecipientBinding,
    native_touch_stream_generation: u64,
    native_stream_reset_pending: bool,
}

impl<A: Application, H: HostAdapter<A>> ApplicationRuntime<A, H> {
    fn admit_continuous_input(
        &mut self,
        input: nickel_input::InputEvent,
    ) -> AdmittedNormalizedInput {
        self.normalized_admission_order = self.normalized_admission_order.wrapping_add(1).max(1);
        let device_generation = input.device().map_or(0, |device| device.0);
        let recipient = self.native_pointer_recipient;
        let role = "native-pointer-seat-0";
        let source =
            native_pointer_source_binding(self.native_pointer_stream_generation, device_generation);
        let authority = NormalizedIngressAuthority {
            source: source.clone(),
            recipient,
            transfer_cutoff: None,
            host_connection_generation: recipient.lifetime,
            operation_epoch: None,
            transform_generation: Some(self.transform_generation),
            text_transaction: None,
            composition_recipient_epoch: Some(recipient.lifetime),
            role: role.into(),
            coordinate_meaning: "window-logical".into(),
        };
        let envelope = NormalizedInputEnvelope {
            input,
            clipboard_text: None,
            source,
            admission: NormalizedAdmissionBinding {
                order: self.normalized_admission_order,
                monotonic_micros: self.normalized_ingress_epoch.elapsed().as_micros() as u64,
            },
            recipient,
            operation: None,
            transform_generation: Some(self.transform_generation),
            text_transaction: None,
            transfer_cutoff: None,
            broker_event_id: None,
            host_connection_generation: recipient.lifetime,
            operation_epoch: None,
            role: role.into(),
            coordinate_meaning: "window-logical".into(),
            composition_recipient_epoch: Some(recipient.lifetime),
        };
        AdmittedNormalizedInput {
            envelope,
            authority,
        }
    }

    fn revoke_native_ingress_after_reset(&mut self) {
        let reset_generation = NEXT_NATIVE_HOST_GENERATION
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .max(1);
        revoke_native_ingress(
            &mut self.native_pointer_recipient,
            &mut self.native_pointer_stream_generation,
            reset_generation,
        );
        self.native_stream_reset_pending = true;
    }

    fn new(application: A, adapter: H, display: OwnedDisplayHandle) -> Self {
        let now = Instant::now();
        let native_host_generation = NEXT_NATIVE_HOST_GENERATION
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .max(1);
        let next_adapter_poll = adapter.poll_interval().map(|interval| now + interval);
        #[cfg(any(unix, windows))]
        let (controller_discovery, controller, local_controller_lease, session_controller) =
            SessionControllerSource::discover();
        #[cfg(not(any(unix, windows)))]
        let controller = Some(ControllerInput::new());
        #[cfg(not(any(unix, windows)))]
        let controller_discovery = ControllerDiscoveryMode::Standalone;
        #[cfg(not(any(unix, windows)))]
        let local_controller_lease = Some(ControllerRoleLease::local());
        Self {
            host: None,
            application: Some(application),
            adapter,
            display,
            window: None,
            surface: None,
            renderer: None,
            input: nickel_input::winit::Adapter::default(),
            clipboard: arboard::Clipboard::new().ok(),
            controller,
            controller_discovery,
            local_controller_lease,
            #[cfg(any(unix, windows))]
            session_controller,
            controller_schedule: ControllerPollSchedule::new(now),
            next_caret_blink: now + Duration::from_millis(500),
            next_adapter_poll,
            scheduler: PresentScheduler::default(),
            pointer_icon: PointerIcon::Default,
            scale: 1.0,
            transform_generation: 1,
            stopped: false,
            error: None,
            pending_continuous_input: Vec::new(),
            normalized_admission_order: 0,
            normalized_ingress_epoch: now,
            native_host_generation,
            standalone_recipient: NormalizedRecipientBinding {
                lease: 0,
                lifetime: native_host_generation,
            },
            native_pointer_recipient: NormalizedRecipientBinding {
                lease: 0,
                lifetime: native_host_generation,
            },
            native_pointer_stream_generation: native_host_generation,
            native_touch_recipient: NormalizedRecipientBinding {
                lease: 0,
                lifetime: native_host_generation,
            },
            native_touch_stream_generation: native_host_generation,
            native_stream_reset_pending: false,
        }
    }

    fn apply_input_outcome(&mut self, window: &Window, outcome: HostEventOutcome) {
        window.set_ime_allowed(outcome.text_input_active);
        if let Some(text) = outcome.clipboard_text
            && let Some(clipboard) = &mut self.clipboard
        {
            let _ = clipboard.set_text(text);
        }
        if let Some(host) = &self.host {
            let next_icon = host.inspect().pointer_icon;
            if next_icon != self.pointer_icon {
                window.set_cursor(match next_icon {
                    PointerIcon::Default => CursorIcon::Default,
                    PointerIcon::Hand => CursorIcon::Pointer,
                    PointerIcon::Text => CursorIcon::Text,
                });
                self.pointer_icon = next_icon;
            }
        }
        if outcome.changed {
            self.next_caret_blink = Instant::now() + Duration::from_millis(500);
            self.scheduler.invalidate();
        }
    }

    fn dispatch_normalized_input(
        &mut self,
        event_loop: &ActiveEventLoop,
        window: &Window,
        inputs: Vec<nickel_input::InputEvent>,
        clipboard_text: Option<String>,
    ) {
        let mut events = Vec::with_capacity(inputs.len());
        let mut normalized_authorities = Vec::with_capacity(inputs.len());
        let mut adapter_changed = false;
        let mut adapter_exit = false;
        for input in inputs {
            let adapted = self.host.as_mut().map(|host| {
                self.adapter
                    .normalized_input(host, &input, HostServices { window })
            });
            let consume = match adapted {
                Some(Ok(outcome)) => {
                    let consume = outcome.disposition != crate::EventDisposition::Unhandled;
                    adapter_changed |= outcome.changed;
                    adapter_exit |= outcome.exit;
                    consume
                }
                Some(Err(error)) => {
                    self.fail(event_loop, error);
                    return;
                }
                None => return,
            };
            if !consume {
                if matches!(input, nickel_input::InputEvent::FocusGained { .. }) {
                    let generation = NEXT_NATIVE_HOST_GENERATION
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                        .max(1);
                    grant_native_ingress(&mut self.standalone_recipient, generation);
                    grant_native_ingress(&mut self.native_pointer_recipient, generation);
                    grant_native_ingress(&mut self.native_touch_recipient, generation);
                }
                self.normalized_admission_order =
                    self.normalized_admission_order.wrapping_add(1).max(1);
                let device_generation = input.device().map_or(0, |device| device.0);
                let class = native_input_class(&input);
                let (recipient, stream_generation, recipient_role) = match class {
                    NativeInputClass::Pointer => (
                        self.native_pointer_recipient,
                        self.native_pointer_stream_generation,
                        "native-pointer-seat-0",
                    ),
                    NativeInputClass::Touch => (
                        self.native_touch_recipient,
                        self.native_touch_stream_generation,
                        "native-touch-seat-0",
                    ),
                    NativeInputClass::KeyboardText => (
                        self.standalone_recipient,
                        self.native_host_generation,
                        "native-keyboard-seat-0",
                    ),
                    NativeInputClass::Window => (
                        self.standalone_recipient,
                        self.native_host_generation,
                        "native-window-seat-0",
                    ),
                };
                let source =
                    native_input_source_binding(class, stream_generation, device_generation);
                let authority = NormalizedIngressAuthority {
                    source: source.clone(),
                    recipient,
                    transfer_cutoff: None,
                    host_connection_generation: recipient.lifetime,
                    operation_epoch: None,
                    transform_generation: Some(self.transform_generation),
                    text_transaction: None,
                    composition_recipient_epoch: Some(recipient.lifetime),
                    role: recipient_role.into(),
                    coordinate_meaning: "window-logical".into(),
                };
                normalized_authorities.push(authority.clone());
                events.push(HostEvent::NormalizedIngress(NormalizedInputEnvelope {
                    input,
                    clipboard_text: clipboard_text.clone(),
                    source,
                    admission: NormalizedAdmissionBinding {
                        order: self.normalized_admission_order,
                        monotonic_micros: Instant::now()
                            .saturating_duration_since(self.normalized_ingress_epoch)
                            .as_micros() as u64,
                    },
                    recipient,
                    operation: None,
                    transform_generation: Some(self.transform_generation),
                    text_transaction: None,
                    transfer_cutoff: None,
                    broker_event_id: None,
                    host_connection_generation: authority.host_connection_generation,
                    operation_epoch: None,
                    role: authority.role,
                    coordinate_meaning: "window-logical".into(),
                    composition_recipient_epoch: authority.composition_recipient_epoch,
                }));
                if matches!(events.last(), Some(HostEvent::NormalizedIngress(envelope)) if matches!(envelope.input, nickel_input::InputEvent::FocusLost { .. }))
                {
                    self.standalone_recipient.lease = 0;
                    self.native_pointer_recipient.lease = 0;
                    self.native_touch_recipient.lease = 0;
                }
            }
        }
        if adapter_exit {
            event_loop.exit();
        }
        if events.is_empty() && !adapter_changed {
            return;
        }
        let Some(host) = &mut self.host else { return };
        let outcome = host.step(HostBatch {
            events,
            normalized_authorities,
            application_changed: adapter_changed,
            ..HostBatch::default()
        });
        self.apply_input_outcome(window, outcome);
        self.start_pending_file_drag(window);
    }

    fn start_pending_file_drag(&mut self, window: &Window) {
        #[cfg(target_os = "windows")]
        let _ = window;
        let Some(drag) = self
            .host
            .as_mut()
            .and_then(|host| host.application_mut().take_outbound_file_drag())
        else {
            return;
        };
        #[cfg(target_os = "linux")]
        {
            use winit::platform::wayland::WindowExtWayland;
            let payload = file_uri_list(&drag.paths);
            if let Err(error) = window.start_file_drag(payload) {
                tracing::warn!(%error, "could not begin native Wayland file drag");
            }
        }
        #[cfg(target_os = "windows")]
        if let Err(error) = start_windows_file_drag(&drag.paths) {
            tracing::warn!(%error, "could not begin native Windows file drag");
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        let _ = (drag, window);
    }

    fn flush_pending_continuous_input(&mut self, event_loop: &ActiveEventLoop, window: &Window) {
        if self.pending_continuous_input.is_empty() {
            return;
        }
        let samples = std::mem::take(&mut self.pending_continuous_input);
        let reset_pending = std::mem::take(&mut self.native_stream_reset_pending);
        let mut events = Vec::new();
        let mut authorities = Vec::new();
        let mut changed = false;
        let mut exit = false;
        let mut reset_reconciled = false;
        for sample in samples {
            if sample.envelope.recipient.lease == 0
                || !transform_is_current(&sample, self.transform_generation)
            {
                continue;
            }
            reset_reconciled |= matches!(
                sample.envelope.input,
                nickel_input::InputEvent::FocusLost { .. }
            );
            match self.adapter.normalized_input(
                self.host.as_mut().unwrap(),
                &sample.envelope.input,
                HostServices { window },
            ) {
                Ok(outcome) if outcome.disposition != crate::EventDisposition::Unhandled => {
                    changed |= outcome.changed;
                    exit |= outcome.exit;
                }
                Ok(outcome) => {
                    changed |= outcome.changed;
                    exit |= outcome.exit;
                    events.push(HostEvent::NormalizedIngress(sample.envelope));
                    authorities.push(sample.authority);
                }
                Err(error) => {
                    self.fail(event_loop, error);
                    return;
                }
            }
        }
        if exit {
            event_loop.exit();
        }
        if events.is_empty() && !changed {
            if reset_pending && reset_reconciled {
                self.recover_native_pointer_focus(event_loop, window);
            }
            return;
        }
        if let Some(host) = &mut self.host {
            let outcome = host.step(HostBatch {
                events,
                normalized_authorities: authorities,
                application_changed: changed,
                ..Default::default()
            });
            self.apply_input_outcome(window, outcome);
            self.start_pending_file_drag(window);
        }
        if reset_pending && reset_reconciled {
            self.recover_native_pointer_focus(event_loop, window);
        }
    }

    fn recover_native_pointer_focus(&mut self, event_loop: &ActiveEventLoop, window: &Window) {
        let generation = NEXT_NATIVE_HOST_GENERATION
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .max(1);
        if !grant_native_ingress_if_focused(
            &mut self.native_pointer_recipient,
            window.has_focus(),
            generation,
        ) {
            return;
        }
        let order = self.normalized_admission_order.wrapping_add(1).max(1);
        let sample = self.admit_continuous_input(nickel_input::InputEvent::FocusGained {
            order: nickel_input::EventOrder(order),
        });
        let adapted = self.host.as_mut().map(|host| {
            self.adapter
                .normalized_input(host, &sample.envelope.input, HostServices { window })
        });
        match adapted {
            Some(Ok(outcome)) if outcome.disposition != crate::EventDisposition::Unhandled => {
                self.apply_adapter_outcome(event_loop, outcome);
            }
            Some(Ok(outcome)) => {
                if outcome.exit {
                    event_loop.exit();
                }
                let Some(host) = &mut self.host else { return };
                let host_outcome = host.step(HostBatch {
                    events: vec![HostEvent::NormalizedIngress(sample.envelope)],
                    normalized_authorities: vec![sample.authority],
                    application_changed: outcome.changed,
                    ..Default::default()
                });
                self.apply_input_outcome(window, host_outcome);
            }
            Some(Err(error)) => self.fail(event_loop, error),
            None => {}
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl Into<Box<dyn Error>>) {
        self.error = Some(error.into());
        event_loop.exit();
    }

    fn apply_adapter_outcome(&mut self, event_loop: &ActiveEventLoop, outcome: AdapterOutcome) {
        if outcome.changed
            && let Some(host) = &mut self.host
        {
            host.rebuild();
            self.scheduler.invalidate();
        }
        if outcome.exit {
            event_loop.exit();
        }
    }

    fn present(&mut self) -> Result<(), Box<dyn Error>> {
        let (Some(host), Some(surface), Some(renderer), Some(window)) = (
            self.host.as_ref(),
            self.surface.as_mut(),
            self.renderer.as_mut(),
            self.window.as_ref(),
        ) else {
            return Ok(());
        };
        let size = window.inner_size();
        let width = NonZeroU32::new(size.width.max(1)).expect("clamped non-zero width");
        let height = NonZeroU32::new(size.height.max(1)).expect("clamped non-zero height");
        surface.resize(width, height)?;
        renderer.resize(width.get(), height.get(), self.scale);
        if renderer.render(host.commands()).is_empty() {
            return Ok(());
        }
        let mut buffer = surface.buffer_mut()?;
        for (target, pixel) in buffer.iter_mut().zip(renderer.pixels()) {
            *target = u32::from(pixel.r) << 16 | u32::from(pixel.g) << 8 | u32::from(pixel.b);
        }
        buffer.present()?;
        Ok(())
    }

    fn tick(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let Some(window) = self.window.clone() else {
            return;
        };
        if now >= self.next_caret_blink {
            self.next_caret_blink = now + Duration::from_millis(500);
            if self
                .host
                .as_mut()
                .is_some_and(|host| host.handle_event(UiEvent::CaretBlink).changed)
            {
                self.scheduler.invalidate();
            }
        }
        if self
            .host
            .as_ref()
            .and_then(UiHost::next_deadline)
            .is_some_and(|deadline| now >= deadline)
            && self.host.as_mut().is_some_and(|host| {
                host.step(HostBatch {
                    now: Some(now),
                    events: vec![HostEvent::Poll],
                    ..HostBatch::default()
                })
                .changed
            })
        {
            self.scheduler.invalidate();
        }
        let adapter_due = self
            .next_adapter_poll
            .is_some_and(|deadline| now >= deadline)
            || self
                .adapter
                .next_deadline(now)
                .is_some_and(|deadline| now >= deadline);
        if adapter_due {
            let outcome = self
                .host
                .as_mut()
                .map(|host| self.adapter.poll(host, HostServices { window: &window }));
            match outcome {
                Some(Ok(outcome)) => self.apply_adapter_outcome(event_loop, outcome),
                Some(Err(error)) => {
                    self.fail(event_loop, error);
                    return;
                }
                None => {}
            }
            self.next_adapter_poll = self.adapter.poll_interval().map(|interval| now + interval);
        }
        if self.controller_schedule.is_due(now) {
            let focused = self
                .host
                .as_ref()
                .is_some_and(|host| host.inspect().window_focused);
            let (actions, controller_connected) = if local_controller_poll_lease(
                self.controller_discovery,
                self.local_controller_lease,
            )
            .is_some()
            {
                let Some(controller) = &mut self.controller else {
                    return;
                };
                let actions: Vec<_> = controller
                    .poll(now, focused)
                    .into_iter()
                    .map(|action| {
                        (
                            Some(action),
                            controller.active_family().unwrap_or_default(),
                            None::<(ControllerExecutionBinding, ControllerExecutionAuthority)>,
                        )
                    })
                    .collect();
                (actions, controller.connected())
            } else {
                #[cfg(any(unix, windows))]
                {
                    let connected = matches!(
                        self.session_controller,
                        SessionControllerSource::Connecting { .. }
                            | SessionControllerSource::Attached { .. }
                    );
                    let broker_actions = self.session_controller.poll_actions();
                    (
                        broker_actions
                            .into_iter()
                            .filter(|_| focused)
                            .map(|(action, family, binding, authority)| {
                                (action, family, Some((binding, authority)))
                            })
                            .collect(),
                        connected,
                    )
                }
                #[cfg(not(any(unix, windows)))]
                {
                    (Vec::new(), false)
                }
            };
            for (action, family, admission) in actions {
                let Some(host) = self.host.as_mut() else {
                    break;
                };
                // The session oracle is independent of the delivery, while native focus is
                // sampled again at the actual dispatch boundary. A queued action therefore
                // cannot survive retirement between poll and execution.
                if admission.is_some() && !host.inspect().window_focused {
                    continue;
                }
                if host.set_controller_family(family) {
                    self.scheduler.invalidate();
                }
                let outcome = if let Some((binding, authority)) = admission {
                    host.step(HostBatch {
                        controller_authority: Some(authority),
                        events: vec![HostEvent::AdmittedController { action, binding }],
                        ..HostBatch::default()
                    })
                } else {
                    host.handle_controller_action(action.expect("local controller action"))
                };
                if action == Some(ControllerAction::Confirm)
                    && outcome.text_input_active
                    && host.controller_targets_text_input()
                {
                    self.adapter
                        .request_text_entry(HostServices { window: &window });
                }
                if outcome.changed {
                    self.scheduler.invalidate();
                }
                #[cfg(any(unix, windows))]
                if let Some(execution) = outcome.controller_executions.iter().find(|execution| {
                    execution.disposition == ControllerExecutionDisposition::ResetOverflow
                }) {
                    self.session_controller
                        .report_execution_overflow(execution.binding);
                }
                for action in outcome.global_actions {
                    match self
                        .adapter
                        .global_action(host, action, HostServices { window: &window })
                    {
                        Ok(outcome) => {
                            if outcome.changed {
                                host.rebuild();
                                self.scheduler.invalidate();
                            }
                            if outcome.exit {
                                event_loop.exit();
                            }
                        }
                        Err(error) => {
                            self.fail(event_loop, error);
                            return;
                        }
                    }
                }
            }
            self.controller_schedule
                .mark_polled(now, controller_connected);
        }
        if self.scheduler.request_present() {
            window.request_redraw();
        }
        let deadline = wait_duration(
            now,
            [
                Some(self.next_caret_blink),
                self.host.as_ref().and_then(UiHost::next_deadline),
                self.next_adapter_poll,
                self.adapter.next_deadline(now),
                Some(self.controller_schedule.deadline()),
            ],
        )
        .map(|wait| now + wait);
        event_loop.set_control_flow(deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }

    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        if let (Some(host), Some(window)) = (self.host.as_mut(), self.window.as_ref()) {
            if let Err(error) = self.adapter.stopped(host, HostServices { window }) {
                self.error.get_or_insert(error);
            }
            host.shutdown();
        }
        self.surface = None;
        self.window = None;
    }
}

impl<A: Application, H: HostAdapter<A>> ApplicationHandler for ApplicationRuntime<A, H> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let application = self
            .application
            .as_ref()
            .expect("application before first resume");
        let (width, height) = application.initial_size();
        let attributes = WindowAttributes::default()
            .with_title(application.title())
            .with_inner_size(LogicalSize::new(width, height))
            .with_resizable(true);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        self.scale = window.scale_factor() as f32;
        self.transform_generation = self.transform_generation.wrapping_add(1).max(1);
        self.input.set_scale_factor(window.scale_factor());
        let logical = window.inner_size().to_logical::<u32>(window.scale_factor());
        let mut host = UiHost::new(
            self.application.take().expect("application available"),
            logical.width,
            logical.height,
        );
        host.set_scale_factor(self.scale);
        match self
            .adapter
            .started(&mut host, HostServices { window: &window })
        {
            Ok(outcome) => {
                if outcome.changed {
                    host.rebuild();
                }
                if outcome.exit {
                    event_loop.exit();
                }
            }
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        }
        let context = match softbuffer::Context::new(self.display.clone()) {
            Ok(context) => context,
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        let surface = match softbuffer::Surface::new(&context, window.clone()) {
            Ok(surface) => surface,
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        let size = window.inner_size();
        self.renderer = Some(SoftwareRenderer::new_pixel_buffer(
            size.width,
            size.height,
            self.scale,
        ));
        self.surface = Some(surface);
        self.host = Some(host);
        self.window = Some(window.clone());
        self.scheduler.invalidate();
        if self.scheduler.request_present() {
            window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if window.id() != window_id {
            return;
        }
        let adapted = self.host.as_mut().map(|host| {
            self.adapter
                .event(host, &event, HostServices { window: &window })
        });
        let consume = match adapted {
            Some(Ok(outcome)) => {
                let consume = outcome.disposition != crate::EventDisposition::Unhandled;
                self.apply_adapter_outcome(event_loop, outcome);
                consume
            }
            Some(Err(error)) => {
                self.fail(event_loop, error);
                return;
            }
            None => return,
        };
        if consume {
            return;
        }
        match &event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => {
                self.flush_pending_continuous_input(event_loop, &window);
                if self.scheduler.begin_present()
                    && let Err(error) = self.present()
                {
                    self.fail(event_loop, error);
                }
                return;
            }
            WindowEvent::Resized(size) => {
                self.transform_generation = self.transform_generation.wrapping_add(1).max(1);
                let logical = size.to_logical::<u32>(window.scale_factor());
                if let Some(host) = &mut self.host {
                    host.resize(logical.width, logical.height);
                }
                self.scheduler.invalidate();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = *scale_factor as f32;
                self.transform_generation = self.transform_generation.wrapping_add(1).max(1);
                self.input.set_scale_factor(*scale_factor);
                let logical = window.inner_size().to_logical::<u32>(*scale_factor);
                if let Some(host) = &mut self.host {
                    host.resize(logical.width, logical.height);
                    host.set_scale_factor(self.scale);
                }
                self.scheduler.invalidate();
            }
            WindowEvent::Occluded(true) => {
                if let Some(host) = &mut self.host {
                    host.handle_event(UiEvent::Suspended);
                }
                if let Some(renderer) = &mut self.renderer {
                    renderer.suspend();
                }
                return;
            }
            WindowEvent::Occluded(false) => self.scheduler.invalidate(),
            WindowEvent::HoveredFile(path) => {
                if self.host.as_mut().is_some_and(|host| {
                    host.application_mut()
                        .file_drag_event(FileDragEvent::Hovered(path.clone()))
                }) {
                    self.scheduler.invalidate();
                }
            }
            WindowEvent::HoveredFileCancelled => {
                if self.host.as_mut().is_some_and(|host| {
                    host.application_mut()
                        .file_drag_event(FileDragEvent::HoverCancelled)
                }) {
                    self.scheduler.invalidate();
                }
            }
            WindowEvent::FileDropActionChanged(action) => {
                let action = match action {
                    winit::event::FileDropAction::Copy => FileDragAction::Copy,
                    winit::event::FileDropAction::Move => FileDragAction::Move,
                };
                if self.host.as_mut().is_some_and(|host| {
                    host.application_mut()
                        .file_drag_event(FileDragEvent::ActionChanged(action))
                }) {
                    self.scheduler.invalidate();
                }
            }
            WindowEvent::DroppedFile(path) => {
                let handled = self.host.as_mut().is_some_and(|host| {
                    host.application_mut()
                        .file_drag_event(FileDragEvent::Dropped(path.clone()))
                });
                if handled {
                    self.scheduler.invalidate();
                }
            }
            _ => {}
        }
        for normalized in self.input.normalize(STANDALONE_AGGREGATE_DEVICE, &event) {
            if matches!(
                normalized,
                nickel_input::InputEvent::Pointer(
                    nickel_input::PointerEvent::Axis { .. }
                        | nickel_input::PointerEvent::Motion { .. }
                )
            ) {
                if self.native_stream_reset_pending {
                    continue;
                }
                let sample = self.admit_continuous_input(normalized);
                if sample.envelope.recipient.lease == 0 {
                    continue;
                }
                if queue_continuous_input(&mut self.pending_continuous_input, sample)
                    == ContinuousQueueOutcome::ResetRequired
                {
                    self.revoke_native_ingress_after_reset();
                }
                self.scheduler.invalidate();
                continue;
            }
            self.flush_pending_continuous_input(event_loop, &window);
            let image_pasted = if is_clipboard_paste(&normalized) {
                self.clipboard
                    .as_mut()
                    .and_then(|clipboard| clipboard.get_image().ok())
                    .and_then(|image| {
                        let width = u32::try_from(image.width).ok()?;
                        let height = u32::try_from(image.height).ok()?;
                        self.host.as_mut().map(|host| {
                            host.paste_clipboard_image(width, height, image.bytes.as_ref())
                        })
                    })
                    .unwrap_or(false)
            } else {
                false
            };
            if image_pasted {
                self.scheduler.invalidate();
                continue;
            }
            let clipboard_text = self
                .clipboard
                .as_mut()
                .and_then(|clipboard| clipboard.get_text().ok());
            self.dispatch_normalized_input(event_loop, &window, vec![normalized], clipboard_text);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.tick(event_loop);
    }
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        #[cfg(any(unix, windows))]
        self.session_controller.relinquish();
        self.stop();
    }
}

fn is_clipboard_paste(input: &nickel_input::InputEvent) -> bool {
    matches!(input, nickel_input::InputEvent::Key(key)
        if key.edge == nickel_input::KeyEdge::Pressed
        && matches!(key.logical, nickel_input::LogicalKey::Character(ref value) if value.eq_ignore_ascii_case("v"))
        && (key.modifiers.aggregate(nickel_input::AggregateModifier::Control)
            || key.modifiers.aggregate(nickel_input::AggregateModifier::Super)))
}

#[cfg(test)]
mod tests {
    use crate::ui::ComponentBuilderExt;
    static SYNTHETIC_INGRESS_ORDER: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(1);
    use nickel_input::{
        DeviceId, EventOrder, InputEvent, KeyCode, KeyEdge, KeyEvent, KeyLocation, LogicalKey,
        Modifier, ModifierState, NamedKey, PhysicalKey, Point, PointerButton, PointerEvent,
        TextEvent, TouchEvent, TouchId, Vector,
    };
    use std::time::{Duration, Instant};

    #[cfg(unix)]
    use super::SessionControllerSource;
    use super::{
        AdmittedNormalizedInput, Application, Completion, CompletionFailure, CompletionFailureKind,
        ContinuousQueueOutcome, ControllerDiscoveryMode, ControllerPollSchedule, ControllerRole,
        ControllerRoleLease, EffectEvidence, FrameOverlay, GlobalAction, HostBatch, HostEvent,
        HostFailure, HostFailureStage, MAX_ADMITTED_CONTROLLER_PRESSES, MessageEvidence,
        NormalizedAdmissionBinding, NormalizedInputEnvelope, NormalizedRecipientBinding,
        NormalizedSourceBinding, PresentScheduler, Shortcut, ShortcutOutcome, UiHost, ViewContext,
        grant_native_ingress_if_focused, local_controller_poll_lease, native_input_class,
        native_input_source_binding, native_pointer_source_binding, queue_continuous_input,
        revoke_native_ingress, transform_is_current, wait_duration,
    };

    #[test]
    fn controller_poll_ownership_is_explicit_and_role_scoped() {
        let local = ControllerRoleLease::local();
        let next_local = ControllerRoleLease::local();
        let session = ControllerRoleLease::session(3, 7);

        assert_eq!(local.role, ControllerRole::Local);
        assert_ne!(local.generation, next_local.generation);
        assert_eq!(
            local_controller_poll_lease(ControllerDiscoveryMode::Standalone, Some(local)),
            Some(local)
        );
        assert_eq!(
            local_controller_poll_lease(ControllerDiscoveryMode::Session, Some(local)),
            None,
            "a session-attached runtime cannot retain an independent local poller"
        );
        assert_eq!(
            local_controller_poll_lease(ControllerDiscoveryMode::Standalone, Some(session)),
            None,
            "a session role lease cannot authorize the standalone device reader"
        );
    }

    fn synthetic_normalized(input: InputEvent, clipboard_text: Option<String>) -> HostEvent {
        let device_generation = input.device().map_or(0, |device| device.0);
        HostEvent::NormalizedIngress(NormalizedInputEnvelope {
            input,
            clipboard_text,
            source: NormalizedSourceBinding {
                seat: 0,
                backend_stream: "runtime-unit-test".into(),
                stream_generation: 1,
                device_generation,
                identity_capability: "synthetic-fixture".into(),
                reconnect_generation: device_generation,
            },
            admission: NormalizedAdmissionBinding {
                order: SYNTHETIC_INGRESS_ORDER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                monotonic_micros: 0,
            },
            recipient: NormalizedRecipientBinding {
                lease: 1,
                lifetime: 1,
            },
            operation: None,
            transform_generation: None,
            text_transaction: None,
            transfer_cutoff: None,
            broker_event_id: None,
            host_connection_generation: 1,
            operation_epoch: None,
            role: "synthetic-test".into(),
            coordinate_meaning: "host-logical".into(),
            composition_recipient_epoch: None,
        })
    }

    use crate::{
        ActionKind, Button, Container, ControllerAction, ControllerExecutionAuthority,
        ControllerExecutionBinding, ControllerExecutionDisposition, ControllerExecutionEvidence,
        InputModality, Invalidation, NavigationEntry, NavigationScope, OverlayId, SemanticAction,
        SemanticActionError, SemanticRole, SemanticValueInput, Slider, TextField, UiEvent, UiId,
        UiStateStore,
    };

    #[cfg(any(unix, windows))]
    #[test]
    fn session_controller_clients_share_the_nonblocking_runtime_contract() {
        use nickel_session_protocol::{
            ControllerHostRequest, ControllerHostResponse, client::AsyncControllerConnection,
        };

        let _begin: fn(Duration) -> std::io::Result<Option<AsyncControllerConnection>> =
            AsyncControllerConnection::begin_from_environment;
        let _send: fn(
            &mut AsyncControllerConnection,
            ControllerHostRequest,
        ) -> std::io::Result<()> = AsyncControllerConnection::send;
        let _receive: fn(
            &mut AsyncControllerConnection,
        ) -> std::io::Result<Option<ControllerHostResponse>> = AsyncControllerConnection::receive;
        let _relinquish: fn(
            &mut AsyncControllerConnection,
            nickel_session_protocol::controller_broker::ConnectionGeneration,
        ) -> std::io::Result<()> = AsyncControllerConnection::relinquish;
    }

    #[test]
    fn normalized_execution_rejects_replay_and_spoofed_recipient() {
        let HostEvent::NormalizedIngress(mut envelope) = synthetic_normalized(
            InputEvent::FocusGained {
                order: EventOrder(1),
            },
            None,
        ) else {
            unreachable!()
        };
        envelope.source.identity_capability = "native-device".into();
        let authority = envelope.execution_authority();
        let mut host = UiHost::new(EffectApplication::default(), 160, 48);
        assert!(host.admits_normalized_ingress(&envelope, std::slice::from_ref(&authority)));
        assert!(!host.admits_normalized_ingress(&envelope, std::slice::from_ref(&authority)));
        envelope.admission.order += 1;
        envelope.recipient.lifetime += 1;
        assert!(!host.admits_normalized_ingress(&envelope, &[authority]));
    }

    #[derive(Clone)]
    enum Message {
        Changed(String),
    }

    #[derive(Default)]
    struct InputApplication {
        text: String,
        submits: usize,
    }

    struct MultilineInputApplication {
        text: String,
    }

    struct SecureInputApplication {
        text: String,
    }

    #[derive(Default)]
    struct ControllerApplication;

    impl Application for ControllerApplication {
        type Message = ();

        fn update(&mut self, (): Self::Message) {}

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            Button::new((), "Activate")
        }
    }

    struct NavigationApplication;

    impl Application for NavigationApplication {
        type Message = ();

        fn update(&mut self, (): Self::Message) {}

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            Container::new()
                .id("scope")
                .navigation_scope(NavigationScope::group().entry(NavigationEntry::Last))
                .children([
                    Button::new((), "First").id("first"),
                    Button::new((), "Last").id("last"),
                ])
        }
    }

    #[derive(Default)]
    struct CrossInputApplication {
        invoked: Vec<&'static str>,
    }

    impl Application for CrossInputApplication {
        type Message = &'static str;

        fn update(&mut self, message: Self::Message) {
            self.invoked.push(message);
        }

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            Container::new()
                .children([Button::new("A", "A").id("a"), Button::new("B", "B").id("b")])
        }
    }

    struct AdjustmentApplication {
        value: f32,
    }

    #[derive(Default)]
    struct ContextApplication {
        invoked: Vec<&'static str>,
    }

    impl Application for ContextApplication {
        type Message = &'static str;

        fn update(&mut self, message: Self::Message) {
            self.invoked.push(message);
        }

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            Container::new().children([
                Button::new("A", "A").id("a").context_message("A-context"),
                Button::new("B", "B").id("b").context_message("B-context"),
            ])
        }
    }

    impl Application for AdjustmentApplication {
        type Message = f32;

        fn update(&mut self, value: Self::Message) {
            self.value = value;
        }

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            Slider::on_change(|value| value, self.value).id("value")
        }
    }

    #[derive(Default)]
    struct ResponsiveApplication;

    impl Application for ResponsiveApplication {
        type Message = ();

        fn update(&mut self, (): Self::Message) {}

        fn view(&self, context: ViewContext) -> impl crate::View<Self::Message> {
            let label = if context.modality == crate::InputModality::Controller {
                "Controller"
            } else if context.viewport.size.width < 200.0 {
                "Narrow"
            } else {
                "Wide"
            };
            Button::new((), label)
        }
    }

    #[derive(Default)]
    struct CompletionApplication(u32);

    impl Application for CompletionApplication {
        type Message = ();

        fn update(&mut self, (): Self::Message) {}

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            Button::new((), self.0.to_string())
        }

        fn complete(&mut self, completion: Completion) -> Result<bool, CompletionFailure> {
            let id = completion.id;
            let value = completion
                .downcast::<u32>()
                .map_err(|_| CompletionFailure {
                    id,
                    kind: CompletionFailureKind::TypeMismatch,
                    detail: "expected u32".into(),
                })?;
            self.0 = value;
            Ok(true)
        }
    }

    #[derive(Default)]
    struct EffectApplication {
        pending: Vec<EffectEvidence>,
    }

    impl Application for EffectApplication {
        type Message = ();

        fn update(&mut self, (): Self::Message) {
            self.pending.push(EffectEvidence {
                type_name: "test.effect",
                label: Some("activated".into()),
            });
        }

        fn take_effect_evidence(&mut self) -> Vec<EffectEvidence> {
            std::mem::take(&mut self.pending)
        }

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            Button::new((), "Effect")
        }
    }

    impl Application for InputApplication {
        type Message = Message;

        fn update(&mut self, message: Self::Message) {
            match message {
                Message::Changed(text) => self.text = text,
            }
        }

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            TextField::on_change(&self.text, Message::Changed)
        }

        fn shortcut_outcome(&mut self, shortcut: Shortcut) -> ShortcutOutcome {
            if shortcut != Shortcut::Submit {
                return ShortcutOutcome::from_changed(false);
            }
            self.submits += 1;
            ShortcutOutcome::handled(true)
        }
    }

    impl Application for MultilineInputApplication {
        type Message = Message;

        fn update(&mut self, message: Self::Message) {
            let Message::Changed(text) = message;
            self.text = text;
        }

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            TextField::on_change(&self.text, Message::Changed).wrap(true)
        }
    }

    impl Application for SecureInputApplication {
        type Message = Message;

        fn update(&mut self, message: Self::Message) {
            let Message::Changed(text) = message;
            self.text = text;
        }

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            TextField::on_change_masked(&self.text, '•', Message::Changed)
        }
    }

    fn key(order: u64, repeat: bool) -> InputEvent {
        InputEvent::Key(KeyEvent {
            device: DeviceId(1),
            order: EventOrder(order),
            physical: PhysicalKey::Code(KeyCode::Enter),
            logical: LogicalKey::Named(NamedKey::Enter),
            location: KeyLocation::Standard,
            edge: KeyEdge::Pressed,
            repeat,
            modifiers: ModifierState::default(),
        })
    }

    fn command_key(order: u64, physical: KeyCode, logical: &str) -> InputEvent {
        InputEvent::Key(KeyEvent {
            device: DeviceId(1),
            order: EventOrder(order),
            physical: PhysicalKey::Code(physical),
            logical: LogicalKey::Character(logical.into()),
            location: KeyLocation::Standard,
            edge: KeyEdge::Pressed,
            repeat: false,
            modifiers: ModifierState::from_sides([Modifier::ControlLeft]),
        })
    }

    fn focus_event() -> InputEvent {
        InputEvent::Pointer(PointerEvent::Button {
            device: DeviceId(2),
            order: EventOrder(1),
            button: PointerButton::Primary,
            edge: KeyEdge::Pressed,
            position: Some(Point { x: 4.0, y: 4.0 }),
        })
    }

    #[test]
    fn idle_frames_do_not_present_and_present_requests_coalesce() {
        let mut scheduler = PresentScheduler::default();
        assert!(!scheduler.request_present());
        scheduler.invalidate();
        scheduler.invalidate();
        scheduler.invalidate();
        assert!(scheduler.request_present());
        assert!(!scheduler.request_present());
        scheduler.invalidate();
        assert!(!scheduler.request_present());
        assert!(scheduler.begin_present());
        assert!(!scheduler.begin_present());
    }

    #[test]
    fn continuous_pointer_motion_keeps_latest_position_and_total_delta() {
        let admitted = |input| {
            let HostEvent::NormalizedIngress(envelope) = synthetic_normalized(input, None) else {
                unreachable!()
            };
            let authority = envelope.execution_authority();
            AdmittedNormalizedInput {
                envelope,
                authority,
            }
        };
        let mut pending = Vec::new();
        for (order, position, delta) in [
            (1, Point { x: 10.0, y: 20.0 }, Vector { x: 1.0, y: 2.0 }),
            (2, Point { x: 14.0, y: 26.0 }, Vector { x: 4.0, y: 6.0 }),
        ] {
            queue_continuous_input(
                &mut pending,
                admitted(InputEvent::Pointer(PointerEvent::Motion {
                    device: DeviceId(7),
                    order: EventOrder(order),
                    position,
                    delta: Some(delta),
                })),
            );
        }

        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].envelope.input,
            InputEvent::Pointer(PointerEvent::Motion {
                device: DeviceId(7),
                order: EventOrder(2),
                position: Point { x: 14.0, y: 26.0 },
                delta: Some(Vector { x: 5.0, y: 8.0 }),
            })
        );
    }

    #[test]
    fn continuous_input_overflow_inserts_reset_barrier_and_stays_bounded() {
        let admitted = |input| {
            let HostEvent::NormalizedIngress(envelope) = synthetic_normalized(input, None) else {
                unreachable!()
            };
            let authority = envelope.execution_authority();
            AdmittedNormalizedInput {
                envelope,
                authority,
            }
        };
        let mut pending = Vec::new();
        let mut reset_required = false;
        for order in 1..=300 {
            reset_required |= queue_continuous_input(
                &mut pending,
                admitted(InputEvent::Pointer(PointerEvent::Motion {
                    device: DeviceId(order % 2),
                    order: EventOrder(order),
                    position: Point {
                        x: order as f64,
                        y: 0.0,
                    },
                    delta: None,
                })),
            ) == ContinuousQueueOutcome::ResetRequired;
        }
        assert!(reset_required);
        assert!(pending.len() <= 256);
        assert!(
            pending
                .iter()
                .any(|event| matches!(event.envelope.input, InputEvent::FocusLost { .. }))
        );
    }

    #[test]
    fn overflow_reset_revokes_recipient_and_rotates_stream_generation() {
        let mut recipient = NormalizedRecipientBinding {
            lease: 41,
            lifetime: 41,
        };
        let mut stream_generation = 41;

        revoke_native_ingress(&mut recipient, &mut stream_generation, 42);

        assert_eq!(stream_generation, 42);
        assert_eq!(recipient.lease, 0);
        assert_eq!(recipient.lifetime, 42);

        assert!(!grant_native_ingress_if_focused(&mut recipient, false, 43));
        assert_eq!(recipient.lease, 0);

        assert!(grant_native_ingress_if_focused(&mut recipient, true, 43));
        assert_eq!(recipient.lease, 43);
        assert_eq!(recipient.lifetime, 43);
    }

    #[test]
    fn native_pointer_stream_has_input_class_authority_separate_from_controller() {
        let pointer = native_pointer_source_binding(17, 9);
        let controller = NormalizedSourceBinding {
            seat: 0,
            backend_stream: "controller-broker".into(),
            stream_generation: 17,
            device_generation: 9,
            identity_capability: "broker".into(),
            reconnect_generation: 9,
        };

        assert_eq!(pointer.backend_stream, "winit-pointer-seat-0");
        assert_ne!(pointer, controller);
    }

    #[test]
    fn pointer_button_edges_share_motion_gesture_authority() {
        let motion = InputEvent::Pointer(PointerEvent::Motion {
            device: DeviceId(7),
            order: EventOrder(1),
            position: Point { x: 2.0, y: 3.0 },
            delta: None,
        });
        let button = InputEvent::Pointer(PointerEvent::Button {
            device: DeviceId(7),
            order: EventOrder(2),
            button: PointerButton::Primary,
            edge: KeyEdge::Released,
            position: Some(Point { x: 2.0, y: 3.0 }),
        });

        let motion_class = native_input_class(&motion);
        let button_class = native_input_class(&button);
        assert_eq!(motion_class, button_class);
        assert_eq!(
            native_input_source_binding(motion_class, 11, 7),
            native_input_source_binding(button_class, 11, 7)
        );
    }

    #[test]
    fn keyboard_text_and_touch_use_focused_authority_classes() {
        let text = InputEvent::Text(TextEvent::Commit {
            device: DeviceId(3),
            order: EventOrder(1),
            text: "x".into(),
        });
        let touch = InputEvent::Touch(TouchEvent::Cancelled {
            device: DeviceId(4),
            order: EventOrder(2),
            contact: TouchId(1),
        });

        let text_source = native_input_source_binding(native_input_class(&text), 8, 3);
        let touch_source = native_input_source_binding(native_input_class(&touch), 8, 4);
        assert_eq!(text_source.backend_stream, "winit-keyboard-seat-0");
        assert_eq!(touch_source.backend_stream, "winit-touch-seat-0");
        assert_ne!(text_source.backend_stream, touch_source.backend_stream);
    }

    #[test]
    fn continuous_sample_is_rejected_after_transform_changes() {
        let HostEvent::NormalizedIngress(mut envelope) = synthetic_normalized(
            InputEvent::Pointer(PointerEvent::Motion {
                device: DeviceId(7),
                order: EventOrder(1),
                position: Point { x: 2.0, y: 3.0 },
                delta: None,
            }),
            None,
        ) else {
            unreachable!()
        };
        envelope.transform_generation = Some(8);
        let authority = envelope.execution_authority();
        let sample = AdmittedNormalizedInput {
            envelope,
            authority,
        };

        assert!(transform_is_current(&sample, 8));
        assert!(!transform_is_current(&sample, 9));
    }

    #[test]
    fn continuous_input_does_not_coalesce_across_recipient_rotation() {
        let admitted = |input, generation| {
            let HostEvent::NormalizedIngress(mut envelope) = synthetic_normalized(input, None)
            else {
                unreachable!()
            };
            envelope.recipient = NormalizedRecipientBinding {
                lease: generation,
                lifetime: generation,
            };
            envelope.host_connection_generation = generation;
            envelope.composition_recipient_epoch = Some(generation);
            let authority = envelope.execution_authority();
            AdmittedNormalizedInput {
                envelope,
                authority,
            }
        };
        let motion = |order| {
            InputEvent::Pointer(PointerEvent::Motion {
                device: DeviceId(7),
                order: EventOrder(order),
                position: Point {
                    x: order as f64,
                    y: 0.0,
                },
                delta: None,
            })
        };
        let mut pending = Vec::new();
        queue_continuous_input(&mut pending, admitted(motion(1), 10));
        queue_continuous_input(&mut pending, admitted(motion(2), 11));

        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].envelope.recipient.lifetime, 10);
        assert_eq!(pending[1].envelope.recipient.lifetime, 11);
    }

    #[test]
    fn host_telemetry_reports_phases_and_explicit_allocation_unavailability() {
        let mut host = UiHost::new(ControllerApplication, 320, 200);
        let outcome = host.step(HostBatch {
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        assert_eq!(outcome.telemetry.scheduled_wakeups, 1);
        assert!(outcome.telemetry.input_to_frame_us >= outcome.telemetry.input_to_message_us);
        assert!(outcome.telemetry.retained_frame_bytes > 0);
        assert_eq!(outcome.telemetry.allocation_count, None);
    }

    #[test]
    fn ui_host_dispatches_declared_scope_entry_through_the_production_frame() {
        let mut host = UiHost::new(NavigationApplication, 320, 200);
        host.handle_event(UiEvent::ControllerDown);
        host.handle_event(UiEvent::ControllerActivate);
        let selected = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.controller_selected)
            .expect("declared entry selects a semantic target");
        assert!(selected.id.as_str().ends_with("/last"));
        assert_eq!(selected.navigation_depth, 1);
    }

    #[test]
    fn host_batch_drains_typed_effects_and_reports_change_token() {
        let mut host = UiHost::new(EffectApplication::default(), 160, 48);
        let outcome = host.perform_semantic_action(
            UiId::from("root"),
            SemanticAction::Invoke(ActionKind::Activate),
        );
        assert_eq!(outcome.effects.len(), 1);
        assert_eq!(outcome.effects[0].type_name, "test.effect");
        assert_eq!(
            outcome.change_token.frame_generation,
            host.inspect().frame_generation
        );
        assert_eq!(
            outcome.change_token.semantic_generation,
            host.inspect().semantic_generation
        );

        let idle = host.step(HostBatch::default());
        assert!(
            idle.effects.is_empty(),
            "effects must be drained exactly once"
        );
        assert_eq!(idle.change_token, outcome.change_token);
    }

    #[test]
    fn admitted_controller_revalidates_identity_at_execution_and_tags_effects() {
        let binding = ControllerExecutionBinding {
            device_generation: 5,
            edge: nickel_input::KeyEdge::Pressed,
            routing_epoch: 9,
            event_id: 41,
            lease_epoch: 7,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: Some(41),
            surface_generation: Some(10),
            repeat: false,
        };
        let authority = ControllerExecutionAuthority {
            routing_epoch: 9,
            lease_epoch: 7,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: Some(41),
            surface_generation: Some(10),
        };
        assert!(!authority.admits(ControllerExecutionBinding {
            surface_generation: Some(11),
            ..binding
        }));
        let mut host = UiHost::new(EffectApplication::default(), 160, 48);
        let outcome = host.step(HostBatch {
            window_focused: Some(true),
            controller_authority: Some(authority),
            events: vec![
                HostEvent::Controller(ControllerAction::Down),
                HostEvent::AdmittedController {
                    action: Some(ControllerAction::Confirm),
                    binding,
                },
            ],
            ..HostBatch::default()
        });
        assert_eq!(outcome.effects.len(), 1);
        assert_eq!(
            outcome.controller_executions,
            [ControllerExecutionEvidence {
                binding,
                disposition: ControllerExecutionDisposition::Executed,
                message_count: 1,
                effect_count: 1,
            }]
        );

        let stale_repeat = ControllerExecutionBinding {
            routing_epoch: 8,
            event_id: 42,
            cutoff: Some(41),
            repeat: true,
            ..binding
        };
        let rejected = host.step(HostBatch {
            controller_authority: Some(authority),
            events: vec![HostEvent::AdmittedController {
                action: Some(ControllerAction::Confirm),
                binding: stale_repeat,
            }],
            ..HostBatch::default()
        });
        assert!(rejected.effects.is_empty());
        assert_eq!(
            rejected.controller_executions,
            [ControllerExecutionEvidence {
                binding: stale_repeat,
                disposition: ControllerExecutionDisposition::RejectedStale,
                message_count: 0,
                effect_count: 0,
            }]
        );
    }

    #[test]
    fn controller_release_is_unpaired_after_focus_or_lease_boundary() {
        let binding = ControllerExecutionBinding {
            device_generation: 5,
            edge: KeyEdge::Pressed,
            routing_epoch: 9,
            event_id: 1,
            lease_epoch: 7,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: None,
            surface_generation: Some(10),
            repeat: false,
        };
        let authority = ControllerExecutionAuthority {
            routing_epoch: 9,
            lease_epoch: 7,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: None,
            surface_generation: Some(10),
        };
        let mut host = UiHost::new(EffectApplication::default(), 160, 48);
        host.step(HostBatch {
            controller_authority: Some(authority),
            events: vec![HostEvent::AdmittedController {
                action: Some(ControllerAction::Confirm),
                binding,
            }],
            ..HostBatch::default()
        });
        host.step(HostBatch {
            window_focused: Some(false),
            ..HostBatch::default()
        });
        let release = ControllerExecutionBinding {
            edge: KeyEdge::Released,
            event_id: 2,
            ..binding
        };
        let outcome = host.step(HostBatch {
            controller_authority: Some(authority),
            events: vec![HostEvent::AdmittedController {
                action: Some(ControllerAction::Confirm),
                binding: release,
            }],
            ..HostBatch::default()
        });

        assert_eq!(
            outcome.controller_executions[0].disposition,
            ControllerExecutionDisposition::RejectedUnpairedRelease
        );
    }

    #[test]
    fn rejected_controller_release_retires_the_affected_press() {
        let binding = ControllerExecutionBinding {
            device_generation: 5,
            edge: KeyEdge::Pressed,
            routing_epoch: 9,
            event_id: 1,
            lease_epoch: 7,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: None,
            surface_generation: Some(10),
            repeat: false,
        };
        let authority = ControllerExecutionAuthority {
            routing_epoch: 9,
            lease_epoch: 7,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: None,
            surface_generation: Some(10),
        };
        let mut host = UiHost::new(EffectApplication::default(), 160, 48);
        host.step(HostBatch {
            controller_authority: Some(authority),
            events: vec![HostEvent::AdmittedController {
                action: Some(ControllerAction::Confirm),
                binding,
            }],
            ..HostBatch::default()
        });
        let stale_release = ControllerExecutionBinding {
            edge: KeyEdge::Released,
            routing_epoch: 8,
            event_id: 2,
            ..binding
        };
        let stale = host.step(HostBatch {
            controller_authority: Some(authority),
            events: vec![HostEvent::AdmittedController {
                action: Some(ControllerAction::Confirm),
                binding: stale_release,
            }],
            ..HostBatch::default()
        });
        assert_eq!(
            stale.controller_executions[0].disposition,
            ControllerExecutionDisposition::RejectedStale
        );

        let valid_release = ControllerExecutionBinding {
            edge: KeyEdge::Released,
            event_id: 3,
            ..binding
        };
        let valid = host.step(HostBatch {
            controller_authority: Some(authority),
            events: vec![HostEvent::AdmittedController {
                action: Some(ControllerAction::Confirm),
                binding: valid_release,
            }],
            ..HostBatch::default()
        });
        assert_eq!(
            valid.controller_executions[0].disposition,
            ControllerExecutionDisposition::RejectedUnpairedRelease
        );
    }

    #[test]
    fn controller_press_ledger_is_bounded_and_resets_on_overflow() {
        let authority = ControllerExecutionAuthority {
            routing_epoch: 9,
            lease_epoch: 7,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: None,
            surface_generation: Some(10),
        };
        let mut host = UiHost::new(EffectApplication::default(), 160, 48);
        let mut last = None;
        for device_generation in 1..=MAX_ADMITTED_CONTROLLER_PRESSES as u64 + 1 {
            let binding = ControllerExecutionBinding {
                device_generation,
                edge: KeyEdge::Pressed,
                routing_epoch: 9,
                event_id: device_generation,
                lease_epoch: 7,
                connection_generation: 3,
                stream_generation: 2,
                cutoff: None,
                surface_generation: Some(10),
                repeat: false,
            };
            last = Some(host.step(HostBatch {
                controller_authority: Some(authority),
                events: vec![HostEvent::AdmittedController {
                    action: Some(ControllerAction::Confirm),
                    binding,
                }],
                ..HostBatch::default()
            }));
        }

        assert!(host.admitted_controller_presses.is_empty());
        assert_eq!(
            last.unwrap().controller_executions[0].disposition,
            ControllerExecutionDisposition::ResetOverflow
        );

        let stale_repeat = ControllerExecutionBinding {
            device_generation: 1,
            edge: KeyEdge::Pressed,
            routing_epoch: 9,
            event_id: 300,
            lease_epoch: 7,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: None,
            surface_generation: Some(10),
            repeat: true,
        };
        let stale = host.step(HostBatch {
            controller_authority: Some(authority),
            events: vec![HostEvent::AdmittedController {
                action: Some(ControllerAction::Confirm),
                binding: stale_repeat,
            }],
            ..HostBatch::default()
        });
        assert_eq!(
            stale.controller_executions[0].disposition,
            ControllerExecutionDisposition::RejectedResetFence
        );

        let replacement = ControllerExecutionAuthority {
            routing_epoch: 10,
            lease_epoch: 8,
            ..authority
        };
        let replacement_press = ControllerExecutionBinding {
            routing_epoch: 10,
            lease_epoch: 8,
            repeat: false,
            event_id: 301,
            ..stale_repeat
        };
        let first_replacement = host.step(HostBatch {
            controller_authority: Some(replacement),
            events: vec![HostEvent::AdmittedController {
                action: Some(ControllerAction::Confirm),
                binding: replacement_press,
            }],
            ..HostBatch::default()
        });
        assert_eq!(
            first_replacement.controller_executions[0].disposition,
            ControllerExecutionDisposition::Executed
        );

        let rearmed = host.step(HostBatch {
            controller_authority: Some(replacement),
            events: vec![HostEvent::AdmittedController {
                action: Some(ControllerAction::Confirm),
                binding: ControllerExecutionBinding {
                    event_id: 302,
                    ..replacement_press
                },
            }],
            ..HostBatch::default()
        });
        assert_eq!(
            rearmed.controller_executions[0].disposition,
            ControllerExecutionDisposition::Executed
        );
    }

    #[test]
    fn event_wait_uses_the_earliest_declared_deadline_and_can_sleep_indefinitely() {
        let now = Instant::now();
        assert_eq!(wait_duration(now, [None, None]), None);
        assert_eq!(
            wait_duration(
                now,
                [
                    Some(now + Duration::from_millis(250)),
                    Some(now + Duration::from_millis(17)),
                    Some(now + Duration::from_secs(1)),
                ],
            ),
            Some(Duration::from_millis(17))
        );
    }

    #[test]
    fn application_poll_deadline_is_host_owned_and_advances_from_batch_time() {
        struct PollApplication;
        impl Application for PollApplication {
            type Message = ();
            fn update(&mut self, (): Self::Message) {}
            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                crate::Container::new()
            }
            fn poll_interval(&self) -> Option<Duration> {
                Some(Duration::from_millis(40))
            }
        }

        let origin = Instant::now();
        let mut host = UiHost::new_at(PollApplication, 100, 40, origin);
        assert_eq!(
            host.next_deadline(),
            Some(origin + Duration::from_millis(40))
        );
        let polled_at = origin + Duration::from_millis(45);
        let outcome = host.step(HostBatch {
            now: Some(polled_at),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        assert_eq!(
            outcome.next_deadline,
            Some(polled_at + Duration::from_millis(40))
        );
        assert_eq!(host.next_deadline(), outcome.next_deadline);
    }

    #[test]
    fn application_work_started_by_message_arms_a_new_poll_deadline() {
        struct DeferredApplication {
            pending: bool,
        }
        impl Application for DeferredApplication {
            type Message = ();
            fn update(&mut self, (): Self::Message) {
                self.pending = true;
            }
            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                Button::new((), "Start work")
            }
            fn poll_interval(&self) -> Option<Duration> {
                self.pending.then_some(Duration::from_millis(16))
            }
        }

        let origin = Instant::now();
        let mut host = UiHost::new_at(DeferredApplication { pending: false }, 100, 40, origin);
        assert_eq!(host.next_deadline(), None);
        let button = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.name.as_deref() == Some("Start work"))
            .expect("button is a semantic target");
        host.perform_semantic_action(button.id, SemanticAction::Invoke(ActionKind::Activate));

        assert!(host.next_deadline().is_some());
    }

    #[test]
    fn controller_poll_schedule_has_one_shared_bounded_cadence() {
        let now = Instant::now();
        let mut schedule = ControllerPollSchedule::new(now);
        assert!(schedule.is_due(now));
        schedule.mark_polled(now, true);
        assert_eq!(
            schedule.deadline(),
            now + ControllerPollSchedule::CONNECTED_INTERVAL
        );
        schedule.mark_polled(now, false);
        assert_eq!(
            schedule.deadline(),
            now + ControllerPollSchedule::DISCONNECTED_INTERVAL
        );
    }

    #[test]
    fn ui_host_batches_changes_into_one_frame_and_idle_steps_do_nothing() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        let initial = host.inspect();
        assert_eq!(initial.frame_generation, 1);
        assert_eq!(initial.resources.retained_build_scratch_bytes, 0);

        let idle = host.step(HostBatch::default());
        assert!(!idle.changed);
        assert_eq!(host.inspect().frame_generation, 1);

        let changed = host.step(HostBatch {
            events: vec![
                HostEvent::Shortcut(Shortcut::Submit),
                HostEvent::Shortcut(Shortcut::Submit),
            ],
            ..HostBatch::default()
        });
        assert!(changed.changed);
        assert_eq!(changed.invalidation, Invalidation::Layout);
        assert_eq!(host.application_mut().submits, 2);
        assert_eq!(host.inspect().frame_generation, 2);
        assert_eq!(host.inspect().resources.retained_build_scratch_bytes, 0);
    }

    #[test]
    fn explicit_application_change_rebuilds_at_an_unchanged_surface_size() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        host.application_mut().text = "loaded after construction".into();

        let outcome = host.step(HostBatch {
            application_changed: true,
            surface_size: Some((320, 48)),
            ..HostBatch::default()
        });

        assert!(outcome.changed);
        assert!(outcome.telemetry.rebuilt);
        assert_eq!(host.inspect().frame_generation, 2);
        assert!(host.commands().iter().any(|command| {
            matches!(command, crate::PaintCommand::Text { text, .. } if text == "loaded after construction")
        }));
    }

    #[test]
    fn typed_completions_are_applied_in_the_same_batch_before_one_rebuild() {
        let mut host = UiHost::new(CompletionApplication::default(), 160, 48);
        let outcome = host.step(HostBatch {
            completions: vec![Completion::new("loaded-count", 7_u32)],
            ..HostBatch::default()
        });
        assert!(outcome.changed);
        assert_eq!(outcome.invalidation, Invalidation::Layout);
        assert!(outcome.completion_failures.is_empty());
        assert_eq!(host.application().0, 7);
        assert_eq!(host.inspect().frame_generation, 2);

        let rejected = host.step(HostBatch {
            completions: vec![Completion::new("loaded-count", "wrong type")],
            ..HostBatch::default()
        });
        assert!(!rejected.changed);
        assert_eq!(
            rejected.completion_failures[0].kind,
            CompletionFailureKind::TypeMismatch
        );
    }

    #[test]
    fn declared_dialog_surface_renders_in_host_stack_and_cancel_restores_focus() {
        struct DialogApplication {
            confirmations: usize,
        }
        impl Application for DialogApplication {
            type Message = bool;
            fn update(&mut self, confirmed: Self::Message) {
                self.confirmations += usize::from(confirmed);
            }
            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                Button::new(false, "Open").id("anchor")
            }
            fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
                vec![FrameOverlay::surface(
                    crate::TransientSurface::dialog(
                        "dialog",
                        crate::OverlayAnchor::Node(UiId::from("anchor")),
                        crate::Size::new(120.0, 80.0),
                        crate::OverlayStyle {
                            background: 0x111111,
                            foreground: 0xffffff,
                            border: 0x888888,
                            selected: 0x333333,
                            radius: 8,
                        },
                    ),
                    Button::new(true, "Confirm"),
                )]
            }
        }
        let mut host = UiHost::new(DialogApplication { confirmations: 0 }, 320, 200);
        host.request_focus(UiId::from("root/anchor"));
        assert!(host.open_transient(OverlayId::new("dialog"), UiId::from("root/anchor")));
        assert!(
            host.semantic_nodes()
                .iter()
                .any(|node| node.role == Some(SemanticRole::Dialog))
        );
        let confirm = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.name.as_deref() == Some("Confirm"))
            .expect("dialog content shares the semantic frame");
        host.handle_event(UiEvent::FocusNext);
        assert_eq!(host.inspect().keyboard_focus, Some(confirm.id.clone()));
        host.handle_event(UiEvent::FocusNext);
        assert_eq!(host.inspect().keyboard_focus, Some(confirm.id.clone()));
        let activated =
            host.perform_semantic_action(confirm.id, SemanticAction::Invoke(ActionKind::Activate));
        assert!(activated.changed);
        assert_eq!(host.application().confirmations, 1);
        host.handle_event(UiEvent::ControllerBack);
        assert!(host.inspect().open_overlay.is_none());
        assert_eq!(
            host.inspect().keyboard_focus,
            Some(UiId::from("root/anchor"))
        );
    }

    #[test]
    fn public_popover_and_tooltip_use_named_canonical_transient_surfaces() {
        struct PopoverApplication;
        impl Application for PopoverApplication {
            type Message = ();
            fn update(&mut self, (): Self::Message) {}
            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                Button::new((), "Details").id("anchor")
            }
            fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
                vec![
                    super::Popover::new(
                        "details-popover",
                        crate::OverlayAnchor::InvocationTarget(UiId::from("anchor")),
                        "Application details",
                        crate::Size::new(80.0, 40.0),
                        crate::OverlayStyle {
                            background: 0x111111,
                            foreground: 0xffffff,
                            border: 0x888888,
                            selected: 0x333333,
                            radius: 8,
                        },
                        Button::new((), "Close"),
                    )
                    .placement(crate::OverlayPlacement::Before)
                    .direction(crate::ReadingDirection::RightToLeft)
                    .scale(1.5)
                    .into(),
                ]
            }
        }

        let mut host = UiHost::new(PopoverApplication, 320, 200);
        let anchor = host
            .query_unique(&crate::SemanticSelector::Role(SemanticRole::Button))
            .expect("popover anchor");
        let anchor_id = anchor.id.clone();
        host.request_focus(anchor_id.clone());
        let opened =
            host.perform_semantic_action(anchor.id, SemanticAction::Invoke(ActionKind::Activate));
        assert!(opened.changed);
        assert_eq!(
            host.inspect().open_overlay,
            Some(OverlayId::new("details-popover"))
        );
        let popover = host
            .query_unique(&crate::SemanticSelector::RoleAndName {
                role: SemanticRole::Popover,
                name: "Application details".into(),
            })
            .expect("named popover");
        assert_eq!(popover.bounds.size, crate::Size::new(120.0, 60.0));
        assert!(host.accessibility_nodes().iter().any(|node| {
            node.role.as_deref() == Some("popover")
                && node.label.as_deref() == Some("Application details")
        }));
        assert!(
            host.inspect()
                .keyboard_focus
                .as_ref()
                .is_some_and(|id| id.as_str().starts_with("details-popover/content")),
            "FirstItem popovers focus their first interactive content immediately: {:?}",
            host.inspect().keyboard_focus
        );
        host.handle_event(UiEvent::ControllerBack);
        assert!(host.inspect().open_overlay.is_none());
        assert_eq!(host.inspect().keyboard_focus.as_ref(), Some(&anchor_id));

        struct TooltipApplication;
        impl Application for TooltipApplication {
            type Message = ();
            fn update(&mut self, (): Self::Message) {}
            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                Button::new((), "Help").id("anchor")
            }
            fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
                vec![
                    super::Tooltip::new(
                        "help-tooltip",
                        crate::OverlayAnchor::InvocationTarget(UiId::from("anchor")),
                        "Explains this control",
                        crate::Size::new(100.0, 32.0),
                        crate::OverlayStyle {
                            background: 0x111111,
                            foreground: 0xffffff,
                            border: 0x888888,
                            selected: 0x333333,
                            radius: 8,
                        },
                        crate::Text::new("Keyboard shortcut: F1"),
                    )
                    .placement(crate::OverlayPlacement::Above)
                    .into(),
                ]
            }
        }

        let mut host = UiHost::new(TooltipApplication, 320, 200);
        let anchor = host
            .query_unique(&crate::SemanticSelector::Role(SemanticRole::Button))
            .expect("tooltip anchor");
        let anchor_id = anchor.id.clone();
        assert!(host.request_focus(anchor_id.clone()).changed);
        assert!(host.open_transient(OverlayId::new("help-tooltip"), anchor.id));
        assert!(
            host.query_unique(&crate::SemanticSelector::RoleAndName {
                role: SemanticRole::Tooltip,
                name: "Explains this control".into(),
            })
            .is_ok()
        );
        assert!(host.accessibility_nodes().iter().any(|node| {
            node.role.as_deref() == Some("tooltip")
                && node.label.as_deref() == Some("Explains this control")
        }));
        assert_eq!(host.inspect().keyboard_focus.as_ref(), Some(&anchor_id));
    }

    #[test]
    fn ui_host_semantic_actions_update_rebuild_and_report_failures_transactionally() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        let text_field = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.role == Some(SemanticRole::TextField))
            .expect("text field semantics");
        let changed = host.perform_semantic_action(
            text_field.id.clone(),
            SemanticAction::SetValue(SemanticValueInput::Text("semantic".into())),
        );
        assert!(changed.changed);
        assert!(changed.semantic_failures.is_empty());
        assert_eq!(host.application_mut().text, "semantic");
        assert_eq!(host.inspect().frame_generation, 2);

        let accessible = host.perform_accessibility_action(
            text_field.id.clone(),
            SemanticAction::SetValue(SemanticValueInput::Text("accessible".into())),
        );
        assert_eq!(accessible.messages.len(), 1);
        assert_eq!(host.application().text, "accessible");
        assert_eq!(host.inspect().modality, InputModality::Accessibility);

        let activated = host
            .perform_semantic_action(text_field.id, SemanticAction::Invoke(ActionKind::Activate));
        assert!(activated.changed);
        assert!(activated.semantic_failures.is_empty());
        assert_eq!(
            host.inspect().target_mode,
            crate::WidgetTargetMode::TextEditing
        );
        assert_eq!(host.inspect().frame_generation, 4);

        let mut accessibility_host = UiHost::new(InputApplication::default(), 320, 48);
        let accessibility_editor = accessibility_host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.role == Some(SemanticRole::TextField))
            .unwrap();
        let accessibility_activation = accessibility_host.perform_accessibility_action(
            accessibility_editor.id,
            SemanticAction::Invoke(ActionKind::Activate),
        );
        assert!(accessibility_activation.changed);
        assert_eq!(
            accessibility_host.inspect().target_mode,
            crate::WidgetTargetMode::TextEditing
        );
        assert_eq!(
            accessibility_host.inspect().modality,
            InputModality::Accessibility
        );

        let missing = host.perform_semantic_action(
            UiId::from("missing"),
            SemanticAction::Invoke(ActionKind::Activate),
        );
        assert_eq!(
            missing.semantic_failures[0].error,
            SemanticActionError::MissingTarget
        );
    }

    #[test]
    fn first_focus_gain_selects_a_control_and_accepts_text() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        assert!(host.inspect().keyboard_focus.is_none());
        host.step(HostBatch {
            window_focused: Some(true),
            ..HostBatch::default()
        });
        assert!(host.inspect().keyboard_focus.is_some());
        host.handle_event(UiEvent::TextInput("first".into()));
        assert_eq!(host.application().text, "first");
    }

    #[test]
    fn initial_focus_honors_declared_entry_and_preserves_explicit_focus() {
        let mut host = UiHost::new(NavigationApplication, 320, 120);
        host.handle_event(UiEvent::FocusGained);
        assert_eq!(
            host.inspect().keyboard_focus,
            Some(UiId::from("root/scope/last"))
        );
        host.request_focus(UiId::from("root/scope/first"));
        host.handle_event(UiEvent::FocusLost);
        host.handle_event(UiEvent::FocusGained);
        assert_eq!(
            host.inspect().keyboard_focus,
            Some(UiId::from("root/scope/first"))
        );
    }

    #[test]
    fn initial_keyboard_focus_precedes_default_controller_pane_and_skips_disabled_controls() {
        struct PaneApplication;
        impl Application for PaneApplication {
            type Message = ();
            fn update(&mut self, (): ()) {}
            fn view(&self, _: ViewContext) -> impl crate::View<()> {
                Container::new()
                    .id("layout")
                    .child(
                        Button::new((), "Disabled outside")
                            .id("disabled-outside")
                            .enabled(false),
                    )
                    .child(Button::new((), "Outside").id("outside"))
                    .child(
                        Container::new()
                            .id("pane")
                            .navigation_scope(NavigationScope::pane(true))
                            .child(Button::new((), "Disabled").id("disabled").enabled(false))
                            .child(Button::new((), "First").id("first")),
                    )
            }
        }
        let mut host = UiHost::new(PaneApplication, 320, 120);
        host.handle_input(
            &InputEvent::FocusGained {
                order: EventOrder(1),
            },
            None,
        );
        assert_eq!(
            host.inspect().keyboard_focus,
            Some(UiId::from("root/layout/outside"))
        );
    }

    #[test]
    fn an_unfocused_caret_tick_does_not_invalidate_the_window() {
        let mut state = UiStateStore::default();

        assert_eq!(state.toggle_caret(), Invalidation::None);
    }

    #[test]
    fn embedded_controller_dispatch_respects_window_focus() {
        let mut host = UiHost::new(ControllerApplication, 320, 48);
        host.handle_event(crate::UiEvent::FocusGained);
        assert!(
            host.handle_controller_action(ControllerAction::Down)
                .changed
        );
        host.handle_event(crate::UiEvent::FocusLost);
        assert!(
            !host
                .handle_controller_action(ControllerAction::Down)
                .changed
        );
        host.handle_event(crate::UiEvent::FocusGained);
        assert!(
            host.handle_controller_action(ControllerAction::Down)
                .changed
        );
    }

    #[test]
    fn controller_text_entry_does_not_require_slider_adjustment_mode() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        let field = host.semantic_nodes()[0].id.clone();
        assert!(host.request_focus(field).changed);
        assert_eq!(
            host.inspect().target_mode,
            crate::WidgetTargetMode::Navigation
        );
        host.handle_controller_action(ControllerAction::Confirm);
        assert!(host.controller_targets_text_input());
        assert!(host.input_context().text_focused);
        assert!(!host.inspect().controller_editing);
        assert_eq!(
            host.inspect().target_mode,
            crate::WidgetTargetMode::TextEditing
        );
    }

    #[test]
    fn handled_no_op_and_rejected_action_have_explicit_dispositions() {
        let mut host = UiHost::new(ControllerApplication, 320, 48);
        host.handle_event(UiEvent::FocusGained);
        let no_op = host.handle_event(UiEvent::FocusGained);
        assert!(!no_op.changed);
        assert_eq!(no_op.disposition, crate::EventDisposition::Handled);

        let button = host.semantic_nodes()[0].id.clone();
        let rejected =
            host.perform_semantic_action(button, SemanticAction::Invoke(ActionKind::Increment));
        assert_eq!(
            rejected.disposition,
            crate::EventDisposition::Rejected("action unavailable")
        );
        assert_eq!(rejected.semantic_failures.len(), 1);
    }

    #[test]
    fn explicit_shortcut_disposition_controls_widget_fallback() {
        struct ShortcutApplication {
            disposition: crate::EventDisposition,
            activations: usize,
        }

        impl Application for ShortcutApplication {
            type Message = ();

            fn update(&mut self, (): Self::Message) {
                self.activations += 1;
            }

            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                Button::new((), "Activate")
            }

            fn shortcut_outcome(&mut self, _shortcut: Shortcut) -> super::ShortcutOutcome {
                super::ShortcutOutcome {
                    disposition: self.disposition,
                    changed: false,
                }
            }
        }

        for disposition in [
            crate::EventDisposition::Handled,
            crate::EventDisposition::Rejected("disabled"),
        ] {
            let mut host = UiHost::new(
                ShortcutApplication {
                    disposition,
                    activations: 0,
                },
                160,
                48,
            );
            let target = host.semantic_nodes()[0].id.clone();
            host.request_focus(target);
            let outcome = host.handle_input(&key(1, false), None);
            assert_eq!(outcome.disposition, disposition);
            assert!(!outcome.changed);
            assert_eq!(host.application().activations, 0);
        }
    }

    #[test]
    fn adapter_consumption_is_explicitly_handled() {
        struct ConsumingAdapterApplication;

        impl Application for ConsumingAdapterApplication {
            type Message = ();

            fn update(&mut self, (): Self::Message) {}

            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                Button::new((), "Ignored")
            }

            fn adapt_input(
                _host: &mut UiHost<Self>,
                _input: &nickel_input::InputEvent,
            ) -> super::AdapterOutcome {
                super::AdapterOutcome::consumed(false)
            }
        }

        let mut host = UiHost::new(ConsumingAdapterApplication, 160, 48);
        let outcome = host.handle_input(&key(1, false), None);
        assert_eq!(outcome.disposition, crate::EventDisposition::Handled);
        assert!(!outcome.changed);
        assert!(outcome.messages.is_empty());
    }

    #[test]
    fn focused_button_is_not_reported_as_a_text_editor() {
        let mut host = UiHost::new(ControllerApplication, 320, 48);
        let button = host.semantic_nodes()[0].id.clone();

        assert!(host.request_focus(button).changed);
        assert!(!host.input_context().text_focused);
        assert!(!host.controller_targets_text_input());
        assert_eq!(
            host.inspect().target_mode,
            crate::WidgetTargetMode::Navigation
        );
    }

    #[test]
    fn controller_host_event_is_equivalent_to_the_canonical_ui_transition() {
        let mut controller = UiHost::new(ControllerApplication, 320, 48);
        let mut semantic = UiHost::new(ControllerApplication, 320, 48);
        controller.handle_event(crate::UiEvent::FocusGained);
        semantic.handle_event(crate::UiEvent::FocusGained);

        let controller_outcome = controller.step(HostBatch {
            events: vec![HostEvent::Controller(ControllerAction::Down)],
            ..HostBatch::default()
        });
        let semantic_outcome = semantic.step(HostBatch {
            events: vec![HostEvent::Ui(crate::UiEvent::ControllerDown)],
            ..HostBatch::default()
        });

        assert_eq!(controller_outcome.changed, semantic_outcome.changed);
        assert_eq!(
            controller_outcome.invalidation,
            semantic_outcome.invalidation
        );
        assert_eq!(controller.inspect(), semantic.inspect());

        let activation = controller.handle_controller_action(ControllerAction::Confirm);
        assert_eq!(activation.messages.len(), 1);
        assert_eq!(activation.messages[0].type_name, "()");
    }

    #[test]
    fn pointer_retargets_the_current_widget_after_controller_navigation() {
        let mut host = UiHost::new(CrossInputApplication::default(), 320, 200);
        host.handle_event(UiEvent::ControllerDown);
        let a = host
            .inspect()
            .controller_target
            .expect("controller selects A");
        assert!(a.as_str().ends_with("/a"));

        let b = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id.as_str().ends_with("/b"))
            .expect("B is in the production semantic tree");
        let point = crate::Point {
            x: b.bounds.origin.x + b.bounds.size.width / 2.0,
            y: b.bounds.origin.y + b.bounds.size.height / 2.0,
        };
        host.handle_event(UiEvent::PointerPressed(point));
        host.handle_event(UiEvent::PointerReleased(point));

        let inspection = host.inspect();
        assert_eq!(inspection.keyboard_focus.as_ref(), Some(&b.id));
        assert_eq!(inspection.controller_target.as_ref(), Some(&b.id));
        host.handle_event(UiEvent::ControllerActivate);
        assert_eq!(host.application().invoked, vec!["B", "B"]);
    }

    #[test]
    fn normalized_touch_and_direct_pointer_adapters_produce_the_same_host_trace() {
        let mut touch = UiHost::new(ControllerApplication, 160, 48);
        let mut pointer = UiHost::new(ControllerApplication, 160, 48);
        let point = crate::Point { x: 40.0, y: 20.0 };
        let pointer_outcome = pointer.step(HostBatch {
            events: vec![
                HostEvent::Ui(UiEvent::PointerMoved(point)),
                HostEvent::Ui(UiEvent::PointerPressed(point)),
                HostEvent::Ui(UiEvent::PointerReleased(point)),
            ],
            ..HostBatch::default()
        });
        let touch_outcome = touch.step(HostBatch {
            events: vec![
                synthetic_normalized(
                    InputEvent::Touch(TouchEvent::Started {
                        device: DeviceId(4),
                        order: EventOrder(1),
                        contact: TouchId(1),
                        position: Point { x: 40.0, y: 20.0 },
                    }),
                    None,
                ),
                synthetic_normalized(
                    InputEvent::Touch(TouchEvent::Ended {
                        device: DeviceId(4),
                        order: EventOrder(2),
                        contact: TouchId(1),
                        position: Point { x: 40.0, y: 20.0 },
                    }),
                    None,
                ),
            ],
            ..HostBatch::default()
        });
        assert_eq!(touch_outcome.messages, pointer_outcome.messages);
        assert_eq!(touch_outcome.invalidation, pointer_outcome.invalidation);
        assert_eq!(touch.inspect(), pointer.inspect());
    }

    fn primary_button(device: u64, order: u64, edge: KeyEdge, point: Point) -> InputEvent {
        InputEvent::Pointer(PointerEvent::Button {
            device: DeviceId(device),
            order: EventOrder(order),
            button: PointerButton::Primary,
            edge,
            position: Some(point),
        })
    }

    #[test]
    fn normalized_host_ignores_release_from_a_device_that_does_not_own_capture() {
        let mut host = UiHost::new(ControllerApplication, 160, 48);
        let point = Point { x: 40.0, y: 20.0 };

        assert!(
            host.handle_input(&primary_button(1, 1, KeyEdge::Pressed, point), None)
                .messages
                .is_empty()
        );
        let capture = host
            .inspect()
            .pointer_capture
            .expect("the production hit test established capture");

        assert!(
            host.handle_input(&primary_button(2, 2, KeyEdge::Released, point), None)
                .messages
                .is_empty()
        );
        assert_eq!(host.inspect().pointer_capture.as_ref(), Some(&capture));

        assert_eq!(
            host.handle_input(&primary_button(1, 3, KeyEdge::Released, point), None)
                .messages
                .len(),
            1
        );
        assert!(host.inspect().pointer_capture.is_none());
    }

    #[test]
    fn normalized_host_keeps_capture_when_an_unrelated_device_is_removed() {
        let mut host = UiHost::new(ControllerApplication, 160, 48);
        let point = Point { x: 40.0, y: 20.0 };

        host.handle_input(&primary_button(1, 1, KeyEdge::Pressed, point), None);
        let capture = host
            .inspect()
            .pointer_capture
            .expect("the production hit test established capture");
        assert!(
            host.handle_input(
                &InputEvent::DeviceRemoved {
                    device: DeviceId(2),
                    order: EventOrder(2),
                },
                None,
            )
            .messages
            .is_empty()
        );
        assert_eq!(host.inspect().pointer_capture.as_ref(), Some(&capture));

        assert_eq!(
            host.handle_input(&primary_button(1, 3, KeyEdge::Released, point), None)
                .messages
                .len(),
            1
        );
        assert!(host.inspect().pointer_capture.is_none());
    }

    #[test]
    fn normalized_host_distinguishes_equal_contact_numbers_on_distinct_devices() {
        let mut host = UiHost::new(ControllerApplication, 160, 48);
        let position = Point { x: 40.0, y: 20.0 };
        let contact = TouchId(7);

        host.handle_input(
            &InputEvent::Touch(TouchEvent::Started {
                device: DeviceId(1),
                order: EventOrder(1),
                contact,
                position,
            }),
            None,
        );
        let capture = host
            .inspect()
            .pointer_capture
            .expect("the production hit test established capture");

        assert!(
            host.handle_input(
                &InputEvent::Touch(TouchEvent::Ended {
                    device: DeviceId(2),
                    order: EventOrder(2),
                    contact,
                    position,
                }),
                None,
            )
            .messages
            .is_empty()
        );
        assert_eq!(host.inspect().pointer_capture.as_ref(), Some(&capture));

        assert_eq!(
            host.handle_input(
                &InputEvent::Touch(TouchEvent::Ended {
                    device: DeviceId(1),
                    order: EventOrder(3),
                    contact,
                    position,
                }),
                None,
            )
            .messages
            .len(),
            1
        );
        assert!(host.inspect().pointer_capture.is_none());
    }

    #[test]
    fn stationary_touch_arms_and_consumes_a_native_long_press_deadline() {
        let origin = Instant::now();
        let mut host = UiHost::new(ControllerApplication, 160, 48);
        let started = host.step(HostBatch {
            now: Some(origin),
            events: vec![synthetic_normalized(
                InputEvent::Touch(TouchEvent::Started {
                    device: DeviceId(4),
                    order: EventOrder(1),
                    contact: TouchId(1),
                    position: Point { x: 40.0, y: 20.0 },
                }),
                None,
            )],
            ..HostBatch::default()
        });
        assert_eq!(
            started.next_deadline,
            Some(origin + super::TOUCH_LONG_PRESS_DELAY)
        );

        let fired = host.step(HostBatch {
            now: Some(origin + super::TOUCH_LONG_PRESS_DELAY),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        assert!(host.pending_long_press.is_none());
        assert_ne!(fired.invalidation, Invalidation::None);
    }

    #[test]
    fn rejected_touch_start_never_arms_or_fires_long_press() {
        let origin = Instant::now();
        let HostEvent::NormalizedIngress(mut envelope) = synthetic_normalized(
            InputEvent::Touch(TouchEvent::Started {
                device: DeviceId(4),
                order: EventOrder(1),
                contact: TouchId(1),
                position: Point { x: 40.0, y: 20.0 },
            }),
            None,
        ) else {
            unreachable!()
        };
        envelope.source.identity_capability = "untrusted-test".into();
        let mut host = UiHost::new(ControllerApplication, 160, 48);
        host.step(HostBatch {
            now: Some(origin),
            events: vec![HostEvent::NormalizedIngress(envelope)],
            ..HostBatch::default()
        });
        assert!(host.pending_long_press.is_none());

        host.step(HostBatch {
            now: Some(origin + super::TOUCH_LONG_PRESS_DELAY),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        assert!(host.pending_long_press.is_none());
    }

    #[test]
    fn recipient_retirement_cancels_admitted_long_press_before_deadline() {
        let origin = Instant::now();
        let mut host = UiHost::new(ControllerApplication, 160, 48);
        host.step(HostBatch {
            now: Some(origin),
            events: vec![synthetic_normalized(
                InputEvent::Touch(TouchEvent::Started {
                    device: DeviceId(4),
                    order: EventOrder(1),
                    contact: TouchId(1),
                    position: Point { x: 40.0, y: 20.0 },
                }),
                None,
            )],
            ..HostBatch::default()
        });
        assert!(host.pending_long_press.is_some());

        let HostEvent::NormalizedIngress(mut moved) = synthetic_normalized(
            InputEvent::Touch(TouchEvent::Moved {
                device: DeviceId(4),
                order: EventOrder(2),
                contact: TouchId(1),
                position: Point { x: 40.0, y: 20.0 },
            }),
            None,
        ) else {
            unreachable!()
        };
        moved.recipient = NormalizedRecipientBinding {
            lease: 2,
            lifetime: 2,
        };
        moved.host_connection_generation = 2;
        moved.composition_recipient_epoch = Some(2);
        let authority = moved.execution_authority();
        host.step(HostBatch {
            now: Some(origin + super::TOUCH_LONG_PRESS_DELAY),
            events: vec![HostEvent::NormalizedIngress(moved)],
            normalized_authorities: vec![authority],
            ..HostBatch::default()
        });

        assert!(host.pending_long_press.is_none());
    }

    #[test]
    fn admitted_focus_loss_cancels_long_press_at_its_deadline() {
        let origin = Instant::now();
        let mut host = UiHost::new(ControllerApplication, 160, 48);
        host.step(HostBatch {
            now: Some(origin),
            events: vec![synthetic_normalized(
                InputEvent::Touch(TouchEvent::Started {
                    device: DeviceId(4),
                    order: EventOrder(1),
                    contact: TouchId(1),
                    position: Point { x: 40.0, y: 20.0 },
                }),
                None,
            )],
            ..HostBatch::default()
        });
        assert!(host.pending_long_press.is_some());

        host.step(HostBatch {
            now: Some(origin + super::TOUCH_LONG_PRESS_DELAY),
            events: vec![synthetic_normalized(
                InputEvent::FocusLost {
                    order: EventOrder(2),
                },
                None,
            )],
            ..HostBatch::default()
        });

        assert!(host.pending_long_press.is_none());
    }

    #[test]
    fn same_owner_controller_confirm_is_busy_and_touch_end_activates_once() {
        let origin = Instant::now();
        let mut host = UiHost::new(ControllerApplication, 160, 48);
        host.step(HostBatch {
            now: Some(origin),
            events: vec![synthetic_normalized(
                InputEvent::Touch(TouchEvent::Started {
                    device: DeviceId(4),
                    order: EventOrder(1),
                    contact: TouchId(1),
                    position: Point { x: 40.0, y: 20.0 },
                }),
                None,
            )],
            ..HostBatch::default()
        });
        assert!(host.pending_long_press.is_some());
        assert!(host.input_dispatcher.touch_active());

        let confirm = host.step(HostBatch {
            now: Some(origin + Duration::from_millis(100)),
            events: vec![HostEvent::Controller(ControllerAction::Confirm)],
            ..HostBatch::default()
        });
        assert!(confirm.messages.is_empty());
        assert_eq!(
            confirm.disposition,
            crate::EventDisposition::Rejected("input busy")
        );
        assert!(host.pending_long_press.is_some());
        assert!(host.input_dispatcher.touch_active());

        let ended = host.step(HostBatch {
            events: vec![synthetic_normalized(
                InputEvent::Touch(TouchEvent::Ended {
                    device: DeviceId(4),
                    order: EventOrder(2),
                    contact: TouchId(1),
                    position: Point { x: 40.0, y: 20.0 },
                }),
                None,
            )],
            ..HostBatch::default()
        });
        assert_eq!(ended.messages.len(), 1);
    }

    #[test]
    fn admitted_same_owner_busy_has_evidence_without_press_ledger_entry() {
        let mut host = UiHost::new(ControllerApplication, 160, 48);
        host.handle_input(
            &InputEvent::Touch(TouchEvent::Started {
                device: DeviceId(4),
                order: EventOrder(1),
                contact: TouchId(1),
                position: Point { x: 40.0, y: 20.0 },
            }),
            None,
        );
        let binding = ControllerExecutionBinding {
            device_generation: 5,
            edge: nickel_input::KeyEdge::Pressed,
            routing_epoch: 9,
            event_id: 41,
            lease_epoch: 7,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: Some(41),
            surface_generation: Some(10),
            repeat: false,
        };
        let authority = ControllerExecutionAuthority {
            routing_epoch: 9,
            lease_epoch: 7,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: Some(41),
            surface_generation: Some(10),
        };
        let busy = host.step(HostBatch {
            window_focused: Some(true),
            controller_authority: Some(authority),
            events: vec![HostEvent::AdmittedController {
                action: Some(ControllerAction::Confirm),
                binding,
            }],
            ..HostBatch::default()
        });
        assert_eq!(busy.messages.len(), 0);
        assert_eq!(busy.effects.len(), 0);
        assert_eq!(
            busy.controller_executions[0].disposition,
            ControllerExecutionDisposition::RejectedBusy
        );

        let released = host.step(HostBatch {
            controller_authority: Some(authority),
            events: vec![HostEvent::AdmittedController {
                action: Some(ControllerAction::Confirm),
                binding: ControllerExecutionBinding {
                    edge: nickel_input::KeyEdge::Released,
                    ..binding
                },
            }],
            ..HostBatch::default()
        });
        assert_eq!(
            released.controller_executions[0].disposition,
            ControllerExecutionDisposition::RejectedUnpairedRelease
        );
    }

    #[test]
    fn occupied_context_menu_is_busy_and_cancel_consumes_one_gesture() {
        let binding = ControllerExecutionBinding {
            device_generation: 5,
            edge: KeyEdge::Pressed,
            routing_epoch: 9,
            event_id: 41,
            lease_epoch: 7,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: Some(41),
            surface_generation: Some(10),
            repeat: false,
        };
        let authority = ControllerExecutionAuthority {
            routing_epoch: 9,
            lease_epoch: 7,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: Some(41),
            surface_generation: Some(10),
        };
        for (action, expected_press, expected_release) in [
            (
                ControllerAction::ContextMenu,
                ControllerExecutionDisposition::RejectedBusy,
                ControllerExecutionDisposition::RejectedUnpairedRelease,
            ),
            (
                ControllerAction::Cancel,
                ControllerExecutionDisposition::Executed,
                ControllerExecutionDisposition::Executed,
            ),
        ] {
            let mut host = UiHost::new(ControllerApplication, 160, 48);
            host.handle_input(
                &InputEvent::Touch(TouchEvent::Started {
                    device: DeviceId(4),
                    order: EventOrder(1),
                    contact: TouchId(1),
                    position: Point { x: 40.0, y: 20.0 },
                }),
                None,
            );
            let press = host.step(HostBatch {
                window_focused: Some(true),
                controller_authority: Some(authority),
                events: vec![HostEvent::AdmittedController {
                    action: Some(action),
                    binding,
                }],
                ..HostBatch::default()
            });
            assert_eq!(press.messages.len(), 0);
            assert_eq!(press.controller_executions[0].disposition, expected_press);
            assert_eq!(
                host.input_dispatcher.touch_active(),
                action != ControllerAction::Cancel
            );

            let release = host.step(HostBatch {
                controller_authority: Some(authority),
                events: vec![HostEvent::AdmittedController {
                    action: Some(action),
                    binding: ControllerExecutionBinding {
                        edge: KeyEdge::Released,
                        ..binding
                    },
                }],
                ..HostBatch::default()
            });
            assert_eq!(
                release.controller_executions[0].disposition,
                expected_release
            );
        }
    }

    #[test]
    fn reset_fence_precedes_busy_and_replacement_rearms_arbitration() {
        let blocked = ControllerExecutionAuthority {
            routing_epoch: 9,
            lease_epoch: 7,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: None,
            surface_generation: Some(10),
        };
        let replacement = ControllerExecutionAuthority {
            lease_epoch: 8,
            ..blocked
        };
        let mut host = UiHost::new(ControllerApplication, 160, 48);
        host.handle_input(
            &InputEvent::Touch(TouchEvent::Started {
                device: DeviceId(4),
                order: EventOrder(1),
                contact: TouchId(1),
                position: Point { x: 40.0, y: 20.0 },
            }),
            None,
        );
        host.controller_overflow_fence = Some(blocked);
        let binding = |lease_epoch| ControllerExecutionBinding {
            device_generation: 5,
            edge: KeyEdge::Pressed,
            routing_epoch: 9,
            event_id: 41,
            lease_epoch,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: None,
            surface_generation: Some(10),
            repeat: false,
        };
        let fenced = host.step(HostBatch {
            controller_authority: Some(blocked),
            events: vec![HostEvent::AdmittedController {
                action: Some(ControllerAction::Confirm),
                binding: binding(7),
            }],
            ..HostBatch::default()
        });
        assert_eq!(
            fenced.controller_executions[0].disposition,
            ControllerExecutionDisposition::RejectedResetFence
        );
        let busy = host.step(HostBatch {
            controller_authority: Some(replacement),
            events: vec![HostEvent::AdmittedController {
                action: Some(ControllerAction::Confirm),
                binding: binding(8),
            }],
            ..HostBatch::default()
        });
        assert_eq!(
            busy.controller_executions[0].disposition,
            ControllerExecutionDisposition::RejectedBusy
        );
        assert!(host.controller_overflow_fence.is_none());
    }

    #[test]
    fn admitted_same_owner_adjustment_is_busy_without_press_ledger_entry() {
        let mut host = UiHost::new(AdjustmentApplication { value: 0.5 }, 160, 48);
        let slider = host.semantic_nodes()[0].clone();
        let point = Point {
            x: f64::from(slider.bounds.origin.x + slider.bounds.size.width / 2.0),
            y: f64::from(slider.bounds.origin.y + slider.bounds.size.height / 2.0),
        };
        host.handle_input(
            &InputEvent::Pointer(PointerEvent::Button {
                device: DeviceId(4),
                order: EventOrder(1),
                button: PointerButton::Primary,
                edge: KeyEdge::Pressed,
                position: Some(point),
            }),
            None,
        );
        host.state
            .navigation_mut()
            .set_controller_selected(Some(slider.id));
        host.state
            .navigation_mut()
            .set_target_mode(crate::WidgetTargetMode::ValueAdjustment);
        let binding = ControllerExecutionBinding {
            device_generation: 5,
            edge: KeyEdge::Pressed,
            routing_epoch: 9,
            event_id: 41,
            lease_epoch: 7,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: Some(41),
            surface_generation: Some(10),
            repeat: false,
        };
        let authority = ControllerExecutionAuthority {
            routing_epoch: 9,
            lease_epoch: 7,
            connection_generation: 3,
            stream_generation: 2,
            cutoff: Some(41),
            surface_generation: Some(10),
        };

        let busy = host.step(HostBatch {
            window_focused: Some(true),
            controller_authority: Some(authority),
            events: vec![HostEvent::AdmittedController {
                action: Some(ControllerAction::Right),
                binding,
            }],
            ..HostBatch::default()
        });
        assert_eq!(host.application().value, 0.5);
        assert_eq!(busy.messages.len(), 0);
        assert_eq!(busy.effects.len(), 0);
        assert_eq!(
            busy.controller_executions[0].disposition,
            ControllerExecutionDisposition::RejectedBusy
        );

        let released = host.step(HostBatch {
            controller_authority: Some(authority),
            events: vec![HostEvent::AdmittedController {
                action: Some(ControllerAction::Right),
                binding: ControllerExecutionBinding {
                    edge: KeyEdge::Released,
                    ..binding
                },
            }],
            ..HostBatch::default()
        });
        assert_eq!(
            released.controller_executions[0].disposition,
            ControllerExecutionDisposition::RejectedUnpairedRelease
        );
    }

    #[test]
    fn unrelated_or_invalid_events_preserve_active_touch_ownership() {
        let mut host = UiHost::new(ControllerApplication, 160, 48);
        host.step(HostBatch {
            events: vec![synthetic_normalized(
                InputEvent::Touch(TouchEvent::Started {
                    device: DeviceId(4),
                    order: EventOrder(1),
                    contact: TouchId(1),
                    position: Point { x: 40.0, y: 20.0 },
                }),
                None,
            )],
            ..HostBatch::default()
        });

        host.step(HostBatch {
            events: vec![HostEvent::Shortcut(Shortcut::Submit)],
            ..HostBatch::default()
        });
        assert!(host.input_dispatcher.touch_active());
        assert!(host.pending_long_press.is_some());

        let stale = host.step(HostBatch {
            events: vec![HostEvent::Accessibility {
                target: UiId::from("missing"),
                action: SemanticAction::Invoke(ActionKind::Activate),
            }],
            ..HostBatch::default()
        });
        assert!(!stale.semantic_failures.is_empty());
        assert!(host.input_dispatcher.touch_active());
        assert!(host.pending_long_press.is_some());

        host.step(HostBatch {
            events: vec![HostEvent::Normalized {
                input: InputEvent::Text(nickel_input::TextEvent::Commit {
                    device: DeviceId(8),
                    order: EventOrder(2),
                    text: "x".into(),
                }),
                clipboard_text: None,
            }],
            ..HostBatch::default()
        });
        assert!(host.input_dispatcher.touch_active());
    }

    #[test]
    fn raw_ui_different_target_cancels_touch_before_transition() {
        let mut host = UiHost::new(CrossInputApplication::default(), 200, 48);
        let nodes = host.semantic_nodes();
        let first = nodes
            .iter()
            .find(|node| node.name.as_deref() == Some("A"))
            .unwrap();
        let second = nodes
            .iter()
            .find(|node| node.name.as_deref() == Some("B"))
            .unwrap();
        let point = Point {
            x: f64::from(first.bounds.origin.x + first.bounds.size.width / 2.0),
            y: f64::from(first.bounds.origin.y + first.bounds.size.height / 2.0),
        };
        host.handle_input(
            &InputEvent::Touch(TouchEvent::Started {
                device: DeviceId(4),
                order: EventOrder(1),
                contact: TouchId(1),
                position: point,
            }),
            None,
        );
        host.state
            .navigation_mut()
            .set_controller_selected(Some(second.id.clone()));

        let activated = host.step(HostBatch {
            events: vec![HostEvent::Ui(UiEvent::ControllerActivate)],
            ..HostBatch::default()
        });
        assert_eq!(activated.messages.len(), 1);
        assert!(!host.input_dispatcher.touch_active());
        assert!(host.pending_long_press.is_none());

        let ended = host.handle_input(
            &InputEvent::Touch(TouchEvent::Ended {
                device: DeviceId(4),
                order: EventOrder(2),
                contact: TouchId(1),
                position: point,
            }),
            None,
        );
        assert!(ended.messages.is_empty());

        let mut accessibility = UiHost::new(CrossInputApplication::default(), 200, 48);
        let nodes = accessibility.semantic_nodes();
        let first = nodes
            .iter()
            .find(|node| node.name.as_deref() == Some("A"))
            .unwrap();
        let second_id = nodes
            .iter()
            .find(|node| node.name.as_deref() == Some("B"))
            .unwrap()
            .id
            .clone();
        let point = Point {
            x: f64::from(first.bounds.origin.x + first.bounds.size.width / 2.0),
            y: f64::from(first.bounds.origin.y + first.bounds.size.height / 2.0),
        };
        accessibility.handle_input(
            &InputEvent::Touch(TouchEvent::Started {
                device: DeviceId(5),
                order: EventOrder(1),
                contact: TouchId(1),
                position: point,
            }),
            None,
        );
        accessibility.step(HostBatch {
            events: vec![HostEvent::Ui(UiEvent::AccessibilityFocus(
                second_id.clone(),
            ))],
            ..HostBatch::default()
        });
        assert!(!accessibility.input_dispatcher.touch_active());
        assert!(accessibility.pending_long_press.is_none());
        assert_eq!(accessibility.state.focused(), Some(&second_id));
    }

    #[test]
    fn different_target_cancels_mouse_capture_before_its_release_tail() {
        let mut host = UiHost::new(CrossInputApplication::default(), 200, 48);
        let nodes = host.semantic_nodes();
        let first = nodes
            .iter()
            .find(|node| node.name.as_deref() == Some("A"))
            .unwrap();
        let second = nodes
            .iter()
            .find(|node| node.name.as_deref() == Some("B"))
            .unwrap();
        let point = Point {
            x: f64::from(first.bounds.origin.x + first.bounds.size.width / 2.0),
            y: f64::from(first.bounds.origin.y + first.bounds.size.height / 2.0),
        };
        host.handle_input(
            &InputEvent::Pointer(PointerEvent::Button {
                device: DeviceId(4),
                order: EventOrder(1),
                button: PointerButton::Primary,
                edge: KeyEdge::Pressed,
                position: Some(point),
            }),
            None,
        );
        assert_eq!(host.state.captured(), Some(&first.id));
        host.state
            .navigation_mut()
            .set_controller_selected(Some(second.id.clone()));

        let activated = host.step(HostBatch {
            events: vec![HostEvent::Ui(UiEvent::ControllerActivate)],
            ..HostBatch::default()
        });
        assert_eq!(activated.messages.len(), 1);
        assert!(host.state.pressed().is_none());
        assert!(host.state.captured().is_none());

        let released = host.handle_input(
            &InputEvent::Pointer(PointerEvent::Button {
                device: DeviceId(4),
                order: EventOrder(2),
                button: PointerButton::Primary,
                edge: KeyEdge::Released,
                position: Some(point),
            }),
            None,
        );
        assert!(released.messages.is_empty());
        assert_eq!(host.application().invoked, ["B"]);
    }

    #[test]
    fn cancellation_outcome_survives_merge_with_unhandled_followup() {
        let mut host = UiHost::new(ControllerApplication, 160, 48);
        host.handle_input(
            &InputEvent::Pointer(PointerEvent::Button {
                device: DeviceId(4),
                order: EventOrder(1),
                button: PointerButton::Primary,
                edge: KeyEdge::Pressed,
                position: Some(Point { x: 40.0, y: 20.0 }),
            }),
            None,
        );
        let mut outcome = host.arbitrate_touch_ownership();
        outcome.merge(super::HostEventOutcome::default());

        assert!(outcome.changed);
        assert_ne!(outcome.invalidation, Invalidation::None);
        assert_eq!(outcome.disposition, crate::EventDisposition::Handled);
        assert!(host.state.pressed().is_none());
        assert!(host.state.captured().is_none());
    }

    #[test]
    fn pointer_and_keyboard_context_arbitrate_primary_capture() {
        let mut host = UiHost::new(ContextApplication::default(), 200, 48);
        let nodes = host.semantic_nodes();
        let first = nodes
            .iter()
            .find(|node| node.name.as_deref() == Some("A"))
            .unwrap();
        let second = nodes
            .iter()
            .find(|node| node.name.as_deref() == Some("B"))
            .unwrap();
        let first_point = Point {
            x: f64::from(first.bounds.origin.x + first.bounds.size.width / 2.0),
            y: f64::from(first.bounds.origin.y + first.bounds.size.height / 2.0),
        };
        let second_point = crate::Point {
            x: second.bounds.origin.x + second.bounds.size.width / 2.0,
            y: second.bounds.origin.y + second.bounds.size.height / 2.0,
        };
        host.handle_input(
            &InputEvent::Pointer(PointerEvent::Button {
                device: DeviceId(4),
                order: EventOrder(1),
                button: PointerButton::Primary,
                edge: KeyEdge::Pressed,
                position: Some(first_point),
            }),
            None,
        );

        let pointer_context = host.handle_event(UiEvent::PointerContext(second_point));
        assert_eq!(
            pointer_context.disposition,
            crate::EventDisposition::Rejected("input busy")
        );
        assert_eq!(host.state.captured(), Some(&first.id));
        assert!(host.application().invoked.is_empty());

        let keyboard_same = host.handle_event(UiEvent::KeyboardContextMenu);
        assert_eq!(
            keyboard_same.disposition,
            crate::EventDisposition::Rejected("input busy")
        );
        assert_eq!(host.state.captured(), Some(&first.id));

        host.state.set_focus(Some(second.id.clone()));
        let keyboard_other = host.handle_event(UiEvent::KeyboardContextMenu);
        assert_eq!(host.application().invoked, ["B-context"]);
        assert!(host.state.captured().is_none());
        assert!(keyboard_other.changed);
    }

    #[test]
    fn layout_generation_change_retires_long_press_target_attachment() {
        let origin = Instant::now();
        let mut host = UiHost::new(ControllerApplication, 160, 48);
        host.step(HostBatch {
            now: Some(origin),
            events: vec![synthetic_normalized(
                InputEvent::Touch(TouchEvent::Started {
                    device: DeviceId(4),
                    order: EventOrder(1),
                    contact: TouchId(1),
                    position: Point { x: 40.0, y: 20.0 },
                }),
                None,
            )],
            ..HostBatch::default()
        });
        assert!(host.pending_long_press.is_some());

        host.resize(200, 48);
        assert!(host.pending_long_press.is_none());
        let deadline = host.step(HostBatch {
            now: Some(origin + super::TOUCH_LONG_PRESS_DELAY),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        assert!(deadline.messages.is_empty());
    }

    #[test]
    fn resize_and_modality_are_supplied_before_the_declarative_rebuild() {
        let mut host = UiHost::new(ResponsiveApplication, 320, 48);
        assert_eq!(host.semantic_nodes()[0].name.as_deref(), Some("Wide"));

        host.resize(160, 48);
        assert_eq!(host.inspect().frame_generation, 2);
        assert_eq!(host.semantic_nodes()[0].name.as_deref(), Some("Narrow"));

        let outcome = host.handle_controller_action(ControllerAction::Down);
        assert!(outcome.changed);
        assert_eq!(host.inspect().modality, crate::InputModality::Controller);
        assert_eq!(host.semantic_nodes()[0].name.as_deref(), Some("Controller"));

        let target = host.semantic_nodes()[0].id.clone();
        let generation = host.inspect().frame_generation;
        assert!(!host.request_focus(target.clone()).changed);
        let inspection = host.inspect();
        assert_eq!(inspection.keyboard_focus, Some(target));
        assert_eq!(inspection.modality, crate::InputModality::Controller);
        assert_eq!(inspection.frame_generation, generation);
    }

    #[test]
    fn guide_action_is_reported_globally_without_entering_application_dispatch() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        let outcome = host.handle_controller_action(ControllerAction::Launcher);

        assert!(!outcome.changed);
        assert_eq!(outcome.global_actions, [GlobalAction::ToggleLauncher]);
        assert_eq!(host.inspect().frame_generation, 1);
    }

    #[test]
    fn embedded_host_dispatches_normalized_text_ime_and_submit_once() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        assert!(host.handle_input(&focus_event(), None).changed);
        assert!(host.input_context().text_focused);
        assert!(
            !host
                .handle_input(
                    &InputEvent::FocusGained {
                        order: EventOrder(2),
                    },
                    None,
                )
                .changed
        );
        assert!(host.input_context().text_focused);

        let preedit = InputEvent::Text(TextEvent::Preedit {
            device: DeviceId(1),
            order: EventOrder(3),
            text: "世".into(),
            selection: Some((0, 3)),
        });
        assert!(host.handle_input(&preedit, None).changed);
        assert!(host.application_mut().text.is_empty());

        let commit = InputEvent::Text(TextEvent::Commit {
            device: DeviceId(1),
            order: EventOrder(4),
            text: "世界".into(),
        });
        assert!(host.handle_input(&commit, None).changed);
        assert_eq!(host.application_mut().text, "世界");

        assert!(host.handle_input(&key(5, false), None).changed);
        assert_eq!(host.application_mut().submits, 1);
        assert!(!host.handle_input(&key(6, true), None).changed);
        assert_eq!(host.application_mut().submits, 1);
    }

    #[test]
    fn embedded_host_owns_one_clipboard_command_path() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        host.handle_input(&focus_event(), None);
        host.handle_input(
            &InputEvent::Text(TextEvent::Commit {
                device: DeviceId(1),
                order: EventOrder(2),
                text: "copy me".into(),
            }),
            None,
        );
        host.handle_input(&command_key(3, KeyCode::KeyA, "a"), None);

        let copied = host.handle_input(&command_key(4, KeyCode::KeyC, "c"), None);
        assert_eq!(copied.clipboard_text.as_deref(), Some("copy me"));
        assert_eq!(host.application_mut().text, "copy me");

        let cut = host.handle_input(&command_key(5, KeyCode::KeyX, "x"), None);
        assert_eq!(cut.clipboard_text.as_deref(), Some("copy me"));
        assert!(host.application_mut().text.is_empty());

        assert!(
            host.handle_input(&command_key(6, KeyCode::KeyV, "v"), Some("pasted"))
                .changed
        );
        assert_eq!(host.application_mut().text, "pasted");
    }

    #[test]
    fn bounded_semantic_dispatch_preserves_adapter_clipboard_limit() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        host.handle_input(&focus_event(), None);
        host.handle_event(UiEvent::PointerCancelled);
        host.step(HostBatch {
            clipboard_text_limit: Some(4),
            ..Default::default()
        });
        let outcome = host
            .perform_bounded_semantic_action(
                host.resolved_frame_generation(),
                0,
                crate::SemanticAction::SetValue(crate::SemanticValueInput::Text(
                    "preserve me".into(),
                )),
                64,
                4096,
                None,
            )
            .unwrap();
        assert!(outcome.semantic_failures.is_empty());
        host.dispatch_ui_event(UiEvent::TextSelectAll);
        let denied = host.dispatch_ui_event(UiEvent::TextCut);
        assert_eq!(host.application().text, "preserve me");
        assert!(denied.clipboard_text.is_none());
        assert!(host.state.clipboard_rejected);
        host.step(HostBatch {
            clipboard_text_limit: Some(11),
            ..Default::default()
        });
        let allowed = host.dispatch_ui_event(UiEvent::TextCut);
        assert_eq!(allowed.clipboard_text.as_deref(), Some("preserve me"));
        assert!(host.application().text.is_empty());
    }

    #[test]
    fn bounded_clipboard_rejects_cut_before_edit_or_ownership_change() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        host.handle_input(&focus_event(), None);
        host.handle_event(UiEvent::TextInput("preserve me".into()));
        host.handle_event(UiEvent::TextSelectAll);
        let denied = host.step(HostBatch {
            clipboard_text_limit: Some(4),
            events: vec![HostEvent::Ui(UiEvent::TextCut)],
            ..Default::default()
        });
        assert_eq!(host.application().text, "preserve me");
        assert!(denied.clipboard_text.is_none());
        assert_eq!(denied.failures.len(), 1);
        let accepted = host.step(HostBatch {
            clipboard_text_limit: Some(11),
            events: vec![HostEvent::Ui(UiEvent::TextCut)],
            ..Default::default()
        });
        assert!(host.application().text.is_empty());
        assert_eq!(accepted.clipboard_text.as_deref(), Some("preserve me"));

        host.handle_event(UiEvent::TextInput("large".into()));
        host.handle_event(UiEvent::TextSelectAll);
        let mixed = host.step(HostBatch {
            clipboard_text_limit: Some(1),
            events: vec![
                HostEvent::Ui(UiEvent::TextCut),
                HostEvent::Ui(UiEvent::TextMoveEnd {
                    extend_selection: false,
                }),
                HostEvent::Ui(UiEvent::TextMoveLeft {
                    extend_selection: true,
                }),
                HostEvent::Ui(UiEvent::TextCut),
            ],
            ..Default::default()
        });
        assert_eq!(host.application().text, "larg");
        assert_eq!(mixed.clipboard_text.as_deref(), Some("e"));
        assert_eq!(mixed.failures.len(), 1);
        let reverse = host.step(HostBatch {
            clipboard_text_limit: Some(1),
            events: vec![
                HostEvent::Ui(UiEvent::TextMoveEnd {
                    extend_selection: false,
                }),
                HostEvent::Ui(UiEvent::TextMoveLeft {
                    extend_selection: true,
                }),
                HostEvent::Ui(UiEvent::TextCut),
                HostEvent::Ui(UiEvent::TextSelectAll),
                HostEvent::Ui(UiEvent::TextCut),
            ],
            ..Default::default()
        });
        assert_eq!(host.application().text, "lar");
        assert_eq!(reverse.clipboard_text.as_deref(), Some("g"));
        assert_eq!(reverse.failures.len(), 1);

        let mut secure = UiHost::new(
            SecureInputApplication {
                text: String::new(),
            },
            320,
            48,
        );
        secure.handle_input(&focus_event(), None);
        secure.handle_event(UiEvent::TextInput("secret".into()));
        secure.handle_event(UiEvent::TextSelectAll);
        for event in [UiEvent::TextCopy, UiEvent::TextCut] {
            let outcome = secure.step(HostBatch {
                clipboard_text_limit: Some(0),
                events: vec![HostEvent::Ui(event)],
                ..Default::default()
            });
            assert!(outcome.clipboard_text.is_none());
            assert!(outcome.failures.is_empty());
            assert_eq!(secure.application().text, "secret");
        }
    }

    #[test]
    fn consumed_select_all_text_does_not_reach_single_multiline_or_secure_fields() {
        fn exercise<A: Application<Message = Message>>(
            host: &mut UiHost<A>,
            text: impl for<'a> FnOnce(&'a mut A) -> &'a str,
        ) {
            host.handle_input(&focus_event(), None);
            assert!(host.input_context().text_focused);
            host.handle_input(&command_key(2, KeyCode::KeyA, "a"), None);
            let leaked = host.handle_input(
                &InputEvent::Text(TextEvent::Commit {
                    device: DeviceId(1),
                    order: EventOrder(2),
                    text: "a".into(),
                }),
                None,
            );
            assert!(!leaked.changed);
            assert_eq!(text(host.application_mut()), "unchanged");
        }

        let mut single = UiHost::new(
            InputApplication {
                text: "unchanged".into(),
                submits: 0,
            },
            320,
            48,
        );
        exercise(&mut single, |application| &application.text);

        let mut multiline = UiHost::new(
            MultilineInputApplication {
                text: "unchanged".into(),
            },
            320,
            96,
        );
        exercise(&mut multiline, |application| &application.text);

        let mut secure = UiHost::new(
            SecureInputApplication {
                text: "unchanged".into(),
            },
            320,
            48,
        );
        exercise(&mut secure, |application| &application.text);
    }

    #[derive(Clone, Copy, Debug)]
    enum ReplayPath {
        Headless,
        StandaloneAdapter,
        EmbeddedAdapter,
    }

    #[derive(Clone, Debug, PartialEq)]
    struct ReplayProof {
        messages: Vec<MessageEvidence>,
        semantics: Vec<crate::SemanticNodeSnapshot>,
        paint: Vec<crate::PaintCommand>,
        accessibility: Vec<crate::AccessibilityNode>,
        inspection: super::HostInspection,
        deadline_offset: Option<Duration>,
    }

    struct ReplayApplication {
        text: String,
    }

    impl Application for ReplayApplication {
        type Message = Message;

        fn update(&mut self, message: Self::Message) {
            match message {
                Message::Changed(text) => self.text = text,
            }
        }

        fn message_evidence(&self, message: &Self::Message) -> MessageEvidence {
            let Message::Changed(text) = message;
            MessageEvidence {
                type_name: "replay.text.changed",
                label: Some(text.clone()),
            }
        }

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            TextField::on_change(&self.text, Message::Changed)
        }

        fn poll_interval(&self) -> Option<Duration> {
            Some(Duration::from_millis(40))
        }
    }

    fn replay_adapter_path(path: ReplayPath) -> ReplayProof {
        let origin = Instant::now();
        let mut host = UiHost::new_at(
            ReplayApplication {
                text: String::new(),
            },
            320,
            48,
            origin,
        );
        let editor = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.role == Some(SemanticRole::TextField))
            .expect("editor semantic target")
            .id;
        let events = vec![
            synthetic_normalized(focus_event(), None),
            // Complete the focus click before replaying independent controller and
            // accessibility intents; an unmatched press correctly owns this editor
            // and makes a same-target semantic value change busy.
            synthetic_normalized(
                InputEvent::Pointer(PointerEvent::Button {
                    device: DeviceId(2),
                    order: EventOrder(2),
                    button: PointerButton::Primary,
                    edge: KeyEdge::Released,
                    position: Some(Point { x: 4.0, y: 4.0 }),
                }),
                None,
            ),
            synthetic_normalized(
                InputEvent::Text(TextEvent::Preedit {
                    device: DeviceId(1),
                    order: EventOrder(2),
                    text: "世".into(),
                    selection: Some((0, 3)),
                }),
                None,
            ),
            synthetic_normalized(
                InputEvent::Text(TextEvent::Commit {
                    device: DeviceId(1),
                    order: EventOrder(3),
                    text: "world".into(),
                }),
                None,
            ),
            synthetic_normalized(command_key(4, KeyCode::KeyA, "a"), None),
            synthetic_normalized(command_key(5, KeyCode::KeyC, "c"), None),
            synthetic_normalized(command_key(6, KeyCode::KeyX, "x"), None),
            synthetic_normalized(command_key(7, KeyCode::KeyV, "v"), Some("pasted".into())),
            synthetic_normalized(
                InputEvent::Touch(TouchEvent::Started {
                    device: DeviceId(4),
                    order: EventOrder(8),
                    contact: TouchId(1),
                    position: Point { x: 8.0, y: 8.0 },
                }),
                None,
            ),
            synthetic_normalized(
                InputEvent::Touch(TouchEvent::Ended {
                    device: DeviceId(4),
                    order: EventOrder(9),
                    contact: TouchId(1),
                    position: Point { x: 8.0, y: 8.0 },
                }),
                None,
            ),
            HostEvent::Controller(ControllerAction::Down),
            HostEvent::Accessibility {
                target: editor,
                action: SemanticAction::SetValue(SemanticValueInput::Text("accessible".into())),
            },
            HostEvent::Shortcut(Shortcut::Submit),
            HostEvent::Controller(ControllerAction::Launcher),
            HostEvent::Ui(UiEvent::FocusLost),
            HostEvent::Poll,
        ];
        let batch = HostBatch {
            now: Some(origin + Duration::from_millis(45)),
            surface_size: Some((480, 72)),
            scale_factor: Some(1.5),
            window_focused: Some(true),
            events,
            ..HostBatch::default()
        };

        // These are deliberately transport-only paths. Standalone windows,
        // embedded shell surfaces, and headless scenarios all surrender the
        // normalized batch to the same host transition authority.
        let outcome = match path {
            ReplayPath::Headless => host.step(batch),
            ReplayPath::StandaloneAdapter => {
                let adapter_batch = batch;
                host.step(adapter_batch)
            }
            ReplayPath::EmbeddedAdapter => {
                let embedded_batch = batch;
                host.step(embedded_batch)
            }
        };
        ReplayProof {
            messages: outcome.messages,
            semantics: host.semantic_nodes(),
            paint: host.commands().to_vec(),
            accessibility: host.accessibility_nodes().to_vec(),
            inspection: host.inspect(),
            deadline_offset: outcome
                .next_deadline
                .map(|deadline| deadline.duration_since(origin)),
        }
    }

    #[test]
    fn one_normalized_trace_is_identical_across_all_host_adapter_paths() {
        let headless = replay_adapter_path(ReplayPath::Headless);
        let standalone = replay_adapter_path(ReplayPath::StandaloneAdapter);
        let embedded = replay_adapter_path(ReplayPath::EmbeddedAdapter);

        assert_eq!(standalone, headless);
        assert_eq!(embedded, headless);
        assert_eq!(headless.deadline_offset, Some(Duration::from_millis(85)));
        assert_eq!(headless.inspection.scale_factor, 1.5);
        assert_eq!(headless.inspection.modality, InputModality::Accessibility);
        assert!(!headless.inspection.window_focused);
        assert!(headless.messages.iter().any(|message| {
            message.type_name == "replay.text.changed"
                && message.label.as_deref() == Some("accessible")
        }));
    }

    #[test]
    fn adapter_faults_are_typed_and_do_not_prevent_overlay_dismissal_or_mutate_state() {
        struct FaultApplication;
        impl Application for FaultApplication {
            type Message = ();
            fn update(&mut self, (): Self::Message) {}
            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                Button::new((), "Anchor").id("anchor")
            }
            fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
                vec![FrameOverlay::surface(
                    crate::TransientSurface::dialog(
                        "fault-dialog",
                        crate::OverlayAnchor::Node(UiId::from("anchor")),
                        crate::Size::new(120.0, 60.0),
                        crate::OverlayStyle {
                            background: 0x111111,
                            foreground: 0xffffff,
                            border: 0x888888,
                            selected: 0x333333,
                            radius: 8,
                        },
                    ),
                    Button::new((), "Dismiss"),
                )]
            }
        }

        let stages = [
            HostFailureStage::Presenter,
            HostFailureStage::Clipboard,
            HostFailureStage::Ime,
            HostFailureStage::Accessibility,
            HostFailureStage::Controller,
        ];
        for stage in stages {
            let mut host = UiHost::new(FaultApplication, 320, 200);
            assert!(host.open_transient(OverlayId::new("fault-dialog"), UiId::from("root/anchor")));
            let before = host.inspect();
            let failure = HostFailure {
                surface: "fault-test".into(),
                stage,
                optional: stage != HostFailureStage::Presenter,
                detail: format!("injected {stage:?} failure"),
            };
            let outcome = host.step(HostBatch {
                failures: vec![failure.clone()],
                events: vec![HostEvent::Ui(UiEvent::ControllerBack)],
                ..HostBatch::default()
            });

            assert_eq!(outcome.failures, [failure]);
            assert!(before.open_overlay.is_some());
            assert!(host.inspect().open_overlay.is_none());
            assert_eq!(host.inspect().window_focused, before.window_focused);
            assert!(
                host.accessibility_nodes()
                    .iter()
                    .any(|node| node.label.as_deref() == Some("Anchor"))
            );
        }
    }

    #[test]
    fn declarative_frame_layers_are_applied_on_initial_resolve_and_rebuild() {
        #[derive(Clone, PartialEq)]
        enum Message {
            Anchor,
            Context,
            Choose,
            ChooseSecond,
        }

        struct LayerApplication;

        impl Application for LayerApplication {
            type Message = Message;

            fn update(&mut self, _message: Self::Message) {}

            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                crate::Container::new()
                    .id("anchor")
                    .message(Message::Anchor)
                    .context_message(Message::Context)
                    .width(120.0)
                    .height(40.0)
            }

            fn frame_overlays(
                &self,
                _context: ViewContext,
            ) -> Vec<super::FrameOverlay<Self::Message>> {
                vec![super::FrameOverlay::Menu(
                    crate::OverlayMenu::new(
                        "context",
                        crate::OverlayAnchor::Point {
                            invocation_target: crate::UiId::from("anchor"),
                            point: crate::Point { x: 72.0, y: 24.0 },
                        },
                    )
                    .semantic_style(crate::OverlayStyle {
                        background: 0x202630,
                        foreground: 0xe8edf4,
                        border: 0x444444,
                        selected: 0x334455,
                        radius: 0,
                    })
                    .item(crate::OverlayMenuItem::action(
                        "choose",
                        "Choose",
                        Message::Choose,
                    ))
                    .item(crate::OverlayMenuItem::action(
                        "choose-second",
                        "Choose second",
                        Message::ChooseSecond,
                    )),
                )]
            }
        }

        let mut host = UiHost::new(LayerApplication, 320, 200);
        assert!(host.inspect().overlay_failures.is_empty());
        let anchor = host
            .semantic_targets_for_message(&Message::Anchor)
            .into_iter()
            .next()
            .unwrap()
            .id;
        host.handle_event(crate::UiEvent::FocusGained);
        host.handle_event(crate::UiEvent::ControllerDown);
        host.perform_semantic_action(
            anchor,
            crate::SemanticAction::Invoke(crate::ActionKind::ContextMenu),
        );
        assert!(host.inspect().overlay_failures.is_empty());
        let menu = host
            .query(&crate::SemanticSelector::Role(crate::SemanticRole::Menu))
            .pop()
            .expect("menu layer must survive the rebuild caused by opening it");
        assert_eq!(menu.bounds.origin, crate::Point { x: 72.0, y: 24.0 });
        assert_eq!(
            host.query(&crate::SemanticSelector::Role(
                crate::SemanticRole::MenuItem
            ))
            .len(),
            2
        );
        let first = host.inspect().controller_target.unwrap();
        let selected_background =
            crate::focused_surface_with_foreground(0x202630, 0x334455, 0xe8edf4);
        let first_selected_rect = host
            .commands()
            .iter()
            .find_map(|command| match command {
                crate::backend::PaintCommand::RoundedFill { rect, color, .. }
                    if *color == selected_background =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .expect("first menu row must own the selected paint");
        host.handle_event(crate::UiEvent::ControllerDown);
        let second = host.inspect().controller_target.unwrap();
        assert_ne!(first, second);
        let items = host.query(&crate::SemanticSelector::Role(
            crate::SemanticRole::MenuItem,
        ));
        assert!(
            items
                .iter()
                .any(|item| item.id == second && item.focused && item.controller_selected)
        );
        assert!(
            items
                .iter()
                .any(|item| item.id == first && !item.focused && !item.controller_selected)
        );
        let second_selected_rect = host
            .commands()
            .iter()
            .find_map(|command| match command {
                crate::backend::PaintCommand::RoundedFill { rect, color, .. }
                    if *color == selected_background =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .expect("second menu row must own the selected paint after rebuild");
        assert_ne!(first_selected_rect, second_selected_rect);
    }

    #[test]
    fn invalid_frame_layer_anchor_is_retained_as_typed_host_evidence() {
        struct InvalidLayerApplication;

        impl Application for InvalidLayerApplication {
            type Message = ();

            fn update(&mut self, (): Self::Message) {}

            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                crate::Container::new().id("present")
            }

            fn frame_overlays(
                &self,
                _context: ViewContext,
            ) -> Vec<super::FrameOverlay<Self::Message>> {
                vec![super::FrameOverlay::Menu(crate::OverlayMenu::new(
                    "broken",
                    crate::OverlayAnchor::Node(crate::UiId::from("missing")),
                ))]
            }
        }

        let host = UiHost::new(InvalidLayerApplication, 320, 200);
        assert_eq!(host.inspect().overlay_failures.len(), 1);
        assert_eq!(
            host.inspect().overlay_failures[0].error,
            crate::SemanticActionError::MissingTarget
        );
    }

    #[test]
    fn inspection_and_accessibility_retain_navigation_depth_and_actions_across_rebuild() {
        struct NestedApplication {
            activated: bool,
        }

        impl Application for NestedApplication {
            type Message = ();

            fn update(&mut self, (): Self::Message) {
                self.activated = true;
            }

            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                crate::Container::new()
                    .id("outer")
                    .navigation_scope(crate::NavigationScope::group())
                    .child(
                        crate::Container::new()
                            .id("inner")
                            .navigation_scope(crate::NavigationScope::group())
                            .child(
                                crate::Button::new(
                                    (),
                                    if self.activated {
                                        "Activated"
                                    } else {
                                        "Activate"
                                    },
                                )
                                .id("action"),
                            ),
                    )
            }
        }

        let mut host = UiHost::new(NestedApplication { activated: false }, 320, 200);
        host.handle_event(UiEvent::ControllerDown);
        host.handle_event(UiEvent::ControllerActivate);
        assert_eq!(host.inspect().navigation_depth, 1);
        host.handle_event(UiEvent::ControllerActivate);
        assert_eq!(host.inspect().navigation_depth, 2);
        assert_eq!(
            host.inspect().available_semantic_actions,
            [crate::ActionKind::Activate]
        );
        let selected = host
            .inspect()
            .controller_target
            .expect("nested action selected");
        let accessible = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.id == selected)
            .expect("selected action remains accessibility-visible");
        assert_eq!(accessible.navigation_depth, 2);
        assert_eq!(accessible.actions, [crate::ActionKind::Activate]);

        host.handle_event(UiEvent::ControllerActivate);
        assert!(host.application().activated);
        assert_eq!(host.inspect().navigation_depth, 2);
        assert_eq!(host.inspect().controller_target.as_ref(), Some(&selected));
        assert_eq!(
            host.inspect().available_semantic_actions,
            [crate::ActionKind::Activate]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn outbound_file_drag_uses_encoded_crlf_uri_list() {
        use std::os::unix::ffi::OsStringExt;

        let paths = [
            std::path::PathBuf::from("/tmp/a file.txt"),
            std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/nonutf8-\xff".to_vec())),
        ];
        assert_eq!(
            super::file_uri_list(&paths),
            b"file:///tmp/a%20file.txt\r\nfile:///tmp/nonutf8-%FF\r\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn pending_session_lease_adopts_later_exact_poll_grant() {
        use nickel_session_protocol::{
            ControllerExecutionOracle,
            controller_broker::{ConnectionGeneration, LeaseEpoch, StreamGeneration},
        };

        let connection = ConnectionGeneration(7);
        let lease = LeaseEpoch(11);
        let oracle = ControllerExecutionOracle {
            routing_epoch: 3,
            lease_epoch: lease,
            connection_generation: connection,
            stream_generation: StreamGeneration(2),
            surface_generation: Some(9),
        };
        let (adopted, role) =
            super::adopt_pending_controller_lease(connection, Some(lease), Some(oracle)).unwrap();
        assert_eq!(adopted, lease);
        assert_eq!(role, super::ControllerRoleLease::session(7, 11));
        assert!(
            super::adopt_pending_controller_lease(
                ConnectionGeneration(8),
                Some(lease),
                Some(oracle)
            )
            .is_none()
        );
    }

    #[cfg(unix)]
    #[test]
    fn session_controller_revocation_drops_queued_old_lease_before_acknowledging() {
        use nickel_session_protocol::{
            ClientEnvelope, ControllerActionMessage, ControllerEnvelopePayload,
            ControllerFamilyMessage, ControllerHostRequest, ControllerHostResponse, InputState,
            Request, ServerEnvelope, ServerMessage,
            client::AsyncControllerConnection,
            controller_broker::{
                BrokerMessage, ConnectionGeneration, Delivery, EventId, HostId, LeaseEpoch,
                StreamGeneration,
            },
        };
        use std::os::unix::net::UnixDatagram;

        let root = std::env::temp_dir().join(format!(
            "nickel-ui-controller-host-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let server_path = root.join("server");
        let server = UnixDatagram::bind(&server_path).unwrap();
        let (acknowledged, acknowledgement) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut buffer = vec![0; nickel_session_protocol::MAX_FRAME_BYTES];
            for step in 0..5 {
                let (length, peer) = server.recv_from(&mut buffer).unwrap();
                let envelope: ClientEnvelope =
                    nickel_session_protocol::decode(&buffer[..length]).unwrap();
                let response = match (step, envelope.request) {
                    (0, Request::ControllerHost(ControllerHostRequest::Attach)) => {
                        ControllerHostResponse::Attached {
                            host: HostId(9),
                            connection_generation: ConnectionGeneration(2),
                        }
                    }
                    (1, Request::ControllerHost(ControllerHostRequest::RequestLease { .. })) => {
                        ControllerHostResponse::LeaseGranted {
                            lease_epoch: LeaseEpoch(4),
                        }
                    }
                    (2, Request::ControllerHost(ControllerHostRequest::Poll { .. })) => {
                        // The action was queued for generation 10, but the compositor has already
                        // retired that surface and reports generation 11.
                        ControllerHostResponse::Messages {
                            lease_epoch: Some(LeaseEpoch(4)),
                            execution_oracle: Some(
                                nickel_session_protocol::ControllerExecutionOracle {
                                    routing_epoch: 8,
                                    lease_epoch: LeaseEpoch(4),
                                    connection_generation: ConnectionGeneration(2),
                                    stream_generation: StreamGeneration(1),
                                    surface_generation: Some(11),
                                },
                            ),
                            messages: vec![BrokerMessage::Deliver(Delivery {
                                event_id: EventId(6),
                                connection_generation: ConnectionGeneration(2),
                                lease_epoch: LeaseEpoch(4),
                                stream_generation: StreamGeneration(1),
                                payload: ControllerEnvelopePayload {
                                    device_generation: 3,
                                    action: Some(ControllerActionMessage::Confirm),
                                    edge: InputState::Pressed,
                                    repeat: false,
                                    family: ControllerFamilyMessage::Xbox,
                                    routing_epoch: 8,
                                    evidence: None,
                                    surface_generation: Some(10),
                                },
                            })],
                        }
                    }
                    (3, Request::ControllerHost(ControllerHostRequest::Poll { .. })) => {
                        ControllerHostResponse::Messages {
                            lease_epoch: None,
                            execution_oracle: None,
                            messages: vec![
                                BrokerMessage::Deliver(Delivery {
                                    event_id: EventId(7),
                                    connection_generation: ConnectionGeneration(2),
                                    lease_epoch: LeaseEpoch(4),
                                    stream_generation: StreamGeneration(1),
                                    payload: ControllerEnvelopePayload {
                                        device_generation: 3,
                                        action: Some(ControllerActionMessage::Confirm),
                                        edge: InputState::Pressed,
                                        repeat: false,
                                        family: ControllerFamilyMessage::Xbox,
                                        routing_epoch: 8,
                                        evidence: None,
                                        surface_generation: Some(10),
                                    },
                                }),
                                BrokerMessage::Revoke {
                                    connection_generation: ConnectionGeneration(2),
                                    lease_epoch: LeaseEpoch(4),
                                    cutoff: EventId(7),
                                },
                            ],
                        }
                    }
                    (
                        4,
                        Request::ControllerHost(ControllerHostRequest::AcknowledgeQuiescence {
                            connection_generation: ConnectionGeneration(2),
                            lease_epoch: LeaseEpoch(4),
                            cutoff: EventId(7),
                        }),
                    ) => ControllerHostResponse::LeaseFailed,
                    _ => panic!("unexpected host controller request at step {step}"),
                };
                server
                    .send_to(
                        &nickel_session_protocol::encode(&ServerEnvelope {
                            request_id: envelope.request_id,
                            message: ServerMessage::ControllerHost(response),
                        })
                        .unwrap(),
                        peer.as_pathname().unwrap(),
                    )
                    .unwrap();
                if step == 4 {
                    acknowledged.send(()).unwrap();
                }
            }
        });
        let connection = AsyncControllerConnection::begin_to(
            &server_path,
            &root,
            "secret".into(),
            Duration::from_secs(1),
        )
        .unwrap();
        let mut source = SessionControllerSource::Connecting { connection };
        let mut ack_seen = false;
        for _ in 0..100 {
            assert!(source.poll_actions().is_empty());
            if acknowledgement.try_recv().is_ok() {
                ack_seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(ack_seen, "revocation acknowledgement reached transport");
        for _ in 0..100 {
            assert!(source.poll_actions().is_empty());
            if matches!(source, SessionControllerSource::Retrying { .. }) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(matches!(source, SessionControllerSource::Retrying { .. }));
        drop(source);
        worker.join().unwrap();
        std::fs::remove_file(server_path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
