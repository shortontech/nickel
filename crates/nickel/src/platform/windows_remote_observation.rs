//! Read-only Windows discovery plus owner-side revalidation. WinEvent callbacks
//! are delivered out of context; native timing/handle-reuse acceptance is still
//! required before enabling control leases. No HWND is claimed to be a kernel pin.
use super::ordinary_window_metadata;
use crate::windows_resource_owner::{self as policy, Output, Rect, Window};
use nickel_platform::process_identity::WindowsProcessIdentity;
use nickel_remote_control::DesktopPermit;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};
use windows::Win32::Foundation::LPARAM as MessageLparam;
use windows::core::BOOL;
use windows::{
    Win32::{
        Foundation::{HANDLE, HWND, LPARAM, RECT, WPARAM},
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC, GetMonitorInfoW,
            HDC, HGDIOBJ, HMONITOR, MONITORINFO, MONITORINFOEXW, ReleaseDC, SRCCOPY, SelectObject,
        },
        System::{
            RemoteDesktop::{
                WTS_SESSIONSTATE_UNLOCK, WTSActive, WTSFreeMemory, WTSINFOEXW,
                WTSQuerySessionInformationW, WTSSessionInfoEx,
            },
            StationsAndDesktops::{
                CloseDesktop, DESKTOP_READOBJECTS, GetThreadDesktop, GetUserObjectInformationW,
                HDESK, OpenInputDesktop, UOI_NAME,
            },
            Threading::GetCurrentThreadId,
        },
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            HiDpi::{
                DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
                GetDpiForMonitor, MDT_EFFECTIVE_DPI, SetThreadDpiAwarenessContext,
            },
            WindowsAndMessaging::{
                BringWindowToTop, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EnumWindows, GA_ROOT,
                GetAncestor, GetClientRect, GetForegroundWindow, GetWindowRect,
                GetWindowThreadProcessId, HWND_TOP, IsIconic, IsWindow, IsWindowVisible, IsZoomed,
                MONITORINFOF_PRIMARY, OBJID_WINDOW, PostMessageW, SW_MAXIMIZE, SW_MINIMIZE,
                SW_RESTORE, SWP_NOACTIVATE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos,
                ShowWindow, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_CLOSE,
            },
        },
    },
    core::PWSTR,
};

const MAX_WINDOWS: usize = nickel_remote_control::diagnostics::MAX_DIAGNOSTIC_WINDOWS;
const MAX_OUTPUTS: usize = nickel_remote_control::diagnostics::MAX_DIAGNOSTIC_OUTPUTS;
const MAX_AGE: Duration = Duration::from_millis(250);
fn unavailable() -> String {
    "Windows desktop evidence is unavailable or changed".into()
}

pub(crate) fn capture_window_client(
    window: &Window,
    session: u32,
) -> Result<image::RgbaImage, String> {
    if !desktop_is_unlocked(session) {
        return Err(unavailable());
    }
    let hwnd = HWND(window.native as *mut std::ffi::c_void);
    let mut bounds = RECT::default();
    // SAFETY: The owner freshly validated this HWND and process incarnation.
    unsafe { GetClientRect(hwnd, &mut bounds) }.map_err(|_| unavailable())?;
    let width = bounds.right - bounds.left;
    let height = bounds.bottom - bounds.top;
    let (_, _, pixels) = policy::capture_dimensions(width, height)?;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // Read the target's client DC, not the composed desktop. Compositor-owned
    // trusted chrome is therefore absent even when it overlaps this rectangle.
    unsafe {
        let source = GetDC(Some(hwnd));
        if source.0.is_null() {
            return Err(unavailable());
        }
        let memory = CreateCompatibleDC(Some(source));
        if memory.0.is_null() {
            ReleaseDC(Some(hwnd), source);
            return Err(unavailable());
        }
        let mut data = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(Some(source), &info, DIB_RGB_COLORS, &mut data, None, 0)
        {
            Ok(bitmap) => bitmap,
            Err(_) => {
                let _ = DeleteDC(memory);
                ReleaseDC(Some(hwnd), source);
                return Err(unavailable());
            }
        };
        let previous = SelectObject(memory, HGDIOBJ(bitmap.0));
        let copied = BitBlt(memory, 0, 0, width, height, Some(source), 0, 0, SRCCOPY);
        let mut rgba = vec![0; pixels * 4];
        if copied.is_ok() && !data.is_null() {
            let bgra = std::slice::from_raw_parts(data.cast::<u8>(), rgba.len());
            for (source, target) in bgra.chunks_exact(4).zip(rgba.chunks_exact_mut(4)) {
                target.copy_from_slice(&[source[2], source[1], source[0], 255]);
            }
        }
        SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        ReleaseDC(Some(hwnd), source);
        copied.map_err(|_| unavailable())?;
        if !desktop_is_unlocked(session) {
            rgba.fill(0);
            return Err(unavailable());
        }
        image::RgbaImage::from_raw(width as u32, height as u32, rgba).ok_or_else(unavailable)
    }
}

