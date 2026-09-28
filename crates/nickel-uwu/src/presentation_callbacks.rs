//! Build-specific callbacks that advance a CoreWindow to presentation readiness.

#[cfg(target_os = "windows")]
#[cfg(target_os = "windows")]
pub(crate) fn probe_view_wrapper_discovery(interface: usize, hwnd: usize) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::Win32::{Foundation::HWND, System::LibraryLoader::GetModuleHandleW};

    if interface == 0
        || hwnd == 0
        || !unsafe {
            windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(HWND(hwnd as *mut c_void)))
        }
        .as_bool()
    {
        return Err("invalid wrapper interface or HWND".into());
    }
    let module = unsafe { GetModuleHandleW(windows::core::w!("twinui.pcshell.dll")) }
        .map_err(|error| error.to_string())?;
    let expected = module.0 as usize + 0x8e960;
    let vtable = unsafe { (interface as *const *const usize).read() };
    let method = if vtable.is_null() {
        0
    } else {
        unsafe { vtable.add(24).read() }
    };
    if method != expected {
        return Err(format!(
            "unexpected discovery method {method:#x}; expected {expected:#x}"
        ));
    }
    type DiscoverWindow = unsafe extern "system" fn(*mut c_void, *mut c_void);
    let discover: DiscoverWindow = unsafe { std::mem::transmute(method) };
    unsafe { discover(interface as *mut c_void, hwnd as *mut c_void) };
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn probe_view_wrapper_readiness(interface: usize, hwnd: usize) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::Win32::{Foundation::HWND, System::LibraryLoader::GetModuleHandleW};

    if interface == 0
        || hwnd == 0
        || !unsafe {
            windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(HWND(hwnd as *mut c_void)))
        }
        .as_bool()
    {
        return Err("invalid wrapper interface or HWND".into());
    }
    let module = unsafe { GetModuleHandleW(windows::core::w!("twinui.pcshell.dll")) }
        .map_err(|error| error.to_string())?;
    let expected_visibility = module.0 as usize + 0x1aa550;
    let expected_layout = module.0 as usize + 0x1a36f0;
    let vtable = unsafe { (interface as *const *const usize).read() };
    if vtable.is_null() {
        return Err("null wrapper vtable".into());
    }
    let visibility_method = unsafe { vtable.add(40).read() };
    let layout_method = unsafe { vtable.add(41).read() };
    if visibility_method != expected_visibility || layout_method != expected_layout {
        return Err(format!(
            "unexpected readiness methods visibility={visibility_method:#x} layout={layout_method:#x}"
        ));
    }
    type VisibilityChanged = unsafe extern "system" fn(*mut c_void, u32, u32);
    type LayoutEvent = unsafe extern "system" fn(*mut c_void, u64);
    let visibility: VisibilityChanged = unsafe { std::mem::transmute(visibility_method) };
    let layout: LayoutEvent = unsafe { std::mem::transmute(layout_method) };
    unsafe {
        visibility(interface as *mut c_void, 1, 1);
        layout(interface as *mut c_void, 0x26);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn probe_view_wrapper_uncloak(interface: usize) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::{Win32::System::LibraryLoader::GetModuleHandleW, core::HRESULT};

    if interface == 0 {
        return Err("null shell-cloak interface".into());
    }
    let module = unsafe { GetModuleHandleW(windows::core::w!("twinui.pcshell.dll")) }
        .map_err(|error| error.to_string())?;
    let expected = module.0 as usize + 0x1a69a0;
    let vtable = unsafe { (interface as *const *const usize).read() };
    let method = if vtable.is_null() {
        0
    } else {
        unsafe { vtable.add(3).read() }
    };
    if method != expected {
        return Err(format!(
            "unexpected shell-cloak method {method:#x}; expected {expected:#x}"
        ));
    }
    type SetShellCloak = unsafe extern "system" fn(*mut c_void, u32) -> HRESULT;
    let set_shell_cloak: SetShellCloak = unsafe { std::mem::transmute(method) };
    unsafe { set_shell_cloak(interface as *mut c_void, 0) }
        .ok()
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
pub(crate) fn probe_view_wrapper_foreground(interface: usize) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::{Win32::System::LibraryLoader::GetModuleHandleW, core::HRESULT};

    if interface == 0 {
        return Err("null foreground interface".into());
    }
    let module = unsafe { GetModuleHandleW(windows::core::w!("twinui.pcshell.dll")) }
        .map_err(|error| error.to_string())?;
    let expected = module.0 as usize + 0x1cb270;
    let vtable = unsafe { (interface as *const *const usize).read() };
    let method = if vtable.is_null() {
        0
    } else {
        unsafe { vtable.add(7).read() }
    };
    if method != expected {
        return Err(format!(
            "unexpected foreground method {method:#x}; expected {expected:#x}"
        ));
    }
    type SetForegroundWindow = unsafe extern "system" fn(*mut c_void) -> HRESULT;
    let set_foreground: SetForegroundWindow = unsafe { std::mem::transmute(method) };
    unsafe { set_foreground(interface as *mut c_void) }
        .ok()
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
pub(crate) fn probe_view_wrapper_set_frame(
    interface: usize,
    frame_hwnd: usize,
) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::{
        Win32::{Foundation::HWND, System::LibraryLoader::GetModuleHandleW},
        core::HRESULT,
    };

    if interface == 0
        || frame_hwnd == 0
        || !unsafe {
            windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(HWND(frame_hwnd as *mut c_void)))
        }
        .as_bool()
    {
        return Err("invalid frame-window interface or HWND".into());
    }
    let module = unsafe { GetModuleHandleW(windows::core::w!("twinui.pcshell.dll")) }
        .map_err(|error| error.to_string())?;
    let expected = module.0 as usize + 0x1a8f40;
    let vtable = unsafe { (interface as *const *const usize).read() };
    let method = if vtable.is_null() {
        0
    } else {
        unsafe { vtable.add(3).read() }
    };
    if method != expected {
        return Err(format!(
            "unexpected SetFrameWindow method {method:#x}; expected {expected:#x}"
        ));
    }
    type SetFrameWindow = unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> HRESULT;
    let set_frame_window: SetFrameWindow = unsafe { std::mem::transmute(method) };
    unsafe { set_frame_window(interface as *mut c_void, frame_hwnd as *mut c_void, 0) }
        .ok()
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
pub(crate) fn probe_view_wrapper_set_size(
    interface: usize,
    frame_hwnd: usize,
) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::{
        Win32::{
            Foundation::{HWND, RECT},
            System::LibraryLoader::GetModuleHandleW,
            UI::WindowsAndMessaging::GetClientRect,
        },
        core::HRESULT,
    };

    if interface == 0 || frame_hwnd == 0 {
        return Err("invalid view wrapper interface or frame HWND".into());
    }
    let module = unsafe { GetModuleHandleW(windows::core::w!("twinui.pcshell.dll")) }
        .map_err(|error| error.to_string())?;
    let expected = module.0 as usize + 0x456640;
    let vtable = unsafe { (interface as *const *const usize).read() };
    let method = if vtable.is_null() {
        0
    } else {
        unsafe { vtable.add(8).read() }
    };
    if method != expected {
        return Err(format!(
            "unexpected SetSize method {method:#x}; expected {expected:#x}"
        ));
    }
    let mut client = RECT::default();
    unsafe { GetClientRect(HWND(frame_hwnd as *mut c_void), &mut client) }
        .map_err(|error| error.to_string())?;
    let width = (client.right - client.left) as f32;
    let height = (client.bottom - client.top) as f32;
    let size = u64::from(width.to_bits()) | (u64::from(height.to_bits()) << 32);
    type SetSize = unsafe extern "system" fn(*mut c_void, u64) -> HRESULT;
    let set_size: SetSize = unsafe { std::mem::transmute(method) };
    unsafe { set_size(interface as *mut c_void, size) }
        .ok()
        .map_err(|error| error.to_string())?;
    println!("phase=view-event-set-size width={width} height={height} result=called");
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn probe_frame_set_position(frame_proxy: usize) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::{
        Win32::{
            Foundation::RECT,
            System::LibraryLoader::GetModuleHandleW,
            UI::WindowsAndMessaging::{SPI_GETWORKAREA, SystemParametersInfoW},
        },
        core::{HRESULT, IUnknown, Interface},
    };

    if frame_proxy == 0 {
        return Err("null frame proxy".into());
    }
    let module = unsafe { GetModuleHandleW(windows::core::w!("twinui.pcshell.dll")) }
        .map_err(|error| error.to_string())?;
    let base = module.0 as usize;
    let vtable = unsafe { (frame_proxy as *const *const usize).read() };
    let set_position_method = if vtable.is_null() {
        0
    } else {
        unsafe { vtable.add(4).read() }
    };
    if set_position_method != base + 0x2066d0 {
        return Err(format!(
            "unexpected frame SetPosition method {set_position_method:#x}"
        ));
    }
    let mut work_area = RECT::default();
    unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some((&raw mut work_area).cast()),
            Default::default(),
        )
    }
    .map_err(|error| format!("SPI_GETWORKAREA: {error}"))?;
    let work_width = work_area.right - work_area.left;
    let work_height = work_area.bottom - work_area.top;
    let width = 800.min(work_width).max(320);
    let height = 700.min(work_height).max(480);
    let left = work_area.left + (work_width - width) / 2;
    let top = work_area.top + (work_height - height) / 2;
    let bounds = RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    };
    type MakePosition = unsafe extern "system" fn(*mut *mut c_void, *const RECT) -> HRESULT;
    type SetPosition = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT;
    let make_position: MakePosition = unsafe { std::mem::transmute(base + 0x1176e8) };
    let set_position: SetPosition = unsafe { std::mem::transmute(set_position_method) };
    let mut position = std::ptr::null_mut();
    unsafe { make_position(&mut position, &bounds) }
        .ok()
        .map_err(|error| format!("create position for {bounds:?}: {error}"))?;
    if position.is_null() {
        return Err("position factory returned null".into());
    }
    let status = unsafe { set_position(frame_proxy as *mut c_void, position) };
    unsafe { drop(IUnknown::from_raw(position)) };
    status
        .ok()
        .map_err(|error| format!("set position to {bounds:?}: {error}"))?;
    println!("phase=view-event-set-position bounds={bounds:?} result=called");
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn probe_frame_set_presented_window(
    frame_proxy: usize,
    core_hwnd: usize,
    app_id: &str,
) -> Result<usize, String> {
    use std::ffi::c_void;
    use windows::{
        Win32::{Foundation::HWND, System::LibraryLoader::GetModuleHandleW},
        core::HRESULT,
    };

    if frame_proxy == 0
        || core_hwnd == 0
        || !unsafe {
            windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(HWND(core_hwnd as *mut c_void)))
        }
        .as_bool()
    {
        return Err("invalid frame proxy or CoreWindow".into());
    }
    let module = unsafe { GetModuleHandleW(windows::core::w!("twinui.pcshell.dll")) }
        .map_err(|error| error.to_string())?;
    let base = module.0 as usize;
    let vtable = unsafe { (frame_proxy as *const *const usize).read() };
    if vtable.is_null() {
        return Err("null frame proxy vtable".into());
    }
    let get_frame_method = unsafe { vtable.add(3).read() };
    let set_presented_method = unsafe { vtable.add(6).read() };
    let set_application_id_method = unsafe { vtable.add(9).read() };
    if get_frame_method != base + 0x177b60
        || set_presented_method != base + 0x20a700
        || set_application_id_method != base + 0x20b8a0
    {
        return Err(format!(
            "unexpected frame proxy methods get={get_frame_method:#x} set-presented={set_presented_method:#x} set-app-id={set_application_id_method:#x}"
        ));
    }
    type GetFrameWindow = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;
    type SetPresentedWindow = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT;
    type SetApplicationId = unsafe extern "system" fn(*mut c_void, *const u16, i32) -> HRESULT;
    let get_frame: GetFrameWindow = unsafe { std::mem::transmute(get_frame_method) };
    let set_presented: SetPresentedWindow = unsafe { std::mem::transmute(set_presented_method) };
    let set_application_id: SetApplicationId =
        unsafe { std::mem::transmute(set_application_id_method) };
    let mut frame_hwnd = std::ptr::null_mut();
    unsafe { get_frame(frame_proxy as *mut c_void, &mut frame_hwnd) }
        .ok()
        .map_err(|error| format!("GetFrameWindow: {error}"))?;
    if frame_hwnd.is_null() {
        return Err("GetFrameWindow returned null".into());
    }
    let wide_app_id: Vec<u16> = app_id.encode_utf16().chain([0]).collect();
    unsafe { set_application_id(frame_proxy as *mut c_void, wide_app_id.as_ptr(), 0) }
        .ok()
        .map_err(|error| format!("SetApplicationId: {error}"))?;
    unsafe { set_presented(frame_proxy as *mut c_void, core_hwnd as *mut c_void) }
        .ok()
        .map_err(|error| format!("SetPresentedWindow: {error}"))?;
    Ok(frame_hwnd as usize)
}

#[cfg(target_os = "windows")]
pub(crate) fn probe_frame_fit_to_work_area(frame_proxy: usize) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::{Win32::System::LibraryLoader::GetModuleHandleW, core::HRESULT};

    if frame_proxy == 0 {
        return Err("null frame proxy".into());
    }
    let module = unsafe { GetModuleHandleW(windows::core::w!("twinui.pcshell.dll")) }
        .map_err(|error| error.to_string())?;
    let expected = module.0 as usize + 0x2f8e20;
    let vtable = unsafe { (frame_proxy as *const *const usize).read() };
    let method = if vtable.is_null() {
        0
    } else {
        unsafe { vtable.add(12).read() }
    };
    if method != expected {
        return Err(format!(
            "unexpected FitToWorkArea method {method:#x}; expected {expected:#x}"
        ));
    }
    type FitToWorkArea = unsafe extern "system" fn(*mut c_void) -> HRESULT;
    let fit_to_work_area: FitToWorkArea = unsafe { std::mem::transmute(method) };
    unsafe { fit_to_work_area(frame_proxy as *mut c_void) }
        .ok()
        .map_err(|error| error.to_string())
}
