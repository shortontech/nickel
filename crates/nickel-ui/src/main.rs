use std::{
    collections::HashMap,
    env,
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
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

mod context_menu;
mod graphics;
mod icons;
mod launcher;
mod layout;
mod model;
mod panel;
mod platform;
mod rectangles;
mod storage;

use launcher::Launcher;
use model::{OpenWindow, TrayItem, WindowGroup, WindowId as ShellWindowId};
use platform::{ShellCommand, TrayFeed, TraySource, WindowAction, WindowFeed};

const SECONDARY_DISPLAY_ENV: &str = "NICKEL_USE_SECONDARY_DISPLAY";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum LauncherVisibility {
    #[default]
    Hidden,
    Visible,
}

#[derive(Clone, Copy)]
enum ContextAction {
    Activate(ShellWindowId),
    Close(ShellWindowId),
    Maximize(ShellWindowId),
    Minimize(ShellWindowId),
}

impl LauncherVisibility {
    fn is_visible(self) -> bool {
        self == Self::Visible
    }

    fn toggle(&mut self) -> bool {
        *self = match *self {
            Self::Hidden => Self::Visible,
            Self::Visible => Self::Hidden,
        };
        self.is_visible()
    }

    fn set(&mut self, visible: bool) {
        *self = if visible { Self::Visible } else { Self::Hidden };
    }
}

struct Nickel {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    panel_window: Option<Arc<Window>>,
    panel_gpu: Option<panel::PanelGpu>,
    panel_hovered: bool,
    panel_task_hovered: Option<usize>,
    panel_tray_hovered: Option<usize>,
    context_menu_window: Option<Arc<Window>>,
    context_menu_gpu: Option<context_menu::ContextMenuGpu>,
    context_menu_hovered: Option<usize>,
    context_close_hovered: Option<usize>,
    context_menu_target: Option<ShellWindowId>,
    context_menu_actions: Vec<ContextAction>,
    context_preview_mode: bool,
    preview_group: Option<usize>,
    preview_hide_deadline: Option<Instant>,
    task_windows: Vec<OpenWindow>,
    window_groups: Vec<WindowGroup>,
    tray_items: Vec<TrayItem>,
    launcher_visibility: LauncherVisibility,
    clock_deadline: Instant,
    window_deadline: Instant,
    window_feed: WindowFeed,
    tray_feed: TrayFeed,
    launcher: Launcher,
    hovered_result: Option<usize>,
    pin_store: Option<storage::PinStore>,
    scroll_offset: usize,
    cursor_position: Option<PhysicalPosition<f64>>,
    scrollbar_drag_offset: Option<f64>,
}

impl Default for Nickel {
    fn default() -> Self {
        let applications = platform::applications();

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
            panel_window: None,
            panel_gpu: None,
            panel_hovered: false,
            panel_task_hovered: None,
            panel_tray_hovered: None,
            context_menu_window: None,
            context_menu_gpu: None,
            context_menu_hovered: None,
            context_close_hovered: None,
            context_menu_target: None,
            context_menu_actions: Vec::new(),
            context_preview_mode: false,
            preview_group: None,
            preview_hide_deadline: None,
            task_windows: Vec::new(),
            window_groups: Vec::new(),
            tray_items: Vec::new(),
            launcher_visibility: LauncherVisibility::default(),
            clock_deadline: next_minute_deadline(Instant::now(), SystemTime::now()),
            window_deadline: Instant::now(),
            window_feed: WindowFeed::new(),
            tray_feed: TrayFeed::new(),
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
    fn ensure_context_menu_gpu(&mut self) -> bool {
        if self.context_menu_gpu.is_some() {
            return true;
        }
        let (Some(window), Some(launcher_gpu)) = (&self.context_menu_window, &self.gpu) else {
            return false;
        };
        match context_menu::ContextMenuGpu::new(window.clone(), launcher_gpu.graphics.clone()) {
            Ok(gpu) => {
                self.context_menu_gpu = Some(gpu);
                true
            }
            Err(error) => {
                eprintln!("failed to initialize Nickel context menu renderer: {error}");
                false
            }
        }
    }

    fn hide_context_menu(&mut self) {
        self.context_menu_target = None;
        self.context_menu_hovered = None;
        self.context_close_hovered = None;
        self.context_menu_actions.clear();
        self.context_preview_mode = false;
        self.preview_group = None;
        self.preview_hide_deadline = None;
        if !platform::send_shell_command(ShellCommand::HideContextMenu)
            && let Some(window) = &self.context_menu_window
        {
            window.set_visible(false);
        }
    }

    fn show_context_menu(&mut self, task_index: usize) {
        let Some(task) = self
            .window_groups
            .get(task_index)
            .and_then(|group| group.windows.last())
        else {
            return;
        };
        let task = task.id;
        self.context_menu_target = Some(task);
        self.context_preview_mode = false;
        self.preview_group = None;
        self.preview_hide_deadline = None;
        if !self.ensure_context_menu_gpu() {
            return;
        }
        self.context_menu_actions = vec![ContextAction::Close(task)];
        if let Some(gpu) = &mut self.context_menu_gpu {
            gpu.set_labels(&["Close window".into()]);
        }
        let x = panel::task_menu_x(task_index);
        if !platform::send_shell_command(ShellCommand::ShowContextMenu {
            x,
            width: context_menu::WIDTH as i32,
            height: context_menu::height_for(1) as i32,
        }) && let Some(window) = &self.context_menu_window
        {
            window.set_visible(true);
            window.focus_window();
        }
        if let Some(window) = &self.context_menu_window {
            window.request_redraw();
        }
    }

    fn show_window_group(&mut self, task_index: usize) {
        let preview_was_visible =
            self.context_preview_mode && self.preview_group == Some(task_index);
        let Some(group) = self.window_groups.get(task_index) else {
            return;
        };
        let windows = group.windows.clone();
        let application_name = group.application_name.clone();
        let previews: Vec<_> = windows
            .iter()
            .filter_map(|window| self.window_feed.preview(window.id))
            .collect();
        self.preview_group = Some(task_index);
        self.preview_hide_deadline = None;
        if previews.is_empty() {
            return;
        }
        let labels: Vec<_> = previews
            .iter()
            .map(|preview| {
                let window = windows
                    .iter()
                    .find(|window| window.id == preview.window)
                    .expect("preview belongs to grouped window");
                if window.title.is_empty() {
                    application_name.clone()
                } else {
                    window.title.clone()
                }
            })
            .collect();
        let actions: Vec<_> = previews
            .iter()
            .map(|preview| ContextAction::Activate(preview.window))
            .collect();
        self.context_menu_target = previews.last().map(|preview| preview.window);
        if !self.ensure_context_menu_gpu() {
            return;
        }
        self.context_menu_actions = actions;
        self.context_preview_mode = true;
        if let Some(gpu) = &mut self.context_menu_gpu {
            gpu.set_previews(
                &labels,
                previews.into_iter().map(|preview| preview.image).collect(),
            );
        }
        let x = panel::task_menu_x(task_index);
        if !preview_was_visible {
            platform::send_shell_command(ShellCommand::ShowPreview {
                x,
                width: context_menu::preview_width(labels.len()) as i32,
                height: context_menu::PREVIEW_HEIGHT as i32,
            });
        }
        if let Some(window) = &self.context_menu_window {
            window.request_redraw();
        }
    }

    fn show_window_actions(&mut self, index: usize) {
        let x = self
            .preview_group
            .map(panel::task_menu_x)
            .unwrap_or_default()
            + index as i32 * context_menu::PREVIEW_CARD_WIDTH as i32;
        let Some(window) = self
            .context_menu_actions
            .get(index)
            .map(|action| match action {
                ContextAction::Activate(window)
                | ContextAction::Close(window)
                | ContextAction::Maximize(window)
                | ContextAction::Minimize(window) => *window,
            })
        else {
            return;
        };
        self.context_menu_target = Some(window);
        self.context_preview_mode = false;
        self.preview_group = None;
        self.preview_hide_deadline = None;
        self.context_menu_hovered = None;
        self.context_close_hovered = None;
        self.context_menu_actions = vec![
            ContextAction::Close(window),
            ContextAction::Maximize(window),
            ContextAction::Minimize(window),
        ];
        if let Some(gpu) = &mut self.context_menu_gpu {
            gpu.set_labels(&[
                "Close window".into(),
                "Maximize / Restore".into(),
                "Minimize window".into(),
            ]);
        }
        platform::send_shell_command(ShellCommand::ShowContextMenu {
            x,
            width: context_menu::WIDTH as i32,
            height: context_menu::height_for(3) as i32,
        });
        if let Some(window) = &self.context_menu_window {
            window.request_redraw();
        }
    }

    fn set_launcher_visible(&mut self, visible: bool) {
        self.hide_context_menu();
        self.launcher_visibility.set(visible);
        if !visible {
            self.launcher.clear();
            self.hovered_result = None;
            self.scroll_offset = 0;
            self.scrollbar_drag_offset = None;
        }
        if platform::send_shell_command(if visible {
            ShellCommand::Show
        } else {
            ShellCommand::Hide
        }) {
            return;
        }
        if let Some(window) = &self.window {
            window.set_visible(visible);
            if visible {
                window.focus_window();
                window.request_redraw();
            }
        }
    }

    fn toggle_launcher(&mut self) {
        self.hide_context_menu();
        let visible = self.launcher_visibility.toggle();
        if !platform::send_shell_command(ShellCommand::Toggle) {
            self.set_launcher_visible(visible);
        }
    }

    fn refresh_task_windows(&mut self) {
        let Some(windows) = self.window_feed.snapshot(&self.launcher) else {
            return;
        };
        if windows == self.task_windows {
            return;
        }
        let groups = self.launcher.group_windows(&windows);
        let tasks = groups
            .iter()
            .map(|group| {
                let resolved = group
                    .application_id
                    .as_ref()
                    .and_then(|id| self.launcher.application(id))
                    .and_then(|application| application.icon_path())
                    .and_then(icons::load);
                if resolved.is_none() {
                    eprintln!("no panel icon resolved for {}", group.application_name);
                }
                let icon = resolved.unwrap_or_else(panel::fallback_icon);
                panel::PanelTask {
                    id: group.windows.last().map_or(0, |window| window.id.0),
                    active: group.active(),
                    icon,
                }
            })
            .collect();
        self.task_windows = windows;
        self.window_groups = groups;
        if self
            .context_menu_target
            .is_some_and(|target| !self.task_windows.iter().any(|window| window.id == target))
        {
            self.hide_context_menu();
        }
        if let Some(gpu) = &mut self.panel_gpu {
            gpu.set_tasks(tasks);
        }
        if let Some(window) = &self.panel_window {
            window.request_redraw();
        }
    }

    fn refresh_tray_items(&mut self) {
        let items = self.tray_feed.snapshot();
        if items == self.tray_items {
            return;
        }
        eprintln!("nickel-ui: tray items updated: {}", items.len());
        let rendered = items
            .iter()
            .map(|item| panel::PanelTrayItem {
                icon: item.icon.clone(),
            })
            .collect();
        self.tray_items = items;
        if let Some(gpu) = &mut self.panel_gpu {
            gpu.set_tray_items(rendered);
        }
        if let Some(window) = &self.panel_window {
            window.request_redraw();
        }
    }

    fn launch_result(&self, index: usize) {
        let Some(result) = self.launcher.result_at(index) else {
            return;
        };
        match result.launch() {
            Ok(child) => println!(
                "launched application: {} (pid {}, icon {})",
                result.name(),
                child.id(),
                result.icon().unwrap_or("none")
            ),
            Err(error) => eprintln!("failed to launch application {}: {error}", result.name()),
        }
    }

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
    graphics: Arc<graphics::SharedGraphics>,
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
        let (graphics, surface) = graphics::SharedGraphics::new(window.clone()).await?;
        let size = window.inner_size();
        let config = surface
            .get_default_config(&graphics.adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| "graphics surface has no supported configuration".to_owned())?;
        surface.configure(&graphics.device, &config);

        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&graphics.device);
        let viewport = Viewport::new(&graphics.device, &cache);
        let mut atlas = TextAtlas::new(&graphics.device, &graphics.queue, &cache, config.format);
        let text_renderer = TextRenderer::new(
            &mut atlas,
            &graphics.device,
            wgpu::MultisampleState::default(),
            None,
        );
        let rectangle_renderer =
            rectangles::RectangleRenderer::new(&graphics.device, config.format);
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
            graphics,
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
        self.surface.configure(&self.graphics.device, &self.config);
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
            &self.graphics.queue,
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
            &self.graphics.queue,
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
                &self.graphics.device,
                &self.graphics.queue,
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
                self.surface.configure(&self.graphics.device, &self.config);
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
        let mut encoder =
            self.graphics
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

        self.graphics.queue.submit([encoder.finish()]);
        self.graphics.queue.present(frame);
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

        let mut launcher_attributes = WindowAttributes::default()
            .with_title("Nickel Launcher Initializing")
            .with_inner_size(LogicalSize::new(960, 640))
            .with_min_inner_size(LogicalSize::new(480, 320))
            .with_decorations(false);
        let mut panel_attributes = WindowAttributes::default()
            .with_title("Nickel Panel")
            .with_inner_size(LogicalSize::new(220, 56))
            .with_decorations(false);
        let context_menu_attributes = WindowAttributes::default()
            .with_title("Nickel Context Menu")
            .with_inner_size(LogicalSize::new(context_menu::WIDTH, context_menu::HEIGHT))
            .with_decorations(false);
        if let Some(monitor) = target {
            launcher_attributes =
                launcher_attributes.with_position(centered_position(&monitor, (960, 640)));
            panel_attributes = panel_attributes.with_position(panel_position(&monitor, (220, 56)));
        }

        let Ok(launcher_window) = event_loop.create_window(launcher_attributes) else {
            eprintln!("failed to create Nickel launcher window");
            event_loop.exit();
            return;
        };
        let launcher_window = Arc::new(launcher_window);
        let Ok(panel_window) = event_loop.create_window(panel_attributes) else {
            eprintln!("failed to create Nickel panel window");
            event_loop.exit();
            return;
        };
        let panel_window = Arc::new(panel_window);
        let Ok(context_menu_window) = event_loop.create_window(context_menu_attributes) else {
            eprintln!("failed to create Nickel context menu window");
            event_loop.exit();
            return;
        };
        let context_menu_window = Arc::new(context_menu_window);
        platform::send_shell_command(ShellCommand::HideContextMenu);
        let Ok(launcher_gpu) = pollster::block_on(Gpu::new(launcher_window.clone())) else {
            eprintln!("failed to initialize Nickel launcher renderer");
            event_loop.exit();
            return;
        };
        let shared_graphics = launcher_gpu.graphics.clone();
        let Ok(panel_gpu) = panel::PanelGpu::new(panel_window.clone(), shared_graphics.clone())
        else {
            eprintln!("failed to initialize Nickel panel renderer");
            event_loop.exit();
            return;
        };
        launcher_window.set_title("Nickel Launcher");
        panel_window.request_redraw();
        self.window = Some(launcher_window);
        self.gpu = Some(launcher_gpu);
        self.panel_window = Some(panel_window);
        self.panel_gpu = Some(panel_gpu);
        self.context_menu_window = Some(context_menu_window);
        self.context_menu_gpu = None;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.context_menu_window.as_ref().map(|window| window.id()) == Some(window_id) {
            match event {
                WindowEvent::CloseRequested => self.hide_context_menu(),
                WindowEvent::Focused(false)
                    if self.context_menu_target.is_some() && !self.context_preview_mode =>
                {
                    self.hide_context_menu();
                }
                WindowEvent::Resized(size) => {
                    if self.context_menu_gpu.is_none() {
                        self.ensure_context_menu_gpu();
                    }
                    if let Some(gpu) = &mut self.context_menu_gpu {
                        gpu.resize(size.width, size.height);
                    }
                }
                WindowEvent::RedrawRequested => {
                    if self.context_menu_gpu.is_none() {
                        self.ensure_context_menu_gpu();
                    }
                    if let Some(gpu) = &mut self.context_menu_gpu {
                        gpu.render(self.context_menu_hovered);
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    self.preview_hide_deadline = None;
                    let hovered = if self.context_preview_mode {
                        context_menu::preview_at(position, self.context_menu_actions.len())
                    } else {
                        context_menu::item_at(position, self.context_menu_actions.len())
                    };
                    let close_hovered = self
                        .context_preview_mode
                        .then(|| {
                            context_menu::preview_close_at(
                                position,
                                self.context_menu_actions.len(),
                            )
                        })
                        .flatten();
                    if self.context_preview_mode && hovered != self.context_menu_hovered {
                        if let Some(window) = hovered.and_then(|index| {
                            self.context_menu_actions
                                .get(index)
                                .map(|action| match action {
                                    ContextAction::Activate(window)
                                    | ContextAction::Close(window)
                                    | ContextAction::Maximize(window)
                                    | ContextAction::Minimize(window) => *window,
                                })
                        }) {
                            platform::send_shell_command(ShellCommand::HighlightWindow(window));
                        } else {
                            platform::send_shell_command(ShellCommand::ClearWindowHighlight);
                        }
                    }
                    if hovered != self.context_menu_hovered
                        || close_hovered != self.context_close_hovered
                    {
                        self.context_menu_hovered = hovered;
                        self.context_close_hovered = close_hovered;
                        self.context_menu_window
                            .as_ref()
                            .expect("context menu window exists")
                            .request_redraw();
                    }
                }
                WindowEvent::CursorLeft { .. } => {
                    if self.context_preview_mode {
                        platform::send_shell_command(ShellCommand::ClearWindowHighlight);
                    }
                    self.context_menu_hovered = None;
                    self.context_close_hovered = None;
                    if self.context_preview_mode {
                        self.preview_hide_deadline =
                            Some(Instant::now() + Duration::from_millis(250));
                    }
                    self.context_menu_window
                        .as_ref()
                        .expect("context menu window exists")
                        .request_redraw();
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } if self.context_menu_hovered.is_some() => {
                    let index = self.context_menu_hovered.expect("hovered item exists");
                    let action = if self.context_close_hovered == Some(index) {
                        self.context_menu_actions
                            .get(index)
                            .map(|action| match action {
                                ContextAction::Activate(window) => ContextAction::Close(*window),
                                other => *other,
                            })
                    } else {
                        self.context_menu_actions.get(index).copied()
                    };
                    if let Some(action) = action {
                        let (window, action) = match action {
                            ContextAction::Activate(window) => (window, WindowAction::Activate),
                            ContextAction::Close(window) => (window, WindowAction::Close),
                            ContextAction::Maximize(window) => (window, WindowAction::Maximize),
                            ContextAction::Minimize(window) => (window, WindowAction::Minimize),
                        };
                        platform::send_shell_command(ShellCommand::WindowAction { window, action });
                    }
                    self.hide_context_menu();
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Right,
                    ..
                } if self.context_preview_mode && self.context_menu_hovered.is_some() => {
                    self.show_window_actions(
                        self.context_menu_hovered.expect("preview is hovered"),
                    );
                }
                _ => {}
            }
            return;
        }

        if self.panel_window.as_ref().map(|window| window.id()) == Some(window_id) {
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::Resized(size) => {
                    if let Some(gpu) = &mut self.panel_gpu {
                        gpu.resize(size.width, size.height);
                    }
                    self.panel_window
                        .as_ref()
                        .expect("panel window exists")
                        .request_redraw();
                }
                WindowEvent::RedrawRequested => {
                    if let Some(gpu) = &mut self.panel_gpu {
                        gpu.render(self.panel_hovered, self.panel_task_hovered);
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    let hovered = panel::launcher_button_contains(position);
                    let task_hovered = panel::task_at(position, self.window_groups.len());
                    let tray_hovered = self.panel_window.as_ref().and_then(|window| {
                        panel::tray_at(position, window.inner_size().width, self.tray_items.len())
                    });
                    if hovered == self.panel_hovered
                        && task_hovered == self.panel_task_hovered
                        && tray_hovered == self.panel_tray_hovered
                    {
                        return;
                    }
                    self.panel_hovered = hovered;
                    self.panel_task_hovered = task_hovered;
                    self.panel_tray_hovered = tray_hovered;
                    if let Some(index) = task_hovered {
                        self.show_window_group(index);
                    } else if self.context_preview_mode {
                        self.preview_hide_deadline =
                            Some(Instant::now() + Duration::from_millis(250));
                    }
                    let window = self.panel_window.as_ref().expect("panel window exists");
                    window.set_cursor(
                        if hovered || task_hovered.is_some() || tray_hovered.is_some() {
                            CursorIcon::Pointer
                        } else {
                            CursorIcon::Default
                        },
                    );
                    window.request_redraw();
                }
                WindowEvent::CursorLeft { .. } => {
                    self.panel_hovered = false;
                    self.panel_task_hovered = None;
                    self.panel_tray_hovered = None;
                    if self.context_preview_mode {
                        self.preview_hide_deadline =
                            Some(Instant::now() + Duration::from_millis(250));
                    }
                    let window = self.panel_window.as_ref().expect("panel window exists");
                    window.set_cursor(CursorIcon::Default);
                    window.request_redraw();
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } if self.panel_hovered => self.toggle_launcher(),
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } if self.panel_tray_hovered.is_some() => {
                    let index = self.panel_tray_hovered.expect("tray item is hovered");
                    if let Some(item) = self.tray_items.get(index) {
                        self.tray_feed.activate(&item.id);
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Right,
                    ..
                } => {
                    if let Some(index) = self.panel_task_hovered {
                        self.show_context_menu(index);
                    } else {
                        self.hide_context_menu();
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } => {
                    if let Some(index) = self.panel_task_hovered {
                        if self
                            .window_groups
                            .get(index)
                            .is_some_and(|group| group.windows.len() > 1)
                        {
                            self.show_window_group(index);
                            return;
                        }
                        if let Some(window) = self
                            .window_groups
                            .get(index)
                            .and_then(|group| group.windows.last())
                        {
                            platform::send_shell_command(ShellCommand::WindowAction {
                                window: window.id,
                                action: WindowAction::Activate,
                            });
                        }
                    }
                    self.hide_context_menu();
                }
                _ => {}
            }
            return;
        }

        if self.window.as_ref().map(|window| window.id()) != Some(window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested => self.set_launcher_visible(false),
            WindowEvent::Focused(false) if self.launcher_visibility.is_visible() => {
                self.set_launcher_visible(false);
            }
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
                    self.launch_result(index);
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
                        self.set_launcher_visible(false);
                    }
                    Key::Named(NamedKey::Escape) => self.launcher.clear(),
                    Key::Named(NamedKey::Enter) => {
                        self.launch_result(self.launcher.selected_index());
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if self
            .preview_hide_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.hide_context_menu();
        }
        if now >= self.window_deadline {
            self.refresh_task_windows();
            self.refresh_tray_items();
            if let Some(group) = self.preview_group
                && (self.context_preview_mode || self.panel_task_hovered == Some(group))
            {
                self.show_window_group(group);
            }
            self.window_deadline = now + Duration::from_millis(250);
        }
        if now >= self.clock_deadline {
            if self
                .panel_gpu
                .as_mut()
                .is_some_and(panel::PanelGpu::update_clock)
                && let Some(window) = &self.panel_window
            {
                window.request_redraw();
            }
            self.clock_deadline = next_minute_deadline(now, SystemTime::now());
        }
        let mut deadline = self.clock_deadline.min(self.window_deadline);
        if let Some(preview_deadline) = self.preview_hide_deadline {
            deadline = deadline.min(preview_deadline);
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
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

fn next_minute_deadline(now: Instant, wall_clock: SystemTime) -> Instant {
    const NANOS_PER_MINUTE: u128 = 60_000_000_000;
    let elapsed = wall_clock.duration_since(UNIX_EPOCH).unwrap_or_default();
    let into_minute = elapsed.as_nanos() % NANOS_PER_MINUTE;
    let remaining = (NANOS_PER_MINUTE - into_minute) as u64;
    now + Duration::from_nanos(remaining)
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

fn panel_position(monitor: &MonitorHandle, panel_size: (u32, u32)) -> PhysicalPosition<i32> {
    let origin = monitor.position();
    let size = monitor.size();
    let x = origin.x + (size.width.saturating_sub(panel_size.0) / 2) as i32;
    let y = origin.y + size.height.saturating_sub(panel_size.1) as i32;
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
    use std::time::{Duration, Instant, UNIX_EPOCH};

    use super::{LauncherVisibility, env_flag, next_minute_deadline};

    #[test]
    fn clock_deadline_targets_the_next_minute_boundary() {
        let now = Instant::now();
        let wall_clock = UNIX_EPOCH + Duration::from_secs(125) + Duration::from_millis(250);
        assert_eq!(
            next_minute_deadline(now, wall_clock).duration_since(now),
            Duration::from_millis(54_750)
        );
    }

    #[test]
    fn launcher_visibility_toggles_without_recreation() {
        let mut visibility = LauncherVisibility::default();
        assert!(!visibility.is_visible());
        assert!(visibility.toggle());
        assert!(!visibility.toggle());
        visibility.set(true);
        assert!(visibility.is_visible());
    }

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
