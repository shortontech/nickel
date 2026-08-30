//! Manual/live Wayland pointer-constraint probe.
//!
//! Run this inside Nickel, move the pointer, and press Escape to exit. Structured
//! stdout distinguishes relative device motion from visible cursor motion so a
//! nested acceptance script can verify that locking does not discard deltas.

use std::{num::NonZeroU32, rc::Rc};

use nickel_input::{InputEvent, KeyCode, KeyEdge, PhysicalKey, PointerEvent};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{DeviceEvent, DeviceId, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop, OwnedDisplayHandle},
    window::{CursorGrabMode, Window, WindowAttributes, WindowId},
};

struct PointerConstraintProbe {
    graphics: softbuffer::Context<OwnedDisplayHandle>,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<OwnedDisplayHandle, Rc<Window>>>,
    constraint: ProbeConstraint,
    input: nickel_input::winit::Adapter,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ProbeConstraint {
    #[default]
    None,
    Confined,
    Locked,
}

impl PointerConstraintProbe {
    fn new(graphics: softbuffer::Context<OwnedDisplayHandle>) -> Self {
        Self {
            graphics,
            window: None,
            surface: None,
            constraint: ProbeConstraint::None,
            input: nickel_input::winit::Adapter::default(),
        }
    }

    fn set_constraint(&mut self, constraint: ProbeConstraint) {
        let Some(window) = &self.window else {
            return;
        };
        let mode = match constraint {
            ProbeConstraint::None => CursorGrabMode::None,
            ProbeConstraint::Confined => CursorGrabMode::Confined,
            ProbeConstraint::Locked => CursorGrabMode::Locked,
        };
        match window.set_cursor_grab(mode) {
            Ok(()) => {
                self.constraint = constraint;
                window.set_cursor_visible(constraint != ProbeConstraint::Locked);
                println!("constraint mode={constraint:?}");
            }
            Err(error) => println!("constraint-error mode={constraint:?} error={error}"),
        }
    }
}

impl ApplicationHandler for PointerConstraintProbe {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        match event_loop.create_window(
            WindowAttributes::default()
                .with_title("Nickel pointer constraints — P toggles, Escape exits")
                .with_inner_size(LogicalSize::new(640, 420)),
        ) {
            Ok(window) => {
                let window = Rc::new(window);
                match softbuffer::Surface::new(&self.graphics, window.clone()) {
                    Ok(surface) => {
                        window.request_redraw();
                        self.surface = Some(surface);
                        self.window = Some(window);
                    }
                    Err(error) => {
                        eprintln!("surface-error error={error}");
                        event_loop.exit();
                    }
                }
            }
            Err(error) => {
                eprintln!("window-error error={error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        for input in self.input.normalize(nickel_input::DeviceId(0), &event) {
            match input {
                InputEvent::Key(key) if key.edge == KeyEdge::Pressed && !key.repeat => {
                    match key.physical {
                        PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                        PhysicalKey::Code(KeyCode::KeyP) => {
                            let next = match self.constraint {
                                ProbeConstraint::Locked => ProbeConstraint::Confined,
                                ProbeConstraint::Confined => ProbeConstraint::None,
                                ProbeConstraint::None => ProbeConstraint::Locked,
                            };
                            self.set_constraint(next);
                        }
                        _ => {}
                    }
                }
                InputEvent::Pointer(PointerEvent::Axis {
                    delta, discrete, ..
                }) => println!("wheel dx={} dy={} discrete={discrete:?}", delta.x, delta.y),
                _ => {}
            }
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(true) if self.constraint == ProbeConstraint::None => {
                self.set_constraint(ProbeConstraint::Locked);
            }
            WindowEvent::RedrawRequested => {
                let Some(window) = &self.window else {
                    return;
                };
                let Some(surface) = &mut self.surface else {
                    return;
                };
                let size = window.inner_size();
                let width = NonZeroU32::new(size.width.max(1)).expect("nonzero width");
                let height = NonZeroU32::new(size.height.max(1)).expect("nonzero height");
                if let Err(error) = surface.resize(width, height) {
                    eprintln!("resize-error error={error}");
                    return;
                }
                match surface.buffer_mut() {
                    Ok(mut buffer) => {
                        buffer.fill(0x20_24_2c);
                        if let Err(error) = buffer.present() {
                            eprintln!("present-error error={error}");
                        }
                    }
                    Err(error) => eprintln!("buffer-error error={error}"),
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                println!("cursor x={} y={}", position.x, position.y);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.input.set_scale_factor(scale_factor);
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta } = event {
            println!("relative dx={} dy={}", delta.0, delta.1);
        }
    }
}

fn main() -> Result<(), winit::error::EventLoopError> {
    let event_loop = EventLoop::new()?;
    let graphics = softbuffer::Context::new(event_loop.owned_display_handle())
        .expect("softbuffer context should initialize");
    event_loop.run_app(&mut PointerConstraintProbe::new(graphics))
}
