//! Suspended one-shot Windows launch broker. The child receives no path or
//! command: it derives its sole target from an inherited pinned shortcut.

use crate::windows_application_registry::native::LaunchCapture;
use nickel_remote_control::{DesktopPermit, leases::ResourceEvidence};
use std::{
    mem::{size_of, zeroed},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{
            DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, WAIT_OBJECT_0,
        },
        Security::SECURITY_ATTRIBUTES,
        Storage::FileSystem::{GetFinalPathNameByHandleW, ReadFile, VOLUME_NAME_GUID, WriteFile},
        System::{Pipes::CreatePipe, Threading::*},
        UI::{
            Shell::{
                SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
    core::{PCWSTR, PWSTR},
};

const COMMIT_TTL: Duration = Duration::from_secs(2);
const EXIT_GRACE: Duration = Duration::from_millis(250);
const MAGIC: u32 = 0x4e4c_4231;
const VERSION: u32 = 1;
// LaunchCapture admits at most 256 ancestors plus the shortcut itself.
const MAX_PINS: usize = 257;

#[repr(C)]
#[derive(Clone, Copy)]
struct Request {
    magic: u32,
    version: u32,
    nonce: u64,
    count: u32,
    reserved: u32,
    pins: [u64; MAX_PINS],
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct Response {
    magic: u32,
    version: u32,
    nonce: u64,
    status: u32,
    pid: u32,
    process: u64,
}

struct Handle(OwnedHandle);
impl Handle {
    /// Takes ownership of a successful Win32 API out-parameter immediately.
    unsafe fn from_api(raw: HANDLE) -> Self {
        debug_assert!(!raw.is_invalid());
        Self(unsafe { OwnedHandle::from_raw_handle(raw.0) })
    }

    unsafe fn new(raw: HANDLE) -> Result<Self, String> {
        if raw.is_invalid() {
            Err("Windows launch broker returned an invalid handle".into())
        } else {
            Ok(Self(unsafe { OwnedHandle::from_raw_handle(raw.0) }))
        }
    }
    fn raw(&self) -> HANDLE {
        HANDLE(self.0.as_raw_handle())
    }
    fn owned(self) -> OwnedHandle {
        self.0
    }
}
// SAFETY: ownership is unique and kernel handles may be operated on cross-thread.
unsafe impl Send for Handle {}

/// A handle value allocated in another process by `DuplicateHandle`.
///
/// The value cannot be wrapped in a local `OwnedHandle`, but this guard owns
/// the obligation to close it in `target_process` until `into_raw` explicitly
/// transfers that obligation to the receiver.
struct RemoteHandle<'a> {
    target_process: &'a Handle,
    raw: Option<HANDLE>,
}
impl<'a> RemoteHandle<'a> {
    fn duplicate(source: HANDLE, target_process: &'a Handle) -> Result<Self, String> {
        let mut raw = HANDLE::default();
        unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                source,
                target_process.raw(),
                &mut raw,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
            .map_err(error)?;
        }
        Ok(Self {
            target_process,
            raw: Some(raw),
        })
    }

    fn raw(&self) -> HANDLE {
        self.raw.expect("remote handle ownership")
    }

    /// Transfers remote-table ownership after the receiver acknowledges it.
    fn into_raw(mut self) -> HANDLE {
        self.raw.take().expect("remote handle ownership")
    }
}
impl Drop for RemoteHandle<'_> {
    fn drop(&mut self) {
        if let Some(raw) = self.raw.take() {
            close_remote(self.target_process.raw(), raw);
        }
    }
}

