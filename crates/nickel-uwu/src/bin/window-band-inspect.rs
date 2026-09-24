#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("window-band-inspect supports Windows only");
}

#[cfg(target_os = "windows")]
fn main() {
    use std::{ffi::c_void, path::PathBuf, sync::atomic::Ordering};
    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM},
            UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId},
        },
        core::BOOL,
    };

    type GetWindowBand = unsafe extern "system" fn(*mut c_void, *mut u32) -> i32;
    static TARGET_PID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    static GET_BAND: std::sync::OnceLock<GetWindowBand> = std::sync::OnceLock::new();

    unsafe extern "system" fn visit(window: HWND, _: LPARAM) -> BOOL {
        let mut pid = 0;
        // SAFETY: The process ID pointer is writable for this call.
        unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
        if pid == TARGET_PID.load(Ordering::Relaxed) {
            let mut band = 0;
            // SAFETY: GetWindowBand is resolved from the loaded system DLL,
            // which remains loaded for the lifetime of this enumeration.
            let ok = unsafe { GET_BAND.get().expect("GetWindowBand pointer")(window.0, &mut band) };
            println!(
                "pid={pid} hwnd={:?} get_band_success={} band={band}",
                window.0,
                ok != 0
            );
        }
        BOOL(1)
    }

    let pid: u32 = std::env::args()
        .nth(1)
        .expect("usage: window-band-inspect PID")
        .parse()
        .expect("numeric PID");
    let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let path = PathBuf::from(system_root)
        .join("System32")
        .join("user32.dll");
    // SAFETY: This is the absolute path to the installed system user32 DLL.
    let library = unsafe { libloading::Library::new(path) }.expect("load user32.dll");
    // SAFETY: The signature matches the private Windows export and the DLL
    // remains loaded until the enumeration is complete.
    let get_band =
        unsafe { library.get::<GetWindowBand>(b"GetWindowBand\0") }.expect("resolve GetWindowBand");
    GET_BAND.set(*get_band).expect("set GetWindowBand pointer");
    TARGET_PID.store(pid, Ordering::Relaxed);
    // SAFETY: The callback only reads window/process identity and band.
    if let Err(error) = unsafe { EnumWindows(Some(visit), LPARAM(0)) } {
        eprintln!("EnumWindows failed: {error}");
        std::process::exit(1);
    }
}
