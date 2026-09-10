//! Windows process evidence for resource-scoped control.
//!
//! Call probes on a platform worker. A retained process handle and creation time
//! identify the process instance; they do not identify an HWND lifetime or grant
//! permission. The desktop owner must revalidate the window/process relationship
//! and its own window generation at every operation.
use std::{
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStringExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
};
use windows::{
    Win32::{
        Foundation::{
            APPMODEL_ERROR_NO_PACKAGE, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, FILETIME, HANDLE,
            WAIT_TIMEOUT,
        },
        Security::{GetTokenInformation, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel},
        Storage::{
            FileSystem::{
                FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_ID_INFO, FILE_SHARE_READ, FileIdInfo,
                GetFileInformationByHandleEx, GetFinalPathNameByHandleW, VOLUME_NAME_GUID,
            },
            Packaging::Appx::GetPackageFamilyName,
        },
        System::{
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
            RemoteDesktop::ProcessIdToSessionId,
            Threading::{
                GetProcessInformation, GetProcessTimes, OpenProcess, OpenProcessToken,
                PROCESS_NAME_WIN32, PROCESS_PROTECTION_LEVEL_INFORMATION,
                PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
                PROTECTION_LEVEL_NONE, ProcessProtectionLevelInfo, QueryFullProcessImageNameW,
                WaitForSingleObject,
            },
        },
    },
    core::PWSTR,
};

const MAX_PACKAGE_FAMILY_UNITS: u32 = 512;

/// OS-backed process evidence. Intentionally not serializable or Debug: callers
/// expose only the bounded, lease-authorized projection they actually need.
pub struct WindowsProcessIdentity {
    handle: OwnedHandle,
    process_id: u32,
    created_at: u64,
    session_id: u32,
    verified_application: Option<String>,
    executable: Option<ExecutableEvidence>,
}
/// Kernel-verified executable file evidence, separate from application identity.
/// Clones retain the file against writes/deletion/replacement and file-ID reuse,
/// including after the last originating process exits. An application lease
/// consumer must retain a clone until revocation/expiry and discard all decisions
/// when the last evidence is released. Never persist a file ID as a grant.
///
/// No raw identifier, serialization or constructor is exposed. Future catalog
/// matching must compare retained evidence and apply explicit shared-runtime
/// policy; comparing files alone never grants application membership.
#[derive(Clone)]
pub struct ExecutableEvidence {
    file: crate::executable_identity::RetainedExecutable<File>,
}
impl ExecutableEvidence {
    /// Both sides retain their verified files throughout comparison. This proves
    /// executable file equality only, not script, publisher or app identity.
    pub fn same_file(&self, other: &Self) -> bool {
        self.file.same_file(&other.file)
    }
}

