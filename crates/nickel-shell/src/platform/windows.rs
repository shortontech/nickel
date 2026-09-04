use std::{
    env,
    ffi::c_void,
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::Ordering,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use windows::{
    Win32::{
        Foundation::{
            COLORREF, CloseHandle, GlobalFree, HANDLE, HWND, LPARAM, LRESULT, LocalFree, POINT,
            RECT, SIZE, WPARAM,
        },
        Graphics::Dwm::{
            DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
            DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE, DWM_WINDOW_CORNER_PREFERENCE,
            DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmQueryThumbnailSourceSize,
            DwmRegisterThumbnail, DwmSetWindowAttribute, DwmUnregisterThumbnail,
            DwmUpdateThumbnailProperties,
        },
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap,
            CreateCompatibleDC, CreateDIBSection, DEVMODEW, DIB_RGB_COLORS, DeleteDC, DeleteObject,
            ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW, GetDC, GetMonitorInfoW, HGDIOBJ,
            MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW, MonitorFromWindow, ReleaseDC,
            SRCCOPY, SelectObject,
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
            DataExchange::{
                COPYDATASTRUCT, CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard,
                SetClipboardData,
            },
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
        },
        UI::{
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, GetCapture, MOD_NOREPEAT, MOD_WIN, RegisterHotKey,
                ReleaseCapture, SetCapture, SetFocus,
            },
            Shell::{
                ABE_BOTTOM, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA,
                CommandLineToArgvW, DWPOS_CENTER, DWPOS_FILL, DWPOS_FIT, DWPOS_SPAN, DWPOS_STRETCH,
                DWPOS_TILE, DesktopWallpaper, IDesktopWallpaper, NIF_GUID, NIF_ICON, NIF_MESSAGE,
                NIF_STATE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT,
                NIS_HIDDEN, NOTIFYICON_VERSION_4, SHAppBarMessage, SHFILEINFOW, SHGFI_ICON,
                SHGetFileInfoW, ShellExecuteW,
            },
            WindowsAndMessaging::{
                BringWindowToTop, CallNextHookEx, CallWindowProcW, CopyImage, CreateWindowExW,
                DI_NORMAL, DefWindowProcW, DestroyIcon, DrawIconEx, EnumWindows, GA_ROOT,
                GA_ROOTOWNER, GCLP_HICON, GCLP_HICONSM, GWL_EXSTYLE, GWLP_WNDPROC, GetAncestor,
                GetClassLongPtrW, GetClassNameW, GetClientRect, GetCursorPos, GetForegroundWindow,
                GetLastActivePopup, GetMessageW, GetSystemMenu, GetSystemMetrics,
                GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
                GetWindowThreadProcessId, HICON, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTLEFT,
                HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, HWND_BOTTOM, HWND_BROADCAST, HWND_TOPMOST,
                IMAGE_ICON, IsIconic, IsWindow, IsWindowVisible, IsZoomed, KBDLLHOOKSTRUCT,
                KillTimer, LR_COPYFROMRESOURCE, LWA_ALPHA, MSG, MSLLHOOKSTRUCT, PostMessageW,
                RegisterClassW, RegisterShellHookWindow, RegisterWindowMessageW, SM_CXICON,
                SM_CYICON, SPI_GETWORKAREA, SPI_SETWORKAREA, SPIF_SENDCHANGE, SW_HIDE, SW_MAXIMIZE,
                SW_MINIMIZE, SW_RESTORE, SW_SHOW, SW_SHOWNOACTIVATE, SW_SHOWNORMAL,
                SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
                SWP_NOZORDER, SendNotifyMessageW, SetForegroundWindow, SetLayeredWindowAttributes,
                SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowsHookExW, ShowWindow,
                SystemParametersInfoW, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
                WH_KEYBOARD_LL, WH_MOUSE_LL, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE,
                WM_CONTEXTMENU, WM_COPYDATA, WM_HOTKEY, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
                WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSCOMMAND,
                WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN,
                WS_CLIPSIBLINGS, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
                WS_EX_TOOLWINDOW, WS_POPUP, WindowFromPoint,
            },
        },
    },
    core::{BOOL, PCWSTR, PWSTR, w},
};

use nickel_core::hotkeys::{
    HotkeyAction, HotkeyController, HotkeyOutcome, HotkeySnapshot, KeyCode, KeyEdge,
};
use nickel_input::{
    AggregateModifier, PhysicalKey, Shortcut, ShortcutKey, ShortcutTrigger,
    global::{GlobalShortcutEdge, Registration, RegistrationError, RegistrationTable},
};

use crate::{
    desktop::{Wallpaper, WallpaperPosition},
    launcher::Launcher,
    model::{
        Application, ApplicationDiscovery, ApplicationId, OpenWindow, TrayItem, WindowId,
        WindowPreview,
    },
    platform::{
        DesktopCapture, FeedState, GlobalShortcut, LaunchError, NotificationSource,
        ScreenshotAction, ShellCommand, TraySource, WindowAction,
    },
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

pub fn capture_active_window() -> Result<(), String> {
    const CF_BITMAP: u32 = 2;
    let window = unsafe { GetForegroundWindow() };
    if window.0.is_null() {
        return Err("Windows reported no foreground window".into());
    }
    let mut bounds = RECT::default();
    unsafe { GetWindowRect(window, &raw mut bounds) }
        .map_err(|error| format!("could not read active-window bounds: {error}"))?;
    let width = bounds.right - bounds.left;
    let height = bounds.bottom - bounds.top;
    if width <= 0 || height <= 0 {
        return Err("active window has empty bounds".into());
    }

    // SAFETY: The screen and memory device contexts are released on every path. Once
    // SetClipboardData succeeds, Windows owns the bitmap and Nickel must not delete it.
    unsafe {
        let screen = GetDC(None);
        if screen.0.is_null() {
            return Err("could not acquire the screen device context".into());
        }
        let memory = CreateCompatibleDC(Some(screen));
        if memory.0.is_null() {
            ReleaseDC(None, screen);
            return Err("could not create the screenshot device context".into());
        }
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        if bitmap.0.is_null() {
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
            return Err("could not allocate the screenshot bitmap".into());
        }
        let previous = SelectObject(memory, HGDIOBJ(bitmap.0));
        let copied = BitBlt(
            memory,
            0,
            0,
            width,
            height,
            Some(screen),
            bounds.left,
            bounds.top,
            SRCCOPY,
        );
        SelectObject(memory, previous);
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        if let Err(error) = copied {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            return Err(format!("could not copy active-window pixels: {error}"));
        }

        if let Err(error) = OpenClipboard(None) {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            return Err(format!("could not open the clipboard: {error}"));
        }
        let clipboard_result = EmptyClipboard()
            .and_then(|()| SetClipboardData(CF_BITMAP, Some(HANDLE(bitmap.0))).map(|_| ()));
        let _ = CloseClipboard();
        if let Err(error) = clipboard_result {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            return Err(format!(
                "could not place screenshot on the clipboard: {error}"
            ));
        }
    }
    tracing::info!(width, height, "captured active window to clipboard");
    Ok(())
}

pub fn capture_desktop() -> Result<DesktopCapture, String> {
    let foreground = unsafe { GetForegroundWindow() };
    let monitor = unsafe { MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &raw mut info) }.as_bool() {
        return Err("could not read monitor bounds".into());
    }
    Ok(DesktopCapture {
        image: capture_rect_rgba(info.rcMonitor)?,
    })
}

pub fn copy_image_to_clipboard(image: &image::RgbaImage) -> Result<(), String> {
    const CF_BITMAP: u32 = 2;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: image.width() as i32,
            biHeight: -(image.height() as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        let mut pixels = std::ptr::null_mut::<c_void>();
        let bitmap = CreateDIBSection(
            None,
            &raw const info,
            DIB_RGB_COLORS,
            &raw mut pixels,
            None,
            0,
        )
        .map_err(|error| format!("could not allocate clipboard image: {error}"))?;
        let bgra = std::slice::from_raw_parts_mut(pixels.cast::<u8>(), image.as_raw().len());
        for (source, target) in image.as_raw().chunks_exact(4).zip(bgra.chunks_exact_mut(4)) {
            target.copy_from_slice(&[source[2], source[1], source[0], 255]);
        }
        if let Err(error) = OpenClipboard(None) {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            return Err(format!("could not open clipboard: {error}"));
        }
        let result = EmptyClipboard()
            .and_then(|()| SetClipboardData(CF_BITMAP, Some(HANDLE(bitmap.0))).map(|_| ()));
        let _ = CloseClipboard();
        if let Err(error) = result {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            return Err(format!("could not copy image: {error}"));
        }
    }
    Ok(())
}

