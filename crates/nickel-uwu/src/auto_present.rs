//! Automates the existing build-specific presentation experiment on the shell STA.
use std::{
    collections::{HashMap, HashSet},
    ffi::c_void,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM},
        Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
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
                IsIconic, IsWindow, IsWindowVisible,
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
    next_attempt: Instant,
    stable_since: Option<Instant>,
    core: Option<Window>,
    last_error: String,
    slow_reported: bool,
}

impl Pending {
    fn new(now: Instant) -> Self {
        Self {
            started: now,
            next_attempt: now,
            stable_since: None,
            core: None,
            last_error: String::new(),
            slow_reported: false,
        }
    }

    fn observe(&mut self, now: Instant, ready: bool) -> bool {
        if !ready {
            self.stable_since = None;
            return false;
        }
        now.duration_since(*self.stable_since.get_or_insert(now)) >= Duration::from_secs(1)
    }

    fn retry(&mut self, now: Instant) {
        // Keep recovering slow apps, but stop scanning their host every 250 ms.
        self.next_attempt = now
            + if now.duration_since(self.started) >= Duration::from_secs(15) {
                Duration::from_secs(5)
            } else {
                Duration::from_millis(250)
            };
    }
}

#[derive(Clone)]
struct CoreWindow {
    window: Window,
    app_id: String,
    parent: Option<usize>,
}

trait Presentation {
    fn wrapper(&mut self, frame: Window) -> Result<crate::wrapper_inspect::Wrapper, String>;
    fn present(
        &mut self,
        frame: Window,
        core: Window,
        app_id: &str,
        wrapper: &crate::wrapper_inspect::Wrapper,
        allow_uncloak: bool,
    ) -> Result<bool, String>;
}

struct NativePresentation;

struct Readiness {
    associated: bool,
    wait_flags: u32,
    attached: bool,
    uncloaked: bool,
    visible: bool,
}

impl Readiness {
    fn ready(&self) -> bool {
        self.associated && self.wait_flags == 0 && self.attached && self.uncloaked && self.visible
    }
}

// Explicit ownership takes precedence over app identity. Never move a child
// from another frame, or replace a live wrapper client with a newer orphan.
fn select_core(
    frame: Window,
    app_id: &str,
    client: usize,
    remembered: Option<Window>,
    cores: &[CoreWindow],
) -> Result<Option<Window>, String> {
    let eligible = |core: &&CoreWindow| {
        core.app_id == app_id && (core.parent.is_none() || core.parent == Some(frame.hwnd))
    };
    if client != 0 {
        return cores
            .iter()
            .filter(eligible)
            .find(|core| core.window.hwnd == client)
            .map(|core| Some(core.window))
            .ok_or_else(|| "wrapper client is unavailable or belongs to another frame".into());
    }
    if let Some(core) = remembered {
        return cores
            .iter()
            .filter(eligible)
            .find(|candidate| candidate.window == core)
            .map(|core| Some(core.window))
            .ok_or_else(|| "pending client is unavailable or belongs to another frame".into());
    }
    let mut children = cores
        .iter()
        .filter(eligible)
        .filter(|core| core.parent == Some(frame.hwnd));
    let first = children.next().map(|core| core.window);
    if children.next().is_some() {
        return Err("multiple CoreWindows belong to frame".into());
    }
    Ok(first)
}

fn unique_core_for_app(
    app_id: &str,
    frames: &[(Window, String)],
    cores: &[(Window, String)],
    resolved_frames: &HashSet<Window>,
    preexisting_cores: &HashSet<Window>,
) -> Option<Window> {
    let mut matching_frames = frames
        .iter()
        .filter(|(window, id)| id == app_id && !resolved_frames.contains(window));
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
    preexisting_cores: HashSet<Window>,
    complete: HashSet<(Window, Window)>,
    pending: HashMap<Window, Pending>,
}