struct Attributes {
    words: Vec<usize>,
    initialized: bool,
}
impl Attributes {
    fn new(handles: &[HANDLE]) -> Result<Self, String> {
        let mut size = 0;
        unsafe {
            let _ = InitializeProcThreadAttributeList(None, 1, None, &mut size);
        }
        if size == 0 {
            return Err(last_error());
        }
        let mut value = Self {
            words: vec![0; size.div_ceil(size_of::<usize>())],
            initialized: false,
        };
        unsafe {
            InitializeProcThreadAttributeList(Some(value.raw()), 1, None, &mut size)
                .map_err(error)?;
        }
        value.initialized = true;
        unsafe {
            UpdateProcThreadAttribute(
                value.raw(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr().cast()),
                std::mem::size_of_val(handles),
                None,
                None,
            )
            .map_err(error)?;
        }
        Ok(value)
    }
    fn raw(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        LPPROC_THREAD_ATTRIBUTE_LIST(self.words.as_mut_ptr().cast())
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { DeleteProcThreadAttributeList(self.raw()) };
        }
    }
}

pub(crate) struct StagedLaunch {
    capture: Option<LaunchCapture>,
    deadline: Instant,
    process: Option<Handle>,
    thread: Option<Handle>,
    response: Option<Handle>,
    response_ready: Option<Handle>,
    ack: Option<Handle>,
    nonce: u64,
    committed: bool,
}
pub(crate) struct CommittedLaunch {
    capture: Option<LaunchCapture>,
    deadline: Instant,
    process: Handle,
    response: Handle,
    response_ready: Handle,
    ack: Handle,
    nonce: u64,
}
pub(crate) struct LaunchResult {
    pub process_spawn_confirmed: bool,
    pub process_id: Option<u32>,
    /// Retained incarnation evidence for bounded first-window attribution.
    /// This is never exposed as application authority or serialized.
    pub process: Option<std::sync::Arc<nickel_platform::process_identity::WindowsProcessIdentity>>,
}
unsafe impl Send for StagedLaunch {}
unsafe impl Send for CommittedLaunch {}

