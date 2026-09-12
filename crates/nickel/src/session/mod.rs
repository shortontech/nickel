#![allow(irrefutable_let_patterns)]

mod handlers;

mod authority;
mod backend;
mod clipboard_transfer;
mod focus;
mod grabs;
mod input;
mod internal_ui;
pub(crate) mod login_services;
mod native_clipboard;
mod on_screen_keyboard;
#[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
mod output_identification;
mod output_retirement;
#[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
mod preview_submission;
mod recovery_ui;
mod remote_accessibility;
mod remote_identity;
pub(crate) use crate::remote_indicator;
mod session_services;
mod shell_layout;
mod state;
mod test_input;
mod window_frame;
mod window_registry;
#[cfg(feature = "backend-winit")]
mod winit;

#[cfg(target_os = "linux")]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub use authority::{SessionAuthority, SessionAuthorityRequest};
pub use internal_ui::{
    InternalSurfaceLayer, InternalSurfacePlacement, InternalSurfaceRole, InternalUiRuntime,
    TouchPhase,
};
use smithay::reexports::{
    calloop::{
        EventLoop,
        timer::{TimeoutAction, Timer},
    },
    wayland_server::Display,
};
pub(crate) use state::InternalCaptureState;
pub use state::NickelSession;

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--available-backends")) {
        if cfg!(feature = "backend-winit") {
            println!("winit");
        }
        if cfg!(feature = "backend-udev") {
            println!("udev");
        }
        return Ok(());
    }

    // The unified session bypasses the old standalone-shell entry point. Set
    // PipeWire's plugin search path before logging or any other worker can be
    // spawned, preserving the process-wide initialization contract.
    crate::platform::prepare_audio_environment();
    nickel_logging::init("nickel")?;

    let arguments = backend::SessionArguments::parse(std::env::args_os().skip(1))?;
    if !test_control_allowed(
        arguments.backend == backend::BackendKind::Udev,
        arguments.test_control,
        std::env::var_os("NICKEL_ALLOW_NATIVE_TEST_CONTROL").as_deref(),
    ) {
        return Err("native --test-control requires NICKEL_ALLOW_NATIVE_TEST_CONTROL=1".into());
    }
    let mut event_loop: EventLoop<'static, NickelSession> = EventLoop::try_new()?;

    let display: Display<NickelSession> = Display::new()?;
    let mut state = NickelSession::new(&mut event_loop, display, arguments.test_control);
    state.internal_ui.set_renderer_mode(arguments.ui_renderer);
    tracing::info!(
        renderer = ?arguments.ui_renderer,
        "compositor-owned UI renderer selected"
    );
    // Keep the typed internal command path live alongside the compatibility
    // socket. Compositor-hosted UI will receive this handle instead of the
    // platform transport when it is moved into `NickelSession`.
    let in_process_session_host = crate::session_host::install_in_process_session_host(
        &event_loop.handle(),
        state.secure_storage_state_handle(),
        state.secure_storage_retry_handle(),
        Arc::clone(&state.internal_projection_outputs),
        Arc::clone(&state.internal_capture),
        Arc::clone(&state.internal_keyboard_snapshot),
        state.remote_control.control(),
    )?;
    let secure_storage_required = secure_storage_required(
        arguments.backend == backend::BackendKind::Udev,
        std::env::var_os("NICKEL_SECURE_STORAGE_REQUIRED").as_deref(),
    );
    if arguments.backend == backend::BackendKind::Udev && !secure_storage_required {
        tracing::warn!(
            variable = "NICKEL_SECURE_STORAGE_REQUIRED",
            "secure storage startup gate disabled by environment"
        );
    }
    let secure_storage_may_start = Arc::new(AtomicBool::new(!secure_storage_required));
    let monitor_secure_storage_state = state.secure_storage_state_handle();
    let monitor_secure_storage_retry = state.secure_storage_retry_handle();
    let monitor_secure_storage_may_start = Arc::clone(&secure_storage_may_start);
    let (secure_storage_changed, secure_storage_changes) =
        smithay::reexports::calloop::channel::channel();
    event_loop
        .handle()
        .insert_source(secure_storage_changes, |event, _, state| {
            if let smithay::reexports::calloop::channel::Event::Msg(()) = event {
                state.refresh_internal_shell_system();
            }
        })?;
    thread::Builder::new()
        .name("nickel-login-services".into())
        .spawn(move || {
            wait_for_secure_storage_start(&monitor_secure_storage_may_start);
            let mut previous = None;
            login_services::monitor_secure_storage(monitor_secure_storage_retry, |storage_state| {
                monitor_secure_storage_state.store(storage_state as u8, Ordering::Release);
                // The old out-of-process shell discovered transitions through
                // its periodic control-socket query. The compositor-owned
                // shell is deadline-driven, so explicitly wake its event loop
                // after publishing the shared state instead of leaving its UI
                // and launch gating on the construction-time snapshot.
                let _ = secure_storage_changed.send(());
                if previous != Some(storage_state) {
                    tracing::info!(
                        state = storage_state.as_str(),
                        "secure storage state changed"
                    );
                    previous = Some(storage_state);
                }
            });
        })?;
    let secure_storage_started = Instant::now();
    event_loop.handle().insert_source(
        Timer::from_duration(Duration::from_secs(1)),
        move |_, _, state| {
            state.poll_idle_policy();
            let storage_state = state.secure_storage_state();
            if secure_storage_startup_timed_out(
                secure_storage_required,
                storage_state,
                secure_storage_started.elapsed(),
            ) {
                tracing::error!(
                    state = storage_state.as_str(),
                    "secure storage startup deadline expired; returning to display manager"
                );
                if let Err(error) = session_services::return_to_display_manager() {
                    tracing::error!(%error, "could not request the display-manager greeter");
                }
                state.loop_signal.stop();
                return TimeoutAction::Drop;
            }
            TimeoutAction::ToDuration(Duration::from_secs(1))
        },
    )?;

    let (controller_changed, controller_events) = smithay::reexports::calloop::channel::channel();
    event_loop
        .handle()
        .insert_source(controller_events, |event, _, state| {
            if let smithay::reexports::calloop::channel::Event::Msg((action, family)) = event {
                state.handle_native_controller_action(action, family);
            }
        })?;
    thread::Builder::new()
        .name("nickel-controller-events".into())
        .spawn(move || {
            let mut controller = nickel_ui::ControllerInput::new();
            loop {
                let actions = controller.wait_global(Duration::from_secs(1));
                let family = controller.active_family().unwrap_or_default();
                for action in actions {
                    if controller_changed.send((action, family)).is_err() {
                        return;
                    }
                }
            }
        })?;

    match arguments.backend {
        backend::BackendKind::Winit => {
            #[cfg(feature = "backend-winit")]
            backend::winit::init_winit(&mut event_loop, &mut state)?;
            #[cfg(not(feature = "backend-winit"))]
            unreachable!("backend availability was validated while parsing arguments");
        }
        backend::BackendKind::Udev => {
            #[cfg(feature = "backend-udev")]
            backend::udev::init_udev(&mut event_loop, &mut state)?;
            #[cfg(not(feature = "backend-udev"))]
            unreachable!("backend availability was validated while parsing arguments");
        }
    }

    state.enable_internal_shell(Arc::new(in_process_session_host.clone()))?;
    state.schedule_internal_shell_deadline();
    tracing::info!(
        surfaces = state.internal_ui.len(),
        "compositor-owned Nickel shell initialized"
    );

    if secure_storage_required {
        // KWallet's PAM child deliberately pauses before constructing its Qt
        // application until this handoff supplies the graphical environment.
        // Run the handoff from the live compositor loop so the authorized
        // client can immediately complete its Wayland connection.
        let secure_storage_may_start = Arc::clone(&secure_storage_may_start);
        event_loop.handle().insert_idle(move |_| {
            login_services::hand_off_login_credentials();
            secure_storage_may_start.store(true, Ordering::Release);
        });
    }

    state.start_xwayland();

    if arguments.test_control {
        let control = std::env::var_os("NICKEL_SESSION_CONTROL")
            .expect("explicit test control initialized without a socket path");
        println!(
            "nickel test control listening on {}",
            control.to_string_lossy()
        );
    }
    // Publishing the test capability is also the readiness barrier for the
    // external acceptance harness. Do not expose it until every backend and
    // internal-shell source has been installed and the event loop can service
    // control datagrams.
    publish_test_control_environment(arguments.test_control)?;

    if arguments.backend == backend::BackendKind::Udev {
        import_runtime_environment();
    }

    event_loop.run(None, &mut state, move |state| {
        state.log_shell_readiness_if_changed();
    })?;

    Ok(())
}

