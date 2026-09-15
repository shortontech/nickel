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
    OverflowReset { event_id: EventId, lease: Lease },
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
    destination_rearm_required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PoisonedPredecessor {
    lease: Lease,
    cutoff: EventId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RecoveryDestination {
    host: HostId,
    abandoned_connection: ConnectionGeneration,
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
    recovery_destination: Option<RecoveryDestination>,
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
            recovery_destination: None,
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
        let rearming_destination = self
            .transfer
            .is_some_and(|transfer| transfer.to == host && transfer.destination_rearm_required);
        let acknowledged_predecessor = self
            .transfer
            .is_some_and(|transfer| transfer.from.host == host && transfer.quiescent);
        if replaced.is_some()
            && (self.active.is_some_and(|lease| lease.host == host)
                || self
                    .transfer
                    .is_some_and(|transfer| transfer.from.host == host || transfer.to == host))
            && !rearming_destination
            && !acknowledged_predecessor
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
        if self.poisoned_predecessor.is_some_and(|poison| {
            poison.lease.host == host && poison.lease.connection_generation == connection
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
            || self.recovery_destination.is_some()
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
        if let Some(recovery) = self.recovery_destination {
            if recovery.host != to
                || recovery.abandoned_connection == connection
                || !self.connection_matches(to, connection)
                || self.poisoned_predecessor.is_some()
                || self.reset_barrier
                || !self.neutral
            {
                return TransferStatus::Failed;
            }
            self.recovery_destination = None;
            return self
                .install_lease(to, connection)
                .map(TransferStatus::Granted)
                .unwrap_or_else(|| {
                    self.fail_closed();
                    TransferStatus::Failed
                });
        }
        if let Some(transfer) = self.transfer {
            if transfer.destination_rearm_required
                && transfer.to == to
                && transfer.to_connection != connection
                && self.connection_matches(to, connection)
                && timeout_ms != 0
            {
                return self.rearm_transfer_destination(connection, now_ms, timeout_ms);
            }
            if transfer.to != to || transfer.to_connection != connection {
                return TransferStatus::Failed;
            }
            return TransferStatus::Pending {
                requested_lease: transfer.requested_lease,
                cutoff: transfer.cutoff,
            };
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
            destination_rearm_required: false,
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
        if let Some(mut transfer) = self.transfer {
            if transfer.to == internal && transfer.to_connection == connection {
                return TransferStatus::Pending {
                    requested_lease: transfer.requested_lease,
                    cutoff: transfer.cutoff,
                };
            }
            if !self.connection_matches(internal, connection) || timeout_ms == 0 {
                return TransferStatus::Failed;
            }
            let abandoned_host = transfer.to;
            let abandoned_connection = transfer.to_connection;
            let Some(requested_lease) = self.allocate_lease_epoch() else {
                self.fail_closed();
                return TransferStatus::Failed;
            };
            if !self.reset_transfer_destination(
                abandoned_host,
                abandoned_connection,
                transfer.cutoff,
            ) {
                return TransferStatus::Failed;
            }
            transfer.to = internal;
            transfer.to_connection = connection;
            transfer.requested_lease = requested_lease;
            transfer.deadline_ms = now_ms.saturating_add(timeout_ms);
            transfer.destination_rearm_required = false;
            self.transfer = Some(transfer);
            return self.try_finish_transfer();
        }
        if self.recovery_destination.is_some() {
            let recovery = self.recovery_destination.expect("checked above");
            if !self.exhausted && self.connection_matches(internal, connection) {
                if !self.reset_transfer_destination(
                    recovery.host,
                    recovery.abandoned_connection,
                    EventId(self.next_event),
                ) {
                    return TransferStatus::Failed;
                }
                self.recovery_destination = Some(RecoveryDestination {
                    host: internal,
                    abandoned_connection: connection,
                });
            }
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

    /// Rearm an in-process policy destination after it consumed an ingress reset without changing
    /// its transport connection. External destinations must reconnect and use `begin_transfer`.
    pub fn rearm_internal_transfer_destination(
        &mut self,
        host: HostId,
        connection: ConnectionGeneration,
        now_ms: u64,
        timeout_ms: u64,
    ) -> TransferStatus {
        let Some(transfer) = self.transfer else {
            let Some(recovery) = self.recovery_destination else {
                return TransferStatus::Failed;
            };
            if recovery.host != host
                || !self.connection_matches(host, connection)
                || self.poisoned_predecessor.is_some()
                || self.reset_barrier
                || !self.neutral
            {
                return TransferStatus::Failed;
            }
            self.recovery_destination = None;
            return self
                .install_lease(host, connection)
                .map(TransferStatus::Granted)
                .unwrap_or(TransferStatus::Failed);
        };
        if !transfer.destination_rearm_required
            || transfer.to != host
            || transfer.to_connection != connection
            || !self.connection_matches(host, connection)
            || timeout_ms == 0
        {
            return TransferStatus::Pending {
                requested_lease: transfer.requested_lease,
                cutoff: transfer.cutoff,
            };
        }
        self.rearm_transfer_destination(connection, now_ms, timeout_ms)
    }

    /// Retire all delivery after an upstream bounded ingress overflow. The dropped native batch
    /// may contain release edges, so authority cannot survive and delivery stays behind the reset
    /// barrier until native neutral is observed and a lease is granted again.
    pub fn reset_ingress(&mut self) -> Option<(Lease, EventId)> {
        if self.exhausted {
            return None;
        }
        let interrupted_transfer = self.transfer;
        let recovery_destination = self.recovery_destination;
        let recoverable = (self.transfer.is_none() && self.poisoned_predecessor.is_none())
            .then_some(self.active)
            .flatten();
        if recoverable.is_some() {
            self.poison_executor(EventId(self.next_event));
        }
        self.install_reset(EventId(self.next_event));
        self.recovery_destination = recovery_destination;
        if let Some(transfer) = interrupted_transfer {
            if self.exhausted {
                return None;
            }
            self.transfer = Some(Transfer {
                destination_rearm_required: true,
                ..transfer
            });
            if let Some(predecessor) = self.hosts.get_mut(&transfer.from.host) {
                predecessor.outbox.clear();
                predecessor.outbox.push_back(BrokerMessage::Revoke {
                    connection_generation: transfer.from.connection_generation,
                    lease_epoch: transfer.from.epoch,
                    cutoff: transfer.cutoff,
                });
            }
        }
        recoverable.map(|lease| (lease, EventId(self.next_event)))
    }

    /// Accept an authenticated executor's report that its bounded press ledger overflowed.
    /// The reporting connection is abandoned: recovery requires neutral input and a fresh
    /// connection/lease request, so a later release on the old stream cannot rearm execution.
    pub fn reset_executor_overflow(
        &mut self,
        host: HostId,
        connection: ConnectionGeneration,
        lease_epoch: LeaseEpoch,
        stream_generation: StreamGeneration,
        through: EventId,
    ) -> bool {
        let Some(active) = self.active else {
            return false;
        };
        if active.host != host
            || active.connection_generation != connection
            || active.epoch != lease_epoch
            || self.stream_generation != stream_generation
            || through.0 > self.next_event
        {
            return false;
        }
        self.install_reset(through);
        if self.exhausted {
            return false;
        }
        self.recovery_destination = Some(RecoveryDestination {
            host,
            abandoned_connection: connection,
        });
        true
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
        if !transfer.quiescent {
            self.poisoned_predecessor = Some(PoisonedPredecessor {
                lease: transfer.from,
                cutoff: transfer.cutoff,
            });
        }
        self.recovery_destination = Some(RecoveryDestination {
            host: transfer.to,
            abandoned_connection: transfer.to_connection,
        });
        if !self.reset_transfer_destination(transfer.to, transfer.to_connection, transfer.cutoff) {
            return TransferStatus::Failed;
        }
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
            self.poison_executor(event_id);
            self.install_reset(event_id);
            if !self.exhausted {
                self.recovery_destination = Some(RecoveryDestination {
                    host: lease.host,
                    abandoned_connection: lease.connection_generation,
                });
            }
            return IngressDisposition::OverflowReset { event_id, lease };
        };
        if host.outbox.len() >= self.queue_limit {
            self.poison_executor(event_id);
            self.install_reset(event_id);
            if !self.exhausted {
                self.recovery_destination = Some(RecoveryDestination {
                    host: lease.host,
                    abandoned_connection: lease.connection_generation,
                });
            }
            return IngressDisposition::OverflowReset { event_id, lease };
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

    /// Whether this exact executor must remain attached long enough to receive and acknowledge
    /// its cutoff. Callers may retire it after acknowledgement or verified termination.
    pub fn revocation_pending(&self, host: HostId, connection: ConnectionGeneration) -> bool {
        self.transfer.is_some_and(|transfer| {
            transfer.from.host == host && transfer.from.connection_generation == connection
        }) || self.poisoned_predecessor.is_some_and(|poison| {
            poison.lease.host == host && poison.lease.connection_generation == connection
        })
    }

    pub fn exhaust(&mut self) {
        self.fail_closed();
    }

    fn try_finish_transfer(&mut self) -> TransferStatus {
        let Some(transfer) = self.transfer else {
            return TransferStatus::Failed;
        };
        if transfer.destination_rearm_required
            || !transfer.quiescent
            || !self.neutral
            || self.reset_barrier
        {
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

    fn rearm_transfer_destination(
        &mut self,
        connection: ConnectionGeneration,
        now_ms: u64,
        timeout_ms: u64,
    ) -> TransferStatus {
        let Some(mut transfer) = self.transfer else {
            return TransferStatus::Failed;
        };
        let Some(requested_lease) = self.allocate_lease_epoch() else {
            self.fail_closed();
            return TransferStatus::Failed;
        };
        transfer.to_connection = connection;
        transfer.requested_lease = requested_lease;
        transfer.deadline_ms = now_ms.saturating_add(timeout_ms);
        transfer.destination_rearm_required = false;
        self.transfer = Some(transfer);
        self.try_finish_transfer()
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

    /// Retire one abandoned transfer destination without touching the predecessor's revoke.
    /// The connection binding keeps a replacement generation from consuming the stale reset.
    fn reset_transfer_destination(
        &mut self,
        host: HostId,
        connection: ConnectionGeneration,
        through: EventId,
    ) -> bool {
        let Some(stream_generation) = self.stream_generation.0.checked_add(1) else {
            self.stream_generation = StreamGeneration(u64::MAX);
            self.fail_closed();
            return false;
        };
        self.stream_generation = StreamGeneration(stream_generation);
        let generation = self.stream_generation;
        if let Some(destination) = self
            .hosts
            .get_mut(&host)
            .filter(|destination| destination.connection == connection)
        {
            destination.outbox.clear();
            destination.outbox.push_back(BrokerMessage::StreamReset {
                stream_generation: generation,
                through,
            });
        }
        true
    }

    fn install_reset(&mut self, through: EventId) {
        self.active = None;
        self.transfer = None;
        self.recovery_destination = None;
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
            || self
                .recovery_destination
                .is_some_and(|recovery| recovery.host == host)
    }

    fn fail_closed(&mut self) {
        self.exhausted = true;
        self.active = None;
        self.transfer = None;
        self.recovery_destination = None;
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
    fn pending_transfer_identity_is_visible_only_to_its_exact_destination() {
        let mut broker = ControllerBroker::<()>::new(8);
        let a = broker.attach(HostId(1));
        let internal = broker.attach(HostId(0));
        let b = broker.attach(HostId(2));
        let predecessor = broker.grant(HostId(1), a).unwrap();
        broker.set_neutral(false);

        let pending = broker.begin_transfer(HostId(0), internal, 10, 100);
        let TransferStatus::Pending { cutoff, .. } = pending else {
            panic!("A to internal transfer must be pending");
        };
        assert_eq!(
            broker.begin_transfer(HostId(2), b, 11, 100),
            TransferStatus::Failed,
            "host B must not adopt the internal destination's pending lease identity"
        );
        assert_eq!(broker.begin_transfer(HostId(0), internal, 12, 100), pending);

        assert_eq!(
            broker.acknowledge_quiescence(HostId(1), a, predecessor.epoch, cutoff),
            pending
        );
        let internal_lease = broker.set_neutral(true).unwrap();
        assert_eq!(internal_lease.host, HostId(0));
        assert!(matches!(
            broker.begin_transfer(HostId(2), b, 13, 100),
            TransferStatus::Pending { .. }
        ));
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
        assert!(broker.grant(HostId(2), b).is_none());
        let fresh_b = broker.attach(HostId(2));
        assert!(matches!(
            broker.begin_transfer(HostId(2), fresh_b, 20, 5),
            TransferStatus::Granted(Lease {
                host: HostId(2),
                ..
            })
        ));
    }

    #[test]
    fn timed_out_transfer_recovers_after_verified_termination_and_fresh_destination() {
        let mut broker = ControllerBroker::<()>::new(4);
        let a = broker.attach(HostId(1));
        let stale_b = broker.attach(HostId(2));
        broker.grant(HostId(1), a).unwrap();
        broker.begin_transfer(HostId(2), stale_b, 10, 5);
        assert_eq!(broker.expire_transfer(15), TransferStatus::Failed);
        broker.set_neutral(true);
        assert_eq!(
            broker.acknowledge_verified_termination(HostId(1), a),
            TransferStatus::Failed
        );
        assert!(broker.grant(HostId(2), stale_b).is_none());

        let fresh_b = broker.attach(HostId(2));
        assert!(matches!(
            broker.begin_transfer(HostId(2), fresh_b, 20, 5),
            TransferStatus::Granted(Lease {
                host: HostId(2),
                connection_generation,
                ..
            }) if connection_generation == fresh_b
        ));
    }

    #[test]
    fn reset_preserved_transfer_timeout_still_requires_late_ack_and_fresh_destination() {
        let mut broker = ControllerBroker::<()>::new(4);
        let a = broker.attach(HostId(1));
        let stale_b = broker.attach(HostId(2));
        let old = broker.grant(HostId(1), a).unwrap();
        let TransferStatus::Pending { cutoff, .. } =
            broker.begin_transfer(HostId(2), stale_b, 10, 5)
        else {
            panic!("transfer must be pending");
        };
        broker.reset_ingress();
        assert_eq!(broker.expire_transfer(15), TransferStatus::Failed);
        broker.set_neutral(true);
        assert!(broker.grant(HostId(2), stale_b).is_none());
        assert_eq!(
            broker.acknowledge_quiescence(HostId(1), a, old.epoch, cutoff),
            TransferStatus::Failed
        );
        assert!(broker.grant(HostId(2), stale_b).is_none());

        let fresh_b = broker.attach(HostId(2));
        assert!(matches!(
            broker.begin_transfer(HostId(2), fresh_b, 20, 5),
            TransferStatus::Granted(Lease {
                host: HostId(2),
                ..
            })
        ));
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
    fn security_takeover_retargets_without_erasing_predecessor_cutoff() {
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

        assert!(matches!(
            broker.security_takeover(HostId(0), internal, 1, 10),
            TransferStatus::Pending {
                requested_lease: replacement,
                cutoff: observed,
            } if replacement != requested_lease && observed == cutoff
        ));
        assert_eq!(broker.active_lease(), None);
        assert!(matches!(
            broker.ingest("late"),
            IngressDisposition::RejectedTransfer { .. }
        ));
        assert!(matches!(
            broker.drain(HostId(1), external).as_slice(),
            [BrokerMessage::StreamReset { through, .. }] if *through == cutoff
        ));
        assert!(matches!(
            broker.drain(HostId(0), internal).as_slice(),
            [BrokerMessage::Revoke {
                connection_generation,
                lease_epoch,
                cutoff: observed,
            }] if *connection_generation == internal
                && *lease_epoch == old.epoch
                && *observed == cutoff
        ));
        assert!(matches!(
            broker.acknowledge_quiescence(HostId(0), internal, old.epoch, cutoff),
            TransferStatus::Granted(Lease {
                host: HostId(0),
                connection_generation,
                ..
            }) if connection_generation == internal
        ));
        assert_ne!(broker.active_lease().unwrap().epoch, requested_lease);
        assert!(broker.grant(HostId(1), external).is_none());
    }

    #[test]
    fn acknowledged_predecessor_reconnect_and_expiry_preserve_recovery() {
        let mut broker = ControllerBroker::<()>::new(4);
        let predecessor = broker.attach(HostId(1));
        let stale_destination = broker.attach(HostId(2));
        let old = broker.grant(HostId(1), predecessor).unwrap();
        broker.set_neutral(false);
        let TransferStatus::Pending { cutoff, .. } =
            broker.begin_transfer(HostId(2), stale_destination, 10, 5)
        else {
            panic!("transfer must start");
        };
        assert!(matches!(
            broker.acknowledge_quiescence(HostId(1), predecessor, old.epoch, cutoff),
            TransferStatus::Pending { .. }
        ));

        let replacement_predecessor = broker.attach(HostId(1));
        assert_ne!(replacement_predecessor, predecessor);
        assert_eq!(broker.expire_transfer(15), TransferStatus::Failed);
        assert!(matches!(
            broker.drain(HostId(2), stale_destination).as_slice(),
            [BrokerMessage::StreamReset { through, .. }] if *through == cutoff
        ));
        broker.set_neutral(true);
        let fresh_destination = broker.attach(HostId(2));
        assert!(matches!(
            broker.begin_transfer(HostId(2), fresh_destination, 20, 5),
            TransferStatus::Granted(Lease {
                host: HostId(2),
                connection_generation,
                ..
            }) if connection_generation == fresh_destination
        ));
    }

    #[test]
    fn transfer_timeout_notifies_only_destination_and_retains_revoke_evidence() {
        let mut broker = ControllerBroker::<()>::new(4);
        let predecessor = broker.attach(HostId(1));
        let destination = broker.attach(HostId(2));
        let old = broker.grant(HostId(1), predecessor).unwrap();
        let TransferStatus::Pending { cutoff, .. } =
            broker.begin_transfer(HostId(2), destination, 0, 10)
        else {
            panic!("transfer must start");
        };

        assert_eq!(broker.expire_transfer(10), TransferStatus::Failed);
        assert!(matches!(
            broker.drain(HostId(2), destination).as_slice(),
            [BrokerMessage::StreamReset { through, .. }] if *through == cutoff
        ));
        assert!(matches!(
            broker.drain(HostId(1), predecessor).as_slice(),
            [BrokerMessage::Revoke {
                connection_generation,
                lease_epoch,
                cutoff: observed,
            }] if *connection_generation == predecessor
                && *lease_epoch == old.epoch
                && *observed == cutoff
        ));
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
        let old = broker.grant(HostId(1), connection).unwrap();
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
        assert_eq!(
            broker.acknowledge_quiescence(HostId(1), connection, old.epoch, EventId(3)),
            TransferStatus::Failed
        );
        assert!(broker.set_neutral(true).is_none());
        assert!(broker.grant(HostId(1), connection).is_none());
        let replacement = broker.attach(HostId(1));
        assert!(matches!(
            broker.begin_transfer(HostId(1), replacement, 0, 10),
            TransferStatus::Granted(Lease {
                host: HostId(1),
                connection_generation,
                ..
            }) if connection_generation == replacement
        ));
    }

    #[test]
    fn slow_host_recovers_after_the_257th_event_overflows_its_outbox() {
        let mut broker = ControllerBroker::new(DEFAULT_CONTROLLER_QUEUE_LIMIT);
        let connection = broker.attach(HostId(9));
        let old = broker.grant(HostId(9), connection).unwrap();
        for payload in 0..DEFAULT_CONTROLLER_QUEUE_LIMIT {
            assert!(matches!(
                broker.ingest(payload),
                IngressDisposition::Delivered { .. }
            ));
        }
        assert!(matches!(
            broker.ingest(DEFAULT_CONTROLLER_QUEUE_LIMIT),
            IngressDisposition::OverflowReset {
                event_id: EventId(257),
                ..
            }
        ));
        assert!(matches!(
            broker.drain(HostId(9), connection).as_slice(),
            [BrokerMessage::StreamReset {
                through: EventId(257),
                ..
            }]
        ));

        assert_eq!(
            broker.acknowledge_quiescence(HostId(9), connection, old.epoch, EventId(257)),
            TransferStatus::Failed
        );
        broker.set_neutral(true);
        let replacement = broker.attach(HostId(9));
        assert!(matches!(
            broker.begin_transfer(HostId(9), replacement, 20, 10),
            TransferStatus::Granted(Lease {
                host: HostId(9),
                connection_generation,
                ..
            }) if connection_generation == replacement
        ));
        assert!(matches!(
            broker.ingest(257),
            IngressDisposition::Delivered { .. }
        ));
    }

    #[test]
    fn upstream_ingress_overflow_retires_authority_until_neutral_and_regrant() {
        let mut broker = ControllerBroker::new(4);
        let connection = broker.attach(HostId(1));
        broker.grant(HostId(1), connection).unwrap();
        broker.ingest(1);

        let recoverable = broker.reset_ingress();

        assert!(broker.active_lease().is_none());
        assert!(matches!(
            broker.drain(HostId(1), connection).as_slice(),
            [BrokerMessage::StreamReset {
                through: EventId(1),
                ..
            }]
        ));
        assert!(matches!(
            broker.ingest(2),
            IngressDisposition::RejectedResetBarrier { .. }
        ));
        assert!(broker.grant(HostId(1), connection).is_none());
        broker.set_neutral(true);
        let (recoverable, cutoff) = recoverable.unwrap();
        assert_eq!(
            broker.acknowledge_quiescence(
                recoverable.host,
                recoverable.connection_generation,
                recoverable.epoch,
                cutoff,
            ),
            TransferStatus::Failed
        );
        broker.set_neutral(true);
        assert!(
            broker
                .grant(recoverable.host, recoverable.connection_generation)
                .is_some()
        );
    }

    #[test]
    fn overflow_during_transfer_preserves_cutoff_and_grants_destination_after_exact_ack() {
        let mut broker = ControllerBroker::new(4);
        let a = broker.attach(HostId(1));
        let b = broker.attach(HostId(2));
        let old = broker.grant(HostId(1), a).unwrap();
        let TransferStatus::Pending {
            requested_lease: abandoned_lease,
            cutoff,
        } = broker.begin_transfer(HostId(2), b, 10, 100)
        else {
            panic!("transfer must be pending");
        };

        assert!(broker.reset_ingress().is_none());
        assert!(matches!(
            broker.drain(HostId(1), a).as_slice(),
            [BrokerMessage::Revoke {
                connection_generation,
                lease_epoch,
                cutoff: observed_cutoff,
            }] if *connection_generation == a
                && *lease_epoch == old.epoch
                && *observed_cutoff == cutoff
        ));
        assert!(matches!(
            broker.drain(HostId(2), b).as_slice(),
            [BrokerMessage::StreamReset { .. }]
        ));
        assert!(matches!(
            broker.ingest(10),
            IngressDisposition::RejectedTransfer { .. }
                | IngressDisposition::RejectedResetBarrier { .. }
        ));
        assert!(broker.set_neutral(true).is_none());
        assert!(matches!(
            broker.acknowledge_quiescence(HostId(1), a, old.epoch, cutoff),
            TransferStatus::Pending { .. }
        ));
        assert!(matches!(
            broker.begin_transfer(HostId(2), b, 20, 100),
            TransferStatus::Pending { .. }
        ));
        let reconnected_b = broker.attach(HostId(2));
        let TransferStatus::Granted(granted) =
            broker.begin_transfer(HostId(2), reconnected_b, 30, 100)
        else {
            panic!("fresh destination request must complete the fenced transfer");
        };
        assert_eq!(granted.host, HostId(2));
        assert_eq!(granted.connection_generation, reconnected_b);
        assert_ne!(granted.epoch, abandoned_lease);
    }

    #[test]
    fn overflow_transfer_rejects_late_predecessor_and_accepts_verified_termination() {
        let mut broker = ControllerBroker::new(4);
        let a = broker.attach(HostId(1));
        let b = broker.attach(HostId(2));
        let old = broker.grant(HostId(1), a).unwrap();
        let TransferStatus::Pending { cutoff, .. } = broker.begin_transfer(HostId(2), b, 10, 100)
        else {
            panic!("transfer must be pending");
        };
        broker.reset_ingress();
        broker.drain(HostId(1), a);

        assert!(matches!(
            broker.acknowledge_quiescence(HostId(1), a, old.epoch, EventId(cutoff.0 + 1)),
            TransferStatus::Pending { cutoff: pending, .. } if pending == cutoff
        ));
        assert!(broker.active_lease().is_none());
        assert!(matches!(
            broker.acknowledge_verified_termination(HostId(1), a),
            TransferStatus::Pending { .. }
        ));
        assert!(broker.set_neutral(true).is_none());
        assert!(broker.active_lease().is_none());
        let reconnected_b = broker.attach(HostId(2));
        let TransferStatus::Granted(granted) =
            broker.begin_transfer(HostId(2), reconnected_b, 30, 100)
        else {
            panic!("verified predecessor still requires fresh destination rearm");
        };
        assert_eq!(granted.host, HostId(2));
        assert!(broker.drain(HostId(1), a).is_empty());
        assert!(matches!(
            broker.ingest(11),
            IngressDisposition::Delivered { lease, .. } if lease.host == HostId(2)
        ));
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
    fn executor_overflow_requires_acknowledged_reset_neutral_and_new_connection() {
        let mut broker = ControllerBroker::new(4);
        let old_connection = broker.attach(HostId(1));
        let lease = broker.grant(HostId(1), old_connection).unwrap();
        let IngressDisposition::Delivered { event_id, .. } = broker.ingest(1) else {
            panic!("initial event must be delivered");
        };

        assert!(broker.reset_executor_overflow(
            HostId(1),
            old_connection,
            lease.epoch,
            broker.stream_generation(),
            event_id,
        ));
        assert!(broker.active_lease().is_none());
        assert!(matches!(
            broker.ingest(2),
            IngressDisposition::RejectedResetBarrier { .. }
        ));
        broker.set_neutral(true);
        assert!(broker.grant(HostId(1), old_connection).is_none());

        let new_connection = broker.attach(HostId(1));
        assert!(matches!(
            broker.begin_transfer(HostId(1), new_connection, 10, 100),
            TransferStatus::Granted(_)
        ));
    }

    #[test]
    fn security_takeover_replaces_timed_out_external_destination_with_internal_recovery() {
        let mut broker = ControllerBroker::<()>::new(8);
        let internal = broker.attach(HostId(0));
        let predecessor = broker.attach(HostId(1));
        let abandoned = broker.attach(HostId(2));
        let old = broker.grant(HostId(1), predecessor).unwrap();
        let TransferStatus::Pending { cutoff, .. } =
            broker.begin_transfer(HostId(2), abandoned, 0, 10)
        else {
            panic!("external transfer must start");
        };
        assert_eq!(broker.expire_transfer(10), TransferStatus::Failed);

        assert_eq!(
            broker.security_takeover(HostId(0), internal, 11, 10),
            TransferStatus::Failed
        );
        let fresh_abandoned = broker.attach(HostId(2));
        assert_eq!(
            broker.begin_transfer(HostId(2), fresh_abandoned, 12, 10),
            TransferStatus::Failed
        );
        assert_eq!(
            broker.acknowledge_quiescence(HostId(1), predecessor, old.epoch, cutoff,),
            TransferStatus::Failed
        );
        assert!(broker.set_neutral(true).is_none());
        let TransferStatus::Granted(granted) =
            broker.rearm_internal_transfer_destination(HostId(0), internal, 13, 10)
        else {
            panic!("internal policy successor must recover after poison and neutral");
        };
        assert_eq!(granted.host, HostId(0));
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
