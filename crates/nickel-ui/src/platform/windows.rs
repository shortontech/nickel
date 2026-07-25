use std::{
    env,
    ffi::c_void,
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    sync::{
        Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use windows::{
    Win32::{
        Foundation::{COLORREF, CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Dwm::{
            DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
            DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE, DwmRegisterThumbnail,
            DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
        },
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW, HGDIOBJ,
            MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, ReleaseDC, SelectObject,
        },
        Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES,
        System::LibraryLoader::GetModuleHandleW,
        System::Threading::{
            AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId, OpenProcess,
            PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        },
        System::{
            Com::{
                CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize,
            },
            DataExchange::{COPYDATASTRUCT, CloseClipboard, GetClipboardData, OpenClipboard},
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        },
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, MOD_NOREPEAT, MOD_WIN, RegisterHotKey, SetFocus,
            },
            Shell::{
                ABE_BOTTOM, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA,
                DWPOS_CENTER, DWPOS_FILL, DWPOS_FIT, DWPOS_SPAN, DWPOS_STRETCH, DWPOS_TILE,
                DesktopWallpaper, IDesktopWallpaper, NIF_GUID, NIF_ICON, NIF_MESSAGE, NIF_STATE,
                NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT, NIS_HIDDEN,
                NOTIFYICON_VERSION_4, SHAppBarMessage, SHFILEINFOW, SHGFI_ICON, SHGetFileInfoW,
                ShellExecuteW,
            },
            WindowsAndMessaging::{
                BringWindowToTop, CallNextHookEx, CallWindowProcW, CreateWindowExW, DI_NORMAL,
                DefWindowProcW, DestroyIcon, DrawIconEx, EnumWindows, GA_ROOT, GA_ROOTOWNER,
                GCLP_HICON, GCLP_HICONSM, GWL_EXSTYLE, GWLP_WNDPROC, GetAncestor, GetClassLongPtrW,
                GetClassNameW, GetCursorPos, GetForegroundWindow, GetLastActivePopup, GetMessageW,
                GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
                GetWindowThreadProcessId, HICON, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTLEFT,
                HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, HWND_BOTTOM, HWND_BROADCAST, HWND_TOPMOST,
                IsIconic, IsWindow, IsWindowVisible, IsZoomed, KBDLLHOOKSTRUCT, LWA_ALPHA, MSG,
                MSLLHOOKSTRUCT, PostMessageW, RegisterClassW, RegisterWindowMessageW,
                SPI_GETWORKAREA, SPI_SETWORKAREA, SPIF_SENDCHANGE, SW_HIDE, SW_MAXIMIZE,
                SW_MINIMIZE, SW_RESTORE, SW_SHOW, SW_SHOWNOACTIVATE, SW_SHOWNORMAL,
                SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
                SWP_NOZORDER, SendNotifyMessageW, SetForegroundWindow, SetLayeredWindowAttributes,
                SetWindowLongPtrW, SetWindowPos, SetWindowsHookExW, ShowWindow,
                SystemParametersInfoW, WH_KEYBOARD_LL, WH_MOUSE_LL, WINDOW_EX_STYLE, WINDOW_STYLE,
                WM_CLOSE, WM_CONTEXTMENU, WM_COPYDATA, WM_HOTKEY, WM_KEYDOWN, WM_KEYUP,
                WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP,
                WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
                WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
                WindowFromPoint,
            },
        },
    },
    core::{BOOL, PCWSTR, PWSTR, w},
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

use nickel_core::hotkeys::{
    Hotkey, HotkeyAction, HotkeyController, HotkeyOutcome, HotkeySnapshot, KeyEdge,
};

use crate::{
    desktop::{Wallpaper, WallpaperPosition},
    launcher::Launcher,
    model::{Application, ApplicationId, OpenWindow, TrayItem, WindowId, WindowPreview},
    platform::{GlobalShortcut, ShellCommand, TraySource, WindowAction},
};

pub fn wallpaper() -> Wallpaper {
    // SAFETY: COM is initialized for this call on Nickel's UI thread and all returned task
    // allocator strings are freed before the apartment is released.
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = query_wallpaper();
        if initialized {
            CoUninitialize();
        }
        result.unwrap_or_else(|error| {
            eprintln!("Nickel wallpaper COM query failed: {error}");
            fallback_wallpaper()
        })
    }
}

fn fallback_wallpaper() -> Wallpaper {
    let cache_path = transcoded_wallpaper_path();
    let image = load_wallpaper_image(&cache_path);
    if let Some(image) = &image {
        eprintln!(
            "Nickel wallpaper fallback: {} ({}x{})",
            cache_path.display(),
            image.width(),
            image.height()
        );
    }
    Wallpaper {
        image,
        ..Wallpaper::default()
    }
}

unsafe fn query_wallpaper() -> windows::core::Result<Wallpaper> {
    let desktop: IDesktopWallpaper =
        unsafe { CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL)? };
    let color = unsafe { desktop.GetBackgroundColor()? }.0;
    let position = unsafe { desktop.GetPosition()? };
    let monitor = unsafe { desktop.GetMonitorDevicePathAt(0)? };
    let path = unsafe { desktop.GetWallpaper(monitor)? };
    let path_string = unsafe { path.to_string() }.unwrap_or_default();
    unsafe {
        CoTaskMemFree(Some(monitor.0.cast()));
        CoTaskMemFree(Some(path.0.cast()));
    }
    let cache_path = transcoded_wallpaper_path();
    let (image, source) = match load_wallpaper_image(PathBuf::from(&path_string)) {
        Some(image) => (Some(image), path_string.clone()),
        None => match load_wallpaper_image(&cache_path) {
            Some(image) => (Some(image), cache_path.to_string_lossy().into_owned()),
            None => (None, "<none>".to_owned()),
        },
    };
    if let Some(image) = &image {
        eprintln!(
            "Nickel wallpaper: {source} ({}x{})",
            image.width(),
            image.height()
        );
    } else {
        eprintln!(
            "Nickel wallpaper: no image; configured={path_string:?}, cache={}",
            cache_path.display()
        );
    }
    Ok(Wallpaper {
        image,
        color: [
            (color & 0xff) as u8,
            ((color >> 8) & 0xff) as u8,
            ((color >> 16) & 0xff) as u8,
        ],
        position: match position {
            value if value == DWPOS_CENTER => WallpaperPosition::Center,
            value if value == DWPOS_TILE => WallpaperPosition::Tile,
            value if value == DWPOS_STRETCH => WallpaperPosition::Stretch,
            value if value == DWPOS_FIT => WallpaperPosition::Fit,
            value if value == DWPOS_SPAN => WallpaperPosition::Span,
            value if value == DWPOS_FILL => WallpaperPosition::Fill,
            _ => WallpaperPosition::Fill,
        },
    })
}

fn transcoded_wallpaper_path() -> PathBuf {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("Microsoft")
        .join("Windows")
        .join("Themes")
        .join("TranscodedWallpaper")
}

