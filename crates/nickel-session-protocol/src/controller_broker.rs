//! Deterministic controller ownership and delivery sequencing.
//!
//! This module owns no sockets or device readers. Session transports attach authenticated hosts,
//! enqueue the returned messages on their existing loops, and feed acknowledgements back here.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

pub const DEFAULT_CONTROLLER_QUEUE_LIMIT: usize = 256;
pub const DEFAULT_TRANSFER_DEADLINE_MS: u64 = 750;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HostId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionGeneration(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseEpoch(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamGeneration(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery<T> {
    pub event_id: EventId,
    pub connection_generation: ConnectionGeneration,
    pub lease_epoch: LeaseEpoch,
    pub stream_generation: StreamGeneration,
    pub payload: T,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "message", rename_all = "snake_case")]
pub enum BrokerMessage<T> {
    Deliver(Delivery<T>),
    Revoke {
        connection_generation: ConnectionGeneration,
        lease_epoch: LeaseEpoch,
        cutoff: EventId,
    },
    /// Drop queued work and held/repeat state. Delivery remains disabled until native neutral is
    /// observed and a new lease is explicitly granted.
    StreamReset {
        stream_generation: StreamGeneration,
        through: EventId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    pub host: HostId,
    pub connection_generation: ConnectionGeneration,
    pub epoch: LeaseEpoch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IngressDisposition {
    Delivered { event_id: EventId, lease: Lease },
    RejectedNoOwner { event_id: EventId },
    RejectedTransfer { event_id: EventId },
    RejectedResetBarrier { event_id: EventId },
    OverflowReset { event_id: EventId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferStatus {
    Pending {
        requested_lease: LeaseEpoch,
        cutoff: EventId,
    },
    Granted(Lease),
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Transfer {
    from: Lease,
    to: HostId,
    to_connection: ConnectionGeneration,
    requested_lease: LeaseEpoch,
    cutoff: EventId,
    deadline_ms: u64,
    quiescent: bool,
}

#[derive(Debug)]
struct Host<T> {
    connection: ConnectionGeneration,
    outbox: VecDeque<BrokerMessage<T>>,
}

#[derive(Debug)]
pub struct ControllerBroker<T> {
    hosts: BTreeMap<HostId, Host<T>>,
    next_connection: u64,
    next_event: u64,
    next_lease: u64,
    stream_generation: StreamGeneration,
    queue_limit: usize,
    neutral: bool,
    reset_barrier: bool,
    active: Option<Lease>,
    transfer: Option<Transfer>,
}

impl<T> ControllerBroker<T> {
    pub fn new(queue_limit: usize) -> Self {
        assert!(
            queue_limit >= 1,
            "controller queue must retain a lifecycle barrier"
        );
        Self {
            hosts: BTreeMap::new(),
            next_connection: 0,
            next_event: 0,
            next_lease: 0,
            stream_generation: StreamGeneration(1),
            queue_limit,
            neutral: true,
            reset_barrier: false,
            active: None,
            transfer: None,
        }
    }

    pub fn attach(&mut self, host: HostId) -> ConnectionGeneration {
        self.next_connection = self.next_connection.saturating_add(1);
        let generation = ConnectionGeneration(self.next_connection);
        let replaced = self.hosts.insert(
            host,
            Host {
                connection: generation,
                outbox: VecDeque::new(),
            },
        );
        if replaced.is_some()
            && (self.active.is_some_and(|lease| lease.host == host)
                || self
                    .transfer
                    .is_some_and(|transfer| transfer.from.host == host || transfer.to == host))
        {
            self.install_reset(EventId(self.next_event));
        }
        generation
    }

    /// Removes a transport connection without treating disconnection as execution quiescence.
    pub fn detach(&mut self, host: HostId, connection: ConnectionGeneration) {
        if !self.connection_matches(host, connection) {
            return;
        }
        self.hosts.remove(&host);
        if self.active.is_some_and(|lease| lease.host == host) {
            self.install_reset(EventId(self.next_event));
        }
    }

    pub fn grant(&mut self, host: HostId, connection: ConnectionGeneration) -> Option<Lease> {
        if self.active.is_some() || self.transfer.is_some() || self.reset_barrier || !self.neutral {
            return None;
        }
        self.connection_matches(host, connection)
            .then(|| self.install_lease(host, connection))
    }

    pub fn begin_transfer(
        &mut self,
        to: HostId,
        connection: ConnectionGeneration,
        now_ms: u64,
        timeout_ms: u64,
    ) -> TransferStatus {
        let Some(from) = self.active.take() else {
            return TransferStatus::Failed;
        };
        if self.transfer.is_some() || !self.connection_matches(to, connection) || timeout_ms == 0 {
            self.active = Some(from);
            return TransferStatus::Failed;
        }
        let cutoff = EventId(self.next_event);
        let requested_lease = self.allocate_lease_epoch();
        self.push_lifecycle(
            from.host,
            BrokerMessage::Revoke {
                connection_generation: from.connection_generation,
                lease_epoch: from.epoch,
                cutoff,
            },
        );
        self.transfer = Some(Transfer {
            from,
            to,
            to_connection: connection,
            requested_lease,
            cutoff,
            deadline_ms: now_ms.saturating_add(timeout_ms),
            quiescent: false,
        });
        TransferStatus::Pending {
            requested_lease,
            cutoff,
        }
    }

    pub fn acknowledge_quiescence(
        &mut self,
        host: HostId,
        connection: ConnectionGeneration,
        lease: LeaseEpoch,
        cutoff: EventId,
    ) -> TransferStatus {
        let Some(mut transfer) = self.transfer else {
            return TransferStatus::Failed;
        };
        if transfer.from.host != host
            || transfer.from.connection_generation != connection
            || transfer.from.epoch != lease
            || transfer.cutoff != cutoff
        {
            return TransferStatus::Pending {
                requested_lease: transfer.requested_lease,
                cutoff: transfer.cutoff,
            };
        }
        transfer.quiescent = true;
        self.transfer = Some(transfer);
        self.try_finish_transfer()
    }

    /// Records independently verified executor termination. A transport disconnect alone must not
    /// call this method.
    pub fn acknowledge_verified_termination(
        &mut self,
        host: HostId,
        connection: ConnectionGeneration,
    ) -> TransferStatus {
        let Some(mut transfer) = self.transfer else {
            return TransferStatus::Failed;
        };
        if transfer.from.host == host && transfer.from.connection_generation == connection {
            transfer.quiescent = true;
            self.transfer = Some(transfer);
        }
        self.try_finish_transfer()
    }

    pub fn set_neutral(&mut self, neutral: bool) -> Option<Lease> {
        self.neutral = neutral;
        if neutral {
            self.reset_barrier = false;
            if let TransferStatus::Granted(lease) = self.try_finish_transfer() {
                return Some(lease);
            }
        }
        None
    }

    pub fn expire_transfer(&mut self, now_ms: u64) -> TransferStatus {
        let Some(transfer) = self.transfer else {
            return TransferStatus::Failed;
        };
        if now_ms < transfer.deadline_ms {
            return TransferStatus::Pending {
                requested_lease: transfer.requested_lease,
                cutoff: transfer.cutoff,
            };
        }
        self.transfer = None;
        self.active = None;
        self.reset_barrier = true;
        TransferStatus::Failed
    }

    pub fn ingest(&mut self, payload: T) -> IngressDisposition {
        self.next_event = self.next_event.saturating_add(1);
        let event_id = EventId(self.next_event);
        if self.transfer.is_some() {
            return IngressDisposition::RejectedTransfer { event_id };
        }
        if self.reset_barrier || !self.neutral {
            return IngressDisposition::RejectedResetBarrier { event_id };
        }
        let Some(lease) = self.active else {
            return IngressDisposition::RejectedNoOwner { event_id };
        };
        let delivery = BrokerMessage::Deliver(Delivery {
            event_id,
            connection_generation: lease.connection_generation,
            lease_epoch: lease.epoch,
            stream_generation: self.stream_generation,
            payload,
        });
        let Some(host) = self.hosts.get_mut(&lease.host) else {
            self.install_reset(event_id);
            return IngressDisposition::OverflowReset { event_id };
        };
        if host.outbox.len() >= self.queue_limit {
            self.install_reset(event_id);
            return IngressDisposition::OverflowReset { event_id };
        }
        host.outbox.push_back(delivery);
        IngressDisposition::Delivered { event_id, lease }
    }

    pub fn drain(
        &mut self,
        host: HostId,
        connection: ConnectionGeneration,
    ) -> Vec<BrokerMessage<T>> {
        self.hosts
            .get_mut(&host)
            .filter(|state| state.connection == connection)
            .map(|state| state.outbox.drain(..).collect())
            .unwrap_or_default()
    }

    pub fn active_lease(&self) -> Option<Lease> {
        self.active
    }

    fn try_finish_transfer(&mut self) -> TransferStatus {
        let Some(transfer) = self.transfer else {
            return TransferStatus::Failed;
        };
        if !transfer.quiescent || !self.neutral || self.reset_barrier {
            return TransferStatus::Pending {
                requested_lease: transfer.requested_lease,
                cutoff: transfer.cutoff,
            };
        }
        if !self.connection_matches(transfer.to, transfer.to_connection) {
            self.transfer = None;
            return TransferStatus::Failed;
        }
        let lease = Lease {
            host: transfer.to,
            connection_generation: transfer.to_connection,
            epoch: transfer.requested_lease,
        };
        self.transfer = None;
        self.active = Some(lease);
        TransferStatus::Granted(lease)
    }

    fn connection_matches(&self, host: HostId, generation: ConnectionGeneration) -> bool {
        self.hosts
            .get(&host)
            .is_some_and(|state| state.connection == generation)
    }

    fn allocate_lease_epoch(&mut self) -> LeaseEpoch {
        self.next_lease = self.next_lease.saturating_add(1);
        LeaseEpoch(self.next_lease)
    }

    fn install_lease(&mut self, host: HostId, connection: ConnectionGeneration) -> Lease {
        let lease = Lease {
            host,
            connection_generation: connection,
            epoch: self.allocate_lease_epoch(),
        };
        self.active = Some(lease);
        lease
    }

    fn push_lifecycle(&mut self, host: HostId, message: BrokerMessage<T>) {
        if let Some(host) = self.hosts.get_mut(&host) {
            if host.outbox.len() >= self.queue_limit {
                host.outbox.clear();
            }
            host.outbox.push_back(message);
        }
    }

    fn install_reset(&mut self, through: EventId) {
        self.active = None;
        self.transfer = None;
        self.neutral = false;
        self.reset_barrier = true;
        self.stream_generation.0 = self.stream_generation.0.saturating_add(1);
        let generation = self.stream_generation;
        for host in self.hosts.values_mut() {
            host.outbox.clear();
            host.outbox.push_back(BrokerMessage::StreamReset {
                stream_generation: generation,
                through,
            });
        }
    }
}

impl<T> Default for ControllerBroker<T> {
    fn default() -> Self {
        Self::new(DEFAULT_CONTROLLER_QUEUE_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_rejects_intervening_events_and_waits_for_quiescence_and_neutral() {
        let mut broker = ControllerBroker::new(8);
        let a = broker.attach(HostId(1));
        let b = broker.attach(HostId(2));
        let old = broker.grant(HostId(1), a).unwrap();
        broker.set_neutral(false);
        let pending = broker.begin_transfer(HostId(2), b, 100, 50);
        let TransferStatus::Pending {
            requested_lease,
            cutoff,
        } = pending
        else {
            panic!()
        };
        assert!(matches!(
            broker.ingest("held"),
            IngressDisposition::RejectedTransfer { .. }
        ));
        assert_eq!(
            broker.acknowledge_quiescence(HostId(1), a, old.epoch, cutoff),
            pending
        );
        assert_eq!(broker.set_neutral(true).unwrap().epoch, requested_lease);
        assert!(
            matches!(broker.ingest("fresh"), IngressDisposition::Delivered { lease, .. } if lease.host == HostId(2))
        );
    }

    #[test]
    fn stale_ack_cannot_grant_and_timeout_grants_neither_host() {
        let mut broker = ControllerBroker::<()>::new(4);
        let a = broker.attach(HostId(1));
        let b = broker.attach(HostId(2));
        let old = broker.grant(HostId(1), a).unwrap();
        let TransferStatus::Pending { cutoff, .. } = broker.begin_transfer(HostId(2), b, 10, 5)
        else {
            panic!()
        };
        assert!(matches!(
            broker.acknowledge_quiescence(
                HostId(1),
                ConnectionGeneration(a.0 + 99),
                old.epoch,
                cutoff
            ),
            TransferStatus::Pending { .. }
        ));
        assert_eq!(broker.expire_transfer(15), TransferStatus::Failed);
        assert_eq!(broker.active_lease(), None);
        assert_eq!(
            broker.acknowledge_quiescence(HostId(1), a, old.epoch, cutoff),
            TransferStatus::Failed
        );
        assert!(matches!(
            broker.ingest(()),
            IngressDisposition::RejectedResetBarrier { .. }
        ));
    }

    #[test]
    fn queue_overflow_replaces_edges_with_non_droppable_reset() {
        let mut broker = ControllerBroker::new(2);
        let connection = broker.attach(HostId(1));
        broker.grant(HostId(1), connection).unwrap();
        assert!(matches!(
            broker.ingest(1),
            IngressDisposition::Delivered { .. }
        ));
        assert!(matches!(
            broker.ingest(2),
            IngressDisposition::Delivered { .. }
        ));
        assert!(matches!(
            broker.ingest(3),
            IngressDisposition::OverflowReset { .. }
        ));
        assert_eq!(broker.active_lease(), None);
        assert!(matches!(
            broker.drain(HostId(1), connection).as_slice(),
            [BrokerMessage::StreamReset {
                through: EventId(3),
                ..
            }]
        ));
        assert!(broker.grant(HostId(1), connection).is_none());
        assert!(broker.set_neutral(true).is_none());
        assert!(broker.grant(HostId(1), connection).is_some());
    }

    #[test]
    fn reconnect_generation_fences_old_drain_and_grant() {
        let mut broker = ControllerBroker::<()>::new(4);
        let stale = broker.attach(HostId(1));
        broker.grant(HostId(1), stale).unwrap();
        let current = broker.attach(HostId(1));
        assert!(broker.grant(HostId(1), stale).is_none());
        assert!(broker.drain(HostId(1), stale).is_empty());
        assert!(broker.grant(HostId(1), current).is_none());
        broker.set_neutral(true);
        assert!(broker.grant(HostId(1), current).is_some());
    }

    #[test]
    fn delivery_message_retains_wire_execution_binding() {
        let message = BrokerMessage::Deliver(Delivery {
            event_id: EventId(12),
            connection_generation: ConnectionGeneration(4),
            lease_epoch: LeaseEpoch(7),
            stream_generation: StreamGeneration(2),
            payload: "release",
        });
        let encoded = serde_json::to_string(&message).unwrap();
        assert_eq!(
            serde_json::from_str::<BrokerMessage<&str>>(&encoded).unwrap(),
            message
        );
    }
}
