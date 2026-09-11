//! Windows owner plumbing. Native launch attribution still requires Windows
//! validation; registry policy tests do not establish ShellExecuteEx behavior.
use super::*;
use crate::model::Application;
use nickel_platform::process_identity::{ExecutableEvidence, WindowsProcessIdentity};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, OwnedHandle},
    },
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

impl ProcessEvidence for WindowsProcessIdentity {
    type Image = ExecutableEvidence;
    fn incarnation(&self) -> (u32, u64) {
        (self.process_id(), self.created_at())
    }
    fn is_live(&self) -> bool {
        WindowsProcessIdentity::is_live(self)
    }
    fn image(&self) -> Option<Self::Image> {
        self.executable_evidence()
    }
    fn same_image(left: &Self::Image, right: &Self::Image) -> bool {
        left.same_file(right)
    }
}

struct LaunchReceipt {
    descriptor: Descriptor,
    process: WindowsProcessIdentity,
    invocation: LaunchInterval,
}
static LAUNCH_HANDLES: OnceLock<SyncSender<(Descriptor, OwnedHandle, Instant, LaunchInterval)>> =
    OnceLock::new();

/// Pin the actual discovered shortcut throughout its launch. No constructor
/// accepts a catalog identity string or process ID from an external endpoint.
pub(crate) struct LaunchCapture {
    descriptor: Descriptor,
    target: String,
    _shortcut: File,
    _ancestors: Vec<File>,
    invoked_at: Option<u64>,
}

/// Immutable owner-attested catalog data transferred to a preparation worker.
/// Constructing this value performs no filesystem or native process operation.
pub(crate) struct LaunchPlan {
    catalog_generation: u64,
    application_id: String,
    identity: String,
    digest: [u8; 32],
    application: Application,
}

impl LaunchPlan {
    /// Pin and hash the exact shortcut off the presentation owner.
    pub(crate) fn prepare(self) -> Result<(Self, LaunchCapture), String> {
        let capture = LaunchCapture::prepare(&self.application)
            .ok_or("installed application launch target could not be pinned")?;
        if capture.application_id() != self.application_id
            || capture.application_identity() != self.identity
            || capture.descriptor.digest != self.digest
        {
            return Err("installed application changed; enumerate it again".into());
        }
        Ok((self, capture))
    }

    pub(crate) fn catalog_generation(&self) -> u64 {
        self.catalog_generation
    }

    pub(crate) fn application_id(&self) -> &str {
        &self.application_id
    }

    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }
}
impl LaunchCapture {
    pub(crate) fn prepare(application: &Application) -> Option<Self> {
        let (descriptor, shortcut) = read_descriptor(
            application,
            windows::Win32::Storage::FileSystem::FILE_SHARE_READ.0,
        )?;
        let (target, ancestors) = pin_launch_path(&shortcut)?;
        Some(Self {
            descriptor,
            target,
            _shortcut: shortcut,
            _ancestors: ancestors,
            invoked_at: None,
        })
    }

    /// Capture native creation-time cutoff immediately before ShellExecuteEx.
    pub(crate) fn begin_invocation(&mut self) {
        self.invoked_at = Some(native_filetime());
    }

    pub(crate) fn target(&self) -> &str {
        &self.target
    }

    pub(crate) fn application_id(&self) -> &str {
        &self.descriptor.id
    }

    pub(crate) fn application_identity(&self) -> &str {
        &self.descriptor.identity
    }

    /// Called only with the owned process handle returned by this exact
    /// ShellExecuteEx invocation. Missing/DDE/reused processes receive no receipt.
    pub(crate) fn complete(self, process: OwnedHandle) {
        if let Some(sender) = LAUNCH_HANDLES.get() {
            let Some(started) = self.invoked_at else {
                return;
            };
            let invocation = LaunchInterval {
                started,
                completed: native_filetime(),
            };
            let _ = sender.try_send((self.descriptor, process, Instant::now(), invocation));
        }
    }
}