impl WindowsProcessIdentity {
    pub fn probe(process_id: u32) -> io::Result<Self> {
        if process_id == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid process identity",
            ));
        }
        // SAFETY: OpenProcess receives a numeric PID, requests query/wait rights
        // only, and returns an owned handle on success. Inheritance is disabled.
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                process_id,
            )
        }
        .map_err(io::Error::other)?;
        // SAFETY: Ownership of the successful OpenProcess handle transfers once
        // to OwnedHandle, which closes it on every subsequent failure and drop.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        let raw = HANDLE(handle.as_raw_handle());
        let created_at = creation_time(raw)?;
        let mut session_id = 0;
        // SAFETY: The output points to a writable u32. The final retained-handle
        // liveness check rejects an exit/PID-reuse race during this PID query.
        unsafe { ProcessIdToSessionId(process_id, &mut session_id) }.map_err(io::Error::other)?;
        let package = package_family(raw)?;
        let verified_application =
            crate::executable_identity::verified_application(package.as_deref());
        // Stronger query/file rights are optional evidence. They neither replace
        // package membership nor discard existing limited-query process evidence.
        let executable = if package.is_none() {
            executable_identity(process_id, created_at).ok()
        } else {
            None
        };
        let result = Self {
            handle,
            process_id,
            created_at,
            session_id,
            verified_application,
            executable,
        };
        if !result.is_live() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "process exited during identity probe",
            ));
        }
        Ok(result)
    }

    /// Revalidate a retained native launch handle, never a process ID supplied by
    /// a caller. This establishes process incarnation only; the launch owner is
    /// responsible for binding its exact command/catalog descriptor to the handle.
    pub fn from_retained_process(process: &OwnedHandle) -> io::Result<Self> {
        let handle = HANDLE(process.as_raw_handle());
        // SAFETY: Borrowed owned process handle remains alive throughout queries.
        let pid = unsafe { windows::Win32::System::Threading::GetProcessId(handle) };
        let expected_creation = creation_time(handle)?;
        let identity = Self::probe(pid)?;
        if identity.created_at() != expected_creation || !identity.is_live() {
            return Err(image_unavailable());
        }
        Ok(identity)
    }

    /// OS-reported executable basename for conservative protection exclusions.
    /// This is not verified application membership, a signature, or a grant.
    pub fn executable_name(&self) -> io::Result<String> {
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        // SAFETY: Retained process handle and bounded writable UTF-16 output.
        unsafe {
            QueryFullProcessImageNameW(
                HANDLE(self.handle.as_raw_handle()),
                PROCESS_NAME_WIN32,
                PWSTR(path.as_mut_ptr()),
                &mut length,
            )
        }
        .map_err(io::Error::other)?;
        if length == 0
            || length as usize >= path.len()
            || path[..length as usize].contains(&0)
            || !self.is_live()
        {
            return Err(image_unavailable());
        }
        let path =
            std::path::PathBuf::from(std::ffi::OsString::from_wide(&path[..length as usize]));
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(image_unavailable)?;
        Ok(name.to_owned())
    }

    pub fn process_id(&self) -> u32 {
        self.process_id
    }
    /// Windows creation FILETIME; this is identity evidence, not a lease clock.
    pub fn created_at(&self) -> u64 {
        self.created_at
    }
    pub fn session_id(&self) -> u32 {
        self.session_id
    }
    /// Fresh OS token integrity RID. Run on a platform worker; failure provides
    /// no eligibility evidence. The owner must compare it against its own token
    /// and separately enforce protected desktops and resource authorization.
    pub fn integrity_level(&self) -> io::Result<u32> {
        let mut token = HANDLE::default();
        // SAFETY: The retained process handle permits query and the output is
        // writable. TOKEN_QUERY grants observation only, not impersonation.
        unsafe { OpenProcessToken(HANDLE(self.handle.as_raw_handle()), TOKEN_QUERY, &mut token) }
            .map_err(io::Error::other)?;
        // SAFETY: OpenProcessToken transferred this successfully opened token
        // handle; OwnedHandle closes it exactly once, including error paths.
        let token = unsafe { OwnedHandle::from_raw_handle(token.0) };
        // Aligned storage for TOKEN_MANDATORY_LABEL and its in-buffer SID. Even
        // on 32-bit Windows this exceeds a label plus the maximum 68-byte SID.
        let mut storage = [0usize; 64];
        let mut returned = 0;
        // SAFETY: The byte count describes the complete writable aligned buffer.
        // Windows returns the label and SID in this storage; both are validated
        // before their contents are used. There is no unbounded allocation/retry.
        unsafe {
            GetTokenInformation(
                HANDLE(token.as_raw_handle()),
                TokenIntegrityLevel,
                Some(storage.as_mut_ptr().cast()),
                std::mem::size_of_val(&storage) as u32,
                &mut returned,
            )
        }
        .map_err(io::Error::other)?;
        let label_size = std::mem::size_of::<TOKEN_MANDATORY_LABEL>();
        if (returned as usize) < label_size || returned as usize > std::mem::size_of_val(&storage) {
            return Err(invalid_integrity());
        }
        // SAFETY: A successful query initialized a complete label in storage,
        // whose reported size was checked above. Unaligned read avoids relying
        // on incidental struct alignment; its SID pointer is not dereferenced.
        let label =
            unsafe { std::ptr::read_unaligned(storage.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()) };
        let offset = (label.Label.Sid.0 as usize)
            .checked_sub(storage.as_ptr() as usize)
            .filter(|offset| *offset >= label_size)
            .ok_or_else(invalid_integrity)?;
        // SAFETY: This view covers only the initialized byte storage, within the
        // allocation and checked returned length. SID access below is via slices.
        let bytes =
            unsafe { std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), returned as usize) };
        let sid = bytes.get(offset..).ok_or_else(invalid_integrity)?;
        let level = integrity_rid(sid)?;
        if !self.is_live() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "process exited during token query",
            ));
        }
        Ok(level)
    }
    /// A necessary process eligibility check, not permission to control a window.
    /// The owner must separately enforce desktop, integrity, protected-UI and
    /// resource-lease policy. Query failure and every protected level fail closed.
    /// Protection is queried again on each call rather than cached at discovery.
    pub fn is_unprotected_in_session(&self, session_id: u32) -> bool {
        if self.session_id != session_id || !self.is_live() {
            return false;
        }
        let mut information = PROCESS_PROTECTION_LEVEL_INFORMATION::default();
        // SAFETY: The retained handle grants limited query rights; the output
        // buffer and its byte count match the selected information class.
        let queried = unsafe {
            GetProcessInformation(
                HANDLE(self.handle.as_raw_handle()),
                ProcessProtectionLevelInfo,
                std::ptr::from_mut(&mut information).cast(),
                std::mem::size_of_val(&information) as u32,
            )
        };
        queried.is_ok() && information.ProtectionLevel == PROTECTION_LEVEL_NONE && self.is_live()
    }
    /// Package identity is supplied by the OS, never a window label or AUMID.
    /// Unpackaged executable evidence is deliberately NOT application membership:
    /// interpreters and shared runtimes can host unrelated applications. A future
    /// normalized app catalog must establish that relationship and retain its
    /// executable evidence for the entire application lease before authorizing it.
    /// An exited process cannot supply current application membership evidence.
    pub fn verified_application(&self) -> Option<&str> {
        self.is_live()
            .then_some(self.verified_application.as_deref())
            .flatten()
    }

    /// Retain verified image evidence while the originating process is live.
    /// The returned pin can outlive that process (for example in an eventual
    /// lease registry). It does not prove another process's application membership.
    pub fn executable_evidence(&self) -> Option<ExecutableEvidence> {
        if self.is_live() {
            self.executable.clone()
        } else {
            None
        }
    }

    pub fn is_live(&self) -> bool {
        // SAFETY: The retained process handle has SYNCHRONIZE rights. A zero
        // timeout never waits; all results except an unsignaled process fail closed.
        unsafe { WaitForSingleObject(HANDLE(self.handle.as_raw_handle()), 0) == WAIT_TIMEOUT }
    }
    pub fn matches_live_process(&self, process_id: u32) -> bool {
        self.process_id == process_id && self.is_live()
    }
}

