//! Build-specific callbacks that advance a CoreWindow to presentation readiness.

#[cfg(target_os = "windows")]
pub(crate) fn probe_window_discovery(wrapper: usize, hwnd: usize) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::Win32::{
        Foundation::HWND, System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::IsWindow,
    };

    const DISCOVER_WINDOW_RVA: usize = 0x8e960;
    const PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20];
    let discovered_interface = wrapper
        .checked_sub(0x20)
        .ok_or("wrapper pointer is too small")?;
    if wrapper == 0 || hwnd == 0 || !unsafe { IsWindow(Some(HWND(hwnd as *mut c_void))) }.as_bool()
    {
        return Err("invalid wrapper or HWND".into());
    }
    // SAFETY: The controller loads this Windows DLL before pumping messages.
    let module = unsafe { GetModuleHandleW(windows::core::w!("twinui.pcshell.dll")) }
        .map_err(|error| error.to_string())?;
    let address = module.0 as usize + DISCOVER_WINDOW_RVA;
    // SAFETY: The module is loaded, and this build's PDB and disassembly
    // identified the function at this RVA. The byte check rejects other
    // builds before the diagnostic call.
    let actual = unsafe { std::slice::from_raw_parts(address as *const u8, PROLOGUE.len()) };
    if actual != PROLOGUE {
        return Err(format!(
            "unexpected window discovery prologue at {address:#x}"
        ));
    }
    type DiscoverWindow = unsafe extern "system" fn(*mut c_void, *mut c_void);
    // SAFETY: The caller supplies a live WRL wrapper interface pointer
    // observed at add_ClientWindowReadyForPresentationChanged in this process.
    // The WindowDiscoveredFromShellHook interface is 0x20 bytes earlier in
    // this build's live object layout. The HWND belongs to the same app.
    let discover: DiscoverWindow = unsafe { std::mem::transmute(address) };
    unsafe { discover(discovered_interface as *mut c_void, hwnd as *mut c_void) };
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn probe_window_visibility(wrapper: usize, hwnd: usize) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::Win32::{
        Foundation::HWND, System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::IsWindow,
    };

    const VISIBILITY_CHANGED_RVA: usize = 0x1aa550;
    const PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18];
    let visibility_interface = wrapper
        .checked_sub(0x20)
        .ok_or("wrapper pointer is too small")?;
    if wrapper == 0 || hwnd == 0 || !unsafe { IsWindow(Some(HWND(hwnd as *mut c_void))) }.as_bool()
    {
        return Err("invalid wrapper or HWND".into());
    }
    // SAFETY: The caller supplies a live wrapper pointer identified by the
    // matching PDB, and +0x138 holds this wrapper's client HWND on this build.
    let client_hwnd = unsafe { ((wrapper + 0x138) as *const usize).read() };
    if client_hwnd != hwnd {
        return Err(format!(
            "wrapper client HWND {client_hwnd:#x} differs from {hwnd:#x}"
        ));
    }
    // SAFETY: The controller loads this Windows DLL before pumping messages.
    let module = unsafe { GetModuleHandleW(windows::core::w!("twinui.pcshell.dll")) }
        .map_err(|error| error.to_string())?;
    let address = module.0 as usize + VISIBILITY_CHANGED_RVA;
    // SAFETY: The module is loaded; the PDB and prologue guard identify this
    // private diagnostic method on the current Windows build.
    let actual = unsafe { std::slice::from_raw_parts(address as *const u8, PROLOGUE.len()) };
    if actual != PROLOGUE {
        return Err(format!("unexpected visibility prologue at {address:#x}"));
    }
    type VisibilityChanged = unsafe extern "system" fn(*mut c_void, u32, u32);
    // SAFETY: The live wrapper is checked against its client HWND above. The
    // interface offset and EventPhase=1, Visibility=1 come from this build's
    // PDB and disassembly. The host's shell thread owns the controller.
    let visibility_changed: VisibilityChanged = unsafe { std::mem::transmute(address) };
    unsafe { visibility_changed(visibility_interface as *mut c_void, 1, 1) };
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn probe_window_layout(wrapper: usize, hwnd: usize) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::Win32::{
        Foundation::HWND, System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::IsWindow,
    };

    const LAYOUT_EVENT_RVA: usize = 0x1a36f0;
    const PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20];
    let event_interface = wrapper
        .checked_sub(0x20)
        .ok_or("wrapper pointer is too small")?;
    if wrapper == 0 || hwnd == 0 || !unsafe { IsWindow(Some(HWND(hwnd as *mut c_void))) }.as_bool()
    {
        return Err("invalid wrapper or HWND".into());
    }
    // SAFETY: The pointer comes from a live wrapper in this diagnostic host;
    // this build's layout has its client HWND at interface +0x138.
    let client_hwnd = unsafe { ((wrapper + 0x138) as *const usize).read() };
    if client_hwnd != hwnd {
        return Err(format!(
            "wrapper client HWND {client_hwnd:#x} differs from {hwnd:#x}"
        ));
    }
    // SAFETY: The controller loads this Windows DLL before pumping messages.
    let module = unsafe { GetModuleHandleW(windows::core::w!("twinui.pcshell.dll")) }
        .map_err(|error| error.to_string())?;
    let address = module.0 as usize + LAYOUT_EVENT_RVA;
    // SAFETY: The PDB and prologue guard identify this private diagnostic
    // method on the current Windows build.
    let actual = unsafe { std::slice::from_raw_parts(address as *const u8, PROLOGUE.len()) };
    if actual != PROLOGUE {
        return Err(format!("unexpected layout event prologue at {address:#x}"));
    }
    type LayoutEvent = unsafe extern "system" fn(*mut c_void, u64);
    // SAFETY: The live wrapper's HWND was checked above. This build's PDB and
    // disassembly identify event 0x26 as clearing layout wait flag 2.
    let layout_event: LayoutEvent = unsafe { std::mem::transmute(address) };
    // SAFETY: On this build the readiness wait flags are immediately after
    // the client HWND, whose value was checked above.
    let flags = (wrapper + 0x140) as *const u32;
    let before = unsafe { flags.read() };
    unsafe { layout_event(event_interface as *mut c_void, 0x26) };
    let after = unsafe { flags.read() };
    println!("phase=layout-wait-flags before={before:#x} after={after:#x}");
    Ok(())
}
