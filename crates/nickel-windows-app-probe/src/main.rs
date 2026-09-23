use std::process::ExitCode;

#[cfg(not(target_os = "windows"))]
fn main() -> ExitCode {
    eprintln!("nickel-windows-app-probe supports Windows only");
    ExitCode::FAILURE
}

#[cfg(target_os = "windows")]
fn main() -> ExitCode {
    use std::{
        process::Command,
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
        eprintln!("usage: nickel-windows-app-probe AUMID [--inproc]");
        return ExitCode::FAILURE;
    };
    let context = match (arguments.next().as_deref(), arguments.next()) {
        (None, None) => "local-server",
        (Some("--inproc"), None) => "inproc",
        _ => {
            eprintln!("usage: nickel-windows-app-probe AUMID [--inproc]");
            return ExitCode::FAILURE;
        }
    };
    if worker {
        return activate(&app_id, context);
    }

    let mut command = Command::new(std::env::current_exe().expect("probe executable path"));
    command.args(["--worker", &app_id]);
    if context == "inproc" {
        command.arg("--inproc");
    }
    let Ok(mut child) = command.spawn() else {
        eprintln!("phase=worker result=spawn-failed");
        return ExitCode::FAILURE;
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                };
            }
            Ok(None) if started.elapsed() < Duration::from_secs(20) => {
                thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                eprintln!("phase=worker result=timeout elapsed_seconds=20");
                return ExitCode::FAILURE;
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                eprintln!("phase=worker result=wait-failed error={error}");
                return ExitCode::FAILURE;
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn activate(app_id: &str, context: &str) -> ExitCode {
    use windows::{
        Win32::{
            System::Com::{
                CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED,
                CoCreateInstance, CoInitializeEx, CoUninitialize,
            },
            UI::Shell::{AO_NONE, ApplicationActivationManager, IApplicationActivationManager},
        },
        core::PCWSTR,
    };

    let context = match context {
        "inproc" => CLSCTX_INPROC_SERVER,
        _ => CLSCTX_LOCAL_SERVER,
    };
    println!(
        "pid={} app_id={} context={}",
        std::process::id(),
        app_id,
        if context == CLSCTX_LOCAL_SERVER {
            "local-server"
        } else {
            "inproc"
        }
    );

    // SAFETY: This process uses COM only on this thread. A successful
    // initialization is balanced after the synchronous activation attempt.
    if let Err(error) = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok() {
        eprintln!(
            "phase=CoInitializeEx hresult={:?} error={error}",
            error.code()
        );
        return ExitCode::FAILURE;
    }
    println!("phase=CoInitializeEx result=success");
    let result = (|| {
        let manager: IApplicationActivationManager =
            unsafe { CoCreateInstance(&ApplicationActivationManager, None, context) }.map_err(
                |error| {
                    eprintln!(
                        "phase=CoCreateInstance hresult={:?} error={error}",
                        error.code()
                    );
                },
            )?;
        println!("phase=CoCreateInstance result=success");
        let wide_app_id: Vec<u16> = app_id.encode_utf16().chain([0]).collect();
        let process_id = unsafe {
            manager.ActivateApplication(PCWSTR(wide_app_id.as_ptr()), PCWSTR::null(), AO_NONE)
        }
        .map_err(|error| {
            eprintln!(
                "phase=ActivateApplication hresult={:?} error={error}",
                error.code()
            );
        })?;
        println!("phase=ActivateApplication result=success process_id={process_id}");
        Ok::<(), ()>(())
    })();
    // SAFETY: Balances the successful CoInitializeEx call on this thread.
    unsafe { CoUninitialize() };
    if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
