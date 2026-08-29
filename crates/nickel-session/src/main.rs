#![allow(irrefutable_let_patterns)]

mod handlers;

mod backend;
mod focus;
mod grabs;
mod input;
mod login_services;
mod session_services;
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
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
pub use state::NickelSession;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }

    let arguments = backend::SessionArguments::parse(std::env::args_os().skip(1))?;
    let mut event_loop: EventLoop<'static, NickelSession> = EventLoop::try_new()?;

    let display: Display<NickelSession> = Display::new()?;
    let mut state = NickelSession::new(&mut event_loop, display, arguments.test_control);

    let (shell_health_tx, shell_health_rx) = smithay::reexports::calloop::channel::channel();
    event_loop
        .handle()
        .insert_source(shell_health_rx, |event, _, data| {
            let smithay::reexports::calloop::channel::Event::Msg(failures) = event else {
                return;
            };
            data.shell_failure_count = failures;
            data.request_output_redraw();
            #[cfg(feature = "backend-udev")]
            if data.native.is_some() {
                data.render_all_outputs();
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

    state.start_xwayland();

    println!(
        "nickel-session listening on {}",
        state.socket_name.to_string_lossy()
    );

    if arguments.backend == backend::BackendKind::Udev {
        login_services::hand_off_login_credentials();
        import_runtime_environment();
    }

    let (supervisor_tx, supervisor_rx) = mpsc::channel();
    state.set_shell_supervisor(supervisor_tx.clone());
    let shell_supervisor = if let Some((program, command_arguments)) = arguments.command {
        Some(spawn_supervised(
            program,
            command_arguments,
            state.secure_storage_state_handle(),
            state.secure_storage_retry_handle(),
            state.expected_shell_pid_handle(),
            shell_health_tx,
            supervisor_rx,
        )?)
    } else {
        None
    };

    event_loop.run(None, &mut state, move |_| {
        // NickelSession is running
    })?;

    let _ = supervisor_tx.send(ShellSupervisorCommand::Stop);
    if let Some(supervisor) = shell_supervisor {
        let _ = supervisor.join();
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShellSupervisorCommand {
    Restart,
    Stop,
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
    supervisor: mpsc::Receiver<ShellSupervisorCommand>,
) -> std::io::Result<thread::JoinHandle<()>> {
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
                supervisor,
            )
        })
}

fn supervise_shell(
    program: OsString,
    arguments: Vec<OsString>,
    secure_storage_state: Arc<AtomicU8>,
    secure_storage_retry: Arc<AtomicBool>,
    expected_shell_pid: Arc<AtomicU32>,
    shell_health: smithay::reexports::calloop::channel::Sender<u8>,
    supervisor: mpsc::Receiver<ShellSupervisorCommand>,
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
        let status = match shell_command(&program, &arguments).spawn() {
            Ok(mut child) => {
                expected_shell_pid.store(child.id(), Ordering::Release);
                let result = wait_for_shell(&mut child, &supervisor);
                expected_shell_pid.store(0, Ordering::Release);
                match result {
                    ShellWait::Exited(status) => status,
                    ShellWait::Restarted => {
                        consecutive_failures = 0;
                        let _ = shell_health.send(0);
                        continue;
                    }
                    ShellWait::Stopped => return,
                }
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

fn shell_command(program: &OsString, arguments: &[OsString]) -> Command {
    use std::os::unix::process::CommandExt;

    let mut command = Command::new(program);
    command.args(arguments).process_group(0);
    command
}

enum ShellWait {
    Exited(std::io::Result<ExitStatus>),
    Restarted,
    Stopped,
}

fn wait_for_shell(
    child: &mut std::process::Child,
    supervisor: &mpsc::Receiver<ShellSupervisorCommand>,
) -> ShellWait {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return ShellWait::Exited(Ok(status)),
            Ok(None) => {}
            Err(error) => return ShellWait::Exited(Err(error)),
        }
        match supervisor.recv_timeout(Duration::from_millis(100)) {
            Ok(command) => {
                terminate_shell_group(child);
                return match command {
                    ShellSupervisorCommand::Restart => ShellWait::Restarted,
                    ShellSupervisorCommand::Stop => ShellWait::Stopped,
                };
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                terminate_shell_group(child);
                return ShellWait::Stopped;
            }
        }
    }
}

fn terminate_shell_group(child: &mut std::process::Child) {
    use nix::{
        sys::signal::{Signal, kill},
        unistd::Pid,
    };

    let group = Pid::from_raw(-(child.id() as i32));
    let _ = kill(group, Signal::SIGTERM);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = kill(group, Signal::SIGKILL);
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, sync::mpsc, time::Duration};

    use super::{
        ShellSupervisorCommand, ShellWait, USER_SESSION_ENVIRONMENT, shell_command,
        shell_restart_delay, wait_for_shell,
    };

    #[test]
    fn shell_restart_backoff_is_bounded_without_ending_the_session() {
        assert_eq!(shell_restart_delay(1), Duration::from_secs(1));
        assert_eq!(shell_restart_delay(3), Duration::from_secs(3));
        assert_eq!(shell_restart_delay(99), Duration::from_secs(4));
    }

    #[test]
    fn supervisor_stop_terminates_and_reaps_the_shell_process_group() {
        let mut child = shell_command(&OsString::from("sleep"), &[OsString::from("30")])
            .spawn()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        sender.send(ShellSupervisorCommand::Stop).unwrap();

        assert!(matches!(
            wait_for_shell(&mut child, &receiver),
            ShellWait::Stopped
        ));
        assert!(child.try_wait().unwrap().is_some());
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
