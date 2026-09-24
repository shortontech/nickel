pub struct Wrapper {
    pub interface: usize,
    pub client: usize,
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

    let mut cursor = 0usize;
    let mut scanned = 0usize;
    let mut dispatcher = None;
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
                if read_usize(process.0, candidate + 8) == Some(expected_second_vtable)
                    && read_usize(process.0, candidate + 0xd8) == Some(expected_collection_vtable)
                {
                    dispatcher = Some(candidate);
                    break;
                }
            }
            if dispatcher.is_some() {
                break;
            }
            address += length;
        }
        if dispatcher.is_some() || scanned >= MAX_SCAN_BYTES {
            break;
        }
    }
    let dispatcher = dispatcher.ok_or("active UWP window dispatcher not found")?;
    let begin =
        read_usize(process.0, dispatcher + 0x180).ok_or("wrapper array start unreadable")?;
    let end = read_usize(process.0, dispatcher + 0x188).ok_or("wrapper array end unreadable")?;
    if end < begin
        || (end - begin) % size_of::<usize>() != 0
        || end - begin > 1024 * size_of::<usize>()
    {
        return Err("wrapper array has an unexpected size".into());
    }
    println!(
        "phase=dispatcher address={dispatcher:#x} wrappers={}",
        (end - begin) / size_of::<usize>()
    );
    for entry in (begin..end).step_by(size_of::<usize>()) {
        let Some(wrapper) = read_usize(process.0, entry) else {
            continue;
        };
        if read_usize(process.0, wrapper) != Some(expected_wrapper_vtable) {
            continue;
        }
        let wrapper_frame = read_usize(process.0, wrapper + 0x78).unwrap_or(0);
        if wrapper_frame != frame {
            continue;
        }
        let client = read_usize(process.0, wrapper + 0x158).unwrap_or(0);
        let flags = read_usize(process.0, wrapper + 0x160).unwrap_or(0) as u32;
        println!(
            "phase=wrapper-found interface={:#x} frame={frame:#x} client={client:#x} wait_flags={flags:#x}",
            wrapper + 0x20
        );
        return Ok(Wrapper {
            interface: wrapper + 0x20,
            client,
        });
    }
    Err(format!("no UWP wrapper owns frame {frame:#x}").into())
}