fn load_wallpaper_image(path: impl AsRef<std::path::Path>) -> Option<image::RgbaImage> {
    image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()
        .map(|image| image.to_rgba8())
}

#[path = "windows_start_menu.rs"]
mod start_menu;

pub fn applications() -> Vec<Application> {
    start_menu::load_applications()
}

pub fn application_icon(reference: &str) -> Option<image::RgbaImage> {
    executable_icon(PathBuf::from(reference).as_path())
}

pub fn network_status() -> super::NetworkStatus {
    use windows::Win32::{
        Foundation::{HANDLE, NO_ERROR},
        NetworkManagement::WiFi::{
            WLAN_CONNECTION_ATTRIBUTES, WLAN_INTERFACE_INFO_LIST, WlanCloseHandle,
            WlanEnumInterfaces, WlanFreeMemory, WlanOpenHandle, WlanQueryInterface,
            wlan_interface_state_connected, wlan_intf_opcode_current_connection,
        },
    };

    let mut negotiated = 0;
    let mut handle = HANDLE::default();
    if unsafe { WlanOpenHandle(2, None, &mut negotiated, &mut handle) } != NO_ERROR.0 {
        return super::NetworkStatus::default();
    }
    let mut interfaces = std::ptr::null_mut::<WLAN_INTERFACE_INFO_LIST>();
    if unsafe { WlanEnumInterfaces(handle, None, &mut interfaces) } != NO_ERROR.0
        || interfaces.is_null()
    {
        unsafe {
            WlanCloseHandle(handle, None);
        }
        return super::NetworkStatus::default();
    }

    let mut status = super::NetworkStatus {
        available: true,
        ..Default::default()
    };
    let entries = unsafe {
        std::slice::from_raw_parts(
            (*interfaces).InterfaceInfo.as_ptr(),
            (*interfaces).dwNumberOfItems as usize,
        )
    };
    for interface in entries {
        let mut bytes = 0;
        let mut data = std::ptr::null_mut::<c_void>();
        if unsafe {
            WlanQueryInterface(
                handle,
                &raw const interface.InterfaceGuid,
                wlan_intf_opcode_current_connection,
                None,
                &mut bytes,
                &mut data,
                None,
            )
        } != NO_ERROR.0
            || data.is_null()
            || bytes < std::mem::size_of::<WLAN_CONNECTION_ATTRIBUTES>() as u32
        {
            continue;
        }
        let connection = unsafe { &*data.cast::<WLAN_CONNECTION_ATTRIBUTES>() };
        if connection.isState == wlan_interface_state_connected {
            let ssid = &connection.wlanAssociationAttributes.dot11Ssid;
            let length = (ssid.uSSIDLength as usize).min(ssid.ucSSID.len());
            status.connected = true;
            status.name = String::from_utf8_lossy(&ssid.ucSSID[..length]).into_owned();
            status.signal_percent = connection.wlanAssociationAttributes.wlanSignalQuality;
        }
        unsafe { WlanFreeMemory(data) };
        if status.connected {
            break;
        }
    }
    unsafe {
        WlanFreeMemory(interfaces.cast());
        WlanCloseHandle(handle, None);
    }
    status
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
    const VK_LWIN: u32 = 0x5b;
    const VK_RWIN: u32 = 0x5c;
    const VK_R: u32 = 0x52;
    const LEFT_WIN_HOTKEY: i32 = 0x4e01;
    const RIGHT_WIN_HOTKEY: i32 = 0x4e02;
    const RUN_HOTKEY: i32 = 0x4e03;

    WINDOWS_KEY_SENDER.set(sender).ok();
    let modifiers = MOD_WIN | MOD_NOREPEAT;
    let left_registered =
        unsafe { RegisterHotKey(None, LEFT_WIN_HOTKEY, modifiers, VK_LWIN) }.is_ok();
    let right_registered =
        unsafe { RegisterHotKey(None, RIGHT_WIN_HOTKEY, modifiers, VK_RWIN) }.is_ok();
    let run_registered = unsafe { RegisterHotKey(None, RUN_HOTKEY, modifiers, VK_R) }.is_ok();
    let registration_bits = u8::from(left_registered) | (u8::from(right_registered) << 1);
    RUN_HOTKEY_REGISTERED.store(run_registered, std::sync::atomic::Ordering::Release);
    if registration_bits == 0 {
        tracing::warn!(
            "bare Windows-key hotkey registration unavailable; using passive hook observation"
        );
    } else {
        tracing::info!(
            left_registered,
            right_registered,
            run_registered,
            "registered bare Windows key through RegisterHotKey"
        );
    }
    if !run_registered {
        tracing::warn!("Win+R registration unavailable; using low-level hook fallback");
    }

    // SAFETY: The callback remains valid for the process lifetime and this thread owns the
    // message loop required by a low-level keyboard hook.
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(windows_key_hook), None, 0) };
    let Ok(_hook) = hook else {
        eprintln!("failed to register the Windows-key launcher hook");
        return;
    };
    let mouse_hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(windows_mouse_hook), None, 0) };
    let Ok(_mouse_hook) = mouse_hook else {
        eprintln!("failed to register Nickel's Windows mouse chord hook");
        return;
    };
    let mut message = MSG::default();
    // SAFETY: message is valid writable storage for each synchronous call.
    while unsafe { GetMessageW(&mut message, None, 0, 0).as_bool() } {
        if message.message == WM_HOTKEY {
            match message.wParam.0 as i32 {
                LEFT_WIN_HOTKEY | RIGHT_WIN_HOTKEY => register_bare_windows_key_press(),
                RUN_HOTKEY => {
                    tracing::debug!("Win+R hotkey received");
                    if let Some(sender) = WINDOWS_KEY_SENDER.get() {
                        let _ = sender.send(GlobalShortcut::ShowRun);
                    }
                }
                _ => {}
            }
        }
    }
}

static WINDOWS_KEY_SENDER: std::sync::OnceLock<Sender<GlobalShortcut>> = std::sync::OnceLock::new();
static HOTKEY_CONTROLLER: std::sync::OnceLock<Mutex<HotkeyController>> = std::sync::OnceLock::new();
static RUN_HOTKEY_REGISTERED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static INPUT_TRACE_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static PANEL_APPBAR_REGISTERED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static PANEL_FULLSCREEN_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static ORIGINAL_WORK_AREA: std::sync::Mutex<Option<RECT>> = std::sync::Mutex::new(None);
static TRAY_ITEMS: Mutex<Vec<NativeTrayIcon>> = Mutex::new(Vec::new());
static PANEL_WINDOW_PROC: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static PANEL_WINDOW_HANDLE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static TRAY_NOTIFY_WINDOW_HANDLE: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(0);
static PREVIOUS_FOREGROUND_WINDOW: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(0);
static LAUNCHER_FOREGROUND_WINDOW: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(0);
static LAUNCHER_WINDOW_HANDLE: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(0);
static CONTEXT_MENU_WINDOW_HANDLE: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(0);
static DWM_THUMBNAILS: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static RESTORE_LAUNCHER_FOCUS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static WINDOW_DRAG: Mutex<Option<WindowDrag>> = Mutex::new(None);
const PANEL_APPBAR_CALLBACK: u32 = 0x8000 + 17;
const ABN_FULLSCREENAPP_CODE: usize = 2;