fn invalid_integrity() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid process integrity evidence",
    )
}

fn integrity_rid(sid: &[u8]) -> io::Result<u32> {
    // A mandatory integrity SID is S-1-16-RID: revision 1, one subauthority,
    // the six-byte big-endian mandatory-label authority, and a little-endian RID.
    if sid.len() < 12 || sid[..8] != [1, 1, 0, 0, 0, 0, 0, 16] {
        return Err(invalid_integrity());
    }
    Ok(u32::from_le_bytes(
        sid[8..12].try_into().map_err(|_| invalid_integrity())?,
    ))
}

fn creation_time(handle: HANDLE) -> io::Result<u64> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: The handle grants process query rights and all output pointers
    // refer to initialized writable FILETIME values for this synchronous call.
    unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
        .map_err(io::Error::other)?;
    let value = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    if value == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "process creation identity unavailable",
        ));
    }
    Ok(value)
}

fn package_family(handle: HANDLE) -> io::Result<Option<String>> {
    let mut units = 0;
    // SAFETY: The first call supplies no buffer and obtains the required count.
    let status = unsafe { GetPackageFamilyName(handle, &mut units, None) };
    if status == APPMODEL_ERROR_NO_PACKAGE {
        return Ok(None);
    }
    if status != ERROR_INSUFFICIENT_BUFFER {
        return Err(io::Error::from_raw_os_error(status.0 as i32));
    }
    if !(2..=MAX_PACKAGE_FAMILY_UNITS).contains(&units) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "package identity exceeds its bound",
        ));
    }
    let mut buffer = vec![0u16; units as usize];
    // SAFETY: Buffer length matches the size returned by Windows and remains
    // alive and writable through the synchronous call; the returned size is checked.
    let status =
        unsafe { GetPackageFamilyName(handle, &mut units, Some(PWSTR(buffer.as_mut_ptr()))) };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status.0 as i32));
    }
    if units < 2 || units as usize > buffer.len() || buffer[units as usize - 1] != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid package identity length",
        ));
    }
    let value = String::from_utf16(&buffer[..units as usize - 1]).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid package identity encoding",
        )
    })?;
    if value.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid package identity terminator",
        ));
    }
    Ok(Some(value))
}

