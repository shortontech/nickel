//! Live Linux acceptance for the remote-control boundary.

#[cfg(target_os = "linux")]
include!("linux_impl/nickel-linux-remote-control-acceptance.rs");

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("nickel-linux-remote-control-acceptance is available only on Linux");
}
