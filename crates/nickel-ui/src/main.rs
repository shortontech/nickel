use std::{
    collections::HashMap,
    env,
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
};

use glyphon::{
    Attrs, Buffer, Cache, Color, ContentType, CustomGlyph, Family, FontSystem, Metrics,
    RasterizeCustomGlyphRequest, RasterizedCustomGlyph, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport,
};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::monitor::MonitorHandle;
use winit::window::{CursorIcon, Window, WindowAttributes, WindowId};

mod icons;
mod launcher;
mod layout;
mod rectangles;
mod storage;

#[cfg(target_os = "linux")]
mod desktop_entries;

use launcher::Launcher;

const SECONDARY_DISPLAY_ENV: &str = "NICKEL_USE_SECONDARY_DISPLAY";

struct Nickel {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    launcher: Launcher,
    hovered_result: Option<usize>,
    pin_store: Option<storage::PinStore>,
    scroll_offset: usize,
    cursor_position: Option<PhysicalPosition<f64>>,
    scrollbar_drag_offset: Option<f64>,
}

impl Default for Nickel {
    fn default() -> Self {
        #[cfg(target_os = "linux")]
        let applications = desktop_entries::load_applications();
        #[cfg(not(target_os = "linux"))]
        let applications = Vec::new();

        let mut launcher = if applications.is_empty() {
            Launcher::default()
        } else {
            Launcher::new(applications)
        };
        let pin_store = match storage::PinStore::open_default() {
            Ok(store) => {
                match store.pins() {
                    Ok(pins) => launcher.set_pins(pins),
                    Err(error) => eprintln!("failed to read pins: {error}"),
                }
                Some(store)
            }
            Err(error) => {
                eprintln!("persistent pin storage unavailable: {error}");
                None
            }
        };
        Self {
            window: None,
            gpu: None,
            launcher,
            hovered_result: None,
            pin_store,
            scroll_offset: 0,
            cursor_position: None,
            scrollbar_drag_offset: None,
        }
    }
}

impl Nickel {
    fn viewport_metrics(&self) -> (u32, u32, usize) {
        let size = self.window.as_ref().expect("window exists").inner_size();
        (
            size.width,
            size.height,
            layout::visible_capacity(size.height),
        )
    }

    fn set_scroll_offset(&mut self, offset: usize) {
        let (_, _, capacity) = self.viewport_metrics();
        self.scroll_offset = offset.min(layout::max_scroll_offset(
            self.launcher.result_count(),
            capacity,
        ));
    }

    fn scroll_by(&mut self, rows: i32) {
        let offset = if rows.is_negative() {
            self.scroll_offset
                .saturating_sub(rows.unsigned_abs() as usize)
        } else {
            self.scroll_offset.saturating_add(rows as usize)
        };
        self.set_scroll_offset(offset);
    }

    fn ensure_selection_visible(&mut self) {
        let (_, _, capacity) = self.viewport_metrics();
        if capacity == 0 {
            return;
        }
        let selected = self.launcher.selected_index();
        if selected < self.scroll_offset {
            self.scroll_offset = selected;
        } else if selected >= self.scroll_offset + capacity {
            self.scroll_offset = selected + 1 - capacity;
        }
        self.set_scroll_offset(self.scroll_offset);
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
    result_buffers: Vec<Buffer>,
    icon_ids: HashMap<PathBuf, u16>,
    icon_images: Vec<Option<image::RgbaImage>>,
    icon_requests: mpsc::Sender<(u16, PathBuf)>,
    icon_results: mpsc::Receiver<(u16, Option<image::RgbaImage>)>,
    rectangle_renderer: rectangles::RectangleRenderer,
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
                label: Some("nickel device"),
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
        let rectangle_renderer = rectangles::RectangleRenderer::new(&device, config.format);
        let mut search_buffer = Buffer::new(&mut font_system, Metrics::new(30.0, 44.0));
        search_buffer.set_size(Some(config.width.saturating_sub(112) as f32), Some(56.0));
        let result_buffers = (0..9)
            .map(|_| Buffer::new(&mut font_system, Metrics::new(24.0, 48.0)))
            .collect();
        let (icon_requests, worker_requests) = mpsc::channel::<(u16, PathBuf)>();
        let (worker_results, icon_results) = mpsc::channel();
        let redraw_window = window.clone();
        thread::Builder::new()
            .name("nickel-icon-loader".into())
            .spawn(move || {
                while let Ok((id, path)) = worker_requests.recv() {
                    if worker_results.send((id, icons::load(&path))).is_err() {
                        break;
                    }
                    redraw_window.request_redraw();
                }
            })
            .map_err(|error| format!("failed to start icon loader: {error}"))?;

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
            result_buffers,
            icon_ids: HashMap::new(),
            icon_images: Vec::new(),
            icon_requests,
            icon_results,
            rectangle_renderer,
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
    }

