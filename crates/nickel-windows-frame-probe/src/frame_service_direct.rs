//! Diagnostic-only call into this Windows build's private frame service.

use std::{cell::RefCell, ffi::c_void, process::Command, ptr, thread, time::Duration};

use windows::{
    Win32::System::{
        Com::{IServiceProvider, IServiceProvider_Impl},
        Ole::IObjectWithSite,
    },
    Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
    },
    core::IUnknown,
};
use windows_core::{Error, GUID, HRESULT, Interface, Result, implement};

// RVAs verified against the matching twinui.pcshell.dll public PDB on build
// 26200. The code signature and vtable slot are checked before either call.
const CREATE_FRAME_SERVICE_RVA: usize = 0x0e5284;
const COMPLETE_INITIALIZATION_RVA: usize = 0x2f8cc0;
const ENSURE_FRAME_POOL_RVA: usize = 0x0d90e0;
const MANAGER_INTERNAL_OFFSET: usize = 0xf0;
const MANAGER_FRAME_SERVICE_OFFSET: usize = 0x260;
const FRAME_SERVICE_IID: GUID = GUID::from_u128(0x88b25c81_171b_48b0_91d6_c75846bcf035);
const E_NOINTERFACE: HRESULT = HRESULT(0x80004002_u32 as i32);
const SHELL_CHROME_CONTROLS_IID: GUID = GUID::from_u128(0xd6f29401_6ea3_4757_a73c_b30abb699dc3);

thread_local! {
    static DIAGNOSTIC_CHROME_OBJECT: RefCell<Option<IServiceProvider>> = const { RefCell::new(None) };
}

type CreateFrameService =
    unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
type CompleteInitialization = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT;
type EnsureFramePool = unsafe extern "system" fn(*mut c_void) -> HRESULT;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
}

#[implement(IServiceProvider)]
struct LoggingServiceProvider;

impl IServiceProvider_Impl for LoggingServiceProvider_Impl {
    fn QueryService(
        &self,
        guidservice: *const GUID,
        riid: *const GUID,
        ppvobject: *mut *mut c_void,
    ) -> Result<()> {
        if !ppvobject.is_null() {
            // SAFETY: COM supplies a writable output slot for QueryService.
            unsafe { *ppvobject = ptr::null_mut() };
        }
        // SAFETY: COM supplies valid GUID pointers for this synchronous call.
        let (service, interface) = unsafe { (*guidservice, *riid) };
        println!("phase=QueryService service={service:?} interface={interface:?}");
        if service == SHELL_CHROME_CONTROLS_IID && interface == SHELL_CHROME_CONTROLS_IID {
            return DIAGNOSTIC_CHROME_OBJECT.with(|cell| {
                let borrowed = cell.borrow();
                let object = borrowed
                    .as_ref()
                    .ok_or_else(|| Error::from_hresult(E_NOINTERFACE))?;
                let owned = object.clone();
                // SAFETY: A cloned COM reference is transferred to the caller.
                // This is only a diagnostic placeholder: it has IUnknown methods
                // but does not implement shell chrome controls methods.
                unsafe { *ppvobject = owned.as_raw() };
                std::mem::forget(owned);
                println!("phase=QueryService chrome-placeholder=provided");
                Ok(())
            });
        }
        Err(Error::from_hresult(E_NOINTERFACE))
    }
}

