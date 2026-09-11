//! Bounded preparation admission with payload-free, nonblocking observation.
use std::{sync::Mutex, time::Instant};

#[derive(Default)]
struct WorkerState {
    busy: bool,
    generation: u64,
    changed_us: u64,
}

pub(super) struct WorkerStaging {
    started: Instant,
    state: Mutex<WorkerState>,
}

impl Default for WorkerStaging {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            state: Mutex::new(WorkerState::default()),
        }
    }
}

pub(super) struct StagingAdmission<'a>(&'a WorkerStaging);

impl WorkerStaging {
    fn uptime_us(&self) -> u64 {
        self.started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
    }

    pub fn acquire(&self) -> Result<StagingAdmission<'_>, String> {
        let mut state = self
            .state
            .try_lock()
            .map_err(|_| "background preparation is busy or unavailable")?;
        if state.busy {
            return Err("background preparation is busy or unavailable".into());
        }
        state.busy = true;
        state.generation = state.generation.saturating_add(1);
        state.changed_us = self.uptime_us();
        Ok(StagingAdmission(self))
    }

    pub fn snapshot(
        &self,
    ) -> Option<nickel_remote_control::diagnostics::BackgroundWorkerDiagnostic> {
        let state = self.state.try_lock().ok()?;
        Some(
            nickel_remote_control::diagnostics::BackgroundWorkerDiagnostic {
                generation: state.generation,
                collector_uptime_us: self.uptime_us(),
                last_changed_uptime_us: state.changed_us,
                busy: state.busy,
            },
        )
    }

    #[cfg(test)]
    pub(super) fn with_snapshot_state_held<T>(&self, effect: impl FnOnce() -> T) -> T {
        let _state = self.state.lock().unwrap();
        effect()
    }
}

impl Drop for StagingAdmission<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.busy = false;
            state.generation = state.generation.saturating_add(1);
            state.changed_us = self.0.uptime_us();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contention_does_not_block_diagnostics_or_admission() {
        let worker = std::sync::Arc::new(WorkerStaging::default());
        let held = worker.state.lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = worker.clone();
        let thread = std::thread::spawn(move || {
            let unavailable = reader.snapshot().is_none();
            let rejected = reader.acquire().is_err();
            tx.send((unavailable, rejected)).unwrap();
        });
        let result = rx.recv_timeout(std::time::Duration::from_secs(1));
        // Release before asserting so a regression cannot strand the worker.
        drop(held);
        thread.join().unwrap();
        assert_eq!(result.unwrap(), (true, true));
        assert!(worker.snapshot().is_some());
        assert!(worker.acquire().is_ok());
    }

    #[test]
    fn admission_drop_releases_capacity_and_preserves_observation_order() {
        let worker = WorkerStaging::default();
        let before = worker.snapshot().unwrap();
        assert!(!before.busy);
        let admission = worker.acquire().unwrap();
        let admitted = worker.snapshot().unwrap();
        assert!(admitted.busy);
        assert!(admitted.generation > before.generation);
        assert!(worker.acquire().is_err());
        drop(admission);
        let released = worker.snapshot().unwrap();
        assert!(!released.busy);
        assert!(released.generation > admitted.generation);
        assert!(released.last_changed_uptime_us >= admitted.last_changed_uptime_us);
        assert!(released.collector_uptime_us >= released.last_changed_uptime_us);
        assert!(worker.acquire().is_ok());
    }
}
