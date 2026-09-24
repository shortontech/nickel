pub struct Wrapper {
    pub interface: usize,
    pub client: usize,
    pub wait_flags: u32,
}

pub fn find_wrapper(frame: usize) -> Result<Wrapper, Box<dyn std::error::Error>> {
    use std::{ffi::c_void, mem::size_of};
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            Diagnostics::{
                Debug::ReadProcessMemory,
                ToolHelp::{
                    CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW,
                    TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
                },
            },
            Memory::{
                MEM_COMMIT, MEM_PRIVATE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READWRITE,
                PAGE_GUARD, PAGE_READWRITE, PAGE_WRITECOPY, VirtualQueryEx,
            },
            Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
        },
        UI::WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId, IsWindow},
    };

    // These layout constants are from this machine's Windows build 26200
    // symbols and live objects. They are private to the verified Windows build.
    const DISPATCHER_VTABLE_RVA: usize = 0x74c4a0;
    const DISPATCHER_SECOND_VTABLE_RVA: usize = 0x74c510;
    const WRAPPER_COLLECTION_VTABLE_RVA: usize = 0x74d618;
    const WRAPPER_VTABLE_RVA: usize = 0x749168;
    const MAX_SCAN_BYTES: usize = 512 * 1024 * 1024;
    const CHUNK_BYTES: usize = 64 * 1024;

    struct OwnedHandle(HANDLE);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: This guard owns the process or snapshot handle.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    fn read_bytes(process: HANDLE, address: usize, buffer: &mut [u8]) -> usize {
        let mut read = 0;
        // SAFETY: Windows validates the remote address. buffer is writable
        // for its entire length and the process has PROCESS_VM_READ access.
        let _ = unsafe {
            ReadProcessMemory(
                process,
                address as *const c_void,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                Some(&mut read),
            )
        };
        read
    }

    fn read_usize(process: HANDLE, address: usize) -> Option<usize> {
        let mut bytes = [0u8; size_of::<usize>()];
        (read_bytes(process, address, &mut bytes) == bytes.len())
            .then(|| usize::from_le_bytes(bytes))
    }

    // SAFETY: Windows validates both HWND values.
    if !unsafe { IsWindow(Some(windows::Win32::Foundation::HWND(frame as *mut c_void))) }.as_bool()
    {
        return Err(format!("frame HWND {frame:#x} is not live").into());
    }
    let shell = unsafe { GetShellWindow() };
    if shell.is_invalid() {
        return Err("no shell window is registered".into());
    }
    let mut host_pid = 0;
    let _ = unsafe { GetWindowThreadProcessId(shell, Some(&mut host_pid)) };
    if host_pid == 0 {
        return Err("shell window has no process ID".into());
    }
    // SAFETY: The requested rights allow read-only inspection of this user's
    // shell host; Windows checks the PID and access token.
    let process = OwnedHandle(unsafe {
        OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, host_pid)?
    });
    // SAFETY: Snapshot enumeration returns a handle owned by this guard.
    let snapshot = OwnedHandle(unsafe {
        CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, host_pid)?
    });
    let mut module = MODULEENTRY32W {
        dwSize: size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };
    let mut twinui_base = None;
    // SAFETY: module.dwSize describes the writable structure passed to the
    // snapshot functions, and the snapshot remains open for this loop.
    if unsafe { Module32FirstW(snapshot.0, &mut module) }.is_ok() {
        loop {
            let name_end = module
                .szModule
                .iter()
                .position(|&character| character == 0)
                .unwrap_or(module.szModule.len());
            let name = String::from_utf16_lossy(&module.szModule[..name_end]);
            if name.eq_ignore_ascii_case("twinui.pcshell.dll") {
                twinui_base = Some(module.modBaseAddr as usize);
                break;
            }
            if unsafe { Module32NextW(snapshot.0, &mut module) }.is_err() {
                break;
            }
        }
    }
    let module_base = twinui_base.ok_or("twinui.pcshell.dll is not loaded in shell process")?;
    let dispatcher_vtable = module_base + DISPATCHER_VTABLE_RVA;
    let expected_second_vtable = module_base + DISPATCHER_SECOND_VTABLE_RVA;
    let expected_collection_vtable = module_base + WRAPPER_COLLECTION_VTABLE_RVA;
    let expected_wrapper_vtable = module_base + WRAPPER_VTABLE_RVA;
    println!("phase=inspect-host pid={host_pid} twinui={module_base:#x} frame={frame:#x}");

    // Validate cached ownership on every use. Shell restarts and dispatcher
    // replacement must not turn an old address into the active controller.
    let inspect = |dispatcher: usize| {
        if read_usize(process.0, dispatcher) != Some(dispatcher_vtable)
            || read_usize(process.0, dispatcher + 8) != Some(expected_second_vtable)
            || read_usize(process.0, dispatcher + 0xd8) != Some(expected_collection_vtable)
        {
            return None;
        }
        wrapper_in_dispatcher(dispatcher, frame, expected_wrapper_vtable, |address| {
            read_usize(process.0, address)
        })
    };
    if let Some((pid, base, dispatcher)) = LAST_DISPATCHER.get()
        && pid == host_pid
        && base == module_base
        && let Some(wrapper) = inspect(dispatcher)
    {
        return Ok(wrapper);
    }
    let mut cursor = 0usize;
    let mut scanned = 0usize;
    let mut buffer = vec![0u8; CHUNK_BYTES];
    loop {
        let mut info = MEMORY_BASIC_INFORMATION::default();
        // SAFETY: VirtualQueryEx writes a structure of the supplied size;
        // Windows validates each candidate address in the remote process.
        let reported = unsafe {
            VirtualQueryEx(
                process.0,
                Some(cursor as *const c_void),
                &mut info,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if reported == 0 {
            break;
        }
        let base = info.BaseAddress as usize;
        let Some(end) = base.checked_add(info.RegionSize) else {
            break;
        };
        if end <= cursor {
            break;
        }
        cursor = end;
        let writable = [PAGE_READWRITE, PAGE_WRITECOPY, PAGE_EXECUTE_READWRITE]
            .iter()
            .any(|flag| info.Protect.0 & flag.0 != 0);
        if info.State != MEM_COMMIT
            || info.Type != MEM_PRIVATE
            || info.Protect.0 & PAGE_GUARD.0 != 0
            || !writable
        {
            continue;
        }
        let mut address = base;
        while address < end && scanned < MAX_SCAN_BYTES {
            let length = (end - address).min(CHUNK_BYTES);
            let read = read_bytes(process.0, address, &mut buffer[..length]);
            scanned += read;
            for offset in
                (0..read.saturating_sub(size_of::<usize>() - 1)).step_by(size_of::<usize>())
            {
                let value =
                    usize::from_le_bytes(buffer[offset..offset + size_of::<usize>()].try_into()?);
                if value != dispatcher_vtable {
                    continue;
                }
                let candidate = address + offset;
                if let Some(wrapper) = inspect(candidate) {
                    LAST_DISPATCHER.set(Some((host_pid, module_base, candidate)));
                    println!(
                        "phase=wrapper-found dispatcher={candidate:#x} interface={:#x} frame={frame:#x} client={:#x} wait_flags={:#x}",
                        wrapper.interface, wrapper.client, wrapper.wait_flags
                    );
                    return Ok(wrapper);
                }
                // A matching dispatcher vtable is insufficient: old dispatcher
                // objects can coexist with the one that owns a newly launched app.
            }
            address += length;
        }
        if scanned >= MAX_SCAN_BYTES {
            break;
        }
    }
    Err(format!("no UWP wrapper owns frame {frame:#x} after scanning {scanned} bytes").into())
}

thread_local! {
    static LAST_DISPATCHER: std::cell::Cell<Option<(u32, usize, usize)>> = const { std::cell::Cell::new(None) };
}

fn wrapper_in_dispatcher(
    dispatcher: usize,
    frame: usize,
    expected_vtable: usize,
    read: impl Fn(usize) -> Option<usize>,
) -> Option<Wrapper> {
    let begin = read(dispatcher.checked_add(0x180)?)?;
    let end = read(dispatcher.checked_add(0x188)?)?;
    let bytes = end.checked_sub(begin)?;
    if bytes % size_of::<usize>() != 0 || bytes > 1024 * size_of::<usize>() {
        return None;
    }
    for entry in (begin..end).step_by(size_of::<usize>()) {
        let Some(wrapper) = read(entry) else {
            continue;
        };
        if read(wrapper) != Some(expected_vtable) || read(wrapper.checked_add(0x78)?) != Some(frame)
        {
            continue;
        }
        let Some(client) = read(wrapper.checked_add(0x158)?) else {
            continue;
        };
        let Some(flags) = read(wrapper.checked_add(0x160)?) else {
            continue;
        };
        return Some(Wrapper {
            interface: wrapper.checked_add(0x20)?,
            client,
            wait_flags: flags as u32,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn memory() -> HashMap<usize, usize> {
        HashMap::from([
            (0x1180, 0x3000),
            (0x1188, 0x3008),
            (0x3000, 0x4000),
            (0x4000, 0x7777),
            (0x4078, 11),
            (0x4158, 12),
            (0x4160, 0),
            (0x2180, 0x5000),
            (0x2188, 0x5008),
            (0x5000, 0x6000),
            (0x6000, 0x7777),
            (0x6078, 21),
            (0x6158, 0),
            (0x6160, 17),
        ])
    }

    #[test]
    fn older_dispatcher_does_not_hide_new_apps_wrapper() {
        let memory = memory();
        let found = [0x1000, 0x2000]
            .into_iter()
            .find_map(|dispatcher| {
                wrapper_in_dispatcher(dispatcher, 21, 0x7777, |address| {
                    memory.get(&address).copied()
                })
            })
            .unwrap();
        assert_eq!(found.interface, 0x6020);
        assert_eq!(found.client, 0);
        assert_eq!(found.wait_flags, 17);
    }

    #[test]
    fn empty_or_invalid_old_collection_can_be_skipped() {
        for end in [0x3000, 0x2fff, 0x3001, 0x9000] {
            let mut memory = memory();
            memory.insert(0x1188, end);
            assert!(
                wrapper_in_dispatcher(0x1000, 21, 0x7777, |address| memory.get(&address).copied())
                    .is_none()
            );
            assert!(
                wrapper_in_dispatcher(0x2000, 21, 0x7777, |address| memory.get(&address).copied())
                    .is_some()
            );
        }
    }

    #[test]
    fn unreadable_readiness_is_not_treated_as_ready() {
        let mut memory = memory();
        memory.remove(&0x6160);
        assert!(
            wrapper_in_dispatcher(0x2000, 21, 0x7777, |address| memory.get(&address).copied())
                .is_none()
        );
    }
}