/// Query the original image's kernel file identity, never its cached pathname.
/// ProcessImageFileMapping consumes a FILE_EXECUTE | SYNCHRONIZE handle and
/// compares the file's SectionObjectPointer with the process image. This NT
/// information class is not a stable Win32 contract: missing API, access denial,
/// unsupported class and every nonzero status all reject the peer.
/// Reference: https://github.com/m417z/ntdoc/blob/main/descriptions/processinfoclass.md#processimagefilemapping-44
pub(crate) fn same_process_image(process: HANDLE, file: HANDLE) -> io::Result<()> {
    type Query =
        unsafe extern "system" fn(HANDLE, u32, *mut std::ffi::c_void, u32, *mut u32) -> i32;
    // SAFETY: ntdll is loaded for every Windows process; this call borrows its
    // existing module reference and cannot load an attacker-chosen DLL.
    let module =
        unsafe { GetModuleHandleW(windows::core::w!("ntdll.dll")) }.map_err(io::Error::other)?;
    // SAFETY: Static NUL-terminated export name and valid borrowed module.
    let symbol = unsafe { GetProcAddress(module, windows::core::s!("NtQueryInformationProcess")) }
        .ok_or_else(image_unavailable)?;
    // SAFETY: The named export has the documented NtQueryInformationProcess ABI.
    // No fallback to path equality is used if the OS lacks this information class.
    let query: Query = unsafe { std::mem::transmute(symbol) };
    let mut file = file;
    // SAFETY: Class44 reads one initialized HANDLE from a correctly sized buffer;
    // both process and file handles stay owned by the caller throughout the call.
    let status = unsafe {
        query(
            process,
            44,
            std::ptr::from_mut(&mut file).cast(),
            std::mem::size_of::<HANDLE>() as u32,
            std::ptr::null_mut(),
        )
    };
    if status != 0 {
        return Err(image_unavailable());
    }
    Ok(())
}

fn image_unavailable() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "unverified process image")
}