struct LifecycleSender {
    sender: SyncSender<usize>,
    overflow: Arc<AtomicBool>,
    serial: Arc<AtomicU64>,
}
static LIFECYCLE: OnceLock<LifecycleSender> = OnceLock::new();
pub(crate) struct Lifecycle {
    hook: HWINEVENTHOOK,
    receiver: Receiver<usize>,
    overflow: Arc<AtomicBool>,
    serial: Arc<AtomicU64>,
}
impl Lifecycle {
    pub(crate) fn install() -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(2048);
        let overflow = Arc::new(AtomicBool::new(false));
        let serial = Arc::new(AtomicU64::new(1));
        LIFECYCLE
            .set(LifecycleSender {
                sender,
                overflow: overflow.clone(),
                serial: serial.clone(),
            })
            .map_err(|_| unavailable())?;
        // SAFETY: Out-of-context callback runs on this owner's existing message
        // loop; no foreign DLL is injected and callback state has static lifetime.
        let hook = unsafe {
            SetWinEventHook(
                EVENT_OBJECT_CREATE,
                EVENT_OBJECT_DESTROY,
                None,
                Some(lifecycle_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if hook.is_invalid() {
            return Err(unavailable());
        }
        Ok(Self {
            hook,
            receiver,
            overflow,
            serial,
        })
    }
    pub(crate) fn drain(&self) -> Result<Vec<usize>, String> {
        if self.overflow.load(Ordering::Acquire) {
            return Err(unavailable());
        }
        Ok(self.receiver.try_iter().take(2048).collect())
    }
    pub(crate) fn serial(&self) -> u64 {
        self.serial.load(Ordering::Acquire)
    }
}
impl Drop for Lifecycle {
    fn drop(&mut self) {
        // SAFETY: This owner owns the installed hook.
        unsafe {
            let _ = UnhookWinEvent(self.hook);
        }
    }
}
unsafe extern "system" fn lifecycle_event(
    _: HWINEVENTHOOK,
    event: u32,
    window: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if window.is_invalid()
        || object != OBJID_WINDOW.0
        || child != 0
        || !matches!(event, EVENT_OBJECT_CREATE | EVENT_OBJECT_DESTROY)
    {
        return;
    }
    if let Some(state) = LIFECYCLE.get()
        && (state
            .serial
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .is_err()
            || state.sender.try_send(window.0 as usize).is_err())
    {
        state.overflow.store(true, Ordering::Release);
    }
}
fn lifecycle_serial() -> Result<u64, String> {
    let state = LIFECYCLE.get().ok_or_else(unavailable)?;
    if state.overflow.load(Ordering::Acquire) {
        Err(unavailable())
    } else {
        Ok(state.serial.load(Ordering::Acquire))
    }
}

struct DpiContext(DPI_AWARENESS_CONTEXT);
impl DpiContext {
    fn enter() -> Result<Self, String> {
        // SAFETY: Changes only this worker/owner thread and is restored by Drop.
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if previous.is_invalid() {
            return Err(unavailable());
        }
        Ok(Self(previous))
    }
}
impl Drop for DpiContext {
    fn drop(&mut self) {
        // SAFETY: Restore the context captured on this thread.
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}
struct Desktop(HDESK);
impl Drop for Desktop {
    fn drop(&mut self) {
        // SAFETY: OpenInputDesktop transferred this desktop handle.
        let _ = unsafe { CloseDesktop(self.0) };
    }
}
fn desktop_name(handle: HANDLE) -> Option<String> {
    let mut name = [0u16; 256];
    let mut needed = 0;
    // SAFETY: Bounded writable UTF-16 buffer, valid borrowed desktop handle.
    unsafe {
        GetUserObjectInformationW(
            handle,
            UOI_NAME,
            Some(name.as_mut_ptr().cast()),
            std::mem::size_of_val(&name) as u32,
            Some(&mut needed),
        )
    }
    .ok()?;
    if needed == 0 || needed as usize > std::mem::size_of_val(&name) {
        return None;
    }
    let end = name.iter().position(|unit| *unit == 0)?;
    String::from_utf16(&name[..end]).ok()
}
pub(crate) fn desktop_is_unlocked(session: u32) -> bool {
    // SAFETY: Read-only access to the current input desktop, inheritance disabled.
    let Ok(input) = (unsafe { OpenInputDesktop(Default::default(), false, DESKTOP_READOBJECTS) })
    else {
        return false;
    };
    let input = Desktop(input);
    // SAFETY: Current thread ID identifies this worker's existing desktop.
    let Ok(current) = (unsafe { GetThreadDesktop(GetCurrentThreadId()) }) else {
        return false;
    };
    if desktop_name(HANDLE(input.0.0)).as_deref() != Some("Default")
        || desktop_name(HANDLE(current.0)).as_deref() != Some("Default")
    {
        return false;
    }
    let mut buffer = PWSTR::null();
    let mut bytes = 0;
    // SAFETY: Outputs are initialized writable storage. Only lock/session flags
    // are inspected; usernames, input times and other returned fields stay private.
    if unsafe {
        WTSQuerySessionInformationW(None, session, WTSSessionInfoEx, &mut buffer, &mut bytes)
    }
    .is_err()
    {
        return false;
    }
    let valid = if !buffer.is_null() && bytes as usize >= std::mem::size_of::<WTSINFOEXW>() {
        // SAFETY: WTS allocated this aligned structure and returned its full size.
        let info = unsafe { &*buffer.0.cast::<WTSINFOEXW>() };
        if info.Level == 1 {
            // SAFETY: Level selects this initialized union member.
            let details = unsafe { info.Data.WTSInfoExLevel1 };
            details.SessionId == session
                && details.SessionState == WTSActive
                && details.SessionFlags == WTS_SESSIONSTATE_UNLOCK as i32
        } else {
            false
        }
    } else {
        false
    };
    // SAFETY: Free the buffer returned by WTS, including unsupported payloads.
    unsafe {
        WTSFreeMemory(buffer.0.cast());
    }
    valid
}
fn rect(rect: RECT) -> Option<Rect> {
    Some(Rect {
        x: rect.left,
        y: rect.top,
        width: u32::try_from(i64::from(rect.right) - i64::from(rect.left)).ok()?,
        height: u32::try_from(i64::from(rect.bottom) - i64::from(rect.top)).ok()?,
    })
    .filter(|rect| rect.valid())
}

struct MonitorCollector {
    outputs: Vec<Output>,
    failed: bool,
}
unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: EnumDisplayMonitors receives the live stack collector below.
    let state = unsafe { &mut *(data.0 as *mut MonitorCollector) };
    if state.outputs.len() >= MAX_OUTPUTS {
        state.failed = true;
        return BOOL(0);
    }
    let mut info = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: Exact monitor structure size and native enumerated monitor.
    if !unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) }.as_bool() {
        state.failed = true;
        return BOOL(0);
    }
    let Some(bounds) = rect(info.monitorInfo.rcMonitor) else {
        state.failed = true;
        return BOOL(0);
    };
    let Some(work_area) = rect(info.monitorInfo.rcWork) else {
        state.failed = true;
        return BOOL(0);
    };
    let Some(end) = info.szDevice.iter().position(|unit| *unit == 0) else {
        state.failed = true;
        return BOOL(0);
    };
    let Ok(name) = String::from_utf16(&info.szDevice[..end]) else {
        state.failed = true;
        return BOOL(0);
    };
    let (mut x, mut y) = (0, 0);
    // SAFETY: Native monitor and writable DPI outputs in per-monitor context.
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) }.is_err()
        || x == 0
        || x != y
    {
        state.failed = true;
        return BOOL(0);
    }
    state.outputs.push(Output {
        native: monitor.0 as usize,
        name,
        bounds,
        work_area,
        scale_120: ((u64::from(x) * 120 + 48) / 96).min(u32::MAX as u64) as u32,
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
    });
    BOOL(1)
}
fn outputs() -> Result<Vec<Output>, String> {
    let mut state = MonitorCollector {
        outputs: Vec::new(),
        failed: false,
    };
    // SAFETY: Synchronous callback borrows this exact stack collector.
    let success = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM((&mut state as *mut MonitorCollector) as isize),
        )
    }
    .as_bool();
    if !success || state.failed || state.outputs.is_empty() {
        return Err(unavailable());
    }
    state.outputs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(state.outputs)
}
struct HandleCollector {
    handles: Vec<usize>,
    overflow: bool,
}
unsafe extern "system" fn collect_handle(window: HWND, data: LPARAM) -> BOOL {
    // SAFETY: EnumWindows borrows the live bounded collector below.
    let state = unsafe { &mut *(data.0 as *mut HandleCollector) };
    if ordinary_window_metadata(window).is_none() {
        return BOOL(1);
    }
    if state.handles.len() >= MAX_WINDOWS {
        state.overflow = true;
        return BOOL(0);
    }
    state.handles.push(window.0 as usize);
    BOOL(1)
}
fn bounded(mut value: String) -> String {
    if value.len() > 512 {
        let mut end = 512;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
    }
    value
}
fn observe_window(native: usize, process: &WindowsProcessIdentity) -> Option<Window> {
    let hwnd = HWND(native as *mut std::ffi::c_void);
    let (pid, title, _) = ordinary_window_metadata(hwnd)?;
    let mut current = 0;
    // SAFETY: Read-only window/process/rectangle queries with initialized outputs.
    let thread = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut current)) };
    if current != pid || !process.matches_live_process(pid) || thread == 0 {
        return None;
    }
    let mut geometry = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut geometry) }.ok()?;
    let bounds = rect(geometry)?;
    let foreground = unsafe { GetForegroundWindow() };
    let root = unsafe { GetAncestor(foreground, GA_ROOT) };
    let minimized = unsafe { IsIconic(hwnd) }.as_bool();
    let maximized = unsafe { IsZoomed(hwnd) }.as_bool();
    Some(Window {
        native,
        pid,
        created: process.created_at(),
        thread,
        title: bounded(title),
        label: bounded(process.executable_name().ok()?),
        bounds,
        active: foreground == hwnd || root == hwnd,
        minimized,
        maximized,
        fullscreen: false,
        protected: false,
        application: process.verified_application().map(str::to_owned),
    })
}

