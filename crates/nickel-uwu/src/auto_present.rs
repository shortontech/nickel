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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

fn unique_core_for_app(
    app_id: &str,
    frames: &[(Window, String)],
    cores: &[(Window, String)],
    preexisting_frames: &HashSet<Window>,
    preexisting_cores: &HashSet<Window>,
) -> Option<Window> {
    let mut matching_frames = frames
        .iter()
        .filter(|(window, id)| id == app_id && !preexisting_frames.contains(window));
    matching_frames.next()?;
    if matching_frames.next().is_some() {
        return None;
    }
    // A newly created view wins over an orphan from an earlier shell session.
    // If activation reused an existing CoreWindow, accept it only when it is
    // the sole unparented candidate for this application.
    let mut new_cores = cores
        .iter()
        .filter(|(window, id)| id == app_id && !preexisting_cores.contains(window));
    if let Some((core, _)) = new_cores.next() {
        return new_cores.next().is_none().then_some(*core);
    }
    let mut old_cores = cores.iter().filter(|(_, id)| id == app_id);
    let core = old_cores.next()?.0;
    old_cores.next().is_none().then_some(core)
}

pub struct AutoPresent {
    preexisting_frames: HashSet<Window>,
    preexisting_cores: HashSet<Window>,
    complete: HashSet<(Window, Window)>,
    pending: HashMap<Window, Pending>,
    unmatched: HashMap<Window, (usize, usize, usize)>,
}

impl AutoPresent {
    /// Snapshot before controller startup: do not attach orphan windows left by
    /// an earlier shell session to a newly activated instance of the same app.
    pub fn new() -> Self {
        let snapshot = windows();
        Self {
            preexisting_frames: snapshot
                .iter()
                .filter(|(_, frame)| *frame)
                .map(|(window, _)| *window)
                .collect(),
            preexisting_cores: snapshot
                .iter()
                .filter(|(_, frame)| !frame)
                .map(|(window, _)| *window)
                .collect(),
            complete: HashSet::new(),
            pending: HashMap::new(),
            unmatched: HashMap::new(),
        }
    }

