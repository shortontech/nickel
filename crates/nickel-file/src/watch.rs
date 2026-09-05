use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

use notify::{
    EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{ModifyKind, RenameMode},
};

const MAX_SHARED_WATCHES: usize = 128;
const MAX_RETAINED_EVENTS: usize = 64;
const MAX_EVENT_PATHS: usize = 8;

/// Portable provider events. Native backend records never escape this boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderEventKind {
    Create,
    Remove,
    Rename,
    Content,
    Metadata,
    Ambiguous,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderEvent {
    pub kind: ProviderEventKind,
    pub paths: Vec<PathBuf>,
}

#[derive(Default)]
struct WatchState {
    revision: u64,
    failure_revision: u64,
    failure: Option<String>,
    events: VecDeque<ProviderEvent>,
}

struct SharedWatch {
    path: PathBuf,
    state: Arc<Mutex<WatchState>>,
    _watcher: Option<RecommendedWatcher>,
}

#[derive(Default)]
struct WatchRegistry {
    watches: HashMap<PathBuf, Weak<SharedWatch>>,
}

fn registry() -> &'static Mutex<WatchRegistry> {
    static REGISTRY: OnceLock<Mutex<WatchRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(WatchRegistry::default()))
}

fn classify(kind: &EventKind) -> Option<ProviderEventKind> {
    match kind {
        EventKind::Access(_) => None,
        EventKind::Create(_) => Some(ProviderEventKind::Create),
        EventKind::Remove(_) => Some(ProviderEventKind::Remove),
        EventKind::Modify(ModifyKind::Name(
            RenameMode::Any
            | RenameMode::From
            | RenameMode::To
            | RenameMode::Both
            | RenameMode::Other,
        )) => Some(ProviderEventKind::Rename),
        EventKind::Modify(ModifyKind::Data(_)) => Some(ProviderEventKind::Content),
        EventKind::Modify(ModifyKind::Metadata(_)) => Some(ProviderEventKind::Metadata),
        EventKind::Modify(ModifyKind::Any | ModifyKind::Other)
        | EventKind::Any
        | EventKind::Other => Some(ProviderEventKind::Ambiguous),
    }
}

impl SharedWatch {
    fn start(path: PathBuf) -> Result<Arc<Self>, String> {
        let state = Arc::new(Mutex::new(WatchState {
            revision: 1,
            ..WatchState::default()
        }));
        let callback_state = Arc::clone(&state);
        let mut watcher = notify::recommended_watcher(
            move |result: notify::Result<notify::Event>| match result {
                Ok(event) => {
                    let Some(kind) = classify(&event.kind) else {
                        return;
                    };
                    if let Ok(mut state) = callback_state.lock() {
                        state.revision = state.revision.wrapping_add(1).max(1);
                        if state.events.len() == MAX_RETAINED_EVENTS {
                            state.events.pop_front();
                            state.events.push_back(ProviderEvent {
                                kind: ProviderEventKind::Ambiguous,
                                paths: Vec::new(),
                            });
                        }
                        if state.events.len() < MAX_RETAINED_EVENTS {
                            state.events.push_back(ProviderEvent {
                                kind,
                                paths: event.paths.into_iter().take(MAX_EVENT_PATHS).collect(),
                            });
                        }
                    }
                }
                Err(error) => {
                    if let Ok(mut state) = callback_state.lock() {
                        state.revision = state.revision.wrapping_add(1).max(1);
                        state.failure_revision = state.failure_revision.wrapping_add(1).max(1);
                        state.failure = Some(error.to_string());
                    }
                }
            },
        )
        .map_err(|error| error.to_string())?;
        watcher
            .watch(&path, RecursiveMode::NonRecursive)
            .map_err(|error| error.to_string())?;
        Ok(Arc::new(Self {
            path,
            state,
            _watcher: Some(watcher),
        }))
    }
}

/// A reference-counted subscription to one canonical local-directory watch.
pub struct DirectoryWatch {
    shared: Arc<SharedWatch>,
    seen_revision: AtomicU64,
    seen_failure_revision: AtomicU64,
}

