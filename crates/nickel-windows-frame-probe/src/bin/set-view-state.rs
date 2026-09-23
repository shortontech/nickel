#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("set-view-state supports Windows only");
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
    const VIEW_STATE_CONTROL: GUID = GUID::from_u128(0xde6e8a03_3811_4239_9de9_96d0dfc301e6);
    const VIEW_COLLECTION: GUID = GUID::from_u128(0x1841c6d7_4f9d_42c0_af41_8747538f10e5);

    type QueryService = unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT;
    type SetViewStateForDesiredAppState =
        unsafe extern "system" fn(*mut c_void, *const u16, u32) -> HRESULT;

    let mut args = std::env::args().skip(1);
    let app_id = args.next();
    let desired_state = args
        .next()
        .map(|value| value.parse::<u32>().expect("state integer"));
    assert!(
        app_id.is_some() == desired_state.is_some(),
        "usage: set-view-state [AUMID STATE]"
    );

    // SAFETY: The COM apartment is balanced below after all interfaces drop.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;
    let result = (|| {
        // SAFETY: Windows owns this registered local COM server.
        let shell: IServiceProvider =
            unsafe { CoCreateInstance(&IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER) }?;
        println!("phase=ImmersiveShell result=created");
        // SAFETY: Slot 3 of IServiceProvider is QueryService, and COM owns
        // each returned reference until IUnknown drops it.
        let query: QueryService = unsafe {
            let vtable = shell.as_raw().cast::<*const usize>().read();
            std::mem::transmute(vtable.add(3).read())
        };
        for sid in [VIEW_STATE_CONTROL, VIEW_COLLECTION] {
            let mut raw = std::ptr::null_mut();
            let status = unsafe { query(shell.as_raw(), &sid, &VIEW_STATE_CONTROL, &mut raw) };
            println!("phase=QueryService sid={sid:?} hresult={status:?}");
            if status.is_err() || raw.is_null() {
                continue;
            }
            let control = unsafe { IUnknown::from_raw(raw) };
            if let (Some(app_id), Some(desired_state)) = (&app_id, desired_state) {
                let app_id: Vec<u16> = app_id.encode_utf16().chain(Some(0)).collect();
                // SAFETY: This build's public PDB and IApplicationViewStateControl
                // proxy vtable identify slot 3 as SetViewStateForDesiredAppState.
                let set_state: SetViewStateForDesiredAppState = unsafe {
                    let vtable = control.as_raw().cast::<*const usize>().read();
                    std::mem::transmute(vtable.add(3).read())
                };
                let status = unsafe { set_state(control.as_raw(), app_id.as_ptr(), desired_state) };
                println!(
                    "phase=SetViewStateForDesiredAppState state={desired_state} hresult={status:?}"
                );
            }
            break;
        }
        Ok(())
    })();
    // SAFETY: Balances successful CoInitializeEx after COM interfaces drop.
    unsafe { CoUninitialize() };
    result
}
