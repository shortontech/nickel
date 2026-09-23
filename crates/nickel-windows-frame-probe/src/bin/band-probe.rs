#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("band-probe supports Windows only");
}

#[cfg(target_os = "windows")]
fn main() {
    use std::{ffi::c_void, path::PathBuf};
    use windows::{Win32::Foundation::GetLastError, core::w};

    type CreateWindowInBand = unsafe extern "system" fn(
        u32,
        *const u16,
        *const u16,
        u32,
        i32,
        i32,
        i32,
        i32,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
    ) -> *mut c_void;
    type DestroyWindow = unsafe extern "system" fn(*mut c_void) -> i32;

    let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let path = PathBuf::from(system_root)
        .join("System32")
        .join("user32.dll");
    // SAFETY: Load the installed Windows user32 DLL by absolute path.
    let library = unsafe { libloading::Library::new(path) }.expect("load user32.dll");
    // SAFETY: Signatures match the native user32 exports. The DLL remains
    // loaded through both calls below.
    let create = unsafe { library.get::<CreateWindowInBand>(b"CreateWindowInBand\0") }
        .expect("resolve CreateWindowInBand");
    let destroy =
        unsafe { library.get::<DestroyWindow>(b"DestroyWindow\0") }.expect("resolve DestroyWindow");

    for band in [1, 12] {
        // SAFETY: STATIC is a system window class. The test window is hidden,
        // top-level, 1x1, and immediately destroyed on success.
        let window = unsafe {
            create(
                0,
                w!("STATIC").0,
                w!("Nickel band diagnostic").0,
                0x8000_0000,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                band,
            )
        };
        if window.is_null() {
            let error = unsafe { GetLastError() };
            println!(
                "band={band} result=denied-or-failed win32_error={}",
                error.0
            );
        } else {
            println!("band={band} result=created");
            // SAFETY: user32 returned a window owned by this process.
            let _ = unsafe { destroy(window) };
        }
    }
}