impl StagedLaunch {
    pub(crate) fn new(capture: LaunchCapture, original_deadline: Instant) -> Result<Self, String> {
        let deadline = original_deadline.min(Instant::now() + COMMIT_TTL);
        if Instant::now() >= deadline {
            return Err("Windows launch preparation expired".into());
        }
        let (request_read, request_write) = pipe(size_of::<Request>() as u32)?;
        let (response_read, response_write) = pipe(size_of::<Response>() as u32)?;
        let response_ready = event()?;
        let ack = event()?;
        let parent = duplicate(unsafe { GetCurrentProcess() })?;
        let mut pins = Vec::new();
        for raw in capture.inherited_handles() {
            if pins.len() == MAX_PINS {
                return Err("Windows launch target has too many pinned ancestors".into());
            }
            pins.push(duplicate(HANDLE(raw as *mut _))?);
        }
        if pins.is_empty() {
            return Err("Windows launch target has no pinned shortcut".into());
        }
        let nonce = nonce();
        let deadline_tick = tick_deadline(deadline);
        let inherited: Vec<_> = pins
            .iter()
            .map(Handle::raw)
            .chain([
                request_read.raw(),
                response_write.raw(),
                response_ready.raw(),
                ack.raw(),
                parent.raw(),
            ])
            .collect();
        let mut attributes = Attributes::new(&inherited)?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        use std::os::windows::ffi::OsStrExt;
        let exe_w: Vec<u16> = exe.as_os_str().encode_wide().chain([0]).collect();
        let mut command: Vec<u16> = format!(
            "\"{}\" --nickel-launch-broker {nonce} {deadline_tick} {} {} {} {} {}",
            exe.display(),
            request_read.raw().0 as usize,
            response_write.raw().0 as usize,
            response_ready.raw().0 as usize,
            ack.raw().0 as usize,
            parent.raw().0 as usize
        )
        .encode_utf16()
        .chain([0])
        .collect();
        let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.lpAttributeList = attributes.raw();
        let mut info: PROCESS_INFORMATION = unsafe { zeroed() };
        unsafe {
            CreateProcessW(
                PCWSTR(exe_w.as_ptr()),
                Some(PWSTR(command.as_mut_ptr())),
                None,
                None,
                true,
                CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
                None,
                PCWSTR::null(),
                &startup.StartupInfo,
                &mut info,
            )
            .map_err(error)?;
        }
        // Both returned handles enter RAII before the first later fallible operation.
        let (process, thread) = unsafe {
            (
                Handle::from_api(info.hProcess),
                Handle::from_api(info.hThread),
            )
        };
        let mut request = Request {
            magic: MAGIC,
            version: VERSION,
            nonce,
            count: pins.len() as u32,
            reserved: 0,
            pins: [0; MAX_PINS],
        };
        for (slot, pin) in request.pins.iter_mut().zip(&pins) {
            *slot = pin.raw().0 as usize as u64;
        }
        if let Err(e) = write_record(request_write.raw(), &request) {
            unsafe {
                let _ = TerminateProcess(process.raw(), 71);
            }
            return Err(e);
        }
        Ok(Self {
            capture: Some(capture),
            deadline,
            process: Some(process),
            thread: Some(thread),
            response: Some(response_read),
            response_ready: Some(response_ready),
            ack: Some(ack),
            nonce,
            committed: false,
        })
    }
    pub(crate) fn application_identity(&self) -> &str {
        self.capture
            .as_ref()
            .expect("capture")
            .application_identity()
    }
    pub(crate) fn commit(
        mut self,
        permit: &DesktopPermit,
        evidence: &ResourceEvidence<'_>,
        now: Instant,
    ) -> Result<CommittedLaunch, String> {
        if now >= self.deadline {
            return Err("Windows launch preparation expired".into());
        }
        self.capture.as_mut().expect("capture").begin_invocation();
        let thread = self.thread.as_ref().expect("thread").raw();
        permit.with_input(evidence, || {
            // The permit mutex may have been contended after the outer check.
            // Recheck the request deadline at the exact irreversible boundary.
            if Instant::now() >= self.deadline {
                return Err("Windows launch preparation expired".into());
            }
            if unsafe { ResumeThread(thread) } == u32::MAX {
                return Err(last_error());
            }
            self.committed = true;
            Ok(())
        })?;
        self.thread.take();
        Ok(CommittedLaunch {
            capture: self.capture.take(),
            deadline: self.deadline,
            process: self.process.take().expect("process"),
            response: self.response.take().expect("response"),
            response_ready: self.response_ready.take().expect("response ready"),
            ack: self.ack.take().expect("ack"),
            nonce: self.nonce,
        })
    }
}
impl Drop for StagedLaunch {
    fn drop(&mut self) {
        if !self.committed
            && let Some(process) = &self.process
        {
            unsafe {
                let _ = TerminateProcess(process.raw(), 71);
            }
        }
    }
}

impl CommittedLaunch {
    pub(crate) fn finish(mut self, caller_deadline: Instant) -> LaunchResult {
        let unavailable = || LaunchResult {
            process_spawn_confirmed: false,
            process_id: None,
            process: None,
        };
        let deadline = caller_deadline.min(self.deadline);
        let Some(wait) = wait_ms(deadline) else {
            return unavailable();
        };
        let result = unsafe {
            WaitForMultipleObjects(
                &[self.response_ready.raw(), self.process.raw()],
                false,
                wait,
            )
        };
        if result != WAIT_OBJECT_0 {
            return unavailable();
        }
        let response = match read_record::<Response>(self.response.raw()) {
            Ok(v) => v,
            Err(_) => return unavailable(),
        };
        // Authenticate the envelope before acknowledging it or interpreting
        // its handle-sized field. Without an ack the broker closes any handle
        // it duplicated into this process after its bounded wait.
        if !authenticated_response(&response, self.nonce) {
            return unavailable();
        }
        // Parsing and validation are bounded local work, but still precede the
        // ownership-transfer acknowledgement. Never acknowledge after the
        // caller's original deadline.
        if Instant::now() >= deadline || unsafe { SetEvent(self.ack.raw()) }.is_err() {
            return unavailable();
        }
        // SetEvent is the ownership-transfer boundary. Before it succeeds the
        // broker may close this value in our handle table; afterwards this
        // process must place it under local RAII before doing any further work.
        let transferred_process = if response.process == 0 {
            None
        } else {
            match unsafe { Handle::new(HANDLE(response.process as usize as *mut _)) } {
                Ok(process) => Some(process),
                Err(_) => return unavailable(),
            }
        };
        if let Some(wait) = wait_ms(deadline.min(Instant::now() + EXIT_GRACE)) {
            let _ = unsafe { WaitForSingleObject(self.process.raw(), wait) };
        }
        if response.status != 0 {
            return unavailable();
        }
        let Some(process) = transferred_process else {
            return unavailable();
        };
        let identity =
            nickel_platform::process_identity::WindowsProcessIdentity::from_retained_process(
                &process.0,
            )
            .ok()
            .filter(|identity| {
                self.capture
                    .as_ref()
                    .is_some_and(|capture| capture.admits_placement_root(identity))
            })
            .map(std::sync::Arc::new);
        if let Some(capture) = self.capture.take() {
            capture.complete(process.owned());
        }
        LaunchResult {
            process_spawn_confirmed: true,
            process_id: (response.pid != 0).then_some(response.pid),
            process: identity,
        }
    }
}