#[derive(Clone, Copy)]
struct WindowDrag {
    window: isize,
    start: POINT,
    rectangle: RECT,
    resize_edge: Option<u32>,
    last_update: u32,
}

#[derive(Clone)]
struct NativeTrayIcon {
    owner: isize,
    id: u32,
    guid: Option<windows::core::GUID>,
    callback_message: u32,
    version: u32,
    hidden: bool,
    item: TrayItem,
}

#[repr(C)]
struct TrayNotifyData {
    signature: i32,
    message: u32,
    icon: TrayNotifyIconData,
}

#[repr(C)]
struct TrayNotifyIconData {
    cb_size: i32,
    window: u32,
    id: u32,
    flags: u32,
    callback_message: u32,
    icon: u32,
    tip: [u16; 128],
    state: i32,
    state_mask: i32,
    info: [u16; 256],
    version: u32,
    info_title: [u16; 64],
    info_flags: u32,
    guid: windows::core::GUID,
    balloon_icon: u32,
}

unsafe extern "system" fn windows_key_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    const VK_LWIN: u32 = 0x5b;
    const VK_RWIN: u32 = 0x5c;
    const VK_MENU: u32 = 0x12;
    const VK_LMENU: u32 = 0xa4;
    const VK_RMENU: u32 = 0xa5;
    const VK_SHIFT: u32 = 0x10;
    const VK_LSHIFT: u32 = 0xa0;
    const VK_RSHIFT: u32 = 0xa1;
    const VK_TAB: u32 = 0x09;
    const VK_OEM_3: u32 = 0xc0;
    const VK_R: u32 = 0x52;

    if code < 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    // SAFETY: WH_KEYBOARD_LL supplies a KBDLLHOOKSTRUCT pointer in LPARAM.
    let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
    let message = wparam.0 as u32;
    let edge = if message == WM_KEYDOWN || message == WM_SYSKEYDOWN {
        KeyEdge::Pressed
    } else if message == WM_KEYUP || message == WM_SYSKEYUP {
        KeyEdge::Released
    } else {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    };
    let key = match event.vkCode {
        VK_LWIN | VK_RWIN => Hotkey::Super,
        VK_MENU | VK_LMENU | VK_RMENU => Hotkey::Alt,
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => Hotkey::Shift,
        VK_TAB => Hotkey::Tab,
        VK_OEM_3 => Hotkey::Grave,
        VK_R => Hotkey::Run,
        _ => Hotkey::Other,
    };
    if key == Hotkey::Run && RUN_HOTKEY_REGISTERED.load(std::sync::atomic::Ordering::Acquire) {
        if edge == KeyEdge::Pressed
            && let Ok(mut controller) = hotkey_controller().lock()
        {
            // RegisterHotKey owns Win+R dispatch. The hook only records that another key joined
            // the Windows-key press, preventing the later release from toggling the launcher.
            controller.handle(Hotkey::Other, KeyEdge::Pressed);
        }
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    if matches!(key, Hotkey::Tab | Hotkey::Grave)
        && edge == KeyEdge::Pressed
        && unsafe { GetAsyncKeyState(VK_MENU as i32) < 0 }
        && let Ok(mut controller) = hotkey_controller().lock()
        && !controller.snapshot().alt_held
    {
        controller.handle(Hotkey::Alt, KeyEdge::Pressed);
    }
    let (outcome, snapshot) = hotkey_controller()
        .lock()
        .map(|mut controller| {
            let outcome = controller.handle(key, edge);
            (outcome, controller.snapshot())
        })
        .unwrap_or_default();
    if !matches!(key, Hotkey::Run | Hotkey::Other) {
        trace_input("key", Some(key), Some(edge), outcome, snapshot);
    }
    send_hotkey_action(outcome.action);
    if outcome.suppress {
        LRESULT(1)
    } else {
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }
}

fn register_bare_windows_key_press() {
    // RegisterHotKey owns bare-Windows dispatch and suppression, but it can post repeated or
    // delayed messages. Physical down/up state comes exclusively from the keyboard hook.
    tracing::debug!("bare Windows hotkey received");
}

fn hotkey_controller() -> &'static Mutex<HotkeyController> {
    HOTKEY_CONTROLLER.get_or_init(|| Mutex::new(HotkeyController::default()))
}

fn input_trace_enabled() -> bool {
    *INPUT_TRACE_ENABLED.get_or_init(|| {
        env::var_os("NICKEL_INPUT_TRACE").is_some_and(|value| {
            !matches!(
                value.to_string_lossy().to_ascii_lowercase().as_str(),
                "" | "0" | "false" | "no" | "off"
            )
        })
    })
}

fn physical_key_states() -> (bool, bool, bool, bool, bool) {
    const VK_LWIN: i32 = 0x5b;
    const VK_RWIN: i32 = 0x5c;
    const VK_MENU: i32 = 0x12;
    const VK_SHIFT: i32 = 0x10;
    const VK_TAB: i32 = 0x09;
    const VK_OEM_3: i32 = 0xc0;
    unsafe {
        (
            GetAsyncKeyState(VK_LWIN) < 0 || GetAsyncKeyState(VK_RWIN) < 0,
            GetAsyncKeyState(VK_MENU) < 0,
            GetAsyncKeyState(VK_SHIFT) < 0,
            GetAsyncKeyState(VK_TAB) < 0,
            GetAsyncKeyState(VK_OEM_3) < 0,
        )
    }
}

fn trace_input(
    source: &str,
    key: Option<Hotkey>,
    edge: Option<KeyEdge>,
    outcome: HotkeyOutcome,
    state: HotkeySnapshot,
) {
    if !input_trace_enabled() {
        return;
    }
    let foreground = unsafe { GetForegroundWindow() };
    let (physical_super, physical_alt, physical_shift, physical_tab, physical_grave) =
        physical_key_states();
    eprintln!(
        "input source={source} event={key:?} edge={edge:?} foreground={:?} \
         physical[super={physical_super} alt={physical_alt} shift={physical_shift} tab={physical_tab} grave={physical_grave}] \
         controller[super={} chord={} alt={} shift={} tab={} grave={} run={} switch={} launcher={}] \
         suppress={} action={:?}",
        foreground.0,
        state.super_held,
        state.super_chorded,
        state.alt_held,
        state.shift_held,
        state.tab_held,
        state.grave_held,
        state.run_held,
        state.switch_active,
        state.launcher_visible,
        outcome.suppress,
        outcome.action
    );
}