impl AutoPresent {
    pub fn new() -> Self {
        Self {
            preexisting_cores: windows()
                .iter()
                .filter(|(_, frame)| !frame)
                .map(|(window, _)| *window)
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
        self.preexisting_cores.retain(|w| live.contains(w));
        let cores: Vec<_> = all
            .iter()
            .filter(|(_, frame)| !frame)
            .filter_map(|(w, _)| {
                process_app_id(w.pid).map(|app_id| CoreWindow {
                    window: *w,
                    app_id,
                    // SAFETY: Windows validates the enumerated HWND.
                    parent: unsafe { GetParent(HWND(w.hwnd as *mut c_void)) }
                        .ok()
                        .map(|p| p.0 as usize),
                })
            })
            .collect();
        let frames: Vec<_> = all
            .iter()
            .filter(|(_, frame)| *frame)
            .filter_map(|(w, _)| frame_app_id(w.hwnd).map(|id| (*w, id)))
            .collect();
        self.reconcile(Instant::now(), &frames, &cores, &mut NativePresentation);
    }

    fn reconcile(
        &mut self,
        now: Instant,
        frames: &[(Window, String)],
        cores: &[CoreWindow],
        platform: &mut impl Presentation,
    ) {
        let unparented: Vec<_> = cores
            .iter()
            .filter(|core| core.parent.is_none())
            .map(|core| (core.window, core.app_id.clone()))
            .collect();
        let resolved: HashSet<_> = frames
            .iter()
            .filter(|(frame, _)| {
                self.complete.iter().any(|(done, _)| done == frame)
                    || cores.iter().any(|core| core.parent == Some(frame.hwnd))
            })
            .map(|(frame, _)| *frame)
            .collect();
        for (frame, app_id) in frames {
            if self.complete.iter().any(|(done, _)| done == frame) {
                continue;
            }
            let pending = self
                .pending
                .entry(*frame)
                .or_insert_with(|| Pending::new(now));
            if now < pending.next_attempt {
                continue;
            }
            if pending
                .core
                .is_some_and(|core| !cores.iter().any(|candidate| candidate.window == core))
            {
                *pending = Pending::new(now);
            }
            let result = (|| {
                let wrapper = platform.wrapper(*frame)?;
                let explicit = select_core(*frame, app_id, wrapper.client, pending.core, cores)?;
                let core = explicit
                    .or_else(|| {
                        unique_core_for_app(
                            app_id,
                            frames,
                            &unparented,
                            &resolved,
                            &self.preexisting_cores,
                        )
                    })
                    .ok_or_else(|| "waiting for an unambiguous CoreWindow".to_string())?;
                if pending.core != Some(core) {
                    pending.core = Some(core);
                    pending.stable_since = None;
                }
                let allow_uncloak = frames.iter().filter(|(_, id)| id == app_id).count() == 1;
                platform.present(*frame, core, app_id, &wrapper, allow_uncloak)
            })();
            let ready = match result {
                Ok(ready) => {
                    pending.last_error.clear();
                    ready
                }
                Err(error) => {
                    if error != pending.last_error {
                        tracing::warn!(app = app_id, frame = frame.hwnd, core = ?pending.core,
                            elapsed_ms = now.duration_since(pending.started).as_millis(), %error,
                            "UWP presentation pending; will retry");
                        eprintln!(
                            "phase=auto-present app={app_id:?} frame={:#x} error={error}",
                            frame.hwnd
                        );
                        pending.last_error = error;
                    }
                    false
                }
            };
            if pending.observe(now, ready) {
                let core = pending.core.expect("ready presentation has a matched core");
                tracing::info!(
                    app = app_id,
                    frame = frame.hwnd,
                    core = core.hwnd,
                    elapsed_ms = now.duration_since(pending.started).as_millis(),
                    "UWP presentation ready"
                );
                println!(
                    "phase=auto-present result=ready app={app_id:?} frame={:#x} core={:#x}",
                    frame.hwnd, core.hwnd
                );
                self.complete.insert((*frame, core));
                self.pending.remove(frame);
            } else {
                if !pending.slow_reported
                    && now.duration_since(pending.started) >= Duration::from_secs(15)
                {
                    tracing::warn!(app = app_id, frame = frame.hwnd, core = ?pending.core,
                        last_error = pending.last_error, "UWP presentation is slow; continuing recovery every five seconds");
                    pending.slow_reported = true;
                }
                pending.retry(now);
            }
        }
    }
}

fn cloaked(window: Window) -> Result<bool, String> {
    let mut flags = 0u32;
    // SAFETY: DWM validates the HWND; flags is a writable DWORD-sized buffer.
    unsafe {
        DwmGetWindowAttribute(
            HWND(window.hwnd as *mut c_void),
            DWMWA_CLOAKED,
            (&mut flags as *mut u32).cast(),
            size_of::<u32>() as u32,
        )
    }
    .map_err(|error| format!("read cloak state: {error}"))?;
    Ok(flags != 0)
}

impl Presentation for NativePresentation {
    fn wrapper(&mut self, frame: Window) -> Result<crate::wrapper_inspect::Wrapper, String> {
        use windows::Win32::UI::WindowsAndMessaging::GetShellWindow;
        let mut pid = 0;
        // SAFETY: Windows validates the shell HWND and writes the PID.
        unsafe { GetWindowThreadProcessId(GetShellWindow(), Some(&mut pid)) };
        if pid != std::process::id() {
            return Err("presentation controller is not owned by this process".into());
        }
        crate::wrapper_inspect::find_wrapper(frame.hwnd).map_err(|e| e.to_string())
    }

