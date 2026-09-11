//! Read-only Windows discovery plus owner-side revalidation. WinEvent callbacks
//! are delivered out of context; native timing/handle-reuse acceptance is still
//! required before enabling control leases. No HWND is claimed to be a kernel pin.
use super::ordinary_window_metadata;
use crate::windows_resource_owner::{self as policy, Output, Rect, Window};
use nickel_platform::process_identity::WindowsProcessIdentity;
use nickel_remote_control::DesktopPermit;
use std::{
    collections::{BTreeMap, BTreeSet},
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
        Foundation::{CloseHandle, HANDLE, HWND, LPARAM, POINT, RECT, WPARAM},
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC, GetMonitorInfoW,
            HDC, HGDIOBJ, HMONITOR, MONITORINFO, MONITORINFOEXW, ReleaseDC, SRCCOPY,
            ScreenToClient, SelectObject,
        },
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
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
                BringWindowToTop, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE,
                EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_REORDER, EVENT_OBJECT_SHOW, EnumWindows,
                GA_ROOT, GUITHREADINFO, GetAncestor, GetClientRect, GetCursorPos,
                GetForegroundWindow, GetGUIThreadInfo, GetWindowRect, GetWindowThreadProcessId,
                HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLOSE, HTLEFT, HTMAXBUTTON,
                HTMINBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, HWND_TOP, IsIconic, IsWindow,
                IsWindowVisible, IsZoomed, MONITORINFOF_PRIMARY, OBJID_WINDOW, PostMessageW,
                SEND_MESSAGE_TIMEOUT_FLAGS, SMTO_ABORTIFHUNG, SMTO_BLOCK, SW_MAXIMIZE, SW_MINIMIZE,
                SW_RESTORE, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageTimeoutW,
                SetForegroundWindow, SetWindowPos, ShowWindow, WINEVENT_OUTOFCONTEXT,
                WINEVENT_SKIPOWNPROCESS, WM_CLOSE, WM_NCHITTEST, WindowFromPoint,
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