impl DirectoryWatch {
    pub fn start(path: &Path) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|error| error.to_string())?;
        let mut registry = registry()
            .lock()
            .map_err(|_| "directory watch registry is unavailable".to_owned())?;
        registry.watches.retain(|_, watch| watch.strong_count() > 0);
        let shared = if let Some(shared) = registry.watches.get(&path).and_then(Weak::upgrade) {
            shared
        } else {
            if registry.watches.len() >= MAX_SHARED_WATCHES {
                return Err(format!(
                    "live directory watch limit ({MAX_SHARED_WATCHES}) reached"
                ));
            }
            let shared = SharedWatch::start(path.clone())?;
            registry.watches.insert(path, Arc::downgrade(&shared));
            shared
        };
        Ok(Self {
            shared,
            seen_revision: AtomicU64::new(0),
            seen_failure_revision: AtomicU64::new(0),
        })
    }

    pub fn watches(&self, path: &Path) -> bool {
        path.canonicalize()
            .is_ok_and(|path| path == self.shared.path)
    }

    pub fn take_invalidation(&self) -> bool {
        let revision = self
            .shared
            .state
            .lock()
            .map(|state| state.revision)
            .unwrap_or_else(|_| self.seen_revision.load(Ordering::Acquire).wrapping_add(1));
        self.seen_revision.swap(revision, Ordering::AcqRel) != revision
    }

    pub fn take_failure(&self) -> Option<String> {
        let state = self.shared.state.lock().ok()?;
        let revision = state.failure_revision;
        if self.seen_failure_revision.swap(revision, Ordering::AcqRel) == revision {
            return None;
        }
        state.failure.clone()
    }

    pub fn recent_events(&self) -> Vec<ProviderEvent> {
        self.shared
            .state
            .lock()
            .map(|state| state.events.iter().cloned().collect())
            .unwrap_or_default()
    }

    #[cfg(test)]
    fn shares_backend_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }

    #[cfg(test)]
    pub(crate) fn fixture(path: PathBuf) -> Self {
        Self {
            shared: Arc::new(SharedWatch {
                path,
                state: Arc::new(Mutex::new(WatchState {
                    revision: 1,
                    ..WatchState::default()
                })),
                _watcher: None,
            }),
            seen_revision: AtomicU64::new(0),
            seen_failure_revision: AtomicU64::new(0),
        }
    }

    #[cfg(test)]
    pub(crate) fn inject_failure(&self, message: &str) {
        let mut state = self.shared.state.lock().unwrap();
        state.revision = state.revision.wrapping_add(1).max(1);
        state.failure_revision = state.failure_revision.wrapping_add(1).max(1);
        state.failure = Some(message.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::{DirectoryWatch, MAX_RETAINED_EVENTS, ProviderEventKind};
    use std::{fs, path::PathBuf, sync::Arc, thread, time::Duration};

    fn wait_for_invalidation(watch: &DirectoryWatch) {
        for _ in 0..100 {
            if watch.take_invalidation() {
                // Native backends may emit several records for one filesystem
                // operation. Let that burst quiesce so the next operation cannot
                // accidentally consume a delayed edge from this one.
                for _ in 0..5 {
                    thread::sleep(Duration::from_millis(10));
                    let _ = watch.take_invalidation();
                }
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("native directory watch did not invalidate within one second");
    }

    #[test]
    fn native_watch_classifies_or_conservatively_invalidates_file_changes() {
        let directory = tempfile::tempdir().unwrap();
        let watch = DirectoryWatch::start(directory.path()).unwrap();
        assert!(watch.take_invalidation());
        let first = directory.path().join("first.txt");
        let renamed = directory.path().join("renamed.txt");
        let start = watch.recent_events().len();
        fs::write(&first, b"one").unwrap();
        wait_for_invalidation(&watch);
        let events = watch.recent_events();
        assert!(events[start..].iter().any(|event| matches!(
            event.kind,
            ProviderEventKind::Create | ProviderEventKind::Ambiguous
        )));
        let start = events.len();
        fs::write(&first, b"two").unwrap();
        wait_for_invalidation(&watch);
        let events = watch.recent_events();
        assert!(events[start..].iter().any(|event| matches!(
            event.kind,
            ProviderEventKind::Content | ProviderEventKind::Metadata | ProviderEventKind::Ambiguous
        )));
        let start = events.len();
        fs::rename(&first, &renamed).unwrap();
        wait_for_invalidation(&watch);
        let events = watch.recent_events();
        assert!(events[start..].iter().any(|event| matches!(
            event.kind,
            ProviderEventKind::Rename | ProviderEventKind::Ambiguous
        )));
        let start = events.len();
        fs::remove_file(&renamed).unwrap();
        wait_for_invalidation(&watch);
        let events = watch.recent_events();
        assert!(events[start..].iter().any(|event| matches!(
            event.kind,
            ProviderEventKind::Remove | ProviderEventKind::Ambiguous
        )));
        assert!(watch.take_failure().is_none());
    }

    #[test]
    fn subscribers_share_backend_and_consume_revisions_independently() {
        let directory = tempfile::tempdir().unwrap();
        let first = DirectoryWatch::start(directory.path()).unwrap();
        let second = DirectoryWatch::start(directory.path()).unwrap();
        assert!(first.shares_backend_with(&second));
        let backend = Arc::downgrade(&first.shared);
        assert!(first.take_invalidation());
        assert!(second.take_invalidation());
        fs::write(directory.path().join("shared.txt"), b"shared").unwrap();
        wait_for_invalidation(&first);
        assert!(second.take_invalidation());
        drop(first);
        assert!(backend.upgrade().is_some());
        drop(second);
        assert!(backend.upgrade().is_none());
    }

    #[test]
    fn watch_does_not_observe_sibling_directory() {
        let parent = tempfile::tempdir().unwrap();
        let watched = parent.path().join("watched");
        let sibling = parent.path().join("sibling");
        fs::create_dir(&watched).unwrap();
        fs::create_dir(&sibling).unwrap();
        let watch = DirectoryWatch::start(&watched).unwrap();
        assert!(watch.take_invalidation());
        fs::write(sibling.join("outside.txt"), b"outside").unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(!watch.take_invalidation());
    }

    #[test]
    fn retained_diagnostics_are_bounded_during_burst() {
        let directory = tempfile::tempdir().unwrap();
        let watch = DirectoryWatch::start(directory.path()).unwrap();
        assert!(watch.take_invalidation());
        for index in 0..MAX_RETAINED_EVENTS * 3 {
            fs::write(directory.path().join(format!("{index}.txt")), b"event").unwrap();
        }
        wait_for_invalidation(&watch);
        thread::sleep(Duration::from_millis(100));
        assert!(watch.recent_events().len() <= MAX_RETAINED_EVENTS);
    }

    #[test]
    fn deterministic_backend_reports_failure_once_per_subscription() {
        let watch = DirectoryWatch::fixture(PathBuf::from("/fixture"));
        assert!(watch.take_invalidation());
        watch.inject_failure("queue overflow");
        assert!(watch.take_invalidation());
        assert_eq!(watch.take_failure().as_deref(), Some("queue overflow"));
        assert_eq!(watch.take_failure(), None);
    }
}
