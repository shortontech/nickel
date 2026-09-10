//! Platform storage mechanics shared by Nickel's domain crates.
//!
//! This crate owns operating-system configuration roots and durable replacement
//! of small files. Callers remain responsible for their schemas and policy.

use std::{
    fs, io,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_STAGING_ID: AtomicU64 = AtomicU64::new(0);

/// A prepared replacement with exclusive ownership of its temporary file.
/// Dropping it cancels the write. No destination change occurs until commit.
#[must_use]
pub struct StagedWrite {
    temporary: Option<PathBuf>,
    destination: PathBuf,
}

impl StagedWrite {
    /// Recheck authority at the commit boundary, after all content staging.
    pub fn commit(mut self, check_commit: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
        check_commit()?;
        replace_file(
            self.temporary.as_ref().expect("uncommitted staged write"),
            &self.destination,
        )?;
        self.temporary = None;
        Ok(())
    }
}

impl Drop for StagedWrite {
    fn drop(&mut self) {
        if let Some(temporary) = self.temporary.take() {
            let _ = fs::remove_file(temporary);
        }
    }
}

/// Prepare on a worker, then transfer the owned result to the commit authority.
/// Unique create-new peers prevent concurrent writers from sharing staged bytes.
pub fn stage_write(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<StagedWrite> {
    stage_write_inner(path, contents.as_ref(), false)
}

fn stage_write_inner(path: &Path, contents: &[u8], durable: bool) -> io::Result<StagedWrite> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "settings path has no parent")
    })?;
    fs::create_dir_all(parent)?;
    for _ in 0..16 {
        let id = NEXT_STAGING_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| io::Error::other("staging identity exhausted"))?;
        let temporary = path.with_extension(format!("tmp-{}-{id}", std::process::id()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&temporary) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let staged = StagedWrite {
            temporary: Some(temporary),
            destination: path.to_owned(),
        };
        let result = file
            .write_all(contents)
            .and_then(|()| if durable { file.sync_all() } else { Ok(()) });
        // Close before either replacement or cleanup, including on Windows.
        drop(file);
        result?;
        return Ok(staged);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "staging peer collision limit reached",
    ))
}

/// Read one small regular file without following its final symlink or waiting for
/// FIFO contents. `None` means the path was observed missing; inaccessible,
/// nonregular, dangling-symlink, changing, and oversized files return an error.
///
/// The byte count is bounded by `limit`; descriptor metadata is checked before and
/// after reading, and the current path is reopened to reject observed replacement.
/// These are revision checks, not an atomic snapshot against uncooperative writers.
/// Real regular-file and filesystem metadata I/O remain OS-bound: use a worker when
/// the caller must not wait on storage. There is no retry loop after a changed file.
pub fn read_regular_file(path: &Path, limit: usize) -> io::Result<Option<Vec<u8>>> {
    read_regular_file_inner(path, limit, |_| Ok(()))
}

fn read_regular_file_inner(
    path: &Path,
    limit: usize,
    before_read: impl FnOnce(&fs::File) -> io::Result<()>,
) -> io::Result<Option<Vec<u8>>> {
    use std::io::Read;
    let read_limit = u64::try_from(limit)
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid file byte limit"))?;
    let mut file = match open_regular_candidate(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // In particular, do not reinterpret a dangling symlink as absence on
            // platforms whose open returned NotFound for it.
            return match fs::symlink_metadata(path) {
                Err(missing) if missing.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
                Ok(_) => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "file changed or is a dangling link",
                )),
            };
        }
        Err(error) => return Err(error),
    };
    let before = revision_from_file(&file)?;
    if before.length > read_limit - 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds byte limit",
        ));
    }
    before_read(&file)?;
    let mut bytes = Vec::new();
    (&mut file).take(read_limit).read_to_end(&mut bytes)?;
    let after = revision_from_file(&file)?;
    if bytes.len() as u64 > read_limit - 1 || bytes.len() as u64 != before.length || after != before
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file changed while reading",
        ));
    }
    let current = open_regular_candidate(path)?;
    if revision_from_file(&current)? != before {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file was replaced while reading",
        ));
    }
    Ok(Some(bytes))
}