fn publish_test_control_environment(enabled: bool) -> std::io::Result<()> {
    let Some(path) = enabled
        .then(|| std::env::var_os("NICKEL_TEST_CONTROL_ENV_FILE"))
        .flatten()
    else {
        return Ok(());
    };
    let contents = TEST_CONTROL_ENVIRONMENT
        .into_iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| format!("{name}={value}\n"))
        })
        .collect::<String>();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    std::io::Write::write_all(&mut options.open(path)?, contents.as_bytes())
}

const TEST_CONTROL_ENVIRONMENT: [&str; 6] = [
    "XDG_RUNTIME_DIR",
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "NICKEL_SESSION_CONTROL",
    "NICKEL_SESSION_TOKEN",
    "NICKEL_SHELL_TEST_CONTROL",
];

const USER_SESSION_ENVIRONMENT: &[&str] = &[
    "DBUS_SESSION_BUS_ADDRESS",
    "DISPLAY",
    "KDE_SESSION_VERSION",
    "WAYLAND_DISPLAY",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_CURRENT_DESKTOP",
    "XDG_DATA_HOME",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_DESKTOP",
    "XDG_SESSION_TYPE",
    "XDG_STATE_HOME",
];
const SECURE_STORAGE_STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

fn import_runtime_environment() {
    match Command::new("dbus-update-activation-environment")
        .arg("--systemd")
        .args(USER_SESSION_ENVIRONMENT)
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => {
            tracing::warn!(?status, "user-session environment import failed");
        }
        Err(error) => {
            tracing::warn!(%error, "could not start user-session environment import");
        }
    }
}

