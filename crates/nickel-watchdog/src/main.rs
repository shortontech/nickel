#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("nickel-watchdog is only supported on Windows");
    std::process::exit(1);
}

#[cfg(target_os = "windows")]
fn main() {
    if let Err(error) = windows_watchdog::run() {
        eprintln!("nickel-watchdog: {error}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "windows")]
mod windows_watchdog {
    use std::collections::{HashMap, HashSet};
    use std::env;
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::os::windows::io::AsRawHandle;
    use std::path::{Path, PathBuf};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WAIT_TIMEOUT, WPARAM};
    use windows::Win32::System::Diagnostics::Debug::{
        MINIDUMP_TYPE, MiniDumpWithHandleData, MiniDumpWithIndirectlyReferencedMemory,
        MiniDumpWithThreadInfo, MiniDumpWithUnloadedModules, MiniDumpWriteDump,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_ACCESS_RIGHTS, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
        WaitForSingleObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, SEND_MESSAGE_TIMEOUT_FLAGS,
        SMTO_ABORTIFHUNG, SMTO_BLOCK, SendMessageTimeoutW, WM_NULL,
    };
    use windows::core::BOOL;

    const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_millis(750);
    const POLL_INTERVAL: Duration = Duration::from_millis(250);
    const DEFAULT_PROCESS_NAME: &str = "nickel.exe";
    struct ProcessHandle(HANDLE);

    impl Drop for ProcessHandle {
        fn drop(&mut self) {
            // SAFETY: this handle was returned by OpenProcess and is owned here.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    #[derive(Debug)]
    struct Options {
        process_name: String,
        output: PathBuf,
        probe_timeout: Duration,
    }

    struct WatchedProcess {
        handle: ProcessHandle,
        captured_this_stall: bool,
    }

    pub fn run() -> Result<(), String> {
        let options = parse_options(env::args_os().skip(1))?;
        fs::create_dir_all(&options.output).map_err(|error| {
            format!(
                "could not create dump directory {}: {error}",
                options.output.display()
            )
        })?;
        let mut log = WatchdogLog::open(&options.output)?;
        log.line(&format!(
            "waiting for process={} timeout_ms={} dump_directory={}",
            options.process_name,
            options.probe_timeout.as_millis(),
            options.output.display()
        ));

        let mut watched = HashMap::<u32, WatchedProcess>::new();
        loop {
            let discovered = matching_processes(&options.process_name)?;
            for pid in discovered.iter().copied() {
                if let std::collections::hash_map::Entry::Vacant(entry) = watched.entry(pid) {
                    match open_process(pid) {
                        Ok(handle) => {
                            log.line(&format!(
                                "watching process={} pid={pid}",
                                options.process_name
                            ));
                            entry.insert(WatchedProcess {
                                handle,
                                captured_this_stall: false,
                            });
                        }
                        Err(error) => log.line(&format!("could not watch pid={pid}: {error}")),
                    }
                }
            }

            watched.retain(|pid, process| {
                // A zero-time wait is non-blocking and distinguishes an exited process from a
                // later process that happens to reuse the same PID.
                // SAFETY: `process.handle` remains valid for the duration of this call.
                let status = unsafe { WaitForSingleObject(process.handle.0, 0) };
                if status != WAIT_TIMEOUT || !discovered.contains(pid) {
                    log.line(&format!(
                        "target pid={pid} exited (wait status={})",
                        status.0
                    ));
                    return false;
                }

                let windows = match process_windows(*pid) {
                    Ok(windows) => windows,
                    Err(error) => {
                        log.line(&format!("could not enumerate pid={pid} windows: {error}"));
                        return true;
                    }
                };
                let stalled = windows
                    .iter()
                    .copied()
                    .find(|window| !responds_to_probe(*window, options.probe_timeout));
                match (stalled, process.captured_this_stall) {
                    (Some(window), false) => {
                        let path = dump_path(&options.output, *pid);
                        log.line(&format!(
                            "unresponsive pid={pid} window={:#x}; capturing {}",
                            window.0 as usize,
                            path.display()
                        ));
                        match write_dump(process.handle.0, *pid, &path) {
                            Ok(()) => log.line(&format!("captured {}", path.display())),
                            Err(error) => log.line(&format!("capture failed: {error}")),
                        }
                        process.captured_this_stall = true;
                    }
                    (None, true) => {
                        log.line(&format!("target pid={pid} window recovered"));
                        process.captured_this_stall = false;
                    }
                    _ => {}
                }
                true
            });
            thread::sleep(POLL_INTERVAL);
        }
    }

    fn parse_options(
        arguments: impl Iterator<Item = std::ffi::OsString>,
    ) -> Result<Options, String> {
        let mut process_name = DEFAULT_PROCESS_NAME.to_owned();
        let mut output = default_output_directory()?;
        let mut probe_timeout = DEFAULT_PROBE_TIMEOUT;
        let mut arguments = arguments.peekable();
        while let Some(argument) = arguments.next() {
            let argument = argument.to_string_lossy();
            match argument.as_ref() {
                "--process-name" => {
                    process_name = arguments
                        .next()
                        .ok_or("--process-name requires a value")?
                        .to_string_lossy()
                        .into_owned();
                    if process_name.is_empty() {
                        return Err("--process-name must not be empty".into());
                    }
                }
                "--output" => {
                    output = PathBuf::from(arguments.next().ok_or("--output requires a path")?);
                }
                "--timeout-ms" => {
                    let value = arguments.next().ok_or("--timeout-ms requires a value")?;
                    let millis = value
                        .to_string_lossy()
                        .parse::<u64>()
                        .map_err(|_| "--timeout-ms must be an integer")?;
                    if !(100..=30_000).contains(&millis) {
                        return Err("--timeout-ms must be between 100 and 30000".into());
                    }
                    probe_timeout = Duration::from_millis(millis);
                }
                "--help" | "-h" => {
                    println!(
                        "Usage: nickel-watchdog [--process-name <EXE>] [--output <DIR>] [--timeout-ms <MS>]"
                    );
                    std::process::exit(0);
                }
                _ => return Err(format!("unknown argument: {argument}")),
            }
        }
        Ok(Options {
            process_name,
            output,
            probe_timeout,
        })
    }

    fn matching_processes(process_name: &str) -> Result<HashSet<u32>, String> {
        // SAFETY: the snapshot handle is owned by ProcessHandle and closed after enumeration.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map(ProcessHandle)
            .map_err(|error| format!("could not enumerate processes: {error}"))?;
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut matches = HashSet::new();
        // SAFETY: `entry` has the required size and remains writable throughout enumeration.
        if unsafe { Process32FirstW(snapshot.0, &mut entry) }.is_ok() {
            loop {
                let length = entry
                    .szExeFile
                    .iter()
                    .position(|character| *character == 0)
                    .unwrap_or(entry.szExeFile.len());
                let executable = String::from_utf16_lossy(&entry.szExeFile[..length]);
                if executable.eq_ignore_ascii_case(process_name) {
                    matches.insert(entry.th32ProcessID);
                }
                // SAFETY: the snapshot and initialized entry remain valid.
                if unsafe { Process32NextW(snapshot.0, &mut entry) }.is_err() {
                    break;
                }
            }
        }
        Ok(matches)
    }

    fn default_output_directory() -> Result<PathBuf, String> {
        env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("Nickel").join("diagnostics").join("hangs"))
            .ok_or("LOCALAPPDATA is not set".into())
    }

    fn open_process(pid: u32) -> Result<ProcessHandle, String> {
        // SAFETY: no pointer arguments; the returned owned handle is closed by ProcessHandle.
        unsafe {
            OpenProcess(
                PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_ACCESS_RIGHTS(0x0010_0000),
                false,
                pid,
            )
        }
        .map(ProcessHandle)
        .map_err(|error| format!("could not open pid {pid}: {error}"))
    }

    fn process_windows(pid: u32) -> Result<Vec<HWND>, String> {
        struct Search {
            pid: u32,
            windows: Vec<HWND>,
        }
        unsafe extern "system" fn visit(window: HWND, state: LPARAM) -> BOOL {
            // SAFETY: EnumWindows invokes this synchronously with the Search pointer below.
            let search = unsafe { &mut *(state.0 as *mut Search) };
            let mut owner = 0;
            // SAFETY: `owner` is writable and HWND values come from EnumWindows.
            unsafe { GetWindowThreadProcessId(window, Some(&mut owner)) };
            // Probe visible top-level windows only; hidden utility windows need not pump UI input.
            if owner == search.pid && unsafe { IsWindowVisible(window).as_bool() } {
                search.windows.push(window);
            }
            true.into()
        }
        let mut search = Search {
            pid,
            windows: Vec::new(),
        };
        // SAFETY: callback and state pointer remain valid for this synchronous enumeration.
        unsafe { EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize)) }
            .map_err(|error| format!("could not enumerate target windows: {error}"))?;
        Ok(search.windows)
    }

    fn responds_to_probe(window: HWND, timeout: Duration) -> bool {
        let timeout = timeout.as_millis().min(u32::MAX as u128) as u32;
        // SAFETY: WM_NULL carries no pointers or process-owned payload. SMTO_BLOCK prevents this
        // watchdog thread from dispatching unrelated messages while the bounded probe is active.
        unsafe {
            SendMessageTimeoutW(
                window,
                WM_NULL,
                WPARAM::default(),
                LPARAM::default(),
                SEND_MESSAGE_TIMEOUT_FLAGS(SMTO_ABORTIFHUNG.0 | SMTO_BLOCK.0),
                timeout,
                None,
            )
            .0 != 0
        }
    }

    fn write_dump(process: HANDLE, pid: u32, path: &Path) -> Result<(), String> {
        let file = File::create(path)
            .map_err(|error| format!("could not create {}: {error}", path.display()))?;
        let file_handle = HANDLE(file.as_raw_handle());
        let dump_type = MINIDUMP_TYPE(
            MiniDumpWithThreadInfo.0
                | MiniDumpWithHandleData.0
                | MiniDumpWithUnloadedModules.0
                | MiniDumpWithIndirectlyReferencedMemory.0,
        );
        // SAFETY: both handles remain open for the call; optional exception/user callbacks are absent.
        let result =
            unsafe { MiniDumpWriteDump(process, pid, file_handle, dump_type, None, None, None) };
        if let Err(error) = result {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(format!("MiniDumpWriteDump failed: {error}"));
        }
        file.sync_all()
            .map_err(|error| format!("could not flush {}: {error}", path.display()))
    }

    fn dump_path(directory: &Path, pid: u32) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        directory.join(format!("nickel-hang-{pid}-{timestamp}.dmp"))
    }

    struct WatchdogLog(File);

    impl WatchdogLog {
        fn open(directory: &Path) -> Result<Self, String> {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(directory.join("watchdog.log"))
                .map(Self)
                .map_err(|error| format!("could not open watchdog log: {error}"))
        }

        fn line(&mut self, message: &str) {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let _ = writeln!(self.0, "{timestamp} {message}");
            let _ = self.0.flush();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn defaults_to_nickel_and_parses_optional_arguments() {
            let options = parse_options(
                ["--output", "dumps", "--timeout-ms", "1250"]
                    .into_iter()
                    .map(Into::into),
            )
            .unwrap();
            assert_eq!(options.process_name, "nickel.exe");
            assert_eq!(options.output, PathBuf::from("dumps"));
            assert_eq!(options.probe_timeout, Duration::from_millis(1250));
        }

        #[test]
        fn parses_process_name() {
            let options = parse_options(
                ["--process-name", "Nickel-preview.exe"]
                    .into_iter()
                    .map(Into::into),
            )
            .unwrap();
            assert_eq!(options.process_name, "Nickel-preview.exe");
        }

        #[test]
        fn rejects_out_of_range_timeout() {
            let result = parse_options(["--timeout-ms", "99"].into_iter().map(Into::into));
            assert!(result.is_err());
        }
    }
}
