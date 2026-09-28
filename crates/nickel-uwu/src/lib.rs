#[cfg(target_os = "windows")]
mod fallback_band1;
#[cfg(target_os = "windows")]
mod frame_service_direct;

#[cfg(target_os = "windows")]
mod controller;
#[cfg(target_os = "windows")]
mod host;
#[cfg(target_os = "windows")]
mod presentation_callbacks;
#[cfg(target_os = "windows")]
mod shell_window;
#[cfg(target_os = "windows")]
mod view_event_trace;
#[cfg(target_os = "windows")]
pub use host::{prepare_app, run_embedded_host, run_host, run_managed_host};
