#![allow(dead_code, unused_imports, unused_variables)]

#[cfg(target_os = "windows")]
mod auto_present;
#[cfg(target_os = "windows")]
mod controller;
#[cfg(target_os = "windows")]
mod fallback_band1;
#[cfg(all(target_os = "windows", feature = "diagnostics"))]
mod frame_service_direct;
#[cfg(target_os = "windows")]
mod host;
#[cfg(target_os = "windows")]
mod presentation_callbacks;
#[cfg(target_os = "windows")]
mod shell_window;
#[cfg(all(target_os = "windows", feature = "diagnostics"))]
mod view_event_trace;
#[cfg(target_os = "windows")]
mod wrapper_inspect;

#[cfg(target_os = "windows")]
pub use host::{run_embedded_host, run_host, run_managed_host};