pub fn copy_temp_image_path(image: &image::RgbaImage) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = env::temp_dir().join(format!("nickel-crop-{stamp}.png"));
    image
        .save(&path)
        .map_err(|error| format!("could not save temporary screenshot: {error}"))?;
    set_clipboard_text(&path.to_string_lossy())?;
    Ok(path)
}

fn capture_rect_rgba(bounds: RECT) -> Result<image::RgbaImage, String> {
    let width = bounds.right - bounds.left;
    let height = bounds.bottom - bounds.top;
    if width <= 0 || height <= 0 {
        return Err("capture rectangle is empty".into());
    }
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
    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let mut pixels = std::ptr::null_mut::<c_void>();
        let bitmap = CreateDIBSection(
            Some(screen),
            &raw const info,
            DIB_RGB_COLORS,
            &raw mut pixels,
            None,
            0,
        )
        .map_err(|error| format!("could not allocate capture: {error}"))?;
        let previous = SelectObject(memory, HGDIOBJ(bitmap.0));
        let copied = BitBlt(
            memory,
            0,
            0,
            width,
            height,
            Some(screen),
            bounds.left,
            bounds.top,
            SRCCOPY,
        );
        let mut rgba = vec![0; width as usize * height as usize * 4];
        if copied.is_ok() {
            let bgra = std::slice::from_raw_parts(pixels.cast::<u8>(), rgba.len());
            for (source, target) in bgra.chunks_exact(4).zip(rgba.chunks_exact_mut(4)) {
                target.copy_from_slice(&[source[2], source[1], source[0], 255]);
            }
        }
        SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        copied.map_err(|error| format!("could not capture desktop: {error}"))?;
        image::RgbaImage::from_raw(width as u32, height as u32, rgba)
            .ok_or_else(|| "could not construct desktop capture".into())
    }
}

pub fn capture_active_window_to_file() -> Result<(), String> {
    let path = save_active_window_to_temp()?;
    set_clipboard_text(&path.to_string_lossy())?;
    tracing::info!(path = %path.display(), "copied temporary screenshot path");
    Ok(())
}

fn save_active_window_to_temp() -> Result<PathBuf, String> {
    let window = unsafe { GetForegroundWindow() };
    if window.0.is_null() {
        return Err("Windows reported no foreground window".into());
    }
    let mut bounds = RECT::default();
    unsafe { GetWindowRect(window, &raw mut bounds) }
        .map_err(|error| format!("could not read active-window bounds: {error}"))?;
    let width = bounds.right - bounds.left;
    let height = bounds.bottom - bounds.top;
    if width <= 0 || height <= 0 {
        return Err("active window has empty bounds".into());
    }

    let mut pixels = std::ptr::null_mut::<c_void>();
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
    let rgba = unsafe {
        let screen = GetDC(None);
        if screen.0.is_null() {
            return Err("could not acquire the screen device context".into());
        }
        let memory = CreateCompatibleDC(Some(screen));
        if memory.0.is_null() {
            ReleaseDC(None, screen);
            return Err("could not create the screenshot device context".into());
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
            Err(error) => {
                let _ = DeleteDC(memory);
                ReleaseDC(None, screen);
                return Err(format!("could not allocate screenshot pixels: {error}"));
            }
        };
        let previous = SelectObject(memory, HGDIOBJ(bitmap.0));
        let copied = BitBlt(
            memory,
            0,
            0,
            width,
            height,
            Some(screen),
            bounds.left,
            bounds.top,
            SRCCOPY,
        );
        let mut rgba = vec![0_u8; width as usize * height as usize * 4];
        if copied.is_ok() && !pixels.is_null() {
            let bgra = std::slice::from_raw_parts(pixels.cast::<u8>(), rgba.len());
            for (source, target) in bgra.chunks_exact(4).zip(rgba.chunks_exact_mut(4)) {
                target.copy_from_slice(&[source[2], source[1], source[0], 255]);
            }
        }
        SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        copied.map_err(|error| format!("could not copy active-window pixels: {error}"))?;
        rgba
    };

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = env::temp_dir().join(format!("nickel-window-{stamp}.png"));
    let image = image::RgbaImage::from_raw(width as u32, height as u32, rgba)
        .ok_or_else(|| "could not construct the screenshot image".to_string())?;
    image
        .save(&path)
        .map_err(|error| format!("could not save temporary screenshot: {error}"))?;
    tracing::info!(path = %path.display(), width, height, "captured active window to a temporary file");
    Ok(path)
}

fn set_clipboard_text(text: &str) -> Result<(), String> {
    const CF_UNICODETEXT: u32 = 13;
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, wide.len() * std::mem::size_of::<u16>())
            .map_err(|error| format!("could not allocate clipboard text: {error}"))?;
        let destination = GlobalLock(memory).cast::<u16>();
        if destination.is_null() {
            let _ = GlobalFree(Some(memory));
            return Err("could not lock clipboard text memory".into());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), destination, wide.len());
        let _ = GlobalUnlock(memory);
        if let Err(error) = OpenClipboard(None) {
            let _ = GlobalFree(Some(memory));
            return Err(format!("could not open the clipboard: {error}"));
        }
        let result = EmptyClipboard()
            .and_then(|()| SetClipboardData(CF_UNICODETEXT, Some(HANDLE(memory.0))).map(|_| ()));
        let _ = CloseClipboard();
        if let Err(error) = result {
            let _ = GlobalFree(Some(memory));
            return Err(format!(
                "could not put the screenshot path on the clipboard: {error}"
            ));
        }
    }
    Ok(())
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

pub fn application_discovery() -> ApplicationDiscovery {
    ApplicationDiscovery::ready(applications())
}

pub fn application_icon(reference: &str) -> Option<image::RgbaImage> {
    nickel_platform::path_icon(PathBuf::from(reference).as_path())
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

pub fn set_wifi_enabled(_enabled: bool) -> bool {
    false
}

pub fn activate_wifi_network(_id: &str) -> bool {
    false
}

pub fn bluetooth_status() -> super::BluetoothStatus {
    super::BluetoothStatus::default()
}

pub fn set_bluetooth_powered(_powered: bool) -> bool {
    false
}

pub fn set_bluetooth_discovery(_discovering: bool) -> bool {
    false
}

pub fn toggle_bluetooth_device(_id: &str) -> bool {
    false
}

pub fn audio_status() -> super::AudioStatus {
    use windows::Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Media::Audio::{
            DEVICE_STATE_ACTIVE, Endpoints::IAudioEndpointVolume, IMMDevice, IMMDeviceEnumerator,
            MMDeviceEnumerator, eMultimedia, eRender,
        },
        System::Com::{CLSCTX_ALL, STGM_READ, StructuredStorage::PropVariantToString},
    };

    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| -> windows::core::Result<super::AudioStatus> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let default_device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
            let default_id = take_com_string(default_device.GetId()?);
            let endpoint: IAudioEndpointVolume = default_device.Activate(CLSCTX_ALL, None)?;
            let volume_percent = (endpoint.GetMasterVolumeLevelScalar()? * 100.0)
                .round()
                .clamp(0.0, 100.0) as u8;
            let muted = endpoint.GetMute()?.as_bool();
            let collection = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            let mut devices = Vec::new();
            for index in 0..collection.GetCount()? {
                let device: IMMDevice = collection.Item(index)?;
                let id = take_com_string(device.GetId()?);
                let store = device.OpenPropertyStore(STGM_READ)?;
                let value = store.GetValue(&PKEY_Device_FriendlyName)?;
                let mut name_buffer = [0_u16; 512];
                let name = if PropVariantToString(&raw const value, &mut name_buffer).is_ok() {
                    String::from_utf16_lossy(
                        &name_buffer[..name_buffer
                            .iter()
                            .position(|unit| *unit == 0)
                            .unwrap_or(name_buffer.len())],
                    )
                } else {
                    id.clone()
                };
                devices.push(super::AudioDeviceStatus {
                    is_default: id == default_id,
                    id,
                    name,
                });
            }
            devices.sort_by(|left, right| {
                right
                    .is_default
                    .cmp(&left.is_default)
                    .then_with(|| left.name.cmp(&right.name))
            });
            Ok(super::AudioStatus {
                available: true,
                devices,
                volume_percent,
                muted,
            })
        })();
        if initialized {
            CoUninitialize();
        }
        result.unwrap_or_default()
    }
}

