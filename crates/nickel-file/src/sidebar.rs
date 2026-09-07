//! Window-owned sidebar listings. Collapsed subtrees retain no snapshots or watches.
//!
//! At most 16 listings retain 256 children / 256 KiB of string and path capacity
//! each (4 MiB total plus fixed entry/container overhead). One worker and a one-slot
//! result channel add at most one listing; sorting adds bounded lowercase keys.
//! Each scan examines at most 8,192 entries. Partial listings are explicit; opening
//! the folder uses the normal directory browser for the complete contents.
//!
//! Sidebar ownership spans tab navigation because expanded places remain visible.
//! Folder/group collapse and disappearance retire descendants, watches and requests;
//! window drop cancels the worker. Cancellation is checked between filesystem calls:
//! a blocked native read cannot be preempted, and retains its sole worker slot.
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};

use crate::watch::DirectoryWatch;

const MAX_FOLDERS: usize = 16;
const MAX_CHILDREN: usize = 256;
const MAX_LISTING_BYTES: usize = 256 * 1024;
const MAX_SCANNED_ENTRIES: usize = 8192;
const MAX_PATH_BYTES: usize = 4096;
const RETRY_INTERVAL: Duration = Duration::from_secs(2);

type Children = Vec<(String, PathBuf)>;

struct Listing {
    children: Children,
    partial: bool,
}

struct Folder {
    generation: u64,
    watch: Option<DirectoryWatch>,
    retry_at: Option<Instant>,
    warning: Option<String>,
}

struct Job {
    path: PathBuf,
    generation: u64,
    cancelled: Arc<AtomicBool>,
    receiver: Receiver<Result<Listing, String>>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

#[derive(Default)]
pub(crate) struct Sidebar {
    pub(crate) expanded: HashSet<PathBuf>,
    pub(crate) children: HashMap<PathBuf, Children>,
    folders: HashMap<PathBuf, Folder>,
    pending: VecDeque<PathBuf>,
    job: Option<Job>,
    generation: u64,
    admission_warning: Option<String>,
}

impl Sidebar {
    pub(crate) fn toggle(&mut self, path: PathBuf) {
        if self.expanded.contains(&path) {
            self.collapse(&path);
            return;
        }
        if self.folders.len() == MAX_FOLDERS || path.as_os_str().len() > MAX_PATH_BYTES {
            self.admission_warning = Some("Sidebar limit reached. Collapse a folder before expanding another, or open the folder to browse it.".into());
            return;
        }
        self.admission_warning = None;
        self.generation = self.generation.wrapping_add(1);
        self.expanded.insert(path.clone());
        self.folders.insert(
            path.clone(),
            Folder {
                generation: self.generation,
                watch: None,
                retry_at: None,
                warning: None,
            },
        );
        self.observe(&path);
        self.enqueue(path);
        self.start_next();
    }

    pub(crate) fn collapse(&mut self, path: &Path) {
        self.expanded
            .retain(|candidate| !candidate.starts_with(path));
        self.children
            .retain(|candidate, _| !candidate.starts_with(path));
        self.folders
            .retain(|candidate, _| !candidate.starts_with(path));
        self.pending
            .retain(|candidate| !candidate.starts_with(path));
        if let Some(job) = &self.job
            && job.path.starts_with(path)
        {
            // Keep the slot until the worker exits, even when the user immediately
            // reopens a folder. Cancellation must not permit unbounded workers.
            job.cancelled.store(true, Ordering::Release);
        }
        self.admission_warning = None;
    }

    fn observe(&mut self, path: &Path) {
        let Some(folder) = self.folders.get_mut(path) else {
            return;
        };
        match DirectoryWatch::start(path) {
            Ok(watch) => {
                // Subscribe before enumeration. Later invalidations remain pending
                // until the result arrives, closing the registration/snapshot race.
                watch.take_invalidation();
                folder.watch = Some(watch);
                folder.retry_at = None;
            }
            Err(error) => {
                folder.warning = Some(format!(
                    "Sidebar live updates unavailable for {}: {error}. Retrying; Refresh is available.",
                    path.display()
                ));
                folder.watch = None;
                folder.retry_at = Some(Instant::now() + RETRY_INTERVAL);
            }
        }
    }

    fn enqueue(&mut self, path: PathBuf) {
        if !self.pending.contains(&path) {
            self.pending.push_back(path);
        }
    }

