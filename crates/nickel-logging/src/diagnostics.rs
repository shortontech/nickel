//! Payload-free warning/error history. Never visits event fields or span contents.
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};
use tracing_subscriber::Layer;

pub const MAX_RECORDS: usize = 256;
#[derive(Clone, Debug)]
pub struct Record {
    pub generation: u64,
    pub observed_at_us: u64,
    pub level: &'static str,
    pub target: &'static str,
    pub file: Option<&'static str>,
    pub line: Option<u32>,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub collecting: bool,
    pub generation: u64,
    pub observed_at_us: u64,
    pub evicted: u64,
    pub contention_drops: u64,
    pub records: Vec<Record>,
}
#[derive(Default)]
struct History {
    generation: u64,
    evicted: u64,
    records: VecDeque<Record>,
}
struct Collector {
    started: Instant,
    collecting: AtomicBool,
    drops: AtomicU64,
    history: Mutex<History>,
}
impl Default for Collector {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            collecting: AtomicBool::new(false),
            drops: AtomicU64::new(0),
            history: Mutex::new(History::default()),
        }
    }
}
impl Collector {
    fn elapsed(&self) -> u64 {
        self.started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
    }
    fn snapshot(&self) -> Option<Snapshot> {
        let history = self.history.try_lock().ok()?;
        Some(Snapshot {
            collecting: self.collecting.load(Ordering::Relaxed),
            generation: history.generation,
            observed_at_us: self.elapsed(),
            evicted: history.evicted,
            contention_drops: self.drops.load(Ordering::Relaxed),
            records: history.records.iter().cloned().collect(),
        })
    }
}
static COLLECTOR: OnceLock<Arc<Collector>> = OnceLock::new();
pub fn snapshot() -> Option<Snapshot> {
    COLLECTOR.get()?.snapshot()
}
pub(crate) fn mark_initialized() {
    if let Some(collector) = COLLECTOR.get() {
        collector.collecting.store(true, Ordering::Relaxed);
    }
}
pub(crate) struct DiagnosticLayer(Arc<Collector>);
pub(crate) fn layer() -> DiagnosticLayer {
    DiagnosticLayer(COLLECTOR.get_or_init(Default::default).clone())
}
impl<S: tracing::Subscriber> Layer<S> for DiagnosticLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let metadata = event.metadata();
        if !matches!(
            *metadata.level(),
            tracing::Level::WARN | tracing::Level::ERROR
        ) || !super::diagnostic_metadata_allowed(metadata)
        {
            return;
        }
        let Ok(mut history) = self.0.history.try_lock() else {
            self.0.drops.fetch_add(1, Ordering::Relaxed);
            return;
        };
        history.generation = history.generation.saturating_add(1);
        let generation = history.generation;
        if history.records.len() == MAX_RECORDS {
            history.records.pop_front();
            history.evicted = history.evicted.saturating_add(1);
        }
        history.records.push_back(Record {
            generation,
            observed_at_us: self.0.elapsed(),
            level: metadata.level().as_str(),
            target: metadata.target(),
            file: metadata.file(),
            line: metadata.line(),
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::prelude::*;
    #[test]
    fn production_events_retain_only_bounded_source_metadata() {
        struct Secret;
        impl std::fmt::Debug for Secret {
            fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                panic!("diagnostic collector visited sensitive payload")
            }
        }
        let collector = Arc::new(Collector::default());
        let subscriber = tracing_subscriber::registry().with(
            DiagnosticLayer(collector.clone())
                .with_filter(tracing_subscriber::filter::LevelFilter::WARN),
        );
        let info_evaluated = std::cell::Cell::new(false);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(
                unused = {
                    info_evaluated.set(true);
                    1
                },
                "not a warning"
            );
            tracing::warn!(target: "rmcp", secret = ?Secret, "excluded dependency");
            tracing::warn!(target: "smithay::input::keyboard", secret = ?Secret, "excluded input");
            for _ in 0..300 {
                tracing::warn!(secret = ?Secret, "sensitive message content");
            }
        });
        assert!(
            !info_evaluated.get(),
            "collector must not enable lower-level field evaluation"
        );
        let snapshot = collector.snapshot().unwrap();
        assert_eq!(snapshot.generation, 300);
        assert_eq!(snapshot.evicted, 44);
        assert_eq!(snapshot.records.len(), MAX_RECORDS);
        assert_eq!(snapshot.records.first().unwrap().generation, 45);
        assert!(snapshot.records.iter().all(|record| record.level == "WARN"
            && record.file.is_some()
            && record.line.is_some()));
        assert!(!format!("{snapshot:?}").contains("sensitive message content"));
        assert!(
            snapshot
                .records
                .windows(2)
                .all(|pair| pair[0].observed_at_us <= pair[1].observed_at_us)
        );
        let held = collector.history.lock().unwrap();
        tracing::subscriber::with_default(
            tracing_subscriber::registry().with(DiagnosticLayer(collector.clone())),
            || tracing::error!("must not block"),
        );
        assert!(collector.snapshot().is_none());
        drop(held);
        assert_eq!(collector.snapshot().unwrap().contention_drops, 1);
    }
}
