//! Nested-session input-method acceptance probe.

#[cfg(target_os = "linux")]
include!("linux_impl/input_method_acceptance.rs");

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("input_method_acceptance is available only on Linux");
}