fn read_descriptor(application: &Application, sharing: u32) -> Option<(Descriptor, File)> {
    let command = application.launch_command()?;
    if command.len() != 1 {
        return None;
    }
    let path = std::path::Path::new(&command[0]);
    if !path.is_absolute() || !path.extension()?.eq_ignore_ascii_case("lnk") {
        return None;
    }
    if application.id() != format!("windows-shortcut:{}", command[0].to_ascii_lowercase()) {
        return None;
    }
    if application.id().len() > 4096 {
        return None;
    }
    let mut shortcut = OpenOptions::new()
        .read(true)
        .share_mode(sharing)
        .open(path)
        .ok()?;
    if !shortcut.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    (&mut shortcut)
        .take(1_048_577)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 1_048_576 {
        return None;
    }
    let digest = Sha256::digest(&bytes).into();
    let identity = format!(
        "windows:catalog-launch:{:x}",
        Sha256::digest(application.id().as_bytes())
    );
    Some((
        Descriptor {
            id: application.id().into(),
            identity,
            digest,
            policy: RuntimePolicy::LaunchBound,
        },
        shortcut,
    ))
}

fn native_filetime() -> u64 {
    // SAFETY: The OS returns an initialized FILETIME value; no pointers escape.
    let time =
        unsafe { windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime() };
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

/// Use the opened file's normalized local volume-GUID path and pin every parent
/// against deletion. WRITE sharing allows unrelated file creation. Pinning only
/// the final file leaves ancestor rename or
/// junction replacement races when ShellExecuteEx resolves the path again.
fn pin_launch_path(file: &File) -> Option<(String, Vec<File>)> {
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_ID_INFO, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdInfo,
            GetFileInformationByHandleEx, GetFinalPathNameByHandleW, VOLUME_NAME_GUID,
        },
    };
    let mut path = vec![0u16; 32768];
    // SAFETY: Retained file and writable bounded UTF-16 buffer.
    let length = unsafe {
        GetFinalPathNameByHandleW(HANDLE(file.as_raw_handle()), &mut path, VOLUME_NAME_GUID)
    } as usize;
    if length == 0 || length >= path.len() {
        return None;
    }
    let path = String::from_utf16(&path[..length]).ok()?;
    if !path.starts_with(r"\\?\Volume{")
        || path.as_bytes().get(48) != Some(&b'\\')
        || path.contains('\0')
    {
        return None;
    }
    let mut ancestors = Vec::new();
    for (index, character) in path.char_indices() {
        if index < 48 || character != '\\' {
            continue;
        }
        if ancestors.len() >= 256 {
            return None;
        }
        let directory = OpenOptions::new()
            .read(true)
            .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(&path[..=index])
            .ok()?;
        let metadata = directory.metadata().ok()?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return None;
        }
        ancestors.push(directory);
    }
    if ancestors.is_empty() {
        return None;
    }
    // Recheck after every child is pinned: intermediate reparse changes during
    // acquisition must not survive. The nondeletable child chain keeps each
    // ancestor nonempty, and Windows rejects setting a reparse point on a
    // nonempty directory. WRITE sharing can therefore remain enabled.
    // https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/fsctl-set-reparse-point
    for directory in &ancestors {
        if directory.metadata().ok()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return None;
        }
    }
    // After ancestors are pinned, reopening must identify the original file.
    // This rejects a swap that preceded acquisition of the parent handles.
    let reopened = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .open(&path)
        .ok()?;
    let query = |file: &File| -> Option<FILE_ID_INFO> {
        let mut info = FILE_ID_INFO::default();
        // SAFETY: Exact FILE_ID_INFO buffer and retained read-only file handle.
        unsafe {
            GetFileInformationByHandleEx(
                HANDLE(file.as_raw_handle()),
                FileIdInfo,
                std::ptr::from_mut(&mut info).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        }
        .ok()?;
        if info.VolumeSerialNumber == 0 || info.FileId.Identifier == [0; 16] {
            return None;
        }
        Some(info)
    };
    let original = query(file)?;
    let current = query(&reopened)?;
    if original.VolumeSerialNumber != current.VolumeSerialNumber
        || original.FileId.Identifier != current.FileId.Identifier
    {
        return None;
    }
    Some((path, ancestors))
}

struct CatalogData {
    descriptors: Vec<Descriptor>,
    applications: Vec<nickel_remote_control::diagnostics::InstalledApplication>,
    launch_applications: Vec<Application>,
    truncated: bool,
}