fn send_hotkey_action(action: Option<HotkeyAction>) {
    let shortcut = match action {
        Some(HotkeyAction::ShowLauncher) => GlobalShortcut::ShowLauncher,
        Some(HotkeyAction::HideLauncher) => GlobalShortcut::HideLauncher,
        Some(HotkeyAction::ShowRun) => GlobalShortcut::ShowRun,
        Some(HotkeyAction::SwitchNext) => GlobalShortcut::SwitchNext,
        Some(HotkeyAction::SwitchPrevious) => GlobalShortcut::SwitchPrevious,
        Some(HotkeyAction::SwitchGroupNext) => GlobalShortcut::SwitchGroupNext,
        Some(HotkeyAction::SwitchGroupPrevious) => GlobalShortcut::SwitchGroupPrevious,
        Some(HotkeyAction::CommitSwitch) => GlobalShortcut::CommitSwitch,
        None => return,
    };
    if let Some(sender) = WINDOWS_KEY_SENDER.get() {
        let _ = sender.send(shortcut);
    }
}

unsafe extern "system" fn windows_mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let message = wparam.0 as u32;
    // SAFETY: WH_MOUSE_LL supplies an MSLLHOOKSTRUCT pointer in LPARAM.
    let event = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
    if let Ok(mut drag) = WINDOW_DRAG.lock()
        && let Some(operation) = *drag
    {
        let release = matches!(message, WM_LBUTTONUP | WM_RBUTTONUP);
        if message == WM_MOUSEMOVE || release {
            if release || event.time.wrapping_sub(operation.last_update) >= 33 {
                update_window_drag(operation, event.pt);
                if !release {
                    drag.as_mut().expect("drag operation exists").last_update = event.time;
                }
            }
            if release {
                *drag = None;
                let snapshot = hotkey_controller()
                    .lock()
                    .map(|controller| controller.snapshot())
                    .unwrap_or_default();
                trace_input(
                    "mouse-release",
                    None,
                    Some(KeyEdge::Released),
                    HotkeyOutcome::default(),
                    snapshot,
                );
                return LRESULT(1);
            }
            // Observe pointer motion without consuming it. Suppressing WM_MOUSEMOVE freezes the
            // real cursor while the window chases coordinates reported by the hook.
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }
    }
    if !matches!(message, WM_LBUTTONDOWN | WM_RBUTTONDOWN) {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let (super_held, chord_started) = hotkey_controller()
        .lock()
        .map(|mut controller| {
            let super_held = controller.snapshot().super_held;
            let chord_started = controller.begin_pointer_chord();
            (super_held, chord_started)
        })
        .unwrap_or_default();
    tracing::debug!(
        super_held,
        chord_started,
        button = if message == WM_LBUTTONDOWN {
            "left"
        } else {
            "right"
        },
        "Windows-key mouse gesture candidate"
    );
    if !chord_started {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let snapshot = hotkey_controller()
        .lock()
        .map(|controller| controller.snapshot())
        .unwrap_or_default();
    trace_input(
        if message == WM_LBUTTONDOWN {
            "mouse-move"
        } else {
            "mouse-resize"
        },
        None,
        Some(KeyEdge::Pressed),
        HotkeyOutcome {
            suppress: true,
            action: None,
        },
        snapshot,
    );

    let target = unsafe { GetAncestor(WindowFromPoint(event.pt), GA_ROOT) };
    if target.0.is_null() {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let mut process_id = 0;
    unsafe {
        GetWindowThreadProcessId(target, Some(&mut process_id));
    }
    if process_id == unsafe { GetCurrentProcessId() } {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let mut rectangle = RECT::default();
    if unsafe { GetWindowRect(target, &mut rectangle) }.is_err() {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    if let Ok(mut drag) = WINDOW_DRAG.lock() {
        *drag = Some(WindowDrag {
            window: target.0 as isize,
            start: event.pt,
            rectangle,
            resize_edge: (message == WM_RBUTTONDOWN).then(|| resize_hit_test(target, event.pt)),
            last_update: event.time,
        });
    }
    unsafe {
        let _ = SetForegroundWindow(target);
    }
    LRESULT(1)
}

fn update_window_drag(operation: WindowDrag, pointer: POINT) {
    let delta_x = pointer.x - operation.start.x;
    let delta_y = pointer.y - operation.start.y;
    let mut rectangle = operation.rectangle;
    if let Some(edge) = operation.resize_edge {
        if matches!(edge, HTLEFT | HTTOPLEFT | HTBOTTOMLEFT) {
            rectangle.left += delta_x;
        }
        if matches!(edge, HTRIGHT | HTTOPRIGHT | HTBOTTOMRIGHT) {
            rectangle.right += delta_x;
        }
        if matches!(edge, HTTOP | HTTOPLEFT | HTTOPRIGHT) {
            rectangle.top += delta_y;
        }
        if matches!(edge, HTBOTTOM | HTBOTTOMLEFT | HTBOTTOMRIGHT) {
            rectangle.bottom += delta_y;
        }
        if rectangle.right - rectangle.left < 120 {
            if matches!(edge, HTLEFT | HTTOPLEFT | HTBOTTOMLEFT) {
                rectangle.left = rectangle.right - 120;
            } else {
                rectangle.right = rectangle.left + 120;
            }
        }
        if rectangle.bottom - rectangle.top < 80 {
            if matches!(edge, HTTOP | HTTOPLEFT | HTTOPRIGHT) {
                rectangle.top = rectangle.bottom - 80;
            } else {
                rectangle.bottom = rectangle.top + 80;
            }
        }
    } else {
        let width = rectangle.right - rectangle.left;
        let height = rectangle.bottom - rectangle.top;
        rectangle.left += delta_x;
        rectangle.top += delta_y;
        rectangle.right = rectangle.left + width;
        rectangle.bottom = rectangle.top + height;
    }
    let window = HWND(operation.window as *mut c_void);
    unsafe {
        let _ = SetWindowPos(
            window,
            None,
            rectangle.left,
            rectangle.top,
            rectangle.right - rectangle.left,
            rectangle.bottom - rectangle.top,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        );
    }
}

fn resize_hit_test(window: HWND, pointer: POINT) -> u32 {
    let mut rectangle = RECT::default();
    if unsafe { GetWindowRect(window, &mut rectangle) }.is_err() {
        return HTBOTTOMRIGHT;
    }
    let width = (rectangle.right - rectangle.left).max(1);
    let height = (rectangle.bottom - rectangle.top).max(1);
    let local_x = pointer.x - rectangle.left;
    let local_y = pointer.y - rectangle.top;
    let horizontal = if local_x < width / 3 {
        -1
    } else if local_x > width * 2 / 3 {
        1
    } else {
        0
    };
    let vertical = if local_y < height / 3 {
        -1
    } else if local_y > height * 2 / 3 {
        1
    } else {
        0
    };
    match (horizontal, vertical) {
        (-1, -1) => HTTOPLEFT,
        (0, -1) => HTTOP,
        (1, -1) => HTTOPRIGHT,
        (-1, 0) => HTLEFT,
        (1, 0) => HTRIGHT,
        (-1, 1) => HTBOTTOMLEFT,
        (0, 1) => HTBOTTOM,
        (1, 1) => HTBOTTOMRIGHT,
        (0, 0) => [
            (local_x, HTLEFT),
            (width - local_x, HTRIGHT),
            (local_y, HTTOP),
            (height - local_y, HTBOTTOM),
        ]
        .into_iter()
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, edge)| edge)
        .unwrap_or(HTBOTTOMRIGHT),
        _ => HTBOTTOMRIGHT,
    }
}

pub fn execute_run_command(command: &str) -> bool {
    if command
        .get(.."ms-settings:".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("ms-settings:"))
    {
        let uri = command.to_owned();
        // Windows can take an unbounded amount of time to activate the packaged Settings app.
        // Waiting here blocks winit's only event thread, which also makes the keyboard and mouse
        // hooks appear wedged. Treat a well-formed Settings URI as submitted and wait off-thread.
        thread::spawn(move || match launch_uri(&uri) {
            Ok(true) => {}
            Ok(false) => eprintln!("Windows declined to launch Settings URI: {uri}"),
            Err(error) => eprintln!("failed to launch Settings URI {uri}: {error}"),
        });
        return true;
    }
    let command: Vec<u16> = command.encode_utf16().chain([0]).collect();
    // SAFETY: The UTF-16 command buffer remains alive through this synchronous call.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(command.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    result.0 as isize > 32
}

pub fn paste_text_if_requested(character: &str) -> Option<String> {
    const VK_CONTROL: i32 = 0x11;
    if !character.eq_ignore_ascii_case("v") || unsafe { GetAsyncKeyState(VK_CONTROL) >= 0 } {
        return None;
    }

    // Returning Some even when the clipboard cannot be read consumes Ctrl+V instead of inserting
    // a literal "v" into the Run command.
    Some(read_clipboard_text().unwrap_or_default())
}

fn read_clipboard_text() -> Option<String> {
    use windows::Win32::Foundation::HGLOBAL;

    unsafe { OpenClipboard(None).ok()? };
    let result = (|| {
        // CF_UNICODETEXT is the standard UTF-16 clipboard format.
        let handle = unsafe { GetClipboardData(13).ok()? };
        let global = HGLOBAL(handle.0);
        let byte_len = unsafe { GlobalSize(global) };
        if byte_len < 2 {
            return None;
        }
        let pointer = unsafe { GlobalLock(global) }.cast::<u16>();
        if pointer.is_null() {
            return None;
        }
        let units = unsafe { std::slice::from_raw_parts(pointer, byte_len / 2) };
        let text_len = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units.len());
        let text = String::from_utf16_lossy(&units[..text_len]);
        let _ = unsafe { GlobalUnlock(global) };
        Some(text.replace(['\r', '\n'], " "))
    })();
    let _ = unsafe { CloseClipboard() };
    result
}

fn launch_uri(uri: &str) -> windows::core::Result<bool> {
    use windows::{
        Win32::System::Com::CLSCTX_LOCAL_SERVER,
        Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
        Win32::UI::Shell::{AO_NONE, ApplicationActivationManager, IApplicationActivationManager},
    };

    unsafe { RoInitialize(RO_INIT_MULTITHREADED)? };
    let result = (|| {
        let manager: IApplicationActivationManager =
            unsafe { CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER)? };
        let arguments: Vec<u16> = uri.encode_utf16().chain([0]).collect();
        let process_id = unsafe {
            manager.ActivateApplication(
                w!("windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel"),
                PCWSTR(arguments.as_ptr()),
                AO_NONE,
            )?
        };
        eprintln!("activated Settings URI {uri} as process {process_id}");
        Ok(process_id != 0)
    })();
    unsafe { RoUninitialize() };
    result
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

pub fn configure_launcher_window(window: &winit::window::Window) -> bool {
    use std::sync::atomic::Ordering;

    let Some(hwnd) = window_hwnd(window) else {
        return false;
    };
    LAUNCHER_WINDOW_HANDLE.store(hwnd.0 as isize, Ordering::Relaxed);
    true
}

pub fn configure_context_menu_window(window: &winit::window::Window) -> bool {
    use std::sync::atomic::Ordering;

    let Some(hwnd) = window_hwnd(window) else {
        return false;
    };
    CONTEXT_MENU_WINDOW_HANDLE.store(hwnd.0 as isize, Ordering::Relaxed);
    true
}

pub fn launcher_has_foreground_focus() -> bool {
    use std::sync::atomic::Ordering;

    let launcher = LAUNCHER_WINDOW_HANDLE.load(Ordering::Relaxed);
    launcher != 0 && unsafe { GetForegroundWindow().0 as isize == launcher }
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
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            (style | WS_EX_TOOLWINDOW.0 | WS_EX_LAYERED.0) as isize,
        );
        let opacity_set = SetLayeredWindowAttributes(hwnd, COLORREF(0), 204, LWA_ALPHA).is_ok();
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
            && opacity_set
    };
    install_tray_host(hwnd);
    let mut appbar = APPBARDATA {
        cbSize: size_of::<APPBARDATA>() as u32,
        hWnd: hwnd,
        uCallbackMessage: PANEL_APPBAR_CALLBACK,
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

pub fn update_panel_fullscreen_state() {
    use std::sync::atomic::Ordering;

    let panel = HWND(PANEL_WINDOW_HANDLE.load(Ordering::Relaxed) as *mut c_void);
    if panel.0.is_null() {
        return;
    }
    let foreground = unsafe { GetForegroundWindow() };
    let fullscreen = foreground.0 != panel.0
        && foreground.0 != std::ptr::null_mut()
        && unsafe { IsWindowVisible(foreground).as_bool() }
        && !unsafe { IsIconic(foreground).as_bool() }
        // Standard maximized windows can report monitor-sized outer bounds because GetWindowRect
        // includes their invisible resize frame. Borderless fullscreen windows do not carry the
        // maximized state, so this separates the two without relying on Explorer's work area.
        && !unsafe { IsZoomed(foreground).as_bool() }
        && is_foreign_process_window(foreground)
        && window_covers_monitor(foreground);
    let previous = PANEL_FULLSCREEN_ACTIVE.swap(fullscreen, Ordering::Relaxed);
    let positioned = apply_panel_fullscreen_state(panel, fullscreen);
    if let Err(error) = positioned {
        tracing::warn!(
            fullscreen,
            panel = panel.0 as usize,
            %error,
            "failed to update panel borderless-fullscreen Z-order"
        );
    } else if previous != fullscreen {
        tracing::debug!(
            fullscreen,
            panel = panel.0 as usize,
            "updated panel borderless-fullscreen state"
        );
    }
}

fn apply_panel_fullscreen_state(panel: HWND, fullscreen: bool) -> windows::core::Result<()> {
    if fullscreen {
        unsafe {
            let _ = ShowWindow(panel, SW_HIDE);
        }
        return Ok(());
    }
    unsafe {
        let _ = ShowWindow(panel, SW_SHOWNOACTIVATE);
        SetWindowPos(
            panel,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        )
    }
}

fn is_foreign_process_window(window: HWND) -> bool {
    let mut process_id = 0;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut process_id));
    }
    process_id != 0 && process_id != unsafe { GetCurrentProcessId() }
}