impl Drop for CommittedLaunch {
    fn drop(&mut self) {
        // A synchronous ShellExecuteEx call in the broker can otherwise outlive
        // the request indefinitely. Termination is harmless after normal exit
        // and closes all child-owned protocol and pinned-file handles.
        unsafe {
            let _ = TerminateProcess(self.process.raw(), 72);
        }
    }
}

pub(crate) fn run_broker_child() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(2).collect();
    if args.len() != 7 {
        return Err("invalid Windows launch broker invocation".into());
    }
    let parse = |i: usize| {
        args[i]
            .to_str()
            .and_then(|v| v.parse::<usize>().ok())
            .ok_or_else(|| "invalid Windows launch broker invocation".to_owned())
    };
    let expected = parse(0)? as u64;
    let deadline_tick = args[1]
        .to_str()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| "invalid Windows launch broker invocation".to_owned())?;
    // Every inherited protocol handle is owned immediately after parsing.
    let request = unsafe { Handle::new(HANDLE(parse(2)? as *mut _))? };
    let response = unsafe { Handle::new(HANDLE(parse(3)? as *mut _))? };
    let response_ready = unsafe { Handle::new(HANDLE(parse(4)? as *mut _))? };
    let ack = unsafe { Handle::new(HANDLE(parse(5)? as *mut _))? };
    let parent = unsafe { Handle::new(HANDLE(parse(6)? as *mut _))? };
    let record = read_record::<Request>(request.raw())?;
    if record.magic != MAGIC
        || record.version != VERSION
        || record.nonce != expected
        || record.reserved != 0
        || record.count == 0
        || record.count as usize > MAX_PINS
    {
        return Err("invalid Windows launch broker authorization".into());
    }
    let mut pins = Vec::with_capacity(record.count as usize);
    for raw in record.pins.iter().take(record.count as usize) {
        pins.push(unsafe { Handle::new(HANDLE(*raw as usize as *mut _))? });
    }
    let mut target = vec![0u16; 32_768];
    let length =
        unsafe { GetFinalPathNameByHandleW(pins[0].raw(), &mut target, VOLUME_NAME_GUID) } as usize;
    if length == 0 || length >= target.len() {
        return Err("pinned launch target unavailable".into());
    }
    target.truncate(length);
    target.push(0);
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(error)?;
    }
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() }
        }
    }
    let _com = Com;
    let mut shell = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_NOCLOSEPROCESS,
        lpFile: PCWSTR(target.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..unsafe { zeroed() }
    };
    let mut result = Response {
        magic: MAGIC,
        version: VERSION,
        nonce: expected,
        ..Default::default()
    };
    let mut remote_process = None;
    match unsafe { ShellExecuteExW(&mut shell) } {
        Ok(()) if !shell.hProcess.is_invalid() => {
            let launched = unsafe { Handle::new(shell.hProcess)? };
            result.pid = unsafe { GetProcessId(launched.raw()) };
            match RemoteHandle::duplicate(launched.raw(), &parent) {
                Ok(remote) => {
                    result.process = remote.raw().0 as usize as u64;
                    remote_process = Some(remote);
                }
                Err(_) => result.status = 2,
            }
        }
        _ => result.status = 1,
    }
    publish_response(response.raw(), response_ready.raw(), &result)?;
    let wait = unsafe {
        WaitForMultipleObjects(
            &[ack.raw(), parent.raw()],
            false,
            tick_wait_ms(deadline_tick),
        )
    };
    if wait == WAIT_OBJECT_0
        && let Some(remote) = remote_process
    {
        // The parent established local RAII ownership after signaling ack.
        // Relinquish this guard without closing the parent-table value.
        let _ = remote.into_raw();
    }
    Ok(())
}

