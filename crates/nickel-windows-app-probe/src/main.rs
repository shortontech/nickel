use std::process::ExitCode;

#[cfg(not(target_os = "windows"))]
fn main() -> ExitCode {
    eprintln!("nickel-windows-app-probe supports Windows only");
    ExitCode::FAILURE
}

#[cfg(target_os = "windows")]
fn main() -> ExitCode {
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

    let mut arguments = std::env::args().skip(1);
    let Some(app_id) = arguments.next() else {
        eprintln!("usage: nickel-windows-app-probe AUMID [--inproc]");
        return ExitCode::FAILURE;
    };
    let context = match (arguments.next().as_deref(), arguments.next()) {
        (None, None) => CLSCTX_LOCAL_SERVER,
        (Some("--inproc"), None) => CLSCTX_INPROC_SERVER,
        _ => {
            eprintln!("usage: nickel-windows-app-probe AUMID [--inproc]");
            return ExitCode::FAILURE;
        }
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