fn open_regular_candidate(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        };
        // Hold the file without write/delete sharing so Windows cannot replace
        // its path or accept a concurrent writer while this read is in progress.
        options
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
            .share_mode(FILE_SHARE_READ.0);
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    return Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "safe regular-file reads are unavailable on this platform",
    ));
    #[cfg(any(unix, target_os = "windows"))]
    options.open(path)
}

/// Opaque descriptor-backed revision for a bounded file transaction. Reading a
/// revision performs metadata I/O only and never consumes file contents.
pub fn regular_file_revision(path: &Path) -> io::Result<Option<RegularFileRevision>> {
    match open_regular_candidate(path) {
        Ok(file) => revision_from_file(&file).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::symlink_metadata(path) {
            Err(missing) if missing.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file changed or is a dangling link",
            )),
        },
        Err(error) => Err(error),
    }
}

/// Equality requires the same native file identity and observed metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegularFileRevision {
    length: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64, u32),
    #[cfg(target_os = "windows")]
    created: std::time::SystemTime,
    #[cfg(target_os = "windows")]
    identity: (u64, [u8; 16]),
}

fn revision_from_file(file: &fs::File) -> io::Result<RegularFileRevision> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "path is not a regular file",
        ));
    }
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    Ok(RegularFileRevision {
        length: metadata.len(),
        modified: metadata.modified()?,
        #[cfg(unix)]
        identity: (
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec(),
            metadata.mode(),
        ),
        #[cfg(target_os = "windows")]
        created: metadata.created()?,
        #[cfg(target_os = "windows")]
        identity: windows_file_identity(file)?,
    })
}

#[cfg(target_os = "windows")]
fn windows_file_identity(file: &fs::File) -> io::Result<(u64, [u8; 16])> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx},
    };
    let mut identity = FILE_ID_INFO::default();
    // SAFETY: the borrowed file keeps this handle live throughout the synchronous
    // query, and the writable buffer has the exact size/layout of FILE_ID_INFO.
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(file.as_raw_handle()),
            FileIdInfo,
            (&mut identity as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    }
    .map_err(io::Error::other)?;
    // Unsupported filesystems fail the query rather than falling back to times
    // and lengths, which cannot distinguish different file objects.
    Ok((identity.VolumeSerialNumber, identity.FileId.Identifier))
}

/// Maximum content accepted by the durable small-file staging API.
pub const MAX_DURABLE_WRITE_BYTES: usize = 1024 * 1024;
#[cfg(unix)]
const MAX_SYNC_DIRECTORIES: usize = 64;

/// Cooperative cross-process exclusion for one destination, including its journal.
///
/// Every writer must use the same destination path and retain this guard throughout
/// preparation, external effects, and durable completion. The stable `.lock` sibling
/// is deliberately never removed: locking the replaceable destination inode or
/// unlinking the sibling would allow different writers to lock different files.
/// This is advisory exclusion, not protection against uncooperative external edits.
#[derive(Debug)]
#[must_use]
pub struct TransactionLock {
    _file: fs::File,
}

impl TransactionLock {
    /// Try once, without waiting for another writer. Call on a worker because
    /// opening the containing directory/file can still wait on filesystem I/O.
    pub fn try_acquire(destination: &Path) -> io::Result<Self> {
        let name = destination.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "transaction path has no filename",
            )
        })?;
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let mut lock_name = name.to_os_string();
        lock_name.push(".lock");
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows::Win32::Storage::FileSystem::{
                FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
            };
            options
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
                .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0);
        }
        let file = options.open(parent.join(lock_name))?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "transaction lock is not a regular file",
            ));
        }
        file.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::WouldBlock,
                "another writer owns the transaction",
            ),
            fs::TryLockError::Error(error) => error,
        })?;
        Ok(Self { _file: file })
    }
}

/// Synced content that has not yet replaced the destination.
///
/// Preparation syncs content on a worker; `commit` only performs the checked
/// replacement. A successful commit is acceptance, not yet durable completion.
#[must_use]
pub struct StagedDurableWrite {
    staged: StagedWrite,
    directories: Vec<fs::File>,
}

