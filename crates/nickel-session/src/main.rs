#![allow(irrefutable_let_patterns)]

mod handlers;

mod backend;
mod grabs;
mod input;
mod login_services;
mod shell_layout;
mod state;
mod test_input;
mod window_frame;
mod window_registry;
#[cfg(feature = "backend-winit")]
mod winit;

use std::{
    ffi::OsString,
    process::{Command, ExitStatus},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use smithay::reexports::{
    calloop::EventLoop,
    wayland_server::{Display, DisplayHandle},
};
pub use state::NickelSession;

pub struct CalloopData {
    state: NickelSession,
    display_handle: DisplayHandle,
    #[cfg(feature = "backend-udev")]
    event_loop_handle: smithay::reexports::calloop::LoopHandle<'static, CalloopData>,
    #[cfg(feature = "backend-udev")]
    native: Option<backend::udev::UdevData>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }

    let arguments = backend::SessionArguments::parse(std::env::args_os().skip(1))?;
    let mut event_loop: EventLoop<'static, CalloopData> = EventLoop::try_new()?;

    let display: Display<NickelSession> = Display::new()?;
    let display_handle = display.handle();
    let state = NickelSession::new(&mut event_loop, display, arguments.test_control);

    let mut data = CalloopData {
        state,
        display_handle,
        #[cfg(feature = "backend-udev")]
        event_loop_handle: event_loop.handle(),
        #[cfg(feature = "backend-udev")]
        native: None,
    };

    let (shell_health_tx, shell_health_rx) = smithay::reexports::calloop::channel::channel();
    event_loop
        .handle()
        .insert_source(shell_health_rx, |event, _, data| {
            let smithay::reexports::calloop::channel::Event::Msg(failures) = event else {
                return;
            };
            data.state.shell_failure_count = failures;
            data.state.request_output_redraw();
            #[cfg(feature = "backend-udev")]
            if data.native.is_some() {
                data.render_all_outputs();
            }
        })?;

    match arguments.backend {
        backend::BackendKind::Winit => {
            #[cfg(feature = "backend-winit")]
            backend::winit::init_winit(&mut event_loop, &mut data)?;
            #[cfg(not(feature = "backend-winit"))]
            unreachable!("backend availability was validated while parsing arguments");
        }
        backend::BackendKind::Udev => {
            #[cfg(feature = "backend-udev")]
            backend::udev::init_udev(&mut event_loop, &mut data)?;
            #[cfg(not(feature = "backend-udev"))]
            unreachable!("backend availability was validated while parsing arguments");
        }
    }

    println!(
        "nickel-session listening on {}",
        data.state.socket_name.to_string_lossy()
    );

    if arguments.backend == backend::BackendKind::Udev {
        login_services::hand_off_login_credentials();
        import_runtime_environment();
    }

    if let Some((program, command_arguments)) = arguments.command {
        spawn_supervised(
            program,
            command_arguments,
            data.state.secure_storage_state_handle(),
            data.state.secure_storage_retry_handle(),
            data.state.expected_shell_pid_handle(),
            shell_health_tx,
        )?;
    }

    event_loop.run(None, &mut data, move |_| {
        // NickelSession is running
    })?;

    Ok(())
}

const USER_SESSION_ENVIRONMENT: &[&str] = &[
    "DBUS_SESSION_BUS_ADDRESS",
    "DISPLAY",
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

const MAX_SHELL_RESTART_DELAY: Duration = Duration::from_secs(4);
const STABLE_SHELL_RUNTIME: Duration = Duration::from_secs(30);

pub(crate) fn shell_recovery_visible_for(failures: u8) -> bool {
    failures >= 3
}

fn shell_restart_delay(consecutive_failures: usize) -> Duration {
    Duration::from_secs(
        consecutive_failures
            .max(1)
            .min(MAX_SHELL_RESTART_DELAY.as_secs() as usize) as u64,
    )
}

fn spawn_supervised(
    program: OsString,
    arguments: Vec<OsString>,
    secure_storage_state: Arc<AtomicU8>,
    secure_storage_retry: Arc<AtomicBool>,
    expected_shell_pid: Arc<AtomicU32>,
    shell_health: smithay::reexports::calloop::channel::Sender<u8>,
) -> std::io::Result<()> {
    thread::Builder::new()
        .name("nickel-shell-supervisor".into())
        .spawn(move || {
            supervise_shell(
                program,
                arguments,
                secure_storage_state,
                secure_storage_retry,
                expected_shell_pid,
                shell_health,
            )
        })
        .map(|_| ())
}

fn supervise_shell(
    program: OsString,
    arguments: Vec<OsString>,
    secure_storage_state: Arc<AtomicU8>,
    secure_storage_retry: Arc<AtomicBool>,
    expected_shell_pid: Arc<AtomicU32>,
    shell_health: smithay::reexports::calloop::channel::Sender<u8>,
) {
    if let Err(error) = thread::Builder::new()
        .name("nickel-login-services".into())
        .spawn(move || {
            let mut previous = None;
            login_services::monitor_secure_storage(secure_storage_retry, |state| {
                secure_storage_state.store(state as u8, Ordering::Release);
                if previous != Some(state) {
                    tracing::info!(state = state.as_str(), "secure storage state changed");
                    previous = Some(state);
                }
            });
        })
    {
        tracing::error!(%error, "failed to start secure-storage preparation");
    }

    let mut consecutive_failures = 0_usize;
    loop {
        let started = Instant::now();
        let status = match Command::new(&program).args(&arguments).spawn() {
            Ok(mut child) => {
                expected_shell_pid.store(child.id(), Ordering::Release);
                let result = child.wait();
                expected_shell_pid.store(0, Ordering::Release);
                result
            }
            Err(error) => Err(error),
        };
        let runtime = started.elapsed();
        if runtime >= STABLE_SHELL_RUNTIME || status.as_ref().is_ok_and(ExitStatus::success) {
            consecutive_failures = 0;
        } else {
            consecutive_failures = consecutive_failures.saturating_add(1);
        }
        match status {
            Ok(status) if status.success() => {
                tracing::info!(?status, "Nickel shell exited normally; restarting");
            }
            Ok(status) => {
                tracing::error!(
                    ?status,
                    consecutive_failures,
                    "Nickel shell exited unexpectedly"
                );
            }
            Err(error) => {
                tracing::error!(%error, consecutive_failures, "failed to start Nickel shell");
            }
        }
        let _ = shell_health.send(u8::try_from(consecutive_failures).unwrap_or(u8::MAX));
        thread::sleep(shell_restart_delay(consecutive_failures));
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{USER_SESSION_ENVIRONMENT, shell_restart_delay};

    #[test]
    fn shell_restart_backoff_is_bounded_without_ending_the_session() {
        assert_eq!(shell_restart_delay(1), Duration::from_secs(1));
        assert_eq!(shell_restart_delay(3), Duration::from_secs(3));
        assert_eq!(shell_restart_delay(99), Duration::from_secs(4));
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
            "WAYLAND_DISPLAY",
            "XDG_CURRENT_DESKTOP",
            "XDG_RUNTIME_DIR",
            "XDG_SESSION_TYPE",
        ] {
            assert!(USER_SESSION_ENVIRONMENT.contains(&variable));
        }
    }
}
