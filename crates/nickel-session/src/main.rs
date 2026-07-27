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

    if let Some((program, arguments)) = arguments.command {
        spawn_supervised(program, arguments);
    }

    event_loop.run(None, &mut data, move |_| {
        // NickelSession is running
    })?;

    Ok(())
}

fn import_runtime_environment() {
    if let Err(error) = Command::new("dbus-update-activation-environment")
        .args([
            "--systemd",
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "KDE_FULL_SESSION",
            "KDE_SESSION_VERSION",
            "XDG_CURRENT_DESKTOP",
            "XDG_SESSION_DESKTOP",
            "XDG_SESSION_TYPE",
        ])
        .spawn()
    {
        tracing::warn!(%error, "could not start user-session environment import");
    }
}

fn spawn_supervised(program: OsString, arguments: Vec<OsString>) {
    thread::spawn(move || {
        if let Err(error) = login_services::prepare_secure_storage() {
            tracing::error!(%error, "secure storage unavailable; Nickel shell remains stopped");
            return;
        }

        const MAX_STARTS: usize = 4;
        for attempt in 1..=MAX_STARTS {
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

            if attempt < MAX_STARTS {
                thread::sleep(Duration::from_secs(attempt as u64));
            }
        }
        tracing::error!("Nickel shell restart limit reached; compositor remains available");
    });
}
