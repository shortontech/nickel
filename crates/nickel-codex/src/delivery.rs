//! Nonblocking, bounded delivery. Payloads are stored encoded so retained bytes
//! include actual allocation capacity, independent of nested collection layouts.
//! A default event stream owns at most 4 MiB / 256 entries, plus one 1 MiB
//! encoder per producer. Decoded objects and bounded coalescing scratch are
//! temporary; nested Rust collections can exceed their encoded byte length.
//! Commands and snapshots override these limits; see DELIVERY.md.
//! Overflow replaces queued data with a reserved
//! terminal failure; consumers must reconnect and reload authoritative state.
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::VecDeque,
    io::Write,
    sync::{Arc, Condvar, Mutex, mpsc},
    time::{Duration, Instant},
};

pub const QUEUE_BYTES: usize = 4 * 1024 * 1024;
pub const EVENT_BYTES: usize = 1024 * 1024;
pub const QUEUE_ENTRIES: usize = 256;

pub trait Delivery: Serialize + DeserializeOwned {
    const MAX_BYTES: usize = QUEUE_BYTES;
    const MAX_EVENT_BYTES: usize = EVENT_BYTES;
    const MAX_ENTRIES: usize = QUEUE_ENTRIES;
    const TERMINAL_OVERFLOW: bool = true;
    fn overflow(&self) -> Self;
    /// A cheap compatibility hint. `merge` must still verify exact identity.
    fn coalesce_key(&self) -> Option<u64> {
        None
    }
    fn merge(&mut self, _next: &Self) -> bool {
        false
    }
}
impl<T: Delivery> Delivery for (u64, T) {
    const MAX_BYTES: usize = T::MAX_BYTES;
    const MAX_EVENT_BYTES: usize = T::MAX_EVENT_BYTES;
    const MAX_ENTRIES: usize = T::MAX_ENTRIES;
    const TERMINAL_OVERFLOW: bool = T::TERMINAL_OVERFLOW;
    fn overflow(&self) -> Self {
        (self.0, self.1.overflow())
    }
    fn merge(&mut self, next: &Self) -> bool {
        self.0 == next.0 && self.1.merge(&next.1)
    }
    fn coalesce_key(&self) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (self.0, self.1.coalesce_key()?).hash(&mut hash);
        Some(hash.finish())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DeliveryMetrics {
    pub bytes: usize,
    pub entries: usize,
    pub high_water_bytes: usize,
    pub high_water_entries: usize,
    pub coalesced: u64,
    pub overflows: u64,
    pub recoveries: u64,
    pub max_latency_micros: u128,
}
struct State {
    queue: VecDeque<(Box<[u8]>, Instant, Option<u64>)>,
    metrics: DeliveryMetrics,
    closed: bool,
    receiver_alive: bool,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}
pub struct DeliverySender<T> {
    shared: Arc<Shared>,
    marker: std::marker::PhantomData<T>,
}
pub struct DeliveryReceiver<T> {
    shared: Arc<Shared>,
    marker: std::marker::PhantomData<T>,
}

pub fn channel<T: Delivery>() -> (DeliverySender<T>, DeliveryReceiver<T>) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            queue: VecDeque::new(),
            metrics: DeliveryMetrics::default(),
            closed: false,
            receiver_alive: true,
        }),
        changed: Condvar::new(),
    });
    (
        DeliverySender {
            shared: shared.clone(),
            marker: std::marker::PhantomData,
        },
        DeliveryReceiver {
            shared,
            marker: std::marker::PhantomData,
        },
    )
}
struct Encoder(Vec<u8>, usize);
impl Write for Encoder {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > self.1 {
            return Err(std::io::Error::other("delivery payload exceeds budget"));
        }
        let needed = self.0.len() + bytes.len();
        if needed > self.0.capacity() {
            let capacity = needed.max(self.0.capacity().saturating_mul(2)).min(self.1);
            self.0.reserve_exact(capacity - self.0.len());
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encode<T: Delivery>(value: &T) -> Option<Box<[u8]>> {
    Some(
        encode_json(value, T::MAX_EVENT_BYTES)
            .ok()?
            .into_boxed_slice(),
    )
}
pub(crate) fn encode_json(
    value: &impl Serialize,
    limit: usize,
) -> Result<Vec<u8>, serde_json::Error> {
    let mut encoder = Encoder(Vec::new(), limit);
    serde_json::to_writer(&mut encoder, value)?;
    Ok(encoder.0)
}
impl<T: Delivery> DeliverySender<T> {
    pub fn is_closed(&self) -> bool {
        let state = self.shared.state.lock().unwrap();
        state.closed || !state.receiver_alive
    }
    pub fn send(&self, value: T) -> Result<(), mpsc::SendError<T>> {
        let encoded = encode(&value);
        let mut state = self.shared.state.lock().unwrap();
        if state.closed || !state.receiver_alive {
            return Err(mpsc::SendError(value));
        }
        if let Some(bytes) = encoded {
            let key = (bytes.len() <= 24 * 1024)
                .then(|| value.coalesce_key())
                .flatten();
            let merged = state.queue.back().and_then(|(previous, _, previous_key)| {
                if key.is_none() || key != *previous_key || previous.len() > 24 * 1024 {
                    return None;
                }
                let mut previous: T = serde_json::from_slice(previous).ok()?;
                if previous.merge(&value) {
                    encode(&previous)
                } else {
                    None
                }
            });
            if let Some(merged) = merged {
                let previous_len = state.queue.back().unwrap().0.len();
                if state.metrics.bytes - previous_len + merged.len() <= T::MAX_BYTES {
                    state.metrics.bytes = state.metrics.bytes - previous_len + merged.len();
                    state.queue.back_mut().unwrap().0 = merged;
                    state.metrics.coalesced += 1;
                    state.metrics.high_water_bytes =
                        state.metrics.high_water_bytes.max(state.metrics.bytes);
                    return Ok(());
                }
            }
            if state.queue.len() < T::MAX_ENTRIES
                && state.metrics.bytes + bytes.len() <= T::MAX_BYTES
            {
                state.metrics.bytes += bytes.len();
                state.queue.push_back((bytes, Instant::now(), key));
                state.metrics.entries = state.queue.len();
                state.metrics.high_water_entries =
                    state.metrics.high_water_entries.max(state.metrics.entries);
                state.metrics.high_water_bytes =
                    state.metrics.high_water_bytes.max(state.metrics.bytes);
                self.shared.changed.notify_one();
                return Ok(());
            }
        }
        if !T::TERMINAL_OVERFLOW {
            state.metrics.overflows += 1;
            return Err(mpsc::SendError(value));
        }
        state.queue.clear();
        let failure = encode(&value.overflow())
            .expect("terminal delivery failure must fit reserved capacity");
        state.metrics.bytes = failure.len();
        state.metrics.entries = 1;
        state.metrics.overflows += 1;
        state.queue.push_back((failure, Instant::now(), None));
        state.closed = true;
        self.shared.changed.notify_all();
        Err(mpsc::SendError(value))
    }
}
impl<T> Drop for DeliverySender<T> {
    fn drop(&mut self) {
        self.shared.state.lock().unwrap().closed = true;
        self.shared.changed.notify_all();
    }
}
impl<T: Delivery> DeliveryReceiver<T> {
    pub fn metrics(&self) -> DeliveryMetrics {
        self.shared.state.lock().unwrap().metrics
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, mpsc::RecvTimeoutError> {
        let mut state = self.shared.state.lock().unwrap();
        let deadline = Instant::now() + timeout;
        loop {
            if let Some((bytes, queued, _)) = state.queue.pop_front() {
                state.metrics.bytes -= bytes.len();
                state.metrics.entries = state.queue.len();
                state.metrics.max_latency_micros = state
                    .metrics
                    .max_latency_micros
                    .max(queued.elapsed().as_micros());
                drop(state);
                return Ok(serde_json::from_slice(&bytes).expect("delivery encodes its own values"));
            }
            if state.closed {
                return Err(mpsc::RecvTimeoutError::Disconnected);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(mpsc::RecvTimeoutError::Timeout);
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .unwrap()
                .0;
        }
    }
    pub fn try_recv(&self) -> Result<T, mpsc::TryRecvError> {
        self.recv_timeout(Duration::ZERO)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => mpsc::TryRecvError::Empty,
                mpsc::RecvTimeoutError::Disconnected => mpsc::TryRecvError::Disconnected,
            })
    }
}
impl<T: Delivery> Iterator for DeliveryReceiver<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.recv_timeout(Duration::from_secs(86400)).ok()
    }
}
impl<T> Drop for DeliveryReceiver<T> {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap();
        state.receiver_alive = false;
        state.queue.clear();
        state.metrics.bytes = 0;
        state.metrics.entries = 0;
        self.shared.changed.notify_all();
    }
}

