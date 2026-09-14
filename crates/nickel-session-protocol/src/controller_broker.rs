//! Deterministic controller ownership and delivery sequencing.
//!
//! This module owns no sockets or device readers. Session transports attach authenticated hosts,
//! enqueue the returned messages on their existing loops, and feed acknowledgements back here.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

pub const DEFAULT_CONTROLLER_QUEUE_LIMIT: usize = 256;
pub const DEFAULT_CONTROLLER_HOST_LIMIT: usize = 64;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PoisonedPredecessor {
    lease: Lease,
    cutoff: EventId,
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
    host_limit: usize,
    exhausted: bool,
    neutral: bool,
    reset_barrier: bool,
    active: Option<Lease>,
    transfer: Option<Transfer>,
    poisoned_predecessor: Option<PoisonedPredecessor>,
}

impl<T> ControllerBroker<T> {
    pub fn new(queue_limit: usize) -> Self {
        Self::with_limits(queue_limit, DEFAULT_CONTROLLER_HOST_LIMIT)
    }

    pub fn with_limits(queue_limit: usize, host_limit: usize) -> Self {
        assert!(
            queue_limit >= 1,
            "controller queue must retain a lifecycle barrier"
        );
        assert!(host_limit >= 1, "controller host registry must be bounded");
        Self {
            hosts: BTreeMap::new(),
            next_connection: 0,
            next_event: 0,
            next_lease: 0,
            stream_generation: StreamGeneration(1),
            queue_limit,
            host_limit,
            exhausted: false,
            neutral: true,
            reset_barrier: false,
            active: None,
            transfer: None,
            poisoned_predecessor: None,
        }
    }

    pub fn attach(&mut self, host: HostId) -> ConnectionGeneration {
        if self.exhausted {
            return ConnectionGeneration(0);
        }
        let Some(next_connection) = self.next_connection.checked_add(1) else {
            self.fail_closed();
            return ConnectionGeneration(0);
        };
        if !self.hosts.contains_key(&host) && self.hosts.len() >= self.host_limit {
            let Some(evicted) = self
                .hosts
                .keys()
                .copied()
                .find(|candidate| !self.host_is_authority_protected(*candidate))
            else {
                return ConnectionGeneration(0);
            };
            self.hosts.remove(&evicted);
        }
        self.next_connection = next_connection;
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
            self.poison_executor(EventId(self.next_event));
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
        if self.active.is_some_and(|lease| lease.host == host)
            || self
                .transfer
                .is_some_and(|transfer| transfer.from.host == host)
        {
            self.poison_executor(EventId(self.next_event));
            self.install_reset(EventId(self.next_event));
        }
    }

    /// Removes a host after an authenticated orderly execution boundary. Transport loss must use
    /// `detach`, which deliberately poisons active admission instead.
    pub fn relinquish(&mut self, host: HostId, connection: ConnectionGeneration) -> TransferStatus {
        if !self.connection_matches(host, connection) {
            return TransferStatus::Failed;
        }
        if self.transfer.is_some_and(|transfer| {
            transfer.from.host == host && transfer.from.connection_generation == connection
        }) {
            let status = self.acknowledge_verified_termination(host, connection);
            self.hosts.remove(&host);
            return status;
        }
        self.hosts.remove(&host);
        if self
            .active
            .is_some_and(|lease| lease.host == host && lease.connection_generation == connection)
        {
            self.install_reset(EventId(self.next_event));
        }
        TransferStatus::Failed
    }

    pub fn grant(&mut self, host: HostId, connection: ConnectionGeneration) -> Option<Lease> {
        if self.exhausted
            || self.active.is_some()
            || self.transfer.is_some()
            || self.poisoned_predecessor.is_some()
            || self.reset_barrier
            || !self.neutral
        {
            return None;
        }
        if !self.connection_matches(host, connection) {
            return None;
        }
        let lease = self.install_lease(host, connection);
        if lease.is_none() {
            self.fail_closed();
        }
        lease
    }