fn bounded_utf8(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn catalog() -> CatalogData {
    // Reuse the shell's installed application discovery, never a second scan or
    // list of executable names. Each unpackaged entry is conservatively treated
    // as a potential shared runtime and requires exact owner launch receipts.
    let discovery = crate::platform::application_discovery();
    if discovery.applications().len() > MAX_ENTRIES {
        return CatalogData {
            descriptors: Vec::new(),
            applications: Vec::new(),
            launch_applications: Vec::new(),
            truncated: true,
        };
    }
    let mut descriptors = Vec::with_capacity(discovery.applications().len());
    let mut applications = Vec::with_capacity(
        discovery
            .applications()
            .len()
            .min(nickel_remote_control::diagnostics::MAX_INSTALLED_APPLICATIONS),
    );
    let mut launch_applications = Vec::with_capacity(discovery.applications().len());
    let mut truncated = false;
    for application in discovery.applications() {
        let descriptor = read_descriptor(
            application,
            (windows::Win32::Storage::FileSystem::FILE_SHARE_READ
                | windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE
                | windows::Win32::Storage::FileSystem::FILE_SHARE_DELETE)
                .0,
        )
        .map(|(descriptor, _)| descriptor)
        .unwrap_or_else(|| Descriptor {
            id: application.id().into(),
            identity: format!(
                "windows:catalog-launch:{:x}",
                Sha256::digest(application.id().as_bytes())
            ),
            digest: [0; 32],
            policy: RuntimePolicy::Unavailable,
        });
        if application.id().len() > 512 {
            truncated = true;
        } else if applications.len()
            < nickel_remote_control::diagnostics::MAX_INSTALLED_APPLICATIONS
        {
            applications.push(nickel_remote_control::diagnostics::InstalledApplication {
                id: descriptor.id.clone(),
                name: bounded_utf8(application.name(), 512),
                verified_application: (descriptor.policy == RuntimePolicy::LaunchBound)
                    .then(|| descriptor.identity.clone()),
            });
            if descriptor.policy == RuntimePolicy::LaunchBound {
                launch_applications.push(application.clone());
            }
        } else {
            truncated = true;
        }
        descriptors.push(descriptor);
    }
    CatalogData {
        descriptors,
        truncated,
        applications,
        launch_applications,
    }
}

struct CatalogSnapshot {
    started: Instant,
    completed: Instant,
    catalog: CatalogData,
}

fn same_catalog(
    left: &[nickel_remote_control::diagnostics::InstalledApplication],
    right: &[nickel_remote_control::diagnostics::InstalledApplication],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.id == right.id
                && left.name == right.name
                && left.verified_application == right.verified_application
        })
}

