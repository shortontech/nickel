//! Manual diagnostic commands retained alongside the managed UWP host.

#[cfg(target_os = "windows")]
use crate::{
    controller::start_immersive_shell_controller,
    fallback_band1, frame_service_direct,
    host::{configure_dpi_awareness, run_host},
    shell_window::ShellWindowGuard,
};
use std::process::ExitCode;

#[cfg(not(target_os = "windows"))]
pub fn run_cli() -> ExitCode {
    eprintln!("nickel-windows-frame-probe supports Windows only");
    ExitCode::FAILURE
}

#[cfg(target_os = "windows")]
pub fn run_cli() -> ExitCode {
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
            DiagnosticOptions {
                bootstrap_private_service: private_service,
                register_shell_window: shell_window,
                fallback_band1,
                frame_service_probe,
                controller_start,
                scenario1,
                skip_window_events,
            },
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
struct DiagnosticOptions {
    bootstrap_private_service: bool,
    register_shell_window: bool,
    fallback_band1: bool,
    frame_service_probe: bool,
    controller_start: bool,
    scenario1: bool,
    skip_window_events: bool,
}

#[cfg(target_os = "windows")]
fn run_with_immersive_manager(app_id: &str, options: DiagnosticOptions) -> ExitCode {
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

    let DiagnosticOptions {
        bootstrap_private_service,
        register_shell_window,
        fallback_band1,
        frame_service_probe,
        controller_start,
        scenario1,
        skip_window_events,
    } = options;
    if controller_start && let Err(error) = configure_dpi_awareness() {
        eprintln!("phase=SetProcessDpiAwarenessContext error={error}");
        return ExitCode::FAILURE;
    }

    let _shell_window = if register_shell_window {
        match ShellWindowGuard::register() {
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
