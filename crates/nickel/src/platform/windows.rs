#[path = "windows_remote_observation.rs"]
pub(crate) mod remote_observation;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    env,
    ffi::c_void,
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::Ordering,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use windows::{
    Win32::{
        Foundation::{
            COLORREF, CloseHandle, FILETIME, GlobalFree, HANDLE, HWND, LPARAM, LRESULT, LocalFree,
            POINT, RECT, SIZE, WPARAM,
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
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        System::LibraryLoader::GetModuleHandleW,
        System::SystemInformation::GetTickCount64,
        System::Threading::{
            AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId, GetProcessTimes,
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
        System::{
            Com::{
                CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize,
            },
            DataExchange::{
                COPYDATASTRUCT, CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
            },
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
        },
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, GetCapture, ReleaseCapture, SetCapture, SetFocus,
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
                BringWindowToTop, CallWindowProcW, CopyImage, CreateWindowExW, DI_NORMAL,
                DefWindowProcW, DestroyIcon, DrawIconEx, EVENT_OBJECT_DESTROY,
                EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART, EnumWindows, GA_ROOT,
                GA_ROOTOWNER, GCLP_HICON, GCLP_HICONSM, GWL_EXSTYLE, GWLP_WNDPROC, GetAncestor,
                GetClassLongPtrW, GetClassNameW, GetClientRect, GetCursorPos, GetForegroundWindow,
                GetLastActivePopup, GetSystemMenu, GetSystemMetrics, GetWindowLongPtrW,
                GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
                HICON, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT,
                HTTOPRIGHT, HWND_BOTTOM, HWND_BROADCAST, HWND_TOPMOST, IMAGE_ICON, IsIconic,
                IsWindow, IsWindowVisible, IsZoomed, LR_COPYFROMRESOURCE, LWA_ALPHA,
                NID_INTEGRATED_TOUCH, NID_READY, PostMessageW, RegisterClassW,
                RegisterShellHookWindow, RegisterWindowMessageW, SM_CXICON, SM_CYICON,
                SM_DIGITIZER, SPI_GETWORKAREA, SPI_SETWORKAREA, SPIF_SENDCHANGE, SW_HIDE,
                SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOW, SW_SHOWNOACTIVATE, SW_SHOWNORMAL,
                SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
                SWP_NOZORDER, SendNotifyMessageW, SetForegroundWindow, SetLayeredWindowAttributes,
                SetWindowLongPtrW, SetWindowPos, ShowWindow, SystemParametersInfoW, TPM_RETURNCMD,
                TPM_RIGHTBUTTON, TrackPopupMenu, WINDOW_EX_STYLE, WINDOW_STYLE,
                WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_CLOSE, WM_CONTEXTMENU,
                WM_COPYDATA, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
                WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSCOMMAND, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN,
                WS_CLIPSIBLINGS, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
                WS_EX_TOOLWINDOW, WS_POPUP, WindowFromPoint,
            },
        },
    },
    core::{BOOL, PCWSTR, PWSTR, w},
};

use nickel_core::{
    geometry::LogicalRect,
    geometry_authority::{
        ControlMode, CoordinateUnits, GeometryAuthority, GeometryConstraints, GeometryMeaning,
        NativeRequest, NativeRequestId, ObservationCausality, Presentation, Settlement,
        SettlementLimits, SettlementStatus, TaggedGeometry,
    },
    hotkeys::{HotkeyAction, KeyCode, KeyEdge},
    window_operation::{
        BeginRequest, CancellationReason, CompletionBinding, CompletionGesture, Disposition,
        Effect as WindowOperationEffect, FailureReason, GeometrySeed, GeometryUpdate,
        HorizontalEdge, MappingGeneration, NativeLifetimeId, OperationId, OperationKind,
        ResizeEdges, ResourceLeaseId, SeatId, Source, SourceGeneration, SourceId, VerticalEdge,
        WindowId as OperationWindowId, WindowMapping, WindowOperationReducer,
    },
};
use nickel_input::{
    AggregateModifier, PhysicalKey, PointerButton, Shortcut, ShortcutKey, ShortcutTrigger,
    global::{GlobalShortcutEdge, Registration, RegistrationError, RegistrationTable},
    windows::{
        HookDisposition, NativeHookCallbacks, NativeHotkeyRegistration, NativeKeyboardEvent,
        NativePointerEvent, NativePointerKind, SuperPointerGesture, WindowsInputAdapter,
        physical_key, run_native_hook_loop,
    },
};

pub(crate) fn touchscreen_present() -> bool {
    // SAFETY: GetSystemMetrics is a process-local, read-only system query.
    let digitizer = unsafe { GetSystemMetrics(SM_DIGITIZER) } as u32;
    digitizer & (NID_READY | NID_INTEGRATED_TOUCH) == (NID_READY | NID_INTEGRATED_TOUCH)
}

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

/// Returns a point on the display currently used by the foreground application.
/// The cursor is the fallback when Windows has no meaningful foreground window.
pub fn active_display_point() -> Option<(i32, i32)> {
    unsafe {
        let foreground = GetForegroundWindow();
        if !foreground.is_invalid() {
            let mut bounds = RECT::default();
            if GetWindowRect(foreground, &mut bounds).is_ok()
                && bounds.right > bounds.left
                && bounds.bottom > bounds.top
            {
                return Some((
                    bounds.left + (bounds.right - bounds.left) / 2,
                    bounds.top + (bounds.bottom - bounds.top) / 2,
                ));
            }
        }
        let mut cursor = POINT::default();
        GetCursorPos(&mut cursor)
            .ok()
            .map(|()| (cursor.x, cursor.y))
    }
}

pub fn copy_image_to_clipboard(image: image::RgbaImage) -> Result<(), String> {
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
    let (applications, truncated) = start_menu::load_application_discovery();
    if truncated {
        let mut report = crate::model::ApplicationDiscoveryReport::new();
        report.record(crate::model::ApplicationSkipReason::Capacity);
        ApplicationDiscovery::from_report(applications, report)
    } else {
        ApplicationDiscovery::ready(applications)
    }
}

pub(crate) fn prepare_application_discovery() -> ApplicationDiscovery {
    application_discovery()
}

pub(crate) fn publish_application_discovery(_: &ApplicationDiscovery) {}

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

pub(crate) fn refresh_connectivity_status() -> Result<super::ConnectivityRefresh, String> {
    Ok(super::bound_connectivity_refresh(
        network_status(),
        bluetooth_status(),
    ))
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
    native_audio_refresh().audio
}

fn native_audio_refresh() -> super::AudioRefresh {
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
        let result = (|| -> windows::core::Result<(super::AudioStatus, bool)> {
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
            let count = collection.GetCount()?;
            for index in 0..count.min(super::AUDIO_DEVICE_LIMIT as u32) {
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
            Ok((
                super::AudioStatus {
                    available: true,
                    devices,
                    volume_percent,
                    muted,
                },
                count as usize > super::AUDIO_DEVICE_LIMIT,
            ))
        })();
        if initialized {
            CoUninitialize();
        }
        match result {
            Ok((status, truncated)) => {
                let mut refresh = super::bound_audio_refresh(status);
                refresh.partial |= truncated;
                refresh
            }
            Err(_) => super::bound_audio_refresh(super::AudioStatus::default()),
        }
    }
}

