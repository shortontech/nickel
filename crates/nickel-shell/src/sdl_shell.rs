//! SDL3 window and event ownership for the Nickel shell.
//!
//! Rendering is deliberately outside this module. A renderer receives stable
//! [`SurfaceId`] values and can attach either a software surface or an
//! accelerated backend without owning the application event pump.

use std::collections::HashMap;
use std::time::Duration;
use std::time::Instant;

use nickel_session_protocol::ShellRole as SessionShellRole;
use nickel_ui::{DamageRegion, PaintCommand};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::{Keycode, Mod};
use sdl3::mouse::MouseButton;
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
pub const PANEL_HEIGHT: u32 = 56;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SurfaceId(u32);

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
    Quit,
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
    PointerMoved {
        surface: SurfaceId,
        x: f32,
        y: f32,
    },
    PointerButton {
        surface: SurfaceId,
        button: MouseButton,
        pressed: bool,
        x: f32,
        y: f32,
    },
    MouseWheel {
        surface: SurfaceId,
        x: f32,
        y: f32,
    },
    Key {
        surface: SurfaceId,
        key: Option<Keycode>,
        modifiers: Mod,
        pressed: bool,
        repeat: bool,
    },
    Text {
        surface: SurfaceId,
        value: String,
    },
    Ime {
        surface: SurfaceId,
        value: String,
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
    // Drop the GPU surface before the native window whose handles it borrows.
    presenter: Option<SdlGpuPresenter>,
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
    video: sdl3::VideoSubsystem,
    _sdl: sdl3::Sdl,
    started: Instant,
}

impl SdlShell {
    pub fn new(started: Instant) -> Result<Self, String> {
        sdl3::hint::set("SDL_MOUSE_FOCUS_CLICKTHROUGH", "1");
        let sdl = sdl3::init().map_err(|error| error.to_string())?;
        let video = sdl.video().map_err(|error| error.to_string())?;
        sdl.event()
            .map_err(|error| error.to_string())?
            .register_custom_event::<crate::platform::GlobalShortcut>()
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
        for (display_index, geometry) in displays.iter().copied().enumerate() {
            if crate::platform::renders_desktop_background() {
                self.create_surface(SurfaceRole::Desktop, display_index, geometry)?;
            }
            self.create_surface(SurfaceRole::Panel, display_index, geometry)?;
        }
        let primary = displays[0];
        self.create_surface(SurfaceRole::Launcher, 0, primary)?;
        self.create_surface(SurfaceRole::ControlCenter, 0, primary)?;
        self.create_surface(SurfaceRole::Notification, 0, primary)?;
        self.create_surface(SurfaceRole::WindowPreview, 0, primary)?;
        self.create_surface(SurfaceRole::WindowContextMenu, 0, primary)?;
        self.create_surface(SurfaceRole::CodexProjectMenu, 0, primary)?;
        tracing::info!(
            elapsed_ms = self.started.elapsed().as_secs_f64() * 1_000.0,
            surface_count = self.surfaces.len(),
            "SDL shell windows created"
        );
        Ok(())
    }

