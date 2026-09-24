#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("uncloak-window supports Windows only");
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

    const IMMERSIVE_SHELL: GUID = GUID::from_u128(0xc2f03a33_21f5_47fa_b4bb_156362a2f239);
    const IMMERSIVE_APPLICATION_MANAGER: GUID =
        GUID::from_u128(0x50fdbb99_5c92_495e_9e81_e2c2f48cddae);
    const IMMERSIVE_APPLICATION_MANAGER_INTERFACE: GUID =
        GUID::from_u128(0xbf63999f_7411_40da_861c_df72c0ffee84);
    // Read from OneCoreUAPCommonProxyStub!IID_IUncloakWindowService on this build.
    const UNCLOAK_WINDOW_SERVICE: GUID = GUID::from_u128(0xb706dded_208c_4795_b610_e7c002c31edc);

    type QueryService = unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT;
    type UncloakWindow = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT;

    let hwnd = std::env::args().nth(1).map(|value| {
        usize::from_str_radix(value.trim_start_matches("0x"), 16).expect("hexadecimal HWND")
    });
    // SAFETY: The COM apartment is balanced below after all interfaces drop.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;
    let result = (|| {
        // SAFETY: Windows owns this registered local COM server.
        let shell: IServiceProvider =
            unsafe { CoCreateInstance(&IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER) }?;
        println!("phase=ImmersiveShell result=created");
        // SAFETY: Slot 3 of IServiceProvider is QueryService. The output is
        // owned by COM and released through IUnknown below.
        let query: QueryService = unsafe {
            let vtable = shell.as_raw().cast::<*const usize>().read();
            std::mem::transmute(vtable.add(3).read())
        };
        for sid in [
            UNCLOAK_WINDOW_SERVICE,
            IMMERSIVE_APPLICATION_MANAGER_INTERFACE,
            IMMERSIVE_APPLICATION_MANAGER,
        ] {
            let mut raw = std::ptr::null_mut();
            let status = unsafe { query(shell.as_raw(), &sid, &UNCLOAK_WINDOW_SERVICE, &mut raw) };
            println!("phase=QueryService sid={sid:?} hresult={status:?}");
            if status.is_err() || raw.is_null() {
                continue;
            }
            let service = unsafe { IUnknown::from_raw(raw) };
            if let Some(hwnd) = hwnd {
                // SAFETY: The IID and method signature come from this build's
                // proxy stub and CApplicationManager public symbols. COM
                // returned an interface with the requested IID above.
                let uncloak: UncloakWindow = unsafe {
                    let vtable = service.as_raw().cast::<*const usize>().read();
                    std::mem::transmute(vtable.add(3).read())
                };
                let status = unsafe { uncloak(service.as_raw(), hwnd as *mut c_void) };
                println!("phase=UncloakWindow hwnd={hwnd:#x} hresult={status:?}");
            }
            break;
        }
        Ok(())
    })();
    // SAFETY: Balances successful CoInitializeEx after COM interfaces drop.
    unsafe { CoUninitialize() };
    result
}