pub fn set_audio_volume(volume_percent: u8) -> bool {
    use windows::Win32::{
        Media::Audio::{
            Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator, MMDeviceEnumerator, eMultimedia,
            eRender,
        },
        System::Com::CLSCTX_ALL,
    };

    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| -> windows::core::Result<()> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
            let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
            endpoint.SetMasterVolumeLevelScalar(
                f32::from(volume_percent.min(100)) / 100.0,
                std::ptr::null(),
            )
        })();
        if initialized {
            CoUninitialize();
        }
        result.is_ok()
    }
}

pub fn handle_consumer_control(_control: nickel_session_protocol::ConsumerControl) {
    // Windows owns consumer controls through WM_APPCOMMAND. Winit delivery must not
    // apply the same physical action a second time.
}

pub fn capture_pointer(window: &impl raw_window_handle::HasWindowHandle) -> bool {
    let Some(hwnd) = window_hwnd(window) else {
        return false;
    };
    // SAFETY: `hwnd` belongs to the live window borrowed from the caller.
    unsafe {
        let _ = SetCapture(hwnd);
        GetCapture() == hwnd
    }
}

pub fn release_pointer() {
    // SAFETY: releasing capture is valid even if this thread owns no capture.
    let _ = unsafe { ReleaseCapture() };
}

pub fn select_audio_device(id: &str) -> bool {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::{
        Media::Audio::{eCommunications, eConsole, eMultimedia},
        System::Com::CLSCTX_ALL,
    };

    let wide: Vec<_> = std::ffi::OsStr::new(id)
        .encode_wide()
        .chain(Some(0))
        .collect();
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| -> windows::core::Result<()> {
            let policy: IPolicyConfig = CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)?;
            for role in [eConsole, eMultimedia, eCommunications] {
                (windows::core::Interface::vtable(&policy).SetDefaultEndpoint)(
                    windows::core::Interface::as_raw(&policy),
                    windows::core::PCWSTR(wide.as_ptr()),
                    role,
                )
                .ok()?;
            }
            Ok(())
        })();
        if initialized {
            CoUninitialize();
        }
        result.is_ok()
    }
}

unsafe fn take_com_string(value: windows::core::PWSTR) -> String {
    let text = unsafe { value.to_string() }.unwrap_or_default();
    unsafe {
        CoTaskMemFree(Some(value.as_ptr().cast()));
    }
    text
}

const POLICY_CONFIG_CLIENT: windows::core::GUID =
    windows::core::GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

windows::core::imp::define_interface!(
    IPolicyConfig,
    IPolicyConfig_Vtbl,
    0x568b9108_44bf_40b4_9006_86afe5b5a620
);
windows::core::imp::interface_hierarchy!(IPolicyConfig, windows::core::IUnknown);

#[repr(C)]
#[allow(non_snake_case)]
pub struct IPolicyConfig_Vtbl {
    base__: windows::core::IUnknown_Vtbl,
    GetMixFormat: usize,
    GetDeviceFormat: usize,
    ResetDeviceFormat: usize,
    SetDeviceFormat: usize,
    GetProcessingPeriod: usize,
    SetProcessingPeriod: usize,
    GetShareMode: usize,
    SetShareMode: usize,
    GetPropertyValue: usize,
    SetPropertyValue: usize,
    SetDefaultEndpoint: unsafe extern "system" fn(
        *mut c_void,
        windows::core::PCWSTR,
        windows::Win32::Media::Audio::ERole,
    ) -> windows::core::HRESULT,
    SetEndpointVisibility: usize,
}

pub fn launcher_hotkey_receiver() -> super::GlobalShortcutFeed {
    let (sender, receiver) = mpsc::channel();
    let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
    let capability = match thread::Builder::new()
        .name("nickel-super-key".into())
        .spawn(move || run_super_key_hook(sender, startup_sender))
    {
        Ok(_) => startup_receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|error| {
                nickel_input::global::ShortcutCapability::Unavailable(
                    nickel_input::global::UnavailableReason::Backend(format!(
                        "Windows shortcut adapter did not initialize: {error}"
                    )),
                )
            }),
        Err(error) => nickel_input::global::ShortcutCapability::Unavailable(
            nickel_input::global::UnavailableReason::Backend(format!(
                "could not start Windows shortcut adapter: {error}"
            )),
        ),
    };
    super::GlobalShortcutFeed {
        receiver,
        ownership: nickel_input::global::ShortcutOwnership::OperatingSystem,
        capability,
    }
}

pub fn handle_focused_shortcut(key: KeyCode, edge: KeyEdge) {
    let action = hotkey_controller()
        .lock()
        .ok()
        .and_then(|mut controller| controller.handle_reconciled(key, edge).action);
    send_hotkey_action(action);
}