impl StagedDurableWrite {
    /// Check authority immediately before rename. Transfer the returned receipt
    /// back to a worker and finish `sync` before starting any dependent mutation.
    pub fn commit(
        self,
        check_commit: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<DirectorySyncReceipt> {
        self.staged.commit(check_commit)?;
        Ok(DirectorySyncReceipt {
            directories: self.directories,
        })
    }
}

/// Evidence that rename was accepted, with directory persistence still pending.
///
/// Dropping this receipt does not roll back the accepted replacement. Complete it
/// on a worker even if authority was lost after rename: syncing that accepted write
/// grants no authority for another mutation. An error means durability is uncertain;
/// callers must preserve their pending journal and must not proceed with effects.
#[must_use]
pub struct DirectorySyncReceipt {
    directories: Vec<fs::File>,
}

impl DirectorySyncReceipt {
    /// Sync the destination directory and every ancestor from leaf to root. All
    /// handles were opened during preparation; this performs no new path lookup,
    /// replacement, or content write. Filesystem/device guarantees still apply.
    pub fn sync(self) -> io::Result<()> {
        for directory in self.directories {
            directory.sync_all()?;
        }
        Ok(())
    }
}

/// Prepare bounded durable content on a worker, including directory handles needed
/// to persist newly created parent directories after replacement. Unix directory
/// fsync is required; platforms without an implemented persistence boundary reject
/// preparation before creating anything. Existing `stage_write` semantics are unchanged.
///
/// Cooperative writers must hold `TransactionLock` throughout the transaction.
/// Parent directory replacement by an uncooperative actor is outside this contract.
pub fn stage_durable_write(
    path: &Path,
    contents: impl AsRef<[u8]>,
) -> io::Result<StagedDurableWrite> {
    let contents = contents.as_ref();
    if contents.len() > MAX_DURABLE_WRITE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "durable write exceeds content limit",
        ));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "durable directory replacement is not implemented on this platform",
        ))
    }
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        // Bound depth before creating directories, including relative-path ancestry.
        let absolute = if parent.is_absolute() {
            parent.to_owned()
        } else {
            std::env::current_dir()?.join(parent)
        };
        if absolute.ancestors().count() > MAX_SYNC_DIRECTORIES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "durable write exceeds directory depth limit",
            ));
        }
        fs::create_dir_all(parent)?;
        let canonical_parent = fs::canonicalize(parent)?;
        if canonical_parent.ancestors().count() > MAX_SYNC_DIRECTORIES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "durable write exceeds directory depth limit",
            ));
        }
        let mut directories = Vec::new();
        for ancestor in canonical_parent.ancestors() {
            use std::os::unix::fs::OpenOptionsExt;
            let directory = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
                .open(ancestor)?;
            directories.push(directory);
        }
        let staged = stage_write_inner(path, contents, true)?;
        Ok(StagedDurableWrite {
            staged,
            directories,
        })
    }
}

/// Resolves a file below Nickel's per-user configuration directory.
pub fn config_path(file_name: &str) -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is not set"))?;
        Ok(PathBuf::from(root).join("Nickel").join(file_name))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let root = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "XDG_CONFIG_HOME and HOME are not set",
                )
            })?;
        Ok(root.join("nickel").join(file_name))
    }
}

/// Replaces a complete small file without exposing readers to a partial write.
pub fn atomic_write(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    atomic_write_checked(path, contents, || Ok(()))
}

/// Stage a complete file, then check the caller's commit condition immediately
/// before replacement. A rejected commit leaves the destination untouched.
pub fn atomic_write_checked(
    path: &Path,
    contents: impl AsRef<[u8]>,
    check_commit: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    stage_write(path, contents)?.commit(check_commit)
}

#[cfg(not(target_os = "windows"))]
fn replace_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(temporary, destination)
}

