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

    if let Some((program, arguments)) = arguments.command {
        std::process::Command::new(program)
            .args(arguments)
            .spawn()?;
    }

    event_loop.run(None, &mut data, move |_| {
        // NickelSession is running
    })?;

    Ok(())
}
