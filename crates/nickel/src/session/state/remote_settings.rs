//! Bounded settings preparation on the remote bridge's blocking worker.
use super::{ShellBehaviorTransaction, ShellSettings, prepare_shell_behavior_update};
use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Instant,
};

const UNAVAILABLE: &str = "settings transaction failed; inspect current state before retrying";
const STALE: &str = "settings transaction rejected: stale state or invalid value";

pub(super) use super::remote_worker::WorkerStaging as SettingsStaging;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct FileRevision {
    device: u64,
    inode: u64,
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

pub(super) fn revision(path: &Path) -> io::Result<Option<FileRevision>> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "settings are not a regular file",
        ));
    }
    Ok(Some(FileRevision {
        device: metadata.dev(),
        inode: metadata.ino(),
        length: metadata.len(),
        modified: (metadata.mtime(), metadata.mtime_nsec()),
        changed: (metadata.ctime(), metadata.ctime_nsec()),
    }))
}

pub(super) struct PreparedShellBehavior {
    pub requested: ShellSettings,
    path: PathBuf,
    revision: Option<FileRevision>,
    staged: nickel_storage::StagedWrite,
    _lock: nickel_storage::TransactionLock,
}

impl PreparedShellBehavior {
    pub fn prepare(transaction: &ShellBehaviorTransaction) -> Result<Self, String> {
        let path = nickel_core::shell_settings::settings_path().map_err(|_| UNAVAILABLE)?;
        Self::prepare_at(path, transaction)
    }

    fn prepare_at(path: PathBuf, transaction: &ShellBehaviorTransaction) -> Result<Self, String> {
        let lock = nickel_storage::TransactionLock::try_acquire(&path).map_err(|_| UNAVAILABLE)?;
        let before = revision(&path).map_err(|_| UNAVAILABLE)?;
        let previous = ShellSettings::load_for_update(&path).map_err(|_| UNAVAILABLE)?;
        if revision(&path).map_err(|_| UNAVAILABLE)? != before {
            return Err(STALE.into());
        }
        // Only value/prior validation here. The owner checks the actual current
        // output topology when accepting the prepared transaction.
        let requested =
            prepare_shell_behavior_update(&previous, transaction.topology_generation, transaction)
                .map_err(|_| STALE)?;
        let staged = requested.stage(&path).map_err(|_| UNAVAILABLE)?;
        Ok(Self {
            requested,
            path,
            revision: before,
            staged,
            _lock: lock,
        })
    }

    pub fn commit(
        self,
        deadline: Instant,
        check_boundary: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        self.staged
            .commit(|| {
                // No content read/write on the owner: only identity validation and
                // atomic replacement. Metadata/rename still depend on the filesystem.
                if revision(&self.path)? != self.revision {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "settings revision changed",
                    ));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "settings commit expired",
                    ));
                }
                check_boundary().map_err(io::Error::other)?;
                Ok(())
            })
            .map_err(|error| {
                if error.kind() == io::ErrorKind::InvalidData {
                    STALE
                } else {
                    UNAVAILABLE
                }
                .to_owned()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_session_protocol::{ShellBehaviorSetting, ShellBehaviorValue};

    #[test]
    fn staging_state_remains_observable_while_busy_and_release_advances_generation() {
        let staging = SettingsStaging::default();
        assert!(!staging.snapshot().unwrap().busy);
        let admitted = staging.acquire().unwrap();
        let busy = staging.snapshot().unwrap();
        assert!(busy.busy);
        assert_eq!(busy.generation, 1);
        assert!(staging.acquire().is_err());
        assert_eq!(staging.snapshot().unwrap().generation, busy.generation);
        drop(admitted);
        let idle = staging.snapshot().unwrap();
        assert!(!idle.busy);
        assert_eq!(idle.generation, 2);
        assert!(idle.last_changed_uptime_us >= busy.last_changed_uptime_us);
        assert!(idle.last_changed_uptime_us <= idle.collector_uptime_us);
        assert!(staging.acquire().is_ok());
    }

    fn request() -> ShellBehaviorTransaction {
        ShellBehaviorTransaction {
            setting: ShellBehaviorSetting::DesktopCount,
            prior: ShellBehaviorValue::Count(4),
            requested: ShellBehaviorValue::Count(6),
            topology_generation: 1,
        }
    }

    #[test]
    fn prepared_settings_reject_changed_configuration_and_expiry_without_overwriting() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings");
        ShellSettings::default().save(&path).unwrap();
        let prepared = PreparedShellBehavior::prepare_at(path.clone(), &request()).unwrap();
        let changed = ShellSettings {
            idle_lock_seconds: Some(71),
            ..ShellSettings::default()
        };
        changed.stage(&path).unwrap().commit(|| Ok(())).unwrap();
        assert!(
            prepared
                .commit(
                    Instant::now() + std::time::Duration::from_secs(1),
                    || Ok(())
                )
                .is_err()
        );
        assert_eq!(ShellSettings::load(&path).unwrap(), changed);
        let expired = PreparedShellBehavior::prepare_at(path.clone(), &request()).unwrap();
        assert!(expired.commit(Instant::now(), || Ok(())).is_err());
        assert_eq!(ShellSettings::load(&path).unwrap(), changed);
        let cancelled = PreparedShellBehavior::prepare_at(path.clone(), &request()).unwrap();
        assert!(
            cancelled
                .commit(Instant::now() + std::time::Duration::from_secs(1), || Err(
                    "request cancelled during staging".into()
                ))
                .is_err()
        );
        assert_eq!(ShellSettings::load(&path).unwrap(), changed);
        let valid = PreparedShellBehavior::prepare_at(path.clone(), &request()).unwrap();
        valid
            .commit(
                Instant::now() + std::time::Duration::from_secs(1),
                || Ok(()),
            )
            .unwrap();
        let actual = ShellSettings::load(&path).unwrap();
        assert_eq!(actual.desktop_count, 6);
        assert_eq!(actual.idle_lock_seconds, Some(71));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }

    #[test]
    fn prepared_settings_exclude_cooperative_local_writers() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings");
        let previous = ShellSettings::default();
        previous.save(&path).unwrap();
        let prepared = PreparedShellBehavior::prepare_at(path.clone(), &request()).unwrap();
        let replacement = ShellSettings {
            idle_lock_seconds: Some(71),
            ..previous.clone()
        };

        assert_eq!(
            replacement.save(&path).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(ShellSettings::load(&path).unwrap(), previous);

        drop(prepared);
        replacement.save(&path).unwrap();
        assert_eq!(ShellSettings::load(&path).unwrap(), replacement);
    }
}
