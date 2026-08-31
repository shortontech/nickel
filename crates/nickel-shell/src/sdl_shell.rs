//! SDL3 window and event ownership for the Nickel shell.
//!
//! Rendering is deliberately outside this module. A renderer receives stable
//! [`SurfaceId`] values and can attach either a software surface or an
//! accelerated backend without owning the application event pump.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;
use std::time::Instant;

use nickel_input::InputEvent;
use nickel_session_protocol::ShellRole as SessionShellRole;
use nickel_ui::backend::PaintCommand;
use nickel_ui::{AggregatePresenterCacheDiagnostics, DamageRegion, HostChangeToken};
use sdl3::event::{Event, WindowEvent};
use sdl3::video::{Window, WindowPos};

use crate::sdl_gpu::{SdlGpuPresenter, SharedSdlGraphics};

pub const DESKTOP_TITLE: &str = "Nickel Desktop";
pub const PANEL_TITLE: &str = "Nickel Panel";
pub const LAUNCHER_TITLE: &str = "Nickel Launcher";
pub const CONTROL_CENTER_TITLE: &str = "Nickel Control Center";
pub const NOTIFICATION_TITLE: &str = "Nickel Notification";
pub const WINDOW_PREVIEW_TITLE: &str = "Nickel Window Preview";
pub const WINDOW_CONTEXT_MENU_TITLE: &str = "Nickel Window Menu";
pub const CODEX_PROJECT_MENU_TITLE: &str = "Nickel Codex Projects";
pub const LOCK_TITLE: &str = "Nickel Lock";
pub const SCREENSHOT_TITLE: &str = "Nickel Screenshot";
pub const PANEL_HEIGHT: u32 = 56;
const RUNTIME_SAMPLE_CAPACITY: usize = 64;

