//! Build-specific redirection of the immersive fallback window's desktop band.

use std::{
    ffi::c_void,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

use libloading::Library;

// RVA of twinui.pcshell!_imp_CreateWindowInBand on Windows build 26200,
// verified against this installation's matching Microsoft public PDB.
const CREATE_WINDOW_IN_BAND_IAT_RVA: usize = 0x9839b0;
const CREATE_WINDOW_IN_BAND_DELAY_THUNK_RVA: usize = 0x2cfe46;
const IMMERSIVE_BACKGROUND_BAND: u32 = 12;
const DESKTOP_BAND: u32 = 1;
const PAGE_READWRITE: u32 = 4;

type CreateWindowInBand = unsafe extern "system" fn(
    u32,
    *const u16,
    *const u16,
    u32,
    i32,
    i32,
    i32,
    i32,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    u32,
) -> *mut c_void;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn VirtualProtect(address: *mut c_void, size: usize, protection: u32, old: *mut u32) -> i32;
}

static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static REDIRECTED: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn redirect_create_window_in_band(
    ex_style: u32,
    class: *const u16,
    name: *const u16,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    parent: *mut c_void,
    menu: *mut c_void,
    instance: *mut c_void,
    parameter: *mut c_void,
    band: u32,
) -> *mut c_void {
    let original = ORIGINAL.load(Ordering::Acquire);
    // SAFETY: install() stores the validated user32 export before replacing
    // the IAT slot, so any call through this hook has a valid original target.
    let original: CreateWindowInBand = unsafe { std::mem::transmute(original) };
    let requested_band = if band == IMMERSIVE_BACKGROUND_BAND {
        REDIRECTED.fetch_add(1, Ordering::Relaxed);
        DESKTOP_BAND
    } else {
        band
    };
    // SAFETY: The signature matches user32!CreateWindowInBand. Only the
    // fallback's requested band changes; all other arguments pass through.
    unsafe {
        original(
            ex_style,
            class,
            name,
            style,
            x,
            y,
            width,
            height,
            parent,
            menu,
            instance,
            parameter,
            requested_band,
        )
    }
}

pub struct FallbackBandRedirect {
    // Keep both DLLs loaded while their function pointers are in use.
    _twinui: Library,
    _user32: Library,
    slot: *mut usize,
    original_slot_value: usize,
}

impl FallbackBandRedirect {
    pub fn install() -> Result<Self, String> {
        let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let system32 = PathBuf::from(system_root).join("System32");
        // SAFETY: Both paths are absolute installed Windows DLL paths. The
        // libraries remain owned by the returned guard until after restore.
        let twinui = unsafe { Library::new(system32.join("twinui.pcshell.dll")) }
            .map_err(|error| format!("load-twinui: {error}"))?;
        let user32 = unsafe { Library::new(system32.join("user32.dll")) }
            .map_err(|error| format!("load-user32: {error}"))?;
        // SAFETY: user32 remains loaded in this guard, so this export address
        // stays valid while the IAT is redirected.
        let original = unsafe {
            *user32
                .get::<CreateWindowInBand>(b"CreateWindowInBand\0")
                .map_err(|error| format!("resolve-CreateWindowInBand: {error}"))?
                as usize
        };
        let wide_name: Vec<u16> = "twinui.pcshell.dll".encode_utf16().chain([0]).collect();
        // SAFETY: The DLL is already loaded by Library::new above.
        let base = unsafe { GetModuleHandleW(wide_name.as_ptr()) };
        if base.is_null() {
            return Err("GetModuleHandleW returned null".into());
        }
        // SAFETY: The RVA is taken from the matching PDB for this installed
        // DLL. The slot is checked against the resolved user32 export before
        // any write, so a different build fails without patching.
        let slot = unsafe {
            base.cast::<u8>()
                .add(CREATE_WINDOW_IN_BAND_IAT_RVA)
                .cast::<usize>()
        };
        let current = unsafe { slot.read_volatile() };
        let delay_thunk = base as usize + CREATE_WINDOW_IN_BAND_DELAY_THUNK_RVA;
        if current != original && current != delay_thunk {
            return Err(format!(
                "IAT target differs from user32 export and delay thunk: current={current:#x} export={original:#x} thunk={delay_thunk:#x}"
            ));
        }
        ORIGINAL.store(original, Ordering::Release);
        // SAFETY: This slot belongs to the DLL loaded in the dedicated host
        // process. The original pointer is restored before either DLL unloads.
        unsafe { write_slot(slot, redirect_create_window_in_band as *const () as usize) }?;
        Ok(Self {
            _twinui: twinui,
            _user32: user32,
            slot,
            original_slot_value: current,
        })
    }

    #[cfg(feature = "diagnostics")]
    pub fn redirected_count(&self) -> usize {
        REDIRECTED.load(Ordering::Relaxed)
    }
}

impl Drop for FallbackBandRedirect {
    fn drop(&mut self) {
        // SAFETY: The guard still owns both DLLs and the validated IAT slot.
        if let Err(error) = unsafe { write_slot(self.slot, self.original_slot_value) } {
            eprintln!("phase=restore-fallback-import error={error}");
        }
    }
}

unsafe fn write_slot(slot: *mut usize, value: usize) -> Result<(), String> {
    let mut original_protection = 0_u32;
    // SAFETY: The caller supplies the validated IAT slot for this process.
    if unsafe {
        VirtualProtect(
            slot.cast(),
            size_of::<usize>(),
            PAGE_READWRITE,
            &mut original_protection,
        )
    } == 0
    {
        return Err(format!(
            "VirtualProtect-write failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: The IAT slot is writable for the duration of this store.
    unsafe { slot.write_volatile(value) };
    let mut ignored = 0_u32;
    // SAFETY: Restore the same slot's original page protection immediately.
    if unsafe {
        VirtualProtect(
            slot.cast(),
            size_of::<usize>(),
            original_protection,
            &mut ignored,
        )
    } == 0
    {
        return Err(format!(
            "VirtualProtect-restore failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}
