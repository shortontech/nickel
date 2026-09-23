#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("switch-view supports Windows only");
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
    const VIEW_COLLECTION: GUID = GUID::from_u128(0x1841c6d7_4f9d_42c0_af41_8747538f10e5);
    const VIEW_SWITCHER: GUID = GUID::from_u128(0xe0fe7384_5482_4659_ab2c_4ff038948779);
    const VIEW_POSITIONER: GUID = GUID::from_u128(0xff62716a_0bd0_4105_a6ec_83b09d864267);

    type QueryService = unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT;
    type GetViewForAppUserModelId =
        unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut c_void) -> HRESULT;
    type SwitchTo = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT;
    type DirectSwitchTo = unsafe extern "system" fn(*mut c_void) -> HRESULT;
    type SetCloak = unsafe extern "system" fn(*mut c_void, u32, i32) -> HRESULT;

    enum Mode {
        Inspect,
        ManagerSwitch,
        DirectSwitch,
        Uncloak,
    }

    let mut args = std::env::args().skip(1);
    let app_id = args.next().expect("usage: switch-view AUMID [--switch]");
    let mode = match args.next().as_deref() {
        None => Mode::Inspect,
        Some("--switch") => Mode::ManagerSwitch,
        Some("--direct-switch") => Mode::DirectSwitch,
        Some("--uncloak") => Mode::Uncloak,
        _ => panic!("usage: switch-view AUMID [--switch|--direct-switch|--uncloak]"),
    };
    assert!(
        args.next().is_none(),
        "usage: switch-view AUMID [--switch|--direct-switch|--uncloak]"
    );

    // SAFETY: The COM apartment is balanced after all acquired interfaces drop.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;
    let result = (|| {
        // SAFETY: Windows owns this registered local COM server.
        let shell: IServiceProvider =
            unsafe { CoCreateInstance(&IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER) }?;
        println!("phase=ImmersiveShell result=created");
        // SAFETY: Slot 3 of IServiceProvider is QueryService. Returned COM
        // references are released by the IUnknown wrappers below.
        let query: QueryService = unsafe {
            let vtable = shell.as_raw().cast::<*const usize>().read();
            std::mem::transmute(vtable.add(3).read())
        };
        let mut collection_raw = std::ptr::null_mut();
        let status = unsafe {
            query(
                shell.as_raw(),
                &VIEW_COLLECTION,
                &VIEW_COLLECTION,
                &mut collection_raw,
            )
        };
        println!("phase=QueryCollection hresult={status:?}");
        status.ok()?;
        let collection = unsafe { IUnknown::from_raw(collection_raw) };
        let wide_app_id: Vec<u16> = app_id.encode_utf16().chain(Some(0)).collect();
        let mut view_raw = std::ptr::null_mut();
        // SAFETY: This build's CApplicationViewManager vtable identifies slot
        // 8 of IApplicationViewCollection as GetViewForAppUserModelId.
        let get_view: GetViewForAppUserModelId = unsafe {
            let vtable = collection.as_raw().cast::<*const usize>().read();
            std::mem::transmute(vtable.add(8).read())
        };
        let status = unsafe { get_view(collection.as_raw(), wide_app_id.as_ptr(), &mut view_raw) };
        println!(
            "phase=GetViewForAppUserModelId hresult={status:?} found={}",
            !view_raw.is_null()
        );
        status.ok()?;
        if view_raw.is_null() {
            return Ok(());
        }
        let view = unsafe { IUnknown::from_raw(view_raw) };
        match view.cast::<IServiceProvider>() {
            Ok(provider) => {
                let mut positioner_raw = std::ptr::null_mut();
                let status = unsafe {
                    query(
                        provider.as_raw(),
                        &VIEW_POSITIONER,
                        &VIEW_POSITIONER,
                        &mut positioner_raw,
                    )
                };
                println!("phase=QueryViewPositioner hresult={status:?}");
                if !positioner_raw.is_null() {
                    drop(unsafe { IUnknown::from_raw(positioner_raw) });
                }
            }
            Err(error) => println!("phase=QueryViewServiceProvider hresult={:?}", error.code()),
        }
        if matches!(mode, Mode::DirectSwitch) {
            // SAFETY: The matching public PDB identifies slot 7 of this
            // IApplicationView interface as SwitchTo(void). This fixture
            // invokes the method on the COM reference returned above.
            let direct_switch_to: DirectSwitchTo = unsafe {
                let vtable = view.as_raw().cast::<*const usize>().read();
                std::mem::transmute(vtable.add(7).read())
            };
            let status = unsafe { direct_switch_to(view.as_raw()) };
            println!("phase=ViewSwitchTo hresult={status:?}");
            status.ok()?;
        } else if matches!(mode, Mode::Uncloak) {
            // SAFETY: The matching public PDB identifies IApplicationView
            // slot 12 as SetCloak(APPLICATION_VIEW_CLOAK_TYPE, int).
            // Type 1 is AVCT_DEFAULT; zero clears that cloak.
            let set_cloak: SetCloak = unsafe {
                let vtable = view.as_raw().cast::<*const usize>().read();
                std::mem::transmute(vtable.add(12).read())
            };
            let status = unsafe { set_cloak(view.as_raw(), 1, 0) };
            println!("phase=ViewSetCloak type=1 enabled=0 hresult={status:?}");
            status.ok()?;
        } else if matches!(mode, Mode::ManagerSwitch) {
            let mut switcher_raw = std::ptr::null_mut();
            let status = unsafe {
                query(
                    shell.as_raw(),
                    &VIEW_SWITCHER,
                    &VIEW_SWITCHER,
                    &mut switcher_raw,
                )
            };
            println!("phase=QuerySwitcher hresult={status:?}");
            status.ok()?;
            let switcher = unsafe { IUnknown::from_raw(switcher_raw) };
            // SAFETY: This build's IApplicationViewSwitcher vtable identifies
            // slot 3 as SwitchTo(IApplicationView*).
            let switch_to: SwitchTo = unsafe {
                let vtable = switcher.as_raw().cast::<*const usize>().read();
                std::mem::transmute(vtable.add(3).read())
            };
            let status = unsafe { switch_to(switcher.as_raw(), view.as_raw()) };
            println!("phase=SwitchTo hresult={status:?}");
            status.ok()?;
        }
        Ok(())
    })();
    // SAFETY: Balances successful CoInitializeEx after COM references drop.
    unsafe { CoUninitialize() };
    result
}
