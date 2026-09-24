//! Automates the existing build-specific presentation experiment on the shell STA.
use std::{
    collections::{HashMap, HashSet},
    ffi::c_void,
    time::Instant,
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM},
        Storage::{
            EnhancedStorage::PKEY_AppUserModel_ID, Packaging::Appx::GetApplicationUserModelId,
        },
        System::{
            Com::{
                CLSCTX_LOCAL_SERVER, CoCreateInstance, CoTaskMemFree, IServiceProvider,
                StructuredStorage::PropVariantToStringAlloc,
            },
            Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        },
        UI::{
            Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow},
            WindowsAndMessaging::{
                EnumChildWindows, EnumWindows, GetClassNameW, GetParent, GetWindowThreadProcessId,
                IsWindow,
            },
        },
    },
    core::{BOOL, GUID, HRESULT, IUnknown, Interface, PWSTR},
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Window {
    hwnd: usize,
    pid: u32,
}

fn windows() -> Vec<(Window, bool)> {
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
        let mut name = [0u16; 128];
        // SAFETY: Windows validates the HWND and the output buffers are writable.
        let length = unsafe { GetClassNameW(hwnd, &mut name) }.max(0) as usize;
        let name = String::from_utf16_lossy(&name[..length]);
        if name == "ApplicationFrameWindow" || name == "Windows.UI.Core.CoreWindow" {
            let mut pid = 0;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            // SAFETY: EnumWindows/EnumChildWindows invoke synchronously with this Vec.
            unsafe { &mut *(data.0 as *mut Vec<(Window, bool)>) }.push((
                Window {
                    hwnd: hwnd.0 as usize,
                    pid,
                },
                name == "ApplicationFrameWindow",
            ));
        }
        BOOL(1)
    }
    unsafe extern "system" fn top(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: Synchronous callbacks share the same valid output Vec.
        let _ = unsafe { visit(hwnd, data) };
        let _ = unsafe { EnumChildWindows(Some(hwnd), Some(visit), data) };
        BOOL(1)
    }
    let mut result = Vec::new();
    // SAFETY: The Vec outlives the synchronous enumeration.
    let _ = unsafe { EnumWindows(Some(top), LPARAM(&mut result as *mut _ as isize)) };
    result
}

fn process_app_id(pid: u32) -> Option<String> {
    // SAFETY: Windows checks the requested process access and output buffer.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;
    let status = unsafe {
        GetApplicationUserModelId(process, &mut length, Some(PWSTR(buffer.as_mut_ptr())))
    };
    let _ = unsafe { CloseHandle(process) };
    if status.0 != 0 || length == 0 || length as usize > buffer.len() {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..length as usize - 1]))
}

fn frame_app_id(hwnd: usize) -> Option<String> {
    // SAFETY: The property store owns its returned variant; conversion allocates
    // a COM task string, which is freed after copying it.
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(HWND(hwnd as *mut c_void)).ok()?;
        let value = store.GetValue(&PKEY_AppUserModel_ID).ok()?;
        let text = PropVariantToStringAlloc(&value).ok()?;
        let result = text.to_string().ok();
        CoTaskMemFree(Some(text.0.cast()));
        result.filter(|s| !s.is_empty())
    }
}

fn uncloak(app_id: &str) -> windows::core::Result<()> {
    const SHELL: GUID = GUID::from_u128(0xc2f03a33_21f5_47fa_b4bb_156362a2f239);
    const COLLECTION: GUID = GUID::from_u128(0x1841c6d7_4f9d_42c0_af41_8747538f10e5);
    type Lookup = unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut c_void) -> HRESULT;
    type Cloak = unsafe extern "system" fn(*mut c_void, u32, i32) -> HRESULT;
    // SAFETY: Runs on the initialized shell STA. These are the same private
    // interface slots verified and used by switch-view on this Windows build.
    unsafe {
        let shell: IServiceProvider = CoCreateInstance(&SHELL, None, CLSCTX_LOCAL_SERVER)?;
        let mut raw = std::ptr::null_mut();
        (shell.vtable().QueryService)(shell.as_raw(), &COLLECTION, &COLLECTION, &mut raw).ok()?;
        if raw.is_null() {
            return Err(windows::core::Error::from_hresult(HRESULT(
                0x80004003u32 as i32,
            )));
        }
        let collection = IUnknown::from_raw(raw);
        let lookup: Lookup = std::mem::transmute(
            collection
                .as_raw()
                .cast::<*const usize>()
                .read()
                .add(8)
                .read(),
        );
        let app_id: Vec<u16> = app_id.encode_utf16().chain(Some(0)).collect();
        raw = std::ptr::null_mut();
        lookup(collection.as_raw(), app_id.as_ptr(), &mut raw).ok()?;
        if raw.is_null() {
            return Err(windows::core::Error::from_hresult(HRESULT(
                0x80004003u32 as i32,
            )));
        }
        let view = IUnknown::from_raw(raw);
        let cloak: Cloak =
            std::mem::transmute(view.as_raw().cast::<*const usize>().read().add(12).read());
        cloak(view.as_raw(), 1, 0).ok()
    }
}

