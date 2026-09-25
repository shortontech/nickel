//! Shell window registration for the host process.

#[cfg(target_os = "windows")]
pub(crate) struct ShellWindowGuard {
    pub(crate) window: windows::Win32::Foundation::HWND,
    _library: libloading::Library,
}

#[cfg(target_os = "windows")]
impl ShellWindowGuard {
    pub(crate) fn register() -> Option<Self> {
        use std::{ffi::c_void, path::PathBuf};
        use windows::{
            Win32::{
                Foundation::GetLastError,
                UI::WindowsAndMessaging::{
                    CreateWindowExW, DestroyWindow, GetShellWindow, WINDOW_EX_STYLE, WS_POPUP,
                },
            },
            core::w,
        };

        type SetShellWindowEx = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;

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
        Some(Self {
            window,
            _library: library,
        })
    }
}

#[cfg(target_os = "windows")]
impl Drop for ShellWindowGuard {
    fn drop(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;

        // SAFETY: The current thread created this window and owns its lifetime.
        let _ = unsafe { DestroyWindow(self.window) };
    }
}