fn window_covers_monitor(window: HWND) -> bool {
    let mut window_rect = RECT::default();
    if unsafe { GetWindowRect(window, &mut window_rect) }.is_err() {
        return false;
    }
    let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return false;
    }
    let mut monitor_info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut monitor_info) }.as_bool() {
        return false;
    }
    rectangle_covers(window_rect, monitor_info.rcMonitor, 2)
}

fn rectangle_covers(window: RECT, monitor: RECT, tolerance: i32) -> bool {
    window.left <= monitor.left + tolerance
        && window.top <= monitor.top + tolerance
        && window.right >= monitor.right - tolerance
        && window.bottom >= monitor.bottom - tolerance
}

fn install_tray_host(hwnd: HWND) {
    use std::sync::atomic::Ordering;

    if PANEL_WINDOW_PROC.load(Ordering::Relaxed) != 0 {
        return;
    }
    // SAFETY: hwnd is Nickel's live winit panel. We retain and call its original window procedure
    // for every message except the tray protocol message handled synchronously below.
    let previous = unsafe {
        SetWindowLongPtrW(
            hwnd,
            GWLP_WNDPROC,
            tray_window_proc as *const () as usize as isize,
        )
    };
    if previous == 0 {
        eprintln!("failed to subclass Nickel's Windows tray host");
        return;
    }
    PANEL_WINDOW_HANDLE.store(hwnd.0 as isize, Ordering::Relaxed);
    PANEL_WINDOW_PROC.store(previous, Ordering::Relaxed);
    install_tray_notify_window(hwnd);
    // Applications cache failed Shell_NotifyIcon registrations. Explorer announces taskbar
    // recreation with this registered message, prompting well-behaved clients to add them again.
    // SAFETY: This is an asynchronous broadcast with scalar parameters only.
    unsafe {
        let message = RegisterWindowMessageW(w!("TaskbarCreated"));
        let _ = SendNotifyMessageW(HWND_BROADCAST, message, WPARAM(0), LPARAM(0));
    }
}