/// Path queries locate a candidate only. The kernel image mapping check binds
/// that pinned file to the retained process instance before any ID is projected.
fn executable_identity(process_id: u32, created_at: u64) -> io::Result<ExecutableEvidence> {
    // SAFETY: Numeric PID, read-only query rights, and inheritance disabled.
    let process = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, false, process_id) }
        .map_err(io::Error::other)?;
    // SAFETY: Transfer sole ownership of the successful OpenProcess result.
    let process = unsafe { OwnedHandle::from_raw_handle(process.0) };
    let raw = HANDLE(process.as_raw_handle());
    if creation_time(raw)? != created_at {
        return Err(image_unavailable());
    }
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    // SAFETY: Retained process handle and bounded writable UTF-16 buffer/count.
    unsafe {
        QueryFullProcessImageNameW(
            raw,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut length,
        )
    }
    .map_err(io::Error::other)?;
    if length == 0 || length as usize >= path.len() || path[..length as usize].contains(&0) {
        return Err(image_unavailable());
    }
    let path = std::ffi::OsString::from_wide(&path[..length as usize]);
    let image = OpenOptions::new()
        .access_mode((FILE_GENERIC_READ | FILE_GENERIC_EXECUTE).0)
        .share_mode(FILE_SHARE_READ.0)
        .open(path)?;
    let file = HANDLE(image.as_raw_handle());
    same_process_image(raw, file)?;
    let mut information = FILE_ID_INFO::default();
    // SAFETY: Exact FILE_ID_INFO layout/size and valid retained file handle.
    unsafe {
        GetFileInformationByHandleEx(
            file,
            FileIdInfo,
            std::ptr::from_mut(&mut information).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    }
    .map_err(io::Error::other)?;
    let mut final_path = vec![0u16; 32768];
    // SAFETY: Writable bounded buffer and retained file. Volume GUID queries
    // exclude network shares and unsupported mount providers rather than relying
    // on remote/synthetic serial numbers as local application identity.
    let length =
        unsafe { GetFinalPathNameByHandleW(file, &mut final_path, VOLUME_NAME_GUID) } as usize;
    if length == 0 || length >= final_path.len() {
        return Err(image_unavailable());
    }
    let final_path = String::from_utf16(&final_path[..length]).map_err(|_| image_unavailable())?;
    let identity = crate::executable_identity::project(
        &final_path,
        information.VolumeSerialNumber,
        information.FileId.Identifier,
    )
    .ok_or_else(image_unavailable)?;
    Ok(ExecutableEvidence {
        file: crate::executable_identity::RetainedExecutable::new(identity, image),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "child fixture launched by retained_process_exit_retires_application_evidence"]
    fn process_identity_child_fixture() {
        if std::env::var_os("NICKEL_PROCESS_IDENTITY_CHILD_FIXTURE").is_some() {
            use std::io::Read;
            let mut byte = [0];
            let _ = std::io::stdin().read_exact(&mut byte);
        }
    }

    #[test]
    fn retained_process_exit_retires_application_evidence() {
        struct OwnedChild(std::process::Child);
        impl Drop for OwnedChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut child = OwnedChild(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "process_identity::tests::process_identity_child_fixture",
                    "--ignored",
                ])
                .env("NICKEL_PROCESS_IDENTITY_CHILD_FIXTURE", "1")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut identity = WindowsProcessIdentity::probe(child.0.id()).unwrap();
        let image = identity.executable_evidence().unwrap();
        let lease_pin = image.clone();
        // A test executable need not be packaged. Seed only the cached label
        // to test its lifetime gate; process evidence and exit are real OS state.
        identity.verified_application = Some("windows:package-family:fixture".into());
        assert!(identity.verified_application().is_some());
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        assert!(!identity.is_live());
        assert!(!identity.matches_live_process(child.0.id()));
        assert!(identity.verified_application().is_none());
        assert!(identity.executable_evidence().is_none());
        drop(identity);
        assert!(image.same_file(&lease_pin));
    }

    #[test]
    fn current_process_has_stable_os_creation_evidence_and_a_retained_live_handle() {
        let pid = std::process::id();
        let first = WindowsProcessIdentity::probe(pid).unwrap();
        let second = WindowsProcessIdentity::probe(pid).unwrap();
        assert_eq!(first.process_id(), pid);
        assert_ne!(first.created_at(), 0);
        assert_eq!(first.created_at(), second.created_at());
        let launch_handle = first.handle.try_clone().unwrap();
        let from_handle = WindowsProcessIdentity::from_retained_process(&launch_handle).unwrap();
        assert_eq!(from_handle.created_at(), first.created_at());
        assert_eq!(from_handle.process_id(), first.process_id());
        assert_eq!(first.session_id(), second.session_id());
        assert_eq!(
            first.integrity_level().unwrap(),
            second.integrity_level().unwrap()
        );
        assert!(first.is_unprotected_in_session(first.session_id()));
        assert!(!first.is_unprotected_in_session(first.session_id() ^ 1));
        assert_eq!(first.verified_application(), second.verified_application());
        assert!(first.matches_live_process(pid));
        assert!(!first.matches_live_process(0));
        drop(second);
        assert!(first.is_live());
        assert!(WindowsProcessIdentity::probe(0).is_err());
    }

    #[test]
    fn unpackaged_image_is_pinned_and_bound_to_the_process_instance() {
        let process = WindowsProcessIdentity::probe(std::process::id()).unwrap();
        let first = executable_identity(process.process_id(), process.created_at()).unwrap();
        let second = executable_identity(process.process_id(), process.created_at()).unwrap();
        assert!(first.same_file(&second));
        assert!(process.verified_application().is_none());
        assert!(process.executable_evidence().unwrap().same_file(&first));
        assert!(
            executable_identity(process.process_id(), process.created_at().wrapping_add(1))
                .is_err()
        );
        // Request write access only; never truncate or mutate the running image.
        assert!(
            OpenOptions::new()
                .write(true)
                .open(std::env::current_exe().unwrap())
                .is_err()
        );
        drop(process);
        assert!(first.same_file(&second));
    }

    #[test]
    fn integrity_sid_requires_the_mandatory_authority_and_one_complete_rid() {
        let medium = [1, 1, 0, 0, 0, 0, 0, 16, 0, 32, 0, 0];
        assert_eq!(integrity_rid(&medium).unwrap(), 0x2000);
        for length in 0..12 {
            assert!(integrity_rid(&medium[..length]).is_err());
        }
        for (index, value) in [(0, 2), (1, 0), (1, 2), (2, 1), (7, 5)] {
            let mut invalid = medium;
            invalid[index] = value;
            assert!(integrity_rid(&invalid).is_err());
        }
    }
}
