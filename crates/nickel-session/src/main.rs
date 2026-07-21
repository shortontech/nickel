#![allow(irrefutable_let_patterns)]

mod handlers;

mod grabs;
mod input;
mod state;
mod window_registry;
mod winit;

use smithay::reexports::{
    calloop::EventLoop,
    wayland_server::{Display, DisplayHandle},
};
pub use state::NickelSession;

pub struct CalloopData {
    state: NickelSession,
    display_handle: DisplayHandle,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }

    let mut event_loop: EventLoop<CalloopData> = EventLoop::try_new()?;

    let display: Display<NickelSession> = Display::new()?;
    let display_handle = display.handle();
    let state = NickelSession::new(&mut event_loop, display);

    let mut data = CalloopData {
        state,
        display_handle,
    };

    crate::winit::init_winit(&mut event_loop, &mut data)?;

    println!(
        "nickel-session listening on {}",
        data.state.socket_name.to_string_lossy()
    );

    let mut args = std::env::args().skip(1);
    let flag = args.next();
    let command = args.next();

    match (flag.as_deref(), command) {
        (Some("-c") | Some("--command"), Some(command)) => {
            std::process::Command::new(command).args(args).spawn()?;
        }
        (None, None) => {}
        _ => return Err("usage: nickel-session [--command PROGRAM [ARG ...]]".into()),
    }

    event_loop.run(None, &mut data, move |_| {
        // NickelSession is running
    })?;

    Ok(())
}
