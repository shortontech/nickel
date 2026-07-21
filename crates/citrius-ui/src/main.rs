use std::{env, sync::Arc};

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::monitor::MonitorHandle;
use winit::window::{Window, WindowAttributes, WindowId};

mod launcher;

#[cfg(target_os = "linux")]
mod desktop_entries;

use launcher::Launcher;

const SECONDARY_DISPLAY_ENV: &str = "CITRIUS_USE_SECONDARY_DISPLAY";

struct Citrius {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    launcher: Launcher,
}

impl Default for Citrius {
    fn default() -> Self {
        #[cfg(target_os = "linux")]
        let applications = desktop_entries::load_applications();
        #[cfg(not(target_os = "linux"))]
        let applications = Vec::new();

        let launcher = if applications.is_empty() {
            Launcher::default()
        } else {
            Launcher::new(applications)
        };
        Self {
            window: None,
            gpu: None,
            launcher,
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
    text_renderer: TextRenderer,
    search_buffer: Buffer,
    results_buffer: Buffer,
}

impl Gpu {
    async fn new(window: Arc<Window>) -> Result<Self, String> {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| format!("failed to create graphics surface: {error}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
            .map_err(|error| format!("failed to find a graphics adapter: {error}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Citrius device"),
                ..Default::default()
            })
            .await
            .map_err(|error| format!("failed to create graphics device: {error}"))?;
        let size = window.inner_size();
        let config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| "graphics surface has no supported configuration".to_owned())?;
        surface.configure(&device, &config);

        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, config.format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let mut search_buffer = Buffer::new(&mut font_system, Metrics::new(30.0, 44.0));
        search_buffer.set_size(Some(config.width.saturating_sub(112) as f32), Some(56.0));
        let mut results_buffer = Buffer::new(&mut font_system, Metrics::new(24.0, 52.0));
        results_buffer.set_size(
            Some(config.width.saturating_sub(112) as f32),
            Some(config.height.saturating_sub(180) as f32),
        );

        Ok(Self {
            surface,
            device,
            queue,
            config,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            search_buffer,
            results_buffer,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        let text_width = width.saturating_sub(112) as f32;
        self.search_buffer.set_size(Some(text_width), Some(56.0));
        self.results_buffer
            .set_size(Some(text_width), Some(height.saturating_sub(180) as f32));
    }

    fn render(&mut self, launcher: &Launcher) {
        let search_text = if launcher.query().is_empty() {
            "Search applications…".to_owned()
        } else {
            format!("{}▏", launcher.query())
        };
        self.search_buffer.set_text(
            &search_text,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        self.search_buffer
            .shape_until_scroll(&mut self.font_system, false);

        let results_text = if launcher.result_count() == 0 {
            "No applications found".to_owned()
        } else {
            (0..launcher.result_count())
                .filter_map(|index| {
                    let result = launcher.result_at(index)?;
                    if index == launcher.selected_index() {
                        Some(format!("›  {}", result.name()))
                    } else {
                        Some(format!("   {}", result.name()))
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        self.results_buffer.set_text(
            &results_text,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        self.results_buffer
            .shape_until_scroll(&mut self.font_system, false);

        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        self.text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [
                    TextArea {
                        buffer: &self.search_buffer,
                        left: 56.0,
                        top: 48.0,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: 56,
                            top: 48,
                            right: self.config.width.saturating_sub(56) as i32,
                            bottom: 108,
                        },
                        default_color: Color::rgb(238, 241, 248),
                        custom_glyphs: &[],
                    },
                    TextArea {
                        buffer: &self.results_buffer,
                        left: 56.0,
                        top: 136.0,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: 56,
                            top: 136,
                            right: self.config.width.saturating_sub(56) as i32,
                            bottom: self.config.height.saturating_sub(32) as i32,
                        },
                        default_color: Color::rgb(208, 216, 232),
                        custom_glyphs: &[],
                    },
                ],
                &mut self.swash_cache,
            )
            .expect("text preparation should succeed");

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("skipped frame after surface validation error");
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Citrius frame encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Citrius background pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.035,
                            g: 0.045,
                            b: 0.065,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                ..Default::default()
            });
            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("text rendering should succeed");
        }

        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        self.atlas.trim();
    }
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
            Ok(window) => {
                let window = Arc::new(window);
                match pollster::block_on(Gpu::new(window.clone())) {
                    Ok(gpu) => {
                        window.request_redraw();
                        self.window = Some(window);
                        self.gpu = Some(gpu);
                    }
                    Err(error) => {
                        eprintln!("{error}");
                        event_loop.exit();
                    }
                }
            }
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
        if self.window.as_ref().map(|window| window.id()) != Some(window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
                self.window
                    .as_ref()
                    .expect("window exists")
                    .request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.render(&self.launcher);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let mut changed = true;
                match event.logical_key {
                    Key::Named(NamedKey::ArrowDown) => self.launcher.select_next(),
                    Key::Named(NamedKey::ArrowUp) => self.launcher.select_previous(),
                    Key::Named(NamedKey::Backspace) => self.launcher.backspace(),
                    Key::Named(NamedKey::Escape) if self.launcher.query().is_empty() => {
                        event_loop.exit();
                    }
                    Key::Named(NamedKey::Escape) => self.launcher.clear(),
                    Key::Named(NamedKey::Enter) => {
                        if let Some(result) = self.launcher.selected_result() {
                            println!(
                                "selected application: {} (icon: {}, exec: {})",
                                result.name(),
                                result.icon().unwrap_or("none"),
                                result.exec().unwrap_or("D-Bus activation")
                            );
                        }
                        changed = false;
                    }
                    Key::Character(text) => self.launcher.insert(&text),
                    _ => changed = false,
                }
                if changed {
                    self.window
                        .as_ref()
                        .expect("window exists")
                        .request_redraw();
                }
            }
            _ => {}
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
