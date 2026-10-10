//! Nickel session discovery, nonblocking transport and authenticated controller leases.
use std::time::{Duration, Instant};
use twinkle::{
    ControllerAction, ControllerExecutionAuthority, ControllerExecutionBinding, ControllerFamily,
    ControllerSource,
};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ControllerRoleLease {
    owner_generation: u64,
    generation: u64,
}
impl ControllerRoleLease {
    const fn session(owner_generation: u64, generation: u64) -> Self {
        Self {
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

#[cfg(any(unix, windows))]
enum SessionControllerSource {
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

    pub fn discover() -> ControllerSource {
        use nickel_session_protocol::client::AsyncControllerConnection;
        match AsyncControllerConnection::begin_from_environment(Duration::from_millis(100)) {
            Ok(None) => ControllerSource::Standalone,
            Ok(Some(connection)) => {
                ControllerSource::Hosted(Box::new(Self::Connecting { connection }))
            }
            Err(error) => {
                tracing::warn!(%error, "advertised session controller connection failed closed");
                ControllerSource::Hosted(Box::new(Self::retrying()))
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
                            let reset = messages.iter().find_map(|message| {
                                let BrokerMessage::StreamReset { through, .. } = message else {
                                    return None;
                                };
                                Some(*through)
                            });
                            if reset.is_some() || revocation.is_some() {
                                let acknowledgement = lease.zip(reset).or(revocation);
                                lease = None;
                                role_lease = None;
                                let Some((lease_epoch, cutoff)) = acknowledgement else {
                                    *self = Self::retrying();
                                    return Vec::new();
                                };
                                Some(ControllerHostRequest::AcknowledgeQuiescence {
                                    connection_generation,
                                    lease_epoch,
                                    cutoff,
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
                                                    twinkle_input::KeyEdge::Pressed
                                                }
                                                InputState::Released => {
                                                    twinkle_input::KeyEdge::Released
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
        Launcher => ControllerAction::HostMenu,
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

impl twinkle::HostedControllerSource for SessionControllerSource {
    fn connected(&self) -> bool {
        matches!(self, Self::Connecting { .. } | Self::Attached { .. })
    }
    fn poll_actions(&mut self) -> Vec<twinkle::HostedControllerDelivery> {
        Self::poll_actions(self)
    }
    fn report_execution_overflow(&mut self, binding: ControllerExecutionBinding) {
        Self::report_execution_overflow(self, binding);
    }
    fn relinquish(&mut self) {
        Self::relinquish(self);
    }
}

pub fn controller_source() -> ControllerSource {
    SessionControllerSource::discover()
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn session_controller_reset_drops_queued_old_lease_before_acknowledging() {
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
            "twinkle-controller-host-{}-{:x}",
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
                                BrokerMessage::StreamReset {
                                    stream_generation: StreamGeneration(2),
                                    through: EventId(7),
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