pub(crate) fn request_window_action(
    window: &Window,
    session: u32,
    action: nickel_remote_control::window_actions::WindowAction,
) -> Result<(), String> {
    use nickel_remote_control::window_actions::WindowAction;
    action.validate()?;
    if !desktop_is_unlocked(session) {
        return Err(unavailable());
    }
    let hwnd = HWND(window.native as *mut std::ffi::c_void);
    // The owner has just revalidated the HWND, process incarnation, desktop and
    // permit. These calls request standard top-level window-manager operations;
    // a separate fresh observation determines whether they actually settled.
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return Err(unavailable());
        }
        match action {
            WindowAction::Activate => {
                if window.minimized {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                let _ = BringWindowToTop(hwnd);
                if !SetForegroundWindow(hwnd).as_bool() {
                    return Err("Windows denied foreground activation".into());
                }
            }
            WindowAction::Minimize => {
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
            }
            WindowAction::Maximize => {
                let _ = ShowWindow(hwnd, SW_MAXIMIZE);
            }
            WindowAction::Restore | WindowAction::ExitFullscreen => {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            WindowAction::Close => {
                PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), MessageLparam(0))
                    .map_err(|_| unavailable())?;
            }
            WindowAction::SetBounds {
                x,
                y,
                width,
                height,
            } => {
                SetWindowPos(
                    hwnd,
                    Some(HWND_TOP),
                    x,
                    y,
                    width as i32,
                    height as i32,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
                .map_err(|_| unavailable())?;
            }
            WindowAction::Fullscreen | WindowAction::MoveToWorkspace { .. } => {
                return Err("Windows does not expose this window operation yet".into());
            }
        }
    }
    if !desktop_is_unlocked(session) {
        return Err(unavailable());
    }
    Ok(())
}