    pub(crate) fn retain_visible_roots<'a>(&mut self, roots: impl Iterator<Item = &'a PathBuf>) {
        let mut visible = roots.cloned().collect::<HashSet<_>>();
        // A descendant remains reachable only through expanded, retained parents.
        // This also handles a disappearing mount that is lexically below another
        // place but whose ancestors were never expanded in the sidebar.
        loop {
            let before = visible.len();
            for (path, children) in &self.children {
                if visible.contains(path) && self.expanded.contains(path) {
                    visible.extend(children.iter().map(|(_, path)| path.clone()));
                }
            }
            if visible.len() == before {
                break;
            }
        }
        let retired = self
            .expanded
            .iter()
            .filter(|path| !visible.contains(*path))
            .cloned()
            .collect::<Vec<_>>();
        for path in retired {
            // Retire the individual unreachable row. A separately listed place
            // can still expose one of its descendants as an independent root.
            self.expanded.remove(&path);
            self.children.remove(&path);
            self.folders.remove(&path);
            self.pending.retain(|pending| pending != &path);
            if let Some(job) = &self.job
                && job.path == path
            {
                job.cancelled.store(true, Ordering::Release);
            }
        }
    }

    pub(crate) fn refresh(&mut self) {
        if let Some(job) = &self.job {
            job.cancelled.store(true, Ordering::Release);
        }
        for path in self.folders.keys().cloned().collect::<Vec<_>>() {
            self.generation = self.generation.wrapping_add(1);
            self.folders.get_mut(&path).unwrap().generation = self.generation;
            self.observe(&path);
            self.enqueue(path);
        }
        self.start_next();
    }

    fn start_next(&mut self) {
        if self.job.is_some() {
            return;
        }
        let Some(path) = self.pending.pop_front() else {
            return;
        };
        let Some(folder) = self.folders.get_mut(&path) else {
            return;
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let worker_path = path.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("nickel-file-sidebar".into())
            .spawn(move || {
                let result = enumerate(&worker_path, &worker_cancelled);
                if !worker_cancelled.load(Ordering::Acquire) {
                    let _ = sender.send(result);
                }
            }) {
            Ok(_) => {
                self.job = Some(Job {
                    path,
                    generation: folder.generation,
                    cancelled,
                    receiver,
                })
            }
            Err(error) => {
                folder.warning = Some(format!(
                    "Sidebar listing unavailable: {error}. Refresh to retry."
                ));
                folder.retry_at = Some(Instant::now() + RETRY_INTERVAL);
            }
        }
    }

    pub(crate) fn poll(&mut self) -> bool {
        let mut changed = false;
        let result = self
            .job
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err("enumeration stopped".into())),
            });
        if let Some(result) = result {
            let job = self.job.take().expect("result belongs to active job");
            if !job.cancelled.load(Ordering::Acquire)
                && let Some(folder) = self.folders.get_mut(&job.path)
                && folder.generation == job.generation
            {
                match result {
                    Ok(listing) => {
                        if folder.watch.is_some() {
                            folder.warning = listing.partial.then(|| format!("Sidebar listing for {} is partial. Open the folder to browse all entries.", job.path.display()));
                        }
                        let removed = self
                            .expanded
                            .iter()
                            .filter(|path| {
                                path.parent() == Some(job.path.as_path())
                                    && !listing.children.iter().any(|(_, child)| child == *path)
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        for path in removed {
                            self.collapse(&path);
                        }
                        self.children.insert(job.path.clone(), listing.children);
                    }
                    Err(error) => {
                        folder.warning = Some(format!(
                            "Sidebar listing for {} is stale: {error}. Retrying; Refresh is available.",
                            job.path.display()
                        ));
                        folder.watch = None;
                        folder.retry_at = Some(Instant::now() + RETRY_INTERVAL);
                    }
                }
                changed = true;
            }
        }
        for path in self.folders.keys().cloned().collect::<Vec<_>>() {
            let folder = self.folders.get_mut(&path).unwrap();
            if let Some(error) = folder.watch.as_ref().and_then(DirectoryWatch::take_failure) {
                folder.warning = Some(format!(
                    "Sidebar listing for {} may be stale: {error}. Retrying; Refresh is available.",
                    path.display()
                ));
                folder.watch = None;
                folder.retry_at = Some(Instant::now() + RETRY_INTERVAL);
                self.enqueue(path.clone());
                changed = true;
            }
            let folder = self.folders.get(&path).unwrap();
            if folder.retry_at.is_some_and(|at| Instant::now() >= at) {
                self.observe(&path);
                self.enqueue(path.clone());
            } else if self.job.as_ref().is_none_or(|job| job.path != path)
                && folder
                    .watch
                    .as_ref()
                    .is_some_and(DirectoryWatch::take_invalidation)
            {
                self.enqueue(path);
            }
        }
        self.start_next();
        changed
    }

    pub(crate) fn poll_interval(&self) -> Option<Duration> {
        if self.job.is_some() || !self.pending.is_empty() {
            Some(Duration::from_millis(16))
        } else if !self.folders.is_empty() {
            Some(Duration::from_millis(100))
        } else {
            None
        }
    }

    pub(crate) fn warning(&self) -> Option<&str> {
        self.admission_warning.as_deref().or_else(|| {
            self.folders
                .values()
                .find_map(|folder| folder.warning.as_deref())
        })
    }

    #[cfg(test)]
    pub(crate) fn loading(&self, path: &Path) -> bool {
        self.pending.iter().any(|pending| pending == path)
            || self.job.as_ref().is_some_and(|job| job.path == path)
    }
}