    fn present(
        &mut self,
        frame: Window,
        core: Window,
        app_id: &str,
        wrapper: &crate::wrapper_inspect::Wrapper,
        allow_uncloak: bool,
    ) -> Result<bool, String> {
        if wrapper.client != 0 && wrapper.client != core.hwnd {
            return Err("wrapper already owns a different CoreWindow".into());
        }
        let raw = (wrapper.interface - 0x20) as *mut c_void;
        // SAFETY: The finder verified a local wrapper; this runs on its shell
        // STA. Retain its COM reference across reentrant presentation calls.
        let _keep_alive = unsafe { IUnknown::from_raw_borrowed(&raw) }
            .ok_or("null wrapper")?
            .clone();
        let frame_hwnd = HWND(frame.hwnd as *mut c_void);
        let core_hwnd = HWND(core.hwnd as *mut c_void);
        // SAFETY: Windows validates these observed handles. Do not restore a
        // user-minimized frame while waiting for initialization to settle.
        if !unsafe { IsWindow(Some(core_hwnd)) }.as_bool()
            || unsafe { IsIconic(frame_hwnd) }.as_bool()
        {
            return Ok(false);
        }
        for window in [frame, core] {
            let mut pid = 0;
            // SAFETY: Windows validates the HWND and writes the current owner.
            unsafe { GetWindowThreadProcessId(HWND(window.hwnd as *mut c_void), Some(&mut pid)) };
            if pid != window.pid {
                return Err("window owner changed during presentation".into());
            }
        }
        // SAFETY: Recheck parenting immediately before callbacks; another
        // process can attach a CoreWindow after the enumeration snapshot.
        let parent = unsafe { GetParent(core_hwnd) }.ok();
        if parent.is_some_and(|parent| parent != frame_hwnd) {
            return Err("CoreWindow was attached to another frame".into());
        }
        let frame_cloaked = cloaked(frame)?;
        let core_cloaked = cloaked(core)?;
        let attached = parent == Some(frame_hwnd);
        let visible = unsafe { IsWindowVisible(frame_hwnd) }.as_bool()
            && unsafe { IsWindowVisible(core_hwnd) }.as_bool();
        // Observe before invoking callbacks: success must survive later message
        // pump turns, not just the synchronous clearing of a wait flag.
        let readiness = Readiness {
            associated: wrapper.client == core.hwnd,
            wait_flags: wrapper.wait_flags,
            attached,
            uncloaked: !frame_cloaked && !core_cloaked,
            visible,
        };
        if readiness.ready() {
            return Ok(true);
        }
        tracing::debug!(
            app = app_id,
            frame = frame.hwnd,
            core = core.hwnd,
            wait_flags = wrapper.wait_flags,
            attached,
            visible,
            frame_cloaked,
            core_cloaked,
            "UWP presentation state"
        );
        if wrapper.client == 0 {
            crate::presentation_callbacks::probe_window_discovery(wrapper.interface, core.hwnd)?;
        }
        if wrapper.client == 0 || wrapper.wait_flags & 1 != 0 || !attached {
            crate::presentation_callbacks::probe_window_visibility(wrapper.interface, core.hwnd)?;
        }
        if wrapper.client == 0 || wrapper.wait_flags & 2 != 0 || !attached {
            crate::presentation_callbacks::probe_window_layout(wrapper.interface, core.hwnd)?;
        }
        if frame_cloaked || core_cloaked {
            // The verified private lookup is by AUMID. Do not uncloak another
            // view when multiple frames share that identity.
            if !allow_uncloak {
                return Err("cannot uncloak an ambiguous application view".into());
            }
            uncloak(app_id).map_err(|e| format!("uncloak: {e}"))?;
        }
        Ok(false)
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
    fn resolved_frame_does_not_block_new_frame() {
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

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use crate::wrapper_inspect::Wrapper;

    struct FakePresentation {
        client: usize,
        result: Result<bool, String>,
        attempts: Vec<Window>,
    }
    impl Presentation for FakePresentation {
        fn wrapper(&mut self, _: Window) -> Result<Wrapper, String> {
            Ok(Wrapper {
                interface: 0,
                client: self.client,
                wait_flags: 0,
            })
        }
        fn present(
            &mut self,
            _: Window,
            core: Window,
            _: &str,
            _: &Wrapper,
            _: bool,
        ) -> Result<bool, String> {
            self.attempts.push(core);
            self.result.clone()
        }
    }
    fn window(hwnd: usize) -> Window {
        Window { hwnd, pid: 10 }
    }
    fn core(hwnd: usize, parent: Option<usize>) -> CoreWindow {
        CoreWindow {
            window: window(hwnd),
            app_id: "app".into(),
            parent,
        }
    }
    fn host() -> AutoPresent {
        AutoPresent {
            preexisting_cores: HashSet::new(),
            complete: HashSet::new(),
            pending: HashMap::new(),
        }
    }
    fn platform() -> FakePresentation {
        FakePresentation {
            client: 0,
            result: Ok(false),
            attempts: Vec::new(),
        }
    }
    fn tick(
        host: &mut AutoPresent,
        now: Instant,
        cores: &[CoreWindow],
        platform: &mut FakePresentation,
    ) {
        host.reconcile(now, &[(window(1), "app".into())], cores, platform);
    }

    #[test]
    fn attached_core_retries_after_uncloak_failure_and_settles() {
        let now = Instant::now();
        let mut host = host();
        let mut platform = platform();
        platform.result = Err("uncloak failed".into());
        tick(&mut host, now, &[core(2, None)], &mut platform);
        platform.client = 2;
        platform.result = Ok(true);
        tick(
            &mut host,
            now + Duration::from_millis(250),
            &[core(2, Some(1))],
            &mut platform,
        );
        assert!(host.complete.is_empty());
        tick(
            &mut host,
            now + Duration::from_millis(1250),
            &[core(2, Some(1))],
            &mut platform,
        );
        assert!(host.complete.contains(&(window(1), window(2))));
        assert_eq!(platform.attempts, vec![window(2); 3]);
    }

    #[test]
    fn asynchronous_attachment_preserves_pending_pair() {
        let now = Instant::now();
        let mut host = host();
        let mut platform = platform();
        tick(&mut host, now, &[core(2, None)], &mut platform);
        tick(
            &mut host,
            now + Duration::from_millis(250),
            &[core(2, Some(1)), core(3, None)],
            &mut platform,
        );
        assert_eq!(platform.attempts, vec![window(2); 2]);
    }

    #[test]
    fn returning_layout_wait_resets_confirmation_period() {
        let now = Instant::now();
        let mut host = host();
        let mut platform = platform();
        platform.client = 2;
        for (millis, ready) in [(0, true), (250, false), (500, true), (1250, true)] {
            platform.result = Ok(ready);
            tick(
                &mut host,
                now + Duration::from_millis(millis),
                &[core(2, Some(1))],
                &mut platform,
            );
            assert!(host.complete.is_empty());
        }
        tick(
            &mut host,
            now + Duration::from_millis(1500),
            &[core(2, Some(1))],
            &mut platform,
        );
        assert!(host.complete.contains(&(window(1), window(2))));
    }

    #[test]
    fn slow_launch_recovers_after_fifteen_seconds_with_backoff() {
        let now = Instant::now();
        let mut host = host();
        let mut platform = platform();
        tick(&mut host, now, &[core(2, None)], &mut platform);
        tick(
            &mut host,
            now + Duration::from_secs(16),
            &[core(2, None)],
            &mut platform,
        );
        tick(
            &mut host,
            now + Duration::from_secs(17),
            &[core(2, None)],
            &mut platform,
        );
        assert_eq!(platform.attempts.len(), 2);
        platform.client = 2;
        platform.result = Ok(true);
        tick(
            &mut host,
            now + Duration::from_secs(21),
            &[core(2, Some(1))],
            &mut platform,
        );
        tick(
            &mut host,
            now + Duration::from_secs(26),
            &[core(2, Some(1))],
            &mut platform,
        );
        assert!(host.complete.contains(&(window(1), window(2))));
    }

    #[test]
    fn host_restart_reconciles_existing_frame_and_core() {
        let now = Instant::now();
        let mut host = host();
        host.preexisting_cores.insert(window(2));
        let mut platform = platform();
        tick(&mut host, now, &[core(2, Some(1))], &mut platform);
        assert_eq!(platform.attempts, vec![window(2)]);
    }

    #[test]
    fn wrapper_identity_wins_over_new_orphan() {
        assert_eq!(
            select_core(window(1), "app", 2, None, &[core(2, None), core(3, None)]).unwrap(),
            Some(window(2))
        );
    }

    #[test]
    fn another_frames_child_is_never_selected() {
        assert!(select_core(window(1), "app", 2, None, &[core(2, Some(3))]).is_err());
        assert!(select_core(window(1), "app", 0, Some(window(2)), &[core(2, Some(3))]).is_err());
    }

    #[test]
    fn completed_pairs_receive_no_more_presentation_calls() {
        let now = Instant::now();
        let mut host = host();
        host.complete.insert((window(1), window(2)));
        let mut platform = platform();
        tick(&mut host, now, &[core(2, Some(1))], &mut platform);
        assert!(platform.attempts.is_empty());
    }
    #[test]
    fn parenting_alone_does_not_establish_readiness() {
        let ready = || Readiness {
            associated: true,
            wait_flags: 0,
            attached: true,
            uncloaked: true,
            visible: true,
        };
        assert!(ready().ready());
        assert!(
            !Readiness {
                associated: false,
                ..ready()
            }
            .ready()
        );
        assert!(
            !Readiness {
                wait_flags: 1,
                ..ready()
            }
            .ready()
        );
        assert!(
            !Readiness {
                wait_flags: 2,
                ..ready()
            }
            .ready()
        );
        assert!(
            !Readiness {
                attached: false,
                ..ready()
            }
            .ready()
        );
        assert!(
            !Readiness {
                uncloaked: false,
                ..ready()
            }
            .ready()
        );
        assert!(
            !Readiness {
                visible: false,
                ..ready()
            }
            .ready()
        );
    }

    #[test]
    fn unattached_existing_core_can_be_recovered_after_restart() {
        let now = Instant::now();
        let mut host = host();
        host.preexisting_cores.insert(window(2));
        let mut platform = platform();
        tick(&mut host, now, &[core(2, None)], &mut platform);
        assert_eq!(platform.attempts, vec![window(2)]);
    }

    #[test]
    fn replacement_core_starts_a_new_confirmation_period() {
        let now = Instant::now();
        let mut host = host();
        let mut platform = platform();
        platform.result = Ok(true);
        tick(&mut host, now, &[core(2, None)], &mut platform);
        tick(
            &mut host,
            now + Duration::from_secs(1),
            &[core(3, None)],
            &mut platform,
        );
        assert!(host.complete.is_empty());
        assert_eq!(platform.attempts, vec![window(2), window(3)]);
        tick(
            &mut host,
            now + Duration::from_secs(2),
            &[core(3, None)],
            &mut platform,
        );
        assert!(host.complete.contains(&(window(1), window(3))));
    }
}
