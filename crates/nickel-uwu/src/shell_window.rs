//! Shell window registration and hook forwarding for the host process.

#[cfg(target_os = "windows")]
static SHELL_HOOK_MESSAGE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
#[cfg(target_os = "windows")]
static ORIGINAL_SHELL_WNDPROC: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(0);

#[cfg(target_os = "windows")]
unsafe extern "system" fn shell_window_proc(
    hwnd: windows::Win32::Foundation::HWND,
    message: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use std::sync::atomic::Ordering;
    use windows::Win32::{
        Foundation::LRESULT,
        UI::WindowsAndMessaging::{CallWindowProcW, DefWindowProcW, PostMessageW, WNDPROC},
    };

    let hook_message = SHELL_HOOK_MESSAGE.load(Ordering::Relaxed);
    if hook_message != 0 && message == hook_message {
        if matches!(wparam.0, 1 | 2) {
            // SAFETY: hwnd is the live window receiving this callback. Posting
            // avoids a COM call inside Windows' synchronous shell hook delivery.
            if let Err(error) = unsafe { PostMessageW(Some(hwnd), 0x8070, wparam, lparam) } {
                eprintln!("phase=queue-shell-hook error={error}");
            }
        }
        return LRESULT(0);
    }
    let original = ORIGINAL_SHELL_WNDPROC.load(Ordering::Relaxed);
    if original != 0 {
        // SAFETY: SetWindowLongPtrW returned this live system STATIC window
        // procedure when the same HWND was subclassed below.
        let previous: WNDPROC = unsafe { std::mem::transmute(original) };
        unsafe { CallWindowProcW(previous, hwnd, message, wparam, lparam) }
    } else {
        // SAFETY: The original procedure may be temporarily unavailable
        // while the window is being registered or destroyed.
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
}

#[cfg(target_os = "windows")]
pub(crate) struct ShellWindowGuard {
    pub(crate) window: windows::Win32::Foundation::HWND,
    shell_hook_registered: bool,
    taskman_registered: bool,
    original_wndproc: isize,
    set_taskman: unsafe extern "system" fn(*mut std::ffi::c_void) -> i32,
    _library: libloading::Library,
}

#[cfg(target_os = "windows")]
impl ShellWindowGuard {
    pub(crate) fn register(register_taskman: bool) -> Option<Self> {
        use std::sync::atomic::Ordering;
        use std::{ffi::c_void, path::PathBuf};
        use windows::{
            Win32::{
                Foundation::GetLastError,
                UI::WindowsAndMessaging::{
                    CreateWindowExW, DeregisterShellHookWindow, DestroyWindow, GWLP_WNDPROC,
                    GetShellWindow, RegisterShellHookWindow, RegisterWindowMessageW,
                    SetWindowLongPtrW, WINDOW_EX_STYLE, WS_POPUP,
                },
            },
            core::w,
        };

        type SetShellWindowEx = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;
        type SetTaskmanWindow = unsafe extern "system" fn(*mut c_void) -> i32;
        type GetTaskmanWindow = unsafe extern "system" fn() -> *mut c_void;

        if !unsafe { GetShellWindow() }.is_invalid() {
            eprintln!("phase=shell-window result=already-registered");
            return None;
        }
        let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let path = PathBuf::from(system_root)
            .join("System32")
            .join("user32.dll");
        // SAFETY: This absolute path targets the installed system user32 DLL.
        let library = match unsafe { libloading::Library::new(path) } {
            Ok(library) => library,
            Err(error) => {
                eprintln!("phase=load-user32 error={error}");
                return None;
            }
        };
        // SAFETY: The DLL remains loaded in the returned guard while the
        // function pointer and registered window are in use.
        let set_shell = match unsafe { library.get::<SetShellWindowEx>(b"SetShellWindowEx\0") } {
            Ok(symbol) => symbol,
            Err(error) => {
                eprintln!("phase=resolve-SetShellWindowEx error={error}");
                return None;
            }
        };
        let set_taskman = match unsafe { library.get::<SetTaskmanWindow>(b"SetTaskmanWindow\0") } {
            Ok(symbol) => *symbol,
            Err(error) => {
                eprintln!("phase=resolve-SetTaskmanWindow error={error}");
                return None;
            }
        };
        let get_taskman = match unsafe { library.get::<GetTaskmanWindow>(b"GetTaskmanWindow\0") } {
            Ok(symbol) => *symbol,
            Err(error) => {
                eprintln!("phase=resolve-GetTaskmanWindow error={error}");
                return None;
            }
        };
        // SAFETY: STATIC is a system-defined class. The hidden top-level
        // window is owned by this thread for the lifetime of the guard.
        let window = match unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Nickel UWU host"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
        } {
            Ok(window) => window,
            Err(error) => {
                eprintln!(
                    "phase=create-shell-window hresult={:?} error={error}",
                    error.code()
                );
                return None;
            }
        };
        // SAFETY: This process owns the valid top-level window; the null
        // second argument means it has no separate desktop list-view window.
        let registered = unsafe { set_shell(window.0, std::ptr::null_mut()) } != 0;
        if !registered {
            let error = unsafe { GetLastError() };
            eprintln!(
                "phase=SetShellWindowEx result=failed last_error={}",
                error.0
            );
            let _ = unsafe { DestroyWindow(window) };
            return None;
        }
        let matches = unsafe { GetShellWindow() } == window;
        println!(
            "phase=SetShellWindowEx result=success shell_hwnd={:?} matches_own_window={matches}",
            window.0
        );
        if !matches {
            let _ = unsafe { DestroyWindow(window) };
            return None;
        }
        // Shell hook notifications are sent directly to the window procedure;
        // they are not returned by GetMessageW. Subclass this hidden window
        // before registering it so the message loop can forward each event.
        let hook_message = unsafe { RegisterWindowMessageW(w!("SHELLHOOK")) };
        if hook_message == 0 {
            eprintln!("phase=register-shell-hook-message result=failed");
            let _ = unsafe { DestroyWindow(window) };
            return None;
        }
        SHELL_HOOK_MESSAGE.store(hook_message, Ordering::Relaxed);
        let original_wndproc = unsafe {
            SetWindowLongPtrW(
                window,
                GWLP_WNDPROC,
                shell_window_proc as *const () as usize as isize,
            )
        };
        if original_wndproc == 0 {
            eprintln!("phase=subclass-shell-window result=failed");
            SHELL_HOOK_MESSAGE.store(0, Ordering::Relaxed);
            let _ = unsafe { DestroyWindow(window) };
            return None;
        }
        ORIGINAL_SHELL_WNDPROC.store(original_wndproc, Ordering::Relaxed);
        println!("phase=shell-hook-message id={hook_message:#x}");
        // SAFETY: This thread owns the live shell HWND. The guard unregisters
        // the same HWND before destroying it.
        let shell_hook_registered = unsafe { RegisterShellHookWindow(window) }.as_bool();
        if shell_hook_registered {
            println!("phase=RegisterShellHookWindow result=success");
        } else {
            println!(
                "phase=RegisterShellHookWindow result=failed last_error={}",
                unsafe { GetLastError() }.0
            );
            let _ = unsafe { SetWindowLongPtrW(window, GWLP_WNDPROC, original_wndproc) };
            ORIGINAL_SHELL_WNDPROC.store(0, Ordering::Relaxed);
            SHELL_HOOK_MESSAGE.store(0, Ordering::Relaxed);
            let _ = unsafe { DestroyWindow(window) };
            return None;
        }
        // The shell window alone does not cause the desktop's shell hook
        // notifications to reach this process. The task manager window is
        // registered by Explorer and by other replacement shells.
        let taskman_registered = if register_taskman {
            let registered = unsafe { set_taskman(window.0) } != 0;
            let matches = unsafe { get_taskman() } == window.0;
            println!("phase=SetTaskmanWindow registered={registered} matches_own_window={matches}");
            if !registered || !matches {
                let _ = unsafe { DeregisterShellHookWindow(window) };
                let _ = unsafe { SetWindowLongPtrW(window, GWLP_WNDPROC, original_wndproc) };
                ORIGINAL_SHELL_WNDPROC.store(0, Ordering::Relaxed);
                SHELL_HOOK_MESSAGE.store(0, Ordering::Relaxed);
                let _ = unsafe { DestroyWindow(window) };
                return None;
            }
            true
        } else {
            false
        };
        Some(Self {
            window,
            shell_hook_registered,
            taskman_registered,
            original_wndproc,
            set_taskman,
            _library: library,
        })
    }
}

#[cfg(target_os = "windows")]
impl Drop for ShellWindowGuard {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        use windows::Win32::UI::WindowsAndMessaging::{
            DeregisterShellHookWindow, DestroyWindow, GWLP_WNDPROC, SetWindowLongPtrW,
        };

        // SAFETY: The current thread created this window and owns its lifetime.
        if self.shell_hook_registered {
            let _ = unsafe { DeregisterShellHookWindow(self.window) };
        }
        if self.taskman_registered {
            let _ = unsafe { (self.set_taskman)(std::ptr::null_mut()) };
        }
        let _ = unsafe { SetWindowLongPtrW(self.window, GWLP_WNDPROC, self.original_wndproc) };
        ORIGINAL_SHELL_WNDPROC.store(0, Ordering::Relaxed);
        SHELL_HOOK_MESSAGE.store(0, Ordering::Relaxed);
        let _ = unsafe { DestroyWindow(self.window) };
    }
}