pub(crate) fn refresh_audio_status() -> Result<super::AudioRefresh, String> {
    Ok(native_audio_refresh())
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

pub fn handle_consumer_control(_control: nickel_session_protocol::ConsumerControl) -> bool {
    // Windows owns consumer controls through WM_APPCOMMAND. Winit delivery must not
    // apply the same physical action a second time.
    false
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
    let diagnostic_state = Arc::new(Mutex::new(WindowsShortcutOwnerState {
        capability: nickel_input::global::ShortcutCapability::Unavailable(
            nickel_input::global::UnavailableReason::MissingRuntime,
        ),
        registration_revision: None,
    }));
    let hook_diagnostic_state = Arc::clone(&diagnostic_state);
    let capability = match thread::Builder::new()
        .name("nickel-super-key".into())
        .spawn(move || {
            run_super_key_hook(sender, startup_sender, &hook_diagnostic_state);
            if let Ok(mut state) = hook_diagnostic_state.lock() {
                state.capability = nickel_input::global::ShortcutCapability::Unavailable(
                    nickel_input::global::UnavailableReason::MissingRuntime,
                );
                state.registration_revision = None;
            }
        }) {
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
        capability: capability.clone(),
        diagnostics: WindowsShortcutDiagnosticSource {
            state: diagnostic_state,
        },
    }
}

struct WindowsShortcutOwnerState {
    capability: nickel_input::global::ShortcutCapability,
    registration_revision: Option<u64>,
}

#[derive(Clone)]
pub(crate) struct WindowsShortcutDiagnosticSource {
    state: Arc<Mutex<WindowsShortcutOwnerState>>,
}

impl WindowsShortcutDiagnosticSource {
    pub(crate) fn unavailable() -> Self {
        Self {
            state: Arc::new(Mutex::new(WindowsShortcutOwnerState {
                capability: nickel_input::global::ShortcutCapability::Unavailable(
                    nickel_input::global::UnavailableReason::MissingRuntime,
                ),
                registration_revision: None,
            })),
        }
    }

    pub(crate) fn snapshot(
        &self,
        observation_generation: u64,
        observed_at_us: u64,
    ) -> nickel_remote_control::diagnostics::ShortcutDiagnostic {
        let owner = self.state.try_lock().ok().map(|state| {
            let capability = if matches!(
                state.capability,
                nickel_input::global::ShortcutCapability::Available
            ) {
                nickel_remote_control::diagnostics::ShortcutDiagnosticCapability::Available
            } else {
                nickel_remote_control::diagnostics::ShortcutDiagnosticCapability::BackendUnavailable
            };
            (capability, state.registration_revision)
        });
        let adapter = windows_input_adapter().try_lock().ok();
        project_windows_shortcuts(
            observation_generation,
            observed_at_us,
            owner,
            adapter.as_deref(),
        )
    }
}

fn project_windows_shortcuts(
    observation_generation: u64,
    observed_at_us: u64,
    owner: Option<(
        nickel_remote_control::diagnostics::ShortcutDiagnosticCapability,
        Option<u64>,
    )>,
    adapter: Option<&WindowsInputAdapter<HotkeyAction>>,
) -> nickel_remote_control::diagnostics::ShortcutDiagnostic {
    use nickel_input::{PhysicalKey, ShortcutKey};
    use nickel_remote_control::diagnostics::{
        MAX_DIAGNOSTIC_SHORTCUTS, ShortcutDiagnostic, ShortcutDiagnosticCapability,
        ShortcutRegistrationDiagnostic,
    };
    let available = owner.is_some_and(|(capability, revision)| {
        capability == ShortcutDiagnosticCapability::Available && revision.is_some()
    }) && adapter.is_some();
    let mut snapshot = ShortcutDiagnostic {
        observation_generation,
        observed_at_us,
        registration_revision: if available {
            owner.and_then(|(_, revision)| revision)
        } else {
            None
        },
        capability: owner
            .map(|(capability, _)| capability)
            .unwrap_or(ShortcutDiagnosticCapability::BackendUnavailable),
        registrations: Vec::new(),
        unprojected_bindings: 0,
        truncated: false,
    };
    let Some(adapter) = adapter.filter(|_| available) else {
        snapshot.capability = ShortcutDiagnosticCapability::BackendUnavailable;
        snapshot.registration_revision = None;
        return snapshot;
    };
    for (index, binding) in adapter.bindings().enumerate() {
        if index == MAX_DIAGNOSTIC_SHORTCUTS {
            snapshot.truncated = true;
            break;
        }
        let ShortcutKey::Physical(PhysicalKey::Code(key)) = &binding.shortcut.key else {
            snapshot.unprojected_bindings += 1;
            continue;
        };
        snapshot.registrations.push(ShortcutRegistrationDiagnostic {
            registration_id: index as u64 + 1,
            physical_key: format!("{key:?}"),
            action: format!("{:?}", binding.action),
            modifiers: binding
                .shortcut
                .modifiers
                .iter()
                .map(|modifier| format!("{modifier:?}"))
                .collect(),
            trigger: format!("{:?}", binding.shortcut.trigger),
        });
    }
    snapshot
}

pub fn handle_focused_shortcut(key: KeyCode, edge: KeyEdge) {
    let actions = windows_input_adapter().lock().ok().map(|mut adapter| {
        if key == KeyCode::PrintScreen
            && edge == KeyEdge::Released
            && !adapter.key_held(KeyCode::PrintScreen)
        {
            let pressed = adapter.handle_key_code(key, KeyEdge::Pressed).outcomes;
            let _ = adapter.handle_key_code(key, KeyEdge::Released);
            pressed
        } else {
            adapter.handle_key_code(key, edge).outcomes
        }
    });
    if let Some(actions) = actions {
        send_hotkey_outcomes(actions);
    }
}

fn run_super_key_hook(
    sender: Sender<GlobalShortcut>,
    startup: mpsc::SyncSender<nickel_input::global::ShortcutCapability>,
    diagnostic_state: &Arc<Mutex<WindowsShortcutOwnerState>>,
) {
    SHORTCUT_SENDER.set(sender).ok();
    let native_move_size_hook = NativeMoveSizeHook::install();
    NATIVE_MOVE_SIZE_OBSERVATION_AVAILABLE
        .store(native_move_size_hook.is_some(), Ordering::Release);
    if native_move_size_hook.is_none() {
        tracing::warn!("native move/size ownership observation is unavailable");
    }
    let run = native_hotkey_requests()[0];
    let registrations = Arc::new(Mutex::new((RegistrationTable::default(), None)));
    let ready_registrations = Arc::clone(&registrations);
    let activation_registrations = Arc::clone(&registrations);
    let ready_run = run;
    let activation_run = run;
    let ready_diagnostic_state = diagnostic_state.clone();
    run_native_hook_loop(
        NativeHookCallbacks {
            keyboard: Arc::new(handle_native_keyboard_hook),
            modifier_released: Arc::new(handle_native_modifier_release),
            pointer: Arc::new(handle_native_pointer_hook),
            pointer_reconcile: Arc::new(handle_native_pointer_reconcile),
            registered_hotkey: Arc::new(move |id| {
                if id == activation_run.id
                    && let Ok(mut registrations) = activation_registrations.lock()
                {
                    let registration = registrations.1;
                    deliver_registered_hotkey(
                        &mut registrations.0,
                        registration,
                        GlobalShortcutEdge::Activated,
                    );
                }
            }),
            ready: Arc::new(move |result| match result {
                Ok(readiness) => {
                    if !readiness.registered_hotkey {
                        tracing::warn!(
                            "Super+R registration unavailable; using low-level hook fallback"
                        );
                    }
                    if !readiness.pointer_hook {
                        tracing::warn!(
                            "Windows pointer hook unavailable; Super+pointer gestures disabled"
                        );
                    }
                    if let Ok(mut registrations) = ready_registrations.lock() {
                        let registration = native_registration(
                            &mut registrations.0,
                            readiness.registered_hotkey,
                            ready_run.key,
                            [AggregateModifier::Super],
                            ready_run.action,
                        );
                        registrations.1 = registration;
                    }
                    let available = nickel_input::global::ShortcutCapability::Available;
                    if let Ok(mut state) = ready_diagnostic_state.lock() {
                        state.capability = available.clone();
                        state.registration_revision = Some(1);
                    }
                    let _ = startup.send(available);
                }
                Err(error) => {
                    let unavailable = nickel_input::global::ShortcutCapability::Unavailable(
                        nickel_input::global::UnavailableReason::Backend(error),
                    );
                    if let Ok(mut state) = ready_diagnostic_state.lock() {
                        state.capability = unavailable.clone();
                        state.registration_revision = None;
                    }
                    let _ = startup.send(unavailable);
                }
            }),
        },
        NativeHotkeyRegistration {
            id: run.id,
            virtual_key: run.virtual_key,
        },
    );
    if let Ok(mut adapter) = windows_input_adapter().lock() {
        adapter.reset();
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
static WINDOWS_INPUT_ADAPTER: std::sync::OnceLock<Mutex<WindowsInputAdapter<HotkeyAction>>> =
    std::sync::OnceLock::new();
static WINDOW_SWITCH_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
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
#[derive(Default)]
struct DwmPreviewState {
    thumbnails: Vec<isize>,
    sources: Vec<usize>,
    presentation_generation: u64,
    presentation_failures: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct NativePreviewDiagnostics {
    pub(crate) presentation_generation: u64,
    pub(crate) presentation_failures: u64,
}

static DWM_PREVIEW_STATE: Mutex<DwmPreviewState> = Mutex::new(DwmPreviewState {
    thumbnails: Vec::new(),
    sources: Vec::new(),
    presentation_generation: 0,
    presentation_failures: 0,
});
static RESTORE_LAUNCHER_FOCUS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static WINDOW_DRAG: LazyLock<Mutex<WindowDragCoordinator>> =
    LazyLock::new(|| Mutex::new(WindowDragCoordinator::default()));
static NATIVE_MOVE_SIZE_OBSERVATION_AVAILABLE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

struct NativeMoveSizeHook {
    move_size: HWINEVENTHOOK,
    window_destroy: HWINEVENTHOOK,
}

impl NativeMoveSizeHook {
    fn install() -> Option<Self> {
        let move_size = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_MOVESIZESTART,
                EVENT_SYSTEM_MOVESIZEEND,
                None,
                Some(native_move_size_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if move_size.is_invalid() {
            return None;
        }
        let window_destroy = unsafe {
            SetWinEventHook(
                EVENT_OBJECT_DESTROY,
                EVENT_OBJECT_DESTROY,
                None,
                Some(native_window_destroyed),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if window_destroy.is_invalid() {
            unsafe {
                let _ = UnhookWinEvent(move_size);
            }
            return None;
        }
        Some(Self {
            move_size,
            window_destroy,
        })
    }
}

impl Drop for NativeMoveSizeHook {
    fn drop(&mut self) {
        NATIVE_MOVE_SIZE_OBSERVATION_AVAILABLE.store(false, Ordering::Release);
        unsafe {
            let _ = UnhookWinEvent(self.move_size);
            let _ = UnhookWinEvent(self.window_destroy);
        }
    }
}

unsafe extern "system" fn native_window_destroyed(
    _: HWINEVENTHOOK,
    _: u32,
    window: HWND,
    object_id: i32,
    child_id: i32,
    _: u32,
    _: u32,
) {
    // OBJID_WINDOW is zero and CHILDID_SELF is zero. Other object teardown does not end the HWND.
    if window.is_invalid() || object_id != 0 || child_id != 0 {
        return;
    }
    if let Ok(mut coordinator) = WINDOW_DRAG.lock() {
        coordinator.window_destroyed(window.0 as isize);
    }
}

unsafe extern "system" fn native_move_size_event(
    _: HWINEVENTHOOK,
    event: u32,
    window: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    if window.is_invalid() {
        return;
    }
    let started = event == EVENT_SYSTEM_MOVESIZESTART;
    if !started && event != EVENT_SYSTEM_MOVESIZEEND {
        return;
    }
    if let Ok(mut coordinator) = WINDOW_DRAG.lock() {
        coordinator.native_move_size(window.0 as isize, started, unsafe { GetTickCount64() });
    }
}
const PANEL_APPBAR_CALLBACK: u32 = 0x8000 + 17;
const ABN_FULLSCREENAPP_CODE: usize = 2;

#[derive(Clone)]
struct WindowDrag {
    operation: OperationId,
    completion: CompletionBinding,
    window: isize,
    lifetime: NativeWindowLifetime,
    start: POINT,
    resize_edge: Option<u32>,
    initiated_at: u64,
    last_update: u64,
    last_apply: NativeApplyState,
    authority: GeometryAuthority,
    settlement: Option<Settlement>,
    last_observed: LogicalRect,
}

struct RetainedNativeSettlement {
    lifetime: NativeWindowLifetime,
    authority: GeometryAuthority,
    settlement: Settlement,
    last_observed: LogicalRect,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct NativeWindowFingerprint {
    window: isize,
    process_id: u32,
    thread_id: u32,
    process_created: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct NativeWindowLifetime {
    fingerprint: NativeWindowFingerprint,
    generation: u64,
}

type RetainedSettlementKey = (NativeWindowLifetime, NativeRequestId);

const MAX_RETAINED_WINDOW_SETTLEMENTS: usize = 64;
const MAX_TERMINAL_SETTLEMENT_OUTCOMES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TerminalSettlementOutcome {
    key: RetainedSettlementKey,
    status: SettlementStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettlementRetentionOutcome {
    Retained,
    EvictedOldest { terminal: TerminalSettlementOutcome },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeApplyState {
    NotSubmitted,
    AcceptedSettlementUnknown,
    Rejected,
}

#[derive(Default)]
struct WindowDragCoordinator {
    reducer: WindowOperationReducer,
    active: Option<WindowDrag>,
    next_mapping_generation: u64,
    next_native_request: u64,
    current_lifetimes: HashMap<isize, NativeWindowLifetime>,
    retained_settlements: HashMap<RetainedSettlementKey, RetainedNativeSettlement>,
    retained_order: VecDeque<RetainedSettlementKey>,
    terminal_settlement_outcomes: VecDeque<TerminalSettlementOutcome>,
    last_retention_outcome: Option<SettlementRetentionOutcome>,
    last_terminal_apply: Option<NativeApplyState>,
}

#[derive(Clone, Copy)]
struct WindowDragAdmission {
    fingerprint: NativeWindowFingerprint,
    start: POINT,
    rectangle: RECT,
    resize_edge: Option<u32>,
    initiating_button: u16,
    time: u64,
}

impl WindowDragCoordinator {
    fn record_terminal_settlement(
        &mut self,
        key: RetainedSettlementKey,
        status: SettlementStatus,
    ) -> TerminalSettlementOutcome {
        debug_assert_ne!(status, SettlementStatus::Pending);
        let outcome = TerminalSettlementOutcome { key, status };
        if self.terminal_settlement_outcomes.len() >= MAX_TERMINAL_SETTLEMENT_OUTCOMES {
            self.terminal_settlement_outcomes.pop_front();
        }
        self.terminal_settlement_outcomes.push_back(outcome);
        outcome
    }

    fn lifetime_is_current(&self, lifetime: NativeWindowLifetime) -> bool {
        self.current_lifetimes
            .get(&lifetime.fingerprint.window)
            .is_some_and(|current| *current == lifetime)
    }

    fn retain_settlement(
        &mut self,
        key: RetainedSettlementKey,
        retained: RetainedNativeSettlement,
    ) {
        let outcome = if self.retained_settlements.len() >= MAX_RETAINED_WINDOW_SETTLEMENTS {
            let evicted = self
                .retained_order
                .pop_front()
                .expect("a full retained-settlement map has an order entry");
            let displaced = self
                .retained_settlements
                .remove(&evicted)
                .expect("a retained-settlement order entry has a settlement");
            let terminal = self.fail_retained(evicted, displaced);
            tracing::warn!(
                ?evicted,
                "evicted oldest pending native settlement at capacity"
            );
            SettlementRetentionOutcome::EvictedOldest { terminal }
        } else {
            SettlementRetentionOutcome::Retained
        };
        self.retained_order.push_back(key);
        self.retained_settlements.insert(key, retained);
        self.last_retention_outcome = Some(outcome);
    }

    fn take_retained(&mut self, key: RetainedSettlementKey) -> Option<RetainedNativeSettlement> {
        self.retained_order.retain(|candidate| *candidate != key);
        self.retained_settlements.remove(&key)
    }

    fn finish_retained(
        &mut self,
        key: RetainedSettlementKey,
        mut retained: RetainedNativeSettlement,
    ) -> TerminalSettlementOutcome {
        debug_assert_ne!(retained.settlement.status, SettlementStatus::Pending);
        retained.authority.base_placement.control = ControlMode::Delegated;
        let outcome = self.record_terminal_settlement(key, retained.settlement.status);
        if self
            .current_lifetimes
            .get(&retained.lifetime.fingerprint.window)
            == Some(&retained.lifetime)
        {
            self.current_lifetimes
                .remove(&retained.lifetime.fingerprint.window);
        }
        outcome
    }

    fn fail_retained(
        &mut self,
        key: RetainedSettlementKey,
        mut retained: RetainedNativeSettlement,
    ) -> TerminalSettlementOutcome {
        retained.settlement.fail();
        self.finish_retained(key, retained)
    }

    fn admit(&mut self, admission: WindowDragAdmission) -> bool {
        let window = admission.fingerprint.window;
        if self.active.is_some()
            || self
                .retained_settlements
                .values()
                .any(|retained| retained.lifetime.fingerprint == admission.fingerprint)
        {
            return false;
        }
        if let Some(last_apply) = self.last_terminal_apply {
            tracing::trace!(
                ?last_apply,
                "previous foreign-window operation native apply state"
            );
        }
        let Some(kind) = operation_kind(admission.resize_edge) else {
            return false;
        };
        let completion = CompletionBinding {
            source: windows_pointer_source(),
            gesture: CompletionGesture::Button(admission.initiating_button),
        };
        self.next_mapping_generation = self.next_mapping_generation.saturating_add(1);
        let mapping_generation = self.next_mapping_generation;
        let lifetime = NativeWindowLifetime {
            fingerprint: admission.fingerprint,
            generation: mapping_generation,
        };
        let rectangle = admission.rectangle;
        let (Some(operation), begin) = self.reducer.begin_with_geometry(
            BeginRequest {
                seat: SeatId::new(1),
                subject: WindowMapping {
                    window: OperationWindowId::new(window as usize as u64),
                    native_lifetime: NativeLifetimeId::new(mapping_generation),
                    generation: MappingGeneration::new(mapping_generation),
                },
                kind,
                control: ControlMode::ExternallyContested,
                origin: completion,
                optional_update_sources: Vec::new(),
            },
            GeometrySeed {
                anchor: LogicalRect {
                    x: rectangle.left,
                    y: rectangle.top,
                    width: rectangle.right - rectangle.left,
                    height: rectangle.bottom - rectangle.top,
                },
                constraints: GeometryConstraints {
                    min_width: 120,
                    min_height: 80,
                    max_width: None,
                    max_height: None,
                },
            },
        ) else {
            return false;
        };
        let [WindowOperationEffect::Acquire { request, .. }] = begin.effects.as_slice() else {
            self.reducer
                .cancel(operation, CancellationReason::AcquisitionFailed);
            return false;
        };
        let request = *request;
        if self
            .reducer
            .acquired(operation, request, ResourceLeaseId::new(operation.get()))
            .disposition
            != Disposition::Applied
            || self.reducer.activate(operation).disposition != Disposition::Applied
        {
            self.reducer
                .cancel(operation, CancellationReason::AcquisitionFailed);
            return false;
        }
        self.current_lifetimes.insert(window, lifetime);
        self.active = Some(WindowDrag {
            operation,
            completion,
            window,
            lifetime,
            start: admission.start,
            resize_edge: admission.resize_edge,
            initiated_at: admission.time,
            last_update: admission.time,
            last_apply: NativeApplyState::NotSubmitted,
            authority: contested_authority(admission.rectangle),
            settlement: None,
            last_observed: logical_rect(admission.rectangle),
        });
        true
    }

    fn update(&mut self, pointer: POINT, time: u64) -> Result<(), ()> {
        let Some(mut active) = self.active.take() else {
            return Err(());
        };
        if !self.lifetime_is_current(active.lifetime)
            || native_window_fingerprint(active.window) != Some(active.lifetime.fingerprint)
        {
            let _ = self
                .reducer
                .cancel(active.operation, CancellationReason::TargetDestroyed);
            self.last_terminal_apply = Some(active.last_apply);
            if self.current_lifetimes.get(&active.window) == Some(&active.lifetime) {
                self.current_lifetimes.remove(&active.window);
            }
            return Err(());
        }
        match observe_window_drag(&mut active, time, false) {
            Err(reason) => {
                let _ = self.reducer.cancel(active.operation, reason);
                self.last_terminal_apply = Some(active.last_apply);
                self.current_lifetimes.remove(&active.window);
                return Err(());
            }
            Ok(false) => {
                self.active = Some(active);
                return Ok(());
            }
            Ok(true) => {}
        }
        let transition = self.reducer.update_geometry(
            active.operation,
            active.completion.source,
            GeometryUpdate::AbsoluteDisplacement {
                x: i64::from(pointer.x) - i64::from(active.start.x),
                y: i64::from(pointer.y) - i64::from(active.start.y),
            },
        );
        let Some(rectangle) = transition.effects.iter().find_map(|effect| match effect {
            WindowOperationEffect::GeometryProposed { constrained, .. } => Some(*constrained),
            _ => None,
        }) else {
            self.active = Some(active);
            return Err(());
        };
        self.next_native_request = self.next_native_request.saturating_add(1);
        active.last_apply = apply_window_drag(
            &mut active,
            rectangle,
            self.next_native_request,
            self.next_mapping_generation,
            time,
        );
        active.last_update = time;
        self.active = Some(active);
        if active.last_apply == NativeApplyState::Rejected {
            self.reducer
                .fail(active.operation, FailureReason::NativeApplyFailed);
            self.last_terminal_apply = Some(active.last_apply);
            self.active = None;
            self.current_lifetimes.remove(&active.window);
            Err(())
        } else {
            Ok(())
        }
    }

    fn release(&mut self, binding: CompletionBinding) {
        let Some(mut active) = self.active.take() else {
            return;
        };
        if self.reducer.release(active.operation, binding).disposition == Disposition::Applied {
            if let Some(settlement) = active.settlement.take() {
                let key = (active.lifetime, settlement.request.id);
                self.retain_settlement(
                    key,
                    RetainedNativeSettlement {
                        lifetime: active.lifetime,
                        authority: active.authority.clone(),
                        settlement,
                        last_observed: active.last_observed,
                    },
                );
            } else {
                self.current_lifetimes.remove(&active.window);
            }
            self.last_terminal_apply = Some(active.last_apply);
            self.active = None;
        } else {
            self.active = Some(active);
        }
    }

    fn cancel(&mut self, reason: CancellationReason) {
        if let Some(active) = self.active.take() {
            let _ = self.reducer.cancel(active.operation, reason);
            self.last_terminal_apply = Some(active.last_apply);
            self.current_lifetimes.remove(&active.window);
        }
    }

    fn observe_retained_settlement(&mut self, now: u64) {
        let keys = self
            .retained_settlements
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for key in keys {
            let Some(mut retained) = self.take_retained(key) else {
                continue;
            };
            if !self.lifetime_is_current(retained.lifetime)
                || native_window_fingerprint(retained.lifetime.fingerprint.window)
                    != Some(retained.lifetime.fingerprint)
            {
                self.fail_retained(key, retained);
                continue;
            }
            let mut rectangle = RECT::default();
            let window = HWND(retained.lifetime.fingerprint.window as *mut c_void);
            if unsafe { GetWindowRect(window, &mut rectangle) }.is_err() {
                self.fail_retained(key, retained);
                continue;
            }
            let observed = logical_rect(rectangle);
            let fact = native_geometry(observed);
            retained
                .settlement
                .observe(fact, ObservationCausality::Unknown);
            retained.settlement.expire(now);
            if retained.settlement.status == SettlementStatus::Pending {
                retained.last_observed = observed;
                self.retained_order.push_back(key);
                self.retained_settlements.insert(key, retained);
            } else {
                retained
                    .authority
                    .observe(fact, ObservationCausality::Unknown);
                self.finish_retained(key, retained);
            }
        }
    }

    fn native_move_size(&mut self, window: isize, started: bool, now: u64) {
        if started {
            if self
                .active
                .as_ref()
                .is_some_and(|active| active.window == window)
            {
                self.cancel(CancellationReason::NativeTakeover);
            }
            let keys = self
                .retained_settlements
                .iter()
                .filter_map(|(key, retained)| {
                    (retained.lifetime.fingerprint.window == window).then_some(*key)
                })
                .collect::<Vec<_>>();
            for key in keys {
                let Some(mut retained) = self.take_retained(key) else {
                    continue;
                };
                let fact = native_geometry(retained.last_observed);
                retained
                    .settlement
                    .observe(fact, ObservationCausality::Independent);
                retained
                    .authority
                    .observe(fact, ObservationCausality::Independent);
                self.finish_retained(key, retained);
            }
        } else if self
            .retained_settlements
            .values()
            .any(|retained| retained.lifetime.fingerprint.window == window)
        {
            self.observe_retained_settlement(now);
        }
    }

    fn window_destroyed(&mut self, window: isize) {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.window == window)
        {
            self.cancel(CancellationReason::TargetDestroyed);
        }
        self.current_lifetimes.remove(&window);
        let keys = self
            .retained_settlements
            .keys()
            .copied()
            .filter(|(lifetime, _)| lifetime.fingerprint.window == window)
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(retained) = self.take_retained(key) {
                self.fail_retained(key, retained);
            }
        }
    }
}

fn native_window_fingerprint(window: isize) -> Option<NativeWindowFingerprint> {
    let hwnd = HWND(window as *mut c_void);
    if unsafe { !IsWindow(hwnd).as_bool() } {
        return None;
    }
    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    if thread_id == 0 || process_id == 0 {
        return None;
    }
    let process =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let result =
        unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
    unsafe {
        let _ = CloseHandle(process);
    }
    if result.is_err() {
        return None;
    }
    Some(NativeWindowFingerprint {
        window,
        process_id,
        thread_id,
        process_created: (u64::from(created.dwHighDateTime) << 32)
            | u64::from(created.dwLowDateTime),
    })
}

fn windows_pointer_source() -> Source {
    Source {
        id: SourceId::new(1),
        generation: SourceGeneration::new(1),
    }
}

fn pointer_release_binding(kind: NativePointerKind) -> Option<CompletionBinding> {
    let button = match kind {
        NativePointerKind::PrimaryReleased => 1,
        NativePointerKind::SecondaryReleased => 2,
        _ => return None,
    };
    Some(CompletionBinding {
        source: windows_pointer_source(),
        gesture: CompletionGesture::Button(button),
    })
}

fn completion_button_physically_held(binding: CompletionBinding) -> bool {
    let CompletionGesture::Button(button) = binding.gesture else {
        return false;
    };
    let virtual_key = if button == 1 { 0x01 } else { 0x02 };
    // SAFETY: this is a read-only physical button-state query on the hook thread.
    unsafe { GetAsyncKeyState(virtual_key) < 0 }
}

fn operation_kind(resize_edge: Option<u32>) -> Option<OperationKind> {
    let Some(edge) = resize_edge else {
        return Some(OperationKind::Move);
    };
    let horizontal = if matches!(edge, HTLEFT | HTTOPLEFT | HTBOTTOMLEFT) {
        Some(HorizontalEdge::Left)
    } else if matches!(edge, HTRIGHT | HTTOPRIGHT | HTBOTTOMRIGHT) {
        Some(HorizontalEdge::Right)
    } else {
        None
    };
    let vertical = if matches!(edge, HTTOP | HTTOPLEFT | HTTOPRIGHT) {
        Some(VerticalEdge::Top)
    } else if matches!(edge, HTBOTTOM | HTBOTTOMLEFT | HTBOTTOMRIGHT) {
        Some(VerticalEdge::Bottom)
    } else {
        None
    };
    ResizeEdges::new(horizontal, vertical)
        .ok()
        .map(OperationKind::Resize)
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

fn handle_native_keyboard_hook(
    event: NativeKeyboardEvent,
    registered_hotkey_owned: bool,
    alt_physically_held: bool,
) -> HookDisposition {
    crate::windows_remote_control::observe_physical_key(event);
    // Alt changes the layout-translated virtual key for the physical grave key on some layouts
    // (for example to VK_HANJA). Preserve the physical shortcut using its stable scan code.
    let translated = physical_key(event.virtual_key, event.scan_code, event.extended);
    let key = match translated {
        PhysicalKey::Code(key) => Some(key),
        PhysicalKey::Native(_) => None,
    };
    let super_edge = matches!(key, Some(KeyCode::SuperLeft | KeyCode::SuperRight));
    if key == Some(KeyCode::KeyR) && registered_hotkey_owned {
        if let Ok(mut adapter) = windows_input_adapter().lock() {
            // RegisterHotKey owns Super+R dispatch. The hook only records that another key joined
            // the Super press, preventing the later release from toggling the launcher.
            adapter.observe_key_code(KeyCode::KeyR, event.edge);
        }
        // RegisterHotKey owns dispatch, while the hook owns suppression. The
        // shared adapter observes both edges only to maintain coherent state;
        // it cannot dispatch a second action on this path.
        return HookDisposition::Suppress;
    }
    if matches!(
        key,
        Some(KeyCode::Tab | KeyCode::Backquote | KeyCode::PrintScreen)
    ) && event.edge == KeyEdge::Pressed
        && alt_physically_held
        && let Ok(mut adapter) = windows_input_adapter().lock()
        && !adapter.modifier_held(AggregateModifier::Alt)
    {
        adapter.observe_key_code(KeyCode::AltLeft, KeyEdge::Pressed);
    }
    let outcomes = windows_input_adapter()
        .lock()
        .map(|mut adapter| {
            adapter
                .handle_native(event)
                .map(|dispatch| dispatch.outcomes)
                .unwrap_or_default()
        })
        .unwrap_or_default();
    // The shared modifier-release binding deliberately dispatches only on the
    // release edge. Ownership still covers the preceding Super press so the
    // native Start menu cannot open alongside Nickel's launcher.
    let suppress = super_edge || outcomes.iter().any(|outcome| outcome.suppress);
    send_hotkey_outcomes(outcomes);
    if suppress {
        HookDisposition::Suppress
    } else {
        HookDisposition::Forward
    }
}

fn handle_native_modifier_release(modifier: AggregateModifier) {
    let keys = match modifier {
        AggregateModifier::Super => [KeyCode::SuperLeft, KeyCode::SuperRight],
        AggregateModifier::Alt => [KeyCode::AltLeft, KeyCode::AltRight],
        _ => return,
    };
    let actions = windows_input_adapter()
        .lock()
        .ok()
        .map(|mut adapter| {
            keys.into_iter()
                .flat_map(|key| adapter.handle_key_code(key, KeyEdge::Released).outcomes)
                .collect()
        })
        .unwrap_or_default();
    send_hotkey_outcomes(actions);
}

fn windows_input_adapter() -> &'static Mutex<WindowsInputAdapter<HotkeyAction>> {
    WINDOWS_INPUT_ADAPTER.get_or_init(|| {
        let bindings: Vec<_> = nickel_core::hotkeys::default_bindings()
            .into_iter()
            .filter(|binding| !is_host_owned_action(binding.action))
            .collect();
        tracing::info!(
            "workspace shortcuts are host-owned on Windows; Nickel leaves them unregistered"
        );
        Mutex::new(WindowsInputAdapter::new(bindings))
    })
}

fn is_host_owned_action(action: HotkeyAction) -> bool {
    matches!(
        action,
        HotkeyAction::SwitchWorkspacePrevious
            | HotkeyAction::SwitchWorkspaceNext
            | HotkeyAction::SwitchWorkspace(_)
            | HotkeyAction::MoveWindowToPreviousWorkspace
            | HotkeyAction::MoveWindowToNextWorkspace
            | HotkeyAction::CreateWorkspace
            | HotkeyAction::RemoveActiveWorkspace
            | HotkeyAction::CloseActiveWindow
            | HotkeyAction::SnapLeading
            | HotkeyAction::SnapTrailing
            | HotkeyAction::MaximizeActiveWindow
            | HotkeyAction::RestoreOrMinimizeActiveWindow
            | HotkeyAction::MoveWindowToPreviousOutput
            | HotkeyAction::MoveWindowToNextOutput
            | HotkeyAction::ShowDesktop
            | HotkeyAction::ProjectDisplays
    )
}

fn send_hotkey_action(action: Option<HotkeyAction>) {
    let shortcut = match action {
        Some(HotkeyAction::LockSession) => GlobalShortcut::LockState { locked: true },
        Some(HotkeyAction::ToggleLauncher) => GlobalShortcut::ToggleLauncher,
        Some(HotkeyAction::ShowRun) => GlobalShortcut::ShowRun,
        Some(HotkeyAction::OpenFiles) => GlobalShortcut::OpenFiles,
        Some(HotkeyAction::OpenSettings) => GlobalShortcut::OpenSettings,
        Some(HotkeyAction::ShowControlCenter) => GlobalShortcut::ShowControlCenter,
        Some(HotkeyAction::ShowNotifications) => GlobalShortcut::ShowNotifications,
        Some(HotkeyAction::ShowWindowMenu) => GlobalShortcut::ShowWindowMenu,
        Some(HotkeyAction::SwitchNext) => GlobalShortcut::SwitchNext,
        Some(HotkeyAction::SwitchPrevious) => GlobalShortcut::SwitchPrevious,
        Some(HotkeyAction::SwitchGroupNext) => GlobalShortcut::SwitchGroupNext,
        Some(HotkeyAction::SwitchGroupPrevious) => GlobalShortcut::SwitchGroupPrevious,
        Some(HotkeyAction::CommitSwitch) => GlobalShortcut::CommitSwitch,
        Some(HotkeyAction::CancelSwitch) => GlobalShortcut::CancelSwitch,
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
            | HotkeyAction::SwitchWorkspace(_)
            | HotkeyAction::MoveWindowToPreviousWorkspace
            | HotkeyAction::MoveWindowToNextWorkspace
            | HotkeyAction::CreateWorkspace
            | HotkeyAction::RemoveActiveWorkspace
            | HotkeyAction::CloseActiveWindow
            | HotkeyAction::SnapLeading
            | HotkeyAction::SnapTrailing
            | HotkeyAction::MaximizeActiveWindow
            | HotkeyAction::RestoreOrMinimizeActiveWindow
            | HotkeyAction::MoveWindowToPreviousOutput
            | HotkeyAction::MoveWindowToNextOutput
            | HotkeyAction::ShowDesktop
            | HotkeyAction::ProjectDisplays,
        ) => return,
        None => return,
    };
    tracing::debug!(?shortcut, "dispatching Windows global shortcut");
    if let Some(sender) = SHORTCUT_SENDER.get()
        && sender.send(shortcut).is_err()
    {
        tracing::warn!("Windows global shortcut receiver disconnected");
    }
}

fn send_hotkey_outcomes(outcomes: Vec<nickel_input::ShortcutOutcome<HotkeyAction>>) {
    for outcome in outcomes {
        match outcome.action {
            HotkeyAction::SwitchNext
            | HotkeyAction::SwitchPrevious
            | HotkeyAction::SwitchGroupNext
            | HotkeyAction::SwitchGroupPrevious => {
                WINDOW_SWITCH_ACTIVE.store(true, Ordering::Release);
            }
            HotkeyAction::CommitSwitch if !WINDOW_SWITCH_ACTIVE.swap(false, Ordering::AcqRel) => {
                continue;
            }
            _ => {}
        }
        send_hotkey_action(Some(outcome.action));
    }
}

fn handle_native_pointer_hook(event: NativePointerEvent) -> HookDisposition {
    crate::windows_remote_control::observe_physical_pointer(event);
    // Injected hook traffic is not the physical Windows pointer source and may
    // neither start, update, nor complete its operation binding.
    if event.injected {
        return HookDisposition::Forward;
    }
    if !permits_contested_workflow(NATIVE_MOVE_SIZE_OBSERVATION_AVAILABLE.load(Ordering::Acquire)) {
        if let Ok(mut coordinator) = WINDOW_DRAG.lock() {
            coordinator.cancel(CancellationReason::AuthorityUnknown);
        }
        return HookDisposition::Forward;
    }
    let point = POINT {
        x: event.x,
        y: event.y,
    };
    let now = unsafe { GetTickCount64() };
    if let Ok(mut coordinator) = WINDOW_DRAG.lock()
        && let Some(operation) = coordinator.active.clone()
    {
        let release = pointer_release_binding(event.kind);
        let current_release = release == Some(operation.completion);
        if event.kind == NativePointerKind::Moved || current_release {
            if event.kind == NativePointerKind::Moved
                && now.saturating_sub(operation.initiated_at) >= 250
                && !completion_button_physically_held(operation.completion)
            {
                // Low-level button-up delivery is not infallible. Once another
                // hook event proves the initiating button has been released,
                // terminate instead of allowing an unbounded stuck drag.
                coordinator.cancel(CancellationReason::CompletionSourceLost);
                return HookDisposition::Forward;
            }
            // Moving is inexpensive and should track the compositor closely. Resizing can make
            // applications such as Windows Terminal reflow and redraw their entire contents, so
            // retain a modest cap there without making ordinary dragging feel like 30 FPS.
            let minimum_interval = if operation.resize_edge.is_some() {
                16
            } else {
                8
            };
            if current_release || now.saturating_sub(operation.last_update) >= minimum_interval {
                if coordinator.update(point, now).is_err() {
                    return if current_release {
                        HookDisposition::Suppress
                    } else {
                        HookDisposition::Forward
                    };
                }
            }
            if current_release {
                coordinator.release(operation.completion);
                return HookDisposition::Suppress;
            }
            // Observe pointer motion without consuming it. Suppressing WM_MOUSEMOVE freezes the
            // real cursor while the window chases coordinates reported by the hook.
            return HookDisposition::Forward;
        }
        if let Some(binding) = release {
            // Route unrelated releases through the reducer. They neither end
            // the operation nor consume the native event.
            let _ = coordinator.reducer.release(operation.operation, binding);
            return HookDisposition::Forward;
        }
    }
    if !matches!(
        event.kind,
        NativePointerKind::PrimaryPressed | NativePointerKind::SecondaryPressed
    ) {
        return HookDisposition::Forward;
    }
    let physical_super = event.super_physically_held;
    let (super_held, gesture) = windows_input_adapter()
        .lock()
        .map(|mut adapter| {
            // Mouse and keyboard low-level hooks are delivered independently. A mouse-down can
            // win the startup/event-order race before Nickel observes Super-down, so reconcile
            // from Windows' physical state at the gesture boundary.
            if physical_super && !adapter.modifier_held(AggregateModifier::Super) {
                adapter.observe_key_code(KeyCode::SuperLeft, KeyEdge::Pressed);
            }
            let super_held = adapter.modifier_held(AggregateModifier::Super);
            let gesture =
                adapter.begin_pointer_gesture(if event.kind == NativePointerKind::PrimaryPressed {
                    PointerButton::Primary
                } else {
                    PointerButton::Secondary
                });
            (super_held, gesture)
        })
        .unwrap_or_default();
    let chord_started = gesture.is_some();
    tracing::debug!(
        super_held,
        physical_super,
        chord_started,
        button = if event.kind == NativePointerKind::PrimaryPressed {
            "left"
        } else {
            "right"
        },
        "Super mouse gesture candidate"
    );
    if !chord_started {
        return HookDisposition::Forward;
    }
    let gesture = gesture.expect("a started pointer chord has a typed gesture");

    let target = unsafe { GetAncestor(WindowFromPoint(point), GA_ROOT) };
    if target.0.is_null() {
        return HookDisposition::Forward;
    }
    let mut process_id = 0;
    unsafe {
        GetWindowThreadProcessId(target, Some(&mut process_id));
    }
    if process_id == unsafe { GetCurrentProcessId() } {
        return HookDisposition::Forward;
    }

    let mut rectangle = RECT::default();
    if unsafe { GetWindowRect(target, &mut rectangle) }.is_err() {
        return HookDisposition::Forward;
    }
    let resize_edge =
        (gesture == SuperPointerGesture::Resize).then(|| resize_hit_test(target, point));
    let initiating_button = if event.kind == NativePointerKind::PrimaryPressed {
        1
    } else {
        2
    };
    let admitted = WINDOW_DRAG.lock().is_ok_and(|mut coordinator| {
        let Some(fingerprint) = native_window_fingerprint(target.0 as isize) else {
            return false;
        };
        coordinator.admit(WindowDragAdmission {
            fingerprint,
            start: point,
            rectangle,
            resize_edge,
            initiating_button,
            time: now,
        })
    });
    if !admitted {
        return HookDisposition::Forward;
    }
    unsafe {
        let _ = SetForegroundWindow(target);
    }
    HookDisposition::Suppress
}

const fn permits_contested_workflow(native_ownership_observation: bool) -> bool {
    native_ownership_observation
}

fn handle_native_pointer_reconcile(primary_held: bool, secondary_held: bool) {
    let Ok(mut coordinator) = WINDOW_DRAG.lock() else {
        return;
    };
    let observation = coordinator
        .active
        .as_mut()
        .map(|active| observe_window_drag(active, unsafe { GetTickCount64() }, false));
    match observation {
        Some(Err(reason)) => {
            coordinator.cancel(reason);
            return;
        }
        Some(Ok(false)) => return,
        _ => {}
    }
    coordinator.observe_retained_settlement(unsafe { GetTickCount64() });
    let Some(active) = coordinator.active.as_ref() else {
        return;
    };
    let held = match active.completion.gesture {
        CompletionGesture::Button(1) => primary_held,
        CompletionGesture::Button(2) => secondary_held,
        _ => false,
    };
    if !held {
        coordinator.cancel(CancellationReason::CompletionSourceLost);
    }
}

fn apply_window_drag(
    operation: &mut WindowDrag,
    rectangle: LogicalRect,
    request_id: u64,
    mapping_generation: u64,
    now: u64,
) -> NativeApplyState {
    let authorized = operation.authority.authorize_placement(
        rectangle,
        GeometryConstraints {
            min_width: 120,
            min_height: 80,
            max_width: None,
            max_height: None,
        },
    );
    operation.settlement = Some(Settlement::new(
        NativeRequest {
            id: NativeRequestId(request_id),
            mapping_generation,
            desired: operation.authority.revisions(),
            placement: authorized.desired,
        },
        SettlementLimits {
            deadline_tick: now.saturating_add(250),
            max_corrections: 0,
        },
    ));
    let window = HWND(operation.window as *mut c_void);
    let accepted = unsafe {
        SetWindowPos(
            window,
            None,
            rectangle.x,
            rectangle.y,
            rectangle.width,
            rectangle.height,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        )
        .is_ok()
    };
    if accepted {
        // ASYNCWINDOWPOS reports queue admission, not native settlement.
        NativeApplyState::AcceptedSettlementUnknown
    } else {
        NativeApplyState::Rejected
    }
}

fn logical_rect(rectangle: RECT) -> LogicalRect {
    LogicalRect {
        x: rectangle.left,
        y: rectangle.top,
        width: (rectangle.right - rectangle.left).max(1),
        height: (rectangle.bottom - rectangle.top).max(1),
    }
}

fn native_geometry(rect: LogicalRect) -> TaggedGeometry {
    TaggedGeometry {
        rect,
        meaning: GeometryMeaning::Win32OuterBounds,
        units: CoordinateUnits::NativePhysical,
        topology_version: 1,
    }
}

fn contested_authority(rectangle: RECT) -> GeometryAuthority {
    let mut authority = GeometryAuthority::new(logical_rect(rectangle), Presentation::Normal);
    authority.base_placement.control = ControlMode::ExternallyContested;
    authority
}

fn classify_window_drag_observation(
    operation: &mut WindowDrag,
    observed: LogicalRect,
    now: u64,
    final_observation: bool,
) -> Result<bool, CancellationReason> {
    let fact = native_geometry(observed);
    if let Some(settlement) = operation.settlement.as_mut() {
        settlement.observe(fact, ObservationCausality::Unknown);
        settlement.expire(now);
        if final_observation || settlement.status == SettlementStatus::Unconfirmed {
            operation
                .authority
                .observe(fact, ObservationCausality::Unknown);
            operation.authority.base_placement.control = ControlMode::Delegated;
            return Err(CancellationReason::AuthorityUnknown);
        }
        return Ok(false);
    }
    if observed != operation.last_observed {
        operation
            .authority
            .observe(fact, ObservationCausality::Independent);
        operation.authority.base_placement.control = ControlMode::Delegated;
        return Err(CancellationReason::NativeTakeover);
    }
    Ok(true)
}

fn observe_window_drag(
    operation: &mut WindowDrag,
    now: u64,
    final_observation: bool,
) -> Result<bool, CancellationReason> {
    let mut rectangle = RECT::default();
    let window = HWND(operation.window as *mut c_void);
    if unsafe { GetWindowRect(window, &mut rectangle) }.is_err() {
        operation.authority.base_placement.control = ControlMode::Delegated;
        return Err(CancellationReason::AuthorityUnknown);
    }
    classify_window_drag_observation(operation, logical_rect(rectangle), now, final_observation)
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
    launch_observed_deferred_terminal(&parts)
}

fn launch_observed_deferred_terminal(arguments: &[String]) -> Result<(), LaunchError> {
    use std::io::Write;

    let mut terminal = super::spawn_deferred_terminal(arguments)?;
    let root_pid = terminal.id();
    let mut decision_input = terminal.stdin.take();
    thread::Builder::new()
        .name("nickel-run-window-observer".into())
        .spawn(move || {
            let started = Instant::now();
            if let Some(input) = decision_input.as_mut() {
                let _ = input.write_all(b"start\n");
                let _ = input.flush();
            }
            while started.elapsed() <= Duration::from_millis(100) {
                let descendants = windows_process_descendants(root_pid);
                if !descendants.is_empty() && has_eligible_window_from(&descendants) {
                    if let Some(input) = decision_input.as_mut() {
                        let _ = input.write_all(b"suppress\n");
                        let _ = input.flush();
                    }
                    return;
                }
                thread::sleep(Duration::from_millis(2));
            }
        })
        .map_err(|error| LaunchError::Platform(error.to_string()))?;
    Ok(())
}

fn windows_process_descendants(root_pid: u32) -> HashSet<u32> {
    const MAX_PROCESSES: usize = 8_192;
    let mut parents = HashMap::new();
    // SAFETY: The snapshot handle is closed below and PROCESSENTRY32W carries its declared size.
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return HashSet::new();
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut result = unsafe { Process32FirstW(snapshot, &raw mut entry) };
    while result.is_ok() && parents.len() < MAX_PROCESSES {
        parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
        result = unsafe { Process32NextW(snapshot, &raw mut entry) };
    }
    // SAFETY: snapshot is the live owned handle returned above.
    let _ = unsafe { CloseHandle(snapshot) };
    parents
        .keys()
        .copied()
        .filter(|pid| windows_pid_descends_from(*pid, root_pid, &parents))
        .collect()
}

fn windows_pid_descends_from(mut pid: u32, root_pid: u32, parents: &HashMap<u32, u32>) -> bool {
    for _ in 0..64 {
        if pid == root_pid {
            return true;
        }
        let Some(parent) = parents.get(&pid).copied() else {
            return false;
        };
        if parent == 0 || parent == pid {
            return false;
        }
        pid = parent;
    }
    false
}

struct PendingWindowSearch<'a> {
    descendants: &'a HashSet<u32>,
    found: bool,
}

fn has_eligible_window_from(descendants: &HashSet<u32>) -> bool {
    let mut search = PendingWindowSearch {
        descendants,
        found: false,
    };
    // SAFETY: The callback is synchronous and state points to a live stack value for the call.
    let _ = unsafe {
        EnumWindows(
            Some(find_pending_launch_window),
            LPARAM((&raw mut search).cast::<c_void>() as isize),
        )
    };
    search.found
}

unsafe extern "system" fn find_pending_launch_window(hwnd: HWND, state: LPARAM) -> BOOL {
    if unsafe { !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() } {
        return BOOL(1);
    }
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    let search = unsafe { &mut *(state.0 as *mut PendingWindowSearch<'_>) };
    if !search.descendants.contains(&process_id) {
        return BOOL(1);
    }
    let Some(class) = window_class(hwnd) else {
        return BOOL(1);
    };
    if window_title(hwnd).is_some() && is_bar_eligible_window(hwnd, &class) {
        search.found = true;
        return BOOL(0);
    }
    BOOL(1)
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
    if let Some(capture) =
        crate::windows_application_registry::native::LaunchCapture::prepare(application)
    {
        return shell_execute_observed(capture);
    }
    shell_execute(target, arguments)?;
    Ok(None)
}

fn shell_execute_observed(
    mut capture: crate::windows_application_registry::native::LaunchCapture,
) -> Result<Option<u32>, LaunchError> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows::Win32::UI::Shell::{
        SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
    };
    let target: Vec<u16> = capture.target().encode_utf16().chain([0]).collect();
    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(|home| {
            home.to_string_lossy()
                .encode_utf16()
                .chain([0])
                .collect::<Vec<_>>()
        });
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("open"),
        lpFile: PCWSTR(target.as_ptr()),
        lpDirectory: home
            .as_ref()
            .map_or(PCWSTR::null(), |home| PCWSTR(home.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: Initialize COM for shell extensions on a launcher worker. A
    // pre-existing apartment is left alone; successful initialization is balanced.
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    // SAFETY: Initialized exact Win32 structure and live NUL-terminated strings.
    // The shortcut file remains pinned by capture across this synchronous call.
    capture.begin_invocation();
    let launched = unsafe { ShellExecuteExW(&mut info) };
    if initialized {
        // SAFETY: Balances the successful initialization above on this thread.
        unsafe { CoUninitialize() };
    }
    launched.map_err(|error| LaunchError::Platform(error.to_string()))?;
    if info.hProcess.is_invalid() {
        return Ok(None);
    }
    // SAFETY: NOCLOSEPROCESS transfers ownership of this successful result. DDE
    // normally return no handle. Creation-time bounds independently reject a
    // returned preexisting process before any application receipt is admitted.
    let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess.0) };
    capture.complete(process);
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
        let result = {
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
        };
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
    // The launcher is a transient shell surface: keep it out of Alt+Tab while allowing it to
    // receive keyboard focus, and place it above fullscreen applications when explicitly opened.
    // Unlike previews it must not use WS_EX_NOACTIVATE because typing belongs to the launcher.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            ((style | WS_EX_TOOLWINDOW.0) & !WS_EX_APPWINDOW.0 & !WS_EX_NOACTIVATE.0) as isize,
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

pub fn launcher_window_visible() -> bool {
    use std::sync::atomic::Ordering;

    let launcher = LAUNCHER_WINDOW_HANDLE.load(Ordering::Relaxed);
    launcher != 0 && unsafe { IsWindowVisible(HWND(launcher as *mut c_void)).as_bool() }
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
        && !foreground.0.is_null()
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
        let procedure = unsafe {
            std::mem::transmute::<
                isize,
                Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>,
            >(previous)
        };
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
                            available: true,
                            volume_percent,
                            muted,
                            output_name: audio_status()
                                .devices
                                .into_iter()
                                .find(|device| device.is_default)
                                .map(|device| device.name),
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
            post_tray_callback(&icon, wparam, lparam);
        } else {
            let wparam = WPARAM(icon.id as usize);
            post_tray_callback(&icon, wparam, LPARAM(legacy_down as isize));
            post_tray_callback(&icon, wparam, LPARAM(legacy_up as isize));
        }
    }
}

fn post_tray_callback(icon: &NativeTrayIcon, wparam: WPARAM, lparam: LPARAM) {
    unsafe {
        if PostMessageW(
            Some(HWND(icon.owner as *mut c_void)),
            icon.callback_message,
            wparam,
            lparam,
        )
        .is_err()
        {
            tracing::warn!("Windows tray callback delivery failed");
        }
    }
}

pub fn send_shell_command(command: ShellCommand) -> bool {
    use std::sync::atomic::Ordering;

    let (window, action) = match command {
        ShellCommand::Show | ShellCommand::ShowFromController => {
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
        ShellCommand::ShowContextMenu {
            x,
            y: _,
            width,
            height,
        } => {
            clear_dwm_thumbnails();
            return show_context_window(x, width, height);
        }
        ShellCommand::ShowPreview {
            x,
            y: _,
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
            WindowAction::Fullscreen => false,
            WindowAction::SnapLeading | WindowAction::SnapTrailing => false,
        }
    }
}

pub fn register_session_shell() -> Result<(), super::SessionRequestError> {
    Ok(())
}

pub fn configured_primary_output() -> Option<String> {
    None
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
        record_dwm_preview_failures(1);
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
            registered.push((thumbnail, source.0 as usize));
        } else {
            unsafe {
                let _ = DwmUnregisterThumbnail(thumbnail);
            }
        }
    }
    let failed = windows.len().saturating_sub(registered.len());
    let success = failed == 0;
    if let Ok(mut state) = DWM_PREVIEW_STATE.lock() {
        state.presentation_failures = state
            .presentation_failures
            .saturating_add(u64::try_from(failed).unwrap_or(u64::MAX));
        state.thumbnails = registered.iter().map(|(thumbnail, _)| *thumbnail).collect();
        state.sources = registered.into_iter().map(|(_, source)| source).collect();
        state.presentation_generation = state.presentation_generation.saturating_add(1);
    }
    success
}

fn record_dwm_preview_failures(count: u64) {
    if let Ok(mut state) = DWM_PREVIEW_STATE.lock() {
        state.presentation_failures = state.presentation_failures.saturating_add(count);
    }
}

pub(crate) fn native_preview_diagnostics(
    allowed_sources: &std::collections::HashSet<usize>,
) -> Option<NativePreviewDiagnostics> {
    DWM_PREVIEW_STATE
        .lock()
        .ok()
        .and_then(|state| project_native_preview_diagnostics(&state, allowed_sources))
}

fn project_native_preview_diagnostics(
    state: &DwmPreviewState,
    allowed_sources: &std::collections::HashSet<usize>,
) -> Option<NativePreviewDiagnostics> {
    (!state.sources.is_empty()
        && state
            .sources
            .iter()
            .all(|source| allowed_sources.contains(source)))
    .then_some(NativePreviewDiagnostics {
        presentation_generation: state.presentation_generation,
        presentation_failures: state.presentation_failures,
    })
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
    let Ok(mut state) = DWM_PREVIEW_STATE.lock() else {
        return;
    };
    if state.thumbnails.is_empty() {
        return;
    }
    for thumbnail in state.thumbnails.drain(..) {
        unsafe {
            let _ = DwmUnregisterThumbnail(thumbnail);
        }
    }
    state.sources.clear();
    state.presentation_generation = state.presentation_generation.saturating_add(1);
}

pub fn launcher_visibility_applied(visible: bool) {
    use std::sync::atomic::Ordering;

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

    pub fn outputs(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn primary_output(&self) -> Option<String> {
        self.outputs().into_iter().next()
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

/// Shared read-only policy for task windows and remote observations. Enumeration
/// callers own their separate bounds; only the bar path parks iconic windows.
fn ordinary_window_metadata(hwnd: HWND) -> Option<(u32, String, String)> {
    // SAFETY: Handle queries do not send input or mutate the target window.
    if unsafe { !IsWindowVisible(hwnd).as_bool() } {
        return None;
    }
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0 || pid == std::process::id() {
        return None;
    }
    let title = window_title(hwnd)?;
    let class = window_class(hwnd)?;
    is_bar_eligible_window(hwnd, &class).then_some((pid, title, class))
}

unsafe extern "system" fn collect_window(hwnd: HWND, state: LPARAM) -> BOOL {
    let Some((_process_id, title, class)) = ordinary_window_metadata(hwnd) else {
        return BOOL(1);
    };
    // This presentation housekeeping belongs only to the existing bar feed.
    if unsafe { IsIconic(hwnd).as_bool() } {
        park_iconic_window(hwnd);
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
        state: crate::model::WindowState {
            // SAFETY: `hwnd` was just validated by EnumWindows for this callback.
            minimized: unsafe { IsIconic(hwnd).as_bool() },
            // SAFETY: `hwnd` was just validated by EnumWindows for this callback.
            maximized: unsafe { IsZoomed(hwnd).as_bool() },
            ..crate::model::WindowState::default()
        },
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
        for _ in 0..64 {
            let popup = GetLastActivePopup(candidate);
            if popup == candidate {
                return candidate == hwnd;
            }
            candidate = popup;
            if IsWindowVisible(candidate).as_bool() {
                return candidate == hwnd;
            }
        }
        false
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
    let mut buffer = vec![0_u16; (length as usize).min(4096) + 1];
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
        .then_some(HICON(handle as *mut c_void))
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

/// Affinity is one prerequisite only: it does not prove trusted z-order,
/// accessibility or exclusion by every future capture implementation.
pub(crate) fn prepare_trusted_control_window(
    window: &impl raw_window_handle::HasWindowHandle,
) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        LWA_ALPHA, SetLayeredWindowAttributes, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
        WS_EX_LAYERED,
    };
    let hwnd = trusted_control_hwnd(window)?;
    // GetWindowDisplayAffinity requires a layered window under DWM. Keep this
    // ordinary opaque UI fully opaque; no input transparency is requested.
    // SAFETY: the caller retains the current-process top-level Window.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED.0 as isize);
        if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_LAYERED.0 as isize == 0 {
            return Err("cannot configure layered trusted window".to_owned());
        }
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)
            .map_err(|error| format!("cannot configure opaque trusted window: {error}"))?;
    }
    // SAFETY: this is a live top-level window borrowed from its winit owner.
    unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) }
        .map_err(|error| format!("cannot exclude trusted window from capture: {error}"))?;
    verify_trusted_control_window(window)
}

/// Winit flag setters rewrite extended styles, dropping the layered bit. The
/// trusted owner therefore shows/raises its retained HWND through this narrow
/// adapter and checks actual native visibility rather than winit's cached flag.
pub(crate) fn expose_trusted_control_window(
    window: &impl raw_window_handle::HasWindowHandle,
    previously_visible: bool,
) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{IsWindowVisible, SWP_SHOWWINDOW};
    verify_trusted_control_window(window)?;
    let hwnd = trusted_control_hwnd(window)?;
    // SAFETY: this live HWND is retained by the owning winit Window.
    unsafe {
        if previously_visible && (!IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool()) {
            return Err("trusted indicator is no longer visible".to_owned());
        }
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
        .map_err(|error| format!("cannot expose trusted indicator: {error}"))?;
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return Err("trusted indicator did not become visible".to_owned());
        }
    }
    verify_trusted_control_window(window)
}

fn trusted_control_hwnd(window: &impl raw_window_handle::HasWindowHandle) -> Result<HWND, String> {
    use windows::Win32::{
        Graphics::Dwm::DwmIsCompositionEnabled,
        UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, GetWindowThreadProcessId},
    };
    let hwnd = window_hwnd(window).ok_or_else(|| "trusted window has no HWND".to_owned())?;
    let mut process = 0;
    // SAFETY: hwnd is borrowed from the live Window and process is writable.
    let valid = unsafe {
        GetWindowThreadProcessId(hwnd, Some(&raw mut process)) != 0
            && process == std::process::id()
            && GetAncestor(hwnd, GA_ROOT) == hwnd
            && DwmIsCompositionEnabled().is_ok_and(|enabled| enabled.as_bool())
    };
    if !valid {
        return Err("trusted window ownership or desktop composition is unavailable".to_owned());
    }
    Ok(hwnd)
}

pub(crate) fn verify_trusted_control_window(
    window: &impl raw_window_handle::HasWindowHandle,
) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
    };
    let hwnd = trusted_control_hwnd(window)?;
    let mut affinity = 0;
    // SAFETY: the owned window is retained and affinity points to writable storage.
    unsafe { GetWindowDisplayAffinity(hwnd, &raw mut affinity) }
        .map_err(|error| format!("cannot verify trusted capture affinity: {error}"))?;
    if affinity != WDA_EXCLUDEFROMCAPTURE.0 {
        return Err("trusted window capture exclusion is unavailable".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use windows::Win32::Foundation::{POINT, RECT};

    use nickel_core::{
        geometry::LogicalRect,
        geometry_authority::{
            ControlMode, FieldOwner, NativeRequest, NativeRequestId, Settlement, SettlementLimits,
        },
        window_operation::{CancellationReason, CompletionBinding, CompletionGesture, OperationId},
    };

    use super::{
        DwmPreviewState, NativeApplyState, NativePreviewDiagnostics, NativeWindowFingerprint,
        NativeWindowLifetime, RetainedNativeSettlement, SettlementRetentionOutcome,
        TerminalSettlementOutcome, TrayNotifyIconData, WindowDrag, WindowDragAdmission,
        WindowDragCoordinator, application_icon, clamp_preview_x, classify_window_drag_observation,
        contain_rect, contested_authority, executable_icon, is_nickel_host_terminal,
        is_shell_infrastructure, native_hotkey_requests, parse_windows_command,
        permits_contested_workflow, project_native_preview_diagnostics, project_windows_shortcuts,
        rectangle_covers, restore_legacy_icon_alpha, should_restore_on_activation,
        windows_pid_descends_from,
    };

    fn fingerprint(window: isize, process_created: u64) -> NativeWindowFingerprint {
        NativeWindowFingerprint {
            window,
            process_id: 7,
            thread_id: 11,
            process_created,
        }
    }

    fn contested_drag() -> WindowDrag {
        let rectangle = RECT {
            left: 10,
            top: 20,
            right: 310,
            bottom: 220,
        };
        WindowDrag {
            operation: OperationId::new(1),
            completion: CompletionBinding {
                source: super::windows_pointer_source(),
                gesture: CompletionGesture::Button(1),
            },
            window: 1,
            lifetime: NativeWindowLifetime {
                fingerprint: fingerprint(1, 13),
                generation: 1,
            },
            start: POINT::default(),
            resize_edge: None,
            initiated_at: 0,
            last_update: 0,
            last_apply: NativeApplyState::NotSubmitted,
            authority: contested_authority(rectangle),
            settlement: None,
            last_observed: LogicalRect {
                x: 10,
                y: 20,
                width: 300,
                height: 200,
            },
        }
    }

    #[test]
    fn independent_native_geometry_revokes_before_another_write() {
        let mut drag = contested_drag();
        let observed = LogicalRect {
            x: 40,
            ..drag.last_observed
        };
        assert_eq!(
            classify_window_drag_observation(&mut drag, observed, 10, false),
            Err(CancellationReason::NativeTakeover)
        );
        assert_eq!(drag.authority.base_placement.owner, FieldOwner::External);
        assert_eq!(
            drag.authority.base_placement.control,
            ControlMode::Delegated
        );
    }

    #[test]
    fn unmatched_native_request_expires_to_unknown_authority() {
        let mut drag = contested_drag();
        let observed = drag.last_observed;
        drag.settlement = Some(Settlement::new(
            NativeRequest {
                id: NativeRequestId(7),
                mapping_generation: 1,
                desired: drag.authority.revisions(),
                placement: LogicalRect { x: 50, ..observed },
            },
            SettlementLimits {
                deadline_tick: 250,
                max_corrections: 0,
            },
        ));
        assert_eq!(
            classify_window_drag_observation(&mut drag, observed, 249, false),
            Ok(false)
        );
        assert_eq!(
            classify_window_drag_observation(&mut drag, observed, 250, false),
            Err(CancellationReason::AuthorityUnknown)
        );
        assert_eq!(drag.authority.base_placement.owner, FieldOwner::Unknown);
        assert_eq!(
            drag.authority.base_placement.control,
            ControlMode::Delegated
        );
    }

    #[test]
    fn equal_async_bounds_do_not_invent_request_causality() {
        let mut drag = contested_drag();
        let observed = drag.last_observed;
        drag.settlement = Some(Settlement::new(
            NativeRequest {
                id: NativeRequestId(8),
                mapping_generation: 1,
                desired: drag.authority.revisions(),
                placement: observed,
            },
            SettlementLimits {
                deadline_tick: 250,
                max_corrections: 0,
            },
        ));
        assert_eq!(
            classify_window_drag_observation(&mut drag, observed, 249, false),
            Ok(false)
        );
        assert!(drag.settlement.is_some());
        assert_eq!(
            classify_window_drag_observation(&mut drag, observed, 250, false),
            Err(CancellationReason::AuthorityUnknown)
        );
    }

    #[test]
    fn contested_workflow_fails_closed_without_native_ownership_hook() {
        assert!(!permits_contested_workflow(false));
        assert!(permits_contested_workflow(true));
    }

    #[test]
    fn release_commits_interaction_and_retains_async_settlement() {
        let mut coordinator = WindowDragCoordinator::default();
        let rectangle = RECT {
            left: 10,
            top: 20,
            right: 310,
            bottom: 220,
        };
        assert!(coordinator.admit(WindowDragAdmission {
            fingerprint: fingerprint(1, 13),
            start: POINT::default(),
            rectangle,
            resize_edge: None,
            initiating_button: 1,
            time: 10,
        }));
        let active = coordinator.active.as_mut().unwrap();
        active.settlement = Some(Settlement::new(
            NativeRequest {
                id: NativeRequestId(9),
                mapping_generation: 1,
                desired: active.authority.revisions(),
                placement: active.last_observed,
            },
            SettlementLimits {
                deadline_tick: 260,
                max_corrections: 0,
            },
        ));
        let completion = active.completion;
        coordinator.release(completion);
        assert!(coordinator.active.is_none());
        assert_eq!(coordinator.retained_settlements.len(), 1);
    }

    #[test]
    fn retained_settlement_does_not_hold_seat_against_unrelated_window() {
        let mut coordinator = WindowDragCoordinator::default();
        let rectangle = RECT {
            left: 10,
            top: 20,
            right: 310,
            bottom: 220,
        };
        assert!(coordinator.admit(WindowDragAdmission {
            fingerprint: fingerprint(1, 13),
            start: POINT::default(),
            rectangle,
            resize_edge: None,
            initiating_button: 1,
            time: 10,
        }));
        let active = coordinator.active.as_mut().unwrap();
        active.settlement = Some(Settlement::new(
            NativeRequest {
                id: NativeRequestId(9),
                mapping_generation: 1,
                desired: active.authority.revisions(),
                placement: active.last_observed,
            },
            SettlementLimits {
                deadline_tick: 260,
                max_corrections: 0,
            },
        ));
        let completion = active.completion;
        coordinator.release(completion);

        assert!(!coordinator.admit(WindowDragAdmission {
            fingerprint: fingerprint(1, 13),
            start: POINT::default(),
            rectangle,
            resize_edge: None,
            initiating_button: 1,
            time: 15,
        }));
        assert!(coordinator.admit(WindowDragAdmission {
            fingerprint: fingerprint(2, 13),
            start: POINT::default(),
            rectangle,
            resize_edge: None,
            initiating_button: 1,
            time: 20,
        }));
        assert_eq!(coordinator.active.as_ref().map(|drag| drag.window), Some(2));
        assert_eq!(coordinator.retained_settlements.len(), 1);
    }

    #[test]
    fn native_move_size_start_supersedes_retained_nickel_settlement() {
        let mut coordinator = WindowDragCoordinator::default();
        let mut drag = contested_drag();
        let settlement = Settlement::new(
            NativeRequest {
                id: NativeRequestId(10),
                mapping_generation: 1,
                desired: drag.authority.revisions(),
                placement: drag.last_observed,
            },
            SettlementLimits {
                deadline_tick: 260,
                max_corrections: 0,
            },
        );
        coordinator
            .current_lifetimes
            .insert(drag.window, drag.lifetime);
        coordinator.retain_settlement(
            (drag.lifetime, settlement.request.id),
            RetainedNativeSettlement {
                lifetime: drag.lifetime,
                authority: std::mem::replace(
                    &mut drag.authority,
                    contested_authority(RECT::default()),
                ),
                settlement,
                last_observed: drag.last_observed,
            },
        );
        coordinator.native_move_size(drag.window, true, 40);
        assert!(coordinator.retained_settlements.is_empty());
        assert_eq!(
            coordinator.terminal_settlement_outcomes.back(),
            Some(&TerminalSettlementOutcome {
                key: (drag.lifetime, NativeRequestId(10)),
                status: SettlementStatus::Superseded,
            })
        );
    }

    #[test]
    fn failed_observation_and_timeout_removals_record_terminal_requests() {
        let mut coordinator = WindowDragCoordinator::default();
        let drag = contested_drag();
        let desired = drag.authority.revisions();
        let failed_key = (drag.lifetime, NativeRequestId(20));
        let failed = RetainedNativeSettlement {
            lifetime: drag.lifetime,
            authority: drag.authority.clone(),
            settlement: Settlement::new(
                NativeRequest {
                    id: failed_key.1,
                    mapping_generation: 1,
                    desired: drag.authority.revisions(),
                    placement: drag.last_observed,
                },
                SettlementLimits {
                    deadline_tick: 50,
                    max_corrections: 0,
                },
            ),
            last_observed: drag.last_observed,
        };
        coordinator.fail_retained(failed_key, failed);

        let timeout_key = (drag.lifetime, NativeRequestId(21));
        let mut timed_out = RetainedNativeSettlement {
            lifetime: drag.lifetime,
            authority: drag.authority,
            settlement: Settlement::new(
                NativeRequest {
                    id: timeout_key.1,
                    mapping_generation: 1,
                    desired,
                    placement: drag.last_observed,
                },
                SettlementLimits {
                    deadline_tick: 50,
                    max_corrections: 0,
                },
            ),
            last_observed: drag.last_observed,
        };
        timed_out.settlement.expire(50);
        coordinator.finish_retained(timeout_key, timed_out);

        assert_eq!(
            coordinator
                .terminal_settlement_outcomes
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![
                TerminalSettlementOutcome {
                    key: failed_key,
                    status: SettlementStatus::Failed,
                },
                TerminalSettlementOutcome {
                    key: timeout_key,
                    status: SettlementStatus::Unconfirmed,
                },
            ]
        );
    }

    #[test]
    fn destroying_window_records_each_retained_request_as_failed() {
        let mut coordinator = WindowDragCoordinator::default();
        let mut drag = contested_drag();
        let key = (drag.lifetime, NativeRequestId(30));
        let settlement = Settlement::new(
            NativeRequest {
                id: key.1,
                mapping_generation: 1,
                desired: drag.authority.revisions(),
                placement: drag.last_observed,
            },
            SettlementLimits {
                deadline_tick: 260,
                max_corrections: 0,
            },
        );
        coordinator
            .current_lifetimes
            .insert(drag.window, drag.lifetime);
        coordinator.retain_settlement(
            key,
            RetainedNativeSettlement {
                lifetime: drag.lifetime,
                authority: std::mem::replace(
                    &mut drag.authority,
                    contested_authority(RECT::default()),
                ),
                settlement,
                last_observed: drag.last_observed,
            },
        );

        coordinator.window_destroyed(drag.window);

        assert!(coordinator.retained_settlements.is_empty());
        assert_eq!(
            coordinator.terminal_settlement_outcomes.back(),
            Some(&TerminalSettlementOutcome {
                key,
                status: SettlementStatus::Failed,
            })
        );
    }

    #[test]
    fn retained_settlement_capacity_evicts_oldest_with_observable_outcome() {
        let mut coordinator = WindowDragCoordinator::default();
        let mut first_key = None;
        for generation in 1..=super::MAX_RETAINED_WINDOW_SETTLEMENTS + 1 {
            let lifetime = NativeWindowLifetime {
                fingerprint: fingerprint(generation as isize, 13),
                generation: generation as u64,
            };
            let mut drag = contested_drag();
            let settlement = Settlement::new(
                NativeRequest {
                    id: NativeRequestId(generation as u64),
                    mapping_generation: generation as u64,
                    desired: drag.authority.revisions(),
                    placement: drag.last_observed,
                },
                SettlementLimits {
                    deadline_tick: 260,
                    max_corrections: 0,
                },
            );
            let key = (lifetime, settlement.request.id);
            first_key.get_or_insert(key);
            coordinator.retain_settlement(
                key,
                RetainedNativeSettlement {
                    lifetime,
                    authority: drag.authority,
                    settlement,
                    last_observed: drag.last_observed,
                },
            );
        }

        assert_eq!(
            coordinator.retained_settlements.len(),
            super::MAX_RETAINED_WINDOW_SETTLEMENTS
        );
        assert!(
            !coordinator
                .retained_settlements
                .contains_key(&first_key.unwrap())
        );
        assert_eq!(
            coordinator.last_retention_outcome,
            Some(SettlementRetentionOutcome::EvictedOldest {
                terminal: TerminalSettlementOutcome {
                    key: first_key.unwrap(),
                    status: SettlementStatus::Failed,
                },
            })
        );
        assert_eq!(
            coordinator.terminal_settlement_outcomes.back(),
            Some(&TerminalSettlementOutcome {
                key: first_key.unwrap(),
                status: SettlementStatus::Failed,
            })
        );
        assert!(
            coordinator.terminal_settlement_outcomes.len()
                <= super::MAX_TERMINAL_SETTLEMENT_OUTCOMES
        );
    }

    #[test]
    fn recycled_window_mapping_cannot_validate_old_settlement_lifetime() {
        let mut coordinator = WindowDragCoordinator::default();
        let old = NativeWindowLifetime {
            fingerprint: fingerprint(44, 13),
            generation: 1,
        };
        let replacement = NativeWindowLifetime {
            fingerprint: fingerprint(44, 21),
            generation: 2,
        };
        coordinator.current_lifetimes.insert(44, old);
        assert!(coordinator.lifetime_is_current(old));

        coordinator.current_lifetimes.insert(44, replacement);
        assert!(!coordinator.lifetime_is_current(old));
        assert!(coordinator.lifetime_is_current(replacement));
    }

    #[test]
    fn terminal_settlement_history_is_bounded_per_request() {
        let mut coordinator = WindowDragCoordinator::default();
        for generation in 1..=super::MAX_TERMINAL_SETTLEMENT_OUTCOMES + 1 {
            let lifetime = NativeWindowLifetime {
                fingerprint: fingerprint(generation as isize, 13),
                generation: generation as u64,
            };
            coordinator.record_terminal_settlement(
                (lifetime, NativeRequestId(generation as u64)),
                SettlementStatus::Unconfirmed,
            );
        }

        assert_eq!(
            coordinator.terminal_settlement_outcomes.len(),
            super::MAX_TERMINAL_SETTLEMENT_OUTCOMES
        );
        assert_eq!(
            coordinator
                .terminal_settlement_outcomes
                .front()
                .map(|outcome| outcome.key.1),
            Some(NativeRequestId(2))
        );
    }

    #[test]
    fn shortcut_diagnostic_projects_only_confirmed_fixed_registration_metadata() {
        use nickel_core::hotkeys::HotkeyAction;
        use nickel_input::{
            Binding, LogicalKey, Shortcut, ShortcutKey, ShortcutTrigger,
            windows::WindowsInputAdapter,
        };
        use nickel_remote_control::diagnostics::ShortcutDiagnosticCapability;

        let adapter = WindowsInputAdapter::new([
            nickel_core::hotkeys::default_bindings()
                .into_iter()
                .next()
                .expect("Nickel has a built-in shortcut"),
            Binding {
                shortcut: Shortcut {
                    key: ShortcutKey::Logical(LogicalKey::Character("typed secret".into())),
                    modifiers: Default::default(),
                    trigger: ShortcutTrigger::Pressed,
                },
                action: HotkeyAction::ShowRun,
                suppress: true,
            },
        ]);
        let snapshot = project_windows_shortcuts(
            7,
            11,
            Some((ShortcutDiagnosticCapability::Available, Some(1))),
            Some(&adapter),
        );
        assert_eq!(snapshot.registration_revision, Some(1));
        assert_eq!(snapshot.registrations.len(), 1);
        assert_eq!(snapshot.unprojected_bindings, 1);
        assert!(snapshot.registrations.iter().all(|registration| {
            registration.physical_key != "typed secret"
                && registration.action != "typed secret"
                && registration
                    .modifiers
                    .iter()
                    .all(|modifier| modifier != "typed secret")
                && registration.trigger != "typed secret"
        }));
    }

    #[test]
    fn unavailable_shortcut_owner_cannot_publish_configured_bindings() {
        use nickel_input::windows::WindowsInputAdapter;
        use nickel_remote_control::diagnostics::ShortcutDiagnosticCapability;

        let adapter = WindowsInputAdapter::new(nickel_core::hotkeys::default_bindings());
        let snapshot = project_windows_shortcuts(
            7,
            11,
            Some((ShortcutDiagnosticCapability::BackendUnavailable, None)),
            Some(&adapter),
        );
        assert_eq!(snapshot.registration_revision, None);
        assert!(snapshot.registrations.is_empty());
    }

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
    fn pending_window_lineage_is_bounded_and_rejects_unrelated_processes() {
        let parents = std::collections::HashMap::from([(30, 20), (20, 10), (40, 1)]);
        assert!(windows_pid_descends_from(10, 10, &parents));
        assert!(windows_pid_descends_from(30, 10, &parents));
        assert!(!windows_pid_descends_from(40, 10, &parents));
        let cycle = std::collections::HashMap::from([(50, 60), (60, 50)]);
        assert!(!windows_pid_descends_from(50, 10, &cycle));
    }

    #[test]
    fn register_hotkey_is_reserved_for_chords_not_bare_super() {
        let requests = native_hotkey_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].virtual_key, 0x52);
        assert_ne!(requests[0].virtual_key, 0x5b);
        assert_ne!(requests[0].virtual_key, 0x5c);
    }

    #[test]
    fn native_preview_diagnostics_require_every_current_source() {
        let state = DwmPreviewState {
            thumbnails: vec![11, 12],
            sources: vec![101, 102],
            presentation_generation: 7,
            presentation_failures: 1,
        };
        let complete = HashSet::from([101, 102]);
        assert_eq!(
            project_native_preview_diagnostics(&state, &complete),
            Some(NativePreviewDiagnostics {
                presentation_generation: 7,
                presentation_failures: 1,
            })
        );
        assert!(project_native_preview_diagnostics(&state, &HashSet::from([101])).is_none());
        assert!(
            project_native_preview_diagnostics(&DwmPreviewState::default(), &complete).is_none()
        );
    }
}
