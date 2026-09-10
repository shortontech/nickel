//! Private Windows Settings transport. This is not an agent endpoint.
//!
//! The pipe ACL admits this Windows user only, remote clients are rejected, and
//! both ends check the connected process against the pinned companion binary,
//! current session and integrity level. No environment secret is inherited by
//! applications. All pipe I/O is nonblocking and bounded.
use crate::process_identity::{WindowsProcessIdentity, same_process_image};
use std::{
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HANDLE, HLOCAL, INVALID_HANDLE_VALUE, LocalFree},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SDDL_REVISION_1,
            },
            GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
            TOKEN_USER, TokenUser,
        },
        Storage::FileSystem::{
            FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ,
            FILE_SHARE_READ, PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_IDENTIFICATION,
            SECURITY_SQOS_PRESENT, WriteFile,
        },
        System::{
            Pipes::*,
            Threading::{
                GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_INFORMATION,
            },
        },
    },
    core::{PCWSTR, PWSTR},
};

const MAX_BYTES: usize = 196_608;
const WAIT: Duration = Duration::from_millis(5);
const DEADLINE: Duration = Duration::from_secs(2);

fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "untrusted local control peer",
    )
}
fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

struct LocalAllocation(HLOCAL);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: Only successful Windows LocalAlloc-family outputs are wrapped.
        unsafe {
            let _ = LocalFree(Some(self.0));
        }
    }
}

fn process_user_sid(process: HANDLE) -> io::Result<String> {
    let mut token = HANDLE::default();
    // SAFETY: Current process pseudo-handle is valid; output is writable.
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.map_err(io::Error::other)?;
    // SAFETY: Successful OpenProcessToken transfers an owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token.0) };
    let mut storage = [0usize; 128];
    let mut size = 0;
    // SAFETY: Aligned storage and its complete writable byte size are provided.
    unsafe {
        GetTokenInformation(
            HANDLE(token.as_raw_handle()),
            TokenUser,
            Some(storage.as_mut_ptr().cast()),
            std::mem::size_of_val(&storage) as u32,
            &mut size,
        )
    }
    .map_err(io::Error::other)?;
    if (size as usize) < std::mem::size_of::<TOKEN_USER>()
        || size as usize > std::mem::size_of_val(&storage)
    {
        return Err(denied());
    }
    // SAFETY: Successful OS call initialized a TOKEN_USER and its in-buffer SID.
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    let mut sid = PWSTR::null();
    // SAFETY: SID remains valid inside storage throughout conversion.
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) }.map_err(io::Error::other)?;
    let allocation = LocalAllocation(HLOCAL(sid.0.cast()));
    // SAFETY: Conversion returned a NUL-terminated UTF-16 allocation.
    let result = unsafe { sid.to_string() }.map_err(io::Error::other);
    drop(allocation);
    result
}

fn user_sid() -> io::Result<String> {
    // SAFETY: GetCurrentProcess returns the valid current process pseudo-handle.
    process_user_sid(unsafe { GetCurrentProcess() })
}

struct Peer {
    pin: File,
    session: u32,
    integrity: u32,
    user: String,
}
impl Peer {
    fn companion(name: &str) -> io::Result<Self> {
        let path = std::fs::canonicalize(std::env::current_exe()?.with_file_name(name))?;
        // Deny delete/write sharing for the life of the endpoint: a pathname
        // replacement cannot change which companion executable is authorized.
        let pin = OpenOptions::new()
            .access_mode((FILE_GENERIC_READ | FILE_GENERIC_EXECUTE).0)
            .share_mode(FILE_SHARE_READ.0)
            .open(&path)?;
        let this = WindowsProcessIdentity::probe(std::process::id())?;
        Ok(Self {
            pin,
            session: this.session_id(),
            integrity: this.integrity_level()?,
            user: user_sid()?,
        })
    }
    fn verify(&self, pid: u32) -> io::Result<()> {
        let identity = WindowsProcessIdentity::probe(pid)?;
        if identity.session_id() != self.session || identity.integrity_level()? != self.integrity {
            return Err(denied());
        }
        // SAFETY: Query-only, noninheritable owned process handle.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, false, pid) }
            .map_err(io::Error::other)?;
        // SAFETY: Successful OpenProcess transfers ownership exactly once.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        if process_user_sid(HANDLE(handle.as_raw_handle()))? != self.user {
            return Err(denied());
        }
        same_process_image(
            HANDLE(handle.as_raw_handle()),
            HANDLE(self.pin.as_raw_handle()),
        )?;
        if !identity.is_live() {
            return Err(denied());
        }
        Ok(())
    }
}

fn pipe_name() -> io::Result<Vec<u16>> {
    let session = WindowsProcessIdentity::probe(std::process::id())?.session_id();
    Ok(wide(std::ffi::OsStr::new(&format!(
        r"\\.\pipe\nickel-settings-{}-{session}",
        user_sid()?
    ))))
}

fn receive(handle: HANDLE, deadline: Instant, stop: &AtomicBool) -> io::Result<Vec<u8>> {
    loop {
        if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "local control response timed out",
            ));
        }
        let mut available = 0;
        // SAFETY: Valid pipe handle and writable count; no buffer is borrowed.
        unsafe { PeekNamedPipe(handle, None, 0, None, Some(&mut available), None) }
            .map_err(io::Error::other)?;
        if available > 0 {
            if available as usize > MAX_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "local control frame exceeds limit",
                ));
            }
            let mut bytes = vec![0; MAX_BYTES];
            let mut read = 0;
            // SAFETY: Buffer is writable; pipe is nonblocking message mode.
            unsafe { ReadFile(handle, Some(&mut bytes), Some(&mut read), None) }
                .map_err(io::Error::other)?;
            bytes.truncate(read as usize);
            return Ok(bytes);
        }
        thread::sleep(WAIT);
    }
}
fn send(handle: HANDLE, bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid local control frame size",
        ));
    }
    let mut written = 0;
    // SAFETY: Valid nonblocking pipe and initialized bytes for the full call.
    unsafe { WriteFile(handle, Some(bytes), Some(&mut written), None) }
        .map_err(io::Error::other)?;
    if written as usize != bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "local control pipe is full",
        ));
    }
    Ok(())
}

