//! Private immersive shell controller startup for the verified Windows build.

#[cfg(target_os = "windows")]
pub(crate) fn start_immersive_shell_controller(
    scenario1: bool,
    skip_window_events: bool,
    skip_touch_keyboard: bool,
) -> Result<ControllerGuard, String> {
    use std::ffi::c_void;
    use windows::{
        Win32::System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
            LibraryLoader::GetModuleHandleW,
        },
        core::{GUID, HRESULT, IUnknown, Interface, w},
    };

    const BUILDER: GUID = GUID::from_u128(0xc71c41f1_ddad_42dc_a8fc_f5bfc61df957);
    const BUILDER2: GUID = GUID::from_u128(0x2eb59b15_1487_40ce_916e_ef65330dd224);
    // These RVAs are PDB-verified for Windows build 26200. Vtable checks
    // prevent an unsupported build from calling unknown private methods.
    const CREATE_CONTROLLER_RVA: usize = 0x238e70;
    const SET_SCENARIO_RVA: usize = 0x3f7660;
    const START_RVA: usize = 0x13b20;
    type QueryInterface =
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
    type CreateController = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;
    type SetScenario = unsafe extern "system" fn(*mut c_void, i32) -> HRESULT;
    type Start = unsafe extern "system" fn(*mut c_void) -> HRESULT;

    // SAFETY: COM is initialized by the caller and owns the returned reference.
    let builder: IUnknown = unsafe { CoCreateInstance(&BUILDER, None, CLSCTX_INPROC_SERVER) }
        .map_err(|error| format!("create builder: {error}"))?;
    // SAFETY: The builder is a live COM object with a valid IUnknown vtable.
    let query_address = unsafe {
        let vtable = builder.as_raw().cast::<*const usize>().read();
        vtable.read()
    };
    // SAFETY: Slot 0 of IUnknown is QueryInterface.
    let query: QueryInterface = unsafe { std::mem::transmute(query_address) };
    let mut builder2_raw = std::ptr::null_mut();
    // SAFETY: QueryInterface writes an owned interface reference on success.
    unsafe { query(builder.as_raw(), &BUILDER2, &mut builder2_raw) }
        .ok()
        .map_err(|error| format!("query builder2: {error}"))?;
    // SAFETY: QueryInterface returned one owned IUnknown-compatible reference.
    let builder2 = unsafe { IUnknown::from_raw(builder2_raw) };
    println!(
        "phase=ImmersiveShellBuilder interfaces builder={:p} builder2={:p}",
        builder.as_raw(),
        builder2.as_raw()
    );
    // SAFETY: The builder COM object keeps this in-process DLL loaded.
    let builder_module = unsafe { GetModuleHandleW(w!("twinui.pcshell.dll")) }
        .map_err(|error| format!("builder module: {error}"))?;
    let expected_create = builder_module.0 as usize + CREATE_CONTROLLER_RVA;
    // SAFETY: Both live COM interfaces have at least the three IUnknown slots
    // and one method slot. The target address is checked before calling it.
    let candidates = [builder.as_raw(), builder2.as_raw()];
    let create_methods = candidates.map(|interface| unsafe {
        let vtable = interface.cast::<*const usize>().read();
        vtable.add(3).read()
    });
    println!(
        "phase=ImmersiveShellBuilder methods={create_methods:#x?} expected_create={expected_create:#x}"
    );
    if scenario1 {
        let scenario_address = create_methods[1];
        let expected_scenario = builder_module.0 as usize + SET_SCENARIO_RVA;
        if scenario_address != expected_scenario {
            return Err("scenario method does not match this Windows build".into());
        }
        // SAFETY: The pointer matches the PDB-verified SetShellScenario method.
        let set_scenario: SetScenario = unsafe { std::mem::transmute(scenario_address) };
        let result = unsafe { set_scenario(builder2.as_raw(), 1) };
        println!("phase=ImmersiveShellBuilder-SetScenario1 hresult={result:?}");
        result
            .ok()
            .map_err(|error| format!("set scenario: {error}"))?;
    }
    let (create_interface, create_address) = candidates
        .into_iter()
        .zip(create_methods)
        .find(|(_, method)| *method == expected_create)
        .ok_or("builder vtable does not match this Windows build")?;
    // SAFETY: The address matches the PDB-verified builder method with ABI
    // HRESULT CreateImmersiveShellController(IImmersiveShellController**).
    let create: CreateController = unsafe { std::mem::transmute(create_address) };
    let mut controller_raw = std::ptr::null_mut();
    let result = unsafe { create(create_interface, &mut controller_raw) };
    println!("phase=ImmersiveShellBuilder-CreateController hresult={result:?}");
    result
        .ok()
        .map_err(|error| format!("create controller through builder: {error}"))?;
    if controller_raw.is_null() {
        return Err("builder returned a null controller".into());
    }
    // SAFETY: The builder returned one owned IUnknown-compatible reference.
    let controller = unsafe { IUnknown::from_raw(controller_raw) };
    // SAFETY: The controller has a valid vtable; slot 3 is checked below.
    let start_address = unsafe {
        let vtable = controller.as_raw().cast::<*const usize>().read();
        vtable.add(3).read()
    };
    let behavior_vtable = if skip_window_events || skip_touch_keyboard {
        Some(install_component_filter(
            &controller,
            builder_module.0 as usize,
            skip_window_events,
            skip_touch_keyboard,
        )?)
    } else {
        None
    };
    // SAFETY: The module is loaded by CoCreateInstance and remains loaded while
    // the returned controller exists.
    let module = unsafe { GetModuleHandleW(w!("Windows.ImmersiveShell.ServiceProvider.dll")) }
        .map_err(|error| format!("controller module: {error}"))?;
    let expected = module.0 as usize + START_RVA;
    println!("phase=ImmersiveShellController start={start_address:#x} expected={expected:#x}");
    if start_address != expected {
        return Err("controller vtable does not match this Windows build".into());
    }
    // SAFETY: The address matches the PDB-verified CImmersiveShellController::Start
    // method, whose ABI is HRESULT Start(void). The object is still alive.
    let start: Start = unsafe { std::mem::transmute(start_address) };
    let result = unsafe { start(controller.as_raw()) };
    println!("phase=ImmersiveShellController-Start hresult={result:?}");
    result
        .ok()
        .map_err(|error| format!("start controller: {error}"))?;
    Ok(ControllerGuard {
        _controller: controller,
        _behavior_vtable: behavior_vtable,
    })
}

