#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("shell-key-probe supports Windows only");
}

#[cfg(target_os = "windows")]
fn main() {
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
    type AcquireIamKey = unsafe extern "system" fn(*mut u64) -> i32;
    type EnableIamAccess = unsafe extern "system" fn(*const u64, u32) -> i32;
    type SetWindowBand = unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> i32;
    type GetWindowBand = unsafe extern "system" fn(*mut c_void, *mut u32) -> i32;
    let test_set_band = std::env::args().any(|argument| argument == "--set-band");

    // Never replace an existing shell window. The parent runner also checks
    // that Explorer has stopped before starting this short-lived process.
    let current = unsafe { GetShellWindow() };
    if !current.is_invalid() {
        eprintln!("phase=preflight result=shell-present");
        std::process::exit(2);
    }
    println!("phase=preflight result=no-shell-window");

    let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let system32 = PathBuf::from(system_root).join("System32");
    let path = system32.join("user32.dll");
    // SAFETY: This is the absolute path to the installed system user32 DLL.
    let library = match unsafe { libloading::Library::new(path) } {
        Ok(library) => library,
        Err(error) => {
            eprintln!("phase=load-user32 error={error}");
            std::process::exit(1);
        }
    };
    // SAFETY: These are Windows' native function signatures. The DLL remains
    // loaded while both function pointers are used.
    let set_shell = match unsafe { library.get::<SetShellWindowEx>(b"SetShellWindowEx\0") } {
        Ok(symbol) => symbol,
        Err(error) => {
            eprintln!("phase=resolve-SetShellWindowEx error={error}");
            std::process::exit(1);
        }
    };
    // user32 has an internal symbol with this name, but the callable syscall
    // export is supplied by win32u on this Windows build.
    let syscall_library = match unsafe { libloading::Library::new(system32.join("win32u.dll")) } {
        Ok(library) => library,
        Err(error) => {
            eprintln!("phase=load-win32u error={error}");
            std::process::exit(1);
        }
    };
    let acquire_key =
        match unsafe { syscall_library.get::<AcquireIamKey>(b"NtUserAcquireIAMKey\0") } {
            Ok(symbol) => symbol,
            Err(error) => {
                eprintln!("phase=resolve-NtUserAcquireIAMKey error={error}");
                std::process::exit(1);
            }
        };

    // SAFETY: STATIC is a system-defined window class. The window is hidden,
    // top-level, and owned by this process until it is destroyed below.
    let window = match unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("Nickel IAM diagnostic"),
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
                "phase=create-window hresult={:?} error={error}",
                error.code()
            );
            std::process::exit(1);
        }
    };
    println!("phase=create-window result=success");
    // SAFETY: This process owns a valid top-level window. The second argument
    // is null because the diagnostic has no separate desktop list view.
    let set = unsafe { set_shell(window.0, std::ptr::null_mut()) };
    let set_error = unsafe { GetLastError() };
    println!(
        "phase=SetShellWindowEx success={} last_error={}",
        set != 0,
        set_error.0
    );
    if set != 0 {
        let registered = unsafe { GetShellWindow() };
        println!(
            "phase=GetShellWindow matches_own_window={}",
            registered == window
        );
        let mut key = 0_u64;
        // SAFETY: The pointer references a writable local u64. The key itself
        // is deliberately not printed or persisted.
        let acquired = unsafe { acquire_key(&mut key) };
        let key_error = unsafe { GetLastError() };
        println!(
            "phase=NtUserAcquireIAMKey success={} last_error={} key_nonzero={}",
            acquired != 0,
            key_error.0,
            key != 0
        );
        if acquired != 0 && test_set_band {
            let enable =
                unsafe { syscall_library.get::<EnableIamAccess>(b"NtUserEnableIAMAccess\0") }
                    .expect("resolve NtUserEnableIAMAccess");
            let set_band = unsafe { library.get::<SetWindowBand>(b"SetWindowBand\0") }
                .expect("resolve SetWindowBand");
            let get_band = unsafe { library.get::<GetWindowBand>(b"GetWindowBand\0") }
                .expect("resolve GetWindowBand");
            // SAFETY: This second hidden top-level window belongs to the
            // probe, while the original window remains registered as shell.
            let test_window = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    w!("Nickel IAM band diagnostic"),
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
            }
            .expect("create hidden band test window");
            let mut before = 0_u32;
            let got_before = unsafe { get_band(test_window.0, &mut before) };
            println!(
                "phase=GetWindowBand-before success={} band={before}",
                got_before != 0
            );
            // SAFETY: The key was acquired by this process and is passed by
            // pointer only to the native syscall. It is never logged.
            let enabled = unsafe { enable(&key, 1) };
            if enabled == 0 {
                let error = unsafe { GetLastError() };
                println!(
                    "phase=NtUserEnableIAMAccess success=false last_error={}",
                    error.0
                );
            } else {
                println!("phase=NtUserEnableIAMAccess success=true");
                // SAFETY: The window belongs to this process. A null insert
                // target requests the top of the chosen band.
                let changed = unsafe { set_band(test_window.0, std::ptr::null_mut(), 12) };
                if changed == 0 {
                    let error = unsafe { GetLastError() };
                    println!(
                        "phase=SetWindowBand-12 success=false last_error={}",
                        error.0
                    );
                } else {
                    println!("phase=SetWindowBand-12 success=true");
                }
                let mut after = 0_u32;
                let got_after = unsafe { get_band(test_window.0, &mut after) };
                println!(
                    "phase=GetWindowBand-after success={} band={after}",
                    got_after != 0
                );
                // SAFETY: Disable access on the same thread that enabled it.
                let disabled = unsafe { enable(&key, 0) };
                println!(
                    "phase=NtUserEnableIAMAccess-disable success={}",
                    disabled != 0
                );
            }
            // SAFETY: This process created and owns the hidden test window.
            let _ = unsafe { DestroyWindow(test_window) };
        }
    }
    // SAFETY: This process created the window and is ending its diagnostic.
    let _ = unsafe { DestroyWindow(window) };
    std::process::exit(i32::from(set == 0));
}
