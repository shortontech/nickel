use std::{io, process::Child, sync::Arc};

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{CursorIcon, Window, WindowAttributes, WindowId},
};

const WIDTH: u32 = 220;
const HEIGHT: u32 = 64;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum LauncherState {
    #[default]
    Stopped,
    Running,
}

impl LauncherState {
    fn request_open(&mut self) -> bool {
        if *self == Self::Running {
            return false;
        }
        *self = Self::Running;
        true
    }

    fn stopped(&mut self) {
        *self = Self::Stopped;
    }
}

#[derive(Default)]
struct LauncherProcess {
    state: LauncherState,
    child: Option<Child>,
}

impl LauncherProcess {
    fn reap(&mut self) {
        let exited = self
            .child
            .as_mut()
            .is_some_and(|child| child.try_wait().is_ok_and(|status| status.is_some()));
        if exited {
            self.child = None;
            self.state.stopped();
        }
    }

    fn open(&mut self) -> io::Result<()> {
        self.reap();
        if !self.state.request_open() {
            return Ok(());
        }
        match launcher_command()?.spawn() {
            Ok(child) => {
                self.child = Some(child);
                Ok(())
            }
            Err(error) => {
                self.state.stopped();
                Err(error)
            }
        }
    }
}

fn launcher_command() -> io::Result<std::process::Command> {
    let executable = std::env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or_else(|| io::Error::other("panel executable has no parent directory"))?;
    Ok(std::process::Command::new(directory.join("nickel-ui")))
}

#[derive(Default)]
struct Panel {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    cursor: Option<PhysicalPosition<f64>>,
    hovered: bool,
    launcher: LauncherProcess,
}

impl ApplicationHandler for Panel {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = WindowAttributes::default()
            .with_title("Nickel Panel")
            .with_inner_size(LogicalSize::new(WIDTH, HEIGHT))
            .with_min_inner_size(LogicalSize::new(WIDTH, HEIGHT))
            .with_max_inner_size(LogicalSize::new(WIDTH, HEIGHT))
            .with_resizable(false);
        match event_loop.create_window(attributes) {
            Ok(window) => {
                let window = Arc::new(window);
                match pollster::block_on(Gpu::new(window.clone())) {
                    Ok(gpu) => {
                        window.request_redraw();
                        self.window = Some(window);
                        self.gpu = Some(gpu);
                    }
                    Err(error) => {
                        eprintln!("failed to initialize panel renderer: {error}");
                        event_loop.exit();
                    }
                }
            }
            Err(error) => {
                eprintln!("failed to create panel window: {error}");
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
        if self.window.as_ref().map(|window| window.id()) != Some(window_id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
                self.redraw();
            }
            WindowEvent::RedrawRequested => {
                self.launcher.reap();
                if let Some(gpu) = &mut self.gpu {
                    gpu.render(self.hovered);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Some(position);
                if !self.hovered {
                    self.hovered = true;
                    let window = self.window.as_ref().expect("panel window exists");
                    window.set_cursor(CursorIcon::Pointer);
                    window.request_redraw();
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = None;
                self.hovered = false;
                let window = self.window.as_ref().expect("panel window exists");
                window.set_cursor(CursorIcon::Default);
                window.request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if self.cursor.is_some() => {
                if let Err(error) = self.launcher.open() {
                    eprintln!("failed to open Nickel launcher: {error}");
                }
            }
            _ => {}
        }
    }
}

impl Panel {
    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderer: TextRenderer,
    label: Buffer,
}

impl Gpu {
    async fn new(window: Arc<Window>) -> Result<Self, String> {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| error.to_string())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
            .map_err(|error| error.to_string())?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Nickel panel device"),
                ..Default::default()
            })
            .await
            .map_err(|error| error.to_string())?;
        let size = window.inner_size();
        let config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| "panel surface has no supported configuration".to_owned())?;
        surface.configure(&device, &config);

        let mut font_system = FontSystem::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, config.format);
        let renderer =
            TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let mut label = Buffer::new(&mut font_system, Metrics::new(22.0, 44.0));
        label.set_size(Some(config.width as f32), Some(config.height as f32));
        label.set_text(
            "◆  Nickel",
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        label.shape_until_scroll(&mut font_system, false);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            font_system,
            swash_cache: SwashCache::new(),
            viewport,
            atlas,
            renderer,
            label,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.label.set_size(Some(width as f32), Some(height as f32));
    }

    fn render(&mut self, hovered: bool) {
        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        self.renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [TextArea {
                    buffer: &self.label,
                    left: 48.0,
                    top: 10.0,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: self.config.width as i32,
                        bottom: self.config.height as i32,
                    },
                    default_color: Color::rgb(238, 241, 248),
                    custom_glyphs: &[],
                }],
                &mut self.swash_cache,
            )
            .expect("panel text preparation succeeds");

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Nickel panel encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Nickel panel pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(if hovered {
                            wgpu::Color {
                                r: 0.12,
                                g: 0.28,
                                b: 0.52,
                                a: 1.0,
                            }
                        } else {
                            wgpu::Color {
                                r: 0.035,
                                g: 0.045,
                                b: 0.065,
                                a: 1.0,
                            }
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                ..Default::default()
            });
            self.renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("panel text rendering succeeds");
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut Panel::default())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::LauncherState;

    #[test]
    fn one_launcher_runs_until_the_child_exits() {
        let mut state = LauncherState::default();
        assert!(state.request_open());
        assert!(!state.request_open());
        state.stopped();
        assert!(state.request_open());
    }
}
