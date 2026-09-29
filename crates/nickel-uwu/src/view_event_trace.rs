//! Diagnostic subscription to the shell's view-event dispatcher.

use std::{
    ffi::c_void,
    io::Write,
    sync::atomic::{AtomicPtr, AtomicU32, Ordering},
    time::Instant,
};
use windows::{
    Win32::System::Com::{CLSCTX_LOCAL_SERVER, CoCreateInstance, CoTaskMemFree, IServiceProvider},
    core::{GUID, HRESULT, HSTRING, IUnknown, Interface},
};

const IMMERSIVE_SHELL: GUID = GUID::from_u128(0xc2f03a33_21f5_47fa_b4bb_156362a2f239);
const VIEW_EVENT_DISPATCHER: GUID = GUID::from_u128(0x6d7258b3_08e8_4142_9b9f_64ecaee6a99f);
const VIEW_EVENT_INTERFACE: GUID = GUID::from_u128(0xf5ef6433_2dc3_4aeb_b387_7c6cba1c012e);
const VIEW_EVENT_HANDLER: GUID = GUID::from_u128(0xc1659867_0cf6_5f3b_99df_4f036280926c);
const UAP_VIEW_WRAPPER: GUID = GUID::from_u128(0xb4282c81_a540_4f5c_b46e_449970e517a8);
const SHELL_CLOAK_WRAPPER: GUID = GUID::from_u128(0x9230a21d_831d_414f_87f1_f0131a16a75a);
const FRAME_WINDOW_WRAPPER: GUID = GUID::from_u128(0x3b422c7e_2c2b_4e2c_81e3_49920376ed20);
const VIEW_WRAPPER: GUID = GUID::from_u128(0x79eaeb1a_4280_4be7_b345_9f7a790149f5);
const IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const IAGILE_OBJECT: GUID = GUID::from_u128(0x94ea2b94_e9cc_49e0_c0ff_ee64ca8f5b90);
const E_NOINTERFACE: HRESULT = HRESULT(0x80004002_u32 as i32);
const E_POINTER: HRESULT = HRESULT(0x80004003_u32 as i32);
const E_NOTIMPL: HRESULT = HRESULT(0x80004001_u32 as i32);

use crate::host::ViewEventCallbackResult;

type QueryService =
    unsafe extern "system" fn(*mut c_void, *const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
type Register = unsafe extern "system" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut i64,
) -> HRESULT;
type Unregister = unsafe extern "system" fn(*mut c_void, i64) -> HRESULT;

#[repr(C)]
struct Callback {
    vtable: *const CallbackVtable,
    references: AtomicU32,
    started: Instant,
    expected_frame_method: usize,
    result: ViewEventCallbackResult,
    retained_frame_interface: AtomicPtr<c_void>,
    retained_cloak_interface: AtomicPtr<c_void>,
    retained_frame_window_interface: AtomicPtr<c_void>,
    retained_view_wrapper: AtomicPtr<c_void>,
    process_id: AtomicU32,
}

#[repr(C)]
struct CallbackVtable {
    query_interface:
        unsafe extern "system" fn(*mut Callback, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut Callback) -> u32,
    release: unsafe extern "system" fn(*mut Callback) -> u32,
    invoke: unsafe extern "system" fn(*mut Callback, *mut c_void, *mut c_void) -> HRESULT,
}

unsafe extern "system" fn query_interface(
    this: *mut Callback,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    // SAFETY: The caller provided writable COM output storage.
    unsafe { *out = std::ptr::null_mut() };
    let requested = unsafe { *iid };
    if requested != IUNKNOWN && requested != IAGILE_OBJECT && requested != VIEW_EVENT_HANDLER {
        return E_NOINTERFACE;
    }
    unsafe { *out = this.cast() };
    unsafe { add_ref(this) };
    HRESULT(0)
}

unsafe extern "system" fn add_ref(this: *mut Callback) -> u32 {
    unsafe { (*this).references.fetch_add(1, Ordering::Relaxed) + 1 }
}

unsafe extern "system" fn release(this: *mut Callback) -> u32 {
    let remaining = unsafe { (*this).references.fetch_sub(1, Ordering::Release) - 1 };
    if remaining == 0 {
        std::sync::atomic::fence(Ordering::Acquire);
        unsafe { drop(Box::from_raw(this)) };
    }
    remaining
}