pub struct LocalControlServer {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl LocalControlServer {
    pub fn start(
        mut handle_request: impl FnMut(Vec<u8>) -> io::Result<Vec<u8>> + Send + 'static,
    ) -> io::Result<Self> {
        let peer = Peer::companion("nickel-settings.exe")?;
        let name = pipe_name()?;
        let sddl = wide(std::ffi::OsStr::new(&format!(
            "D:P(A;;GA;;;{})",
            user_sid()?
        )));
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: NUL-terminated SDDL and valid output for allocated descriptor.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(io::Error::other)?;
        let allocation = LocalAllocation(HLOCAL(descriptor.0));
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        // SAFETY: Descriptor lives through creation. First-instance flag rejects
        // an existing endpoint; reject-remote protects against network access.
        let raw = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE
                    | PIPE_READMODE_MESSAGE
                    | PIPE_NOWAIT
                    | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                MAX_BYTES as u32,
                MAX_BYTES as u32,
                0,
                Some(&attributes),
            )
        };
        drop(allocation);
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: Successful creation transfers the handle to RAII ownership.
        let pipe = unsafe { OwnedHandle::from_raw_handle(raw.0) };
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = thread::Builder::new()
            .name("nickel-settings-pipe".into())
            .spawn(move || {
                let handle = HANDLE(pipe.as_raw_handle());
                while !stopping.load(Ordering::Acquire) {
                    // SAFETY: Valid owned nonblocking pipe; no OVERLAPPED operation.
                    unsafe {
                        let _ = ConnectNamedPipe(handle, None);
                    }
                    let mut pid = 0;
                    // SAFETY: Writable PID, queried only from the OS pipe endpoint.
                    if unsafe { GetNamedPipeClientProcessId(handle, &mut pid) }.is_err() {
                        thread::sleep(WAIT);
                        continue;
                    }
                    let result = peer
                        .verify(pid)
                        .and_then(|()| receive(handle, Instant::now() + DEADLINE, &stopping))
                        .and_then(&mut handle_request);
                    if let Ok(bytes) = result {
                        let _ = send(handle, &bytes);
                    }
                    // A client must read before disconnect. Poll until it closes or
                    // deadline; never flush a pipe (which would block on the peer).
                    let deadline = Instant::now() + DEADLINE;
                    while Instant::now() < deadline && !stopping.load(Ordering::Acquire) {
                        let mut available = 0;
                        // SAFETY: Valid handle, writable size. Disconnect is terminal.
                        if unsafe {
                            PeekNamedPipe(handle, None, 0, None, Some(&mut available), None)
                        }
                        .is_err()
                        {
                            break;
                        }
                        thread::sleep(WAIT);
                    }
                    // SAFETY: Reset only our single owned pipe instance.
                    unsafe {
                        let _ = DisconnectNamedPipe(handle);
                    }
                }
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}
impl Drop for LocalControlServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub fn request(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let peer = Peer::companion("nickel.exe")?;
    let name = pipe_name()?;
    use std::os::windows::ffi::OsStringExt;
    let path = std::ffi::OsString::from_wide(&name[..name.len() - 1]);
    // Identification-only SQOS prevents a spoof pipe server impersonating Settings.
    let pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags((SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION).0)
        .open(path)?;
    let handle = HANDLE(pipe.as_raw_handle());
    let mut pid = 0;
    // SAFETY: Valid client handle and writable server PID output.
    unsafe { GetNamedPipeServerProcessId(handle, &mut pid) }.map_err(io::Error::other)?;
    peer.verify(pid)?;
    let mode = PIPE_READMODE_MESSAGE | PIPE_NOWAIT;
    // SAFETY: Switch our connected endpoint to nonblocking message reads.
    unsafe { SetNamedPipeHandleState(handle, Some(&mode), None, None) }
        .map_err(io::Error::other)?;
    send(handle, bytes)?;
    receive(handle, Instant::now() + DEADLINE, &AtomicBool::new(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_peer_checks_current_process_and_rejects_wrong_companion() {
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_str().unwrap();
        let peer = Peer::companion(name).unwrap();
        peer.verify(std::process::id()).unwrap();
        assert!(peer.verify(0).is_err());
        let temporary = tempfile::tempdir().unwrap();
        let copied = temporary.path().join("copied-image.exe");
        std::fs::copy(&exe, &copied).unwrap();
        let other = OpenOptions::new()
            .access_mode((FILE_GENERIC_READ | FILE_GENERIC_EXECUTE).0)
            .share_mode(FILE_SHARE_READ.0)
            .open(copied)
            .unwrap();
        // SAFETY: Current process pseudo-handle remains valid during comparison.
        assert!(
            same_process_image(
                unsafe { GetCurrentProcess() },
                HANDLE(other.as_raw_handle())
            )
            .is_err()
        );
        assert!(Peer::companion("missing-nickel-companion.exe").is_err());
        assert!(user_sid().unwrap().starts_with("S-1-"));
    }
    #[test]
    fn oversized_private_frame_is_rejected_before_native_write() {
        assert_eq!(
            send(INVALID_HANDLE_VALUE, &vec![0; MAX_BYTES + 1])
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            send(INVALID_HANDLE_VALUE, &[]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
