use std::{
    ffi::c_void,
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HGDIOBJ, ReleaseDC, SelectObject,
        },
        Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES,
        System::Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
        UI::Shell::{
            ABE_BOTTOM, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA, SHAppBarMessage,
            SHFILEINFOW, SHGFI_ICON, SHGetFileInfoW,
        },
        UI::WindowsAndMessaging::{
            CallNextHookEx, DI_NORMAL, DestroyIcon, DrawIconEx, EnumWindows, GA_ROOT, GA_ROOTOWNER,
            GCLP_HICON, GCLP_HICONSM, GWL_EXSTYLE, GetAncestor, GetClassLongPtrW, GetClassNameW,
            GetForegroundWindow, GetLastActivePopup, GetMessageW, GetWindowLongPtrW, GetWindowRect,
            GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, HICON, HWND_BOTTOM,
            HWND_TOPMOST, IsWindow, IsWindowVisible, IsZoomed, KBDLLHOOKSTRUCT, MSG, PostMessageW,
            SPI_GETWORKAREA, SPI_SETWORKAREA, SPIF_SENDCHANGE, SW_MAXIMIZE, SW_MINIMIZE,
            SW_RESTORE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
            SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, SetWindowsHookExW, ShowWindow,
            SystemParametersInfoW, WH_KEYBOARD_LL, WM_CLOSE, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
            WM_SYSKEYUP, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        },
    },
    core::{BOOL, PCWSTR, PWSTR},
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{
    launcher::Launcher,
    model::{Application, ApplicationId, OpenWindow, TrayItem, WindowId, WindowPreview},
    platform::{GlobalShortcut, ShellCommand, TraySource, WindowAction},
};

#[path = "windows_start_menu.rs"]
mod start_menu;

pub fn applications() -> Vec<Application> {
    start_menu::load_applications()
}

pub fn application_icon(reference: &str) -> Option<image::RgbaImage> {
    executable_icon(PathBuf::from(reference).as_path())
}

pub fn launcher_hotkey_receiver() -> Receiver<GlobalShortcut> {
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("nickel-windows-key".into())
        .spawn(move || run_windows_key_hook(sender))
        .expect("failed to start Windows-key listener");
    receiver
}

fn run_windows_key_hook(sender: Sender<GlobalShortcut>) {
    // SAFETY: The callback remains valid for the process lifetime and this thread owns the
    // message loop required by a low-level keyboard hook.
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(windows_key_hook), None, 0) };
    let Ok(_hook) = hook else {
        eprintln!("failed to register the Windows-key launcher hook");
        return;
    };
    WINDOWS_KEY_SENDER.set(sender).ok();
    let mut message = MSG::default();
    // SAFETY: message is valid writable storage for each synchronous call.
    while unsafe { GetMessageW(&mut message, None, 0, 0).as_bool() } {}
}

static WINDOWS_KEY_SENDER: std::sync::OnceLock<Sender<GlobalShortcut>> = std::sync::OnceLock::new();
static WINDOWS_KEY_HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static WINDOWS_KEY_CHORDED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static ALT_HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SHIFT_HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SWITCH_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static TAB_SUPPRESSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PANEL_APPBAR_REGISTERED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static ORIGINAL_WORK_AREA: std::sync::Mutex<Option<RECT>> = std::sync::Mutex::new(None);