static PREPARATIONS: AtomicUsize = AtomicUsize::new(0);
pub(crate) struct Admission;
impl Admission {
    pub(crate) fn acquire() -> Result<Self, String> {
        PREPARATIONS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 2).then_some(count + 1)
            })
            .map_err(|_| "Windows observation workers are busy".to_owned())?;
        Ok(Self)
    }
}
impl Drop for Admission {
    fn drop(&mut self) {
        PREPARATIONS.fetch_sub(1, Ordering::AcqRel);
    }
}
pub(crate) struct Prepared {
    pub started: Instant,
    pub completed: Instant,
    pub serial: u64,
    pub session: u32,
    pub integrity: u32,
    pub windows: Vec<Window>,
    pub outputs: Vec<Output>,
    pub processes: BTreeMap<usize, Arc<WindowsProcessIdentity>>,
}
impl Prepared {
    pub(crate) fn prepare(permit: &DesktopPermit) -> Result<Self, String> {
        permit.check_live()?;
        let _dpi = DpiContext::enter()?;
        let started = Instant::now();
        let serial = lifecycle_serial()?;
        let own = WindowsProcessIdentity::probe(std::process::id()).map_err(|_| unavailable())?;
        let session = own.session_id();
        let integrity = own.integrity_level().map_err(|_| unavailable())?;
        if !desktop_is_unlocked(session) {
            return Err(unavailable());
        }
        let outputs = outputs()?;
        let mut handles = HandleCollector {
            handles: Vec::new(),
            overflow: false,
        };
        // SAFETY: Synchronous callback references this live collector.
        unsafe {
            EnumWindows(
                Some(collect_handle),
                LPARAM((&mut handles as *mut HandleCollector) as isize),
            )
        }
        .map_err(|_| unavailable())?;
        if handles.overflow {
            return Err(unavailable());
        }
        let mut by_pid = BTreeMap::<u32, Arc<WindowsProcessIdentity>>::new();
        let mut processes = BTreeMap::new();
        let mut windows = Vec::new();
        for native in handles.handles {
            permit.check_live()?;
            let hwnd = HWND(native as *mut std::ffi::c_void);
            let mut pid = 0;
            // SAFETY: This is a revalidated window observation, not PID authority.
            unsafe {
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
            }
            if pid == 0 || pid == std::process::id() {
                continue;
            }
            if let std::collections::btree_map::Entry::Vacant(entry) = by_pid.entry(pid) {
                let Ok(process) = WindowsProcessIdentity::probe(pid) else {
                    continue;
                };
                entry.insert(Arc::new(process));
            }
            let process = by_pid[&pid].clone();
            if !process.is_unprotected_in_session(session)
                || process
                    .integrity_level()
                    .map_or(true, |level| level > integrity)
                || policy::protected_executable(process.executable_name().ok().as_deref())
            {
                continue;
            }
            let Some(mut window) = observe_window(native, &process) else {
                continue;
            };
            window.fullscreen = !window.minimized
                && !window.maximized
                && outputs.iter().any(|output| output.bounds == window.bounds);
            windows.push(window);
            processes.insert(native, process);
        }
        if serial != lifecycle_serial()? || !desktop_is_unlocked(session) {
            return Err(unavailable());
        }
        permit.check_live()?;
        let completed = Instant::now();
        if completed.duration_since(started) >= Duration::from_secs(2) {
            return Err(unavailable());
        }
        Ok(Self {
            started,
            completed,
            serial,
            session,
            integrity,
            windows,
            outputs,
            processes,
        })
    }
    pub(crate) fn revalidate(&self) -> Result<(), String> {
        let _dpi = DpiContext::enter()?;
        if self
            .completed
            .checked_duration_since(self.started)
            .is_none()
            || Instant::now()
                .checked_duration_since(self.completed)
                .is_none_or(|age| age >= MAX_AGE)
            || lifecycle_serial()? != self.serial
            || !desktop_is_unlocked(self.session)
            || outputs()? != self.outputs
        {
            return Err(unavailable());
        }
        for window in &self.windows {
            let process = self.processes.get(&window.native).ok_or_else(unavailable)?;
            if !process.is_unprotected_in_session(self.session)
                || process
                    .integrity_level()
                    .map_or(true, |level| level > self.integrity)
            {
                return Err(unavailable());
            }
            let hwnd = HWND(window.native as *mut std::ffi::c_void);
            let mut pid = 0;
            let mut geometry = RECT::default();
            // SAFETY: Fresh read-only native identity/geometry checks. Raw HWNDs
            // remain subject to native lifecycle delivery limitations documented above.
            let valid = unsafe {
                IsWindow(Some(hwnd)).as_bool()
                    && IsWindowVisible(hwnd).as_bool()
                    && GetWindowThreadProcessId(hwnd, Some(&mut pid)) == window.thread
                    && GetWindowRect(hwnd, &mut geometry).is_ok()
            };
            if !valid
                || !process.matches_live_process(pid)
                || pid != window.pid
                || process.created_at() != window.created
                || rect(geometry) != Some(window.bounds)
                || ordinary_window_metadata(hwnd).is_none()
            {
                return Err(unavailable());
            }
        }
        // Revalidation itself takes native calls: bound its completion as well
        // as its start, and reject lifecycle/desktop changes during that work.
        if lifecycle_serial()? != self.serial
            || !desktop_is_unlocked(self.session)
            || Instant::now()
                .checked_duration_since(self.completed)
                .is_none_or(|age| age >= MAX_AGE)
        {
            return Err(unavailable());
        }
        Ok(())
    }
}
