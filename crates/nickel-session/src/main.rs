#![allow(irrefutable_let_patterns)]

mod handlers;

mod backend;
mod grabs;
mod input;
mod login_services;
mod shell_layout;
mod state;
mod window_frame;
mod window_registry;
#[cfg(feature = "backend-winit")]
mod winit;

use std::{ffi::OsString, process::Command, thread, time::Duration};

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
    let state = NickelSession::new(&mut event_loop, display);

    let mut data = CalloopData {
        state,
        display_handle,
        #[cfg(feature = "backend-udev")]
        event_loop_handle: event_loop.handle(),
        #[cfg(feature = "backend-udev")]
        native: None,
    };

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
        import_runtime_environment();
    }

    let prepare_login_services = arguments.backend == backend::BackendKind::Udev;
    if let Some((program, command_arguments)) = arguments.command {
        spawn_supervised(program, command_arguments, prepare_login_services)?;
    }

    event_loop.run(None, &mut data, move |_| {
        // NickelSession is running
    })?;

    Ok(())
}

fn import_runtime_environment() {
    match Command::new("dbus-update-activation-environment")
        .args([
            "--systemd",
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "KDE_FULL_SESSION",
            "KDE_SESSION_VERSION",
            "XDG_CACHE_HOME",
            "XDG_CONFIG_HOME",
            "XDG_CURRENT_DESKTOP",
            "XDG_DATA_HOME",
            "XDG_SESSION_DESKTOP",
            "XDG_SESSION_TYPE",
            "XDG_STATE_HOME",
        ])
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

const MAX_SHELL_STARTS: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellFailureAction {
    Retry,
    ExitSession,
}

fn shell_failure_action(attempt: usize) -> ShellFailureAction {
    if attempt < MAX_SHELL_STARTS {
        ShellFailureAction::Retry
    } else {
        ShellFailureAction::ExitSession
    }
}

fn spawn_supervised(
    program: OsString,
    arguments: Vec<OsString>,
    prepare_login_services: bool,
) -> std::io::Result<()> {
    thread::Builder::new()
        .name("nickel-shell-supervisor".into())
        .spawn(move || supervise_shell(program, arguments, prepare_login_services))
        .map(|_| ())
}

fn supervise_shell(program: OsString, arguments: Vec<OsString>, prepare_login_services: bool) {
    if prepare_login_services
        && let Err(error) = thread::Builder::new()
            .name("nickel-login-services".into())
            .spawn(|| {
                if let Err(error) = login_services::prepare_secure_storage() {
                    tracing::error!(
                        %error,
                        "secure storage unavailable; applications requiring credentials may fail"
                    );
                }
            })
    {
        tracing::error!(%error, "failed to start secure-storage preparation");
    }

    for attempt in 1..=MAX_SHELL_STARTS {
        match Command::new(&program).args(&arguments).status() {
            Ok(status) if status.success() => {
                tracing::info!(?status, "Nickel shell exited normally");
                std::process::exit(0);
            }
            Ok(status) => {
                tracing::error!(?status, attempt, "Nickel shell exited unexpectedly");
            }
            Err(error) => {
                tracing::error!(%error, attempt, "failed to start Nickel shell");
            }
        }

        match shell_failure_action(attempt) {
            ShellFailureAction::Retry => {
                thread::sleep(Duration::from_secs(attempt as u64));
            }
            ShellFailureAction::ExitSession => {
                tracing::error!(
                    attempts = MAX_SHELL_STARTS,
                    "Nickel shell restart limit reached; terminating session for display-manager recovery"
                );
                std::process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_SHELL_STARTS, ShellFailureAction, shell_failure_action};

    #[test]
    fn shell_restart_budget_ends_the_session() {
        assert_eq!(
            shell_failure_action(MAX_SHELL_STARTS - 1),
            ShellFailureAction::Retry
        );
        assert_eq!(
            shell_failure_action(MAX_SHELL_STARTS),
            ShellFailureAction::ExitSession
        );
    }
}