    pub fn tick(&mut self) {
        let all = windows();
        let live: HashSet<_> = all.iter().map(|(w, _)| *w).collect();
        self.complete
            .retain(|(frame, core)| live.contains(frame) && live.contains(core));
        self.pending.retain(|frame, _| live.contains(frame));
        self.unmatched.retain(|frame, _| live.contains(frame));
        self.preexisting_frames.retain(|w| live.contains(w));
        self.preexisting_cores.retain(|w| live.contains(w));
        let cores: Vec<_> = all
            .iter()
            .filter(|(window, frame)| {
                !frame && unsafe { GetParent(HWND(window.hwnd as *mut c_void)) }.is_err()
            })
            .filter_map(|(w, _)| process_app_id(w.pid).map(|id| (*w, id)))
            .collect();
        let frames: Vec<_> = all
            .iter()
            .filter(|(_, is_frame)| *is_frame)
            .filter_map(|(window, _)| frame_app_id(window.hwnd).map(|id| (*window, id)))
            .collect();
        for (frame, app_id) in &frames {
            if self.preexisting_frames.contains(frame)
                || self.complete.iter().any(|(done, _)| done == frame)
            {
                continue;
            }
            let Some(core) = unique_core_for_app(
                app_id,
                &frames,
                &cores,
                &self.preexisting_frames,
                &self.preexisting_cores,
            ) else {
                let counts = (
                    frames
                        .iter()
                        .filter(|(window, id)| {
                            id == app_id && !self.preexisting_frames.contains(window)
                        })
                        .count(),
                    cores
                        .iter()
                        .filter(|(window, id)| {
                            id == app_id && !self.preexisting_cores.contains(window)
                        })
                        .count(),
                    cores
                        .iter()
                        .filter(|(window, id)| {
                            id == app_id && self.preexisting_cores.contains(window)
                        })
                        .count(),
                );
                if self.unmatched.insert(*frame, counts) != Some(counts) {
                    eprintln!(
                        "phase=auto-present result=waiting app={app_id:?} frame={:#x} frames={} new_cores={} old_cores={}",
                        frame.hwnd, counts.0, counts.1, counts.2
                    );
                }
                continue;
            };
            self.unmatched.remove(frame);
            // Ambiguous views are left untouched. A slow app has its own
            // pending record and cannot consume another app's retry budget.
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
            let result = Self::present(*frame, core, app_id);
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
            crate::presentation_callbacks::probe_window_discovery(wrapper.interface, core.hwnd)?;
        }
        crate::presentation_callbacks::probe_window_visibility(wrapper.interface, core.hwnd)?;
        crate::presentation_callbacks::probe_window_layout(wrapper.interface, core.hwnd)?;
        uncloak(app_id).map_err(|e| e.to_string())?;
        // SAFETY: Parenting to the correct AFH frame is the observable result of
        // SetPresentedWindow; do not report success based only on an HRESULT.
        Ok(unsafe { GetParent(HWND(core.hwnd as *mut c_void)) }.ok()
            == Some(HWND(frame.hwnd as *mut c_void)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{Window, unique_core_for_app};

    fn window(hwnd: usize) -> Window {
        Window { hwnd, pid: 1 }
    }

    #[test]
    fn distinct_apps_can_each_match_one_core() {
        let frames = [
            (window(1), "Calculator".into()),
            (window(2), "Settings".into()),
        ];
        let cores = [
            (window(3), "Calculator".into()),
            (window(4), "Settings".into()),
        ];
        assert_eq!(
            unique_core_for_app(
                "Calculator",
                &frames,
                &cores,
                &HashSet::new(),
                &HashSet::new()
            ),
            Some(window(3))
        );
        assert_eq!(
            unique_core_for_app(
                "Settings",
                &frames,
                &cores,
                &HashSet::new(),
                &HashSet::new()
            ),
            Some(window(4))
        );
    }

    #[test]
    fn multiple_views_of_one_app_are_ambiguous() {
        let frames = [
            (window(1), "Calculator".into()),
            (window(2), "Calculator".into()),
        ];
        let cores = [(window(3), "Calculator".into())];
        assert_eq!(
            unique_core_for_app(
                "Calculator",
                &frames,
                &cores,
                &HashSet::new(),
                &HashSet::new()
            ),
            None
        );

        let frames = [(window(1), "Calculator".into())];
        let cores = [
            (window(3), "Calculator".into()),
            (window(4), "Calculator".into()),
        ];
        assert_eq!(
            unique_core_for_app(
                "Calculator",
                &frames,
                &cores,
                &HashSet::new(),
                &HashSet::new()
            ),
            None
        );
    }

    #[test]
    fn new_core_wins_over_old_orphan() {
        let frames = [(window(1), "Calculator".into())];
        let cores = [
            (window(2), "Calculator".into()),
            (window(3), "Calculator".into()),
        ];
        assert_eq!(
            unique_core_for_app(
                "Calculator",
                &frames,
                &cores,
                &HashSet::new(),
                &HashSet::from([window(2)])
            ),
            Some(window(3))
        );
    }

    #[test]
    fn one_reused_core_can_be_presented() {
        let frames = [(window(1), "Calculator".into())];
        let cores = [(window(2), "Calculator".into())];
        assert_eq!(
            unique_core_for_app(
                "Calculator",
                &frames,
                &cores,
                &HashSet::new(),
                &HashSet::from([window(2)])
            ),
            Some(window(2))
        );
    }

    #[test]
    fn multiple_old_orphans_remain_ambiguous() {
        let frames = [(window(1), "Calculator".into())];
        let cores = [
            (window(2), "Calculator".into()),
            (window(3), "Calculator".into()),
        ];
        assert_eq!(
            unique_core_for_app(
                "Calculator",
                &frames,
                &cores,
                &HashSet::new(),
                &HashSet::from([window(2), window(3)])
            ),
            None
        );
    }

    #[test]
    fn old_frame_does_not_block_new_frame() {
        let frames = [
            (window(1), "Calculator".into()),
            (window(2), "Calculator".into()),
        ];
        let cores = [(window(3), "Calculator".into())];
        assert_eq!(
            unique_core_for_app(
                "Calculator",
                &frames,
                &cores,
                &HashSet::from([window(1)]),
                &HashSet::new()
            ),
            Some(window(3))
        );
    }
}