struct Pending {
    started: Instant,
    last_error: String,
}

pub struct AutoPresent {
    preexisting: HashSet<Window>,
    complete: HashSet<(Window, Window)>,
    pending: HashMap<Window, Pending>,
}

impl AutoPresent {
    /// Snapshot before controller startup: do not attach orphan windows left by
    /// an earlier shell session to a newly activated instance of the same app.
    pub fn new() -> Self {
        Self {
            preexisting: windows()
                .into_iter()
                .filter(|(_, frame)| !frame)
                .map(|(w, _)| w)
                .collect(),
            complete: HashSet::new(),
            pending: HashMap::new(),
        }
    }

    pub fn tick(&mut self) {
        let all = windows();
        let live: HashSet<_> = all.iter().map(|(w, _)| *w).collect();
        self.complete
            .retain(|(frame, core)| live.contains(frame) && live.contains(core));
        self.pending.retain(|frame, _| live.contains(frame));
        self.preexisting.retain(|w| live.contains(w));
        let cores: Vec<_> = all
            .iter()
            .filter(|(w, frame)| !frame && !self.preexisting.contains(w))
            .filter_map(|(w, _)| process_app_id(w.pid).map(|id| (*w, id)))
            .collect();
        for (frame, is_frame) in &all {
            if !is_frame || self.complete.iter().any(|(done, _)| done == frame) {
                continue;
            }
            let Some(app_id) = frame_app_id(frame.hwnd) else {
                continue;
            };
            let candidates: Vec<_> = cores
                .iter()
                .filter(|(_, id)| id == &app_id)
                .map(|(w, _)| *w)
                .collect();
            // Never guess between multiple views of the same app. Keep retries
            // independent so an ambiguous or slow app cannot block another.
            let matching_frames = all
                .iter()
                .filter(|(_, is_frame)| *is_frame)
                .filter(|(w, _)| frame_app_id(w.hwnd).as_deref() == Some(&app_id))
                .count();
            if candidates.len() != 1 || matching_frames != 1 {
                continue;
            }
            let core = candidates[0];
            let pending = self.pending.entry(*frame).or_insert_with(|| Pending {
                started: Instant::now(),
                last_error: String::new(),
            });
            if pending.started.elapsed().as_secs() >= 15 {
                if pending.last_error != "presentation-timeout" {
                    eprintln!(
                        "phase=auto-present app={app_id:?} frame={:#x} error=presentation-timeout",
                        frame.hwnd
                    );
                    pending.last_error = "presentation-timeout".into();
                }
                continue;
            }
            let result = Self::present(*frame, core, &app_id);
            match result {
                Ok(true) => {
                    println!(
                        "phase=auto-present result=ready app={app_id:?} frame={:#x} core={:#x} elapsed_ms={}",
                        frame.hwnd,
                        core.hwnd,
                        pending.started.elapsed().as_millis()
                    );
                    self.complete.insert((*frame, core));
                    self.pending.remove(frame);
                }
                Ok(false) => {}
                Err(error) if error != pending.last_error => {
                    eprintln!("phase=auto-present app={app_id:?} error={error}");
                    pending.last_error = error;
                }
                Err(_) => {}
            }
        }
    }

    fn present(frame: Window, core: Window, app_id: &str) -> Result<bool, String> {
        let wrapper =
            crate::wrapper_inspect::find_wrapper(frame.hwnd).map_err(|e| e.to_string())?;
        if wrapper.client != 0 && wrapper.client != core.hwnd {
            return Err("wrapper already owns a different CoreWindow".into());
        }
        let raw = (wrapper.interface - 0x20) as *mut c_void;
        // SAFETY: The finder verified the live wrapper vtable. This runs on its
        // owning shell STA; retain a COM reference across reentrant calls.
        let _keep_alive = unsafe { IUnknown::from_raw_borrowed(&raw) }
            .ok_or("null wrapper")?
            .clone();
        // SAFETY: Windows validates the observed HWNDs before mutation.
        if !unsafe { IsWindow(Some(HWND(core.hwnd as *mut c_void))) }.as_bool() {
            return Ok(false);
        }
        if wrapper.client == 0 {
            crate::probe_window_discovery(wrapper.interface, core.hwnd)?;
        }
        crate::probe_window_visibility(wrapper.interface, core.hwnd)?;
        crate::probe_window_layout(wrapper.interface, core.hwnd)?;
        uncloak(app_id).map_err(|e| e.to_string())?;
        // SAFETY: Parenting to the correct AFH frame is the observable result of
        // SetPresentedWindow; do not report success based only on an HRESULT.
        Ok(unsafe { GetParent(HWND(core.hwnd as *mut c_void)) }.ok()
            == Some(HWND(frame.hwnd as *mut c_void)))
    }
}