    fn render(&mut self, launcher: &Launcher, hovered_result: Option<usize>, scroll_offset: usize) {
        while let Ok((id, image)) = self.icon_results.try_recv() {
            if let Some(slot) = self.icon_images.get_mut(id as usize) {
                *slot = image;
            }
        }
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

        let capacity = layout::visible_capacity(self.config.height);
        let visible_count = launcher
            .result_count()
            .saturating_sub(scroll_offset)
            .min(capacity)
            .max(1);
        while self.result_buffers.len() < visible_count {
            self.result_buffers
                .push(Buffer::new(&mut self.font_system, Metrics::new(24.0, 48.0)));
        }
        let mut row_glyphs = vec![Vec::new(); visible_count];
        for (index, buffer) in self
            .result_buffers
            .iter_mut()
            .take(visible_count)
            .enumerate()
        {
            let result_index = scroll_offset + index;
            let row = layout::ResultRow::allocate(index, self.config.width);
            let text = launcher
                .result_at(result_index)
                .map_or("No applications found", |result| result.name());
            let selected = launcher.result_count() > 0 && result_index == launcher.selected_index();
            let pinned = launcher
                .result_at(result_index)
                .is_some_and(|application| launcher.is_pinned(application.id()));
            let marker = match (selected, pinned) {
                (true, true) => "› ★",
                (true, false) => "›  ",
                (false, true) => "  ★",
                (false, false) => "   ",
            };
            let text = format!("{marker} {text}");
            buffer.set_size(Some(row.label.width), Some(row.label.height));
            buffer.set_text(
                &text,
                &Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(&mut self.font_system, false);

            let Some(path) = launcher
                .result_at(result_index)
                .and_then(|application| application.icon_path())
            else {
                continue;
            };
            let glyph_id = if let Some(id) = self.icon_ids.get(path) {
                *id
            } else {
                let Ok(id) = u16::try_from(self.icon_images.len()) else {
                    continue;
                };
                self.icon_ids.insert(path.to_owned(), id);
                self.icon_images.push(None);
                if self.icon_requests.send((id, path.to_owned())).is_err() {
                    eprintln!("icon loader stopped before loading {}", path.display());
                }
                id
            };
            if self.icon_images[glyph_id as usize].is_some() {
                row_glyphs[index].push(CustomGlyph {
                    id: glyph_id,
                    left: row.icon.x - row.label.x,
                    top: row.icon.y - row.label.y,
                    width: row.icon.width,
                    height: row.icon.height,
                    color: None,
                    snap_to_physical_pixel: true,
                    metadata: 0,
                });
            }
        }

        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        let scrollbar = layout::scrollbar(
            self.config.width,
            self.config.height,
            launcher.result_count(),
            capacity,
            scroll_offset,
        );
        let hovered_row = hovered_result.and_then(|index| index.checked_sub(scroll_offset));
        self.rectangle_renderer.update(
            &self.queue,
            (self.config.width, self.config.height),
            hovered_row.filter(|index| *index < visible_count),
            scrollbar,
        );
        let icon_images = &self.icon_images;
        let mut text_areas = Vec::with_capacity(visible_count + 1);
        text_areas.push(TextArea {
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
        });
        for (index, buffer) in self.result_buffers.iter().take(visible_count).enumerate() {
            let row = layout::ResultRow::allocate(index, self.config.width);
            text_areas.push(TextArea {
                buffer,
                left: row.label.x,
                top: row.label.y,
                scale: 1.0,
                bounds: TextBounds {
                    left: row.outer.x as i32,
                    top: row.outer.y as i32,
                    right: row.outer.right() as i32,
                    bottom: row.outer.bottom() as i32,
                },
                default_color: Color::rgb(208, 216, 232),
                custom_glyphs: &row_glyphs[index],
            });
        }
        self.text_renderer
            .prepare_with_custom(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                text_areas,
                &mut self.swash_cache,
                &|request: RasterizeCustomGlyphRequest| {
                    let source = icon_images.get(request.id as usize)?.as_ref()?;
                    let image = icons::resized(source, request.width.into(), request.height.into());
                    Some(RasterizedCustomGlyph {
                        data: image.into_raw(),
                        content_type: ContentType::Color,
                    })
                },
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
                label: Some("nickel frame encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nickel background pass"),
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
            self.rectangle_renderer.render(&mut pass);
            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("text rendering should succeed");
        }

        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
    }
}

impl ApplicationHandler for Nickel {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let use_secondary = env_flag(SECONDARY_DISPLAY_ENV);
        let monitors: Vec<_> = event_loop.available_monitors().collect();
        let primary = event_loop.primary_monitor();
        let target = select_monitor(&monitors, primary.as_ref(), use_secondary);

