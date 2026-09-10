//! Local preference writes use the same checked writer as MCP, on one bounded worker.
use nickel_core::launcher_preferences::{LauncherPreferences, PreparedLauncherPreferences};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
};

struct Job {
    epoch: u64,
    path: PathBuf,
    prior: LauncherPreferences,
    requested: LauncherPreferences,
    recent: Option<String>,
}
struct Completion {
    epoch: u64,
    favorite: bool,
    result: Result<LauncherPreferences, Option<LauncherPreferences>>,
}
struct Worker {
    sender: mpsc::SyncSender<Job>,
    receiver: mpsc::Receiver<Completion>,
    alive: Arc<AtomicBool>,
    epoch: Arc<AtomicU64>,
}
impl Worker {
    fn new() -> std::io::Result<Self> {
        let (sender, requests) = mpsc::sync_channel::<Job>(8);
        let (responses, receiver) = mpsc::sync_channel(8);
        let alive = Arc::new(AtomicBool::new(true));
        let epoch = Arc::new(AtomicU64::new(1));
        let worker_alive = alive.clone();
        let worker_epoch = epoch.clone();
        std::thread::Builder::new()
            .name("launcher-preferences".into())
            .spawn(move || {
                while let Ok(job) = requests.recv() {
                    if !worker_alive.load(Ordering::Acquire) {
                        break;
                    }
                    let current = || {
                        worker_alive.load(Ordering::Acquire)
                            && worker_epoch.load(Ordering::Acquire) == job.epoch
                    };
                    let result = (|| {
                        if !current() {
                            return Err(std::io::Error::other("preference action superseded"));
                        }
                        let prepared = if let Some(recent) = &job.recent {
                            PreparedLauncherPreferences::prepare_recent(job.path.clone(), recent)
                        } else {
                            PreparedLauncherPreferences::prepare_favorites(
                                job.path.clone(),
                                job.prior.favorites(),
                                job.requested.favorites().to_vec(),
                            )
                        }?;
                        prepared.commit(|| {
                            if current() {
                                Ok(())
                            } else {
                                Err(std::io::Error::other("preference action superseded"))
                            }
                        })
                    })();
                    let result = match result {
                        Ok(preferences) => Ok(preferences),
                        Err(_) => {
                            if let Some(next) = job.epoch.checked_add(1) {
                                let _ = worker_epoch.compare_exchange(
                                    job.epoch,
                                    next,
                                    Ordering::AcqRel,
                                    Ordering::Acquire,
                                );
                            } else {
                                worker_alive.store(false, Ordering::Release);
                            }
                            let observed = match LauncherPreferences::load(&job.path) {
                                Ok(value) => Some(value),
                                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                    Some(LauncherPreferences::default())
                                }
                                Err(_) => None,
                            };
                            Err(observed)
                        }
                    };
                    if responses
                        .send(Completion {
                            epoch: job.epoch,
                            favorite: job.recent.is_none(),
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self {
            sender,
            receiver,
            alive,
            epoch,
        })
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

pub(super) struct PreferencePersistence {
    worker: Option<Worker>,
    committed: LauncherPreferences,
    submitted: LauncherPreferences,
    pending: usize,
    favorite_failure: bool,
    epoch: u64,
}
impl PreferencePersistence {
    pub fn new(preferences: LauncherPreferences) -> Self {
        Self {
            worker: None,
            committed: preferences.clone(),
            submitted: preferences,
            pending: 0,
            favorite_failure: false,
            epoch: 1,
        }
    }
    pub fn busy(&self) -> bool {
        self.pending != 0
    }
    pub fn replace_committed(
        &mut self,
        preferences: LauncherPreferences,
    ) -> Result<(), &'static str> {
        if self.busy() {
            return Err("local preference action is pending");
        }
        self.committed = preferences.clone();
        self.submitted = preferences;
        self.favorite_failure = false;
        Ok(())
    }
    pub fn enqueue(
        &mut self,
        path: PathBuf,
        requested: LauncherPreferences,
        recent: Option<String>,
    ) -> Result<(), &'static str> {
        if self.pending == 8 {
            return Err("preference queue is full");
        }
        if self.worker.is_none() {
            self.worker = Some(Worker::new().map_err(|_| "preference worker unavailable")?);
        }
        let worker = self.worker.as_ref().unwrap();
        if worker.epoch.load(Ordering::Acquire) != self.epoch
            || !worker.alive.load(Ordering::Acquire)
        {
            return Err("previous preference action failed; retry after refresh");
        }
        worker
            .sender
            .try_send(Job {
                epoch: self.epoch,
                path,
                prior: self.submitted.clone(),
                requested: requested.clone(),
                recent,
            })
            .map_err(|_| "preference worker is busy or stopped")?;
        self.submitted = requested;
        self.pending += 1;
        Ok(())
    }
    /// Returns actual accepted/observed preferences, and whether a write failed.
    pub fn poll(&mut self) -> Option<(LauncherPreferences, bool)> {
        let worker = self.worker.as_ref()?;
        let mut changed = false;
        let mut failed = false;
        loop {
            let completion = match worker.receiver.try_recv() {
                Ok(completion) => completion,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = 0;
                    self.submitted = self.committed.clone();
                    self.worker = None;
                    self.epoch = 1;
                    return Some((self.committed.clone(), true));
                }
            };
            self.pending = self.pending.saturating_sub(1);
            if completion.epoch != self.epoch {
                continue;
            }
            match completion.result {
                Ok(preferences) => {
                    self.committed = preferences;
                    if completion.favorite {
                        self.favorite_failure = false;
                    }
                    changed = true;
                }
                Err(observed) => {
                    if let Some(preferences) = observed {
                        self.committed = preferences;
                    }
                    self.submitted = self.committed.clone();
                    self.epoch = worker.epoch.load(Ordering::Acquire);
                    failed = true;
                    self.favorite_failure |= completion.favorite;
                    changed = true;
                }
            }
        }
        if changed && (failed || self.pending == 0) {
            Some((self.committed.clone(), failed || self.favorite_failure))
        } else {
            None
        }
    }
}
