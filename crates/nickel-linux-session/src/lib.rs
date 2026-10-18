//! Linux compositor and login-session ownership for Nickel.
#![cfg(target_os = "linux")]

pub(crate) use nickel_shell::compositor_host::*;
pub mod session;