fn push_bounded(samples: &mut VecDeque<u64>, sample: u64) {
    if samples.len() == RUNTIME_SAMPLE_CAPACITY {
        samples.pop_front();
    }
    samples.push_back(sample);
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SurfaceId(u32);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShellMemoryDiagnostics {
    /// Cache-owned bytes reported by every currently instantiated surface presenter.
    pub presenter_caches: AggregatePresenterCacheDiagnostics,
    /// Allocator/process-visible resident bytes from the operating system.
    /// This is intentionally independent of `presenter_caches.live_bytes`.
    pub process_rss_bytes: Option<usize>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShellRuntimeDiagnostics {
    /// Completed presents after the presenter was initialized, in microseconds.
    pub warm_present_us: Vec<u64>,
    /// Input receipt through the first synchronous present it caused, in microseconds.
    pub input_to_present_us: Vec<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceRole {
    Desktop,
    Panel,
    Launcher,
    ControlCenter,
    Notification,
    WindowPreview,
    WindowContextMenu,
    CodexProjectMenu,
    Lock,
    Screenshot,
    CodexChat,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShellEvent {
    GlobalShortcut(crate::platform::GlobalShortcut),
    #[cfg(target_os = "linux")]
    TestControl(crate::platform::ShellTestRequest),
    Quit,
    Input {
        surface: SurfaceId,
        event: InputEvent,
    },
    Shown(SurfaceId),
    Hidden(SurfaceId),
    CloseRequested(SurfaceId),
    FocusChanged {
        surface: SurfaceId,
        focused: bool,
    },
    PointerEntered {
        surface: SurfaceId,
        entered: bool,
    },
    LogicalResize {
        surface: SurfaceId,
        width: u32,
        height: u32,
    },
    PixelResize {
        surface: SurfaceId,
        width: u32,
        height: u32,
        scale: f32,
    },
    DisplayTopologyChanged,
    Redraw(SurfaceId),
}

pub struct ShellSurface {
    id: SurfaceId,
    role: SurfaceRole,
    display_index: usize,
    output_name: String,
    display_connected: bool,
    // Drop the GPU surface before the native window whose handles it borrows.
    presenter: Option<SdlGpuPresenter>,
    last_host_change_token: Option<HostChangeToken>,
    window: Window,
}

impl ShellSurface {
    pub fn id(&self) -> SurfaceId {
        self.id
    }

    pub fn role(&self) -> SurfaceRole {
        self.role
    }

    pub fn display_index(&self) -> usize {
        self.display_index
    }

    pub fn output_name(&self) -> &str {
        &self.output_name
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn window_mut(&mut self) -> &mut Window {
        &mut self.window
    }
}

pub struct SdlShell {
    // Surface presenters must drop before their shared device, and all native
    // windows must drop before SDL's video subsystem.
    surfaces: Vec<ShellSurface>,
    graphics: Option<std::sync::Arc<SharedSdlGraphics>>,
    surface_indices: HashMap<u32, usize>,
    events: sdl3::EventPump,
    pending_events: VecDeque<ShellEvent>,
    input_adapter: nickel_input::sdl::Adapter,
    warm_present_us: VecDeque<u64>,
    input_to_present_us: VecDeque<u64>,
    pending_input_started: Option<Instant>,
    video: sdl3::VideoSubsystem,
    _sdl: sdl3::Sdl,
    started: Instant,
}

impl SdlShell {
    pub fn new(started: Instant) -> Result<Self, String> {
        configure_input_hints();
        let sdl = sdl3::init().map_err(|error| error.to_string())?;
        let video = sdl.video().map_err(|error| error.to_string())?;
        // SDL disables the screen saver by default and creates one Wayland
        // idle inhibitor per shell surface. Nickel owns idle policy itself,
        // so its shell must never globally inhibit the compositor.
        video.enable_screen_saver();
        sdl.event()
            .map_err(|error| error.to_string())?
            .register_custom_event::<crate::platform::GlobalShortcut>()
            .map_err(|error| error.to_string())?;
        #[cfg(target_os = "linux")]
        sdl.event()
            .map_err(|error| error.to_string())?
            .register_custom_event::<crate::platform::ShellTestRequest>()
            .map_err(|error| error.to_string())?;
        let events = sdl.event_pump().map_err(|error| error.to_string())?;
        tracing::info!(
            elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0,
            driver = video.current_video_driver(),
            "SDL video initialized"
        );
        #[cfg(target_os = "linux")]
        if video.current_video_driver() != "wayland" {
            tracing::warn!(
                driver = video.current_video_driver(),
                "Nickel session expected the SDL Wayland video driver"
            );
        }
        Ok(Self {
            surfaces: Vec::new(),
            graphics: None,
            surface_indices: HashMap::new(),
            events,
            pending_events: VecDeque::new(),
            input_adapter: nickel_input::sdl::Adapter::default(),
            warm_present_us: VecDeque::with_capacity(RUNTIME_SAMPLE_CAPACITY),
            input_to_present_us: VecDeque::with_capacity(RUNTIME_SAMPLE_CAPACITY),
            pending_input_started: None,
            video,
            _sdl: sdl,
            started,
        })
    }

    pub fn event_sender(&self) -> sdl3::event::EventSender {
        self._sdl
            .event()
            .expect("SDL event subsystem remains initialized")
            .event_sender()
    }

    pub fn create_shell_surfaces(&mut self) -> Result<(), String> {
        self.surfaces.clear();
        self.surface_indices.clear();
        let displays = require_displays(self.display_geometries()?)?;
        let output_names = self.display_names()?;
        for (display_index, geometry) in displays.iter().copied().enumerate() {
            let output_name = output_names.get(display_index).ok_or_else(|| {
                "SDL output identity count changed during shell startup".to_string()
            })?;
            if crate::platform::renders_desktop_background() {
                self.create_surface(SurfaceRole::Desktop, display_index, geometry, output_name)?;
            }
            self.create_surface(SurfaceRole::Panel, display_index, geometry, output_name)?;
            self.create_surface(SurfaceRole::Lock, display_index, geometry, output_name)?;
        }
        let primary = displays[0];
        let primary_name = output_names
            .first()
            .ok_or_else(|| "SDL reported no output identity for the primary display".to_string())?;
        self.create_surface(SurfaceRole::Launcher, 0, primary, primary_name)?;
        self.create_surface(SurfaceRole::ControlCenter, 0, primary, primary_name)?;
        self.create_surface(SurfaceRole::Notification, 0, primary, primary_name)?;
        self.create_surface(SurfaceRole::WindowPreview, 0, primary, primary_name)?;
        self.create_surface(SurfaceRole::WindowContextMenu, 0, primary, primary_name)?;
        self.create_surface(SurfaceRole::CodexProjectMenu, 0, primary, primary_name)?;
        self.create_surface(SurfaceRole::Screenshot, 0, primary, primary_name)?;
        tracing::info!(
            elapsed_ms = self.started.elapsed().as_secs_f64() * 1_000.0,
            surface_count = self.surfaces.len(),
            "SDL shell windows created"
        );
        Ok(())
    }

    pub fn sync_display_geometry(&mut self) -> Result<(), String> {
        let displays = require_displays(self.display_geometries()?)?;
        let output_names = self.display_names()?;
        let panels_match_outputs = output_names.iter().all(|output_name| {
            self.surfaces.iter().any(|surface| {
                surface.display_connected
                    && surface.role == SurfaceRole::Panel
                    && surface.output_name == *output_name
            })
        });
        if !panels_match_outputs {
            for surface in &mut self.surfaces {
                if matches!(
                    surface.role,
                    SurfaceRole::Desktop | SurfaceRole::Panel | SurfaceRole::Lock
                ) && !output_names.iter().any(|name| name == &surface.output_name)
                {
                    surface.display_connected = false;
                    let _ = surface.window.hide();
                    if let Some(presenter) = surface.presenter.as_mut() {
                        presenter.suspend();
                    }
                }
            }
            self.rebuild_surface_indices();
            for (display_index, geometry) in displays.iter().copied().enumerate() {
                let output_name = output_names.get(display_index).ok_or_else(|| {
                    "SDL output identity count changed during shell sync".to_string()
                })?;
                let has_panel = self.surfaces.iter().any(|surface| {
                    surface.display_connected
                        && surface.role == SurfaceRole::Panel
                        && surface.output_name == *output_name
                });
                if has_panel {
                    continue;
                }
                for role in [SurfaceRole::Desktop, SurfaceRole::Panel, SurfaceRole::Lock] {
                    if role == SurfaceRole::Desktop
                        && !crate::platform::renders_desktop_background()
                    {
                        continue;
                    }
                    if let Some(surface) = self.surfaces.iter_mut().find(|surface| {
                        !surface.display_connected
                            && surface.role == role
                            && surface.output_name == *output_name
                    }) {
                        surface.display_index = display_index;
                        surface.display_connected = true;
                        let (_, x, y, width, height, _) = surface_geometry(role, geometry);
                        surface
                            .window
                            .set_position(WindowPos::Positioned(x), WindowPos::Positioned(y));
                        surface
                            .window
                            .set_size(width, height)
                            .map_err(|error| error.to_string())?;
                        let _ = surface.window.show();
                    } else {
                        self.create_surface(role, display_index, geometry, output_name)?;
                    }
                }
            }
            self.rebuild_surface_indices();
        }

        for surface in &mut self.surfaces {
            if !surface.display_connected {
                continue;
            }
            if matches!(
                surface.role,
                SurfaceRole::CodexChat
                    | SurfaceRole::WindowPreview
                    | SurfaceRole::WindowContextMenu
            ) {
                continue;
            }
            let Some(display_index) = output_names
                .iter()
                .position(|name| name == &surface.output_name)
            else {
                continue;
            };
            surface.display_index = display_index;
            let Some(display) = displays.get(display_index).copied() else {
                continue;
            };
            let (_, x, y, width, height, _) = surface_geometry(surface.role, display);
            surface
                .window
                .set_position(WindowPos::Positioned(x), WindowPos::Positioned(y));
            surface
                .window
                .set_size(width, height)
                .map_err(|error| error.to_string())?;
            if let Some(presenter) = surface.presenter.as_mut() {
                presenter.invalidate();
            }
        }
        Ok(())
    }

    fn rebuild_surface_indices(&mut self) {
        self.surface_indices.clear();
        for (index, surface) in self
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, surface)| surface.display_connected)
        {
            self.surface_indices.insert(surface.id.0, index);
        }
    }

    pub fn surfaces(&self) -> impl Iterator<Item = &ShellSurface> {
        self.surfaces
            .iter()
            .filter(|surface| surface.display_connected)
    }

    pub fn surface(&self, id: SurfaceId) -> Option<&ShellSurface> {
        self.surface_indices
            .get(&id.0)
            .and_then(|index| self.surfaces.get(*index))
    }

    pub fn surface_display_geometry(&self, id: SurfaceId) -> Option<DisplayGeometry> {
        let display_index = self.surface(id)?.display_index();
        self.display_geometries().ok()?.get(display_index).copied()
    }

    pub fn surface_mut(&mut self, id: SurfaceId) -> Option<&mut ShellSurface> {
        let index = *self.surface_indices.get(&id.0)?;
        self.surfaces.get_mut(index)
    }

    pub fn create_codex_chat_surface(
        &mut self,
        title: &str,
        application_id: &str,
    ) -> Result<SurfaceId, String> {
        let geometry = require_displays(self.display_geometries()?)?[0];
        let previous_app_id = sdl3::hint::get("SDL_APP_ID");
        sdl3::hint::set("SDL_APP_ID", application_id);
        let mut builder =
            self.video
                .window(title, 1120.min(geometry.width), 760.min(geometry.height));
        builder.position_centered().resizable().high_pixel_density();
        let window = builder.build().map_err(|error| error.to_string());
        if let Some(previous_app_id) = previous_app_id {
            sdl3::hint::set("SDL_APP_ID", &previous_app_id);
        } else {
            sdl3::hint::set("SDL_APP_ID", "io.nickel.shell");
        }
        let window = window?;
        self.video.text_input().start(&window);
        let id = SurfaceId(window.id());
        let index = self.surfaces.len();
        self.surface_indices.insert(id.0, index);
        self.surfaces.push(ShellSurface {
            id,
            role: SurfaceRole::CodexChat,
            display_index: 0,
            output_name: String::new(),
            display_connected: true,
            presenter: None,
            last_host_change_token: None,
            window,
        });
        Ok(id)
    }

    pub fn destroy_surface(&mut self, id: SurfaceId) {
        let Some(index) = self.surface_indices.remove(&id.0) else {
            return;
        };
        self.surfaces.remove(index);
        self.rebuild_surface_indices();
        let diagnostics = self.memory_diagnostics();
        tracing::debug!(
            presenters = diagnostics.presenter_caches.presenters,
            cache_live_bytes = diagnostics.presenter_caches.live_bytes,
            process_rss_bytes = diagnostics.process_rss_bytes,
            "shell surface closed and presenter accounting refreshed"
        );
    }

    pub fn memory_diagnostics(&self) -> ShellMemoryDiagnostics {
        ShellMemoryDiagnostics {
            presenter_caches: AggregatePresenterCacheDiagnostics::from_presenters(
                self.surfaces
                    .iter()
                    .filter_map(|surface| surface.presenter.as_ref())
                    .map(SdlGpuPresenter::cache_diagnostics),
            ),
            process_rss_bytes: process_rss_bytes(),
        }
    }

    pub fn runtime_diagnostics(&self) -> ShellRuntimeDiagnostics {
        ShellRuntimeDiagnostics {
            warm_present_us: self.warm_present_us.iter().copied().collect(),
            input_to_present_us: self.input_to_present_us.iter().copied().collect(),
        }
    }

    /// Starts a bounded input-to-visible observation. Call `finish_input_observation`
    /// after routing the input so inputs that do not paint cannot contaminate a later sample.
    pub fn begin_input_observation(&mut self, now: Instant) {
        self.pending_input_started = Some(now);
    }

    pub fn finish_input_observation(&mut self) {
        self.pending_input_started = None;
    }

    pub fn clipboard_text(&self) -> Option<String> {
        self.video.clipboard().clipboard_text().ok()
    }

    pub fn set_clipboard_text(&self, text: &str) {
        let _ = self.video.clipboard().set_clipboard_text(text);
    }

    pub fn present(
        &mut self,
        id: SurfaceId,
        commands: &[PaintCommand],
    ) -> Result<DamageRegion, String> {
        let index = *self
            .surface_indices
            .get(&id.0)
            .ok_or_else(|| "unknown SDL shell surface".to_owned())?;
        if self.graphics.is_none() {
            self.graphics = Some(SharedSdlGraphics::new(self.surfaces[index].window())?);
        }
        let graphics = self
            .graphics
            .as_ref()
            .expect("shared GPU initialized")
            .clone();
        let entry = &mut self.surfaces[index];
        let warm = entry.presenter.is_some();
        if entry.presenter.is_none() {
            entry.presenter = Some(SdlGpuPresenter::new(&entry.window, graphics)?);
        }
        let started = Instant::now();
        let damage = entry
            .presenter
            .as_mut()
            .expect("shell presenter initialized")
            .present(&entry.window, commands)?;
        let elapsed_us = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        if warm {
            push_bounded(&mut self.warm_present_us, elapsed_us);
        }
        if let Some(input_started) = self.pending_input_started.take() {
            let input_us = input_started
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64;
            push_bounded(&mut self.input_to_present_us, input_us);
        }
        Ok(damage)
    }

    /// Presents a canonical UI host frame only when its semantic or paint generation changed.
    pub fn present_host_frame(
        &mut self,
        id: SurfaceId,
        token: HostChangeToken,
        commands: &[PaintCommand],
    ) -> Result<Option<DamageRegion>, String> {
        let index = *self
            .surface_indices
            .get(&id.0)
            .ok_or_else(|| "unknown SDL shell surface".to_owned())?;
        if self.surfaces[index].last_host_change_token == Some(token) {
            return Ok(None);
        }
        let damage = self.present(id, commands)?;
        self.surfaces[index].last_host_change_token = Some(token);
        Ok(Some(damage))
    }

    pub fn show(&mut self, id: SurfaceId) -> bool {
        self.surface_mut(id).is_some_and(|surface| {
            surface.last_host_change_token = None;
            if let Some(presenter) = surface.presenter.as_mut() {
                presenter.invalidate();
            }
            surface.window_mut().show()
        })
    }

    pub fn hide(&mut self, id: SurfaceId) -> bool {
        self.surface_mut(id).is_some_and(|surface| {
            let hidden = surface.window_mut().hide();
            if hidden && let Some(presenter) = surface.presenter.as_mut() {
                presenter.suspend();
            }
            hidden
        })
    }

    pub fn raise(&mut self, id: SurfaceId) -> bool {
        self.surface_mut(id)
            .is_some_and(|surface| surface.window_mut().raise())
    }

    pub fn raise_role(&mut self, role: SurfaceRole) -> bool {
        let ids = self
            .surfaces
            .iter()
            .filter(|surface| surface.role() == role)
            .map(ShellSurface::id)
            .collect::<Vec<_>>();
        let mut raised = false;
        for id in ids {
            if let Some(surface) = self.surface_mut(id) {
                raised |= surface.window_mut().raise();
            }
        }
        raised
    }

    pub fn start_text_input(&self, id: SurfaceId) -> bool {
        self.surface(id).is_some_and(|surface| {
            self.video.text_input().start(surface.window());
            true
        })
    }

    pub fn poll_events(&mut self) -> Vec<ShellEvent> {
        let mut translated = self.pending_events.drain(..).collect::<Vec<_>>();
        let raw = self.events.poll_iter().collect::<Vec<_>>();
        for event in raw {
            if let Some(event) = self.translate_event(event) {
                translated.push(event);
            }
            translated.extend(self.pending_events.drain(..));
        }
        translated
    }

    pub fn wait_event(&mut self) -> Option<ShellEvent> {
        loop {
            if let Some(event) = self.pending_events.pop_front() {
                return Some(event);
            }
            let event = self.events.wait_event();
            if let Some(event) = self.translate_event(event) {
                return Some(event);
            }
        }
    }

    pub fn wait_event_timeout(&mut self, timeout: Duration) -> Option<ShellEvent> {
        if let Some(event) = self.pending_events.pop_front() {
            return Some(event);
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = self.events.wait_event_timeout(remaining)?;
            if let Some(event) = self.translate_event(event) {
                return Some(event);
            }
            if Instant::now() >= deadline {
                if let Some(event) = self.pending_events.pop_front() {
                    return Some(event);
                }
                return None;
            }
        }
    }

    pub fn display_geometries(&self) -> Result<Vec<DisplayGeometry>, String> {
        self.video
            .displays()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|display| {
                let bounds = display.get_bounds().map_err(|error| error.to_string())?;
                let mode = display.get_mode().map_err(|error| error.to_string())?;
                Ok(DisplayGeometry {
                    x: bounds.x,
                    y: bounds.y,
                    width: u32::try_from(bounds.w).unwrap_or_default().max(1),
                    height: u32::try_from(bounds.h).unwrap_or_default().max(1),
                    scale: mode.pixel_density.max(1.0),
                })
            })
            .collect()
    }

    fn display_names(&self) -> Result<Vec<String>, String> {
        self.video
            .displays()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|display| display.get_name().map_err(|error| error.to_string()))
            .collect()
    }

    fn create_surface(
        &mut self,
        role: SurfaceRole,
        display_index: usize,
        geometry: DisplayGeometry,
        output_name: &str,
    ) -> Result<(), String> {
        let (base_title, x, y, width, height, hidden) = surface_geometry(role, geometry);
        let title = shell_surface_title(role, base_title, output_name);
        let application_id = match role {
            SurfaceRole::Desktop => SessionShellRole::Desktop.application_id(),
            SurfaceRole::Panel => SessionShellRole::Panel.application_id(),
            SurfaceRole::Launcher => SessionShellRole::Launcher.application_id(),
            SurfaceRole::ControlCenter => SessionShellRole::ControlCenter.application_id(),
            SurfaceRole::Notification => SessionShellRole::Notification.application_id(),
            SurfaceRole::WindowPreview => SessionShellRole::Preview.application_id(),
            SurfaceRole::WindowContextMenu => SessionShellRole::ContextMenu.application_id(),
            SurfaceRole::CodexProjectMenu => SessionShellRole::ProjectMenu.application_id(),
            SurfaceRole::Lock => SessionShellRole::Lock.application_id(),
            SurfaceRole::Screenshot => SessionShellRole::Screenshot.application_id(),
            SurfaceRole::CodexChat => unreachable!("chat surfaces are dynamic"),
        };
        let previous_app_id = sdl3::hint::get("SDL_APP_ID");
        sdl3::hint::set("SDL_APP_ID", application_id);
        let mut builder = self.video.window(&title, width, height);
        builder.position(x, y).high_pixel_density();
        if surface_is_borderless(role) {
            builder.borderless();
        }
        if matches!(
            role,
            SurfaceRole::WindowPreview | SurfaceRole::WindowContextMenu | SurfaceRole::Screenshot
        ) {
            builder.resizable();
        }
        // A hidden Wayland toplevel receives no initial configure, so SDL waits
        // for its timeout before returning from window creation. Map Linux
        // shell surfaces once, then `sync_visibility` immediately applies the
        // production visibility state after every role has registered.
        if hidden && cfg!(not(target_os = "linux")) {
            builder.hidden();
        }
        let window = builder.build().map_err(|error| error.to_string());
        if let Some(previous_app_id) = previous_app_id {
            sdl3::hint::set("SDL_APP_ID", &previous_app_id);
        }
        let mut window = window?;
        if role == SurfaceRole::Screenshot {
            window
                .set_minimum_size(720, 480)
                .map_err(|error| error.to_string())?;
            #[cfg(target_os = "windows")]
            if !crate::platform::configure_screenshot_window(&window) {
                tracing::warn!("failed to configure Nickel screenshot utility window");
            }
        }
        if role == SurfaceRole::CodexProjectMenu {
            self.video.text_input().start(&window);
        }
        if role == SurfaceRole::Launcher {
            self.video.text_input().start(&window);
        }
        if role == SurfaceRole::Lock {
            self.video.text_input().start(&window);
        }
        let id = SurfaceId(window.id());
        let index = self.surfaces.len();
        self.surface_indices.insert(id.0, index);
        self.surfaces.push(ShellSurface {
            id,
            role,
            display_index,
            output_name: output_name.to_owned(),
            display_connected: true,
            presenter: None,
            last_host_change_token: None,
            window,
        });
        Ok(())
    }

    fn translate_event(&mut self, event: Event) -> Option<ShellEvent> {
        if let Some(shortcut) = event.as_user_event_type::<crate::platform::GlobalShortcut>() {
            return Some(ShellEvent::GlobalShortcut(shortcut));
        }
        #[cfg(target_os = "linux")]
        if let Some(request) = event.as_user_event_type::<crate::platform::ShellTestRequest>() {
            return Some(ShellEvent::TestControl(request));
        }
        let surface = event
            .get_window_id()
            .map(SurfaceId)
            .filter(|id| self.surface_indices.contains_key(&id.0));
        if matches!(
            event,
            Event::Window {
                win_event: WindowEvent::FocusLost,
                ..
            }
        ) {
            let _ = self.input_adapter.normalize(&event);
        } else if !matches!(event, Event::Window { .. })
            && let Some(input) = self.input_adapter.normalize(&event)
        {
            let surface = surface?;
            return Some(ShellEvent::Input {
                surface,
                event: input,
            });
        }
        match event {
            Event::Quit { .. } => Some(ShellEvent::Quit),
            Event::Display { .. } => Some(ShellEvent::DisplayTopologyChanged),
            Event::Window {
                win_event,
                window_id,
                ..
            } => self.translate_window_event(SurfaceId(window_id), win_event),
            _ => None,
        }
    }

    fn translate_window_event(&self, surface: SurfaceId, event: WindowEvent) -> Option<ShellEvent> {
        if !self.surface_indices.contains_key(&surface.0) {
            return None;
        }
        match event {
            WindowEvent::Shown => Some(ShellEvent::Shown(surface)),
            WindowEvent::Hidden => Some(ShellEvent::Hidden(surface)),
            WindowEvent::CloseRequested => Some(ShellEvent::CloseRequested(surface)),
            WindowEvent::FocusGained => Some(ShellEvent::FocusChanged {
                surface,
                focused: true,
            }),
            WindowEvent::FocusLost => Some(ShellEvent::FocusChanged {
                surface,
                focused: false,
            }),
            WindowEvent::MouseEnter => Some(ShellEvent::PointerEntered {
                surface,
                entered: true,
            }),
            WindowEvent::MouseLeave => Some(ShellEvent::PointerEntered {
                surface,
                entered: false,
            }),
            WindowEvent::Resized(width, height) => Some(ShellEvent::LogicalResize {
                surface,
                width: u32::try_from(width).unwrap_or_default(),
                height: u32::try_from(height).unwrap_or_default(),
            }),
            WindowEvent::PixelSizeChanged(width, height) => Some(ShellEvent::PixelResize {
                surface,
                width: u32::try_from(width).unwrap_or_default(),
                height: u32::try_from(height).unwrap_or_default(),
                scale: self
                    .surface(surface)
                    .map(|surface| surface.window().display_scale())
                    .unwrap_or(1.0),
            }),
            WindowEvent::Exposed => Some(ShellEvent::Redraw(surface)),
            WindowEvent::DisplayChanged(_) => Some(ShellEvent::DisplayTopologyChanged),
            _ => None,
        }
    }
}

fn configure_input_hints() {
    sdl3::hint::set("SDL_VIDEO_ALLOW_SCREENSAVER", "1");
    sdl3::hint::set("SDL_MOUSE_FOCUS_CLICKTHROUGH", "1");
    // SDL translates a primary finger into the ordinary pointer path. Nickel
    // therefore applies production hit testing and reducers identically for
    // mouse and single-touch activation, without a second geometry model.
    sdl3::hint::set("SDL_TOUCH_MOUSE_EVENTS", "1");
}

fn shell_surface_title(role: SurfaceRole, title: &str, output_name: &str) -> String {
    if matches!(
        role,
        SurfaceRole::Desktop | SurfaceRole::Panel | SurfaceRole::Lock
    ) {
        let output_name = output_name
            .chars()
            .filter(|character| !character.is_control() && *character != ']')
            .collect::<String>();
        return format!("{title} [output={output_name}]");
    }
    title.to_owned()
}

fn surface_geometry(
    role: SurfaceRole,
    geometry: DisplayGeometry,
) -> (&'static str, i32, i32, u32, u32, bool) {
    match role {
        SurfaceRole::Desktop => (
            DESKTOP_TITLE,
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            false,
        ),
        SurfaceRole::Panel => (
            PANEL_TITLE,
            geometry.x,
            geometry.y + geometry.height.saturating_sub(PANEL_HEIGHT) as i32,
            geometry.width,
            PANEL_HEIGHT,
            false,
        ),
        SurfaceRole::Launcher => (
            LAUNCHER_TITLE,
            geometry.x + 18,
            geometry.y + geometry.height.saturating_sub(744) as i32,
            920.min(geometry.width),
            680.min(geometry.height.saturating_sub(PANEL_HEIGHT + 8)),
            cfg!(not(target_os = "linux")),
        ),
        SurfaceRole::ControlCenter => (
            CONTROL_CENTER_TITLE,
            geometry.x + geometry.width.saturating_sub(438) as i32,
            geometry.y + geometry.height.saturating_sub(672) as i32,
            420.min(geometry.width),
            600.min(geometry.height),
            true,
        ),
        SurfaceRole::Notification => (
            NOTIFICATION_TITLE,
            geometry.x + geometry.width.saturating_sub(438) as i32,
            geometry.y + 24,
            420.min(geometry.width),
            180.min(geometry.height),
            true,
        ),
        SurfaceRole::WindowPreview => (
            WINDOW_PREVIEW_TITLE,
            geometry.x,
            geometry.y,
            300.min(geometry.width),
            220.min(geometry.height),
            true,
        ),
        SurfaceRole::WindowContextMenu => (
            WINDOW_CONTEXT_MENU_TITLE,
            geometry.x,
            geometry.y,
            220.min(geometry.width),
            156.min(geometry.height),
            true,
        ),
        SurfaceRole::CodexProjectMenu => (
            CODEX_PROJECT_MENU_TITLE,
            geometry.x + geometry.width.saturating_sub(464) as i32,
            geometry.y + geometry.height.saturating_sub(476) as i32,
            360.min(geometry.width),
            420.min(geometry.height.saturating_sub(PANEL_HEIGHT)),
            true,
        ),
        SurfaceRole::Lock => (
            LOCK_TITLE,
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            true,
        ),
        SurfaceRole::Screenshot => (
            SCREENSHOT_TITLE,
            geometry.x + (geometry.width.saturating_sub(1200) / 2) as i32,
            geometry.y + (geometry.height.saturating_sub(760) / 2) as i32,
            1200.min(geometry.width),
            760.min(geometry.height),
            true,
        ),
        SurfaceRole::CodexChat => unreachable!("chat surfaces are created dynamically"),
    }
}

fn require_displays(displays: Vec<DisplayGeometry>) -> Result<Vec<DisplayGeometry>, String> {
    if displays.is_empty() {
        Err("SDL reported no displays; refusing to start a headless Nickel shell".into())
    } else {
        Ok(displays)
    }
}

#[cfg(test)]
mod runtime_diagnostics_tests {
    use super::{RUNTIME_SAMPLE_CAPACITY, push_bounded};
    use std::collections::VecDeque;

    #[test]
    fn runtime_samples_are_bounded_and_keep_the_newest_observations() {
        let mut samples = VecDeque::new();
        for sample in 0..(RUNTIME_SAMPLE_CAPACITY as u64 + 7) {
            push_bounded(&mut samples, sample);
        }
        assert_eq!(samples.len(), RUNTIME_SAMPLE_CAPACITY);
        assert_eq!(samples.front(), Some(&7));
        assert_eq!(samples.back(), Some(&(RUNTIME_SAMPLE_CAPACITY as u64 + 6)));
    }
}

fn surface_is_borderless(role: SurfaceRole) -> bool {
    // Linux shell roles are compositor-owned chrome. In particular, allowing SDL to decorate the
    // screenshot utility adds client-side shadow/titlebar extents to its Wayland geometry, so the
    // compositor can no longer translate renderer-owned local input targets correctly. Windows
    // intentionally keeps the screenshot utility as a conventional decorated tool window.
    role != SurfaceRole::Screenshot || cfg!(target_os = "linux")
}

#[cfg(target_os = "linux")]
fn process_rss_bytes() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_proc_status_rss(&status)
}

