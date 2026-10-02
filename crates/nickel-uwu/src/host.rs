//! Owns the shell STA, lifecycle messages, and UWP presentation loop.

use crate::{
    auto_present,
    controller::start_immersive_shell_controller,
    fallback_band1,
    presentation_callbacks::{
        probe_frame_fit_to_work_area, probe_frame_set_position, probe_frame_set_presented_window,
        probe_view_wrapper_discovery, probe_view_wrapper_foreground, probe_view_wrapper_readiness,
        probe_view_wrapper_set_frame, probe_view_wrapper_set_size, probe_view_wrapper_uncloak,
        probe_window_discovery, probe_window_layout, probe_window_visibility,
    },
    shell_window::ShellWindowGuard,
};
use std::{
    process::ExitCode,
    time::{Duration, Instant},
};
#[cfg(target_os = "windows")]
use windows::core::Interface;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewEventCallbackResult {
    Success,
    NotImplemented,
}

#[cfg(all(target_os = "windows", feature = "diagnostics"))]
fn terminate_view_event_app(process_id: u32) -> Result<(), String> {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess},
    };

    let process = unsafe { OpenProcess(PROCESS_TERMINATE, false, process_id) }
        .map_err(|error| error.to_string())?;
    let result = unsafe { TerminateProcess(process, 0) }.map_err(|error| error.to_string());
    let _ = unsafe { CloseHandle(process) };
    result
}

#[cfg(all(target_os = "windows", feature = "diagnostics"))]
fn log_view_event_window_state(kind: &str, hwnd: usize) {
    use std::ffi::c_void;
    use windows::Win32::{
        Foundation::{HWND, RECT},
        UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GWL_STYLE, GetClientRect, GetParent, GetWindowLongPtrW, GetWindowRect,
            GetWindowThreadProcessId, IsWindowVisible,
        },
    };

    let window = HWND(hwnd as *mut c_void);
    let mut pid = 0;
    let mut rect = RECT::default();
    let mut client = RECT::default();
    // SAFETY: These calls only inspect a live HWND supplied by the view wrapper.
    let tid = unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    let parent = unsafe { GetParent(window) }
        .ok()
        .map(|value| value.0 as usize)
        .unwrap_or_default();
    let style = unsafe { GetWindowLongPtrW(window, GWL_STYLE) } as usize;
    let ex_style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as usize;
    let visible = unsafe { IsWindowVisible(window) }.as_bool();
    let _ = unsafe { GetWindowRect(window, &mut rect) };
    let _ = unsafe { GetClientRect(window, &mut client) };
    println!(
        "phase=view-event-window-state kind={kind} hwnd={hwnd:#x} pid={pid} tid={tid} parent={parent:#x} visible={visible} style={style:#x} ex_style={ex_style:#x} rect={rect:?} client={client:?}"
    );
}

#[cfg(target_os = "windows")]
struct HostReadiness(Option<std::sync::mpsc::SyncSender<Result<(), String>>>);

#[cfg(target_os = "windows")]
impl HostReadiness {
    fn ready(&mut self) {
        if let Some(sender) = self.0.take() {
            tracing::info!("embedded UWP shell host ready");
            let _ = sender.send(Ok(()));
        }
    }

