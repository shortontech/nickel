//! Read window geometry, or apply one explicit frame resize for comparison.

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("resize-inspect supports Windows only");
}

#[cfg(target_os = "windows")]
fn main() -> windows::core::Result<()> {
    use std::{ffi::c_void, time::Duration};
    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM, RECT},
            UI::WindowsAndMessaging::{
                EnumChildWindows, EnumWindows, GWLP_USERDATA, GetClassNameW, GetClientRect,
                GetWindowLongPtrW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
                SW_MAXIMIZE, SW_RESTORE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SetWindowPos,
                ShowWindow, WINDOW_LONG_PTR_INDEX,
            },
        },
        core::BOOL,
    };

    fn class(hwnd: HWND) -> String {
        let mut name = [0u16; 256];
        // SAFETY: The output buffer is valid; Windows validates the HWND.
        let length = unsafe { GetClassNameW(hwnd, &mut name) };
        String::from_utf16_lossy(&name[..length.max(0) as usize])
    }

    unsafe extern "system" fn inspect(hwnd: HWND, _: LPARAM) -> BOOL {
        let name = class(hwnd);
        if name == "ApplicationFrameWindow" || name == "Windows.UI.Core.CoreWindow" {
            let mut pid = 0;
            let mut rect = RECT::default();
            let mut client = RECT::default();
            let mut title = [0u16; 256];
            // SAFETY: These are read-only HWND queries with valid output buffers.
            let tid = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            let length = unsafe { GetWindowTextW(hwnd, &mut title) };
            let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
            let _ = unsafe { GetClientRect(hwnd, &mut client) };
            let extra = unsafe { GetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(0)) };
            let user = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
            println!(
                "hwnd={:#x} pid={pid} tid={tid} class={name} title={:?} rect=({},{},{},{}) client={}x{} extra={extra:#x} user={user:#x}",
                hwnd.0 as usize,
                String::from_utf16_lossy(&title[..length.max(0) as usize]),
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
                client.right - client.left,
                client.bottom - client.top,
            );
        }
        BOOL(1)
    }

    unsafe extern "system" fn top(hwnd: HWND, parameter: LPARAM) -> BOOL {
        // SAFETY: Both callbacks only inspect windows and always continue.
        let _ = unsafe { inspect(hwnd, parameter) };
        let _ = unsafe { EnumChildWindows(Some(hwnd), Some(inspect), parameter) };
        BOOL(1)
    }

    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if !arguments.is_empty() {
        assert!(
            arguments.len() == 2 || arguments.len() == 3,
            "usage: resize-inspect [HEX_FRAME_HWND maximize|restore|WIDTH HEIGHT]"
        );
        let value = usize::from_str_radix(arguments[0].trim_start_matches("0x"), 16)
            .expect("hexadecimal HWND");
        let hwnd = HWND(value as *mut c_void);
        assert_eq!(
            class(hwnd),
            "ApplicationFrameWindow",
            "expected a live app frame"
        );
        match arguments[1].as_str() {
            "maximize" | "restore" => {
                assert_eq!(arguments.len(), 2);
                let command = if arguments[1] == "maximize" {
                    SW_MAXIMIZE
                } else {
                    SW_RESTORE
                };
                // SAFETY: The explicitly selected HWND is a validated app frame.
                let _ = unsafe { ShowWindow(hwnd, command) };
            }
            width => {
                assert_eq!(arguments.len(), 3);
                let width: i32 = width.parse().expect("width");
                let height: i32 = arguments[2].parse().expect("height");
                assert!(width > 0 && height > 0);
                // SAFETY: Resize only the explicitly selected, validated frame.
                unsafe {
                    SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        width,
                        height,
                        SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOZORDER,
                    )?
                };
            }
        }
        std::thread::sleep(Duration::from_millis(700));
    }
    // SAFETY: The callbacks use no borrowed context and only read window state.
    unsafe { EnumWindows(Some(top), LPARAM(0)) }
}
