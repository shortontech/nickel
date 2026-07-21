use std::env;

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::monitor::MonitorHandle;
use winit::window::{Window, WindowAttributes, WindowId};

const SECONDARY_DISPLAY_ENV: &str = "CITRIUS_USE_SECONDARY_DISPLAY";

#[derive(Default)]
struct Citrius {
    window: Option<Window>,
}

impl ApplicationHandler for Citrius {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let use_secondary = env_flag(SECONDARY_DISPLAY_ENV);
        let monitors: Vec<_> = event_loop.available_monitors().collect();
        let primary = event_loop.primary_monitor();
        let target = select_monitor(&monitors, primary.as_ref(), use_secondary);

        let mut attributes = WindowAttributes::default()
            .with_title("Citrius")
            .with_inner_size(LogicalSize::new(960, 640))
            .with_min_inner_size(LogicalSize::new(480, 320));

        if let Some(monitor) = target {
            attributes = attributes.with_position(centered_position(&monitor, (960, 640)));
        }

        match event_loop.create_window(attributes) {
            Ok(window) => self.window = Some(window),
            Err(error) => {
                eprintln!("failed to create Citrius window: {error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.as_ref().map(Window::id) != Some(window_id) {
            return;
        }

        if event == WindowEvent::CloseRequested {
            event_loop.exit();
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut Citrius::default())?;
    Ok(())
}

fn env_flag(name: &str) -> bool {
    env::var(name).is_ok_and(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn select_monitor(
    monitors: &[MonitorHandle],
    primary: Option<&MonitorHandle>,
    use_secondary: bool,
) -> Option<MonitorHandle> {
    if use_secondary {
        monitors
            .iter()
            .find(|monitor| primary != Some(*monitor))
            .cloned()
            .or_else(|| primary.cloned())
    } else {
        primary.cloned().or_else(|| monitors.first().cloned())
    }
}

fn centered_position(monitor: &MonitorHandle, window_size: (u32, u32)) -> PhysicalPosition<i32> {
    let origin = monitor.position();
    let size = monitor.size();
    let x = origin.x + (size.width.saturating_sub(window_size.0) / 2) as i32;
    let y = origin.y + (size.height.saturating_sub(window_size.1) / 2) as i32;
    PhysicalPosition::new(x, y)
}

#[cfg(test)]
mod tests {
    use super::env_flag;

    #[test]
    fn missing_environment_flag_is_disabled() {
        let name = "CITRIUS_TEST_MISSING_FLAG";
        // SAFETY: This test uses a unique variable name and no other thread accesses it.
        unsafe { std::env::remove_var(name) };
        assert!(!env_flag(name));
    }

    #[test]
    fn common_true_values_enable_environment_flag() {
        let name = "CITRIUS_TEST_TRUE_FLAG";
        for value in ["1", "true", "TRUE", "yes", "on"] {
            // SAFETY: This test uses a unique variable name and no other thread accesses it.
            unsafe { std::env::set_var(name, value) };
            assert!(env_flag(name));
        }
        // SAFETY: This test uses a unique variable name and no other thread accesses it.
        unsafe { std::env::remove_var(name) };
    }
}