#[cfg(not(target_os = "linux"))]
fn process_rss_bytes() -> Option<usize> {
    None
}

pub(crate) fn parse_proc_status_rss(status: &str) -> Option<usize> {
    let kibibytes = status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?.trim();
        let number = value.strip_suffix("kB")?.trim().parse::<usize>().ok()?;
        Some(number)
    })?;
    kibibytes.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::{
        DESKTOP_TITLE, DisplayGeometry, LAUNCHER_TITLE, PANEL_TITLE, SurfaceRole,
        parse_proc_status_rss, require_displays, shell_surface_title, surface_geometry,
        surface_is_borderless,
    };

    #[test]
    fn linux_screenshot_shell_surface_has_no_client_decoration_extents() {
        if cfg!(target_os = "linux") {
            assert!(surface_is_borderless(SurfaceRole::Screenshot));
        }
    }

    #[test]
    fn primary_touch_uses_the_production_pointer_path() {
        super::configure_input_hints();
        assert_eq!(
            sdl3::hint::get("SDL_TOUCH_MOUSE_EVENTS").as_deref(),
            Some("1")
        );
    }

    #[test]
    fn shell_does_not_disable_compositor_idle_policy() {
        super::configure_input_hints();
        assert_eq!(
            sdl3::hint::get("SDL_VIDEO_ALLOW_SCREENSAVER").as_deref(),
            Some("1")
        );
    }

    #[test]
    fn rejects_a_headless_shell_startup() {
        assert_eq!(
            require_displays(Vec::new()).unwrap_err(),
            "SDL reported no displays; refusing to start a headless Nickel shell"
        );
    }

    #[test]
    fn accepts_visible_displays() {
        let display = DisplayGeometry {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
        };
        assert_eq!(require_displays(vec![display]).unwrap(), vec![display]);
    }

    #[test]
    fn proc_rss_is_allocator_visible_and_parsed_independently() {
        assert_eq!(
            parse_proc_status_rss("Name:\tnickel\nVmRSS:\t   12345 kB\nThreads:\t1\n"),
            Some(12_641_280)
        );
        assert_eq!(parse_proc_status_rss("VmRSS:\tunknown kB\n"), None);
        assert_eq!(parse_proc_status_rss("VmSize:\t123 kB\n"), None);
    }

    #[test]
    fn shell_surfaces_follow_updated_display_geometry() {
        let display = DisplayGeometry {
            x: 40,
            y: 20,
            width: 1920,
            height: 1006,
            scale: 1.5,
        };
        assert_eq!(
            surface_geometry(SurfaceRole::Desktop, display),
            ("Nickel Desktop", 40, 20, 1920, 1006, false)
        );
        assert_eq!(
            surface_geometry(SurfaceRole::Panel, display),
            ("Nickel Panel", 40, 970, 1920, 56, false)
        );
    }

    #[test]
    fn per_output_shell_titles_carry_sanitized_output_identity() {
        assert_eq!(
            shell_surface_title(SurfaceRole::Desktop, DESKTOP_TITLE, "DP-1"),
            "Nickel Desktop [output=DP-1]"
        );
        assert_eq!(
            shell_surface_title(SurfaceRole::Panel, PANEL_TITLE, "HDMI A/1"),
            "Nickel Panel [output=HDMI A/1]"
        );
        assert_eq!(
            shell_surface_title(SurfaceRole::Launcher, LAUNCHER_TITLE, "DP-1"),
            LAUNCHER_TITLE
        );
    }
}