fn install_tray_notify_window(parent: HWND) {
    use std::sync::atomic::Ordering;

    if TRAY_NOTIFY_WINDOW_HANDLE.load(Ordering::Relaxed) != 0 {
        return;
    }
    // SAFETY: The class procedure is static for the process lifetime. The child is an invisible
    // protocol endpoint owned by Nickel's live panel window.
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            eprintln!("failed to resolve Nickel's module for the notification-area host");
            return;
        };
        let class = WNDCLASSW {
            hInstance: windows::Win32::Foundation::HINSTANCE(module.0),
            lpszClassName: w!("TrayNotifyWnd"),
            lpfnWndProc: Some(tray_window_proc),
            ..Default::default()
        };
        if RegisterClassW(&raw const class) == 0 {
            eprintln!("failed to register Nickel's TrayNotifyWnd class");
            return;
        }
        let Ok(window) = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class.lpszClassName,
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_CLIPCHILDREN.0 | WS_CLIPSIBLINGS.0),
            0,
            0,
            1,
            1,
            Some(parent),
            None,
            Some(class.hInstance),
            None,
        ) else {
            eprintln!("failed to create Nickel's TrayNotifyWnd child window");
            return;
        };
        TRAY_NOTIFY_WINDOW_HANDLE.store(window.0 as isize, Ordering::Relaxed);
    }
}

unsafe extern "system" fn tray_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    use std::sync::atomic::Ordering;

    if message == PANEL_APPBAR_CALLBACK && wparam.0 == ABN_FULLSCREENAPP_CODE {
        // AppBars are notified when a fullscreen application enters or leaves the foreground.
        // Drop Nickel behind it while it is active, then restore the panel's topmost band. The
        // AppBar reservation remains intact, so ordinary maximized windows still avoid the panel.
        let fullscreen = lparam.0 != 0;
        PANEL_FULLSCREEN_ACTIVE.store(fullscreen, Ordering::Relaxed);
        tracing::debug!(
            fullscreen,
            panel = hwnd.0 as usize,
            "received AppBar fullscreen notification"
        );
        // SAFETY: hwnd is Nickel's live panel HWND and this changes only its Z-order.
        let _ = apply_panel_fullscreen_state(hwnd, fullscreen);
        return LRESULT(0);
    }
    if message == WM_COPYDATA {
        // SAFETY: WM_COPYDATA guarantees the COPYDATASTRUCT and its buffer remain valid for this
        // synchronous call. Bounds and protocol signature are validated before interpretation.
        let copy = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
        if copy.dwData == 1
            && copy.cbData as usize >= size_of::<TrayNotifyData>()
            && !copy.lpData.is_null()
        {
            let data = unsafe { &*(copy.lpData as *const TrayNotifyData) };
            if data.signature == 0x3475_3423_u32 as i32
                && update_tray_icon(data.message, &data.icon)
            {
                return LRESULT(1);
            }
        }
    }
    let previous = PANEL_WINDOW_PROC.load(Ordering::Relaxed);
    let panel = PANEL_WINDOW_HANDLE.load(Ordering::Relaxed);
    if previous != 0 && hwnd.0 as isize == panel {
        // SAFETY: previous is the live winit WNDPROC returned by SetWindowLongPtrW.
        let procedure = unsafe { std::mem::transmute(previous) };
        return unsafe { CallWindowProcW(procedure, hwnd, message, wparam, lparam) };
    }
    // SAFETY: Messages for the private TrayNotifyWnd child use the system default procedure.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn update_tray_icon(message: u32, icon: &TrayNotifyIconData) -> bool {
    let owner = icon.window as isize;
    let id = icon.id;
    let guid = tray_guid(icon);
    if input_trace_enabled() {
        eprintln!(
            "tray receive operation={} owner={owner:#x} id={id} guid={guid:?} flags={:#x} \
             callback={:#x} icon={:#x} state={:#x} state_mask={:#x} version={}",
            tray_operation_name(message),
            icon.flags,
            icon.callback_message,
            icon.icon,
            icon.state,
            icon.state_mask,
            icon.version
        );
    }
    let Ok(mut items) = TRAY_ITEMS.lock() else {
        return false;
    };
    let existing = items
        .iter()
        .position(|item| tray_icon_matches(item, owner, id, guid));
    match message {
        value if value == NIM_ADD.0 => {
            let Some(image) = render_icon(HICON(icon.icon as usize as *mut c_void)) else {
                return false;
            };
            let title = wide_text(&icon.tip);
            let registration = NativeTrayIcon {
                owner,
                id,
                guid,
                callback_message: icon.callback_message,
                version: 0,
                hidden: tray_icon_hidden(icon),
                item: TrayItem {
                    id: guid.map_or_else(
                        || format!("windows:{owner}:{id}"),
                        |guid| format!("windows-guid:{guid:?}"),
                    ),
                    title,
                    icon: image,
                },
            };
            if let Some(index) = existing {
                items[index] = registration;
            } else {
                items.push(registration);
            }
            true
        }
        value if value == NIM_MODIFY.0 => {
            let Some(index) = existing else {
                return false;
            };
            if icon.flags & NIF_MESSAGE.0 != 0 {
                items[index].callback_message = icon.callback_message;
            }
            if icon.flags & NIF_TIP.0 != 0 {
                items[index].item.title = wide_text(&icon.tip);
            }
            if icon.flags & NIF_STATE.0 != 0 {
                items[index].hidden = tray_icon_hidden(icon);
            }
            if icon.flags & NIF_ICON.0 != 0
                && let Some(image) = render_icon(HICON(icon.icon as usize as *mut c_void))
            {
                items[index].item.icon = image;
            }
            true
        }
        value if value == NIM_DELETE.0 => {
            if let Some(index) = existing {
                items.remove(index);
            }
            true
        }
        value if value == NIM_SETVERSION.0 => {
            let Some(index) = existing else {
                return false;
            };
            items[index].version = icon.version;
            true
        }
        _ => false,
    }
}