unsafe extern "system" fn windows_key_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    use std::sync::atomic::Ordering;

    const VK_LWIN: u32 = 0x5b;
    const VK_RWIN: u32 = 0x5c;
    const VK_MENU: u32 = 0x12;
    const VK_LMENU: u32 = 0xa4;
    const VK_RMENU: u32 = 0xa5;
    const VK_SHIFT: u32 = 0x10;
    const VK_LSHIFT: u32 = 0xa0;
    const VK_RSHIFT: u32 = 0xa1;
    const VK_TAB: u32 = 0x09;

    if code >= 0 {
        // SAFETY: WH_KEYBOARD_LL supplies a KBDLLHOOKSTRUCT pointer in LPARAM.
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let message = wparam.0 as u32;
        let key_down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
        let key_up = message == WM_KEYUP || message == WM_SYSKEYUP;
        let windows_key = event.vkCode == VK_LWIN || event.vkCode == VK_RWIN;
        if matches!(event.vkCode, VK_MENU | VK_LMENU | VK_RMENU) {
            ALT_HELD.store(key_down, Ordering::Relaxed);
            if key_up && SWITCH_ACTIVE.swap(false, Ordering::Relaxed) {
                if let Some(sender) = WINDOWS_KEY_SENDER.get() {
                    let _ = sender.send(GlobalShortcut::CommitSwitch);
                }
            }
        } else if matches!(event.vkCode, VK_SHIFT | VK_LSHIFT | VK_RSHIFT) {
            SHIFT_HELD.store(key_down, Ordering::Relaxed);
        } else if event.vkCode == VK_TAB && ALT_HELD.load(Ordering::Relaxed) {
            if key_down {
                SWITCH_ACTIVE.store(true, Ordering::Relaxed);
                TAB_SUPPRESSED.store(true, Ordering::Relaxed);
                if let Some(sender) = WINDOWS_KEY_SENDER.get() {
                    let shortcut = if SHIFT_HELD.load(Ordering::Relaxed) {
                        GlobalShortcut::SwitchPrevious
                    } else {
                        GlobalShortcut::SwitchNext
                    };
                    let _ = sender.send(shortcut);
                }
            }
            return LRESULT(1);
        } else if event.vkCode == VK_TAB && key_up && TAB_SUPPRESSED.swap(false, Ordering::Relaxed)
        {
            return LRESULT(1);
        }
        if windows_key && key_down {
            WINDOWS_KEY_HELD.store(true, Ordering::Relaxed);
            WINDOWS_KEY_CHORDED.store(false, Ordering::Relaxed);
            // Nickel owns the Windows key completely. Forwarding only the press while consuming
            // the release leaves Windows' modifier state stuck and prevents launcher typing.
            return LRESULT(1);
        } else if WINDOWS_KEY_HELD.load(Ordering::Relaxed) && key_down {
            WINDOWS_KEY_CHORDED.store(true, Ordering::Relaxed);
        } else if windows_key && key_up {
            WINDOWS_KEY_HELD.store(false, Ordering::Relaxed);
            if !WINDOWS_KEY_CHORDED.swap(false, Ordering::Relaxed) {
                if let Some(sender) = WINDOWS_KEY_SENDER.get() {
                    let _ = sender.send(GlobalShortcut::ToggleLauncher);
                }
            }
            return LRESULT(1);
        }
    }
    // SAFETY: Unhandled events must continue through the hook chain.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

pub fn configure_desktop_window(window: &winit::window::Window) -> bool {
    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };
    let hwnd = HWND(handle.hwnd.get() as *mut c_void);
    // SAFETY: hwnd belongs to the live winit desktop window. The style change prevents activation,
    // and SetWindowPos moves only its Z-order without changing its monitor geometry.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            (style | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize,
        );
        SetWindowPos(
            hwnd,
            Some(HWND_BOTTOM),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
        .is_ok()
    }
}

pub fn configure_panel_window(window: &winit::window::Window) -> bool {
    let Some(hwnd) = window_hwnd(window) else {
        return false;
    };
    let mut rectangle = Default::default();
    // SAFETY: hwnd belongs to the live winit panel and rectangle is writable storage.
    if unsafe { GetWindowRect(hwnd, &mut rectangle) }.is_err() {
        return false;
    }
    let height = rectangle.bottom - rectangle.top;
    // SAFETY: The style and z-order changes apply only to Nickel's live panel HWND. TOOLWINDOW
    // keeps it out of native task switching; TOPMOST keeps it above ordinary application windows.
    let topmost = unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (style | WS_EX_TOOLWINDOW.0) as isize);
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            rectangle.left,
            rectangle.top,
            rectangle.right - rectangle.left,
            height,
            SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
        .is_ok()
    };
    let mut appbar = APPBARDATA {
        cbSize: size_of::<APPBARDATA>() as u32,
        hWnd: hwnd,
        uCallbackMessage: 0x8000 + 17,
        uEdge: ABE_BOTTOM,
        rc: rectangle,
        lParam: LPARAM(0),
    };
    // SAFETY: appbar describes a live top-level window owned by this process. The calls are
    // synchronous and Shell32 copies the structure before returning.
    let registered = unsafe { SHAppBarMessage(ABM_NEW, &mut appbar) } != 0;
    if !registered {
        return reserve_work_area_without_explorer(rectangle) && topmost;
    }
    PANEL_APPBAR_REGISTERED.store(true, std::sync::atomic::Ordering::Relaxed);
    unsafe {
        SHAppBarMessage(ABM_QUERYPOS, &mut appbar);
    }
    appbar.rc.top = appbar.rc.bottom - height;
    let positioned = unsafe { SHAppBarMessage(ABM_SETPOS, &mut appbar) } != 0;
    // SAFETY: This only applies Shell32's negotiated geometry and the persistent topmost band to
    // Nickel's live panel; it neither activates nor resizes any foreign window.
    let topmost = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            appbar.rc.left,
            appbar.rc.top,
            appbar.rc.right - appbar.rc.left,
            height,
            SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
        .is_ok()
    };
    positioned && topmost
}