#[cfg(target_os = "windows")]
pub(crate) struct ControllerGuard {
    // Drop the controller before its behavior's copied vtable.
    _controller: windows::core::IUnknown,
    _behavior_vtable: Option<Box<[usize; 12]>>,
}

#[cfg(target_os = "windows")]
static ORIGINAL_SHOULD_CREATE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(target_os = "windows")]
static COMPONENT_LIMIT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(u32::MAX);
#[cfg(target_os = "windows")]
static SKIP_WINDOW_EVENTS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(target_os = "windows")]
static SKIP_TOUCH_KEYBOARD: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "windows")]
unsafe extern "system" fn should_create_component_diagnostic(
    this: *mut std::ffi::c_void,
    index: u32,
    should_create: *mut i32,
    class_id: *mut windows::core::GUID,
) -> windows::core::HRESULT {
    use std::sync::atomic::Ordering;
    type Method = unsafe extern "system" fn(
        *mut std::ffi::c_void,
        u32,
        *mut i32,
        *mut windows::core::GUID,
    ) -> windows::core::HRESULT;
    let address = ORIGINAL_SHOULD_CREATE.load(Ordering::Acquire);
    // SAFETY: The installation routine stores a PDB-verified method address
    // before publishing this replacement vtable to another thread.
    let original: Method = unsafe { std::mem::transmute(address) };
    let result = unsafe { original(this, index, should_create, class_id) };
    const WINDOW_MANAGEMENT_EVENTS: windows::core::GUID =
        windows::core::GUID::from_u128(0x0cd069cf_ac9b_41f4_9571_3a95a62c36a1);
    const TOUCH_KEYBOARD_EXPERIENCE_MANAGER: windows::core::GUID =
        windows::core::GUID::from_u128(0xf580a09b_ea6e_4d83_9a03_add6cb756ab3);
    let limit = COMPONENT_LIMIT.load(Ordering::Acquire);
    let class = result.is_ok().then(|| unsafe { class_id.read() });
    let is_window_events =
        class == Some(WINDOW_MANAGEMENT_EVENTS) && SKIP_WINDOW_EVENTS.load(Ordering::Acquire);
    let is_touch_keyboard = class == Some(TOUCH_KEYBOARD_EXPERIENCE_MANAGER)
        && SKIP_TOUCH_KEYBOARD.load(Ordering::Acquire);
    if result.is_ok() && (is_window_events || is_touch_keyboard || index >= limit) {
        // SAFETY: The COM method contract provides this writable output.
        unsafe { should_create.write(0) };
        if is_window_events || is_touch_keyboard || index == limit {
            println!(
                "phase=skip-component index={index} window_events={is_window_events} touch_keyboard={is_touch_keyboard}"
            );
        }
    }
    result
}

