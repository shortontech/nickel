use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// A narrow provider-watch boundary. Native records are deliberately collapsed into an
/// invalidation: the directory snapshot remains the source of truth on every platform.
pub struct DirectoryWatch {
    path: PathBuf,
    dirty: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
    _watcher: RecommendedWatcher,
}

impl DirectoryWatch {
    pub fn start(path: &Path) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|error| error.to_string())?;
        let dirty = Arc::new(AtomicBool::new(true));
        let callback_dirty = Arc::clone(&dirty);
        let failure = Arc::new(Mutex::new(None));
        let callback_failure = Arc::clone(&failure);
        let mut watcher = notify::recommended_watcher(
            move |result: notify::Result<notify::Event>| match result {
                Ok(_event) => callback_dirty.store(true, Ordering::Release),
                Err(error) => {
                    callback_dirty.store(true, Ordering::Release);
                    if let Ok(mut failure) = callback_failure.lock() {
                        *failure = Some(error.to_string());
                    }
                }
            },
        )
        .map_err(|error| error.to_string())?;
        watcher
            .watch(&path, RecursiveMode::NonRecursive)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            path,
            dirty,
            failure,
            _watcher: watcher,
        })
    }

    pub fn watches(&self, path: &Path) -> bool {
        path.canonicalize().is_ok_and(|path| path == self.path)
    }

    pub fn take_invalidation(&self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }

    pub fn take_failure(&self) -> Option<String> {
        self.failure.lock().ok()?.take()
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, thread, time::Duration};

    use super::DirectoryWatch;

    fn wait_for_invalidation(watch: &DirectoryWatch) {
        for _ in 0..100 {
            if watch.take_invalidation() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("native directory watch did not invalidate within one second");
    }

    #[test]
    fn native_watch_invalidates_for_create_rename_and_remove() {
        let directory = tempfile::tempdir().unwrap();
        let watch = DirectoryWatch::start(directory.path()).unwrap();
        assert!(
            watch.take_invalidation(),
            "registration requires reconciliation"
        );

        let first = directory.path().join("first.txt");
        let renamed = directory.path().join("renamed.txt");
        fs::write(&first, b"one").unwrap();
        wait_for_invalidation(&watch);
        fs::rename(&first, &renamed).unwrap();
        wait_for_invalidation(&watch);
        fs::remove_file(&renamed).unwrap();
        wait_for_invalidation(&watch);
        assert!(watch.take_failure().is_none());
    }

    #[test]
    fn a_watch_does_not_observe_a_sibling_directory() {
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
}