unsafe extern "system" fn invoke(
    this: *mut Callback,
    wrapper: *mut c_void,
    args: *mut c_void,
) -> HRESULT {
    let elapsed_ms = unsafe { (*this).started.elapsed().as_millis() };
    println!("phase=view-event-enter elapsed_ms={elapsed_ms} wrapper={wrapper:p} args={args:p}");
    let _ = std::io::stdout().flush();
    let mut event_type = -1;
    if !args.is_null() {
        type GetEventType = unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT;
        type GetIids = unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut GUID) -> HRESULT;
        let args_vtable = unsafe { args.cast::<*const usize>().read() };
        if !args_vtable.is_null() {
            let get_event_type: GetEventType =
                unsafe { std::mem::transmute(args_vtable.add(6).read()) };
            let status = unsafe { get_event_type(args, &mut event_type) };
            println!("phase=view-event-type hresult={status:?} value={event_type}");

            // The event type alone does not distinguish all of the launch
            // geometry carried by ShowAsViewMode. Record the concrete event
            // interfaces so their ABI can be matched against the PDB before
            // calling any build-private property getters.
            let get_iids: GetIids = unsafe { std::mem::transmute(args_vtable.add(3).read()) };
            let mut iid_count = 0;
            let mut iids = std::ptr::null_mut();
            let iid_status = unsafe { get_iids(args, &mut iid_count, &mut iids) };
            println!("phase=view-event-args-iids hresult={iid_status:?} count={iid_count}");
            if iid_status.is_ok() && !iids.is_null() {
                for (index, iid) in unsafe { std::slice::from_raw_parts(iids, iid_count as usize) }
                    .iter()
                    .enumerate()
                {
                    println!("phase=view-event-args-iid index={index} iid={iid:?}");
                }
                unsafe { CoTaskMemFree(Some(iids.cast())) };
            }
        }
    }
    let mut frame = None;
    let mut frame_method = 0;
    let mut frame_slot = None;
    let mut frame_interface: *mut c_void = std::ptr::null_mut();
    if !wrapper.is_null() {
        type GetIids = unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut GUID) -> HRESULT;
        type QueryInterface =
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
        // SAFETY: Invoke supplies an IViewWrapper valid for the callback. Its
        // IInspectable prefix provides GetIids in slot 3 and QueryInterface in
        // slot 0. Successful queries return owned references released below.
        let vtable = unsafe { wrapper.cast::<*const usize>().read() };
        if !vtable.is_null() {
            let get_iids: GetIids = unsafe { std::mem::transmute(vtable.add(3).read()) };
            let query_interface: QueryInterface = unsafe { std::mem::transmute(vtable.read()) };
            let mut iid_count = 0;
            let mut iids = std::ptr::null_mut();
            let iid_status = unsafe { get_iids(wrapper, &mut iid_count, &mut iids) };
            println!("phase=view-event-iids hresult={iid_status:?} count={iid_count}");
            if iid_status.is_ok() && !iids.is_null() {
                for (index, iid) in unsafe { std::slice::from_raw_parts(iids, iid_count as usize) }
                    .iter()
                    .enumerate()
                {
                    let mut interface = std::ptr::null_mut();
                    let status = unsafe { query_interface(wrapper, iid, &mut interface) };
                    let candidate_vtable = if status.is_ok() && !interface.is_null() {
                        unsafe { interface.cast::<*const usize>().read() }
                    } else {
                        std::ptr::null()
                    };
                    println!(
                        "phase=view-event-iid index={index} iid={iid:?} hresult={status:?} interface={interface:p} vtable={candidate_vtable:p}"
                    );
                    if status.is_ok() && !interface.is_null() {
                        if *iid == VIEW_WRAPPER {
                            let previous = unsafe {
                                (*this)
                                    .retained_view_wrapper
                                    .swap(interface, Ordering::AcqRel)
                            };
                            if !previous.is_null() {
                                unsafe { drop(IUnknown::from_raw(previous)) };
                            }
                            type GetProcessId =
                                unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT;
                            let mut process_id = 0;
                            let get_process_id: GetProcessId =
                                unsafe { std::mem::transmute(candidate_vtable.add(54).read()) };
                            let pid_status = unsafe { get_process_id(interface, &mut process_id) };
                            println!(
                                "phase=view-event-process-id hresult={pid_status:?} pid={process_id}"
                            );
                            if pid_status.is_ok() {
                                unsafe { (*this).process_id.store(process_id, Ordering::Release) };
                            }
                        }
                        if *iid == UAP_VIEW_WRAPPER
                            && frame_interface.is_null()
                            && !candidate_vtable.is_null()
                            && unsafe { candidate_vtable.add(44).read() }
                                == unsafe { (*this).expected_frame_method }
                        {
                            frame_interface = interface;
                            frame_slot = Some(44);
                            frame_method = unsafe { candidate_vtable.add(44).read() };
                        }
                        if *iid == SHELL_CLOAK_WRAPPER {
                            let previous = unsafe {
                                (*this)
                                    .retained_cloak_interface
                                    .swap(interface, Ordering::AcqRel)
                            };
                            if !previous.is_null() {
                                unsafe { drop(IUnknown::from_raw(previous)) };
                            }
                        }
                        if *iid == FRAME_WINDOW_WRAPPER {
                            let previous = unsafe {
                                (*this)
                                    .retained_frame_window_interface
                                    .swap(interface, Ordering::AcqRel)
                            };
                            if !previous.is_null() {
                                unsafe { drop(IUnknown::from_raw(previous)) };
                            }
                        }
                        if frame_interface != interface
                            && *iid != VIEW_WRAPPER
                            && *iid != SHELL_CLOAK_WRAPPER
                            && *iid != FRAME_WINDOW_WRAPPER
                        {
                            unsafe { drop(IUnknown::from_raw(interface)) };
                        }
                    }
                }
                unsafe { CoTaskMemFree(Some(iids.cast())) };
            }
            if !frame_interface.is_null() {
                println!(
                    "phase=view-event-frame-method interface={frame_interface:p} slot={frame_slot:?} method={frame_method:#x}"
                );
                let _ = std::io::stdout().flush();
                type GetFrameHwnd = unsafe extern "system" fn(*mut c_void) -> *mut c_void;
                let get_frame: GetFrameHwnd = unsafe { std::mem::transmute(frame_method) };
                frame = Some(unsafe { get_frame(frame_interface) } as usize);
                let previous = unsafe {
                    (*this)
                        .retained_frame_interface
                        .swap(frame_interface, Ordering::AcqRel)
                };
                if !previous.is_null() {
                    unsafe { drop(IUnknown::from_raw(previous)) };
                }
            }
        }
    }
    println!(
        "phase=view-event elapsed_ms={elapsed_ms} type={event_type} wrapper={wrapper:p} args={args:p} frame_slot={frame_slot:?} frame_method={frame_method:#x} frame={frame:?}"
    );
    let result = match unsafe { (*this).result } {
        ViewEventCallbackResult::Success => HRESULT(0),
        ViewEventCallbackResult::NotImplemented => E_NOTIMPL,
    };
    println!("phase=view-event-return hresult={result:?}");
    let _ = std::io::stdout().flush();
    result
}