pub(crate) fn capture_output_pixels(
    output: &Output,
    prepared: &Prepared,
    mut check: impl FnMut() -> Result<(), String>,
) -> Result<image::RgbaImage, String> {
    check()?;
    prepared.revalidate()?;
    if prepared.output_capture_has_unlocated_protected_external
        || prepared
            .output_capture_protected_external
            .iter()
            .any(|bounds| bounds.intersects(output.bounds))
        || prepared
            .outputs
            .iter()
            .filter(|candidate| *candidate == output)
            .count()
            != 1
    {
        return Err("Windows output contains protected or changed content".into());
    }
    let width = i32::try_from(output.bounds.width)
        .map_err(|_| "Windows capture dimensions exceed limits")?;
    let height = i32::try_from(output.bounds.height)
        .map_err(|_| "Windows capture dimensions exceed limits")?;
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
    let _dpi = DpiContext::enter()?;
    // Read the composed physical output. Production shell policy has already
    // rejected protected surfaces and verified WDA_EXCLUDEFROMCAPTURE on the
    // trusted indicator retained by its winit owner.
    let rgba = unsafe {
        let source = GetDC(None);
        if source.0.is_null() {
            return Err(unavailable());
        }
        let memory = CreateCompatibleDC(Some(source));
        if memory.0.is_null() {
            ReleaseDC(None, source);
            return Err(unavailable());
        }
        let mut data = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(Some(source), &info, DIB_RGB_COLORS, &mut data, None, 0)
        {
            Ok(bitmap) if !data.is_null() => bitmap,
            Ok(bitmap) => {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                let _ = DeleteDC(memory);
                ReleaseDC(None, source);
                return Err(unavailable());
            }
            Err(_) => {
                let _ = DeleteDC(memory);
                ReleaseDC(None, source);
                return Err(unavailable());
            }
        };
        let previous = SelectObject(memory, HGDIOBJ(bitmap.0));
        let copied = BitBlt(
            memory,
            0,
            0,
            width,
            height,
            Some(source),
            output.bounds.x,
            output.bounds.y,
            SRCCOPY,
        );
        let result: Result<Vec<u8>, String> = (|| {
            copied.map_err(|_| unavailable())?;
            check()?;
            let bytes = pixels.checked_mul(4).ok_or_else(unavailable)?;
            let bgra = std::slice::from_raw_parts(data.cast::<u8>(), bytes);
            let mut rgba = vec![0; bytes];
            for (source_row, target_row) in bgra
                .chunks_exact(usize::try_from(width).map_err(|_| unavailable())? * 4)
                .zip(rgba.chunks_exact_mut(usize::try_from(width).map_err(|_| unavailable())? * 4))
            {
                check()?;
                for (source, target) in source_row
                    .chunks_exact(4)
                    .zip(target_row.chunks_exact_mut(4))
                {
                    target.copy_from_slice(&[source[2], source[1], source[0], 255]);
                }
            }
            Ok(rgba)
        })();
        SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        ReleaseDC(None, source);
        result?
    };
    check()?;
    prepared.revalidate()?;
    image::RgbaImage::from_raw(output.bounds.width, output.bounds.height, rgba)
        .ok_or_else(unavailable)
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
                EVENT_OBJECT_LOCATIONCHANGE,
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
        || !matches!(
            event,
            EVENT_OBJECT_CREATE
                | EVENT_OBJECT_DESTROY
                | EVENT_OBJECT_SHOW
                | EVENT_OBJECT_HIDE
                | EVENT_OBJECT_REORDER
                | EVENT_OBJECT_LOCATIONCHANGE
        )
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
            || (matches!(event, EVENT_OBJECT_CREATE | EVENT_OBJECT_DESTROY)
                && state.sender.try_send(window.0 as usize).is_err()))
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
    let mut pid = 0;
    // Capture preparation must account for every visible external top-level
    // window, including popups that are not ordinary task-switcher resources.
    // An unattributed window blocks composed readback instead of leaking pixels.
    if !unsafe { IsWindowVisible(window) }.as_bool()
        || unsafe { GetAncestor(window, GA_ROOT) } != window
        || unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) } == 0
        || pid == 0
        || pid == std::process::id()
    {
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

fn observed_window_bounds(native: usize) -> Result<Option<Rect>, ()> {
    let mut geometry = RECT::default();
    // SAFETY: This bounded read is for a freshly enumerated top-level HWND.
    unsafe { GetWindowRect(HWND(native as *mut std::ffi::c_void), &mut geometry) }
        .map_err(|_| ())?;
    if geometry.right <= geometry.left || geometry.bottom <= geometry.top {
        return Ok(None);
    }
    rect(geometry).map(Some).ok_or(())
}

fn retain_protected_capture_bounds(
    native: usize,
    bounds: &mut Vec<Rect>,
    has_unlocated: &mut bool,
) {
    match observed_window_bounds(native) {
        Ok(Some(window_bounds)) => bounds.push(window_bounds),
        Ok(None) => {}
        Err(()) => *has_unlocated = true,
    }
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

const MAX_PROCESS_ANCESTRY: usize = 8_192;
const MAX_ANCESTRY_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProcessLink {
    parent: u32,
    created: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LaunchProcessAncestry {
    Descendant,
    Unrelated,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LaunchProcessRoot {
    pub pid: u32,
    pub created: u64,
}

fn bounded_process_parents() -> Result<BTreeMap<u32, u32>, String> {
    // SAFETY: The returned snapshot handle is closed on every path below and
    // PROCESSENTRY32W advertises its exact initialized size to Toolhelp.
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.map_err(|_| unavailable())?;
    let mut parents = BTreeMap::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut result = unsafe { Process32FirstW(snapshot, &raw mut entry) };
    while result.is_ok() {
        if parents.len() == MAX_PROCESS_ANCESTRY
            || parents
                .insert(entry.th32ProcessID, entry.th32ParentProcessID)
                .is_some()
        {
            // SAFETY: snapshot is the live owned handle returned above.
            let _ = unsafe { CloseHandle(snapshot) };
            return Err(unavailable());
        }
        result = unsafe { Process32NextW(snapshot, &raw mut entry) };
    }
    // SAFETY: snapshot is the live owned handle returned above.
    let _ = unsafe { CloseHandle(snapshot) };
    if parents.is_empty() {
        Err(unavailable())
    } else {
        Ok(parents)
    }
}

fn prepared_process_links(
    windows: &[Window],
    processes: &BTreeMap<usize, Arc<WindowsProcessIdentity>>,
    parents: &BTreeMap<u32, u32>,
    check: &mut impl FnMut() -> Result<(), String>,
) -> Result<BTreeMap<u32, ProcessLink>, String> {
    let mut identities = BTreeMap::<u32, Arc<WindowsProcessIdentity>>::new();
    let mut ambiguous = BTreeSet::new();
    for process in processes.values() {
        let pid = process.process_id();
        if identities
            .get(&pid)
            .is_some_and(|current| current.created_at() != process.created_at())
        {
            identities.remove(&pid);
            ambiguous.insert(pid);
        } else if !ambiguous.contains(&pid) {
            identities.insert(pid, process.clone());
        }
    }

    let mut probes = 0usize;
    let mut links = BTreeMap::new();
    for window in windows {
        check()?;
        let mut current = window.pid;
        let mut visited = BTreeSet::new();
        for _ in 0..MAX_ANCESTRY_DEPTH {
            check()?;
            if !visited.insert(current) || ambiguous.contains(&current) {
                break;
            }
            if !identities.contains_key(&current) {
                if probes == MAX_PROCESS_ANCESTRY {
                    break;
                }
                probes += 1;
                let Ok(identity) = WindowsProcessIdentity::probe(current) else {
                    ambiguous.insert(current);
                    break;
                };
                let identity = Arc::new(identity);
                if identities
                    .get(&current)
                    .is_some_and(|known| known.created_at() != identity.created_at())
                {
                    identities.remove(&current);
                    ambiguous.insert(current);
                    break;
                }
                identities.insert(current, identity);
            }
            let Some(identity) = identities.get(&current) else {
                break;
            };
            let Some(parent) = parents.get(&current).copied() else {
                break;
            };
            links.insert(
                current,
                ProcessLink {
                    parent,
                    created: identity.created_at(),
                },
            );
            if parent == 0 || parent == current {
                break;
            }
            current = parent;
        }
    }
    Ok(links)
}

fn process_chain_ancestry(
    mut candidate: u32,
    mut child_created: u64,
    root: LaunchProcessRoot,
    links: &BTreeMap<u32, ProcessLink>,
) -> LaunchProcessAncestry {
    if child_created < root.created {
        return LaunchProcessAncestry::Unrelated;
    }
    for _ in 0..MAX_ANCESTRY_DEPTH {
        if candidate == root.pid {
            return if child_created == root.created {
                LaunchProcessAncestry::Descendant
            } else {
                LaunchProcessAncestry::Unrelated
            };
        }
        let Some(link) = links.get(&candidate) else {
            return LaunchProcessAncestry::Unknown;
        };
        if link.created != child_created {
            return LaunchProcessAncestry::Unknown;
        }
        if link.parent == 0 {
            return LaunchProcessAncestry::Unrelated;
        }
        if link.parent == root.pid {
            return if root.created <= child_created {
                LaunchProcessAncestry::Descendant
            } else {
                LaunchProcessAncestry::Unknown
            };
        }
        let Some(parent) = links.get(&link.parent) else {
            return LaunchProcessAncestry::Unknown;
        };
        if link.parent == candidate || parent.created > child_created {
            return LaunchProcessAncestry::Unknown;
        }
        if parent.created < root.created {
            return LaunchProcessAncestry::Unrelated;
        }
        candidate = link.parent;
        child_created = parent.created;
    }
    LaunchProcessAncestry::Unknown
}

fn process_chain_matches(
    candidate: u32,
    created: u64,
    root: LaunchProcessRoot,
    links: &BTreeMap<u32, ProcessLink>,
) -> bool {
    process_chain_ancestry(candidate, created, root, links) == LaunchProcessAncestry::Descendant
}

fn fit_on_output(window: Rect, output: Rect) -> Option<Rect> {
    if !window.valid() || !output.valid() {
        return None;
    }
    Some(Rect {
        x: output.x,
        y: output.y,
        width: window.width.min(output.width),
        height: window.height.min(output.height),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LaunchPlacementResult {
    Waiting,
    Requested,
    Confirmed,
}

/// Find the one first ordinary window that can be tied to the exact retained
/// launch process, then request placement on the attested output. A successful
/// SetWindowPos return is only a request; a later fresh snapshot must observe
/// containment before the window can enter the ordinary resource registry.
pub(crate) fn place_first_launch_window(
    prepared: &Prepared,
    root: &WindowsProcessIdentity,
    target: &Output,
) -> Result<LaunchPlacementResult, String> {
    prepared.revalidate()?;
    if !root.is_unprotected_in_session(prepared.session)
        || root
            .integrity_level()
            .map_or(true, |level| level > prepared.integrity)
        || policy::protected_executable(root.executable_name().ok().as_deref())
        || prepared
            .outputs
            .iter()
            .filter(|output| *output == target)
            .count()
            != 1
    {
        return Err(unavailable());
    }
    let root = LaunchProcessRoot {
        pid: root.process_id(),
        created: root.created_at(),
    };
    let mut candidates = Vec::new();
    for window in &prepared.windows {
        let process = prepared
            .processes
            .get(&window.native)
            .ok_or_else(unavailable)?;
        if !process.is_unprotected_in_session(prepared.session)
            || process
                .integrity_level()
                .map_or(true, |level| level > prepared.integrity)
            || policy::protected_executable(process.executable_name().ok().as_deref())
        {
            continue;
        }
        if process_chain_matches(
            process.process_id(),
            process.created_at(),
            root,
            &prepared.process_links,
        ) {
            candidates.push(window);
        }
    }
    let window = match candidates.as_slice() {
        [] => return Ok(LaunchPlacementResult::Waiting),
        [window] => *window,
        _ => return Err("Windows launch produced ambiguous first-window attribution".into()),
    };
    if target.bounds.contains(window.bounds) {
        return Ok(LaunchPlacementResult::Confirmed);
    }
    let placement = fit_on_output(window.bounds, target.work_area).ok_or_else(unavailable)?;
    let hwnd = HWND(window.native as *mut std::ffi::c_void);
    let process = prepared
        .processes
        .get(&window.native)
        .ok_or_else(unavailable)?;
    let mut pid = 0;
    let mut bounds = RECT::default();
    // SAFETY: Every input came from the fresh snapshot above. Native identity,
    // geometry and visibility are repeated at the request boundary. Async
    // positioning cannot block the owner on an unresponsive external process.
    let valid = unsafe {
        IsWindow(Some(hwnd)).as_bool()
            && IsWindowVisible(hwnd).as_bool()
            && GetWindowThreadProcessId(hwnd, Some(&mut pid)) == window.thread
            && GetWindowRect(hwnd, &mut bounds).is_ok()
    };
    if !valid
        || pid != window.pid
        || !process.matches_live_process(pid)
        || process.created_at() != window.created
        || rect(bounds) != Some(window.bounds)
        || ordinary_window_metadata(hwnd).is_none()
        || outputs()? != prepared.outputs
        || lifecycle_serial()? != prepared.serial
        || !desktop_is_unlocked(prepared.session)
    {
        return Err(unavailable());
    }
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            placement.x,
            placement.y,
            placement.width as i32,
            placement.height as i32,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        )
    }
    .map_err(|_| unavailable())?;
    Ok(LaunchPlacementResult::Requested)
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
    pub input: NativeInputObservation,
    output_capture_protected_external: Vec<Rect>,
    output_capture_has_unlocated_protected_external: bool,
    process_links: BTreeMap<u32, ProcessLink>,
}

/// Short-lived native input routing evidence. Coordinates remain inside the
/// request-local preparation and are never copied into a diagnostic snapshot.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct NativeInputObservation {
    pub keyboard_root: Option<usize>,
    pub pointer_root: Option<usize>,
    pub pointer_recipient_root: Option<usize>,
    pub pointer_recipient_available: bool,
    pub pointer_grabbed: bool,
    pub pointer_client: Option<[i32; 2]>,
    pub pointer_decoration: Option<nickel_remote_control::diagnostics::InternalDecorationHit>,
}

fn root_handle(window: HWND) -> Option<usize> {
    if window.0.is_null() {
        return None;
    }
    // SAFETY: Read-only ancestry query for a handle returned by User32.
    let root = unsafe { GetAncestor(window, GA_ROOT) };
    (!root.0.is_null()).then_some(root.0 as usize)
}

fn observe_native_input(own_pid: u32) -> NativeInputObservation {
    let foreground = unsafe { GetForegroundWindow() };
    let keyboard_root = root_handle(foreground);
    let mut cursor = POINT::default();
    // SAFETY: User32 writes the current cursor position to initialized storage.
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        return NativeInputObservation {
            keyboard_root,
            ..Default::default()
        };
    }
    // SAFETY: Read-only hit test at the sampled point.
    let pointer = unsafe { WindowFromPoint(cursor) };
    let pointer_root = root_handle(pointer);
    let pointer_client = pointer_root.and_then(|native| {
        let mut client = cursor;
        // SAFETY: The root came from User32 and `client` is writable storage.
        unsafe { ScreenToClient(HWND(native as *mut std::ffi::c_void), &mut client) }
            .as_bool()
            .then_some([client.x, client.y])
    });
    let pointer_decoration = pointer_root.and_then(|native| {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(HWND(native as *mut std::ffi::c_void), Some(&mut pid)) };
        if pid != own_pid {
            return None;
        }
        // WM_NCHITTEST is Nickel's native frame hit test. Bound it on
        // this observation worker so a hung client can neither stall the owner
        // nor create an unbounded worker backlog.
        let packed = i32::from(cursor.x as i16 as u16) | (i32::from(cursor.y as i16 as u16) << 16);
        let mut result = 0usize;
        let completed = unsafe {
            SendMessageTimeoutW(
                HWND(native as *mut std::ffi::c_void),
                WM_NCHITTEST,
                WPARAM(0),
                LPARAM(packed as isize),
                SEND_MESSAGE_TIMEOUT_FLAGS(SMTO_ABORTIFHUNG.0 | SMTO_BLOCK.0),
                25,
                Some(&mut result),
            )
        };
        if completed.0 == 0 {
            return None;
        }
        use nickel_remote_control::diagnostics::InternalDecorationHit;
        Some(match result as u32 {
            HTCAPTION => InternalDecorationHit::Titlebar,
            HTMINBUTTON => InternalDecorationHit::Minimize,
            HTMAXBUTTON => InternalDecorationHit::Maximize,
            HTCLOSE => InternalDecorationHit::Close,
            HTTOP => InternalDecorationHit::ResizeNorth,
            HTTOPRIGHT => InternalDecorationHit::ResizeNorthEast,
            HTRIGHT => InternalDecorationHit::ResizeEast,
            HTBOTTOMRIGHT => InternalDecorationHit::ResizeSouthEast,
            HTBOTTOM => InternalDecorationHit::ResizeSouth,
            HTBOTTOMLEFT => InternalDecorationHit::ResizeSouthWest,
            HTLEFT => InternalDecorationHit::ResizeWest,
            HTTOPLEFT => InternalDecorationHit::ResizeNorthWest,
            _ => return None,
        })
    });
    let mut gui = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    // A missing GUI-thread snapshot makes the recipient unavailable, while the
    // independent hit remains useful. Never guess that hover owns a capture.
    let gui_available = unsafe { GetGUIThreadInfo(0, &mut gui) }.is_ok();
    let pointer_grabbed = gui_available && !gui.hwndCapture.0.is_null();
    let pointer_recipient_root = gui_available
        .then(|| {
            let recipient = if gui.hwndCapture.0.is_null() {
                pointer
            } else {
                gui.hwndCapture
            };
            root_handle(recipient)
        })
        .flatten();
    NativeInputObservation {
        keyboard_root,
        pointer_root,
        pointer_recipient_root,
        pointer_recipient_available: gui_available,
        pointer_grabbed,
        pointer_client,
        pointer_decoration,
    }
}
impl Prepared {
    /// Pure owner-side projection over ancestry evidence acquired by the
    /// observation worker. This performs no native calls or process probes.
    pub(crate) fn classify_launch_window_ancestry(
        &self,
        roots: &[LaunchProcessRoot],
    ) -> BTreeMap<usize, Vec<LaunchProcessAncestry>> {
        self.windows
            .iter()
            .map(|window| {
                (
                    window.native,
                    roots
                        .iter()
                        .map(|root| {
                            process_chain_ancestry(
                                window.pid,
                                window.created,
                                *root,
                                &self.process_links,
                            )
                        })
                        .collect(),
                )
            })
            .collect()
    }

    pub(crate) fn prepare(permit: &DesktopPermit) -> Result<Self, String> {
        Self::prepare_checked(|| permit.check_live(), false)
    }

    /// Add request-local input routing evidence only for the diagnostic
    /// snapshot that publishes it. Ordinary observation and mutation workers
    /// must not pay for or depend on input sampling.
    pub(crate) fn prepare_with_input(permit: &DesktopPermit) -> Result<Self, String> {
        Self::prepare_checked(|| permit.check_live(), true)
    }

    /// Collect the same bounded native evidence for a trusted Settings
    /// decision. This path has no remote capability to authenticate; its
    /// authority is the owner-thread local transport and the live input
    /// desktop. The caller must still compare the requested resource identity
    /// with the reconciled owner inventory before changing lease authority.
    pub(crate) fn prepare_local() -> Result<Self, String> {
        Self::prepare_checked(|| Ok(()), false)
    }

    fn prepare_checked(
        mut check: impl FnMut() -> Result<(), String>,
        include_input: bool,
    ) -> Result<Self, String> {
        check()?;
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
        let input = if include_input {
            observe_native_input(std::process::id())
        } else {
            NativeInputObservation::default()
        };
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
        let mut output_capture_protected_external = Vec::new();
        let mut output_capture_has_unlocated_protected_external = false;
        for native in handles.handles {
            check()?;
            let hwnd = HWND(native as *mut std::ffi::c_void);
            if ordinary_window_metadata(hwnd).is_none() {
                retain_protected_capture_bounds(
                    native,
                    &mut output_capture_protected_external,
                    &mut output_capture_has_unlocated_protected_external,
                );
                continue;
            }
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
                    retain_protected_capture_bounds(
                        native,
                        &mut output_capture_protected_external,
                        &mut output_capture_has_unlocated_protected_external,
                    );
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
                retain_protected_capture_bounds(
                    native,
                    &mut output_capture_protected_external,
                    &mut output_capture_has_unlocated_protected_external,
                );
                continue;
            }
            let Some(mut window) = observe_window(native, &process) else {
                retain_protected_capture_bounds(
                    native,
                    &mut output_capture_protected_external,
                    &mut output_capture_has_unlocated_protected_external,
                );
                continue;
            };
            window.fullscreen = !window.minimized
                && !window.maximized
                && outputs.iter().any(|output| output.bounds == window.bounds);
            windows.push(window);
            processes.insert(native, process);
        }
        let process_links = match bounded_process_parents() {
            Ok(parents) => prepared_process_links(&windows, &processes, &parents, &mut check)?,
            // Parentage is optional evidence. Absence keeps post-baseline
            // windows conservatively unknown without failing ordinary output
            // and window observation.
            Err(_) => Default::default(),
        };
        if serial != lifecycle_serial()? || !desktop_is_unlocked(session) {
            return Err(unavailable());
        }
        check()?;
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
            input,
            output_capture_protected_external,
            output_capture_has_unlocated_protected_external,
            process_links,
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

#[cfg(test)]
mod launch_placement_tests {
    use super::*;

    fn link(parent: u32, created: u64) -> ProcessLink {
        ProcessLink { parent, created }
    }

    fn chain_matches(candidate: u32, root: u32, links: &BTreeMap<u32, ProcessLink>) -> bool {
        process_chain_matches(
            candidate,
            links[&candidate].created,
            LaunchProcessRoot {
                pid: root,
                created: links[&root].created,
            },
            links,
        )
    }

    #[test]
    fn ancestry_requires_the_exact_monotonic_process_chain() {
        let links = BTreeMap::from([(10, link(1, 100)), (11, link(10, 110)), (12, link(11, 120))]);
        assert!(chain_matches(10, 10, &links));
        assert!(chain_matches(12, 10, &links));

        let reused_parent =
            BTreeMap::from([(10, link(1, 100)), (11, link(10, 130)), (12, link(11, 120))]);
        assert!(!chain_matches(12, 10, &reused_parent));
    }

    #[test]
    fn ancestry_rejects_missing_cycles_and_excessive_depth() {
        assert!(!chain_matches(
            12,
            10,
            &BTreeMap::from([(10, link(1, 100)), (12, link(11, 120))])
        ));
        assert!(!chain_matches(
            12,
            10,
            &BTreeMap::from([(10, link(1, 100)), (11, link(12, 110)), (12, link(11, 120)),])
        ));
        let mut deep = BTreeMap::from([(1, link(0, 1))]);
        for pid in 2..=u32::try_from(MAX_ANCESTRY_DEPTH).unwrap() + 2 {
            deep.insert(pid, link(pid - 1, u64::from(pid)));
        }
        assert!(!chain_matches(
            u32::try_from(MAX_ANCESTRY_DEPTH).unwrap() + 2,
            1,
            &deep,
        ));
    }

    #[test]
    fn owner_classifier_reads_only_prepared_ancestry_evidence() {
        let source = include_str!("windows_remote_observation.rs");
        let start = source
            .find("    pub(crate) fn classify_launch_window_ancestry(")
            .unwrap();
        let end = source[start..]
            .find("    pub(crate) fn prepare(")
            .map(|offset| start + offset)
            .unwrap();
        let classifier = &source[start..end];
        for forbidden in [
            "CreateToolhelp32Snapshot",
            "WindowsProcessIdentity::probe",
            ".revalidate(",
            ".is_live(",
            "std::fs",
            "sleep(",
            "recv",
            "wait",
        ] {
            assert!(
                !classifier.contains(forbidden),
                "owner classifier contains blocking/native operation {forbidden}"
            );
        }
        assert!(classifier.contains("&self.process_links"));
    }

    #[test]
    fn placement_contains_oversized_windows_in_the_exact_work_area() {
        let work = Rect {
            x: -1920,
            y: 40,
            width: 1920,
            height: 1040,
        };
        assert_eq!(
            fit_on_output(
                Rect {
                    x: 200,
                    y: 100,
                    width: 3000,
                    height: 2000,
                },
                work,
            ),
            Some(work)
        );
        assert!(work.contains(fit_on_output(work, work).unwrap()));
    }
}