        let mut attributes = WindowAttributes::default()
            .with_title("Nickel")
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
                eprintln!("failed to create nickel window: {error}");
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
                self.set_scroll_offset(self.scroll_offset);
                self.window
                    .as_ref()
                    .expect("window exists")
                    .request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.render(&self.launcher, self.hovered_result, self.scroll_offset);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = Some(position);
                if let Some(grab_offset) = self.scrollbar_drag_offset {
                    let (width, height, capacity) = self.viewport_metrics();
                    if let Some(scrollbar) = layout::scrollbar(
                        width,
                        height,
                        self.launcher.result_count(),
                        capacity,
                        self.scroll_offset,
                    ) {
                        let offset = layout::offset_from_thumb_y(
                            position.y - grab_offset,
                            scrollbar,
                            self.launcher.result_count(),
                            capacity,
                        );
                        self.set_scroll_offset(offset);
                        self.hovered_result = None;
                        self.window
                            .as_ref()
                            .expect("window exists")
                            .request_redraw();
                    }
                    return;
                }
                let (width, _, capacity) = self.viewport_metrics();
                let local_count = self
                    .launcher
                    .result_count()
                    .saturating_sub(self.scroll_offset)
                    .min(capacity);
                let hovered = hit_test_result(position, width, local_count)
                    .map(|index| index + self.scroll_offset);
                if hovered != self.hovered_result {
                    self.hovered_result = hovered;
                    let window = self.window.as_ref().expect("window exists");
                    window.set_cursor(if hovered.is_some() {
                        CursorIcon::Pointer
                    } else {
                        CursorIcon::Default
                    });
                    window.request_redraw();
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor_position = None;
                self.hovered_result = None;
                let window = self.window.as_ref().expect("window exists");
                window.set_cursor(CursorIcon::Default);
                window.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let rows = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y.round() as i32,
                    MouseScrollDelta::PixelDelta(position) => (-position.y / 52.0).round() as i32,
                };
                if rows != 0 {
                    self.scroll_by(rows);
                    self.hovered_result = None;
                    self.window
                        .as_ref()
                        .expect("window exists")
                        .request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let (width, height, capacity) = self.viewport_metrics();
                if let (Some(position), Some(scrollbar)) = (
                    self.cursor_position,
                    layout::scrollbar(
                        width,
                        height,
                        self.launcher.result_count(),
                        capacity,
                        self.scroll_offset,
                    ),
                ) {
                    if layout::rect_contains(scrollbar.thumb, position.x, position.y) {
                        self.scrollbar_drag_offset =
                            Some(position.y - f64::from(scrollbar.thumb.y));
                        return;
                    }
                    if layout::rect_contains(scrollbar.track, position.x, position.y) {
                        if position.y < f64::from(scrollbar.thumb.y) {
                            self.scroll_by(-(capacity as i32));
                        } else {
                            self.scroll_by(capacity as i32);
                        }
                        self.window
                            .as_ref()
                            .expect("window exists")
                            .request_redraw();
                        return;
                    }
                }
                if let Some(index) = self.hovered_result {
                    self.launcher.select(index);
                    if let Some(result) = self.launcher.selected_result() {
                        println!("clicked application: {}", result.name());
                    }
                    self.window
                        .as_ref()
                        .expect("window exists")
                        .request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => self.scrollbar_drag_offset = None,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                if let Some(index) = self.hovered_result
                    && let Some(application_id) = self
                        .launcher
                        .result_at(index)
                        .map(|application| application.id().to_owned())
                    && let Some(store) = &self.pin_store
                {
                    match store.toggle(&application_id).and_then(|_| store.pins()) {
                        Ok(pins) => {
                            self.launcher.set_pins(pins);
                            self.scroll_offset = 0;
                            self.hovered_result = None;
                            self.window
                                .as_ref()
                                .expect("window exists")
                                .request_redraw();
                        }
                        Err(error) => eprintln!("failed to update pin: {error}"),
                    }
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
                                "selected application: {} (icon: {}, path: {}, exec: {})",
                                result.name(),
                                result.icon().unwrap_or("none"),
                                result.icon_path().map_or_else(
                                    || "unresolved".into(),
                                    |path| path.display().to_string()
                                ),
                                result.exec().unwrap_or("D-Bus activation")
                            );
                        }
                        changed = false;
                    }
                    Key::Character(text) => self.launcher.insert(&text),
                    _ => changed = false,
                }
                if changed {
                    self.ensure_selection_visible();
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
    event_loop.run_app(&mut Nickel::default())?;
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

fn hit_test_result(
    position: PhysicalPosition<f64>,
    window_width: u32,
    result_count: usize,
) -> Option<usize> {
    layout::hit_test_result(position.x, position.y, window_width, result_count)
}

#[cfg(test)]
mod tests {
    use super::env_flag;

    #[test]
    fn missing_environment_flag_is_disabled() {
        let name = "NICKEL_TEST_MISSING_FLAG";
        // SAFETY: This test uses a unique variable name and no other thread accesses it.
        unsafe { std::env::remove_var(name) };
        assert!(!env_flag(name));
    }

    #[test]
    fn common_true_values_enable_environment_flag() {
        let name = "NICKEL_TEST_TRUE_FLAG";
        for value in ["1", "true", "TRUE", "yes", "on"] {
            // SAFETY: This test uses a unique variable name and no other thread accesses it.
            unsafe { std::env::set_var(name, value) };
            assert!(env_flag(name));
        }
        // SAFETY: This test uses a unique variable name and no other thread accesses it.
        unsafe { std::env::remove_var(name) };
    }
}