    pub fn begin_transfer(
        &mut self,
        to: HostId,
        connection: ConnectionGeneration,
        now_ms: u64,
        timeout_ms: u64,
    ) -> TransferStatus {
        if self.exhausted {
            return TransferStatus::Failed;
        }
        let Some(from) = self.active.take() else {
            return TransferStatus::Failed;
        };
        if self.transfer.is_some() || !self.connection_matches(to, connection) || timeout_ms == 0 {
            self.active = Some(from);
            return TransferStatus::Failed;
        }
        let cutoff = EventId(self.next_event);
        let Some(requested_lease) = self.allocate_lease_epoch() else {
            self.active = Some(from);
            self.fail_closed();
            return TransferStatus::Failed;
        };
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

    /// Cancels every pending external ownership identity at a security boundary. An unacknowledged
    /// predecessor remains poisoned; the abandoned requested lease can never be granted or
    /// revived by a late acknowledgement.
    pub fn security_takeover(
        &mut self,
        internal: HostId,
        connection: ConnectionGeneration,
        now_ms: u64,
        timeout_ms: u64,
    ) -> TransferStatus {
        if let Some(transfer) = self.transfer {
            if transfer.to == internal && transfer.to_connection == connection {
                return TransferStatus::Pending {
                    requested_lease: transfer.requested_lease,
                    cutoff: transfer.cutoff,
                };
            }
            if !transfer.quiescent {
                self.poisoned_predecessor = Some(PoisonedPredecessor {
                    lease: transfer.from,
                    cutoff: transfer.cutoff,
                });
            }
            self.install_reset(transfer.cutoff);
            return TransferStatus::Failed;
        }
        if self.active.is_some_and(|lease| lease.host != internal) {
            return self.begin_transfer(internal, connection, now_ms, timeout_ms);
        }
        TransferStatus::Failed
    }

    pub fn acknowledge_quiescence(
        &mut self,
        host: HostId,
        connection: ConnectionGeneration,
        lease: LeaseEpoch,
        cutoff: EventId,
    ) -> TransferStatus {
        let Some(mut transfer) = self.transfer else {
            if self.poisoned_predecessor.is_some_and(|poison| {
                poison.lease.host == host
                    && poison.lease.connection_generation == connection
                    && poison.lease.epoch == lease
                    && poison.cutoff == cutoff
            }) {
                self.clear_poison();
            }
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
            if self.poisoned_predecessor.is_some_and(|poison| {
                poison.lease.host == host && poison.lease.connection_generation == connection
            }) {
                self.clear_poison();
            }
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
        if neutral && self.poisoned_predecessor.is_none() {
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
        self.poisoned_predecessor = Some(PoisonedPredecessor {
            lease: transfer.from,
            cutoff: transfer.cutoff,
        });
        self.transfer = None;
        self.active = None;
        self.reset_barrier = true;
        TransferStatus::Failed
    }

    pub fn ingest(&mut self, payload: T) -> IngressDisposition {
        if self.exhausted {
            return IngressDisposition::RejectedResetBarrier {
                event_id: EventId(self.next_event),
            };
        }
        let Some(next_event) = self.next_event.checked_add(1) else {
            self.fail_closed();
            return IngressDisposition::RejectedResetBarrier {
                event_id: EventId(self.next_event),
            };
        };
        self.next_event = next_event;
        let event_id = EventId(self.next_event);
        if self.transfer.is_some() {
            return IngressDisposition::RejectedTransfer { event_id };
        }
        if self.reset_barrier {
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

    pub fn stream_generation(&self) -> StreamGeneration {
        self.stream_generation
    }

    pub fn is_attached(&self, host: HostId, connection: ConnectionGeneration) -> bool {
        self.connection_matches(host, connection)
    }

    pub fn exhaust(&mut self) {
        self.fail_closed();
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

    fn allocate_lease_epoch(&mut self) -> Option<LeaseEpoch> {
        self.next_lease = self.next_lease.checked_add(1)?;
        Some(LeaseEpoch(self.next_lease))
    }

    fn install_lease(&mut self, host: HostId, connection: ConnectionGeneration) -> Option<Lease> {
        let lease = Lease {
            host,
            connection_generation: connection,
            epoch: self.allocate_lease_epoch()?,
        };
        self.active = Some(lease);
        Some(lease)
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
        let Some(stream_generation) = self.stream_generation.0.checked_add(1) else {
            self.stream_generation = StreamGeneration(u64::MAX);
            self.fail_closed();
            return;
        };
        self.stream_generation = StreamGeneration(stream_generation);
        let generation = self.stream_generation;
        for host in self.hosts.values_mut() {
            host.outbox.clear();
            host.outbox.push_back(BrokerMessage::StreamReset {
                stream_generation: generation,
                through,
            });
        }
    }

    fn poison_executor(&mut self, cutoff: EventId) {
        if let Some(transfer) = self.transfer {
            self.poisoned_predecessor = Some(PoisonedPredecessor {
                lease: transfer.from,
                cutoff: transfer.cutoff,
            });
        } else if let Some(lease) = self.active {
            self.poisoned_predecessor = Some(PoisonedPredecessor { lease, cutoff });
        }
    }

    fn clear_poison(&mut self) {
        self.poisoned_predecessor = None;
        if self.neutral {
            self.reset_barrier = false;
        }
    }

    fn host_is_authority_protected(&self, host: HostId) -> bool {
        self.active.is_some_and(|lease| lease.host == host)
            || self
                .transfer
                .is_some_and(|transfer| transfer.from.host == host || transfer.to == host)
            || self
                .poisoned_predecessor
                .is_some_and(|poison| poison.lease.host == host)
    }

    fn fail_closed(&mut self) {
        self.exhausted = true;
        self.active = None;
        self.transfer = None;
        self.neutral = false;
        self.reset_barrier = true;
        let generation = self.stream_generation;
        let through = EventId(self.next_event);
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
    fn active_lease_retains_release_delivery_while_stream_is_not_neutral() {
        let mut broker = ControllerBroker::new(4);
        let connection = broker.attach(HostId(1));
        broker.grant(HostId(1), connection).unwrap();
        broker.set_neutral(false);
        assert!(matches!(
            broker.ingest("release"),
            IngressDisposition::Delivered { .. }
        ));
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
        assert!(matches!(
            broker.ingest(()),
            IngressDisposition::RejectedResetBarrier { .. }
        ));
        broker.set_neutral(true);
        assert!(broker.grant(HostId(2), b).is_none());
        assert_eq!(
            broker.acknowledge_quiescence(HostId(1), a, old.epoch, cutoff),
            TransferStatus::Failed
        );
        assert!(broker.grant(HostId(2), b).is_some());
    }

    #[test]
    fn detached_executor_stays_poisoned_across_neutral_until_verified_dead() {
        let mut broker = ControllerBroker::<()>::new(4);
        let old_connection = broker.attach(HostId(1));
        let successor = broker.attach(HostId(2));
        broker.grant(HostId(1), old_connection).unwrap();

        broker.detach(HostId(1), old_connection);
        broker.set_neutral(true);
        assert!(broker.grant(HostId(2), successor).is_none());
        assert_eq!(
            broker.acknowledge_verified_termination(HostId(1), old_connection),
            TransferStatus::Failed
        );
        assert!(broker.grant(HostId(2), successor).is_some());
    }

    #[test]
    fn orderly_relinquish_rearms_after_neutral_without_crash_poison() {
        let mut broker = ControllerBroker::<()>::new(4);
        let old_connection = broker.attach(HostId(1));
        let successor = broker.attach(HostId(2));
        broker.grant(HostId(1), old_connection).unwrap();

        assert_eq!(
            broker.relinquish(HostId(1), old_connection),
            TransferStatus::Failed
        );
        assert!(broker.grant(HostId(2), successor).is_none());
        broker.set_neutral(true);
        assert!(broker.grant(HostId(2), successor).is_some());
    }

    #[test]
    fn security_transfer_publishes_cutoff_before_rejecting_new_routing() {
        let mut broker = ControllerBroker::new(4);
        let external = broker.attach(HostId(1));
        let protected = broker.attach(HostId(0));
        let lease = broker.grant(HostId(1), external).unwrap();
        assert!(matches!(
            broker.ingest("before"),
            IngressDisposition::Delivered { .. }
        ));
        let TransferStatus::Pending { cutoff, .. } =
            broker.begin_transfer(HostId(0), protected, 0, 10)
        else {
            panic!("security transfer must revoke the external executor")
        };
        assert_eq!(cutoff, EventId(1));
        assert!(matches!(
            broker.drain(HostId(1), external).as_slice(),
            [BrokerMessage::Deliver(_), BrokerMessage::Revoke {
                connection_generation,
                lease_epoch,
                cutoff: EventId(1),
            }] if *connection_generation == external && *lease_epoch == lease.epoch
        ));
        assert!(matches!(
            broker.ingest("protected"),
            IngressDisposition::RejectedTransfer { .. }
        ));
    }

    #[test]
    fn security_takeover_retires_pending_external_lease_and_late_ack() {
        let mut broker = ControllerBroker::new(4);
        let internal = broker.attach(HostId(0));
        let external = broker.attach(HostId(1));
        let old = broker.grant(HostId(0), internal).unwrap();
        let TransferStatus::Pending {
            requested_lease,
            cutoff,
        } = broker.begin_transfer(HostId(1), external, 0, 10)
        else {
            panic!("external transfer must be pending")
        };

        assert_eq!(
            broker.security_takeover(HostId(0), internal, 1, 10),
            TransferStatus::Failed
        );
        assert_eq!(broker.active_lease(), None);
        assert!(matches!(
            broker.ingest("late"),
            IngressDisposition::RejectedResetBarrier { .. }
        ));
        assert_eq!(
            broker.acknowledge_quiescence(HostId(0), internal, old.epoch, cutoff),
            TransferStatus::Failed
        );
        assert!(broker.active_lease().is_none());
        broker.set_neutral(true);
        let protected = broker.grant(HostId(0), internal).unwrap();
        assert_ne!(protected.epoch, requested_lease);
        assert!(broker.grant(HostId(1), external).is_none());
    }

    #[test]
    fn reconnect_does_not_let_a_new_generation_clear_the_old_poison() {
        let mut broker = ControllerBroker::<()>::new(4);
        let old_connection = broker.attach(HostId(1));
        let old_lease = broker.grant(HostId(1), old_connection).unwrap();
        broker.detach(HostId(1), old_connection);
        let new_connection = broker.attach(HostId(1));
        broker.set_neutral(true);

        assert_eq!(
            broker.acknowledge_verified_termination(HostId(1), new_connection),
            TransferStatus::Failed
        );
        assert!(broker.grant(HostId(1), new_connection).is_none());
        assert_eq!(
            broker.acknowledge_quiescence(HostId(1), old_connection, old_lease.epoch, EventId(0)),
            TransferStatus::Failed
        );
        assert!(broker.grant(HostId(1), new_connection).is_some());
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
        assert!(broker.grant(HostId(1), current).is_none());
        assert_eq!(
            broker.acknowledge_verified_termination(HostId(1), stale),
            TransferStatus::Failed
        );
        assert!(broker.grant(HostId(1), current).is_some());
    }

    #[test]
    fn host_registry_evicts_only_idle_non_authority_entries() {
        let mut broker = ControllerBroker::<()>::with_limits(4, 2);
        let protected = broker.attach(HostId(1));
        let idle = broker.attach(HostId(2));
        broker.grant(HostId(1), protected).unwrap();

        let replacement = broker.attach(HostId(3));
        assert_ne!(replacement, ConnectionGeneration(0));
        assert_eq!(broker.hosts.len(), 2);
        assert!(!broker.connection_matches(HostId(2), idle));
        assert!(broker.connection_matches(HostId(1), protected));
    }

    #[test]
    fn full_authority_protected_registry_rejects_new_attachment() {
        let mut broker = ControllerBroker::<()>::with_limits(4, 2);
        let active = broker.attach(HostId(1));
        let target = broker.attach(HostId(2));
        broker.grant(HostId(1), active).unwrap();
        assert!(matches!(
            broker.begin_transfer(HostId(2), target, 0, 10),
            TransferStatus::Pending { .. }
        ));

        assert_eq!(broker.attach(HostId(3)), ConnectionGeneration(0));
        assert_eq!(broker.hosts.len(), 2);
    }

    #[test]
    fn identity_exhaustion_fails_closed_without_reusing_maximum() {
        let mut connection_broker = ControllerBroker::<()>::new(2);
        connection_broker.next_connection = u64::MAX;
        assert_eq!(connection_broker.attach(HostId(1)), ConnectionGeneration(0));
        assert!(connection_broker.exhausted);

        let mut event_broker = ControllerBroker::new(2);
        let connection = event_broker.attach(HostId(1));
        event_broker.grant(HostId(1), connection).unwrap();
        event_broker.next_event = u64::MAX;
        assert!(matches!(
            event_broker.ingest(()),
            IngressDisposition::RejectedResetBarrier {
                event_id: EventId(u64::MAX)
            }
        ));
        assert_eq!(event_broker.active_lease(), None);

        let mut lease_broker = ControllerBroker::<()>::new(2);
        let connection = lease_broker.attach(HostId(1));
        lease_broker.next_lease = u64::MAX;
        assert!(lease_broker.grant(HostId(1), connection).is_none());
        assert!(lease_broker.exhausted);

        let mut stream_broker = ControllerBroker::new(1);
        let connection = stream_broker.attach(HostId(1));
        stream_broker.grant(HostId(1), connection).unwrap();
        stream_broker.stream_generation = StreamGeneration(u64::MAX);
        assert!(matches!(
            stream_broker.ingest(1),
            IngressDisposition::Delivered { .. }
        ));
        assert!(matches!(
            stream_broker.ingest(2),
            IngressDisposition::OverflowReset { .. }
        ));
        assert!(stream_broker.exhausted);
        assert!(matches!(
            stream_broker.drain(HostId(1), connection).as_slice(),
            [BrokerMessage::StreamReset { .. }]
        ));
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