static CALLBACK_VTABLE: CallbackVtable = CallbackVtable {
    query_interface,
    add_ref,
    release,
    invoke,
};

pub(crate) struct ViewEventTrace {
    dispatcher: IUnknown,
    callback: *mut Callback,
    token: i64,
    last_frame: usize,
    last_core: usize,
}

impl ViewEventTrace {
    pub(crate) fn register(result: ViewEventCallbackResult, app_id: &str) -> Result<Self, String> {
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::core::w;

        // SAFETY: The host STA initialized COM and owns the running controller.
        let shell: IServiceProvider =
            unsafe { CoCreateInstance(&IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER) }
                .map_err(|error| format!("create ImmersiveShell: {error}"))?;
        let query: QueryService = unsafe {
            let vtable = shell.as_raw().cast::<*const usize>().read();
            std::mem::transmute(vtable.add(3).read())
        };
        let mut raw = std::ptr::null_mut();
        unsafe {
            query(
                shell.as_raw(),
                &VIEW_EVENT_DISPATCHER,
                &VIEW_EVENT_INTERFACE,
                &mut raw,
            )
        }
        .ok()
        .map_err(|error| format!("query IViewEventDispatcher: {error}"))?;
        if raw.is_null() {
            return Err("view-event dispatcher returned null".into());
        }
        let dispatcher = unsafe { IUnknown::from_raw(raw) };
        let vtable = unsafe { dispatcher.as_raw().cast::<*const usize>().read() };
        let register: Register = unsafe { std::mem::transmute(vtable.add(6).read()) };
        let module = unsafe { GetModuleHandleW(w!("twinui.pcshell.dll")) }
            .map_err(|error| format!("find twinui.pcshell.dll: {error}"))?;
        let expected_frame_method = crate::symbols::address(
            "twinui.pcshell.dll",
            "?GetFrameHwnd@UwpWindowWrapperBase@@UEAAPEAUHWND__@@XZ",
        )?;
        let callback = Box::into_raw(Box::new(Callback {
            vtable: &CALLBACK_VTABLE,
            references: AtomicU32::new(1),
            started: Instant::now(),
            expected_frame_method,
            result,
            retained_frame_interface: AtomicPtr::new(std::ptr::null_mut()),
            retained_cloak_interface: AtomicPtr::new(std::ptr::null_mut()),
            retained_frame_window_interface: AtomicPtr::new(std::ptr::null_mut()),
            retained_view_wrapper: AtomicPtr::new(std::ptr::null_mut()),
            process_id: AtomicU32::new(0),
        }));
        let filter = HSTRING::from(app_id);
        let filter_raw = unsafe { (&filter as *const HSTRING).cast::<*mut c_void>().read() };
        let mut token = 0;
        let status = unsafe {
            register(
                dispatcher.as_raw(),
                callback.cast(),
                filter_raw,
                std::ptr::null_mut(),
                &mut token,
            )
        };
        if let Err(error) = status.ok() {
            unsafe { release(callback) };
            return Err(format!("register view events: {error}"));
        }
        println!("phase=view-event-register token={token} callback_result={result:?}");
        Ok(Self {
            dispatcher,
            callback,
            token,
            last_frame: 0,
            last_core: 0,
        })
    }

