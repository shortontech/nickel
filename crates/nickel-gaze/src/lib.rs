extern crate self as nickel_gaze;

pub mod cache;
pub mod contract;
pub mod grid;
pub mod model;

#[path = "bin/nickel-gaze-grid.rs"]
#[allow(dead_code)]
mod grid_application;

pub use grid_application::{GazeGridApplication, GazeGridFixtureProvider, GazeGridFixtureState};

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub mod camera;