fn enumerate(path: &Path, cancelled: &AtomicBool) -> Result<Listing, String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    let entries = std::fs::read_dir(path).map_err(|error| error.to_string())?;
    let mut children = Vec::with_capacity(MAX_CHILDREN);
    let mut bytes = 0;
    let mut partial = false;
    for (index, entry) in entries.enumerate() {
        if cancelled.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        if index == MAX_SCANNED_ENTRIES {
            partial = true;
            break;
        }
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let size = name.capacity() + path.capacity();
        if children.len() == MAX_CHILDREN || bytes + size > MAX_LISTING_BYTES {
            partial = true;
            break;
        }
        bytes += size;
        children.push((name, path));
    }
    children.sort_by_cached_key(|(name, _)| name.to_lowercase());
    Ok(Listing { children, partial })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, thread};

    fn wait_until(sidebar: &mut Sidebar, condition: impl Fn(&Sidebar) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            sidebar.poll();
            if condition(sidebar) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "sidebar timed out: {:?}",
                sidebar.warning()
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn names(sidebar: &Sidebar, path: &Path) -> Vec<String> {
        sidebar
            .children
            .get(path)
            .into_iter()
            .flatten()
            .map(|(name, _)| name.clone())
            .collect()
    }

    #[test]
    fn native_mutations_reconcile_expanded_children_and_retire_deleted_subtrees() {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join("before");
        fs::create_dir(&child).unwrap();
        fs::create_dir(child.join("nested")).unwrap();
        let mut sidebar = Sidebar::default();
        sidebar.toggle(root.path().to_path_buf());
        wait_until(&mut sidebar, |state| {
            names(state, root.path()) == ["before"]
        });
        sidebar.toggle(child.clone());
        wait_until(&mut sidebar, |state| names(state, &child) == ["nested"]);
        let renamed = root.path().join("after");
        fs::rename(&child, &renamed).unwrap();
        wait_until(&mut sidebar, |state| names(state, root.path()) == ["after"]);
        assert!(!sidebar.expanded.contains(&child));
        assert!(!sidebar.children.contains_key(&child));
        assert!(!sidebar.folders.contains_key(&child));
        fs::create_dir(root.path().join("created")).unwrap();
        wait_until(&mut sidebar, |state| {
            names(state, root.path()) == ["after", "created"]
        });
        fs::remove_dir(root.path().join("created")).unwrap();
        wait_until(&mut sidebar, |state| names(state, root.path()) == ["after"]);
    }

    #[test]
    fn collapse_cycles_release_snapshots_and_reopen_reads_current_contents() {
        let root = tempfile::tempdir().unwrap();
        let mut sidebar = Sidebar::default();
        for index in 0..64 {
            let path = root.path().join(format!("folder-{index}"));
            fs::create_dir(&path).unwrap();
            fs::create_dir(path.join("first")).unwrap();
            sidebar.toggle(path.clone());
            wait_until(&mut sidebar, |state| names(state, &path) == ["first"]);
            sidebar.toggle(path.clone());
            assert!(sidebar.children.is_empty());
            assert!(sidebar.expanded.is_empty());
            assert!(sidebar.folders.is_empty());
            assert!(sidebar.pending.is_empty());
            fs::rename(path.join("first"), path.join("second")).unwrap();
            sidebar.toggle(path.clone());
            wait_until(&mut sidebar, |state| names(state, &path) == ["second"]);
            sidebar.toggle(path);
        }
        wait_until(&mut sidebar, |state| state.job.is_none());
        assert!(sidebar.poll_interval().is_none());
        assert!(sidebar.children.capacity() < MAX_FOLDERS * 2);
    }

    #[test]
    fn huge_directory_bounds_result_capacity_and_reports_partial_listing() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..MAX_CHILDREN * 4 {
            fs::create_dir(root.path().join(format!("folder-{index:04}"))).unwrap();
        }
        let listing = enumerate(root.path(), &AtomicBool::new(false)).unwrap();
        assert!(listing.partial);
        assert_eq!(listing.children.len(), MAX_CHILDREN);
        assert_eq!(listing.children.capacity(), MAX_CHILDREN);
        assert!(
            listing
                .children
                .iter()
                .map(|(name, path)| name.capacity() + path.capacity())
                .sum::<usize>()
                <= MAX_LISTING_BYTES
        );
        let mut sidebar = Sidebar::default();
        sidebar.toggle(root.path().to_path_buf());
        wait_until(&mut sidebar, |state| {
            state.children.contains_key(root.path())
        });
        assert!(sidebar.warning().unwrap().contains("partial"));
    }

    #[test]
    fn files_only_directory_stops_at_scan_budget() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..MAX_SCANNED_ENTRIES + 1 {
            fs::write(root.path().join(index.to_string()), []).unwrap();
        }
        let listing = enumerate(root.path(), &AtomicBool::new(false)).unwrap();
        assert!(listing.children.is_empty());
        assert!(listing.partial);
    }

    #[test]
    fn expanded_folder_and_pending_work_counts_are_bounded() {
        let root = tempfile::tempdir().unwrap();
        let mut sidebar = Sidebar::default();
        for index in 0..MAX_FOLDERS * 3 {
            let path = root.path().join(index.to_string());
            fs::create_dir(&path).unwrap();
            sidebar.toggle(path);
        }
        assert_eq!(sidebar.folders.len(), MAX_FOLDERS);
        assert_eq!(sidebar.expanded.len(), MAX_FOLDERS);
        assert_eq!(sidebar.pending.len(), MAX_FOLDERS - 1);
        assert!(sidebar.job.is_some());
        assert!(sidebar.warning().unwrap().contains("limit"));
        wait_until(&mut sidebar, |state| {
            state.job.is_none() && state.pending.is_empty()
        });
        assert_eq!(sidebar.children.len(), MAX_FOLDERS);
    }

    #[test]
    fn watcher_failure_is_visible_until_subscription_and_snapshot_recover() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("confirmed")).unwrap();
        let mut sidebar = Sidebar::default();
        sidebar.toggle(root.path().to_path_buf());
        wait_until(&mut sidebar, |state| {
            names(state, root.path()) == ["confirmed"]
        });
        let watch = DirectoryWatch::fixture(root.path().to_path_buf());
        watch.inject_failure("queue overflow");
        sidebar.folders.get_mut(root.path()).unwrap().watch = Some(watch);
        assert!(sidebar.poll());
        assert!(sidebar.warning().unwrap().contains("stale"));
        assert_eq!(names(&sidebar, root.path()), ["confirmed"]);
        wait_until(&mut sidebar, |state| state.job.is_none());
        assert!(
            sidebar.warning().is_some(),
            "successful enumeration alone cannot repair a failed watch"
        );
        fs::create_dir(root.path().join("new")).unwrap();
        sidebar.refresh();
        wait_until(&mut sidebar, |state| {
            names(state, root.path()) == ["confirmed", "new"] && state.warning().is_none()
        });
    }

    #[test]
    fn provider_loss_preserves_confirmed_listing_and_refresh_recovers() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("folder");
        fs::create_dir(&folder).unwrap();
        fs::create_dir(folder.join("confirmed")).unwrap();
        let mut sidebar = Sidebar::default();
        sidebar.toggle(folder.clone());
        wait_until(&mut sidebar, |state| names(state, &folder) == ["confirmed"]);
        fs::rename(&folder, root.path().join("moved")).unwrap();
        sidebar.refresh();
        wait_until(&mut sidebar, |state| {
            state.job.is_none() && state.warning().is_some()
        });
        assert_eq!(names(&sidebar, &folder), ["confirmed"]);
        fs::create_dir(&folder).unwrap();
        fs::create_dir(folder.join("replacement")).unwrap();
        sidebar.refresh();
        wait_until(&mut sidebar, |state| {
            names(state, &folder) == ["replacement"] && state.warning().is_none()
        });
    }

    #[test]
    fn completed_old_generation_cannot_resurrect_collapsed_or_reopened_folder() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("old")).unwrap();
        let mut sidebar = Sidebar::default();
        sidebar.toggle(root.path().to_path_buf());
        // Complete a real provider snapshot, but hold publication until after collapse.
        let old = enumerate(root.path(), &AtomicBool::new(false)).unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        sidebar.job.as_mut().unwrap().receiver = receiver;
        sender.send(Ok(old)).unwrap();
        sidebar.toggle(root.path().to_path_buf());
        fs::rename(root.path().join("old"), root.path().join("new")).unwrap();
        sidebar.toggle(root.path().to_path_buf());
        assert_eq!(
            sidebar.pending.len(),
            1,
            "retired worker still occupies its slot"
        );
        sidebar.poll();
        assert!(!sidebar.children.contains_key(root.path()));
        wait_until(&mut sidebar, |state| names(state, root.path()) == ["new"]);
    }

    #[test]
    fn dropping_window_cancels_worker_and_disconnects_result_delivery() {
        let root = tempfile::tempdir().unwrap();
        let mut sidebar = Sidebar::default();
        sidebar.toggle(root.path().to_path_buf());
        let cancelled = sidebar.job.as_ref().unwrap().cancelled.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        sidebar.job.as_mut().unwrap().receiver = receiver;
        drop(sidebar);
        assert!(cancelled.load(Ordering::Acquire));
        assert!(
            sender
                .send(enumerate(root.path(), &AtomicBool::new(false)))
                .is_err()
        );
        assert!(enumerate(root.path(), &cancelled).is_err());
    }

    #[test]
    fn removed_place_retires_invisible_listing_but_preserves_independent_descendant_root() {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join("child");
        fs::create_dir(&child).unwrap();
        let mut sidebar = Sidebar::default();
        sidebar.toggle(root.path().to_path_buf());
        wait_until(&mut sidebar, |state| {
            state.children.contains_key(root.path())
        });
        sidebar.toggle(child.clone());
        wait_until(&mut sidebar, |state| state.children.contains_key(&child));
        sidebar.retain_visible_roots(std::iter::once(&child));
        assert!(!sidebar.children.contains_key(root.path()));
        assert!(!sidebar.folders.contains_key(root.path()));
        assert!(sidebar.children.contains_key(&child));
        assert!(sidebar.expanded.contains(&child));
        sidebar.retain_visible_roots(std::iter::empty());
        assert!(sidebar.children.is_empty());
        assert!(sidebar.folders.is_empty());
        assert!(sidebar.expanded.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn permission_loss_marks_listing_stale_and_retry_restores_it() {
        use std::os::unix::fs::PermissionsExt;
        struct RestorePermissions(PathBuf, fs::Permissions);
        impl Drop for RestorePermissions {
            fn drop(&mut self) {
                let _ = fs::set_permissions(&self.0, self.1.clone());
            }
        }
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("confirmed")).unwrap();
        let mut sidebar = Sidebar::default();
        sidebar.toggle(root.path().to_path_buf());
        wait_until(&mut sidebar, |state| {
            names(state, root.path()) == ["confirmed"]
        });
        let permissions = fs::metadata(root.path()).unwrap().permissions();
        let restore = RestorePermissions(root.path().to_path_buf(), permissions.clone());
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o0)).unwrap();
        // Restore permissions even if this assertion fails; privileged runners do
        // not enforce this provider failure and must cover it natively instead.
        let denied = fs::read_dir(root.path()).is_err();
        if !denied {
            fs::set_permissions(root.path(), permissions).unwrap();
            eprintln!("permission-loss test requires an unprivileged runner");
            return;
        }
        sidebar.refresh();
        wait_until(&mut sidebar, |state| {
            state.job.is_none() && state.warning().is_some()
        });
        fs::set_permissions(root.path(), permissions).unwrap();
        drop(restore);
        assert_eq!(names(&sidebar, root.path()), ["confirmed"]);
        assert!(sidebar.warning().unwrap().contains("stale"));
        // Exercise the automatic retry rather than a separate refresh authority.
        sidebar.folders.get_mut(root.path()).unwrap().retry_at = Some(Instant::now());
        wait_until(&mut sidebar, |state| state.warning().is_none());
    }
}