fn run_super_key_hook(
    sender: Sender<GlobalShortcut>,
    startup: mpsc::SyncSender<nickel_input::global::ShortcutCapability>,
) {
    SHORTCUT_SENDER.set(sender).ok();
    let run = native_hotkey_requests()[0];
    let modifiers = MOD_WIN | MOD_NOREPEAT;
    let run_registered =
        unsafe { RegisterHotKey(None, run.id, modifiers, run.virtual_key) }.is_ok();
    let mut registrations = RegistrationTable::default();
    let run_registration = native_registration(
        &mut registrations,
        run_registered,
        run.key,
        [AggregateModifier::Super],
        run.action,
    );
    RUN_HOTKEY_REGISTERED.store(run_registered, std::sync::atomic::Ordering::Release);
    if !run_registered {
        tracing::warn!("Super+R registration unavailable; using low-level hook fallback");
    }

    // SAFETY: The callback remains valid for the process lifetime and this thread owns the
    // message loop required by a low-level keyboard hook.
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(super_key_hook), None, 0) };
    let _hook = match hook {
        Ok(hook) => hook,
        Err(error) => {
            let _ = startup.send(nickel_input::global::ShortcutCapability::Unavailable(
                nickel_input::global::UnavailableReason::Backend(format!(
                    "failed to register the Windows keyboard hook: {error}"
                )),
            ));
            return;
        }
    };
    let mouse_hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(windows_mouse_hook), None, 0) };
    let _mouse_hook = match mouse_hook {
        Ok(hook) => Some(hook),
        Err(error) => {
            tracing::warn!(%error, "failed to register Nickel's Windows mouse chord hook");
            None
        }
    };
    let _ = startup.send(nickel_input::global::ShortcutCapability::Available);
    let mut message = MSG::default();
    // SAFETY: message is valid writable storage for each synchronous call.
    while unsafe { GetMessageW(&mut message, None, 0, 0).as_bool() } {
        if message.message == WM_HOTKEY {
            match message.wParam.0 as i32 {
                id if id == run.id => {
                    deliver_registered_hotkey(
                        &mut registrations,
                        run_registration,
                        GlobalShortcutEdge::Activated,
                    );
                }
                _ => {}
            }
        } else if message.message == WM_TIMER {
            reconcile_modifier_release(message.wParam.0);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RegisteredHotkey {
    ShowRun,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NativeHotkeyRequest {
    id: i32,
    virtual_key: u32,
    key: KeyCode,
    action: RegisteredHotkey,
}

fn native_hotkey_requests() -> [NativeHotkeyRequest; 1] {
    [NativeHotkeyRequest {
        id: 0x4e03,
        virtual_key: 0x52,
        key: KeyCode::KeyR,
        action: RegisteredHotkey::ShowRun,
    }]
}

fn native_registration(
    registrations: &mut RegistrationTable<RegisteredHotkey>,
    native_registered: bool,
    key: KeyCode,
    modifiers: impl IntoIterator<Item = AggregateModifier>,
    action: RegisteredHotkey,
) -> Option<nickel_input::global::RegistrationId> {
    if !native_registered {
        let error =
            RegistrationError::Backend(format!("RegisterHotKey rejected {key:?} for {action:?}"));
        tracing::warn!(?error, "global shortcut registration unavailable");
        return None;
    }
    match registrations.register(Registration {
        shortcut: Shortcut {
            key: ShortcutKey::Physical(PhysicalKey::Code(key)),
            modifiers: modifiers.into_iter().collect(),
            trigger: ShortcutTrigger::Pressed,
        },
        action,
    }) {
        Ok(id) => Some(id),
        Err(error) => {
            tracing::warn!(?error, "global shortcut registration conflict");
            None
        }
    }
}

fn deliver_registered_hotkey(
    registrations: &mut RegistrationTable<RegisteredHotkey>,
    id: Option<nickel_input::global::RegistrationId>,
    edge: GlobalShortcutEdge,
) {
    let Some(event) = id.and_then(|id| registrations.deliver(id, edge)) else {
        return;
    };
    match event.action {
        RegisteredHotkey::ShowRun => {
            tracing::debug!("Super+R hotkey received");
            if let Some(sender) = SHORTCUT_SENDER.get() {
                let _ = sender.send(GlobalShortcut::ShowRun);
            }
        }
    }
}

static SHORTCUT_SENDER: std::sync::OnceLock<Sender<GlobalShortcut>> = std::sync::OnceLock::new();
static HOTKEY_CONTROLLER: std::sync::OnceLock<Mutex<HotkeyController>> = std::sync::OnceLock::new();
static RUN_HOTKEY_REGISTERED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
const SUPER_RELEASE_TIMER: usize = 0x4e04;
const ALT_RELEASE_TIMER: usize = 0x4e05;
static SUPER_RELEASE_TIMER_ID: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
static ALT_RELEASE_TIMER_ID: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
static INPUT_TRACE_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static PANEL_FULLSCREEN_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static ORIGINAL_WORK_AREA: std::sync::Mutex<Option<RECT>> = std::sync::Mutex::new(None);
static TRAY_ITEMS: Mutex<Vec<NativeTrayIcon>> = Mutex::new(Vec::new());
static PANEL_WINDOW_PROC: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static PANEL_WINDOW_HANDLE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static SHELL_HOOK_MESSAGE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static TRAY_NOTIFY_WINDOW_HANDLE: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(0);
static SHELL_TRAY_WINDOW_HANDLE: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(0);
static PREVIOUS_FOREGROUND_WINDOW: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(0);
static LAUNCHER_FOREGROUND_WINDOW: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(0);
static LAUNCHER_WINDOW_HANDLE: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(0);
static PREVIEW_WINDOW_HANDLE: std::sync::atomic::AtomicIsize =
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
struct TrayNotifyIconData {
    cb_size: u32,
    window: u32,
    id: u32,
    flags: u32,
    callback_message: u32,
    icon: u32,
    tip: [u16; 128],
    state: u32,
    state_mask: u32,
    info: [u16; 256],
    version: u32,
    info_title: [u16; 64],
    info_flags: u32,
    guid: windows::core::GUID,
    balloon_icon: u32,
}

unsafe extern "system" fn super_key_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    const VK_R: u32 = 0x52;
    const VK_MENU: u32 = 0x12;

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
    // Alt changes the layout-translated virtual key for the physical grave key on some layouts
    // (for example to VK_HANJA). Preserve the physical shortcut using its stable scan code.
    let key = key_code_from_windows_vk(event.vkCode)
        .or_else(|| (event.scanCode == 0x29).then_some(KeyCode::Backquote));
    if edge == KeyEdge::Released {
        let release_timer = match key {
            Some(KeyCode::SuperLeft | KeyCode::SuperRight) => {
                Some((SUPER_RELEASE_TIMER, &SUPER_RELEASE_TIMER_ID))
            }
            Some(KeyCode::AltLeft | KeyCode::AltRight) => {
                Some((ALT_RELEASE_TIMER, &ALT_RELEASE_TIMER_ID))
            }
            _ => None,
        };
        if let Some((requested_id, timer_id)) = release_timer {
            // Taking focus for an overlay can emit an apparent modifier release while the key is
            // physically held. Confirm the physical state asynchronously before ending a chord.
            unsafe {
                let actual_id = SetTimer(None, requested_id, 10, None);
                timer_id.store(actual_id, Ordering::Release);
            }
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }
    }
    if key == Some(KeyCode::KeyR)
        && RUN_HOTKEY_REGISTERED.load(std::sync::atomic::Ordering::Acquire)
    {
        if edge == KeyEdge::Pressed
            && let Ok(mut controller) = hotkey_controller().lock()
        {
            // RegisterHotKey owns Super+R dispatch. The hook only records that another key joined
            // the Super press, preventing the later release from toggling the launcher.
            controller.handle_unmapped(KeyEdge::Pressed);
        }
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    if matches!(
        key,
        Some(KeyCode::Tab | KeyCode::Backquote | KeyCode::PrintScreen)
    ) && edge == KeyEdge::Pressed
        && unsafe { GetAsyncKeyState(VK_MENU as i32) < 0 }
        && let Ok(mut controller) = hotkey_controller().lock()
        && !controller.snapshot().alt_held
    {
        controller.handle(KeyCode::AltLeft, KeyEdge::Pressed);
    }
    let (outcome, snapshot) = hotkey_controller()
        .lock()
        .map(|mut controller| {
            let outcome = match key {
                Some(key) => controller.handle(key, edge),
                None => controller.handle_unmapped(edge),
            };
            (outcome, controller.snapshot())
        })
        .unwrap_or_default();
    if key != Some(KeyCode::KeyR) {
        trace_input("key", key, Some(edge), outcome, snapshot);
    }
    send_hotkey_action(outcome.action);
    if outcome.suppress {
        LRESULT(1)
    } else {
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }
}

fn reconcile_modifier_release(timer: usize) {
    const VK_LWIN: i32 = 0x5b;
    const VK_RWIN: i32 = 0x5c;
    const VK_MENU: i32 = 0x12;

    let super_timer = SUPER_RELEASE_TIMER_ID.load(Ordering::Acquire);
    let alt_timer = ALT_RELEASE_TIMER_ID.load(Ordering::Acquire);
    let released = unsafe {
        if timer == super_timer && super_timer != 0 {
            GetAsyncKeyState(VK_LWIN) >= 0 && GetAsyncKeyState(VK_RWIN) >= 0
        } else if timer == alt_timer && alt_timer != 0 {
            GetAsyncKeyState(VK_MENU) >= 0
        } else {
            return;
        }
    };
    if !released {
        return;
    }
    unsafe {
        let _ = KillTimer(None, timer);
    }
    let key = if timer == super_timer {
        SUPER_RELEASE_TIMER_ID.store(0, Ordering::Release);
        KeyCode::SuperLeft
    } else {
        ALT_RELEASE_TIMER_ID.store(0, Ordering::Release);
        KeyCode::AltLeft
    };
    let action = hotkey_controller()
        .lock()
        .ok()
        .and_then(|mut controller| controller.handle(key, KeyEdge::Released).action);
    send_hotkey_action(action);
}

fn key_code_from_windows_vk(vk: u32) -> Option<KeyCode> {
    Some(match vk {
        0x5b => KeyCode::SuperLeft,
        0x5c => KeyCode::SuperRight,
        0xa4 => KeyCode::AltLeft,
        0xa5 => KeyCode::AltRight,
        0x12 => KeyCode::AltLeft,
        0xa0 => KeyCode::ShiftLeft,
        0xa1 => KeyCode::ShiftRight,
        0x10 => KeyCode::ShiftLeft,
        0xa2 => KeyCode::ControlLeft,
        0xa3 => KeyCode::ControlRight,
        0x11 => KeyCode::ControlLeft,
        0x09 => KeyCode::Tab,
        0x25 => KeyCode::ArrowLeft,
        0x27 => KeyCode::ArrowRight,
        0xc0 => KeyCode::Backquote,
        0x52 => KeyCode::KeyR,
        0x2c => KeyCode::PrintScreen,
        _ => return None,
    })
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
    key: Option<KeyCode>,
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
        Some(HotkeyAction::LockSession) => GlobalShortcut::LockState { locked: true },
        Some(HotkeyAction::ToggleLauncher) => GlobalShortcut::ToggleLauncher,
        Some(HotkeyAction::ShowRun) => GlobalShortcut::ShowRun,
        Some(HotkeyAction::SwitchNext) => GlobalShortcut::SwitchNext,
        Some(HotkeyAction::SwitchPrevious) => GlobalShortcut::SwitchPrevious,
        Some(HotkeyAction::SwitchGroupNext) => GlobalShortcut::SwitchGroupNext,
        Some(HotkeyAction::SwitchGroupPrevious) => GlobalShortcut::SwitchGroupPrevious,
        Some(HotkeyAction::CommitSwitch) => GlobalShortcut::CommitSwitch,
        Some(HotkeyAction::CaptureActiveWindow) => {
            GlobalShortcut::Screenshot(ScreenshotAction::ActiveWindow)
        }
        Some(HotkeyAction::CaptureActiveWindowToFile) => {
            GlobalShortcut::Screenshot(ScreenshotAction::ActiveWindowToFile)
        }
        Some(HotkeyAction::ShowScreenshotTool) => {
            GlobalShortcut::Screenshot(ScreenshotAction::InteractiveRegion)
        }
        Some(
            HotkeyAction::SwitchWorkspacePrevious
            | HotkeyAction::SwitchWorkspaceNext
            | HotkeyAction::MoveWindowToPreviousWorkspace
            | HotkeyAction::MoveWindowToNextWorkspace,
        ) => return,
        None => return,
    };
    tracing::debug!(?shortcut, "dispatching Windows global shortcut");
    if let Some(sender) = SHORTCUT_SENDER.get() {
        if sender.send(shortcut).is_err() {
            tracing::warn!("Windows global shortcut receiver disconnected");
        }
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
            // Moving is inexpensive and should track the compositor closely. Resizing can make
            // applications such as Windows Terminal reflow and redraw their entire contents, so
            // retain a modest cap there without making ordinary dragging feel like 30 FPS.
            let minimum_interval = if operation.resize_edge.is_some() {
                16
            } else {
                8
            };
            if release || event.time.wrapping_sub(operation.last_update) >= minimum_interval {
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
    const VK_LWIN: i32 = 0x5b;
    const VK_RWIN: i32 = 0x5c;
    let physical_super = unsafe { GetAsyncKeyState(VK_LWIN) < 0 || GetAsyncKeyState(VK_RWIN) < 0 };
    let (super_held, chord_started) = hotkey_controller()
        .lock()
        .map(|mut controller| {
            // Mouse and keyboard low-level hooks are delivered independently. A mouse-down can
            // win the startup/event-order race before Nickel observes Super-down, so reconcile
            // from Windows' physical state at the gesture boundary.
            if physical_super && !controller.snapshot().super_held {
                controller.handle(KeyCode::SuperLeft, KeyEdge::Pressed);
            } else {
                controller.reconcile_super(physical_super);
            }
            let super_held = controller.snapshot().super_held;
            let chord_started = controller.begin_pointer_chord();
            (super_held, chord_started)
        })
        .unwrap_or_default();
    tracing::debug!(
        super_held,
        physical_super,
        chord_started,
        button = if message == WM_LBUTTONDOWN {
            "left"
        } else {
            "right"
        },
        "Super mouse gesture candidate"
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

pub fn execute_run_command(command: &str) -> Result<(), LaunchError> {
    if command
        .get(.."ms-settings:".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("ms-settings:"))
    {
        let uri = command.to_owned();
        // Windows can take an unbounded amount of time to activate the packaged Settings app.
        // Waiting here blocks the runtime event thread, which also makes the keyboard and mouse
        // hooks appear wedged. Treat a well-formed Settings URI as submitted and wait off-thread.
        thread::spawn(move || match launch_uri(&uri) {
            Ok(true) => {}
            Ok(false) => eprintln!("Windows declined to launch Settings URI: {uri}"),
            Err(error) => eprintln!("failed to launch Settings URI {uri}: {error}"),
        });
        return Ok(());
    }
    let parts = parse_windows_command(command)?;
    let (target, arguments) = parts.split_first().ok_or(LaunchError::EmptyCommand)?;
    shell_execute(target, arguments)
}

fn parse_windows_command(command: &str) -> Result<Vec<String>, LaunchError> {
    let command_wide: Vec<u16> = command.encode_utf16().chain([0]).collect();
    let mut count = 0;
    // SAFETY: command_wide is terminated and remains alive through parsing. Shell32 returns one
    // LocalAlloc block containing the pointer table and strings; LocalFree releases that block.
    let arguments = unsafe { CommandLineToArgvW(PCWSTR(command_wide.as_ptr()), &mut count) };
    if arguments.is_null() || count <= 0 {
        return Err(LaunchError::InvalidQuotes);
    }
    let parts = unsafe { std::slice::from_raw_parts(arguments, count as usize) }
        .iter()
        .map(|argument| unsafe { argument.to_string() }.unwrap_or_default())
        .collect();
    unsafe {
        LocalFree(Some(windows::Win32::Foundation::HLOCAL(arguments.cast())));
    }
    Ok(parts)
}

pub fn launch_application(application: &Application) -> Result<Option<u32>, LaunchError> {
    let (target, arguments) = application
        .launch_command()
        .and_then(|command| command.split_first())
        .ok_or_else(|| LaunchError::MissingTarget(application.name().to_owned()))?;
    shell_execute(target, arguments)?;
    Ok(None)
}

fn shell_execute(target: &str, arguments: &[String]) -> Result<(), LaunchError> {
    let target_wide: Vec<u16> = target.encode_utf16().chain([0]).collect();
    let home_wide = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(|home| {
            home.to_string_lossy()
                .encode_utf16()
                .chain([0])
                .collect::<Vec<_>>()
        });
    let argument_line = arguments
        .iter()
        .map(|argument| quote_windows_argument(argument))
        .collect::<Vec<_>>()
        .join(" ");
    let argument_wide: Vec<u16> = argument_line.encode_utf16().chain([0]).collect();
    let parameters = if arguments.is_empty() {
        PCWSTR::null()
    } else {
        PCWSTR(argument_wide.as_ptr())
    };
    // SAFETY: The target, argument, and optional home-directory UTF-16 buffers remain alive
    // through this synchronous Shell32 call.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(target_wide.as_ptr()),
            parameters,
            home_wide
                .as_ref()
                .map_or(PCWSTR::null(), |home| PCWSTR(home.as_ptr())),
            SW_SHOWNORMAL,
        )
    }
    .0 as isize;
    if result > 32 {
        Ok(())
    } else {
        Err(match result {
            2 => LaunchError::NotFound(target.to_owned()),
            3 => LaunchError::PathNotFound(target.to_owned()),
            5 => LaunchError::AccessDenied(target.to_owned()),
            31 => LaunchError::NoAssociation(target.to_owned()),
            _ => LaunchError::Platform(format!("{target} ({result})")),
        })
    }
}

fn quote_windows_argument(argument: &str) -> String {
    if !argument.chars().any(char::is_whitespace) && !argument.contains('"') {
        return argument.to_owned();
    }
    format!("\"{}\"", argument.replace('"', "\\\""))
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

pub fn configure_desktop_window(
    window: &impl raw_window_handle::HasWindowHandle,
    physical_position: (i32, i32),
    physical_size: (u32, u32),
) -> bool {
    let Some(hwnd) = window_hwnd(window) else {
        return false;
    };
    // SAFETY: hwnd belongs to the live desktop window. Windows returns a monitor rectangle
    // for that window, and SetWindowPos applies that rectangle while keeping the desktop at the
    // bottom of the Z-order. This also corrects stale runtime geometry after a display-mode change.
    unsafe {
        let previous_dpi_context =
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let result = (|| {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                (style | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize,
            );
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut monitor_info = MONITORINFOEXW {
                monitorInfo: MONITORINFO {
                    cbSize: size_of::<MONITORINFOEXW>() as u32,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut mode = DEVMODEW {
                dmSize: size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            let mode_size = GetMonitorInfoW(monitor, &raw mut monitor_info.monitorInfo)
                .as_bool()
                .then(|| {
                    EnumDisplaySettingsW(
                        windows::core::PCWSTR(monitor_info.szDevice.as_ptr()),
                        ENUM_CURRENT_SETTINGS,
                        &mut mode,
                    )
                    .as_bool()
                    .then_some((mode.dmPelsWidth, mode.dmPelsHeight))
                })
                .flatten();
            let (width, height) = mode_size.unwrap_or(physical_size);
            SetWindowPos(
                hwnd,
                Some(HWND_BOTTOM),
                physical_position.0,
                physical_position.1,
                width as i32,
                height as i32,
                SWP_NOACTIVATE | SWP_FRAMECHANGED,
            )
            .is_ok()
        })();
        if !previous_dpi_context.is_invalid() {
            let _ = SetThreadDpiAwarenessContext(previous_dpi_context);
        }
        result
    }
}

pub fn surface_size(
    window: &impl raw_window_handle::HasWindowHandle,
    fallback: (u32, u32),
) -> (u32, u32) {
    let Some(hwnd) = window_hwnd(window) else {
        return fallback;
    };
    let mut bounds = RECT::default();
    // SAFETY: hwnd is the live window borrowed from the caller and bounds is writable storage.
    if unsafe { GetClientRect(hwnd, &raw mut bounds) }.is_err() {
        return fallback;
    }
    (
        (bounds.right - bounds.left).max(1) as u32,
        (bounds.bottom - bounds.top).max(1) as u32,
    )
}

pub fn configure_launcher_window(window: &impl raw_window_handle::HasWindowHandle) -> bool {
    use std::sync::atomic::Ordering;

    let Some(hwnd) = window_hwnd(window) else {
        return false;
    };
    LAUNCHER_WINDOW_HANDLE.store(hwnd.0 as isize, Ordering::Relaxed);
    true
}

pub fn configure_preview_window(window: &impl raw_window_handle::HasWindowHandle) -> bool {
    use std::sync::atomic::Ordering;

    let Some(hwnd) = window_hwnd(window) else {
        return false;
    };
    PREVIEW_WINDOW_HANDLE.store(hwnd.0 as isize, Ordering::Relaxed);
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            (style | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) as isize,
        );
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
        .is_ok()
    }
}

pub fn configure_context_menu_window(window: &impl raw_window_handle::HasWindowHandle) -> bool {
    use std::sync::atomic::Ordering;

    let Some(hwnd) = window_hwnd(window) else {
        return false;
    };
    CONTEXT_MENU_WINDOW_HANDLE.store(hwnd.0 as isize, Ordering::Relaxed);
    true
}

pub fn configure_screenshot_window(window: &impl raw_window_handle::HasWindowHandle) -> bool {
    let Some(hwnd) = window_hwnd(window) else {
        return false;
    };
    // SAFETY: hwnd belongs to Nickel's live screenshot tool. TOOLWINDOW keeps the temporary
    // utility out of the taskbar and Alt+Tab while preserving its ordinary decorated window.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            ((style | WS_EX_TOOLWINDOW.0) & !WS_EX_APPWINDOW.0) as isize,
        );
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
        .is_ok()
    }
}

pub fn show_window_system_menu(window: WindowId) -> bool {
    use std::sync::atomic::Ordering;

    let target = hwnd(window);
    let owner_raw = CONTEXT_MENU_WINDOW_HANDLE.load(Ordering::Relaxed);
    let owner = if owner_raw == 0 {
        target
    } else {
        HWND(owner_raw as *mut c_void)
    };
    // SAFETY: both handles are revalidated by the operating system. GetSystemMenu returns a menu
    // owned by the target window; TrackPopupMenu only borrows it for this synchronous call.
    unsafe {
        if !IsWindow(Some(target)).as_bool() {
            return false;
        }
        let menu = GetSystemMenu(target, false);
        if menu.is_invalid() {
            return false;
        }
        let mut cursor = POINT::default();
        if GetCursorPos(&mut cursor).is_err() {
            return false;
        }
        let selected = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            cursor.x,
            cursor.y,
            None,
            owner,
            None,
        )
        .0;
        if selected != 0 {
            let _ = PostMessageW(
                Some(target),
                WM_SYSCOMMAND,
                WPARAM(selected as usize),
                LPARAM(0),
            );
        }
        true
    }
}

pub fn configure_volume_osd_window(window: &impl raw_window_handle::HasWindowHandle) -> bool {
    let Some(hwnd) = window_hwnd(window) else {
        return false;
    };
    // SAFETY: style and DWM attributes apply only to Nickel's live indicator window.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            (style | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) as isize,
        );
        let preference: DWM_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND;
        let rounded = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const preference).cast(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        )
        .is_ok();
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
        .is_ok()
            && rounded
    }
}