#[cfg(target_os = "windows")]
fn replace_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{
            MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        },
        core::PCWSTR,
    };
    let temporary = temporary
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: both paths are terminated and remain live for the synchronous call.
    unsafe {
        MoveFileExW(
            PCWSTR(temporary.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVE_FILE_FLAGS(MOVEFILE_REPLACE_EXISTING.0 | MOVEFILE_WRITE_THROUGH.0),
        )
    }
    .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::atomic_write;

    #[test]
    fn regular_reads_distinguish_absence_empty_and_exact_byte_limit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings");
        assert_eq!(super::read_regular_file(&path, 4).unwrap(), None);
        std::fs::write(&path, []).unwrap();
        assert_eq!(
            super::read_regular_file(&path, 0).unwrap(),
            Some(Vec::new())
        );
        std::fs::write(&path, "four").unwrap();
        assert_eq!(
            super::read_regular_file(&path, 4).unwrap(),
            Some(b"four".to_vec())
        );
        assert!(super::read_regular_file(&path, 3).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn regular_reads_reject_observed_in_place_changes_and_inode_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings");
        std::fs::write(&path, "first").unwrap();
        let result = super::read_regular_file_inner(&path, 16, |_| {
            std::fs::write(&path, "other")?;
            std::fs::OpenOptions::new()
                .write(true)
                .open(&path)?
                .set_times(std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
        });
        assert!(result.is_err());
        let result = super::read_regular_file_inner(&path, 16, |_| {
            let replacement = directory.path().join("replacement");
            std::fs::write(&replacement, "other")?;
            std::fs::rename(replacement, &path)
        });
        assert!(result.is_err());
        let result = super::read_regular_file_inner(&path, 5, |_| {
            std::fs::write(&path, "now exceeds bound")
        });
        assert!(result.is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_file_revision_distinguishes_identical_bytes_and_timestamps() {
        use std::os::windows::fs::FileTimesExt;
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        for path in [&first, &second] {
            let mut file = std::fs::File::create(path).unwrap();
            use std::io::Write;
            file.write_all(b"identical bytes").unwrap();
            file.set_times(
                std::fs::FileTimes::new()
                    .set_created(std::time::UNIX_EPOCH)
                    .set_modified(std::time::UNIX_EPOCH),
            )
            .unwrap();
        }
        let first_revision =
            super::revision_from_file(&super::open_regular_candidate(&first).unwrap()).unwrap();
        let same_revision =
            super::revision_from_file(&super::open_regular_candidate(&first).unwrap()).unwrap();
        let second_revision =
            super::revision_from_file(&super::open_regular_candidate(&second).unwrap()).unwrap();
        assert!(first_revision == same_revision);
        assert_eq!(first_revision.length, second_revision.length);
        assert_eq!(first_revision.created, second_revision.created);
        assert_eq!(first_revision.modified, second_revision.modified);
        assert_ne!(first_revision.identity, second_revision.identity);
        assert!(first_revision != second_revision);
    }

    #[test]
    fn regular_file_child_probe() {
        let Some(path) = std::env::var_os("NICKEL_STORAGE_TEST_READ_PATH") else {
            return;
        };
        assert!(super::read_regular_file(std::path::Path::new(&path), 64).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn nonregular_reads_reject_fifo_device_directory_and_links_without_waiting() {
        let directory = tempfile::tempdir().unwrap();
        let fifo = directory.path().join("fifo");
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: the owned terminated path remains valid for mkfifo, and this
        // creates only a FIFO inside the fixture's private temporary directory.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let ordinary = directory.path().join("ordinary");
        std::fs::write(&ordinary, "contents").unwrap();
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&ordinary, &link).unwrap();
        let dangling = directory.path().join("dangling");
        std::os::unix::fs::symlink(directory.path().join("missing"), &dangling).unwrap();
        for path in [
            &fifo,
            std::path::Path::new("/dev/zero"),
            directory.path(),
            &link,
            &dangling,
        ] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "tests::regular_file_child_probe"])
                .env("NICKEL_STORAGE_TEST_READ_PATH", path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(
                        status.success(),
                        "nonregular read was not rejected: {path:?}"
                    );
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("nonregular read blocked: {path:?}");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn durable_replacement_separates_acceptance_from_worker_sync() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("new/parents/settings");
        let staged = std::thread::scope(|scope| {
            scope
                .spawn(|| super::stage_durable_write(&path, "intent").unwrap())
                .join()
                .unwrap()
        });
        assert!(!path.exists());
        let receipt = staged
            .commit(|| {
                assert!(!path.exists());
                Ok(())
            })
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "intent");
        // Accepted intent stays in place even if the requesting authority is now
        // gone. Completing persistence has no callback that starts another effect.
        std::thread::spawn(move || receipt.sync())
            .join()
            .unwrap()
            .unwrap();
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn durable_rejected_commit_preserves_original_and_cleans_stage() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings");
        atomic_write(&path, "original").unwrap();
        let staged = super::stage_durable_write(&path, "intent").unwrap();
        assert!(
            staged
                .commit(|| Err(std::io::Error::other("revoked")))
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn failed_directory_sync_does_not_claim_or_undo_durable_completion() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings");
        let mut receipt = super::stage_durable_write(&path, "pending intent")
            .unwrap()
            .commit(|| Ok(()))
            .unwrap();
        // A real descriptor whose filesystem refuses fsync models a directory
        // persistence failure after rename; the accepted journal must survive it.
        receipt
            .directories
            .insert(0, std::fs::File::open("/dev/null").unwrap());
        assert!(receipt.sync().is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "pending intent");
    }

    #[test]
    fn oversized_durable_content_is_rejected_before_creating_any_paths() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("not-created/settings");
        let bytes = vec![0; super::MAX_DURABLE_WRITE_BYTES + 1];
        let error = super::stage_durable_write(&path, bytes).err().unwrap();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!path.parent().unwrap().exists());
    }

    #[cfg(not(unix))]
    #[test]
    fn unsupported_durability_does_not_mutate_the_filesystem() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("not-created/settings");
        let error = super::stage_durable_write(&path, "intent").err().unwrap();
        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn transaction_lock_child_probe() {
        let Some(path) = std::env::var_os("NICKEL_STORAGE_TEST_LOCK_PATH") else {
            return;
        };
        let result = super::TransactionLock::try_acquire(std::path::Path::new(&path));
        if std::env::var_os("NICKEL_STORAGE_TEST_LOCK_BUSY").is_some() {
            assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        } else {
            assert!(result.is_ok());
        }
    }

    #[test]
    fn stable_sibling_lock_excludes_processes_across_destination_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings");
        let probe = |busy: bool| {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "tests::transaction_lock_child_probe"])
                .env("NICKEL_STORAGE_TEST_LOCK_PATH", &path)
                .env_remove("NICKEL_STORAGE_TEST_LOCK_BUSY");
            if busy {
                command.env("NICKEL_STORAGE_TEST_LOCK_BUSY", "1");
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        };
        let lock = super::TransactionLock::try_acquire(&path).unwrap();
        probe(true);
        atomic_write(&path, "first").unwrap();
        atomic_write(&path, "second").unwrap();
        assert_eq!(
            super::TransactionLock::try_acquire(&path)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        probe(true);
        drop(lock);
        assert!(directory.path().join("settings.lock").is_file());
        probe(false);
        let _next = super::TransactionLock::try_acquire(&path).unwrap();
        probe(true);
    }

    #[cfg(unix)]
    #[test]
    fn transaction_lock_rejects_symlink_and_nonregular_siblings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings");
        let sibling = directory.path().join("settings.lock");
        let victim = directory.path().join("victim");
        std::fs::write(&victim, "untouched").unwrap();
        std::os::unix::fs::symlink(&victim, &sibling).unwrap();
        assert!(super::TransactionLock::try_acquire(&path).is_err());
        std::fs::remove_file(&sibling).unwrap();
        std::fs::create_dir(&sibling).unwrap();
        assert!(super::TransactionLock::try_acquire(&path).is_err());
        assert_eq!(std::fs::read_to_string(victim).unwrap(), "untouched");
    }

    #[test]
    fn worker_staging_is_independent_until_explicit_commit_and_drop_cancels() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings");
        atomic_write(&path, "original").unwrap();
        let staged = std::thread::scope(|scope| {
            let first = scope.spawn(|| super::stage_write(&path, "first").unwrap());
            let second = scope.spawn(|| super::stage_write(&path, "second").unwrap());
            (first.join().unwrap(), second.join().unwrap())
        });
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        assert_ne!(staged.0.temporary, staged.1.temporary);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 3);
        staged.0.commit(|| Ok(())).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        drop(staged.1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_replacement_cleans_its_owned_peer() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("destination-directory");
        std::fs::create_dir(&destination).unwrap();
        let staged = super::stage_write(&destination, "contents").unwrap();
        assert!(staged.commit(|| Ok(())).is_err());
        assert!(destination.is_dir());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn staged_settings_are_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let staged = super::stage_write(&directory.path().join("settings"), "contents").unwrap();
        let mode = std::fs::metadata(staged.temporary.as_ref().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
    }

    #[test]
    fn rejected_commit_preserves_destination_and_removes_staged_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings");
        atomic_write(&path, "original").unwrap();
        let result = super::atomic_write_checked(&path, "replacement", || {
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "commit expired",
            ))
        });
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn atomic_replacement_never_leaves_the_temporary_peer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings");
        atomic_write(&path, "first").unwrap();
        atomic_write(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