fn secure_storage_startup_timed_out(
    required: bool,
    state: login_services::SecureStorageState,
    elapsed: Duration,
) -> bool {
    required
        && !matches!(
            state,
            login_services::SecureStorageState::Ready
                | login_services::SecureStorageState::PromptRequired
        )
        && elapsed >= SECURE_STORAGE_STARTUP_TIMEOUT
}

fn secure_storage_required(native_backend: bool, configured: Option<&std::ffi::OsStr>) -> bool {
    native_backend && configured != Some(std::ffi::OsStr::new("0"))
}

fn test_control_allowed(
    native_backend: bool,
    requested: bool,
    configured: Option<&std::ffi::OsStr>,
) -> bool {
    !native_backend || !requested || configured == Some(std::ffi::OsStr::new("1"))
}

pub(crate) fn shell_recovery_visible_for(failures: u8) -> bool {
    failures >= 3
}

fn wait_for_secure_storage_start(start: &AtomicBool) {
    while !start.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    use super::{
        TEST_CONTROL_ENVIRONMENT, USER_SESSION_ENVIRONMENT, secure_storage_required,
        secure_storage_startup_timed_out, test_control_allowed, wait_for_secure_storage_start,
    };

    #[test]
    fn login_services_wait_until_the_compositor_accepts_graphical_clients() {
        let may_start = Arc::new(AtomicBool::new(false));
        let waiting = Arc::clone(&may_start);
        let (finished_tx, finished_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            wait_for_secure_storage_start(&waiting);
            finished_tx.send(()).unwrap();
        });

        assert!(finished_rx.recv_timeout(Duration::from_millis(30)).is_err());
        may_start.store(true, Ordering::Release);
        finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn recovery_threshold_requires_repeated_failures() {
        assert!(!super::shell_recovery_visible_for(2));
        assert!(super::shell_recovery_visible_for(3));
    }

    #[test]
    fn runtime_import_covers_display_bus_and_xdg_service_authority() {
        for variable in [
            "DBUS_SESSION_BUS_ADDRESS",
            "KDE_SESSION_VERSION",
            "WAYLAND_DISPLAY",
            "XDG_CURRENT_DESKTOP",
            "XDG_RUNTIME_DIR",
            "XDG_SESSION_TYPE",
        ] {
            assert!(USER_SESSION_ENVIRONMENT.contains(&variable));
        }
    }

    #[test]
    fn test_control_publishes_both_native_client_displays() {
        assert!(TEST_CONTROL_ENVIRONMENT.contains(&"WAYLAND_DISPLAY"));
        assert!(TEST_CONTROL_ENVIRONMENT.contains(&"DISPLAY"));
    }

    #[test]
    fn native_storage_deadline_returns_to_sddm_but_preserves_provider_prompts() {
        use crate::session::login_services::SecureStorageState;

        assert!(!secure_storage_startup_timed_out(
            true,
            SecureStorageState::Locked,
            Duration::from_secs(14)
        ));
        assert!(secure_storage_startup_timed_out(
            true,
            SecureStorageState::Locked,
            Duration::from_secs(15)
        ));
        assert!(!secure_storage_startup_timed_out(
            true,
            SecureStorageState::PromptRequired,
            Duration::from_secs(300)
        ));
        assert!(!secure_storage_startup_timed_out(
            false,
            SecureStorageState::Unavailable,
            Duration::from_secs(300)
        ));
    }

    #[test]
    fn native_secure_storage_gate_requires_an_explicit_zero_to_disable() {
        use std::ffi::OsStr;

        assert!(secure_storage_required(true, None));
        assert!(secure_storage_required(true, Some(OsStr::new("1"))));
        assert!(secure_storage_required(true, Some(OsStr::new("false"))));
        assert!(!secure_storage_required(true, Some(OsStr::new("0"))));
        assert!(!secure_storage_required(false, None));
    }

    #[test]
    fn native_test_control_requires_an_explicit_one_to_enable() {
        use std::ffi::OsStr;

        assert!(!test_control_allowed(true, true, None));
        assert!(!test_control_allowed(true, true, Some(OsStr::new("0"))));
        assert!(test_control_allowed(true, true, Some(OsStr::new("1"))));
        assert!(test_control_allowed(true, false, None));
        assert!(test_control_allowed(false, true, None));
    }
}

#[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
mod window_capture;