fn tray_operation_name(message: u32) -> &'static str {
    match message {
        value if value == NIM_ADD.0 => "add",
        value if value == NIM_MODIFY.0 => "modify",
        value if value == NIM_DELETE.0 => "delete",
        value if value == NIM_SETVERSION.0 => "set-version",
        _ => "unknown",
    }
}

fn tray_guid(icon: &TrayNotifyIconData) -> Option<windows::core::GUID> {
    (icon.flags & NIF_GUID.0 != 0 && icon.guid != windows::core::GUID::from_u128(0))
        .then_some(icon.guid)
}

fn tray_icon_hidden(icon: &TrayNotifyIconData) -> bool {
    icon.flags & NIF_STATE.0 != 0
        && icon.state_mask as u32 & NIS_HIDDEN.0 != 0
        && icon.state as u32 & NIS_HIDDEN.0 != 0
}

fn tray_icon_matches(
    item: &NativeTrayIcon,
    owner: isize,
    id: u32,
    guid: Option<windows::core::GUID>,
) -> bool {
    match (item.guid, guid) {
        (Some(existing), Some(incoming)) => existing == incoming,
        (None, None) => item.owner == owner && item.id == id,
        _ => false,
    }
}

fn wide_text(buffer: &[u16]) -> String {
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
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
        let mut icons = TRAY_ITEMS.lock().expect("Windows tray icon lock poisoned");
        icons.retain(|icon| unsafe { IsWindow(Some(HWND(icon.owner as *mut c_void))).as_bool() });
        icons
            .iter()
            .filter(|icon| !icon.hidden)
            .map(|icon| icon.item.clone())
            .collect()
    }
    fn activate(&self, id: &str) {
        self.send_callback(id, WM_LBUTTONDOWN, WM_LBUTTONUP);
    }
    fn context_menu(&self, id: &str) {
        self.send_callback(id, WM_RBUTTONDOWN, WM_RBUTTONUP);
    }
}

impl TrayFeed {
    fn send_callback(&self, id: &str, legacy_down: u32, legacy_up: u32) {
        let icon = TRAY_ITEMS
            .lock()
            .expect("Windows tray icon lock poisoned")
            .iter()
            .find(|icon| icon.item.id == id)
            .cloned();
        let Some(icon) = icon else {
            return;
        };
        if legacy_up == WM_RBUTTONUP {
            unsafe {
                let _ = SetForegroundWindow(HWND(icon.owner as *mut c_void));
            }
        }
        if icon.version == NOTIFYICON_VERSION_4 {
            let mut cursor = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut cursor);
            }
            let message = if legacy_up == WM_RBUTTONUP {
                WM_CONTEXTMENU
            } else {
                NIN_SELECT
            };
            let wparam = WPARAM(((cursor.y as u16 as usize) << 16) | cursor.x as u16 as usize);
            let lparam = LPARAM(((icon.id as u16 as isize) << 16) | message as isize);
            post_tray_callback(&icon, wparam, lparam, message);
        } else {
            let wparam = WPARAM(icon.id as usize);
            post_tray_callback(&icon, wparam, LPARAM(legacy_down as isize), legacy_down);
            post_tray_callback(&icon, wparam, LPARAM(legacy_up as isize), legacy_up);
        }
    }
}

fn post_tray_callback(icon: &NativeTrayIcon, wparam: WPARAM, lparam: LPARAM, event: u32) {
    unsafe {
        let result = PostMessageW(
            Some(HWND(icon.owner as *mut c_void)),
            icon.callback_message,
            wparam,
            lparam,
        );
        if input_trace_enabled() {
            eprintln!(
                "tray send owner={:#x} id={} guid={:?} version={} callback={:#x} \
                 event={event:#x} wparam={:#x} lparam={:#x} result={result:?}",
                icon.owner,
                icon.id,
                icon.guid,
                icon.version,
                icon.callback_message,
                wparam.0,
                lparam.0,
            );
        }
    }
}

pub fn send_shell_command(command: ShellCommand) -> bool {
    use std::sync::atomic::Ordering;

    let (window, action) = match command {
        ShellCommand::Show => {
            let foreground = unsafe { GetForegroundWindow() };
            PREVIOUS_FOREGROUND_WINDOW.store(foreground.0 as isize, Ordering::Relaxed);
            let launcher = LAUNCHER_WINDOW_HANDLE.load(Ordering::Relaxed);
            if launcher == 0 {
                return false;
            }
            let hwnd = HWND(launcher as *mut c_void);
            // SAFETY: The handle belongs to Nickel's live launcher window.
            unsafe {
                let foreground_thread = GetWindowThreadProcessId(foreground, None);
                let current_thread = GetCurrentThreadId();
                let attached = foreground_thread != 0
                    && foreground_thread != current_thread
                    && AttachThreadInput(current_thread, foreground_thread, true).as_bool();
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = BringWindowToTop(hwnd);
                let _ = SetForegroundWindow(hwnd);
                let _ = SetFocus(Some(hwnd));
                if attached {
                    let _ = AttachThreadInput(current_thread, foreground_thread, false);
                }
            }
            return true;
        }
        ShellCommand::Hide => {
            let foreground = unsafe { GetForegroundWindow() };
            let launcher = LAUNCHER_FOREGROUND_WINDOW.load(Ordering::Relaxed);
            RESTORE_LAUNCHER_FOCUS.store(
                launcher != 0 && foreground.0 as isize == launcher,
                Ordering::Relaxed,
            );
            let hwnd = LAUNCHER_WINDOW_HANDLE.load(Ordering::Relaxed);
            if hwnd == 0 {
                return false;
            }
            // SAFETY: The handle belongs to Nickel's live launcher window.
            unsafe {
                let _ = ShowWindow(HWND(hwnd as *mut c_void), SW_HIDE);
            }
            return true;
        }
        ShellCommand::ShowContextMenu { x, width, height } => {
            clear_dwm_thumbnails();
            return show_context_window(x, width, height);
        }
        ShellCommand::ShowPreview {
            x,
            width,
            height,
            windows,
        } => {
            if !show_context_window(x, width, height) {
                return false;
            }
            return show_dwm_previews(&windows);
        }
        ShellCommand::HideContextMenu => {
            clear_dwm_thumbnails();
            let context = CONTEXT_MENU_WINDOW_HANDLE.load(Ordering::Relaxed);
            if context == 0 {
                return false;
            }
            unsafe {
                let _ = ShowWindow(HWND(context as *mut c_void), SW_HIDE);
            }
            return true;
        }
        ShellCommand::WindowAction { window, action } => (window, action),
        _ => return false,
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
                let foreground = GetForegroundWindow();
                let current_thread = GetCurrentThreadId();
                let foreground_thread = GetWindowThreadProcessId(foreground, None);
                let target_thread = GetWindowThreadProcessId(hwnd, None);
                let attached_foreground = foreground_thread != 0
                    && foreground_thread != current_thread
                    && AttachThreadInput(current_thread, foreground_thread, true).as_bool();
                let attached_target = target_thread != 0
                    && target_thread != current_thread
                    && target_thread != foreground_thread
                    && AttachThreadInput(current_thread, target_thread, true).as_bool();
                let _ = BringWindowToTop(hwnd);
                let activated = SetForegroundWindow(hwnd).as_bool();
                if attached_target {
                    let _ = AttachThreadInput(current_thread, target_thread, false);
                }
                if attached_foreground {
                    let _ = AttachThreadInput(current_thread, foreground_thread, false);
                }
                if input_trace_enabled() {
                    eprintln!(
                        "input source=activate target={:?} previous={:?} current={:?} result={activated}",
                        hwnd.0,
                        foreground.0,
                        GetForegroundWindow().0
                    );
                }
                activated
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
                park_iconic_window(hwnd);
                true
            }
        }
    }
}