    pub(crate) fn poll_frame_hwnd(&mut self) -> Option<usize> {
        let interface = unsafe {
            (*self.callback)
                .retained_frame_interface
                .load(Ordering::Acquire)
        };
        if interface.is_null() {
            return None;
        }
        let vtable = unsafe { interface.cast::<*const usize>().read() };
        if vtable.is_null()
            || unsafe { vtable.add(44).read() } != unsafe { (*self.callback).expected_frame_method }
        {
            return None;
        }
        type GetFrameHwnd = unsafe extern "system" fn(*mut c_void) -> *mut c_void;
        let get_frame: GetFrameHwnd = unsafe { std::mem::transmute(vtable.add(44).read()) };
        let frame = unsafe { get_frame(interface) } as usize;
        if frame == 0 || frame == self.last_frame {
            return None;
        }
        self.last_frame = frame;
        Some(frame)
    }

    pub(crate) fn poll_core_target(&mut self) -> Option<(usize, usize, usize, usize, usize, u32)> {
        let wrapper = unsafe {
            (*self.callback)
                .retained_view_wrapper
                .load(Ordering::Acquire)
        };
        if wrapper.is_null() {
            return None;
        }
        let vtable = unsafe { wrapper.cast::<*const usize>().read() };
        if vtable.is_null() {
            return None;
        }
        type GetProcessId = unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT;
        let get_process_id: GetProcessId = unsafe { std::mem::transmute(vtable.add(54).read()) };
        let mut process_id = 0;
        if unsafe { get_process_id(wrapper, &mut process_id) }.is_err() || process_id == 0 {
            return None;
        }
        unsafe {
            (*self.callback)
                .process_id
                .store(process_id, Ordering::Release)
        };
        let core = crate::auto_present::core_window_for_pid(process_id)?;
        if core == self.last_core {
            return None;
        }
        self.last_core = core;
        let discovery_interface = unsafe {
            (*self.callback)
                .retained_frame_interface
                .load(Ordering::Acquire)
        };
        if discovery_interface.is_null() {
            return None;
        }
        let cloak_interface = unsafe {
            (*self.callback)
                .retained_cloak_interface
                .load(Ordering::Acquire)
        };
        if cloak_interface.is_null() {
            return None;
        }
        let frame_window_interface = unsafe {
            (*self.callback)
                .retained_frame_window_interface
                .load(Ordering::Acquire)
        };
        if frame_window_interface.is_null() {
            return None;
        }
        Some((
            wrapper as usize,
            discovery_interface as usize,
            cloak_interface as usize,
            frame_window_interface as usize,
            core,
            process_id,
        ))
    }
}

impl Drop for ViewEventTrace {
    fn drop(&mut self) {
        let vtable = unsafe { self.dispatcher.as_raw().cast::<*const usize>().read() };
        let unregister: Unregister = unsafe { std::mem::transmute(vtable.add(7).read()) };
        let status = unsafe { unregister(self.dispatcher.as_raw(), self.token) };
        println!("phase=view-event-unregister hresult={status:?}");
        let retained = unsafe {
            (*self.callback)
                .retained_frame_interface
                .swap(std::ptr::null_mut(), Ordering::AcqRel)
        };
        if !retained.is_null() {
            unsafe { drop(IUnknown::from_raw(retained)) };
        }
        let retained = unsafe {
            (*self.callback)
                .retained_frame_window_interface
                .swap(std::ptr::null_mut(), Ordering::AcqRel)
        };
        if !retained.is_null() {
            unsafe { drop(IUnknown::from_raw(retained)) };
        }
        let retained = unsafe {
            (*self.callback)
                .retained_cloak_interface
                .swap(std::ptr::null_mut(), Ordering::AcqRel)
        };
        if !retained.is_null() {
            unsafe { drop(IUnknown::from_raw(retained)) };
        }
        let retained = unsafe {
            (*self.callback)
                .retained_view_wrapper
                .swap(std::ptr::null_mut(), Ordering::AcqRel)
        };
        if !retained.is_null() {
            unsafe { drop(IUnknown::from_raw(retained)) };
        }
        unsafe { release(self.callback) };
    }
}