fn pipe(size: u32) -> Result<(Handle, Handle), String> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: true.into(),
    };
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    unsafe {
        CreatePipe(&mut read, &mut write, Some(&sa), size).map_err(error)?;
    }
    let (read, write) = unsafe { (Handle::from_api(read), Handle::from_api(write)) };
    Ok((read, write))
}
fn event() -> Result<Handle, String> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: true.into(),
    };
    let raw = unsafe { CreateEventW(Some(&sa), true, false, None).map_err(error)? };
    Ok(unsafe { Handle::from_api(raw) })
}
fn duplicate(raw: HANDLE) -> Result<Handle, String> {
    let mut out = HANDLE::default();
    unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            raw,
            GetCurrentProcess(),
            &mut out,
            0,
            true,
            DUPLICATE_SAME_ACCESS,
        )
        .map_err(error)?;
        Ok(Handle::from_api(out))
    }
}
fn close_remote(parent: HANDLE, remote: HANDLE) {
    let mut local = HANDLE::default();
    unsafe {
        let _ = DuplicateHandle(
            parent,
            remote,
            GetCurrentProcess(),
            &mut local,
            0,
            false,
            DUPLICATE_CLOSE_SOURCE,
        );
        if let Ok(local) = Handle::new(local) {
            drop(local);
        }
    }
}
fn read_record<T: Copy>(handle: HANDLE) -> Result<T, String> {
    let mut value: T = unsafe { zeroed() };
    let bytes =
        unsafe { std::slice::from_raw_parts_mut((&mut value as *mut T).cast(), size_of::<T>()) };
    let mut offset = 0;
    while offset < bytes.len() {
        let mut count = 0;
        let read = unsafe { ReadFile(handle, Some(&mut bytes[offset..]), Some(&mut count), None) };
        if let Err(value) = read {
            return if offset == 0 {
                Err(error(value))
            } else {
                Err("truncated Windows launch broker record".into())
            };
        }
        if count == 0 {
            return Err("truncated Windows launch broker record".into());
        }
        offset += count as usize;
    }
    Ok(value)
}
fn write_record<T>(handle: HANDLE, value: &T) -> Result<(), String> {
    let bytes = unsafe { std::slice::from_raw_parts((value as *const T).cast(), size_of::<T>()) };
    let mut offset = 0;
    while offset < bytes.len() {
        let mut count = 0;
        unsafe {
            WriteFile(handle, Some(&bytes[offset..]), Some(&mut count), None).map_err(error)?;
        }
        if count == 0 {
            return Err("truncated Windows launch broker record".into());
        }
        offset += count as usize;
    }
    Ok(())
}
fn publish_response(response: HANDLE, ready: HANDLE, value: &Response) -> Result<(), String> {
    write_record(response, value)?;
    unsafe { SetEvent(ready).map_err(error) }
}
fn wait_ms(deadline: Instant) -> Option<u32> {
    Some(
        deadline
            .checked_duration_since(Instant::now())?
            .as_millis()
            .min(u128::from(u32::MAX)) as u32,
    )
}
fn tick_deadline(deadline: Instant) -> u64 {
    // Sample Windows uptime first so conversion can only shorten the original
    // Instant deadline while local work below proceeds; it must never add the
    // conversion time back to the child's budget.
    let now_tick = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .unwrap_or_default();
    let milliseconds = remaining
        .as_millis()
        .saturating_add(u128::from(
            !remaining.subsec_nanos().is_multiple_of(1_000_000),
        ))
        .min(u128::from(u64::MAX)) as u64;
    now_tick.saturating_add(milliseconds)
}
fn tick_wait_ms(deadline: u64) -> u32 {
    deadline
        .saturating_sub(unsafe { windows::Win32::System::SystemInformation::GetTickCount64() })
        .min(u64::from(u32::MAX)) as u32
}
fn authenticated_response(response: &Response, nonce: u64) -> bool {
    response.magic == MAGIC && response.version == VERSION && response.nonce == nonce
}
fn nonce() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let now =
        unsafe { windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime() };
    ((u64::from(now.dwHighDateTime) << 32) | u64::from(now.dwLowDateTime))
        ^ NEXT.fetch_add(1, Ordering::Relaxed)
        ^ u64::from(std::process::id())
}
fn last_error() -> String {
    error(windows::core::Error::from_thread())
}
fn error(value: windows::core::Error) -> String {
    format!("Windows launch broker failed: {value}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_is_fixed_and_bounded() {
        assert_eq!(size_of::<Request>(), 2_080);
        assert_eq!(size_of::<Response>(), 32);
        assert_eq!(COMMIT_TTL, Duration::from_secs(2));
        assert_eq!(MAX_PINS, 257);
    }

    #[test]
    fn response_handle_is_hidden_until_envelope_authenticates() {
        let response = Response {
            magic: MAGIC,
            version: VERSION,
            nonce: 9,
            status: 0,
            pid: 4,
            process: 0x1234,
        };
        assert!(authenticated_response(&response, 9));
        assert!(!authenticated_response(&response, 10));
        assert!(!authenticated_response(
            &Response {
                magic: 0,
                ..response
            },
            9
        ));
    }

    #[test]
    fn response_ready_is_signaled_only_after_record_publication() {
        let (read, write) = pipe(size_of::<Response>() as u32).unwrap();
        let ready = event().unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(ready.raw(), 0) },
            windows::Win32::Foundation::WAIT_TIMEOUT
        );
        let response = Response {
            magic: MAGIC,
            version: VERSION,
            nonce: 17,
            status: 0,
            pid: 19,
            process: 23,
        };
        publish_response(write.raw(), ready.raw(), &response).unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(ready.raw(), 0) },
            WAIT_OBJECT_0
        );
        let observed = read_record::<Response>(read.raw()).unwrap();
        assert!(authenticated_response(&observed, 17));
        assert_eq!(observed.pid, 19);
        assert_eq!(observed.process, 23);
    }

    #[test]
    fn remote_handle_explicit_transfer_preserves_receiver_handle() {
        let source = event().unwrap();
        let parent = duplicate(unsafe { GetCurrentProcess() }).unwrap();
        let remote = RemoteHandle::duplicate(source.raw(), &parent).unwrap();
        let raw = remote.into_raw();
        assert_eq!(
            unsafe { WaitForSingleObject(raw, 0) },
            windows::Win32::Foundation::WAIT_TIMEOUT
        );
        drop(unsafe { Handle::new(raw) }.unwrap());
    }

    #[test]
    fn partial_response_is_rejected() {
        let (read, write) = pipe(size_of::<Response>() as u32).unwrap();
        let byte = [0u8; 1];
        let mut written = 0;
        unsafe {
            WriteFile(write.raw(), Some(&byte), Some(&mut written), None).unwrap();
        }
        assert_eq!(written, 1);
        drop(write);
        assert_eq!(
            read_record::<Response>(read.raw()).unwrap_err(),
            "truncated Windows launch broker record"
        );
    }

    #[test]
    fn inherited_deadline_never_regains_time() {
        let expired = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
            .saturating_sub(1);
        assert_eq!(tick_wait_ms(expired), 0);
        let soon = tick_deadline(Instant::now() + Duration::from_millis(25));
        assert!(tick_wait_ms(soon) <= 26);
    }
}
