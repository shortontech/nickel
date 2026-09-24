#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("shell-hook-service-probe supports Windows only");
}

#[cfg(target_os = "windows")]
fn main() -> windows::core::Result<()> {
    use std::ffi::c_void;
    use windows::{
        Win32::System::Com::{
            CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, IServiceProvider,
        },
        core::{GUID, HRESULT, IUnknown, Interface},
    };

    if std::env::args().nth(1).as_deref() == Some("--get-taskman") {
        type GetTaskmanWindow = unsafe extern "system" fn() -> windows::Win32::Foundation::HWND;
        // SAFETY: user32 is a Windows system DLL and remains loaded for the call.
        let library = unsafe { libloading::Library::new("user32.dll") }
            .map_err(|_| windows::core::Error::from_hresult(HRESULT(0x80004005_u32 as i32)))?;
        // SAFETY: This private export takes no arguments and returns an HWND.
        let get_taskman = unsafe { library.get::<GetTaskmanWindow>(b"GetTaskmanWindow\0") }
            .map_err(|_| windows::core::Error::from_hresult(HRESULT(0x80004005_u32 as i32)))?;
        let taskman = unsafe { get_taskman() };
        println!("phase=get-taskman-window hwnd={:?}", taskman.0);
        return Ok(());
    }

    const IMMERSIVE_SHELL: GUID = GUID::from_u128(0xc2f03a33_21f5_47fa_b4bb_156362a2f239);
    const SHELL_HOOK_SERVICE: GUID = GUID::from_u128(0x4624bd39_5fc3_44a8_a809_163a836e9031);
    const SHELL_HOOK_INTERFACE: GUID = GUID::from_u128(0x914d9b3a_5e53_4e14_bbba_46062acb35a4);
    type QueryService = unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT;

    // SAFETY: This thread balances COM initialization after releasing its interfaces.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;
    let result = (|| {
        // SAFETY: Windows owns this registered COM server and returns an owned reference.
        let shell: IServiceProvider =
            unsafe { CoCreateInstance(&IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER) }?;
        let mut raw = std::ptr::null_mut();
        // SAFETY: IServiceProvider slot 3 is QueryService, and raw is writable.
        let query: QueryService = unsafe {
            let vtable = shell.as_raw().cast::<*const usize>().read();
            std::mem::transmute(vtable.add(3).read())
        };
        let status = unsafe {
            query(
                shell.as_raw(),
                &SHELL_HOOK_SERVICE,
                &SHELL_HOOK_INTERFACE,
                &mut raw,
            )
        };
        println!("phase=query-shell-hook-service hresult={status:?} interface={raw:p}");
        status.ok()?;
        if raw.is_null() {
            return Err(windows::core::Error::from_hresult(HRESULT(
                0x80004003_u32 as i32,
            )));
        }
        // SAFETY: A successful QueryService transferred one COM reference.
        let service = unsafe { IUnknown::from_raw(raw) };
        // SAFETY: This build's interface has IUnknown plus Register,
        // Unregister, and PostShellHookMessage, for six slots in total.
        let methods = unsafe {
            let vtable = service.as_raw().cast::<*const usize>().read();
            std::array::from_fn::<_, 6, _>(|index| vtable.add(index).read())
        };
        println!("phase=shell-hook-service-vtable methods={methods:#x?}");
        Ok(())
    })();
    // SAFETY: Balances the successful initialization above.
    unsafe { CoUninitialize() };
    result
}