fn reserve_work_area_without_explorer(panel: RECT) -> bool {
    let mut work_area = RECT::default();
    // SAFETY: work_area is writable storage and the fallback is used only for the single-monitor
    // Explorer-free session. SPIF_SENDCHANGE broadcasts the new work area without persisting it.
    if unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some((&mut work_area as *mut RECT).cast()),
            Default::default(),
        )
    }
    .is_err()
    {
        return false;
    }
    let mut original = work_area;
    original.bottom = panel.bottom;
    if let Ok(mut saved) = ORIGINAL_WORK_AREA.lock() {
        *saved = Some(original);
    }
    work_area.left = panel.left;
    work_area.right = panel.right;
    work_area.bottom = panel.top;
    unsafe {
        SystemParametersInfoW(
            SPI_SETWORKAREA,
            0,
            Some((&mut work_area as *mut RECT).cast()),
            SPIF_SENDCHANGE,
        )
        .is_ok()
    }
}

pub fn release_panel_window(window: &winit::window::Window) {
    let Some(hwnd) = window_hwnd(window) else {
        return;
    };
    let mut appbar = APPBARDATA {
        cbSize: size_of::<APPBARDATA>() as u32,
        hWnd: hwnd,
        ..Default::default()
    };
    // SAFETY: Removing an appbar registration is idempotent for a live HWND.
    unsafe {
        if PANEL_APPBAR_REGISTERED.swap(false, std::sync::atomic::Ordering::Relaxed) {
            SHAppBarMessage(ABM_REMOVE, &mut appbar);
        }
    }
    if let Ok(mut saved) = ORIGINAL_WORK_AREA.lock()
        && let Some(mut original) = saved.take()
    {
        // SAFETY: original is the work area captured before Nickel reserved the panel strip.
        unsafe {
            let _ = SystemParametersInfoW(
                SPI_SETWORKAREA,
                0,
                Some((&mut original as *mut RECT).cast()),
                SPIF_SENDCHANGE,
            );
        }
    }
}

fn window_hwnd(window: &winit::window::Window) -> Option<HWND> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };
    Some(HWND(handle.hwnd.get() as *mut c_void))
}

pub struct TrayFeed;
impl TrayFeed {
    pub fn new() -> Self {
        Self
    }
}
impl TraySource for TrayFeed {
    fn snapshot(&self) -> Vec<TrayItem> {
        Vec::new()
    }
    fn activate(&self, _: &str) {}
}

pub fn send_shell_command(command: ShellCommand) -> bool {
    let ShellCommand::WindowAction { window, action } = command else {
        return false;
    };
    let hwnd = hwnd(window);
    // SAFETY: The handle comes from EnumWindows and is revalidated immediately before use.
    if unsafe { !IsWindow(Some(hwnd)).as_bool() } {
        return false;
    }
    // SAFETY: These operations do not dereference application memory; they send standard window
    // manager requests to a currently valid top-level HWND.
    unsafe {
        match action {
            WindowAction::Activate => {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd).as_bool()
            }
            WindowAction::Close => PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok(),
            WindowAction::Maximize => {
                let _ = ShowWindow(
                    hwnd,
                    if IsZoomed(hwnd).as_bool() {
                        SW_RESTORE
                    } else {
                        SW_MAXIMIZE
                    },
                );
                true
            }
            WindowAction::Minimize => {
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
                true
            }
        }
    }
}

pub struct WindowFeed;

impl WindowFeed {
    pub fn new() -> Self {
        Self
    }

    pub fn snapshot(&self, _: &Launcher) -> Option<Vec<OpenWindow>> {
        let mut windows = Vec::new();
        // SAFETY: The callback only reads top-level window metadata and the LPARAM points to this
        // live vector for the duration of the synchronous EnumWindows call.
        unsafe {
            let state = LPARAM((&mut windows as *mut Vec<OpenWindow>) as isize);
            EnumWindows(Some(collect_window), state).ok()?;
        }
        Some(windows)
    }