#[cfg(target_os = "windows")]
fn install_component_filter(
    controller: &windows::core::IUnknown,
    twinui_base: usize,
    skip_window_events: bool,
    skip_touch_keyboard: bool,
) -> Result<Box<[usize; 12]>, String> {
    use std::{ffi::c_void, sync::atomic::Ordering};
    use windows::core::Interface;

    // CImmersiveShellController stores the behavior COM interface at +0x60
    // on this build. The builder has already installed it before returning.
    // SAFETY: This private offset was verified against the matching PDB and
    // the controller stays alive throughout this probe.
    let behavior = unsafe {
        controller
            .as_raw()
            .cast::<u8>()
            .add(0x60)
            .cast::<*mut c_void>()
            .read()
    };
    if behavior.is_null() {
        return Err("builder did not install creation behavior".into());
    }
    // SAFETY: The live COM interface begins with a valid vtable pointer.
    let original_vtable = unsafe { behavior.cast::<*const usize>().read() };
    // The behavior interface has 12 slots on this Windows build. Slot 8 is
    // CImmersiveShellCreationBehavior::ShouldCreateComponent.
    let method = unsafe { original_vtable.add(8).read() };
    const SHOULD_CREATE_RVA: usize = 0x1806d0;
    let expected = twinui_base + SHOULD_CREATE_RVA;
    println!("phase=creation-behavior should_create={method:#x} expected={expected:#x}");
    if method != expected {
        return Err("creation behavior vtable does not match this Windows build".into());
    }
    let mut copied = Box::new([0usize; 12]);
    // SAFETY: The PDB-verified COM interface has exactly 12 vtable slots.
    unsafe { std::ptr::copy_nonoverlapping(original_vtable, copied.as_mut_ptr(), copied.len()) };
    copied[8] = should_create_component_diagnostic as *const () as usize;
    ORIGINAL_SHOULD_CREATE.store(method, Ordering::Release);
    SKIP_WINDOW_EVENTS.store(skip_window_events, Ordering::Release);
    SKIP_TOUCH_KEYBOARD.store(skip_touch_keyboard, Ordering::Release);
    let limit = std::env::var("NICKEL_FRAME_PROBE_COMPONENT_LIMIT")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(u32::MAX);
    println!("phase=creation-behavior component_limit={limit}");
    COMPONENT_LIMIT.store(limit, Ordering::Release);
    // SAFETY: The COM object resides in writable heap memory. The copied
    // vtable remains alive until after its owning controller is dropped.
    unsafe { behavior.cast::<*const usize>().write(copied.as_ptr()) };
    Ok(copied)
}
