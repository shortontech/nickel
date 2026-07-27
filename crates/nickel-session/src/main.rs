#![allow(irrefutable_let_patterns)]

mod handlers;

mod backend;
mod grabs;
mod input;
mod shell_layout;
mod state;
mod window_registry;
#[cfg(feature = "backend-winit")]
mod winit;

use std::{
    ffi::{OsStr, OsString},
    process::{Command, ExitStatus},
    thread,
    time::Duration,
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
        prepare_login_session()?;
    }

    if let Some((program, arguments)) = arguments.command {
        spawn_supervised(program, arguments);
    }

    event_loop.run(None, &mut data, move |_| {
        // NickelSession is running
    })?;

    Ok(())
}

fn prepare_login_session() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: native session initialization is single-threaded and no child
    // application is launched until the complete environment is installed.
    unsafe {
        std::env::set_var("XDG_SESSION_TYPE", "wayland");
        std::env::set_var("XDG_CURRENT_DESKTOP", "Nickel");
        std::env::set_var("XDG_SESSION_DESKTOP", "Nickel");
    }

    run_checked(
        "dbus-update-activation-environment",
        [
            OsStr::new("--systemd"),
            OsStr::new("DISPLAY"),
            OsStr::new("WAYLAND_DISPLAY"),
            OsStr::new("XDG_CURRENT_DESKTOP"),
            OsStr::new("XDG_SESSION_DESKTOP"),
            OsStr::new("XDG_SESSION_TYPE"),
        ],
    )?;

    run_checked(
        "busctl",
        [
            OsStr::new("--user"),
            OsStr::new("call"),
            OsStr::new("org.freedesktop.DBus"),
            OsStr::new("/org/freedesktop/DBus"),
            OsStr::new("org.freedesktop.DBus"),
            OsStr::new("StartServiceByName"),
            OsStr::new("su"),
            OsStr::new("org.freedesktop.secrets"),
            OsStr::new("0"),
        ],
    )?;

    wait_for_secret_service(Duration::from_secs(15))?;
    verify_default_collection()?;
    Ok(())
}

fn wait_for_secret_service(timeout: Duration) -> Result<(), Box<dyn std::error::Error>> {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        if Command::new("busctl")
            .args(["--user", "--quiet", "status", "org.freedesktop.secrets"])
            .status()
            .is_ok_and(|status| status.success())
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }

    Err("secure storage did not become ready; refusing to launch applications".into())
}

fn verify_default_collection() -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("busctl")
        .args([
            "--user",
            "call",
            "org.freedesktop.secrets",
            "/org/freedesktop/secrets",
            "org.freedesktop.Secret.Service",
            "ReadAlias",
            "s",
            "default",
        ])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "could not read the existing Secret Service default collection: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }

    let response = String::from_utf8_lossy(&output.stdout);
    let collection = default_collection_path(&response)?;
    let locked = Command::new("busctl")
        .args([
            "--user",
            "get-property",
            "org.freedesktop.secrets",
            collection,
            "org.freedesktop.Secret.Collection",
            "Locked",
        ])
        .output()?;
    if !locked.status.success() {
        return Err(format!(
            "could not verify the existing Secret Service collection: {}",
            String::from_utf8_lossy(&locked.stderr).trim()
        )
        .into());
    }
    if String::from_utf8_lossy(&locked.stdout)
        .split_whitespace()
        .last()
        != Some("false")
    {
        return Err(
            "the existing Secret Service collection is locked; refusing to launch applications"
                .into(),
        );
    }
    Ok(())
}

fn run_checked<I, S>(program: &str, arguments: I) -> Result<ExitStatus, Box<dyn std::error::Error>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = Command::new(program).args(arguments).status()?;
    if !status.success() {
        return Err(format!("{program} failed with {status}").into());
    }
    Ok(status)
}

fn spawn_supervised(program: OsString, arguments: Vec<OsString>) {
    thread::spawn(move || {
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

fn default_collection_path(response: &str) -> Result<&str, &'static str> {
    let path = response
        .split_whitespace()
        .last()
        .map(|path| path.trim_matches('"'))
        .ok_or("Secret Service returned no default collection identity")?;
    if path == "/" {
        Err("Secret Service has no default collection; refusing to create a replacement")
    } else {
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::default_collection_path;

    #[test]
    fn accepts_existing_default_collection() {
        assert_eq!(
            default_collection_path("o \"/org/freedesktop/secrets/collection/kdewallet\"\n")
                .unwrap(),
            "/org/freedesktop/secrets/collection/kdewallet"
        );
    }

    #[test]
    fn rejects_missing_default_collection() {
        assert!(default_collection_path("o \"/\"\n").is_err());
    }
}