    fn failed(&mut self, phase: &'static str, error: impl std::fmt::Display) {
        tracing::warn!(phase, %error, "embedded UWP shell host failed to start");
        if let Some(sender) = self.0.take() {
            let _ = sender.send(Err(format!("{phase}: {error}")));
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for HostReadiness {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(Err("UWP shell host exited before ready".into()));
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn configure_dpi_awareness() -> Result<(), String> {
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
pub fn run_host(pump_messages: bool) -> ExitCode {
    run_host_with_parent(
        pump_messages,
        None,
        true,
        false,
        true,
        None,
        None,
        HostReadiness(None),
    )
}

/// Diagnostic variant that publishes the ImmersiveShellBroker class.
#[cfg(all(target_os = "windows", feature = "diagnostics"))]
pub fn run_host_with_broker(pump_messages: bool) -> ExitCode {
    run_host_with_parent(
        pump_messages,
        None,
        true,
        true,
        true,
        None,
        None,
        HostReadiness(None),
    )
}

/// Diagnostic fixture for comparing broker publication without AutoPresent.
#[cfg(all(target_os = "windows", feature = "diagnostics"))]
pub fn run_host_without_auto_present(publish_broker: bool) -> ExitCode {
    run_host_with_parent(
        true,
        None,
        true,
        publish_broker,
        false,
        None,
        None,
        HostReadiness(None),
    )
}

/// Runs the regular controller with a local view-event listener and no polling.
#[cfg(all(target_os = "windows", feature = "diagnostics"))]
pub fn run_host_view_events(app_id: String) -> ExitCode {
    run_host_with_parent(
        true,
        None,
        true,
        false,
        false,
        Some(ViewEventCallbackResult::Success),
        Some(app_id),
        HostReadiness(None),
    )
}

/// Runs the view-event probe while returning E_NOTIMPL from its callback.
/// twinui.pcshell then skips ConnectedStandbyHelper::HandleViewCreated.
#[cfg(all(target_os = "windows", feature = "diagnostics"))]
pub fn run_host_view_events_fail_callback() -> ExitCode {
    run_host_with_parent(
        true,
        None,
        true,
        false,
        false,
        Some(ViewEventCallbackResult::NotImplemented),
        Some("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App".to_string()),
        HostReadiness(None),
    )
}

/// Short-lived controller test for an injected RuntimeBroker diagnostic DLL.
/// The caller must first stop Explorer so this process can own GetShellWindow.
/// No band redirection, component filtering, or AutoPresent is installed.
#[cfg(all(target_os = "windows", feature = "diagnostics"))]
pub fn run_runtimebroker_controller_probe(log_path: &std::path::Path) -> u32 {
    use std::{
        fs::OpenOptions,
        io::Write,
        time::{Duration, Instant},
    };
    use windows::Win32::{
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, KillTimer, MSG, SetTimer, TranslateMessage, WM_TIMER,
        },
    };

    let log = |phase: &str| {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
            let _ = writeln!(file, "phase={phase}");
        }
    };
    log("runtimebroker-host-enter");
    let Some(shell_window) = ShellWindowGuard::register() else {
        log("runtimebroker-shell-window-failed");
        return 1;
    };
    log("runtimebroker-shell-window-ready");
    // SAFETY: This dedicated remote thread balances COM initialization below.
    if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
        log("runtimebroker-com-failed");
        return 2;
    }
    log("runtimebroker-controller-starting");
    let controller = match start_immersive_shell_controller(false, false, false) {
        Ok(controller) => controller,
        Err(error) => {
            log(&format!("runtimebroker-controller-failed error={error}"));
            // SAFETY: Balances this thread's successful COM initialization.
            unsafe { CoUninitialize() };
            return 3;
        }
    };
    log("runtimebroker-controller-ready");
    // SAFETY: This thread owns the shell window and pumps its message queue.
    if unsafe { SetTimer(Some(shell_window.window), 0x72, 250, None) } == 0 {
        log("runtimebroker-timer-failed");
        drop(controller);
        unsafe { CoUninitialize() };
        return 4;
    }
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
        if message.hwnd == shell_window.window
            && message.message == WM_TIMER
            && message.wParam.0 == 0x72
        {
            if Instant::now() >= deadline {
                break;
            }
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    let _ = unsafe { KillTimer(Some(shell_window.window), 0x72) };
    log("runtimebroker-controller-stopping");
    drop(controller);
    // SAFETY: Balances this thread's successful COM initialization.
    unsafe { CoUninitialize() };
    log("runtimebroker-controller-stopped");
    0
}

/// Runs the shell controller on an STA owned by the main Nickel process.
#[cfg(target_os = "windows")]
pub fn run_embedded_host(
    ready: std::sync::mpsc::SyncSender<Result<(), String>>,
    configure_process_dpi: bool,
) -> ExitCode {
    run_host_with_parent(
        true,
        None,
        configure_process_dpi,
        false,
        true,
        None,
        None,
        HostReadiness(Some(ready)),
    )
}

/// Runs the shell controller for a Nickel process. The open process handle
/// prevents PID reuse from making an orphaned host appear owned by Nickel.
#[cfg(target_os = "windows")]
pub fn run_managed_host(parent_pid: u32) -> ExitCode {
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};

    if parent_pid == 0 {
        eprintln!("phase=host-parent error=invalid-pid");
        return ExitCode::FAILURE;
    }
    // SAFETY: Windows validates the PID and requested right. The handle is
    // kept open until the host stops, then closed by ParentHandle::drop.
    let parent = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, parent_pid) } {
        Ok(handle) => ParentHandle(handle),
        Err(error) => {
            eprintln!("phase=host-parent error={error}");
            return ExitCode::FAILURE;
        }
    };
    run_host_with_parent(
        true,
        Some(parent),
        true,
        false,
        true,
        None,
        None,
        HostReadiness(None),
    )
}

