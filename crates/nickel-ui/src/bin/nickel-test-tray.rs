#[cfg(target_os = "linux")]
use std::{thread, time::Duration};

#[cfg(target_os = "linux")]
struct TestTray;

#[cfg(target_os = "linux")]
#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl TestTray {
    fn activate(&self, _x: i32, _y: i32) {
        println!("nickel-test-tray: activated");
    }

    fn context_menu(&self, _x: i32, _y: i32) {}

    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        "nickel-test-tray"
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }

    #[zbus(property)]
    fn title(&self) -> &str {
        "Nickel Test Tray"
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        ""
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        let bytes = (0..32 * 32)
            .flat_map(|index| {
                let x = index % 32;
                let y = index / 32;
                let (red, green, blue) = if (x / 8 + y / 8) % 2 == 0 {
                    (55, 200, 255)
                } else {
                    (245, 190, 55)
                };
                [255, red, green, blue]
            })
            .collect();
        vec![(32, 32, bytes)]
    }
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let connection = zbus::blocking::Connection::session()?;
    connection.request_name("org.nickel.TestTray")?;
    connection
        .object_server()
        .at("/StatusNotifierItem", TestTray)?;
    let watcher = zbus::blocking::Proxy::new(
        &connection,
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
    )?;
    watcher.call_method("RegisterStatusNotifierItem", &("org.nickel.TestTray"))?;
    println!("nickel-test-tray: registered; press Ctrl-C to stop");
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

#[cfg(target_os = "windows")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::mem::size_of;

    use windows::{
        Win32::{
            Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                Shell::{
                    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
                    Shell_NotifyIconW,
                },
                WindowsAndMessaging::{
                    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
                    DispatchMessageW, GetCursorPos, GetMessageW, IDI_APPLICATION, LoadIconW,
                    MF_SEPARATOR, MF_STRING, MSG, PostQuitMessage, RegisterClassW,
                    SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
                    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_LBUTTONUP, WM_RBUTTONUP,
                    WM_USER, WNDCLASSW,
                },
            },
        },
        core::w,
    };

    const CALLBACK_MESSAGE: u32 = WM_USER + 1;

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == CALLBACK_MESSAGE {
            let mouse_message = lparam.0 as u32 & 0xffff;
            if mouse_message == WM_RBUTTONUP {
                // SAFETY: The menu is owned for this synchronous call and destroyed afterward.
                unsafe {
                    let Ok(menu) = CreatePopupMenu() else {
                        return LRESULT(0);
                    };
                    let _ = AppendMenuW(menu, MF_STRING, 1, w!("Test action"));
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
                    let _ = AppendMenuW(menu, MF_STRING, 2, w!("Exit"));
                    let mut cursor = POINT::default();
                    let _ = GetCursorPos(&mut cursor);
                    let _ = SetForegroundWindow(hwnd);
                    let selected = TrackPopupMenu(
                        menu,
                        TPM_RETURNCMD | TPM_RIGHTBUTTON,
                        cursor.x,
                        cursor.y,
                        None,
                        hwnd,
                        None,
                    )
                    .0;
                    let _ = DestroyMenu(menu);
                    match selected {
                        1 => println!("nickel-test-tray: test action selected"),
                        2 => PostQuitMessage(0),
                        _ => {}
                    }
                }
                return LRESULT(0);
            }
            if mouse_message == WM_LBUTTONUP {
                println!("nickel-test-tray: activated");
                return LRESULT(0);
            }
        }
        // SAFETY: Unhandled messages are forwarded unchanged to the system default procedure.
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    // SAFETY: The registered class, hidden owner window, and notification icon all live until the
    // message loop exits. Windows copies the class and notification structures synchronously.
    unsafe {
        let module = GetModuleHandleW(None)?;
        let instance = HINSTANCE(module.0);
        let class = WNDCLASSW {
            hInstance: instance,
            lpszClassName: w!("NickelTestTrayWindow"),
            lpfnWndProc: Some(window_proc),
            ..Default::default()
        };
        if RegisterClassW(&raw const class) == 0 {
            return Err(std::io::Error::other(format!(
                "failed to register the test window class: {}",
                std::io::Error::last_os_error()
            ))
            .into());
        }
        let window = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class.lpszClassName,
            w!("Nickel Test Tray"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )?;
        let mut icon = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: window,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: CALLBACK_MESSAGE,
            hIcon: LoadIconW(None, IDI_APPLICATION)?,
            ..Default::default()
        };
        let tooltip: Vec<u16> = "Nickel Test Tray\0".encode_utf16().collect();
        icon.szTip[..tooltip.len()].copy_from_slice(&tooltip);
        if !Shell_NotifyIconW(NIM_ADD, &raw const icon).as_bool() {
            return Err(std::io::Error::other(
                "Windows could not register the test notification icon",
            )
            .into());
        }
        println!("nickel-test-tray: registered; press Ctrl-C to stop");
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        let _ = Shell_NotifyIconW(NIM_DELETE, &raw const icon);
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn main() {}
