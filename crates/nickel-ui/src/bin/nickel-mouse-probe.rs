#[cfg(target_os = "windows")]
fn main() {
    windows_probe::run();
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("nickel-mouse-probe currently supports Windows only");
}

#[cfg(target_os = "windows")]
mod windows_probe {
    use std::{
        env,
        fs::{File, OpenOptions},
        io::Write,
        path::PathBuf,
        sync::{Mutex, OnceLock},
    };

    use windows::Win32::{
        Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CallNextHookEx, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, MSG,
            MSLLHOOKSTRUCT, SetWindowsHookExW, UnhookWindowsHookEx, WH_MOUSE_LL, WM_MOUSEMOVE,
        },
    };

    static LOG: OnceLock<Mutex<File>> = OnceLock::new();

    pub fn run() {
        let path = log_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .expect("failed to create mouse probe log");
        LOG.set(Mutex::new(file))
            .unwrap_or_else(|_| panic!("mouse probe log was already initialized"));
        log_line(&format!(
            "nickel-mouse-probe started path={} injected_flag=0x1 lower_integrity_flag=0x2",
            path.display()
        ));

        let module = unsafe { GetModuleHandleW(None) }.expect("failed to find probe module");
        let hook = unsafe {
            SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(HINSTANCE(module.0)), 0)
        }
        .expect("failed to install mouse probe hook");

        let mut message = MSG::default();
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {}
        let _ = unsafe { UnhookWindowsHookEx(hook) };
    }

    unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 && wparam.0 as u32 == WM_MOUSEMOVE {
            let event = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            let flags = event.flags;
            let foreground = unsafe { GetForegroundWindow() };
            let mut foreground_pid = 0;
            unsafe {
                GetWindowThreadProcessId(foreground, Some(&mut foreground_pid));
            }
            log_line(&format!(
                "time={} x={} y={} flags=0x{flags:x} injected={} lower_integrity={} extra=0x{:x} foreground_pid={foreground_pid}",
                event.time,
                event.pt.x,
                event.pt.y,
                flags & 0x1 != 0,
                flags & 0x2 != 0,
                event.dwExtraInfo,
            ));
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    fn log_line(line: &str) {
        println!("{line}");
        if let Some(log) = LOG.get()
            && let Ok(mut file) = log.lock()
        {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }

    fn log_path() -> PathBuf {
        env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .expect("LOCALAPPDATA is not set")
            .join("Nickel")
            .join("logs")
            .join("nickel-mouse-probe.log")
    }
}