    pub fn preview(&self, _: WindowId) -> Option<WindowPreview> {
        None
    }

    pub fn supports_previews(&self) -> bool {
        false
    }

    pub fn icon(&self, window: WindowId) -> Option<image::RgbaImage> {
        let hwnd = hwnd(window);
        executable_path(hwnd)
            .as_deref()
            .and_then(executable_icon)
            .or_else(|| window_icon(hwnd))
    }
}

unsafe extern "system" fn collect_window(hwnd: HWND, state: LPARAM) -> BOOL {
    // SAFETY: EnumWindows invokes this callback with a valid top-level HWND.
    if unsafe { !IsWindowVisible(hwnd).as_bool() } {
        return BOOL(1);
    }
    let mut process_id = 0;
    // SAFETY: process_id is valid writable storage for the duration of this call.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    if process_id == std::process::id() {
        return BOOL(1);
    }
    let Some(title) = window_title(hwnd) else {
        return BOOL(1);
    };
    let Some(class) = window_class(hwnd) else {
        return BOOL(1);
    };
    if !is_taskbar_window(hwnd, &class) {
        return BOOL(1);
    }
    let application_id = Some(
        executable_path(hwnd)
            .map(|path| {
                ApplicationId::new(format!(
                    "windows-exe:{}",
                    path.to_string_lossy().to_ascii_lowercase()
                ))
            })
            .unwrap_or_else(|| {
                ApplicationId::new(format!("windows-class:{}", class.to_ascii_lowercase()))
            }),
    );
    // SAFETY: state was constructed from a live Vec<OpenWindow> immediately before EnumWindows.
    let windows = unsafe { &mut *(state.0 as *mut Vec<OpenWindow>) };
    // Chrome and other multi-process applications can report a descendant HWND as foreground.
    // Compare the enumerated task window with the foreground window's top-level ancestor.
    // SAFETY: Reading the foreground handle and its root ancestor does not mutate either window.
    let foreground = unsafe { GetForegroundWindow() };
    let foreground_root = if foreground.0.is_null() {
        foreground
    } else {
        unsafe { GetAncestor(foreground, GA_ROOT) }
    };
    windows.push(OpenWindow {
        id: window_id(hwnd),
        application_id,
        active: foreground == hwnd || foreground_root == hwnd,
        title,
    });
    BOOL(1)
}

fn is_taskbar_window(hwnd: HWND, class: &str) -> bool {
    if is_shell_infrastructure(class) {
        return false;
    }
    // SAFETY: hwnd is a top-level handle supplied by EnumWindows.
    let extended_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    let explicitly_app = extended_style & WS_EX_APPWINDOW.0 != 0;
    let tool_window = extended_style & WS_EX_TOOLWINDOW.0 != 0;
    explicitly_app || (!tool_window && is_last_visible_owned_window(hwnd))
}

fn is_last_visible_owned_window(hwnd: HWND) -> bool {
    // Windows represents several real application windows, including Chromium windows, as owned
    // top-level windows. The task-switch target is the last visible popup in the root-owner chain,
    // not simply every unowned HWND.
    // SAFETY: All handles are obtained from live top-level windows and are only queried.
    unsafe {
        let mut candidate = GetAncestor(hwnd, GA_ROOTOWNER);
        if candidate.0.is_null() {
            candidate = hwnd;
        }
        loop {
            let popup = GetLastActivePopup(candidate);
            if popup == candidate {
                break;
            }
            candidate = popup;
            if IsWindowVisible(candidate).as_bool() {
                break;
            }
        }
        candidate == hwnd
    }
}

fn is_shell_infrastructure(class: &str) -> bool {
    matches!(
        class,
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "Windows.UI.Core.CoreWindow"
    )
}

fn window_title(hwnd: HWND) -> Option<String> {
    // SAFETY: hwnd is supplied by EnumWindows and remains valid during the callback.
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return None;
    }
    let mut buffer = vec![0_u16; length as usize + 1];
    // SAFETY: buffer is writable and includes space for the terminating null.
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    (copied > 0).then(|| String::from_utf16_lossy(&buffer[..copied as usize]))
}

fn window_class(hwnd: HWND) -> Option<String> {
    let mut buffer = [0_u16; 256];
    // SAFETY: hwnd is supplied by EnumWindows and buffer is valid writable storage.
    let copied = unsafe { GetClassNameW(hwnd, &mut buffer) };
    (copied > 0).then(|| String::from_utf16_lossy(&buffer[..copied as usize]))
}