pub fn launcher_has_foreground_focus() -> bool {
    use std::sync::atomic::Ordering;

    let launcher = LAUNCHER_WINDOW_HANDLE.load(Ordering::Relaxed);
    launcher != 0 && unsafe { GetForegroundWindow().0 as isize == launcher }
}

pub fn configure_panel_window(window: &impl raw_window_handle::HasWindowHandle) -> bool {
    let Some(hwnd) = window_hwnd(window) else {
        return false;
    };
    let mut rectangle = Default::default();
    // SAFETY: hwnd belongs to the live panel and rectangle is writable storage.
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
        tracing::info!(
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
    // SAFETY: hwnd is Nickel's live panel. We retain and call its original window procedure
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
    // SAFETY: hwnd is Nickel's live top-level panel window. Shell-hook notifications are delivered
    // to its subclass procedure on the creating UI thread.
    unsafe {
        let shell_hook_message = RegisterWindowMessageW(w!("SHELLHOOK"));
        if shell_hook_message != 0 && RegisterShellHookWindow(hwnd).as_bool() {
            SHELL_HOOK_MESSAGE.store(shell_hook_message, Ordering::Relaxed);
        } else {
            tracing::warn!("failed to register Nickel for Windows shell-hook messages");
        }
    }
    install_tray_notify_window(hwnd);
    // Applications cache failed Shell_NotifyIcon registrations. Explorer announces taskbar
    // recreation with this registered message, prompting well-behaved clients to add them again.
    // SAFETY: This is an asynchronous broadcast with scalar parameters only.
    unsafe {
        let message = RegisterWindowMessageW(w!("TaskbarCreated"));
        let _ = SendNotifyMessageW(HWND_BROADCAST, message, WPARAM(0), LPARAM(0));
    }
}

fn install_tray_notify_window(_panel: HWND) {
    use std::sync::atomic::Ordering;

    if TRAY_NOTIFY_WINDOW_HANDLE.load(Ordering::Relaxed) != 0 {
        return;
    }
    // SAFETY: The class procedures are static for the process lifetime. Shell_NotifyIcon first
    // discovers a top-level Shell_TrayWnd, then sends its protocol message to TrayNotifyWnd.
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            eprintln!("failed to resolve Nickel's module for the notification-area host");
            return;
        };
        let shell_class = WNDCLASSW {
            hInstance: windows::Win32::Foundation::HINSTANCE(module.0),
            lpszClassName: w!("Shell_TrayWnd"),
            lpfnWndProc: Some(tray_window_proc),
            ..Default::default()
        };
        if RegisterClassW(&raw const shell_class) == 0 {
            eprintln!("failed to register Nickel's Shell_TrayWnd class");
            return;
        }
        let Ok(shell_window) = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            shell_class.lpszClassName,
            w!(""),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(shell_class.hInstance),
            None,
        ) else {
            eprintln!("failed to create Nickel's Shell_TrayWnd protocol host");
            return;
        };
        SHELL_TRAY_WINDOW_HANDLE.store(shell_window.0 as isize, Ordering::Relaxed);

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
            Some(shell_window),
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
    if message == SHELL_HOOK_MESSAGE.load(Ordering::Relaxed) && wparam.0 == 12 {
        let command = ((lparam.0 as u32) >> 16) & 0x0fff;
        if handle_shell_app_command(command) {
            return LRESULT(0);
        }
    }
    if message == WM_COPYDATA {
        // SAFETY: WM_COPYDATA guarantees the COPYDATASTRUCT and its buffer remain valid for this
        // synchronous call. Bounds and protocol signature are validated before interpretation.
        let copy = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
        const TRAY_HEADER_SIZE: usize = size_of::<i32>() + size_of::<u32>();
        let minimum_icon_size = std::mem::offset_of!(TrayNotifyIconData, icon) + size_of::<u32>();
        if copy.dwData == 1
            && copy.cbData as usize >= TRAY_HEADER_SIZE + minimum_icon_size
            && !copy.lpData.is_null()
        {
            let bytes = copy.lpData.cast::<u8>();
            let signature = unsafe { bytes.cast::<i32>().read_unaligned() };
            let operation = unsafe { bytes.add(size_of::<i32>()).cast::<u32>().read_unaligned() };
            let mut icon: TrayNotifyIconData = unsafe { std::mem::zeroed() };
            let supplied = (copy.cbData as usize - TRAY_HEADER_SIZE).min(size_of_val(&icon));
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.add(TRAY_HEADER_SIZE),
                    (&raw mut icon).cast::<u8>(),
                    supplied,
                );
            }
            if signature == 0x3475_3423_u32 as i32 && update_tray_icon(operation, &icon) {
                return LRESULT(1);
            }
        }
    }
    let previous = PANEL_WINDOW_PROC.load(Ordering::Relaxed);
    let panel = PANEL_WINDOW_HANDLE.load(Ordering::Relaxed);
    if previous != 0 && hwnd.0 as isize == panel {
        // SAFETY: previous is the live WNDPROC returned by SetWindowLongPtrW.
        let procedure = unsafe { std::mem::transmute(previous) };
        return unsafe { CallWindowProcW(procedure, hwnd, message, wparam, lparam) };
    }
    // SAFETY: Messages for the private TrayNotifyWnd child use the system default procedure.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn handle_shell_app_command(command: u32) -> bool {
    const VOLUME_MUTE: u32 = 8;
    const VOLUME_DOWN: u32 = 9;
    const VOLUME_UP: u32 = 10;
    const MEDIA_NEXT: u32 = 11;
    const MEDIA_PREVIOUS: u32 = 12;
    const MEDIA_STOP: u32 = 13;
    const MEDIA_PLAY_PAUSE: u32 = 14;
    const MEDIA_PLAY: u32 = 46;
    const MEDIA_PAUSE: u32 = 47;
    const MEDIA_FAST_FORWARD: u32 = 49;
    const MEDIA_REWIND: u32 = 50;

    match command {
        VOLUME_MUTE | VOLUME_DOWN | VOLUME_UP => {
            match apply_endpoint_app_command(command) {
                Ok((volume_percent, muted)) => {
                    if let Some(sender) = SHORTCUT_SENDER.get() {
                        let _ = sender.send(GlobalShortcut::AudioChanged {
                            volume_percent,
                            muted,
                        });
                    }
                }
                Err(error) => {
                    tracing::warn!(command, %error, "failed to apply shell audio command");
                }
            }
            true
        }
        MEDIA_NEXT | MEDIA_PREVIOUS | MEDIA_STOP | MEDIA_PLAY_PAUSE | MEDIA_PLAY | MEDIA_PAUSE
        | MEDIA_FAST_FORWARD | MEDIA_REWIND => {
            dispatch_media_app_command(command);
            true
        }
        _ => false,
    }
}

