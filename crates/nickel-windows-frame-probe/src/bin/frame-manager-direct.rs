use std::process::ExitCode;

#[cfg(not(target_os = "windows"))]
fn main() -> ExitCode {
    eprintln!("frame-manager-direct supports Windows only");
    ExitCode::FAILURE
}

#[cfg(target_os = "windows")]
mod windows_probe {
    use std::{ffi::c_void, process::ExitCode};
    use windows::Win32::System::Com::{
        CLSCTX_LOCAL_SERVER, CLSCTX_NO_CODE_DOWNLOAD, COINIT_APARTMENTTHREADED, CoCreateInstance,
        CoInitializeEx, CoUninitialize,
    };
    use windows_core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, implement, interface};

    const APPLICATION_FRAME_HOST: GUID = GUID::from_u128(0xb9b05098_3e30_483f_87f7_027ca78da287);

    // These private IIDs and vtable positions were checked against this
    // installation's ApplicationFrame.dll and twinui.pcshell.dll symbols.
    #[interface("a914f499-4633-4b26-a93e-707eb5bdc0b6")]
    unsafe trait IApplicationFrameManager: IUnknown {
        fn create_frame(&self, frame: *mut *mut c_void) -> HRESULT;
        fn create_frame_with_options(&self, options: u32, frame: *mut *mut c_void) -> HRESULT;
        fn destroy_frame(&self, frame: *mut c_void) -> HRESULT;
        fn get_frame_array(&self, frames: *mut *mut c_void) -> HRESULT;
        fn register_for_frame_events(
            &self,
            handler: &IApplicationFrameEventHandler,
            cookie: *mut u32,
        ) -> HRESULT;
        fn unregister_for_frame_events(&self, cookie: u32) -> HRESULT;
    }

    #[interface("ea5d0de4-770d-4da0-a9f8-d7f9a140ff79")]
    unsafe trait IApplicationFrameEventHandler: IUnknown {
        fn on_position_changed(&self, frame: *mut c_void) -> HRESULT;
        fn on_command(&self, frame: *mut c_void, command: *const GUID, value: u32) -> HRESULT;
        fn on_chrome_offsets_changed(&self, frame: *mut c_void) -> HRESULT;
    }

    #[implement(IApplicationFrameEventHandler)]
    struct FrameEvents;

    impl IApplicationFrameEventHandler_Impl for FrameEvents_Impl {
        unsafe fn on_position_changed(&self, _frame: *mut c_void) -> HRESULT {
            println!("phase=frame-event kind=position-changed");
            HRESULT(0)
        }

        unsafe fn on_command(
            &self,
            _frame: *mut c_void,
            _command: *const GUID,
            value: u32,
        ) -> HRESULT {
            println!("phase=frame-event kind=command value={value}");
            HRESULT(0)
        }

        unsafe fn on_chrome_offsets_changed(&self, _frame: *mut c_void) -> HRESULT {
            println!("phase=frame-event kind=chrome-offsets-changed");
            HRESULT(0)
        }
    }

    pub fn run() -> ExitCode {
        use std::{
            thread,
            time::{Duration, Instant},
        };

        let mut arguments = std::env::args().skip(1);
        let worker = arguments.next().as_deref() == Some("--worker");
        let app_id = if worker {
            arguments.next()
        } else {
            std::env::args().nth(1)
        };
        let Some(app_id) = app_id else {
            eprintln!("usage: frame-manager-direct AUMID");
            return ExitCode::FAILURE;
        };
        if !worker {
            let mut child =
                match std::process::Command::new(std::env::current_exe().expect("probe path"))
                    .args(["--worker", &app_id])
                    .spawn()
                {
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
                        eprintln!("phase=frame-manager-worker result=timeout elapsed_seconds=30");
                        return ExitCode::FAILURE;
                    }
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        eprintln!("phase=frame-manager-worker error={error}");
                        return ExitCode::FAILURE;
                    }
                }
            }
        }

        println!("phase=worker-start pid={}", std::process::id());

        // SAFETY: This worker uses COM on its main thread and balances the
        // initialization after all COM objects have been released.
        if let Err(error) = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok() {
            eprintln!("phase=CoInitializeEx hresult={:?}", error.code());
            return ExitCode::FAILURE;
        }
        println!("phase=CoInitializeEx result=success");

        let outcome = (|| {
            // SAFETY: The registered local server supplies and owns the COM
            // object. The private IID was verified against this build.
            println!("phase=CoCreateFrameManager result=begin");
            let manager: IApplicationFrameManager = unsafe {
                CoCreateInstance(
                    &APPLICATION_FRAME_HOST,
                    None,
                    CLSCTX_LOCAL_SERVER | CLSCTX_NO_CODE_DOWNLOAD,
                )
            }
            .map_err(|error| {
                eprintln!("phase=CoCreateFrameManager hresult={:?}", error.code());
            })?;
            println!("phase=CoCreateFrameManager result=success");

            let handler: IApplicationFrameEventHandler = FrameEvents.into();
            let mut cookie = 0_u32;
            // SAFETY: The callback implements the IID and vtable observed in
            // the matching Windows binaries, and remains alive until it is
            // unregistered below.
            let registration = unsafe { manager.register_for_frame_events(&handler, &mut cookie) };
            if let Err(error) = registration.ok() {
                eprintln!("phase=RegisterForFrameEvents hresult={:?}", error.code());
                return Err(());
            }
            println!("phase=RegisterForFrameEvents result=success cookie={cookie}");

            let probe = std::env::current_exe()
                .expect("frame probe path")
                .with_file_name("nickel-windows-app-probe.exe");
            let result = std::process::Command::new(probe).arg(&app_id).status();
            let activated = match result {
                Ok(status) => {
                    println!("phase=ActivateApplication status={status}");
                    status.success()
                }
                Err(error) => {
                    eprintln!("phase=launch-app-probe error={error}");
                    false
                }
            };

            // SAFETY: The cookie came from successful registration with this
            // manager and the callback remains alive until this call returns.
            let unregistration = unsafe { manager.unregister_for_frame_events(cookie) };
            println!(
                "phase=UnregisterForFrameEvents hresult={:?}",
                unregistration
            );
            if activated { Ok(()) } else { Err(()) }
        })();

        // SAFETY: All COM objects in the closure have been dropped.
        unsafe { CoUninitialize() };
        ExitCode::from(u8::from(outcome.is_err()))
    }
}

#[cfg(target_os = "windows")]
fn main() -> ExitCode {
    windows_probe::run()
}