#[cfg(target_os = "windows")]
struct ParentHandle(windows::Win32::Foundation::HANDLE);

#[cfg(target_os = "windows")]
impl ParentHandle {
    fn exited(&self) -> bool {
        use windows::Win32::{Foundation::WAIT_TIMEOUT, System::Threading::WaitForSingleObject};
        // SAFETY: This guard owns a process handle with SYNCHRONIZE access.
        unsafe { WaitForSingleObject(self.0, 0) != WAIT_TIMEOUT }
    }
}

#[cfg(target_os = "windows")]
impl Drop for ParentHandle {
    fn drop(&mut self) {
        // SAFETY: This guard owns exactly one open process handle.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(target_os = "windows")]
#[allow(clippy::too_many_arguments)]
fn run_host_with_parent(
    pump_messages: bool,
    parent: Option<ParentHandle>,
    configure_process_dpi: bool,
    publish_broker: bool,
    auto_present: bool,
    trace_view_events: Option<ViewEventCallbackResult>,
    view_event_app: Option<String>,
    mut readiness: HostReadiness,
) -> ExitCode {
    use std::io::Write;
    use windows::Win32::{
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, KillTimer, MSG, SetTimer, TranslateMessage, WM_TIMER,
        },
    };

    if configure_process_dpi && let Err(error) = configure_dpi_awareness() {
        eprintln!("phase=host-dpi error={error}");
        readiness.failed("host-dpi", error);
        return ExitCode::FAILURE;
    }
    let mut presenter = auto_present::AutoPresent::new();
    let Some(shell_window) = ShellWindowGuard::register() else {
        readiness.failed("host-shell-window", "shell window registration failed");
        return ExitCode::FAILURE;
    };
    // SAFETY: This thread balances successful COM initialization below.
    if let Err(error) = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok() {
        eprintln!("phase=host-com error={error}");
        readiness.failed("host-com", error);
        return ExitCode::FAILURE;
    }
    let redirect = match fallback_band1::FallbackBandRedirect::install() {
        Ok(redirect) => redirect,
        Err(error) => {
            eprintln!("phase=host-band-redirect error={error}");
            readiness.failed("host-band-redirect", error);
            unsafe { CoUninitialize() };
            return ExitCode::FAILURE;
        }
    };
    let controller = match start_immersive_shell_controller(false, false, true) {
        Ok(controller) => controller,
        Err(error) => {
            eprintln!("phase=host-controller error={error}");
            readiness.failed("host-controller", error);
            drop(redirect);
            unsafe { CoUninitialize() };
            return ExitCode::FAILURE;
        }
    };
    #[cfg(feature = "diagnostics")]
    let frame_pool = if trace_view_events.is_some() {
        const IMMERSIVE_APPLICATION_MANAGER: windows_core::GUID =
            windows_core::GUID::from_u128(0x50fdbb99_5c92_495e_9e81_e2c2f48cddae);
        let manager = match unsafe {
            windows::Win32::System::Com::CoCreateInstance::<_, windows::core::IUnknown>(
                &IMMERSIVE_APPLICATION_MANAGER,
                None,
                windows::Win32::System::Com::CLSCTX_INPROC_SERVER,
            )
        } {
            Ok(manager) => manager,
            Err(error) => {
                eprintln!("phase=view-event-frame-manager error={error}");
                drop(controller);
                drop(redirect);
                unsafe { CoUninitialize() };
                return ExitCode::FAILURE;
            }
        };
        match crate::frame_service_direct::ensure_frame_pool(&manager) {
            Ok(pool) => {
                println!("phase=view-event-frame-pool result=ready");
                Some((manager, pool))
            }
            Err(error) => {
                eprintln!("phase=view-event-frame-pool error={error}");
                drop(manager);
                drop(controller);
                drop(redirect);
                unsafe { CoUninitialize() };
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    let broker = if publish_broker {
        match publish_immersive_shell_broker() {
            Ok(broker) => Some(broker),
            Err(error) => {
                eprintln!("phase=broker-host error={error}");
                readiness.failed("broker-host", error);
                #[cfg(feature = "diagnostics")]
                drop(frame_pool);
                drop(controller);
                drop(redirect);
                unsafe { CoUninitialize() };
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    #[cfg(feature = "diagnostics")]
    let mut view_event_presentation = None;
    #[cfg(feature = "diagnostics")]
    let view_event_timeout_at = trace_view_events.map(|_| Instant::now() + Duration::from_secs(30));
    #[cfg(feature = "diagnostics")]
    let mut view_event_exit_at = None;
    #[cfg(feature = "diagnostics")]
    let mut view_event_frame_hwnd = None;
    #[cfg(feature = "diagnostics")]
    let mut view_event_foreground_complete = false;
    #[cfg(feature = "diagnostics")]
    let mut view_event_core_hwnd = None;
    #[cfg(feature = "diagnostics")]
    let mut view_event_frame_proxy: Option<windows::core::IUnknown> = None;
    #[cfg(feature = "diagnostics")]
    let mut view_event_app_pid = None;
    #[cfg(feature = "diagnostics")]
    let mut view_event_trace = if let Some(callback_result) = trace_view_events {
        let app_id = view_event_app
            .as_deref()
            .unwrap_or("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App");
        match crate::view_event_trace::ViewEventTrace::register(callback_result, app_id) {
            Ok(trace) => Some(trace),
            Err(error) => {
                eprintln!("phase=view-event-register error={error}");
                None
            }
        }
    } else {
        None
    };
    #[cfg(not(feature = "diagnostics"))]
    let _ = trace_view_events;
    println!("phase=host-ready");
    let _ = std::io::stdout().flush();
    #[cfg(feature = "diagnostics")]
    if trace_view_events.is_some()
        && let Some(app_id) = view_event_app.as_deref()
    {
        let probe = std::env::current_exe()
            .ok()
            .map(|path| path.with_file_name("nickel-windows-app-probe.exe"));
        match probe.and_then(|probe| std::process::Command::new(probe).arg(app_id).spawn().ok()) {
            Some(child) => println!("phase=view-event-test-launch pid={}", child.id()),
            None => eprintln!("phase=view-event-test-launch error=spawn-failed"),
        }
        let _ = std::io::stdout().flush();
    }
    #[cfg(feature = "diagnostics")]
    let keep_alive = (broker, controller, redirect);
    #[cfg(not(feature = "diagnostics"))]
    let keep_alive = (broker, controller, redirect);
    if pump_messages {
        // SAFETY: This thread owns the HWND and consumes its timer messages.
        if unsafe { SetTimer(Some(shell_window.window), 0x71, 250, None) } == 0 {
            eprintln!("phase=auto-present error=SetTimer-failed");
            readiness.failed("auto-present", "SetTimer failed");
            drop(keep_alive);
            // SAFETY: Balances this thread's successful CoInitializeEx.
            unsafe { CoUninitialize() };
            return ExitCode::FAILURE;
        }
        println!("phase=host-message-loop");
        readiness.ready();
        let mut message = MSG::default();
        // SAFETY: The shell window belongs to this thread, and the message
        // structure remains valid through each dispatch.
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
            if message.hwnd == shell_window.window
                && message.message == WM_TIMER
                && message.wParam.0 == 0x71
            {
                if parent.as_ref().is_some_and(ParentHandle::exited) {
                    println!("phase=host-parent result=exited");
                    break;
                }
                #[cfg(feature = "diagnostics")]
                if view_event_exit_at.is_some_and(|deadline| Instant::now() >= deadline) {
                    if let Some(process_id) = view_event_app_pid {
                        match terminate_view_event_app(process_id) {
                            Ok(()) => println!(
                                "phase=view-event-cleanup pid={process_id} result=terminated"
                            ),
                            Err(error) => {
                                println!("phase=view-event-cleanup pid={process_id} error={error}")
                            }
                        }
                    }
                    println!("phase=view-event-one-shot result=complete");
                    break;
                }
                #[cfg(feature = "diagnostics")]
                if view_event_exit_at.is_none()
                    && view_event_timeout_at.is_some_and(|deadline| Instant::now() >= deadline)
                {
                    if let Some(process_id) = view_event_app_pid {
                        match terminate_view_event_app(process_id) {
                            Ok(()) => println!(
                                "phase=view-event-cleanup pid={process_id} result=terminated"
                            ),
                            Err(error) => {
                                println!("phase=view-event-cleanup pid={process_id} error={error}")
                            }
                        }
                    }
                    println!("phase=view-event-one-shot result=timeout");
                    break;
                }
                if auto_present {
                    presenter.tick();
                }
                #[cfg(feature = "diagnostics")]
                if let Some(frame) = view_event_trace
                    .as_mut()
                    .and_then(crate::view_event_trace::ViewEventTrace::poll_frame_hwnd)
                {
                    println!("phase=view-event-frame-ready hwnd={frame:#x}");
                    view_event_frame_hwnd = Some(frame);
                    log_view_event_window_state("frame", frame);
                    if let Some(core) = view_event_core_hwnd {
                        log_view_event_window_state("core", core);
                    }
                    if view_event_foreground_complete {
                        view_event_exit_at = Some(Instant::now() + Duration::from_secs(10));
                    }
                }
                #[cfg(feature = "diagnostics")]
                if let Some((
                    wrapper,
                    interface,
                    cloak_interface,
                    frame_window_interface,
                    hwnd,
                    process_id,
                )) = view_event_trace
                    .as_mut()
                    .and_then(crate::view_event_trace::ViewEventTrace::poll_core_target)
                {
                    println!("phase=view-event-core-ready interface={interface:#x} hwnd={hwnd:#x}");
                    view_event_core_hwnd = Some(hwnd);
                    view_event_app_pid = Some(process_id);
                    match probe_view_wrapper_discovery(interface, hwnd) {
                        Ok(()) => {
                            println!("phase=view-event-discovery result=called");
                            view_event_presentation = Some((
                                wrapper,
                                interface,
                                cloak_interface,
                                frame_window_interface,
                                hwnd,
                                process_id,
                            ));
                        }
                        Err(error) => println!("phase=view-event-discovery error={error}"),
                    }
                }
                #[cfg(feature = "diagnostics")]
                if let Some((
                    wrapper,
                    interface,
                    cloak_interface,
                    frame_window_interface,
                    hwnd,
                    process_id,
                )) = view_event_presentation
                {
                    if view_event_frame_proxy.is_none() {
                        let app_id = view_event_app.as_deref().unwrap_or_default();
                        let acquired = match frame_pool.as_ref() {
                            Some((_, pool)) => pool.acquire_frame(app_id),
                            None => Err("frame pool unavailable".to_string()),
                        };
                        match acquired {
                            Ok(proxy) => {
                                println!("phase=view-event-get-frame result=called");
                                view_event_frame_proxy = Some(proxy);
                            }
                            Err(error) => println!("phase=view-event-get-frame error={error}"),
                        }
                    }
                    if let Some(proxy) = view_event_frame_proxy.as_ref() {
                        let app_id = view_event_app.as_deref().unwrap_or_default();
                        match probe_frame_set_presented_window(
                            proxy.as_raw() as usize,
                            hwnd,
                            app_id,
                        ) {
                            Ok(frame_hwnd) => {
                                println!(
                                    "phase=view-event-set-presented-window frame={frame_hwnd:#x} core={hwnd:#x} result=called"
                                );
                                match probe_view_wrapper_set_frame(
                                    frame_window_interface,
                                    frame_hwnd,
                                ) {
                                    Ok(()) => println!(
                                        "phase=view-event-set-frame hwnd={frame_hwnd:#x} result=called"
                                    ),
                                    Err(error) => println!(
                                        "phase=view-event-set-frame hwnd={frame_hwnd:#x} error={error}"
                                    ),
                                }
                                match probe_frame_set_position(proxy.as_raw() as usize) {
                                    Ok(()) => {}
                                    Err(error) => println!(
                                        "phase=view-event-set-position hwnd={hwnd:#x} error={error}"
                                    ),
                                }
                                match probe_frame_fit_to_work_area(proxy.as_raw() as usize) {
                                    Ok(()) => println!(
                                        "phase=view-event-fit-frame hwnd={frame_hwnd:#x} result=called"
                                    ),
                                    Err(error) => println!(
                                        "phase=view-event-fit-frame hwnd={frame_hwnd:#x} error={error}"
                                    ),
                                }
                                match probe_view_wrapper_set_size(wrapper, frame_hwnd) {
                                    Ok(()) => {}
                                    Err(error) => println!(
                                        "phase=view-event-set-size hwnd={hwnd:#x} error={error}"
                                    ),
                                }
                            }
                            Err(error) => {
                                println!("phase=view-event-set-presented-window error={error}")
                            }
                        }
                    }
                    match probe_view_wrapper_readiness(interface, hwnd) {
                        Ok(()) => println!("phase=view-event-readiness result=called"),
                        Err(error) => println!("phase=view-event-readiness error={error}"),
                    }
                    match probe_view_wrapper_uncloak(cloak_interface) {
                        Ok(()) => {
                            println!(
                                "phase=view-event-uncloak hwnd={hwnd:#x} pid={process_id} result=called"
                            );
                            let app_id = view_event_app.as_deref().unwrap_or_default();
                            let frame = crate::auto_present::frame_window_for_app(app_id);
                            let parent = unsafe {
                                windows::Win32::UI::WindowsAndMessaging::GetParent(
                                    windows::Win32::Foundation::HWND(hwnd as *mut _),
                                )
                            }
                            .ok()
                            .map(|window| window.0 as usize)
                            .unwrap_or_default();
                            println!(
                                "phase=view-event-frame-state app={app_id} frame={frame:?} core={hwnd:#x} parent={parent:#x}"
                            );
                            println!(
                                "phase=view-event-frame-windows values={:?}",
                                crate::auto_present::frame_windows()
                            );
                            match probe_view_wrapper_foreground(cloak_interface) {
                                Ok(()) => {
                                    println!(
                                        "phase=view-event-foreground hwnd={hwnd:#x} result=called"
                                    );
                                }
                                Err(error) => println!(
                                    "phase=view-event-foreground hwnd={hwnd:#x} error={error}"
                                ),
                            }
                            view_event_presentation = None;
                            view_event_foreground_complete = true;
                            if frame.is_some() || view_event_frame_hwnd.is_some() {
                                view_event_exit_at = Some(Instant::now() + Duration::from_secs(10));
                            }
                        }
                        Err(error) => {
                            println!(
                                "phase=view-event-uncloak hwnd={hwnd:#x} pid={process_id} error={error}"
                            )
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
        // SAFETY: Cancel the timer before the window guard destroys its HWND.
        let _ = unsafe { KillTimer(Some(shell_window.window), 0x71) };
    } else {
        // The one-shot probe kept this thread blocked during activation.
        loop {
            std::thread::park();
        }
    }
    drop(keep_alive);
    #[cfg(feature = "diagnostics")]
    drop(frame_pool);
    // SAFETY: COM references are released before leaving this apartment.
    unsafe { CoUninitialize() };
    ExitCode::SUCCESS
}

#[cfg(target_os = "windows")]
fn publish_immersive_shell_broker() -> windows::core::Result<windows::core::IUnknown> {
    use std::ffi::c_void;
    use windows::{
        Win32::System::Com::{
            CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, CoCreateInstance, IServiceProvider,
        },
        core::{GUID, HRESULT, IUnknown, Interface},
    };

    const IMMERSIVE_SHELL: GUID = GUID::from_u128(0xc2f03a33_21f5_47fa_b4bb_156362a2f239);
    const BROKER_HOST: GUID = GUID::from_u128(0x4075b76f_fcdc_43b6_b0b9_5a005b38b335);
    type PublishServices =
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT;

    // SAFETY: The shell controller has already published ImmersiveShell for
    // this process. Both CoCreateInstance calls return owned COM references.
    let shell: IServiceProvider =
        unsafe { CoCreateInstance(&IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER) }?;
    let host: IUnknown = unsafe { CoCreateInstance(&BROKER_HOST, None, CLSCTX_INPROC_SERVER) }?;
    // SAFETY: This build's PDB places PublishServices at slot 3 of the host's
    // primary IImmersiveShellComponent vtable. Disassembly shows that its
    // IProfferService argument is unused; the IServiceProvider is retained by
    // the broker class factory. The host stays alive until the fixture exits.
    let publish: PublishServices = unsafe {
        let vtable = host.as_raw().cast::<*const usize>().read();
        std::mem::transmute(vtable.add(3).read())
    };
    let status = unsafe { publish(host.as_raw(), shell.as_raw(), std::ptr::null_mut()) };
    println!("phase=broker-host-publish hresult={status:?}");
    status.ok()?;
    Ok(host)
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