fn apply_endpoint_app_command(command: u32) -> windows::core::Result<(u8, bool)> {
    use windows::Win32::{
        Media::Audio::{
            Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator, MMDeviceEnumerator, eMultimedia,
            eRender,
        },
        System::Com::CLSCTX_ALL,
    };

    // SAFETY: COM initialization is balanced on this thread when this call initialized it.
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
            let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
            match command {
                8 => endpoint.SetMute(!endpoint.GetMute()?.as_bool(), std::ptr::null()),
                9 => endpoint.VolumeStepDown(std::ptr::null()),
                10 => endpoint.VolumeStepUp(std::ptr::null()),
                _ => Ok(()),
            }?;
            Ok((
                (endpoint.GetMasterVolumeLevelScalar()? * 100.0)
                    .round()
                    .clamp(0.0, 100.0) as u8,
                endpoint.GetMute()?.as_bool(),
            ))
        })();
        if initialized {
            CoUninitialize();
        }
        result
    }
}

fn dispatch_media_app_command(command: u32) {
    thread::spawn(move || {
        use std::future::IntoFuture;
        use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;

        let result = (|| -> windows::core::Result<bool> {
            let manager = pollster::block_on(
                GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.into_future(),
            )?;
            let session = manager.GetCurrentSession()?;
            let operation = match command {
                11 => session.TrySkipNextAsync()?,
                12 => session.TrySkipPreviousAsync()?,
                13 => session.TryStopAsync()?,
                14 => session.TryTogglePlayPauseAsync()?,
                46 => session.TryPlayAsync()?,
                47 => session.TryPauseAsync()?,
                49 => session.TryFastForwardAsync()?,
                50 => session.TryRewindAsync()?,
                _ => return Ok(false),
            };
            pollster::block_on(operation.into_future())
        })();
        if let Err(error) = result {
            tracing::debug!(command, %error, "media command had no controllable Windows session");
        }
    });
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
            let Some(image) = render_tray_icon(HICON(icon.icon as usize as *mut c_void)) else {
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
                && let Some(image) = render_tray_icon(HICON(icon.icon as usize as *mut c_void))
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
        && icon.state_mask & NIS_HIDDEN.0 != 0
        && icon.state & NIS_HIDDEN.0 != 0
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

pub fn release_panel_window(window: &impl raw_window_handle::HasWindowHandle) {
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
        SHAppBarMessage(ABM_REMOVE, &mut appbar);
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

fn window_hwnd(window: &impl raw_window_handle::HasWindowHandle) -> Option<HWND> {
    use raw_window_handle::RawWindowHandle;

    let handle = window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut c_void)),
        _ => None,
    }
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

pub struct NotificationFeed;
impl NotificationFeed {
    pub fn new() -> Result<Self, String> {
        Ok(Self)
    }
}
impl NotificationSource for NotificationFeed {
    fn snapshot(&self) -> Option<crate::notification::DesktopNotification> {
        None
    }
    fn dismiss(&self, _: u32) {}
    fn invoke(&self, _: u32, _: &str) {}
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
            let preview = PREVIEW_WINDOW_HANDLE.load(Ordering::Relaxed);
            if preview == 0 {
                return false;
            }
            let hwnd = HWND(preview as *mut c_void);
            let panel = PANEL_WINDOW_HANDLE.load(Ordering::Relaxed);
            let Some(work_area) = monitor_work_area(HWND(panel as *mut c_void)) else {
                return false;
            };
            unsafe {
                let top = (work_area.bottom - height).max(work_area.top);
                let left = clamp_preview_x(x, width, work_area);
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
            return show_dwm_previews(&windows);
        }
        ShellCommand::ShowTaskSwitcher {
            width,
            height,
            windows,
        } => {
            let preview = PREVIEW_WINDOW_HANDLE.load(Ordering::Relaxed);
            if preview == 0 {
                return false;
            }
            let hwnd = HWND(preview as *mut c_void);
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
                let x = work_area.left + ((work_area.right - work_area.left - width) / 2).max(0);
                let y = work_area.top + ((work_area.bottom - work_area.top - height) / 2).max(0);
                if SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    x,
                    y,
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
                if should_restore_on_activation(
                    IsIconic(hwnd).as_bool(),
                    window_covers_monitor(hwnd),
                ) {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
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

pub fn register_session_shell() -> Result<(), super::SessionRequestError> {
    Ok(())
}

fn should_restore_on_activation(iconic: bool, covers_monitor: bool) -> bool {
    iconic && !covers_monitor
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
    let destination = PREVIEW_WINDOW_HANDLE.load(Ordering::Relaxed);
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
        let (left, top, right, bottom) = crate::window_preview::native_thumbnail_bounds(index);
        let bounds = RECT {
            left,
            top,
            right,
            bottom,
        };
        let destination_rect = unsafe { DwmQueryThumbnailSourceSize(thumbnail) }
            .map(|source_size| contain_rect(bounds, source_size))
            .unwrap_or(bounds);
        let properties = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION
                | DWM_TNP_OPACITY
                | DWM_TNP_VISIBLE
                | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: destination_rect,
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

fn contain_rect(bounds: RECT, source: SIZE) -> RECT {
    let width = i64::from((bounds.right - bounds.left).max(0));
    let height = i64::from((bounds.bottom - bounds.top).max(0));
    let source_width = i64::from(source.cx.max(0));
    let source_height = i64::from(source.cy.max(0));
    if width == 0 || height == 0 || source_width == 0 || source_height == 0 {
        return bounds;
    }
    let (fitted_width, fitted_height) = if source_width * height > width * source_height {
        (width, (source_height * width / source_width).max(1))
    } else {
        ((source_width * height / source_height).max(1), height)
    };
    let left = bounds.left + ((width - fitted_width) / 2) as i32;
    let top = bounds.top + ((height - fitted_height) / 2) as i32;
    RECT {
        left,
        top,
        right: left + fitted_width as i32,
        bottom: top + fitted_height as i32,
    }
}

fn clamp_preview_x(requested: i32, width: i32, work_area: RECT) -> i32 {
    requested.clamp(
        work_area.left,
        (work_area.right - width).max(work_area.left),
    )
}

fn monitor_work_area(window: HWND) -> Option<RECT> {
    if window.0.is_null() {
        return None;
    }
    let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetMonitorInfoW(monitor, &mut info) }
        .as_bool()
        .then_some(info.rcWork)
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
    pub fn launcher_visible(&self) -> Option<bool> {
        None
    }
    pub fn new() -> Self {
        Self
    }

    pub fn snapshot(&self, _: &Launcher) -> FeedState<Vec<OpenWindow>> {
        let mut windows = Vec::new();
        // SAFETY: The callback only reads top-level window metadata and the LPARAM points to this
        // live vector for the duration of the synchronous EnumWindows call.
        unsafe {
            let state = LPARAM((&mut windows as *mut Vec<OpenWindow>) as isize);
            if EnumWindows(Some(collect_window), state).is_err() {
                return FeedState::Failed;
            }
        }
        FeedState::Ready(windows)
    }

    pub fn window_output(&self, _: WindowId) -> Option<String> {
        None
    }

    pub fn workspaces(&self) -> FeedState<Vec<super::WorkspaceSummary>> {
        FeedState::Ready(Vec::new())
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
    if !is_bar_eligible_window(hwnd, &class) {
        return BOOL(1);
    }
    let application_id = Some(if is_nickel_host_terminal(&title) {
        ApplicationId::new("org.nickel.ShellTerminal")
    } else {
        executable_path(hwnd)
            .map(|path| {
                ApplicationId::new(format!(
                    "windows-exe:{}",
                    path.to_string_lossy().to_ascii_lowercase()
                ))
            })
            .unwrap_or_else(|| {
                ApplicationId::new(format!("windows-class:{}", class.to_ascii_lowercase()))
            })
    });
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

fn is_nickel_host_terminal(title: &str) -> bool {
    static EXECUTABLE_TITLE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    EXECUTABLE_TITLE
        .get_or_init(|| {
            env::current_exe()
                .ok()
                .map(|path| path.to_string_lossy().into_owned())
        })
        .as_deref()
        .is_some_and(|executable| title.eq_ignore_ascii_case(executable))
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

fn is_bar_eligible_window(hwnd: HWND, class: &str) -> bool {
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
    render_icon_sized(icon, 32, 32)
}

fn render_tray_icon(icon: HICON) -> Option<image::RgbaImage> {
    // Ask USER32 for the full icon resource before rasterizing. Many tray clients submit a
    // small HICON even though its resource contains sharper sizes for DPI-aware rendering.
    let width = unsafe { GetSystemMetrics(SM_CXICON) }.max(1);
    let height = unsafe { GetSystemMetrics(SM_CYICON) }.max(1);
    let copied = unsafe {
        CopyImage(
            HANDLE(icon.0),
            IMAGE_ICON,
            width,
            height,
            LR_COPYFROMRESOURCE,
        )
        .ok()
        .map(|handle| HICON(handle.0))
    };
    let rendered = render_icon_sized(copied.unwrap_or(icon), width as u32, height as u32);
    if let Some(copied) = copied {
        // SAFETY: CopyImage returned a distinct icon because LR_COPYRETURNORG was not requested.
        let _ = unsafe { DestroyIcon(copied) };
    }
    rendered
}

fn render_icon_sized(icon: HICON, width: u32, height: u32) -> Option<image::RgbaImage> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
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
            width as i32,
            height as i32,
            0,
            None,
            DI_NORMAL,
        )
        .is_ok();
        let mut rgba = vec![0_u8; (width * height * 4) as usize];
        if drawn && !pixels.is_null() {
            let bgra = std::slice::from_raw_parts(pixels.cast::<u8>(), rgba.len());
            for (source, target) in bgra.chunks_exact(4).zip(rgba.chunks_exact_mut(4)) {
                target.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
            }
            restore_legacy_icon_alpha(&mut rgba);
        }
        SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        drawn
            .then(|| image::RgbaImage::from_raw(width, height, rgba))
            .flatten()
    }
}

fn restore_legacy_icon_alpha(rgba: &mut [u8]) {
    if rgba.chunks_exact(4).all(|pixel| pixel[3] == 0) {
        for pixel in rgba.chunks_exact_mut(4) {
            if pixel[..3].iter().any(|channel| *channel != 0) {
                pixel[3] = 255;
            }
        }
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

    use super::{
        TrayNotifyIconData, application_icon, clamp_preview_x, contain_rect, executable_icon,
        is_nickel_host_terminal, is_shell_infrastructure, native_hotkey_requests,
        parse_windows_command, rectangle_covers, restore_legacy_icon_alpha,
        should_restore_on_activation,
    };

    #[test]
    fn native_tray_wire_layout_uses_packed_32_bit_handles() {
        assert_eq!(std::mem::offset_of!(TrayNotifyIconData, icon), 20);
        assert_eq!(std::mem::size_of::<TrayNotifyIconData>(), 956);
    }

    #[test]
    fn legacy_icon_color_pixels_gain_opaque_alpha() {
        let mut rgba = [20, 30, 40, 0, 0, 0, 0, 0];
        restore_legacy_icon_alpha(&mut rgba);
        assert_eq!(rgba, [20, 30, 40, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn taskbar_preview_is_clamped_to_work_area_edges() {
        let work_area = RECT {
            left: 100,
            top: 0,
            right: 1100,
            bottom: 700,
        };
        assert_eq!(clamp_preview_x(-500, 300, work_area), 100);
        assert_eq!(clamp_preview_x(1000, 300, work_area), 800);
        assert_eq!(clamp_preview_x(400, 300, work_area), 400);
        assert_eq!(clamp_preview_x(400, 1200, work_area), 100);
    }

    #[test]
    fn dwm_thumbnail_is_contained_and_centered_without_distortion() {
        let bounds = RECT {
            left: 20,
            top: 50,
            right: 280,
            bottom: 166,
        };
        assert_eq!(
            contain_rect(
                bounds,
                windows::Win32::Foundation::SIZE { cx: 1920, cy: 1080 }
            ),
            RECT {
                left: 47,
                top: 50,
                right: 253,
                bottom: 166,
            }
        );
        assert_eq!(
            contain_rect(
                bounds,
                windows::Win32::Foundation::SIZE { cx: 1080, cy: 1920 }
            ),
            RECT {
                left: 117,
                top: 50,
                right: 182,
                bottom: 166,
            }
        );
    }

    #[test]
    fn shell_executable_icon_has_visible_pixels() {
        let image = executable_icon(&std::env::current_exe().expect("test executable path"))
            .expect("Windows Shell returns an executable icon");
        assert!(image.pixels().any(|pixel| pixel.0[3] != 0));
    }

    #[test]
    fn shell_host_terminal_matches_only_the_nickel_executable_title() {
        let executable = std::env::current_exe().expect("test executable path");
        assert!(is_nickel_host_terminal(&executable.to_string_lossy()));
        assert!(!is_nickel_host_terminal("PowerShell"));
    }

    #[test]
    fn installed_shortcut_icon_has_visible_pixels() {
        let Some(program_data) = std::env::var_os("PROGRAMDATA") else {
            return;
        };
        let root =
            std::path::PathBuf::from(program_data).join("Microsoft/Windows/Start Menu/Programs");
        for shortcut in [
            root.join("Google Chrome.lnk"),
            root.join("Windows Kits/Application Verifier (X64)/Application Verifier (X64).lnk"),
        ] {
            if !shortcut.is_file() {
                continue;
            }
            let image = application_icon(&shortcut.to_string_lossy())
                .expect("resolve the installed shortcut icon");
            assert!(image.pixels().any(|pixel| pixel.0[3] != 0));
        }
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

    #[test]
    fn activation_restores_only_minimized_non_fullscreen_windows() {
        assert!(should_restore_on_activation(true, false));
        assert!(!should_restore_on_activation(false, false));
        assert!(!should_restore_on_activation(false, true));
        assert!(!should_restore_on_activation(true, true));
    }

    #[test]
    fn run_parser_preserves_windows_paths_and_quoted_arguments() {
        assert_eq!(
            parse_windows_command(r#""C:\Program Files\Nickel\nickel.exe" --name "Nickel Shell""#)
                .expect("valid Windows command line"),
            [
                r"C:\Program Files\Nickel\nickel.exe",
                "--name",
                "Nickel Shell"
            ]
        );
    }

    #[test]
    fn register_hotkey_is_reserved_for_chords_not_bare_super() {
        let requests = native_hotkey_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].virtual_key, 0x52);
        assert_ne!(requests[0].virtual_key, 0x5b);
        assert_ne!(requests[0].virtual_key, 0x5c);
    }
}