    pub fn sync_display_geometry(&mut self) -> Result<(), String> {
        let displays = require_displays(self.display_geometries()?)?;
        let panel_count = self
            .surfaces
            .iter()
            .filter(|surface| surface.role == SurfaceRole::Panel)
            .count();
        if panel_count != displays.len() {
            return Err(format!(
                "display count changed from {panel_count} to {}; shell surface recreation is required",
                displays.len()
            ));
        }

        for surface in &mut self.surfaces {
            if matches!(
                surface.role,
                SurfaceRole::CodexChat
                    | SurfaceRole::WindowPreview
                    | SurfaceRole::WindowContextMenu
            ) {
                continue;
            }
            let Some(display) = displays.get(surface.display_index).copied() else {
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

    pub fn surfaces(&self) -> impl Iterator<Item = &ShellSurface> {
        self.surfaces.iter()
    }

    pub fn surface(&self, id: SurfaceId) -> Option<&ShellSurface> {
        self.surface_indices
            .get(&id.0)
            .and_then(|index| self.surfaces.get(*index))
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
            presenter: None,
            window,
        });
        Ok(id)
    }

    pub fn destroy_surface(&mut self, id: SurfaceId) {
        let Some(index) = self.surface_indices.remove(&id.0) else {
            return;
        };
        self.surfaces.remove(index);
        for (index, surface) in self.surfaces.iter().enumerate() {
            self.surface_indices.insert(surface.id().0, index);
        }
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
        if entry.presenter.is_none() {
            entry.presenter = Some(SdlGpuPresenter::new(&entry.window, graphics)?);
        }
        entry
            .presenter
            .as_mut()
            .expect("shell presenter initialized")
            .present(&entry.window, commands)
    }

    pub fn show(&mut self, id: SurfaceId) -> bool {
        self.surface_mut(id).is_some_and(|surface| {
            if let Some(presenter) = surface.presenter.as_mut() {
                presenter.invalidate();
            }
            surface.window_mut().show()
        })
    }

    pub fn hide(&mut self, id: SurfaceId) -> bool {
        self.surface_mut(id)
            .is_some_and(|surface| surface.window_mut().hide())
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
        let raw = self.events.poll_iter().collect::<Vec<_>>();
        raw.into_iter()
            .filter_map(|event| self.translate_event(event))
            .collect()
    }

    pub fn wait_event(&mut self) -> Option<ShellEvent> {
        loop {
            let event = self.events.wait_event();
            if let Some(event) = self.translate_event(event) {
                return Some(event);
            }
        }
    }

    pub fn wait_event_timeout(&mut self, timeout: Duration) -> Option<ShellEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = self.events.wait_event_timeout(remaining)?;
            if let Some(event) = self.translate_event(event) {
                return Some(event);
            }
            if Instant::now() >= deadline {
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

    fn create_surface(
        &mut self,
        role: SurfaceRole,
        display_index: usize,
        geometry: DisplayGeometry,
    ) -> Result<(), String> {
        let (title, x, y, width, height, hidden) = surface_geometry(role, geometry);
        let session_role = match role {
            SurfaceRole::Desktop => SessionShellRole::Desktop,
            SurfaceRole::Panel => SessionShellRole::Panel,
            SurfaceRole::Launcher => SessionShellRole::Launcher,
            SurfaceRole::ControlCenter => SessionShellRole::ControlCenter,
            SurfaceRole::Notification => SessionShellRole::Notification,
            SurfaceRole::WindowPreview => SessionShellRole::Preview,
            SurfaceRole::WindowContextMenu => SessionShellRole::ContextMenu,
            SurfaceRole::CodexProjectMenu => SessionShellRole::ProjectMenu,
            SurfaceRole::CodexChat => unreachable!("chat surfaces are dynamic"),
        };
        let previous_app_id = sdl3::hint::get("SDL_APP_ID");
        sdl3::hint::set("SDL_APP_ID", session_role.application_id());
        let mut builder = self.video.window(title, width, height);
        builder.position(x, y).high_pixel_density();
        builder.borderless();
        if matches!(
            role,
            SurfaceRole::WindowPreview | SurfaceRole::WindowContextMenu
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
        let window = window?;
        if role == SurfaceRole::CodexProjectMenu {
            self.video.text_input().start(&window);
        }
        if role == SurfaceRole::Launcher {
            self.video.text_input().start(&window);
        }
        let id = SurfaceId(window.id());
        let index = self.surfaces.len();
        self.surface_indices.insert(id.0, index);
        self.surfaces.push(ShellSurface {
            id,
            role,
            display_index,
            presenter: None,
            window,
        });
        Ok(())
    }

    fn translate_event(&self, event: Event) -> Option<ShellEvent> {
        if let Some(shortcut) = event.as_user_event_type::<crate::platform::GlobalShortcut>() {
            return Some(ShellEvent::GlobalShortcut(shortcut));
        }
        let surface = event
            .get_window_id()
            .map(SurfaceId)
            .filter(|id| self.surface_indices.contains_key(&id.0));
        match event {
            Event::Quit { .. } => Some(ShellEvent::Quit),
            Event::Display { .. } => Some(ShellEvent::DisplayTopologyChanged),
            Event::Window {
                win_event,
                window_id,
                ..
            } => self.translate_window_event(SurfaceId(window_id), win_event),
            Event::MouseMotion { x, y, .. } => Some(ShellEvent::PointerMoved {
                surface: surface?,
                x,
                y,
            }),
            Event::MouseButtonDown {
                mouse_btn, x, y, ..
            } => Some(ShellEvent::PointerButton {
                surface: surface?,
                button: mouse_btn,
                pressed: true,
                x,
                y,
            }),
            Event::MouseButtonUp {
                mouse_btn, x, y, ..
            } => Some(ShellEvent::PointerButton {
                surface: surface?,
                button: mouse_btn,
                pressed: false,
                x,
                y,
            }),
            Event::MouseWheel { x, y, .. } => Some(ShellEvent::MouseWheel {
                surface: surface?,
                x,
                y,
            }),
            Event::KeyDown {
                keycode,
                keymod,
                repeat,
                ..
            } => Some(ShellEvent::Key {
                surface: surface?,
                key: keycode,
                modifiers: keymod,
                pressed: true,
                repeat,
            }),
            Event::KeyUp {
                keycode,
                keymod,
                repeat,
                ..
            } => Some(ShellEvent::Key {
                surface: surface?,
                key: keycode,
                modifiers: keymod,
                pressed: false,
                repeat,
            }),
            Event::TextInput { text, .. } => Some(ShellEvent::Text {
                surface: surface?,
                value: text,
            }),
            Event::TextEditing { text, .. } => Some(ShellEvent::Ime {
                surface: surface?,
                value: text,
            }),
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
            140.min(geometry.height),
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
mod tests {
    use super::{DisplayGeometry, SurfaceRole, require_displays, surface_geometry};

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
}
