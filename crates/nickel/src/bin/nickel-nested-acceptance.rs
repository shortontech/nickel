//! Bounded live acceptance harness for Nickel's nested compositor.

#[cfg(target_os = "linux")]
include!("linux_impl/nickel-nested-acceptance.rs");

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("nickel-nested-acceptance is available only on Linux");
}