impl Delivery for crate::CodexEvent {
    fn coalesce_key(&self) -> Option<u64> {
        use crate::EventKind::*;
        use std::hash::{Hash, Hasher};
        let item_id = match &self.kind {
            AgentMessageDelta { item_id, .. }
            | CommandOutputDelta { item_id, .. }
            | FileChangeDelta { item_id, .. }
            | PlanDelta { item_id, .. }
            | ReasoningDelta { item_id, .. } => item_id,
            _ => return None,
        };
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (std::mem::discriminant(&self.kind), item_id).hash(&mut hash);
        Some(hash.finish())
    }
    fn overflow(&self) -> Self {
        Self {
            sequence: 0,
            kind: crate::EventKind::Connection {
                state: "failed: event delivery overflow; reconnect to reload authoritative state"
                    .into(),
            },
        }
    }
    fn merge(&mut self, next: &Self) -> bool {
        use crate::EventKind::*;
        let pair = match (&mut self.kind, &next.kind) {
            (
                AgentMessageDelta {
                    item_id: a,
                    delta: x,
                },
                AgentMessageDelta {
                    item_id: b,
                    delta: y,
                },
            )
            | (
                CommandOutputDelta {
                    item_id: a,
                    delta: x,
                },
                CommandOutputDelta {
                    item_id: b,
                    delta: y,
                },
            )
            | (
                FileChangeDelta {
                    item_id: a,
                    delta: x,
                },
                FileChangeDelta {
                    item_id: b,
                    delta: y,
                },
            )
            | (
                PlanDelta {
                    item_id: a,
                    delta: x,
                },
                PlanDelta {
                    item_id: b,
                    delta: y,
                },
            )
            | (
                ReasoningDelta {
                    item_id: a,
                    delta: x,
                },
                ReasoningDelta {
                    item_id: b,
                    delta: y,
                },
            ) => Some((a, x, b, y)),
            _ => None,
        };
        if let Some((a, x, b, y)) = pair
            && a == b
            && x.len().saturating_add(y.len()) <= 16 * 1024
        {
            x.push_str(y);
            self.sequence = next.sequence;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CodexEvent, EventKind};

    #[test]
    fn rejected_oversized_retryable_payload_preserves_accepted_work() {
        #[derive(Debug, serde::Serialize, serde::Deserialize)]
        struct Retryable(String);
        impl Delivery for Retryable {
            const MAX_EVENT_BYTES: usize = 16;
            const TERMINAL_OVERFLOW: bool = false;
            fn overflow(&self) -> Self {
                Self("failed".into())
            }
        }
        let (sender, receiver) = channel();
        sender.send(Retryable("accepted".into())).unwrap();
        assert!(sender.send(Retryable("x".repeat(32))).is_err());
        assert!(!sender.is_closed());
        assert_eq!(receiver.try_recv().unwrap().0, "accepted");
        sender.send(Retryable("retry".into())).unwrap();
        assert_eq!(receiver.try_recv().unwrap().0, "retry");
        assert_eq!(receiver.metrics().overflows, 1);
    }
    fn delta(sequence: u64, text: &str) -> CodexEvent {
        CodexEvent {
            sequence,
            kind: EventKind::AgentMessageDelta {
                item_id: "item".into(),
                delta: text.into(),
            },
        }
    }
    #[test]
    fn adjacent_deltas_coalesce_without_crossing_approval() {
        let (tx, rx) = channel();
        tx.send(delta(1, "a")).unwrap();
        tx.send(delta(2, "b")).unwrap();
        tx.send(CodexEvent {
            sequence: 3,
            kind: EventKind::ApprovalRequested {
                request_id: crate::ServerRequestId("request".into()),
                approval_type: "command".into(),
                summary: None,
            },
        })
        .unwrap();
        tx.send(delta(4, "c")).unwrap();
        assert_eq!(rx.metrics().coalesced, 1);
        assert!(
            matches!(rx.try_recv().unwrap().kind, EventKind::AgentMessageDelta { delta, .. } if delta == "ab")
        );
        assert!(matches!(
            rx.try_recv().unwrap().kind,
            EventKind::ApprovalRequested { .. }
        ));
        assert!(
            matches!(rx.try_recv().unwrap().kind, EventKind::AgentMessageDelta { delta, .. } if delta == "c")
        );
        assert_eq!(rx.metrics().bytes, 0);
    }
    #[test]
    fn saturation_replaces_partial_history_with_terminal_failure() {
        let (tx, rx) = channel();
        for sequence in 0..QUEUE_ENTRIES as u64 {
            tx.send(CodexEvent {
                sequence,
                kind: EventKind::ItemCompleted {
                    item_id: sequence.to_string(),
                },
            })
            .unwrap();
        }
        assert!(tx.send(delta(999, "overflow")).is_err());
        assert_eq!(rx.metrics().overflows, 1);
        assert_eq!(rx.metrics().entries, 1);
        assert!(rx.metrics().high_water_bytes <= QUEUE_BYTES);
        assert!(
            matches!(rx.try_recv().unwrap().kind, EventKind::Connection { state } if state.contains("overflow"))
        );
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
        assert!(tx.send(delta(1000, "late")).is_err());
    }

    #[test]
    fn sustained_text_is_split_into_bounded_coalescing_chunks() {
        let (tx, rx) = channel();
        for sequence in 0..1000 {
            tx.send(delta(sequence, &"x".repeat(128))).unwrap();
        }
        assert_eq!(rx.metrics().entries, 8);
        drop(tx);
        let mut received = 0;
        for event in rx {
            if let EventKind::AgentMessageDelta { delta, .. } = event.kind {
                assert!(delta.len() <= 16 * 1024);
                received += delta.len();
            }
        }
        assert_eq!(received, 128_000);
    }
    #[test]
    fn oversized_payload_fails_and_receiver_drop_releases_storage() {
        let (tx, rx) = channel();
        assert!(tx.send(delta(1, &"x".repeat(EVENT_BYTES + 1))).is_err());
        assert_eq!(rx.metrics().entries, 1);
        drop(rx);
        assert_eq!(tx.shared.state.lock().unwrap().metrics.bytes, 0);
        assert!(tx.send(delta(2, "late")).is_err());
    }
    #[test]
    fn byte_limit_is_enforced_before_entry_limit() {
        let (tx, rx) = channel();
        for sequence in 0..20 {
            let result = tx.send(CodexEvent {
                sequence,
                kind: EventKind::Error {
                    message: "x".repeat(EVENT_BYTES / 2),
                },
            });
            assert!(rx.metrics().bytes <= QUEUE_BYTES);
            if result.is_err() {
                break;
            }
        }
        assert_eq!(rx.metrics().overflows, 1);
        assert!(rx.metrics().high_water_entries < QUEUE_ENTRIES);
    }
    #[test]
    fn terminal_failure_preserves_generation_and_never_blocks_sender() {
        let (tx, rx) = channel();
        assert!(
            tx.send((42, delta(1, &"x".repeat(EVENT_BYTES + 1))))
                .is_err()
        );
        assert_eq!(rx.try_recv().unwrap().0, 42);
        drop(tx);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(1)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    #[ignore = "repeatable release delivery workload"]
    fn release_burst_measurement() {
        let (tx, rx) = channel();
        let start = Instant::now();
        let mut received = 0;
        for sequence in 0..100_000 {
            tx.send(delta(sequence, &"x".repeat(128))).unwrap();
            if sequence % 128 == 127 {
                while let Ok(event) = rx.try_recv() {
                    if let EventKind::AgentMessageDelta { delta, .. } = event.kind {
                        received += delta.len();
                    }
                }
            }
        }
        while let Ok(event) = rx.try_recv() {
            if let EventKind::AgentMessageDelta { delta, .. } = event.kind {
                received += delta.len();
            }
        }
        assert_eq!(received, 100_000 * 128);
        println!(
            "delivery burst: elapsed={:?}, metrics={:?}",
            start.elapsed(),
            rx.metrics()
        );
        for sequence in 0..100_000 {
            if tx
                .send(CodexEvent {
                    sequence,
                    kind: EventKind::Error {
                        message: "x".repeat(128 * 1024),
                    },
                })
                .is_err()
            {
                break;
            }
        }
        println!("stalled delivery: {:?}", rx.metrics());
        assert_eq!(rx.metrics().overflows, 1);
    }
}