fn executable_path(hwnd: HWND) -> Option<PathBuf> {
    let mut process_id = 0;
    // SAFETY: process_id is valid writable storage and hwnd is a current top-level window.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    if process_id == 0 {
        return None;
    }
    // SAFETY: The process handle is closed on every path after a successful open.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id).ok()?;
        let mut buffer = vec![0_u16; 32_768];
        let mut length = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &raw mut length,
        );
        let _ = CloseHandle(process);
        result.ok()?;
        buffer.truncate(length as usize);
        Some(PathBuf::from(std::ffi::OsString::from_wide(&buffer)))
    }
}

fn executable_icon(path: &std::path::Path) -> Option<image::RgbaImage> {
    use std::os::windows::ffi::OsStrExt;

    let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut info = SHFILEINFOW::default();
    // SAFETY: wide is null-terminated, info is valid writable storage, and the returned icon is
    // owned by this call and destroyed after its pixels are copied.
    unsafe {
        let result = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&raw mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON,
        );
        if result == 0 || info.hIcon.0.is_null() {
            return None;
        }
        let image = render_icon(info.hIcon);
        let _ = DestroyIcon(info.hIcon);
        image
    }
}

fn window_icon(hwnd: HWND) -> Option<image::RgbaImage> {
    // SAFETY: These class icon handles are owned by the window class and remain borrowed here.
    let handle = unsafe {
        let large = GetClassLongPtrW(hwnd, GCLP_HICON);
        if large != 0 {
            large
        } else {
            GetClassLongPtrW(hwnd, GCLP_HICONSM)
        }
    };
    (handle != 0)
        .then(|| HICON(handle as *mut c_void))
        .and_then(render_icon)
}

fn render_icon(icon: HICON) -> Option<image::RgbaImage> {
    const SIZE: u32 = 32;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIZE as i32,
            biHeight: -(SIZE as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = std::ptr::null_mut();
    // SAFETY: GDI resources are checked before use and restored/released before returning. The DIB
    // remains selected and alive while its pixel memory is copied.
    unsafe {
        let screen = GetDC(None);
        if screen.0.is_null() {
            return None;
        }
        let memory = CreateCompatibleDC(Some(screen));
        if memory.0.is_null() {
            ReleaseDC(None, screen);
            return None;
        }
        let bitmap = match CreateDIBSection(
            Some(screen),
            &raw const info,
            DIB_RGB_COLORS,
            &raw mut pixels,
            None,
            0,
        ) {
            Ok(bitmap) => bitmap,
            Err(_) => {
                let _ = DeleteDC(memory);
                ReleaseDC(None, screen);
                return None;
            }
        };
        let previous = SelectObject(memory, HGDIOBJ(bitmap.0));
        let drawn = DrawIconEx(
            memory,
            0,
            0,
            icon,
            SIZE as i32,
            SIZE as i32,
            0,
            None,
            DI_NORMAL,
        )
        .is_ok();
        let mut rgba = vec![0_u8; (SIZE * SIZE * 4) as usize];
        if drawn && !pixels.is_null() {
            let bgra = std::slice::from_raw_parts(pixels.cast::<u8>(), rgba.len());
            for (source, target) in bgra.chunks_exact(4).zip(rgba.chunks_exact_mut(4)) {
                target.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
            }
        }
        SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        drawn
            .then(|| image::RgbaImage::from_raw(SIZE, SIZE, rgba))
            .flatten()
    }
}

fn window_id(hwnd: HWND) -> WindowId {
    WindowId(hwnd.0 as usize as u64)
}

fn hwnd(window: WindowId) -> HWND {
    HWND(window.0 as usize as *mut c_void)
}

#[cfg(test)]
mod tests {
    use super::{executable_icon, is_shell_infrastructure};

    #[test]
    fn shell_executable_icon_has_visible_pixels() {
        let image = executable_icon(&std::env::current_exe().expect("test executable path"))
            .expect("Windows Shell returns an executable icon");
        assert!(image.pixels().any(|pixel| pixel.0[3] != 0));
    }

    #[test]
    fn desktop_and_nickel_surfaces_are_not_panel_tasks() {
        assert!(is_shell_infrastructure("Progman"));
        assert!(is_shell_infrastructure("WorkerW"));
        assert!(!is_shell_infrastructure("winit"));
        assert!(!is_shell_infrastructure("CabinetWClass"));
    }
}