fn show_context_window(x: i32, width: i32, height: i32) -> bool {
    use std::sync::atomic::Ordering;

    let context = CONTEXT_MENU_WINDOW_HANDLE.load(Ordering::Relaxed);
    if context == 0 {
        return false;
    }
    let hwnd = HWND(context as *mut c_void);
    let mut work_area = RECT::default();
    unsafe {
        if SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some((&mut work_area as *mut RECT).cast()),
            Default::default(),
        )
        .is_err()
        {
            return false;
        }
        let max_x = (work_area.right - width).max(work_area.left);
        let left = x.clamp(work_area.left, max_x);
        let top = (work_area.bottom - height).max(work_area.top);
        if SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            left,
            top,
            width,
            height,
            SWP_NOACTIVATE,
        )
        .is_err()
        {
            return false;
        }
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    true
}

fn show_dwm_previews(windows: &[WindowId]) -> bool {
    use std::sync::atomic::Ordering;

    clear_dwm_thumbnails();
    let destination = CONTEXT_MENU_WINDOW_HANDLE.load(Ordering::Relaxed);
    if destination == 0 {
        return false;
    }
    let destination = HWND(destination as *mut c_void);
    let mut registered = Vec::new();
    for (index, window) in windows.iter().enumerate() {
        let source = hwnd(*window);
        let Ok(thumbnail) = (unsafe { DwmRegisterThumbnail(destination, source) }) else {
            continue;
        };
        let left = 10 + index as i32 * crate::context_menu::PREVIEW_CARD_WIDTH as i32;
        let properties = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION
                | DWM_TNP_OPACITY
                | DWM_TNP_VISIBLE
                | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: RECT {
                left,
                top: 44,
                right: left + 240,
                bottom: 179,
            },
            opacity: 255,
            fVisible: BOOL(1),
            fSourceClientAreaOnly: BOOL(1),
            ..Default::default()
        };
        if unsafe { DwmUpdateThumbnailProperties(thumbnail, &properties) }.is_ok() {
            registered.push(thumbnail);
        } else {
            unsafe {
                let _ = DwmUnregisterThumbnail(thumbnail);
            }
        }
    }
    let success = registered.len() == windows.len();
    if let Ok(mut thumbnails) = DWM_THUMBNAILS.lock() {
        *thumbnails = registered;
    }
    success
}

fn clear_dwm_thumbnails() {
    let Ok(mut thumbnails) = DWM_THUMBNAILS.lock() else {
        return;
    };
    for thumbnail in thumbnails.drain(..) {
        unsafe {
            let _ = DwmUnregisterThumbnail(thumbnail);
        }
    }
}

pub fn launcher_visibility_applied(visible: bool) {
    use std::sync::atomic::Ordering;

    if let Ok(mut controller) = hotkey_controller().lock() {
        controller.launcher_visibility_applied(visible);
    }
    if visible {
        let foreground = unsafe { GetForegroundWindow() };
        LAUNCHER_FOREGROUND_WINDOW.store(foreground.0 as isize, Ordering::Relaxed);
        return;
    }
    LAUNCHER_FOREGROUND_WINDOW.store(0, Ordering::Relaxed);
    if !RESTORE_LAUNCHER_FOCUS.swap(false, Ordering::Relaxed) {
        return;
    }
    let previous = PREVIOUS_FOREGROUND_WINDOW.swap(0, Ordering::Relaxed);
    if previous == 0 {
        return;
    }
    let hwnd = HWND(previous as *mut c_void);
    // SAFETY: The handle was captured from GetForegroundWindow and is revalidated before use.
    unsafe {
        if IsWindow(Some(hwnd)).as_bool() {
            let _ = SetForegroundWindow(hwnd);
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

    pub fn preview(&self, window: WindowId) -> Option<WindowPreview> {
        unsafe { IsWindow(Some(hwnd(window))).as_bool() }.then(|| WindowPreview {
            window,
            image: image::RgbaImage::new(1, 1),
        })
    }

    pub fn supports_previews(&self) -> bool {
        true
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
    if unsafe { IsIconic(hwnd).as_bool() } {
        park_iconic_window(hwnd);
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

fn park_iconic_window(hwnd: HWND) {
    // Explorer normally conceals the legacy iconic HWND representation. In an Explorer-free
    // session, Windows places that tiny minimized titlebar on the desktop. Moving only the
    // iconic representation off-screen preserves WINDOWPLACEMENT.rcNormalPosition for restore.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            -32_000,
            -32_000,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        );
    }
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
    use windows::Win32::Foundation::RECT;

    use super::{executable_icon, is_shell_infrastructure, rectangle_covers};

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

    #[test]
    fn borderless_window_covering_monitor_is_fullscreen() {
        let monitor = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };

        assert!(rectangle_covers(monitor, monitor, 2));
        assert!(rectangle_covers(
            RECT {
                left: -1,
                top: -1,
                right: 1921,
                bottom: 1081,
            },
            monitor,
            2,
        ));
    }

    #[test]
    fn maximized_window_respecting_panel_is_not_fullscreen() {
        assert!(!rectangle_covers(
            RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1024,
            },
            RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            2,
        ));
    }
}
