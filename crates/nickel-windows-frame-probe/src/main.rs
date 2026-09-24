use std::process::ExitCode;

#[cfg(target_os = "windows")]
mod fallback_band1;
#[cfg(target_os = "windows")]
mod frame_service_direct;

#[cfg(not(target_os = "windows"))]
fn main() -> ExitCode {
    eprintln!("nickel-windows-frame-probe supports Windows only");
    ExitCode::FAILURE
}

#[cfg(target_os = "windows")]
fn main() -> ExitCode {
    use std::{
        process::Command,
        thread,
        time::{Duration, Instant},
    };

    if std::env::args().nth(1).as_deref() == Some("--connect-shell") {
        return inspect_immersive_shell();
    }
    if std::env::args().nth(1).as_deref() == Some("--check-immersive-factory") {
        return inspect_immersive_factory();
    }
    if std::env::args().nth(1).as_deref() == Some("--inspect-broker-host") {
        return inspect_broker_host();
    }
    if let Some(mode @ ("--host" | "--host-pump")) = std::env::args().nth(1).as_deref() {
        return run_host(mode == "--host-pump");
    }

    let mut args = std::env::args().skip(1);
    let worker = args.next().as_deref() == Some("--worker");
    let app_id = if worker {
        args.next()
    } else {
        std::env::args().nth(1)
    };
    let Some(app_id) = app_id else {
        eprintln!("usage: nickel-windows-frame-probe AUMID");
        return ExitCode::FAILURE;
    };
    let (
        private_service,
        shell_window,
        fallback_band1,
        frame_service_probe,
        controller_start,
        scenario1,
        skip_window_events,
    ) = match (args.next().as_deref(), args.next()) {
        (None, None) => (false, false, false, false, false, false, false),
        (Some("--private-service"), None) => (true, false, false, false, false, false, false),
        (Some("--shell-window"), None) => (true, true, false, false, false, false, false),
        (Some("--shell-window-fallback-band1"), None) => {
            (true, true, true, false, false, false, false)
        }
        (Some("--frame-service-direct"), None) => (true, true, true, true, false, false, false),
        (Some("--controller-start"), None) => (false, true, false, false, true, false, false),
        (Some("--controller-start-band1"), None) => (false, true, true, false, true, false, false),
        (Some("--controller-start-band1-scenario1"), None) => {
            (false, true, true, false, true, true, false)
        }
        (Some("--controller-start-band1-skip-window-events"), None) => {
            (false, true, true, false, true, false, true)
        }
        _ => {
            eprintln!(
                "usage: nickel-windows-frame-probe AUMID [--private-service|--shell-window|--shell-window-fallback-band1|--frame-service-direct|--controller-start|--controller-start-band1|--controller-start-band1-scenario1|--controller-start-band1-skip-window-events]"
            );
            return ExitCode::FAILURE;
        }
    };
    if worker {
        return run_with_immersive_manager(
            &app_id,
            private_service,
            shell_window,
            fallback_band1,
            frame_service_probe,
            controller_start,
            scenario1,
            skip_window_events,
        );
    }

    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            eprintln!("phase=self-path error={error}");
            return ExitCode::FAILURE;
        }
    };
    let mut command = Command::new(exe);
    command.args(["--worker", &app_id]);
    if frame_service_probe {
        command.arg("--frame-service-direct");
    } else if controller_start && skip_window_events {
        command.arg("--controller-start-band1-skip-window-events");
    } else if controller_start && scenario1 {
        command.arg("--controller-start-band1-scenario1");
    } else if controller_start && fallback_band1 {
        command.arg("--controller-start-band1");
    } else if controller_start {
        command.arg("--controller-start");
    } else if fallback_band1 {
        command.arg("--shell-window-fallback-band1");
    } else if shell_window {
        command.arg("--shell-window");
    } else if private_service {
        command.arg("--private-service");
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            eprintln!("phase=spawn-worker error={error}");
            return ExitCode::FAILURE;
        }
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return ExitCode::from(u8::from(!status.success())),
            Ok(None) if started.elapsed() < Duration::from_secs(30) => {
                thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                eprintln!("phase=frame-worker result=timeout elapsed_seconds=30");
                return ExitCode::FAILURE;
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                eprintln!("phase=frame-worker error={error}");
                return ExitCode::FAILURE;
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn inspect_broker_host() -> ExitCode {
    use windows::{
        Win32::System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        core::{GUID, IUnknown, Interface},
    };
    const CANDIDATES: [GUID; 2] = [
        GUID::from_u128(0x23650f94_13b8_4f39_b2c3_817e6564a756),
        GUID::from_u128(0x4075b76f_fcdc_43b6_b0b9_5a005b38b335),
    ];
    // SAFETY: This thread balances COM initialization below.
    if let Err(error) = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok() {
        eprintln!("phase=CoInitializeEx hresult={:?}", error.code());
        return ExitCode::FAILURE;
    }
    for clsid in CANDIDATES {
        // SAFETY: CoCreateInstance returns a COM-owned interface reference.
        match unsafe { CoCreateInstance::<_, IUnknown>(&clsid, None, CLSCTX_INPROC_SERVER) } {
            Ok(object) => {
                // SAFETY: A live COM interface begins with its vtable pointer.
                let vtable = unsafe { object.as_raw().cast::<usize>().read() };
                println!("phase=broker-host-candidate clsid={clsid:?} vtable={vtable:#x}");
            }
            Err(error) => println!(
                "phase=broker-host-candidate clsid={clsid:?} hresult={:?} error={error}",
                error.code()
            ),
        }
    }
    // SAFETY: Balances CoInitializeEx above.
    unsafe { CoUninitialize() };
    ExitCode::SUCCESS
}

#[cfg(target_os = "windows")]
fn inspect_immersive_factory() -> ExitCode {
    use std::{ffi::c_void, path::PathBuf};
    use windows::{
        Win32::System::Com::IClassFactory,
        core::{GUID, HRESULT, Interface},
    };

    type DllGetClassObject =
        unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
    const IMMERSIVE_SHELL: GUID = GUID::from_u128(0xc2f03a33_21f5_47fa_b4bb_156362a2f239);
    let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let system32 = PathBuf::from(system_root).join("System32");
    let mut found = false;
    for name in [
        "twinui.pcshell.dll",
        "twinui.dll",
        "Windows.ImmersiveShell.ServiceProvider.dll",
        "Windows.UI.Immersive.dll",
        "CoreShell.dll",
        "ExplorerFrame.dll",
        "shell32.dll",
        "Windows.Internal.Shell.Broker.dll",
    ] {
        // SAFETY: This is an installed system DLL, retained during the call.
        let library = match unsafe { libloading::Library::new(system32.join(name)) } {
            Ok(library) => library,
            Err(error) => {
                eprintln!("phase=load-twinui module={name} error={error}");
                continue;
            }
        };
        // SAFETY: The exported function has the documented COM entrypoint ABI.
        let entry = match unsafe { library.get::<DllGetClassObject>(b"DllGetClassObject\0") } {
            Ok(entry) => entry,
            Err(error) => {
                eprintln!("phase=DllGetClassObject-export module={name} error={error}");
                continue;
            }
        };
        let mut raw = std::ptr::null_mut();
        // SAFETY: The output is writable; success transfers a COM reference.
        let result = unsafe { entry(&IMMERSIVE_SHELL, &IClassFactory::IID, &mut raw) };
        println!("phase=ImmersiveShell-DllGetClassObject module={name} hresult={result:?}");
        if result.is_ok() {
            // SAFETY: DllGetClassObject returned one owned factory reference.
            drop(unsafe { IClassFactory::from_raw(raw) });
            found = true;
        }
    }
    ExitCode::from(u8::from(!found))
}

#[cfg(target_os = "windows")]
fn inspect_immersive_shell() -> ExitCode {
    use windows::{
        Win32::System::Com::{
            CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, IServiceProvider,
        },
        core::{GUID, IUnknown},
    };

    const IMMERSIVE_SHELL: GUID = GUID::from_u128(0xc2f03a33_21f5_47fa_b4bb_156362a2f239);
    const IMMERSIVE_SHELL_BROKER: GUID = GUID::from_u128(0x228826af_02e1_4226_a9e0_99a855e455a6);
    const IMMERSIVE_APPLICATION_MANAGER: GUID =
        GUID::from_u128(0x50fdbb99_5c92_495e_9e81_e2c2f48cddae);
    const FRAME_SERVICE_INTERFACE: GUID = GUID::from_u128(0x88b25c81_171b_48b0_91d6_c75846bcf035);

    // SAFETY: COM is balanced on this thread after all interfaces are dropped.
    if let Err(error) = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok() {
        eprintln!("phase=CoInitializeEx hresult={:?}", error.code());
        return ExitCode::FAILURE;
    }
    let result = (|| {
        // SAFETY: COM creates and owns the local-server interface reference.
        let shell: IServiceProvider =
            unsafe { CoCreateInstance(&IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER) }
                .map_err(|error| format!("ImmersiveShell: {error}"))?;
        println!("phase=ImmersiveShell result=created");
        for (name, service_id) in [
            ("ImmersiveApplicationManager", IMMERSIVE_APPLICATION_MANAGER),
            ("ApplicationFrameService", FRAME_SERVICE_INTERFACE),
        ] {
            // SAFETY: QueryService returns an owned COM reference on success.
            match unsafe { shell.QueryService::<IUnknown>(&service_id) } {
                Ok(_) => println!("phase=QueryService-{name} result=success"),
                Err(error) => println!(
                    "phase=QueryService-{name} hresult={:?} error={error}",
                    error.code()
                ),
            }
        }
        // SAFETY: This registered local COM class publishes the shell broker.
        match unsafe {
            CoCreateInstance::<_, IServiceProvider>(
                &IMMERSIVE_SHELL_BROKER,
                None,
                CLSCTX_LOCAL_SERVER,
            )
        } {
            Ok(_) => println!("phase=ImmersiveShellBroker result=created"),
            Err(error) => println!(
                "phase=ImmersiveShellBroker hresult={:?} error={error}",
                error.code()
            ),
        }
        Ok::<(), String>(())
    })();
    // SAFETY: Balances successful CoInitializeEx above.
    unsafe { CoUninitialize() };
    if let Err(error) = result {
        eprintln!("phase=connect-shell error={error}");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(target_os = "windows")]
fn configure_dpi_awareness() -> Result<(), String> {
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };

    // SAFETY: Callers invoke this before creating any HWND in this process.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
        .map_err(|error| error.to_string())?;
    println!("phase=SetProcessDpiAwarenessContext result=per-monitor-v2");
    Ok(())
}

#[cfg(target_os = "windows")]
fn run_host(pump_messages: bool) -> ExitCode {
    use std::io::Write;
    use windows::Win32::{
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, MSG, TranslateMessage},
    };

    if let Err(error) = configure_dpi_awareness() {
        eprintln!("phase=host-dpi error={error}");
        return ExitCode::FAILURE;
    }
    let Some(shell_window) = ShellWindowGuard::register(pump_messages) else {
        return ExitCode::FAILURE;
    };
    // SAFETY: This thread balances successful COM initialization below.
    if let Err(error) = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok() {
        eprintln!("phase=host-com error={error}");
        return ExitCode::FAILURE;
    }
    let redirect = match fallback_band1::FallbackBandRedirect::install() {
        Ok(redirect) => redirect,
        Err(error) => {
            eprintln!("phase=host-band-redirect error={error}");
            unsafe { CoUninitialize() };
            return ExitCode::FAILURE;
        }
    };
    let controller = match start_immersive_shell_controller(false, false, true) {
        Ok(controller) => controller,
        Err(error) => {
            eprintln!("phase=host-controller error={error}");
            drop(redirect);
            unsafe { CoUninitialize() };
            return ExitCode::FAILURE;
        }
    };
    let shell_hook_service = if pump_messages {
        match connect_shell_hook_service() {
            Ok(service) => Some(service),
            Err(error) => {
                eprintln!("phase=host-shell-hook-service error={error}");
                drop(controller);
                drop(redirect);
                unsafe { CoUninitialize() };
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    println!("phase=host-ready");
    let _ = std::io::stdout().flush();
    let _keep_alive = (controller, redirect);
    if pump_messages {
        println!("phase=host-message-loop");
        let mut message = MSG::default();
        // SAFETY: The shell window belongs to this thread, and the message
        // structure remains valid through each dispatch.
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
            if message.hwnd == shell_window.window && message.message == 0x8070 {
                // Forward only the documented top-level window lifecycle
                // events while probing. Forwarding HSHELL_REDRAW (6) caused
                // the hook service to request another redraw indefinitely.
                if matches!(message.wParam.0, 1 | 2) {
                    println!(
                        "phase=shell-hook-event code={} hwnd={:#x}",
                        message.wParam.0, message.lParam.0
                    );
                    if let Some(service) = shell_hook_service.as_ref() {
                        match forward_shell_hook(service, message.wParam.0, message.lParam.0) {
                            Ok(()) => println!("phase=shell-hook-forward result=success"),
                            Err(error) => println!("phase=shell-hook-forward error={error}"),
                        }
                    }
                }
                let _ = std::io::stdout().flush();
                continue;
            }
            if message.hwnd == shell_window.window && message.message == 0x806c {
                match switch_settings_view_in_host() {
                    Ok(()) => println!("phase=host-switch-settings result=success"),
                    Err(error) => println!("phase=host-switch-settings error={error}"),
                }
                let _ = std::io::stdout().flush();
                continue;
            }
            if message.hwnd == shell_window.window && message.message == 0x806d {
                let wrapper = message.wParam.0;
                let hwnd = message.lParam.0 as usize;
                match probe_window_discovery(wrapper, hwnd) {
                    Ok(()) => println!(
                        "phase=host-window-discovery wrapper={wrapper:#x} hwnd={hwnd:#x} result=called"
                    ),
                    Err(error) => println!("phase=host-window-discovery error={error}"),
                }
                let _ = std::io::stdout().flush();
                continue;
            }
            if message.hwnd == shell_window.window && message.message == 0x806e {
                let wrapper = message.wParam.0;
                let hwnd = message.lParam.0 as usize;
                match probe_window_visibility(wrapper, hwnd) {
                    Ok(()) => println!(
                        "phase=host-window-visible wrapper={wrapper:#x} hwnd={hwnd:#x} result=called"
                    ),
                    Err(error) => println!("phase=host-window-visible error={error}"),
                }
                let _ = std::io::stdout().flush();
                continue;
            }
            if message.hwnd == shell_window.window && message.message == 0x806f {
                let wrapper = message.wParam.0;
                let hwnd = message.lParam.0 as usize;
                match probe_window_layout(wrapper, hwnd) {
                    Ok(()) => println!(
                        "phase=host-window-layout wrapper={wrapper:#x} hwnd={hwnd:#x} result=called"
                    ),
                    Err(error) => println!("phase=host-window-layout error={error}"),
                }
                let _ = std::io::stdout().flush();
                continue;
            }
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    } else {
        // The one-shot probe kept this thread blocked during activation.
        loop {
            std::thread::park();
        }
    }
    drop(shell_hook_service);
    unsafe { CoUninitialize() };
    ExitCode::SUCCESS
}

#[cfg(target_os = "windows")]
fn connect_shell_hook_service() -> windows::core::Result<windows::core::IUnknown> {
    use std::ffi::c_void;
    use windows::{
        Win32::System::Com::{CLSCTX_LOCAL_SERVER, CoCreateInstance, IServiceProvider},
        core::{GUID, HRESULT, IUnknown, Interface},
    };

    const IMMERSIVE_SHELL: GUID = GUID::from_u128(0xc2f03a33_21f5_47fa_b4bb_156362a2f239);
    const SHELL_HOOK_SERVICE: GUID = GUID::from_u128(0x4624bd39_5fc3_44a8_a809_163a836e9031);
    const SHELL_HOOK_INTERFACE: GUID = GUID::from_u128(0x914d9b3a_5e53_4e14_bbba_46062acb35a4);
    type QueryService = unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT;

    // SAFETY: run_host initialized COM on this thread; Windows owns the
    // registered local server and returns an owned interface reference.
    let shell: IServiceProvider =
        unsafe { CoCreateInstance(&IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER) }?;
    let mut raw = std::ptr::null_mut();
    // SAFETY: IServiceProvider slot 3 is QueryService, and raw is writable.
    let query: QueryService = unsafe {
        let vtable = shell.as_raw().cast::<*const usize>().read();
        std::mem::transmute(vtable.add(3).read())
    };
    unsafe {
        query(
            shell.as_raw(),
            &SHELL_HOOK_SERVICE,
            &SHELL_HOOK_INTERFACE,
            &mut raw,
        )
    }
    .ok()?;
    if raw.is_null() {
        return Err(windows::core::Error::from_hresult(HRESULT(
            0x80004003_u32 as i32,
        )));
    }
    println!("phase=host-shell-hook-service interface={raw:p}");
    // SAFETY: Successful QueryService transferred one owned COM reference.
    Ok(unsafe { IUnknown::from_raw(raw) })
}

#[cfg(target_os = "windows")]
fn forward_shell_hook(
    service: &windows::core::IUnknown,
    code: usize,
    hwnd: isize,
) -> windows::core::Result<()> {
    use std::ffi::c_void;
    use windows::core::{HRESULT, Interface};

    type PostShellHookMessage = unsafe extern "system" fn(*mut c_void, usize, isize) -> HRESULT;
    // SAFETY: The queried interface's vtable has IUnknown slots 0-2,
    // Register at 3, Unregister at 4, and PostShellHookMessage at 5.
    let post: PostShellHookMessage = unsafe {
        let vtable = service.as_raw().cast::<*const usize>().read();
        std::mem::transmute(vtable.add(5).read())
    };
    unsafe { post(service.as_raw(), code, hwnd) }.ok()
}

#[cfg(target_os = "windows")]
fn probe_window_discovery(wrapper: usize, hwnd: usize) -> Result<(), String> {
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
fn probe_window_visibility(wrapper: usize, hwnd: usize) -> Result<(), String> {
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
fn probe_window_layout(wrapper: usize, hwnd: usize) -> Result<(), String> {
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

#[cfg(target_os = "windows")]
fn switch_settings_view_in_host() -> windows::core::Result<()> {
    use std::ffi::c_void;
    use windows::{
        Win32::System::Com::{CLSCTX_LOCAL_SERVER, CoCreateInstance, IServiceProvider},
        core::{GUID, HRESULT, IUnknown, Interface},
    };

    const IMMERSIVE_SHELL: GUID = GUID::from_u128(0xc2f03a33_21f5_47fa_b4bb_156362a2f239);
    const VIEW_COLLECTION: GUID = GUID::from_u128(0x1841c6d7_4f9d_42c0_af41_8747538f10e5);
    const VIEW_SWITCHER: GUID = GUID::from_u128(0xe0fe7384_5482_4659_ab2c_4ff038948779);
    const SETTINGS_AUMID: &str =
        "windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel";

    type QueryService = unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT;
    type GetViewForAppUserModelId =
        unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut c_void) -> HRESULT;
    type SwitchTo = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT;

    // SAFETY: Windows owns this registered local COM server; run_host already
    // initialized this shell thread's COM apartment.
    let shell: IServiceProvider =
        unsafe { CoCreateInstance(&IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER) }?;
    // SAFETY: Slot 3 of IServiceProvider is QueryService, returning owned
    // references released by the wrappers below.
    let query: QueryService = unsafe {
        let vtable = shell.as_raw().cast::<*const usize>().read();
        std::mem::transmute(vtable.add(3).read())
    };
    let mut collection_raw = std::ptr::null_mut();
    unsafe {
        query(
            shell.as_raw(),
            &VIEW_COLLECTION,
            &VIEW_COLLECTION,
            &mut collection_raw,
        )
    }
    .ok()?;
    let collection = unsafe { IUnknown::from_raw(collection_raw) };
    let wide_app_id: Vec<u16> = SETTINGS_AUMID.encode_utf16().chain(Some(0)).collect();
    let mut view_raw = std::ptr::null_mut();
    // SAFETY: This build's collection vtable identifies slot 8 as the AUMID
    // lookup method, which writes an owned IApplicationView reference.
    let get_view: GetViewForAppUserModelId = unsafe {
        let vtable = collection.as_raw().cast::<*const usize>().read();
        std::mem::transmute(vtable.add(8).read())
    };
    unsafe { get_view(collection.as_raw(), wide_app_id.as_ptr(), &mut view_raw) }.ok()?;
    if view_raw.is_null() {
        return Err(windows::core::Error::from_hresult(HRESULT(
            0x8002802b_u32 as i32,
        )));
    }
    let view = unsafe { IUnknown::from_raw(view_raw) };
    let mut switcher_raw = std::ptr::null_mut();
    unsafe {
        query(
            shell.as_raw(),
            &VIEW_SWITCHER,
            &VIEW_SWITCHER,
            &mut switcher_raw,
        )
    }
    .ok()?;
    let switcher = unsafe { IUnknown::from_raw(switcher_raw) };
    // SAFETY: This build's switcher vtable identifies slot 3 as SwitchTo.
    let switch_to: SwitchTo = unsafe {
        let vtable = switcher.as_raw().cast::<*const usize>().read();
        std::mem::transmute(vtable.add(3).read())
    };
    unsafe { switch_to(switcher.as_raw(), view.as_raw()) }.ok()
}

#[cfg(target_os = "windows")]
fn run_with_immersive_manager(
    app_id: &str,
    bootstrap_private_service: bool,
    register_shell_window: bool,
    fallback_band1: bool,
    frame_service_probe: bool,
    controller_start: bool,
    scenario1: bool,
    skip_window_events: bool,
) -> ExitCode {
    use std::process::Command;
    use windows::{
        Win32::System::Com::{
            CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance,
            CoInitializeEx, CoUninitialize, IServiceProvider,
        },
        core::{GUID, IUnknown},
    };

    const IMMERSIVE_APPLICATION_MANAGER: GUID =
        GUID::from_u128(0x50fdbb99_5c92_495e_9e81_e2c2f48cddae);
    const IMMERSIVE_SHELL: GUID = GUID::from_u128(0xc2f03a33_21f5_47fa_b4bb_156362a2f239);
    // Observed in CApplicationFrameService_CreateInstance on this Windows build.
    const FRAME_SERVICE_INTERFACE: GUID = GUID::from_u128(0x88b25c81_171b_48b0_91d6_c75846bcf035);

    if controller_start {
        if let Err(error) = configure_dpi_awareness() {
            eprintln!("phase=SetProcessDpiAwarenessContext error={error}");
            return ExitCode::FAILURE;
        }
    }

    let _shell_window = if register_shell_window {
        match ShellWindowGuard::register(false) {
            Some(window) => Some(window),
            None => return ExitCode::FAILURE,
        }
    } else {
        None
    };

    // SAFETY: COM is initialized and uninitialized on this worker thread.
    if let Err(error) = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok() {
        eprintln!(
            "phase=CoInitializeEx hresult={:?} error={error}",
            error.code()
        );
        return ExitCode::FAILURE;
    }
    println!("phase=CoInitializeEx result=success");
    let fallback_redirect = if fallback_band1 {
        match fallback_band1::FallbackBandRedirect::install() {
            Ok(redirect) => {
                println!("phase=fallback-band1-import result=installed");
                Some(redirect)
            }
            Err(error) => {
                eprintln!("phase=fallback-band1-import error={error}");
                // SAFETY: Balances successful COM initialization above.
                unsafe { CoUninitialize() };
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    let private_service = if bootstrap_private_service {
        match create_private_service() {
            Some(service) => Some(service),
            None => {
                // A failed private class construction can leave DLL-local
                // state partially initialized. Do not call into that DLL
                // again from this worker.
                // SAFETY: Balances successful COM initialization above.
                unsafe { CoUninitialize() };
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    let controller = if controller_start {
        match start_immersive_shell_controller(scenario1, skip_window_events, false) {
            Ok(controller) => Some(controller),
            Err(error) => {
                eprintln!("phase=ImmersiveShellController error={error}");
                drop(private_service);
                drop(fallback_redirect);
                // SAFETY: Balances successful COM initialization above.
                unsafe { CoUninitialize() };
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    // SAFETY: This registered CLSID is instantiated through COM. The returned
    // IUnknown stays alive until the activation probe exits on this thread.
    let manager: Option<IUnknown> = match unsafe {
        CoCreateInstance(&IMMERSIVE_APPLICATION_MANAGER, None, CLSCTX_INPROC_SERVER)
    } {
        Ok(manager) => {
            println!("phase=ImmersiveApplicationManager result=created");
            Some(manager)
        }
        Err(error) => {
            eprintln!(
                "phase=ImmersiveApplicationManager hresult={:?} error={error}",
                error.code()
            );
            None
        }
    };
    if frame_service_probe {
        let result = match manager.as_ref() {
            Some(manager) => frame_service_direct::probe(manager, app_id),
            None => Err("manager was not created".into()),
        };
        if let Err(error) = &result {
            eprintln!("phase=frame-service-direct error={error}");
        }
        drop(manager);
        drop(private_service);
        if let Some(redirect) = &fallback_redirect {
            println!(
                "phase=fallback-band1-import redirected_count={}",
                redirect.redirected_count()
            );
        }
        drop(fallback_redirect);
        // SAFETY: Balances the successful initialization above.
        unsafe { CoUninitialize() };
        return ExitCode::from(u8::from(result.is_err()));
    }
    // SAFETY: COM owns the local server lifetime. No raw pointer is retained.
    let shell: Option<IServiceProvider> =
        match unsafe { CoCreateInstance(&IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER) } {
            Ok(shell) => {
                println!("phase=ImmersiveShell result=created");
                Some(shell)
            }
            Err(error) => {
                eprintln!(
                    "phase=ImmersiveShell hresult={:?} error={error}",
                    error.code()
                );
                None
            }
        };
    let mut shell_manager = None;
    let mut frame_service = None;
    if let Some(shell) = &shell {
        // SAFETY: QueryService writes a COM-owned interface pointer, which the
        // windows crate releases when the returned IUnknown is dropped.
        match unsafe { shell.QueryService::<IUnknown>(&IMMERSIVE_APPLICATION_MANAGER) } {
            Ok(service) => {
                println!("phase=QueryService-ImmersiveApplicationManager result=created");
                shell_manager = Some(service);
            }
            Err(error) => eprintln!(
                "phase=QueryService-ImmersiveApplicationManager hresult={:?} error={error}",
                error.code()
            ),
        }
        // This GUID is an observed interface IID, not a documented service SID.
        // Querying it tests whether the shell provider exposes it directly.
        match unsafe { shell.QueryService::<IUnknown>(&FRAME_SERVICE_INTERFACE) } {
            Ok(service) => {
                println!("phase=QueryService-FrameInterface result=created");
                frame_service = Some(service);
            }
            Err(error) => eprintln!(
                "phase=QueryService-FrameInterface hresult={:?} error={error}",
                error.code()
            ),
        }
    }

    let probe = std::env::current_exe()
        .expect("worker path")
        .with_file_name("nickel-windows-app-probe.exe");
    let result = Command::new(probe).arg(app_id).status();
    let success = match result {
        Ok(status) => {
            println!("phase=ActivateApplication result={status}");
            status.success()
        }
        Err(error) => {
            eprintln!("phase=launch-app-probe error={error}");
            false
        }
    };
    drop(frame_service);
    drop(shell_manager);
    drop(shell);
    drop(manager);
    drop(private_service);
    drop(controller);
    if let Some(redirect) = &fallback_redirect {
        println!(
            "phase=fallback-band1-import redirected_count={}",
            redirect.redirected_count()
        );
    }
    drop(fallback_redirect);
    // SAFETY: Balances the successful initialization above, after COM objects drop.
    unsafe { CoUninitialize() };
    ExitCode::from(u8::from(!success))
}

#[cfg(target_os = "windows")]
fn start_immersive_shell_controller(
    scenario1: bool,
    skip_window_events: bool,
    skip_touch_keyboard: bool,
) -> Result<ControllerGuard, String> {
    use std::ffi::c_void;
    use windows::{
        Win32::System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
            LibraryLoader::GetModuleHandleW,
        },
        core::{GUID, HRESULT, IUnknown, Interface, w},
    };

    const BUILDER: GUID = GUID::from_u128(0xc71c41f1_ddad_42dc_a8fc_f5bfc61df957);
    const BUILDER2: GUID = GUID::from_u128(0x2eb59b15_1487_40ce_916e_ef65330dd224);
    // These RVAs are PDB-verified for Windows build 26200. Vtable checks
    // prevent an unsupported build from calling unknown private methods.
    const CREATE_CONTROLLER_RVA: usize = 0x238e70;
    const SET_SCENARIO_RVA: usize = 0x3f7660;
    const START_RVA: usize = 0x13b20;
    type QueryInterface =
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
    type CreateController = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;
    type SetScenario = unsafe extern "system" fn(*mut c_void, i32) -> HRESULT;
    type Start = unsafe extern "system" fn(*mut c_void) -> HRESULT;

    // SAFETY: COM is initialized by the caller and owns the returned reference.
    let builder: IUnknown = unsafe { CoCreateInstance(&BUILDER, None, CLSCTX_INPROC_SERVER) }
        .map_err(|error| format!("create builder: {error}"))?;
    // SAFETY: The builder is a live COM object with a valid IUnknown vtable.
    let query_address = unsafe {
        let vtable = builder.as_raw().cast::<*const usize>().read();
        vtable.read()
    };
    // SAFETY: Slot 0 of IUnknown is QueryInterface.
    let query: QueryInterface = unsafe { std::mem::transmute(query_address) };
    let mut builder2_raw = std::ptr::null_mut();
    // SAFETY: QueryInterface writes an owned interface reference on success.
    unsafe { query(builder.as_raw(), &BUILDER2, &mut builder2_raw) }
        .ok()
        .map_err(|error| format!("query builder2: {error}"))?;
    // SAFETY: QueryInterface returned one owned IUnknown-compatible reference.
    let builder2 = unsafe { IUnknown::from_raw(builder2_raw) };
    println!(
        "phase=ImmersiveShellBuilder interfaces builder={:p} builder2={:p}",
        builder.as_raw(),
        builder2.as_raw()
    );
    // SAFETY: The builder COM object keeps this in-process DLL loaded.
    let builder_module = unsafe { GetModuleHandleW(w!("twinui.pcshell.dll")) }
        .map_err(|error| format!("builder module: {error}"))?;
    let expected_create = builder_module.0 as usize + CREATE_CONTROLLER_RVA;
    // SAFETY: Both live COM interfaces have at least the three IUnknown slots
    // and one method slot. The target address is checked before calling it.
    let candidates = [builder.as_raw(), builder2.as_raw()];
    let create_methods = candidates.map(|interface| unsafe {
        let vtable = interface.cast::<*const usize>().read();
        vtable.add(3).read()
    });
    println!(
        "phase=ImmersiveShellBuilder methods={create_methods:#x?} expected_create={expected_create:#x}"
    );
    if scenario1 {
        let scenario_address = create_methods[1];
        let expected_scenario = builder_module.0 as usize + SET_SCENARIO_RVA;
        if scenario_address != expected_scenario {
            return Err("scenario method does not match this Windows build".into());
        }
        // SAFETY: The pointer matches the PDB-verified SetShellScenario method.
        let set_scenario: SetScenario = unsafe { std::mem::transmute(scenario_address) };
        let result = unsafe { set_scenario(builder2.as_raw(), 1) };
        println!("phase=ImmersiveShellBuilder-SetScenario1 hresult={result:?}");
        result
            .ok()
            .map_err(|error| format!("set scenario: {error}"))?;
    }
    let (create_interface, create_address) = candidates
        .into_iter()
        .zip(create_methods)
        .find(|(_, method)| *method == expected_create)
        .ok_or("builder vtable does not match this Windows build")?;
    // SAFETY: The address matches the PDB-verified builder method with ABI
    // HRESULT CreateImmersiveShellController(IImmersiveShellController**).
    let create: CreateController = unsafe { std::mem::transmute(create_address) };
    let mut controller_raw = std::ptr::null_mut();
    let result = unsafe { create(create_interface, &mut controller_raw) };
    println!("phase=ImmersiveShellBuilder-CreateController hresult={result:?}");
    result
        .ok()
        .map_err(|error| format!("create controller through builder: {error}"))?;
    if controller_raw.is_null() {
        return Err("builder returned a null controller".into());
    }
    // SAFETY: The builder returned one owned IUnknown-compatible reference.
    let controller = unsafe { IUnknown::from_raw(controller_raw) };
    // SAFETY: The controller has a valid vtable; slot 3 is checked below.
    let start_address = unsafe {
        let vtable = controller.as_raw().cast::<*const usize>().read();
        vtable.add(3).read()
    };
    let behavior_vtable = if skip_window_events || skip_touch_keyboard {
        Some(install_component_filter(
            &controller,
            builder_module.0 as usize,
            skip_window_events,
            skip_touch_keyboard,
        )?)
    } else {
        None
    };
    // SAFETY: The module is loaded by CoCreateInstance and remains loaded while
    // the returned controller exists.
    let module = unsafe { GetModuleHandleW(w!("Windows.ImmersiveShell.ServiceProvider.dll")) }
        .map_err(|error| format!("controller module: {error}"))?;
    let expected = module.0 as usize + START_RVA;
    println!("phase=ImmersiveShellController start={start_address:#x} expected={expected:#x}");
    if start_address != expected {
        return Err("controller vtable does not match this Windows build".into());
    }
    // SAFETY: The address matches the PDB-verified CImmersiveShellController::Start
    // method, whose ABI is HRESULT Start(void). The object is still alive.
    let start: Start = unsafe { std::mem::transmute(start_address) };
    let result = unsafe { start(controller.as_raw()) };
    println!("phase=ImmersiveShellController-Start hresult={result:?}");
    result
        .ok()
        .map_err(|error| format!("start controller: {error}"))?;
    Ok(ControllerGuard {
        _controller: controller,
        _behavior_vtable: behavior_vtable,
    })
}

#[cfg(target_os = "windows")]
struct ControllerGuard {
    // Drop the controller before its behavior's copied vtable.
    _controller: windows::core::IUnknown,
    _behavior_vtable: Option<Box<[usize; 12]>>,
}

#[cfg(target_os = "windows")]
static ORIGINAL_SHOULD_CREATE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(target_os = "windows")]
static COMPONENT_LIMIT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(u32::MAX);
#[cfg(target_os = "windows")]
static SKIP_WINDOW_EVENTS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(target_os = "windows")]
static SKIP_TOUCH_KEYBOARD: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "windows")]
unsafe extern "system" fn should_create_component_diagnostic(
    this: *mut std::ffi::c_void,
    index: u32,
    should_create: *mut i32,
    class_id: *mut windows::core::GUID,
) -> windows::core::HRESULT {
    use std::sync::atomic::Ordering;
    type Method = unsafe extern "system" fn(
        *mut std::ffi::c_void,
        u32,
        *mut i32,
        *mut windows::core::GUID,
    ) -> windows::core::HRESULT;
    let address = ORIGINAL_SHOULD_CREATE.load(Ordering::Acquire);
    // SAFETY: The installation routine stores a PDB-verified method address
    // before publishing this replacement vtable to another thread.
    let original: Method = unsafe { std::mem::transmute(address) };
    let result = unsafe { original(this, index, should_create, class_id) };
    const WINDOW_MANAGEMENT_EVENTS: windows::core::GUID =
        windows::core::GUID::from_u128(0x0cd069cf_ac9b_41f4_9571_3a95a62c36a1);
    const TOUCH_KEYBOARD_EXPERIENCE_MANAGER: windows::core::GUID =
        windows::core::GUID::from_u128(0xf580a09b_ea6e_4d83_9a03_add6cb756ab3);
    let limit = COMPONENT_LIMIT.load(Ordering::Acquire);
    let class = result.is_ok().then(|| unsafe { class_id.read() });
    let is_window_events =
        class == Some(WINDOW_MANAGEMENT_EVENTS) && SKIP_WINDOW_EVENTS.load(Ordering::Acquire);
    let is_touch_keyboard = class == Some(TOUCH_KEYBOARD_EXPERIENCE_MANAGER)
        && SKIP_TOUCH_KEYBOARD.load(Ordering::Acquire);
    if result.is_ok() && (is_window_events || is_touch_keyboard || index >= limit) {
        // SAFETY: The COM method contract provides this writable output.
        unsafe { should_create.write(0) };
        if is_window_events || is_touch_keyboard || index == limit {
            println!(
                "phase=skip-component index={index} window_events={is_window_events} touch_keyboard={is_touch_keyboard}"
            );
        }
    }
    result
}

#[cfg(target_os = "windows")]
fn install_component_filter(
    controller: &windows::core::IUnknown,
    twinui_base: usize,
    skip_window_events: bool,
    skip_touch_keyboard: bool,
) -> Result<Box<[usize; 12]>, String> {
    use std::{ffi::c_void, sync::atomic::Ordering};
    use windows::core::Interface;

    // CImmersiveShellController stores the behavior COM interface at +0x60
    // on this build. The builder has already installed it before returning.
    // SAFETY: This private offset was verified against the matching PDB and
    // the controller stays alive throughout this probe.
    let behavior = unsafe {
        controller
            .as_raw()
            .cast::<u8>()
            .add(0x60)
            .cast::<*mut c_void>()
            .read()
    };
    if behavior.is_null() {
        return Err("builder did not install creation behavior".into());
    }
    // SAFETY: The live COM interface begins with a valid vtable pointer.
    let original_vtable = unsafe { behavior.cast::<*const usize>().read() };
    // The behavior interface has 12 slots on this Windows build. Slot 8 is
    // CImmersiveShellCreationBehavior::ShouldCreateComponent.
    let method = unsafe { original_vtable.add(8).read() };
    const SHOULD_CREATE_RVA: usize = 0x1806d0;
    let expected = twinui_base + SHOULD_CREATE_RVA;
    println!("phase=creation-behavior should_create={method:#x} expected={expected:#x}");
    if method != expected {
        return Err("creation behavior vtable does not match this Windows build".into());
    }
    let mut copied = Box::new([0usize; 12]);
    // SAFETY: The PDB-verified COM interface has exactly 12 vtable slots.
    unsafe { std::ptr::copy_nonoverlapping(original_vtable, copied.as_mut_ptr(), copied.len()) };
    copied[8] = should_create_component_diagnostic as *const () as usize;
    ORIGINAL_SHOULD_CREATE.store(method, Ordering::Release);
    SKIP_WINDOW_EVENTS.store(skip_window_events, Ordering::Release);
    SKIP_TOUCH_KEYBOARD.store(skip_touch_keyboard, Ordering::Release);
    let limit = std::env::var("NICKEL_FRAME_PROBE_COMPONENT_LIMIT")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(u32::MAX);
    println!("phase=creation-behavior component_limit={limit}");
    COMPONENT_LIMIT.store(limit, Ordering::Release);
    // SAFETY: The COM object resides in writable heap memory. The copied
    // vtable remains alive until after its owning controller is dropped.
    unsafe { behavior.cast::<*const usize>().write(copied.as_ptr()) };
    Ok(copied)
}

#[cfg(target_os = "windows")]
static SHELL_HOOK_MESSAGE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
#[cfg(target_os = "windows")]
static ORIGINAL_SHELL_WNDPROC: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(0);

#[cfg(target_os = "windows")]
unsafe extern "system" fn shell_window_proc(
    hwnd: windows::Win32::Foundation::HWND,
    message: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use std::sync::atomic::Ordering;
    use windows::Win32::{
        Foundation::LRESULT,
        UI::WindowsAndMessaging::{CallWindowProcW, DefWindowProcW, PostMessageW, WNDPROC},
    };

    let hook_message = SHELL_HOOK_MESSAGE.load(Ordering::Relaxed);
    if hook_message != 0 && message == hook_message {
        if matches!(wparam.0, 1 | 2) {
            // SAFETY: hwnd is the live window receiving this callback. Posting
            // avoids a COM call inside Windows' synchronous shell hook delivery.
            if let Err(error) = unsafe { PostMessageW(Some(hwnd), 0x8070, wparam, lparam) } {
                eprintln!("phase=queue-shell-hook error={error}");
            }
        }
        return LRESULT(0);
    }
    let original = ORIGINAL_SHELL_WNDPROC.load(Ordering::Relaxed);
    if original != 0 {
        // SAFETY: SetWindowLongPtrW returned this live system STATIC window
        // procedure when the same HWND was subclassed below.
        let previous: WNDPROC = unsafe { std::mem::transmute(original) };
        unsafe { CallWindowProcW(previous, hwnd, message, wparam, lparam) }
    } else {
        // SAFETY: The original procedure may be temporarily unavailable
        // while the window is being registered or destroyed.
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
}

#[cfg(target_os = "windows")]
struct ShellWindowGuard {
    window: windows::Win32::Foundation::HWND,
    shell_hook_registered: bool,
    taskman_registered: bool,
    original_wndproc: isize,
    set_taskman: unsafe extern "system" fn(*mut std::ffi::c_void) -> i32,
    _library: libloading::Library,
}

#[cfg(target_os = "windows")]
impl ShellWindowGuard {
    fn register(register_taskman: bool) -> Option<Self> {
        use std::sync::atomic::Ordering;
        use std::{ffi::c_void, path::PathBuf};
        use windows::{
            Win32::{
                Foundation::GetLastError,
                UI::WindowsAndMessaging::{
                    CreateWindowExW, DeregisterShellHookWindow, DestroyWindow, GWLP_WNDPROC,
                    GetShellWindow, RegisterShellHookWindow, RegisterWindowMessageW,
                    SetWindowLongPtrW, WINDOW_EX_STYLE, WS_POPUP,
                },
            },
            core::w,
        };

        type SetShellWindowEx = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;
        type SetTaskmanWindow = unsafe extern "system" fn(*mut c_void) -> i32;
        type GetTaskmanWindow = unsafe extern "system" fn() -> *mut c_void;

        if !unsafe { GetShellWindow() }.is_invalid() {
            eprintln!("phase=shell-window result=already-registered");
            return None;
        }
        let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let path = PathBuf::from(system_root)
            .join("System32")
            .join("user32.dll");
        // SAFETY: This absolute path targets the installed system user32 DLL.
        let library = match unsafe { libloading::Library::new(path) } {
            Ok(library) => library,
            Err(error) => {
                eprintln!("phase=load-user32 error={error}");
                return None;
            }
        };
        // SAFETY: The DLL remains loaded in the returned guard while the
        // function pointer and registered window are in use.
        let set_shell = match unsafe { library.get::<SetShellWindowEx>(b"SetShellWindowEx\0") } {
            Ok(symbol) => symbol,
            Err(error) => {
                eprintln!("phase=resolve-SetShellWindowEx error={error}");
                return None;
            }
        };
        let set_taskman = match unsafe { library.get::<SetTaskmanWindow>(b"SetTaskmanWindow\0") } {
            Ok(symbol) => *symbol,
            Err(error) => {
                eprintln!("phase=resolve-SetTaskmanWindow error={error}");
                return None;
            }
        };
        let get_taskman = match unsafe { library.get::<GetTaskmanWindow>(b"GetTaskmanWindow\0") } {
            Ok(symbol) => *symbol,
            Err(error) => {
                eprintln!("phase=resolve-GetTaskmanWindow error={error}");
                return None;
            }
        };
        // SAFETY: STATIC is a system-defined class. The hidden top-level
        // window is owned by this thread for the lifetime of the guard.
        let window = match unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Nickel frame diagnostic"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
        } {
            Ok(window) => window,
            Err(error) => {
                eprintln!(
                    "phase=create-shell-window hresult={:?} error={error}",
                    error.code()
                );
                return None;
            }
        };
        // SAFETY: This process owns the valid top-level window; the null
        // second argument means it has no separate desktop list-view window.
        let registered = unsafe { set_shell(window.0, std::ptr::null_mut()) } != 0;
        if !registered {
            let error = unsafe { GetLastError() };
            eprintln!(
                "phase=SetShellWindowEx result=failed last_error={}",
                error.0
            );
            let _ = unsafe { DestroyWindow(window) };
            return None;
        }
        let matches = unsafe { GetShellWindow() } == window;
        println!(
            "phase=SetShellWindowEx result=success shell_hwnd={:?} matches_own_window={matches}",
            window.0
        );
        if !matches {
            let _ = unsafe { DestroyWindow(window) };
            return None;
        }
        // Shell hook notifications are sent directly to the window procedure;
        // they are not returned by GetMessageW. Subclass this hidden window
        // before registering it so the message loop can forward each event.
        let hook_message = unsafe { RegisterWindowMessageW(w!("SHELLHOOK")) };
        if hook_message == 0 {
            eprintln!("phase=register-shell-hook-message result=failed");
            let _ = unsafe { DestroyWindow(window) };
            return None;
        }
        SHELL_HOOK_MESSAGE.store(hook_message, Ordering::Relaxed);
        let original_wndproc = unsafe {
            SetWindowLongPtrW(
                window,
                GWLP_WNDPROC,
                shell_window_proc as *const () as usize as isize,
            )
        };
        if original_wndproc == 0 {
            eprintln!("phase=subclass-shell-window result=failed");
            SHELL_HOOK_MESSAGE.store(0, Ordering::Relaxed);
            let _ = unsafe { DestroyWindow(window) };
            return None;
        }
        ORIGINAL_SHELL_WNDPROC.store(original_wndproc, Ordering::Relaxed);
        println!("phase=shell-hook-message id={hook_message:#x}");
        // SAFETY: This thread owns the live shell HWND. The guard unregisters
        // the same HWND before destroying it.
        let shell_hook_registered = unsafe { RegisterShellHookWindow(window) }.as_bool();
        if shell_hook_registered {
            println!("phase=RegisterShellHookWindow result=success");
        } else {
            println!(
                "phase=RegisterShellHookWindow result=failed last_error={}",
                unsafe { GetLastError() }.0
            );
            let _ = unsafe { SetWindowLongPtrW(window, GWLP_WNDPROC, original_wndproc) };
            ORIGINAL_SHELL_WNDPROC.store(0, Ordering::Relaxed);
            SHELL_HOOK_MESSAGE.store(0, Ordering::Relaxed);
            let _ = unsafe { DestroyWindow(window) };
            return None;
        }
        // The shell window alone does not cause the desktop's shell hook
        // notifications to reach this process. The task manager window is
        // registered by Explorer and by other replacement shells.
        let taskman_registered = if register_taskman {
            let registered = unsafe { set_taskman(window.0) } != 0;
            let matches = unsafe { get_taskman() } == window.0;
            println!("phase=SetTaskmanWindow registered={registered} matches_own_window={matches}");
            if !registered || !matches {
                let _ = unsafe { DeregisterShellHookWindow(window) };
                let _ = unsafe { SetWindowLongPtrW(window, GWLP_WNDPROC, original_wndproc) };
                ORIGINAL_SHELL_WNDPROC.store(0, Ordering::Relaxed);
                SHELL_HOOK_MESSAGE.store(0, Ordering::Relaxed);
                let _ = unsafe { DestroyWindow(window) };
                return None;
            }
            true
        } else {
            false
        };
        Some(Self {
            window,
            shell_hook_registered,
            taskman_registered,
            original_wndproc,
            set_taskman,
            _library: library,
        })
    }
}

#[cfg(target_os = "windows")]
impl Drop for ShellWindowGuard {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        use windows::Win32::UI::WindowsAndMessaging::{
            DeregisterShellHookWindow, DestroyWindow, GWLP_WNDPROC, SetWindowLongPtrW,
        };

        // SAFETY: The current thread created this window and owns its lifetime.
        if self.shell_hook_registered {
            let _ = unsafe { DeregisterShellHookWindow(self.window) };
        }
        if self.taskman_registered {
            let _ = unsafe { (self.set_taskman)(std::ptr::null_mut()) };
        }
        let _ = unsafe { SetWindowLongPtrW(self.window, GWLP_WNDPROC, self.original_wndproc) };
        ORIGINAL_SHELL_WNDPROC.store(0, Ordering::Relaxed);
        SHELL_HOOK_MESSAGE.store(0, Ordering::Relaxed);
        let _ = unsafe { DestroyWindow(self.window) };
    }
}

#[cfg(target_os = "windows")]
fn create_private_service() -> Option<(libloading::Library, windows::core::IUnknown)> {
    use std::{ffi::c_void, path::PathBuf};
    use windows::{
        Win32::System::Com::IClassFactory,
        core::{GUID, HRESULT, IUnknown, Interface},
    };

    type DllGetClassObject =
        unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
    const PRIVILEGED_OPERATIONS_SERVICE: GUID =
        GUID::from_u128(0xe7a8f97b_ca27_413b_851b_af50345ecc63);

    let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let path = PathBuf::from(system_root)
        .join("System32")
        .join("twinui.pcshell.dll");
    // SAFETY: The absolute path targets Windows' system DLL; it remains loaded
    // until after the COM object returned by its class factory is released.
    let library = match unsafe { libloading::Library::new(&path) } {
        Ok(library) => library,
        Err(error) => {
            eprintln!("phase=LoadTwinui error={error}");
            return None;
        }
    };
    let service = {
        // SAFETY: DllGetClassObject has the documented COM signature. The
        // requested CLSID is internal and used only by this diagnostic probe.
        let entry = match unsafe { library.get::<DllGetClassObject>(b"DllGetClassObject\0") } {
            Ok(entry) => entry,
            Err(error) => {
                eprintln!("phase=DllGetClassObject-export error={error}");
                return None;
            }
        };
        let mut raw = std::ptr::null_mut();
        // SAFETY: The output pointer is valid; a successful call transfers one
        // COM reference to the caller, which IClassFactory then owns.
        let result = unsafe {
            entry(
                &PRIVILEGED_OPERATIONS_SERVICE,
                &IClassFactory::IID,
                &mut raw,
            )
        };
        if let Err(error) = result.ok() {
            eprintln!(
                "phase=DllGetClassObject hresult={:?} error={error}",
                error.code()
            );
            return None;
        }
        println!("phase=DllGetClassObject result=success");
        // SAFETY: DllGetClassObject succeeded and returned an IClassFactory
        // pointer with one owned reference.
        let factory = unsafe { IClassFactory::from_raw(raw) };
        // SAFETY: COM constructs the object and returns an owned IUnknown.
        match unsafe { factory.CreateInstance::<_, IUnknown>(None) } {
            Ok(service) => {
                println!("phase=PrivilegedOperationsService result=created");
                service
            }
            Err(error) => {
                eprintln!(
                    "phase=PrivilegedOperationsService hresult={:?} error={error}",
                    error.code()
                );
                return None;
            }
        }
    };
    Some((library, service))
}
