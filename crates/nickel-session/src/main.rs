#![allow(irrefutable_let_patterns)]

mod handlers;

mod backend;
mod focus;
mod grabs;
mod input;
mod login_services;
mod recovery_ui;
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

#[cfg(target_os = "linux")]
use std::io::Write;
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "linux")]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(target_os = "linux")]
use std::sync::{Condvar, Mutex, atomic::AtomicU64};

use smithay::reexports::{
    calloop::{
        EventLoop,
        timer::{TimeoutAction, Timer},
    },
    wayland_server::Display,
};
pub use state::NickelSession;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--available-backends")) {
        if cfg!(feature = "backend-winit") {
            println!("winit");
        }
        if cfg!(feature = "backend-udev") {
            println!("udev");
        }
        return Ok(());
    }

    nickel_logging::init("nickel-session")?;

    let arguments = backend::SessionArguments::parse(std::env::args_os().skip(1))?;
    let mut event_loop: EventLoop<'static, NickelSession> = EventLoop::try_new()?;

    let display: Display<NickelSession> = Display::new()?;
    let mut state = NickelSession::new(&mut event_loop, display, arguments.test_control);
    let secure_storage_required = arguments.backend == backend::BackendKind::Udev;
    let secure_storage_may_start = Arc::new(AtomicBool::new(!secure_storage_required));
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

    let (shell_health_tx, shell_health_rx) = smithay::reexports::calloop::channel::channel();
    event_loop
        .handle()
        .insert_source(shell_health_rx, |event, _, data| {
            let smithay::reexports::calloop::channel::Event::Msg(failures) = event else {
                return;
            };
            // Every health message follows a supervised shell exit, explicit restart, or
            // failed spawn. No preview interest from that shell generation survives it.
            data.retire_shell_preview_memory();
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

    println!(
        "nickel-session listening on {}",
        state.socket_name.to_string_lossy()
    );

    if arguments.backend == backend::BackendKind::Udev {
        import_runtime_environment();
    }

    let (supervisor_tx, supervisor_rx) = mpsc::channel();
    state.set_shell_supervisor(supervisor_tx.clone());
    let shell_supervisor = if let Some((program, command_arguments)) = arguments.command {
        Some(spawn_supervised(
            program,
            command_arguments,
            ShellSupervisorContext {
                secure_storage_state: state.secure_storage_state_handle(),
                secure_storage_retry: state.secure_storage_retry_handle(),
                secure_storage_may_start,
                expected_shell_pid: state.expected_shell_pid_handle(),
                secure_storage_required,
                shell_health: shell_health_tx,
                commands: supervisor_rx,
            },
        )?)
    } else {
        None
    };

    event_loop.run(None, &mut state, move |state| {
        state.log_shell_readiness_if_changed();
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

const MAX_SHELL_RESTART_DELAY: Duration = Duration::from_secs(4);
const STABLE_SHELL_RUNTIME: Duration = Duration::from_secs(30);

#[cfg(target_os = "linux")]
const SHELL_STARTUP_BARRIER_ENV: &str = "NICKEL_SHELL_STARTUP_BARRIER";
#[cfg(target_os = "linux")]
const SHELL_STARTUP_BARRIER_MAGIC: &[u8; 8] = b"NIKREADY";
#[cfg(target_os = "linux")]
static SHELL_STARTUP_BARRIER_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[cfg(target_os = "linux")]
struct ShellStartupBarrier {
    directory: std::path::PathBuf,
    path: std::path::PathBuf,
    state: Arc<(Mutex<Option<Option<u32>>>, Condvar)>,
    worker: Option<thread::JoinHandle<()>>,
}

#[cfg(target_os = "linux")]
impl ShellStartupBarrier {
    fn new() -> std::io::Result<Self> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| std::io::Error::other("XDG_RUNTIME_DIR is missing"))?;
        let directory = runtime.join(format!(
            "nickel-shell-startup-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            SHELL_STARTUP_BARRIER_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory)?;
        if let Err(error) =
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
        {
            let _ = std::fs::remove_dir(&directory);
            return Err(error);
        }
        let path = directory.join("ready.sock");
        let listener = match UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(error) => {
                let _ = std::fs::remove_dir(&directory);
                return Err(error);
            }
        };
        let state = Arc::new((Mutex::new(None::<Option<u32>>), Condvar::new()));
        let worker_state = Arc::clone(&state);
        let worker = thread::Builder::new()
            .name("nickel-shell-startup-barrier".into())
            .spawn(move || {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let (lock, condition) = &*worker_state;
                let mut state = lock.lock().expect("startup barrier state is not poisoned");
                while state.is_none() {
                    state = condition
                        .wait(state)
                        .expect("startup barrier state is not poisoned");
                }
                let Some(Some(pid)) = *state else {
                    return;
                };
                let mut token = Vec::with_capacity(SHELL_STARTUP_BARRIER_MAGIC.len() + 4);
                token.extend_from_slice(SHELL_STARTUP_BARRIER_MAGIC);
                token.extend_from_slice(&pid.to_ne_bytes());
                let _ = stream.write_all(&token);
            });
        let worker = match worker {
            Ok(worker) => worker,
            Err(error) => {
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::remove_dir(&directory);
                return Err(error);
            }
        };
        Ok(Self {
            directory,
            path,
            state,
            worker: Some(worker),
        })
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn release(&self, pid: u32) {
        let (lock, condition) = &*self.state;
        let mut state = lock.lock().expect("startup barrier state is not poisoned");
        if state.is_none() {
            *state = Some(Some(pid));
            condition.notify_one();
        }
    }

    fn stop(mut self) {
        let (lock, condition) = &*self.state;
        let mut state = lock.lock().expect("startup barrier state is not poisoned");
        if state.is_none() {
            *state = Some(None);
            condition.notify_one();
        }
        drop(state);
        // Wake accept() when the child failed before reaching the barrier.
        let _ = UnixStream::connect(&self.path);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(self.directory);
    }
}

struct ShellSupervisorContext {
    secure_storage_state: Arc<AtomicU8>,
    secure_storage_retry: Arc<AtomicBool>,
    secure_storage_may_start: Arc<AtomicBool>,
    expected_shell_pid: Arc<AtomicU32>,
    secure_storage_required: bool,
    shell_health: smithay::reexports::calloop::channel::Sender<u8>,
    commands: mpsc::Receiver<ShellSupervisorCommand>,
}

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
    context: ShellSupervisorContext,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("nickel-shell-supervisor".into())
        .spawn(move || supervise_shell(program, arguments, context))
}

fn supervise_shell(program: OsString, arguments: Vec<OsString>, context: ShellSupervisorContext) {
    let ShellSupervisorContext {
        secure_storage_state,
        secure_storage_retry,
        secure_storage_may_start,
        expected_shell_pid,
        secure_storage_required,
        shell_health,
        commands: supervisor,
    } = context;
    let monitor_secure_storage_state = Arc::clone(&secure_storage_state);
    let monitor_secure_storage_retry = Arc::clone(&secure_storage_retry);
    if let Err(error) = thread::Builder::new()
        .name("nickel-login-services".into())
        .spawn(move || {
            wait_for_secure_storage_start(&secure_storage_may_start);
            let mut previous = None;
            login_services::monitor_secure_storage(monitor_secure_storage_retry, |state| {
                monitor_secure_storage_state.store(state as u8, Ordering::Release);
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
        if secure_storage_required
            && !wait_for_secure_storage(&secure_storage_state, &secure_storage_retry, &supervisor)
        {
            return;
        }
        let started = Instant::now();
        #[cfg(target_os = "linux")]
        let startup_barrier = match ShellStartupBarrier::new() {
            Ok(barrier) => barrier,
            Err(error) => {
                tracing::error!(%error, "failed to create Nickel shell startup barrier");
                let _ = shell_health.send(u8::try_from(consecutive_failures).unwrap_or(u8::MAX));
                thread::sleep(shell_restart_delay(consecutive_failures));
                continue;
            }
        };
        let mut command = shell_command(&program, &arguments);
        #[cfg(target_os = "linux")]
        command.env(SHELL_STARTUP_BARRIER_ENV, startup_barrier.path());
        let status = match command.spawn() {
            Ok(mut child) => {
                expected_shell_pid.store(child.id(), Ordering::Release);
                #[cfg(target_os = "linux")]
                startup_barrier.release(child.id());
                let result = wait_for_shell(
                    &mut child,
                    &supervisor,
                    secure_storage_required.then_some(secure_storage_state.as_ref()),
                );
                expected_shell_pid.store(0, Ordering::Release);
                #[cfg(target_os = "linux")]
                startup_barrier.stop();
                match result {
                    ShellWait::Exited(status) => status,
                    ShellWait::Restarted => {
                        consecutive_failures = 0;
                        let _ = shell_health.send(0);
                        continue;
                    }
                    ShellWait::Stopped => return,
                    ShellWait::SecureStorageLost => {
                        consecutive_failures = 0;
                        tracing::warn!("secure storage readiness revoked; Nickel shell stopped");
                        let _ = shell_health.send(0);
                        continue;
                    }
                }
            }
            Err(error) => {
                #[cfg(target_os = "linux")]
                startup_barrier.stop();
                Err(error)
            }
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

fn wait_for_secure_storage_start(start: &AtomicBool) {
    while !start.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_secure_storage(
    state: &AtomicU8,
    retry: &AtomicBool,
    supervisor: &mpsc::Receiver<ShellSupervisorCommand>,
) -> bool {
    loop {
        if state.load(Ordering::Acquire) == login_services::SecureStorageState::Ready as u8 {
            return true;
        }
        match supervisor.recv_timeout(Duration::from_millis(100)) {
            Ok(ShellSupervisorCommand::Restart) => retry.store(true, Ordering::Release),
            Ok(ShellSupervisorCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                return false;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn shell_command(program: &OsString, arguments: &[OsString]) -> Command {
    use std::os::unix::process::CommandExt;

    let mut command = Command::new(program);
    command
        .args(arguments)
        // XWayland intentionally exports DISPLAY for ordinary applications.
        // Winit selects Wayland whenever WAYLAND_DISPLAY is present, even when
        // DISPLAY is also available, so no private backend override is needed.
        .process_group(0);
    command
}

enum ShellWait {
    Exited(std::io::Result<ExitStatus>),
    Restarted,
    Stopped,
    SecureStorageLost,
}

fn wait_for_shell(
    child: &mut std::process::Child,
    supervisor: &mpsc::Receiver<ShellSupervisorCommand>,
    secure_storage_state: Option<&AtomicU8>,
) -> ShellWait {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return ShellWait::Exited(Ok(status)),
            Ok(None) => {}
            Err(error) => return ShellWait::Exited(Err(error)),
        }
        if secure_storage_state.is_some_and(|state| {
            state.load(Ordering::Acquire) != login_services::SecureStorageState::Ready as u8
        }) {
            terminate_shell_group(child);
            return ShellWait::SecureStorageLost;
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
    use std::{
        ffi::OsString,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU8, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    use super::{
        ShellSupervisorCommand, ShellWait, USER_SESSION_ENVIRONMENT,
        secure_storage_startup_timed_out, shell_command, shell_restart_delay,
        wait_for_secure_storage, wait_for_secure_storage_start, wait_for_shell,
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
            wait_for_shell(&mut child, &receiver, None,),
            ShellWait::Stopped
        ));
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn shell_start_gate_waits_for_authoritative_secure_storage_readiness() {
        let state = Arc::new(AtomicU8::new(
            crate::login_services::SecureStorageState::Locked as u8,
        ));
        let retry = Arc::new(AtomicBool::new(false));
        let (_sender, receiver) = mpsc::channel();
        let gate_state = Arc::clone(&state);
        let gate_retry = Arc::clone(&retry);
        let gate =
            thread::spawn(move || wait_for_secure_storage(&gate_state, &gate_retry, &receiver));

        thread::sleep(Duration::from_millis(150));
        assert!(
            !gate.is_finished(),
            "locked storage must hold the shell gate"
        );
        state.store(
            crate::login_services::SecureStorageState::Ready as u8,
            Ordering::Release,
        );
        assert!(gate.join().unwrap());
    }

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
    fn running_shell_is_stopped_when_secure_storage_readiness_is_revoked() {
        let mut child = shell_command(&OsString::from("sleep"), &[OsString::from("30")])
            .spawn()
            .unwrap();
        let (_sender, receiver) = mpsc::channel();
        let state = AtomicU8::new(crate::login_services::SecureStorageState::Locked as u8);

        assert!(matches!(
            wait_for_shell(&mut child, &receiver, Some(&state)),
            ShellWait::SecureStorageLost
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
    fn supervised_shell_inherits_standard_display_selection() {
        let command = shell_command(&OsString::from("nickel"), &[]);
        assert!(
            !command
                .get_envs()
                .any(|(name, _)| name == "WAYLAND_DISPLAY" || name == "DISPLAY")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn shell_startup_barrier_releases_only_after_expected_pid_publication() {
        use std::{io::Read, os::unix::net::UnixStream, sync::atomic::AtomicU32};

        use super::{SHELL_STARTUP_BARRIER_MAGIC, ShellStartupBarrier};

        let barrier = ShellStartupBarrier::new().unwrap();
        let path = barrier.path().to_owned();
        let expected_pid = Arc::new(AtomicU32::new(0));
        let child_expected_pid = Arc::clone(&expected_pid);
        let child = thread::spawn(move || {
            let mut stream = UnixStream::connect(path).unwrap();
            let mut token = [0_u8; SHELL_STARTUP_BARRIER_MAGIC.len() + 4];
            stream.read_exact(&mut token).unwrap();
            assert_ne!(
                child_expected_pid.load(Ordering::Acquire),
                0,
                "the shell must not receive readiness before PID publication"
            );
            u32::from_ne_bytes(
                token[SHELL_STARTUP_BARRIER_MAGIC.len()..]
                    .try_into()
                    .unwrap(),
            )
        });

        expected_pid.store(4242, Ordering::Release);
        barrier.release(4242);
        assert_eq!(child.join().unwrap(), 4242);
        barrier.stop();
    }

    #[test]
    fn native_storage_deadline_returns_to_sddm_but_preserves_provider_prompts() {
        use crate::login_services::SecureStorageState;

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
}