pub(crate) struct OwnerRegistry {
    registry: Registry<WindowsProcessIdentity>,
    receipts: Receiver<LaunchReceipt>,
    snapshots: Receiver<CatalogSnapshot>,
    refresh: SyncSender<()>,
    stop: Arc<AtomicBool>,
    next_refresh: Instant,
    observed: Option<Instant>,
    catalog_generation: u64,
    applications: Vec<nickel_remote_control::diagnostics::InstalledApplication>,
    catalog_descriptors: Vec<Descriptor>,
    launch_applications: Vec<Application>,
    catalog_truncated: bool,
}
impl Default for OwnerRegistry {
    fn default() -> Self {
        let (receipt_sender, receipts) = mpsc::sync_channel(64);
        let (handle_sender, handle_requests) =
            mpsc::sync_channel::<(Descriptor, OwnedHandle, Instant, LaunchInterval)>(16);
        // A second owner receives no launch receipts. The shell's existing
        // singleton emergency ownership also rejects duplicate production owners.
        let _ = LAUNCH_HANDLES.set(handle_sender);
        let (refresh, requests) = mpsc::sync_channel(1);
        let (sender, snapshots) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let launch_stop = stop.clone();
        std::thread::spawn(move || {
            while !launch_stop.load(Ordering::Acquire) {
                match handle_requests.recv_timeout(Duration::from_secs(1)) {
                    Ok((descriptor, handle, queued_at, invocation)) => {
                        if queued_at.elapsed() >= Duration::from_secs(5) {
                            continue;
                        }
                        // Native process/token/file probes run on this bounded
                        // worker, never while the desktop owner holds authority.
                        if let Ok(process) = WindowsProcessIdentity::from_retained_process(&handle)
                            && queued_at.elapsed() < Duration::from_secs(5)
                        {
                            let _ = receipt_sender.try_send(LaunchReceipt {
                                descriptor,
                                process,
                                invocation,
                            });
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        });
        let worker_stop = stop.clone();
        std::thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match requests.recv_timeout(Duration::from_secs(1)) {
                    Ok(()) => {
                        let started = Instant::now();
                        let catalog = catalog();
                        let completed = Instant::now();
                        let _ = sender.try_send(CatalogSnapshot {
                            started,
                            completed,
                            catalog,
                        });
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        });
        Self {
            registry: Registry::default(),
            receipts,
            snapshots,
            refresh,
            stop,
            next_refresh: Instant::now(),
            observed: None,
            catalog_generation: 0,
            applications: Vec::new(),
            catalog_descriptors: Vec::new(),
            launch_applications: Vec::new(),
            catalog_truncated: false,
        }
    }
}

impl OwnerRegistry {
    /// Snapshot an exact generation-bearing entry without filesystem access.
    pub(crate) fn plan_launch(
        &self,
        catalog_generation: u64,
        application_id: &str,
    ) -> Result<LaunchPlan, String> {
        if catalog_generation == 0 || catalog_generation != self.catalog_generation {
            return Err("application catalog changed; enumerate it again".into());
        }
        let listed = self
            .applications
            .iter()
            .find(|application| application.id == application_id)
            .ok_or("installed application is unavailable")?;
        let expected_identity = listed
            .verified_application
            .as_deref()
            .ok_or("installed application has no launch-bound identity")?;
        let descriptor = self
            .catalog_descriptors
            .iter()
            .find(|descriptor| descriptor.id == application_id)
            .ok_or("installed application changed; enumerate it again")?;
        let application = self
            .launch_applications
            .iter()
            .find(|application| application.id() == application_id)
            .cloned()
            .ok_or("installed application changed; enumerate it again")?;
        if descriptor.identity != expected_identity
            || descriptor.policy != RuntimePolicy::LaunchBound
        {
            return Err("installed application changed; enumerate it again".into());
        }
        Ok(LaunchPlan {
            catalog_generation,
            application_id: application_id.into(),
            identity: expected_identity.into(),
            digest: descriptor.digest,
            application,
        })
    }

    /// Revalidate every owner-controlled catalog field immediately before commit.
    pub(crate) fn revalidate_launch(&self, plan: &LaunchPlan) -> Result<(), String> {
        let current = self.plan_launch(plan.catalog_generation, &plan.application_id)?;
        if current.identity != plan.identity || current.digest != plan.digest {
            return Err("installed application changed; enumerate it again".into());
        }
        Ok(())
    }
}
impl OwnerRegistry {
    /// Request an early bounded refresh from the existing production worker.
    /// A full queue already contains an equivalent request, so coalescing is a
    /// successful request rather than a reason to block the compositor owner.
    pub(crate) fn request_refresh(&self) {
        let _ = self.refresh.try_send(());
    }

    pub(crate) fn poll(&mut self, control: &mut nickel_remote_control::ControlPlane) {
        let now = Instant::now();
        if now >= self.next_refresh {
            let _ = self.refresh.try_send(());
            self.next_refresh = now + Duration::from_secs(5);
        }
        if let Ok(snapshot) = self.snapshots.try_recv() {
            if fresh_catalog(snapshot.started, snapshot.completed, Instant::now()) {
                self.registry
                    .reconcile(snapshot.catalog.descriptors.clone(), |id| {
                        control.leases_mut().revoke(id);
                    });
                let changed = self.catalog_truncated != snapshot.catalog.truncated
                    || !same_catalog(&self.applications, &snapshot.catalog.applications)
                    || self.catalog_descriptors != snapshot.catalog.descriptors;
                if !changed || self.catalog_generation < u64::MAX {
                    if changed {
                        self.catalog_generation += 1;
                    }
                    self.applications = snapshot.catalog.applications;
                    self.catalog_descriptors = snapshot.catalog.descriptors;
                    self.launch_applications = snapshot.catalog.launch_applications;
                    self.catalog_truncated = snapshot.catalog.truncated;
                    self.observed = Some(snapshot.completed);
                } else {
                    self.applications.clear();
                    self.catalog_descriptors.clear();
                    self.launch_applications.clear();
                    self.catalog_truncated = false;
                    self.observed = None;
                }
            } else {
                self.registry.reconcile(Vec::new(), |id| {
                    control.leases_mut().revoke(id);
                });
                self.observed = None;
                self.applications.clear();
                self.catalog_descriptors.clear();
                self.launch_applications.clear();
                self.catalog_truncated = false;
            }
        }
        // A worker can publish after this poll's initial clock observation.
        // Sample again at admission/expiry rather than rejecting fresh progress.
        let now = Instant::now();
        // Failed/hung discovery cannot preserve stale catalog authority forever.
        if self
            .observed
            .is_none_or(|observed| now.duration_since(observed) >= MAX_CATALOG_AGE)
        {
            self.registry.reconcile(Vec::new(), |id| {
                control.leases_mut().revoke(id);
            });
            self.applications.clear();
            self.catalog_descriptors.clear();
            self.launch_applications.clear();
            self.catalog_truncated = false;
        }
        for _ in 0..16 {
            let Ok(receipt) = self.receipts.try_recv() else {
                break;
            };
            self.registry.attest(
                &receipt.descriptor,
                receipt.process,
                receipt.invocation,
                |id| {
                    control.leases_mut().revoke(id);
                },
            );
        }
        let active: Vec<_> = control
            .leases()
            .iter()
            .filter_map(|lease| match &lease.scope {
                nickel_remote_control::leases::ResourceScope::Application(id)
                    if id.starts_with("windows:catalog-launch:") =>
                {
                    Some((lease.id, id.clone()))
                }
                _ => None,
            })
            .collect();
        self.registry.sync_leases(&active, |id| {
            control.leases_mut().revoke(id);
        });
    }

    /// Future resource owner must call this with freshly validated process/HWND
    /// evidence for every operation. Unknown external processes remain eligible
    /// for explicit window scope, never discovered same-image app membership.
    pub(crate) fn verified_application<'a>(
        &'a self,
        process: &'a WindowsProcessIdentity,
    ) -> Option<&'a str> {
        process.verified_application().or_else(|| {
            self.observed
                .filter(|observed| observed.elapsed() < MAX_CATALOG_AGE)?;
            self.registry.membership(process)
        })
    }

    /// Returns whether the fresh production Start Menu catalog contains an
    /// exact launch-bound identity. This permits a trusted local application
    /// lease decision before the application has created its first window;
    /// later windows must still prove the same process-backed identity.
    pub(crate) fn contains_application(&self, identity: &str) -> bool {
        self.observed
            .is_some_and(|observed| observed.elapsed() < MAX_CATALOG_AGE)
            && self
                .applications
                .iter()
                .any(|application| application.verified_application.as_deref() == Some(identity))
    }

    pub(crate) fn inventory(
        &self,
        session_start: Instant,
        observation_generation: u64,
        expected_application: Option<&str>,
    ) -> nickel_remote_control::diagnostics::ApplicationInventory {
        let observed_at_us = session_start.elapsed().as_micros().min(u64::MAX as u128) as u64;
        let fresh = self
            .observed
            .is_some_and(|observed| observed.elapsed() < MAX_CATALOG_AGE);
        let applications = if fresh {
            self.applications
                .iter()
                .filter(|application| {
                    expected_application.is_none_or(|expected| {
                        application.verified_application.as_deref() == Some(expected)
                    })
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        nickel_remote_control::diagnostics::ApplicationInventory {
            observation_generation,
            observed_at_us,
            catalog_observed_at_us: self.observed.map_or(0, |observed| {
                observed
                    .saturating_duration_since(session_start)
                    .as_micros()
                    .min(u64::MAX as u128) as u64
            }),
            catalog_generation: self.catalog_generation,
            available: fresh && self.catalog_generation != 0,
            applications,
            truncated: fresh && expected_application.is_none() && self.catalog_truncated,
        }
    }
}
impl Drop for OwnerRegistry {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planned_registry() -> (OwnerRegistry, String, String) {
        let path = r"C:\missing\PlanOnly.lnk".to_owned();
        let application_id = format!("windows-shortcut:{}", path.to_ascii_lowercase());
        let identity = "windows:catalog-launch:test-plan".to_owned();
        let application = Application::new(
            application_id.clone(),
            "Plan only".into(),
            None,
            None,
            Some(vec![path]),
        );
        let mut registry = OwnerRegistry::default();
        registry.catalog_generation = 7;
        registry.applications = vec![nickel_remote_control::diagnostics::InstalledApplication {
            id: application_id.clone(),
            name: "Plan only".into(),
            verified_application: Some(identity.clone()),
        }];
        registry.catalog_descriptors = vec![Descriptor {
            id: application_id.clone(),
            identity: identity.clone(),
            digest: [9; 32],
            policy: RuntimePolicy::LaunchBound,
        }];
        registry.launch_applications = vec![application];
        (registry, application_id, identity)
    }

    #[test]
    fn launch_plan_is_catalog_only_and_revalidates_exact_digest() {
        let (mut registry, application_id, identity) = planned_registry();
        // The shortcut deliberately does not exist: owner planning must not touch it.
        let plan = registry.plan_launch(7, &application_id).unwrap();
        assert_eq!(plan.catalog_generation(), 7);
        assert_eq!(plan.application_id(), application_id);
        assert_eq!(plan.identity(), identity);
        registry.revalidate_launch(&plan).unwrap();

        registry.catalog_descriptors[0].digest[0] ^= 1;
        assert!(registry.revalidate_launch(&plan).is_err());
    }

    #[test]
    fn launch_plan_rejects_stale_catalog_generation() {
        let (registry, application_id, _) = planned_registry();
        assert!(registry.plan_launch(6, &application_id).is_err());
    }

    #[test]
    fn catalog_read_does_not_hold_launch_ancestry_or_write_locks() {
        use windows::Win32::Storage::FileSystem::{
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("menu");
        std::fs::create_dir(&directory).unwrap();
        let shortcut = directory.join("App.lnk");
        std::fs::write(&shortcut, b"catalog fixture").unwrap();
        let path = shortcut.to_str().unwrap().to_owned();
        let application = Application::new(
            format!("windows-shortcut:{}", path.to_ascii_lowercase()),
            "App".into(),
            None,
            None,
            Some(vec![path]),
        );
        let (_descriptor, file) = read_descriptor(
            &application,
            (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0,
        )
        .unwrap();
        std::fs::write(&shortcut, b"allowed update").unwrap();
        std::fs::rename(&directory, root.path().join("moved")).unwrap();
        assert!(file.metadata().unwrap().is_file());
    }

    #[test]
    fn observed_shortcut_pins_its_canonical_path_and_ancestors() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("menu");
        std::fs::create_dir(&directory).unwrap();
        let shortcut = directory.join("App.lnk");
        std::fs::write(&shortcut, b"descriptor fixture; no launch occurs").unwrap();
        let path = shortcut.to_str().unwrap().to_owned();
        let application = Application::new(
            format!("windows-shortcut:{}", path.to_ascii_lowercase()),
            "App".into(),
            None,
            None,
            Some(vec![path]),
        );
        let capture = LaunchCapture::prepare(&application).unwrap();
        assert!(capture.target().starts_with(r"\\?\Volume{"));
        assert!(OpenOptions::new().write(true).open(&shortcut).is_err());
        // Actual native regression checks, not inferred from share flags. These
        // remain unexecuted until run on Windows with a supported local volume.
        std::fs::write(directory.join("unrelated.txt"), b"allowed").unwrap();
        std::fs::write(root.path().join("unrelated.txt"), b"allowed").unwrap();
        assert!(std::fs::rename(&directory, root.path().join("moved")).is_err());
        drop(capture);
        std::fs::rename(&directory, root.path().join("moved")).unwrap();
    }
}
