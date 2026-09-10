//! Bounded admission without request payloads, addresses, or identity labels in metrics.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub(crate) const MAX_REQUEST_BYTES: usize = 4096;
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

struct Gate {
    active: usize,
    tokens: f64,
    updated: Instant,
}

impl Gate {
    fn new(now: Instant, burst: usize) -> Self {
        Self {
            active: 0,
            tokens: burst as f64,
            updated: now,
        }
    }

    fn acquire(&mut self, now: Instant, concurrency: usize, burst: usize) -> bool {
        self.tokens = (self.tokens
            + now.saturating_duration_since(self.updated).as_secs_f64() * burst as f64 / 2.0)
            .min(burst as f64);
        self.updated = now;
        if self.active >= concurrency || self.tokens < 1.0 {
            return false;
        }
        self.active += 1;
        self.tokens -= 1.0;
        true
    }
}

struct State {
    generation: u64,
    started: Instant,
    global: Gate,
    clients: HashMap<String, Gate>,
    admitted: u64,
    rejected: u64,
}

pub(crate) struct AdmissionLimits(Mutex<State>);

impl AdmissionLimits {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(State {
            generation: 0,
            started: Instant::now(),
            global: Gate::new(Instant::now(), 128),
            clients: HashMap::new(),
            admitted: 0,
            rejected: 0,
        })))
    }

    /// Client IDs must already have passed authentication. No attacker-selected
    /// header value is retained by the per-client table.
    pub(crate) fn acquire(
        self: &Arc<Self>,
        client: Option<&str>,
        now: Instant,
    ) -> Option<Admission> {
        let mut state = self.0.lock().unwrap();
        state.generation = state.generation.saturating_add(1);
        let accepted = if let Some(client) = client {
            state.clients.retain(|_, gate| {
                gate.active > 0
                    || now.saturating_duration_since(gate.updated) < Duration::from_secs(60)
            });
            if !state.clients.contains_key(client) && state.clients.len() >= 128 {
                false
            } else {
                state
                    .clients
                    .entry(client.to_owned())
                    .or_insert_with(|| Gate::new(now, 32))
                    .acquire(now, 4, 32)
            }
        } else {
            state.global.acquire(now, 32, 128)
        };
        if !accepted {
            state.rejected = state.rejected.saturating_add(1);
            return None;
        }
        if client.is_none() {
            state.admitted = state.admitted.saturating_add(1);
        }
        Some(Admission {
            limits: self.clone(),
            client: client.map(str::to_owned),
        })
    }

    /// Never make the compositor wait for transport admission bookkeeping.
    pub(crate) fn snapshot(&self) -> Option<crate::diagnostics::AdmissionDiagnostic> {
        let state = self.0.try_lock().ok()?;
        Some(crate::diagnostics::AdmissionDiagnostic {
            generation: state.generation,
            collector_uptime_us: state.started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            requests_admitted: state.admitted,
            admission_rejections: state.rejected,
            active_requests: state.global.active as u64,
            active_authenticated_requests: state
                .clients
                .values()
                .map(|gate| gate.active as u64)
                .sum(),
        })
    }

    pub(crate) fn metrics(&self) -> String {
        let state = self.0.lock().unwrap();
        format!(
            "# TYPE nickel_mcp_requests_admitted_total counter\nnickel_mcp_requests_admitted_total {}\n# TYPE nickel_mcp_admission_rejections_total counter\nnickel_mcp_admission_rejections_total {}\n# TYPE nickel_mcp_requests_active gauge\nnickel_mcp_requests_active {}\n",
            state.admitted, state.rejected, state.global.active
        )
    }
}

pub(crate) struct Admission {
    limits: Arc<AdmissionLimits>,
    client: Option<String>,
}

impl Drop for Admission {
    fn drop(&mut self) {
        let mut state = self.limits.0.lock().unwrap();
        let gate = match &self.client {
            Some(client) => state
                .clients
                .get_mut(client)
                .expect("active client retained"),
            None => &mut state.global,
        };
        gate.active -= 1;
        state.generation = state.generation.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_snapshot_tracks_production_admission_and_never_waits_on_contention() {
        let limits = AdmissionLimits::new();
        let now = Instant::now();
        let initial = limits.snapshot().unwrap();
        let global = limits.acquire(None, now).unwrap();
        let clients: Vec<_> = (0..4)
            .map(|_| limits.acquire(Some("private-client-canary"), now).unwrap())
            .collect();
        assert!(limits.acquire(Some("private-client-canary"), now).is_none());
        let active = limits.snapshot().unwrap();
        assert!(active.generation > initial.generation);
        assert!(active.collector_uptime_us >= initial.collector_uptime_us);
        assert_eq!(active.requests_admitted, 1);
        assert_eq!(active.admission_rejections, 1);
        assert_eq!(active.active_requests, 1);
        assert_eq!(active.active_authenticated_requests, 4);
        assert!(
            !serde_json::to_string(&active)
                .unwrap()
                .contains("private-client-canary")
        );
        drop(clients);
        drop(global);
        let idle = limits.snapshot().unwrap();
        assert!(idle.generation > active.generation);
        assert_eq!(idle.active_requests, 0);
        assert_eq!(idle.active_authenticated_requests, 0);
        let held = limits.0.lock().unwrap();
        let other = limits.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || tx.send(other.snapshot().is_none()).unwrap());
        let response = rx.recv_timeout(Duration::from_secs(1));
        drop(held);
        worker.join().unwrap();
        assert_eq!(
            response,
            Ok(true),
            "busy admission must report unavailable before unlock"
        );
        assert!(limits.snapshot().is_some());
    }

    #[test]
    fn independent_clients_cannot_exceed_shared_or_individual_capacity() {
        let limits = AdmissionLimits::new();
        let now = Instant::now();
        let held: Vec<_> = (0..32)
            .map(|_| limits.acquire(None, now).unwrap())
            .collect();
        assert!(limits.acquire(None, now).is_none());
        drop(held);
        assert!(limits.acquire(None, now).is_some());
        let client: Vec<_> = (0..4)
            .map(|_| limits.acquire(Some("authenticated-a"), now).unwrap())
            .collect();
        assert!(limits.acquire(Some("authenticated-a"), now).is_none());
        assert!(limits.acquire(Some("authenticated-b"), now).is_some());
        drop(client);
        assert!(limits.acquire(Some("authenticated-a"), now).is_some());
        assert!(!limits.metrics().contains("authenticated-"));
    }

    #[test]
    fn rate_budget_refills_without_accumulating_client_records() {
        let limits = AdmissionLimits::new();
        let now = Instant::now();
        for _ in 0..32 {
            drop(limits.acquire(Some("agent"), now).unwrap());
        }
        assert!(limits.acquire(Some("agent"), now).is_none());
        assert!(
            limits
                .acquire(Some("agent"), now + Duration::from_secs(2))
                .is_some()
        );
        for id in 0..127 {
            drop(limits.acquire(Some(&id.to_string()), now).unwrap());
        }
        assert!(limits.acquire(Some("overflow"), now).is_none());
        assert!(
            limits
                .acquire(Some("new"), now + Duration::from_secs(63))
                .is_some()
        );
        assert_eq!(limits.0.lock().unwrap().clients.len(), 1);
    }
}