pub fn probe(manager: &IUnknown, app_id: &str) -> std::result::Result<(), String> {
    let wide_name: Vec<u16> = "twinui.pcshell.dll".encode_utf16().chain([0]).collect();
    // SAFETY: The fallback redirect has loaded this DLL and remains alive
    // throughout this call.
    let base = unsafe { GetModuleHandleW(wide_name.as_ptr()) };
    if base.is_null() {
        return Err("twinui.pcshell.dll is not loaded".into());
    }
    let base = base as usize;
    // SAFETY: The matching PDB supplies this RVA. Verify the installed code
    // prefix before interpreting the address as a function pointer.
    let address = base + CREATE_FRAME_SERVICE_RVA;
    let prefix = unsafe { std::slice::from_raw_parts(address as *const u8, 7) };
    if prefix != [0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18] {
        return Err(format!("frame-service code prefix differs at {address:#x}"));
    }
    // SAFETY: This PDB-verified manager interface subobject is within the
    // manager returned by this DLL on the same Windows build.
    let internal = unsafe { manager.as_raw().cast::<u8>().add(MANAGER_INTERNAL_OFFSET) };
    // SAFETY: The subobject begins with a COM vtable pointer.
    let internal_vtable = unsafe { internal.cast::<usize>().read() };
    if !(base..base + 0x99e000).contains(&internal_vtable) {
        return Err(format!(
            "manager internal vtable outside twinui: {internal_vtable:#x}"
        ));
    }
    println!("phase=manager-internal vtable={internal_vtable:#x}");

    // The manager's initialization stores its own frame service at +0x260.
    // Use that instance so subsequent manager callbacks see the same service.
    // SAFETY: The offset comes from this build's matching PDB and disassembly.
    let mut raw_frame = unsafe {
        manager
            .as_raw()
            .cast::<u8>()
            .add(MANAGER_FRAME_SERVICE_OFFSET)
            .cast::<*mut c_void>()
            .read()
    };
    let manager_owned = !raw_frame.is_null();
    println!("phase=manager-frame-service present={manager_owned}");
    if !manager_owned {
        // SAFETY: The code prefix, function signature, and manager subobject
        // were verified against this installed DLL and its matching PDB.
        let create: CreateFrameService = unsafe { std::mem::transmute(address) };
        let result = unsafe { create(internal.cast(), &FRAME_SERVICE_IID, &mut raw_frame) };
        println!("phase=CApplicationFrameService_CreateInstance hresult={result:?}");
        result
            .ok()
            .map_err(|error| format!("frame service creation: {error}"))?;
    }
    if raw_frame.is_null() {
        return Err("frame service returned a null interface".into());
    }
    // SAFETY: For the manager-owned path, the manager keeps the COM reference
    // alive. For the factory path, this probe owns the returned reference.
    let owned_frame = if manager_owned {
        None
    } else {
        Some(unsafe { IUnknown::from_raw(raw_frame) })
    };
    let frame = if let Some(frame) = owned_frame.as_ref() {
        frame
    } else {
        // SAFETY: The manager retains this interface throughout the probe.
        unsafe { IUnknown::from_raw_borrowed(&raw_frame) }
            .ok_or_else(|| "manager frame service pointer is null".to_string())?
    };
    if !manager_owned {
        let site: IObjectWithSite = frame
            .cast()
            .map_err(|error| format!("IObjectWithSite: {error}"))?;
        // SAFETY: SetSite retains the live manager's IUnknown as its COM site.
        unsafe { site.SetSite(manager) }.map_err(|error| format!("SetSite: {error}"))?;
        println!("phase=frame-service-site result=success");
    }

    // SAFETY: This interface's vtable was identified in the matching PDB.
    let vtable = unsafe { frame.as_raw().cast::<*const usize>().read() };
    let complete_address = unsafe { vtable.add(3).read() };
    if complete_address != base + COMPLETE_INITIALIZATION_RVA {
        return Err(format!(
            "frame service method 3 differs: actual={complete_address:#x} expected={:#x}",
            base + COMPLETE_INITIALIZATION_RVA
        ));
    }
    let provider: IServiceProvider = LoggingServiceProvider.into();
    DIAGNOSTIC_CHROME_OBJECT.with(|cell| *cell.borrow_mut() = Some(provider.clone()));
    // SAFETY: Method 3 was verified as CompleteInitialization(IServiceProvider*)
    // and both COM objects remain alive through the call.
    let complete: CompleteInitialization = unsafe { std::mem::transmute(complete_address) };
    let result = unsafe { complete(frame.as_raw(), provider.as_raw()) };
    DIAGNOSTIC_CHROME_OBJECT.with(|cell| *cell.borrow_mut() = None);
    println!("phase=CompleteInitialization hresult={result:?}");
    result
        .ok()
        .map_err(|error| format!("frame service initialization: {error}"))?;

    // SAFETY: The installed PDB identifies vtable slot 5 as EnsureFramePool.
    let ensure_address = unsafe { vtable.add(5).read() };
    if ensure_address != base + ENSURE_FRAME_POOL_RVA {
        return Err(format!(
            "frame service method 5 differs: actual={ensure_address:#x} expected={:#x}",
            base + ENSURE_FRAME_POOL_RVA
        ));
    }
    println!("phase=EnsureFramePool starting");
    // SAFETY: The method signature and address were verified against this DLL.
    let ensure: EnsureFramePool = unsafe { std::mem::transmute(ensure_address) };
    let result = unsafe { ensure(frame.as_raw()) };
    println!("phase=EnsureFramePool hresult={result:?}");
    result
        .ok()
        .map_err(|error| format!("frame pool creation: {error}"))?;

    let launcher = std::env::current_exe()
        .map_err(|error| format!("fixture path: {error}"))?
        .with_file_name("nickel-windows-app-probe.exe");
    println!("phase=ActivateApplication starting");
    let mut child = Command::new(launcher)
        .arg(app_id)
        .spawn()
        .map_err(|error| format!("activation probe: {error}"))?;
    let mut dispatched = 0_u64;
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("activation probe wait: {error}"))?
        {
            break status;
        }
        let mut message = MSG::default();
        // SAFETY: The message is initialized, and all messages belong to this
        // worker's UI thread, which owns the diagnostic shell window.
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            // SAFETY: PeekMessageW returned a valid message for this thread.
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            dispatched += 1;
        }
        thread::sleep(Duration::from_millis(10));
    };
    println!("phase=ActivateApplication dispatched_messages={dispatched}");
    println!("phase=ActivateApplication probe_status={status}");
    if status.success() {
        Ok(())
    } else {
        Err(format!("activation probe exited with {status}"))
    }
}
