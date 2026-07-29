//! SDL3 window and event ownership for the Nickel shell.
//!
//! Rendering is deliberately outside this module. A renderer receives stable
//! [`SurfaceId`] values and can attach either a software surface or an
//! accelerated backend without owning the application event pump.

use std::collections::HashMap;
use std::time::Duration;
use std::time::Instant;

use nickel_components::{DamageRegion, PaintCommand, SdlCanvasPresenter};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::{Keycode, Mod};
use sdl3::mouse::MouseButton;
use sdl3::video::Window;

pub const DESKTOP_TITLE: &str = "Nickel Desktop";
pub const PANEL_TITLE: &str = "Nickel Panel";
pub const LAUNCHER_TITLE: &str = "Nickel Launcher";
pub const CONTROL_CENTER_TITLE: &str = "Nickel Control Center";
pub const PANEL_HEIGHT: u32 = 56;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SurfaceId(u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceRole {
    Desktop,
    Panel,
    Launcher,
    ControlCenter,
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
    window: Option<Window>,
    presenter: Option<SdlCanvasPresenter>,
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
        if let Some(presenter) = &self.presenter {
            presenter.window()
        } else {
            self.window.as_ref().expect("shell surface owns a window")
        }
    }

    pub fn window_mut(&mut self) -> &mut Window {
        if let Some(presenter) = &mut self.presenter {
            presenter.window_mut()
        } else {
            self.window.as_mut().expect("shell surface owns a window")
        }
    }

    fn ensure_presenter(&mut self) -> Result<&mut SdlCanvasPresenter, String> {
        if self.presenter.is_none() {
            let window = self.window.take().expect("pending shell window exists");
            self.presenter = Some(SdlCanvasPresenter::new(window)?);
        }
        Ok(self
            .presenter
            .as_mut()
            .expect("shell presenter initialized"))
    }
}

pub struct SdlShell {
    _sdl: sdl3::Sdl,
    video: sdl3::VideoSubsystem,
    events: sdl3::EventPump,
    surfaces: Vec<ShellSurface>,
    surface_indices: HashMap<u32, usize>,
    started: Instant,
}

impl SdlShell {
    pub fn new(started: Instant) -> Result<Self, String> {
        let sdl = sdl3::init().map_err(|error| error.to_string())?;
        let video = sdl.video().map_err(|error| error.to_string())?;
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
            _sdl: sdl,
            video,
            events,
            surfaces: Vec::new(),
            surface_indices: HashMap::new(),
            started,
        })
    }

    pub fn create_shell_surfaces(&mut self) -> Result<(), String> {
        self.surfaces.clear();
        self.surface_indices.clear();
        for (display_index, geometry) in self.display_geometries()?.into_iter().enumerate() {
            self.create_surface(SurfaceRole::Desktop, display_index, geometry)?;
            self.create_surface(SurfaceRole::Panel, display_index, geometry)?;
        }
        if let Some(primary) = self.display_geometries()?.first().copied() {
            self.create_surface(SurfaceRole::Launcher, 0, primary)?;
            self.create_surface(SurfaceRole::ControlCenter, 0, primary)?;
        }
        tracing::info!(
            elapsed_ms = self.started.elapsed().as_secs_f64() * 1_000.0,
            surface_count = self.surfaces.len(),
            "SDL shell windows created"
        );
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

    pub fn present(
        &mut self,
        id: SurfaceId,
        commands: &[PaintCommand],
    ) -> Result<DamageRegion, String> {
        let index = *self
            .surface_indices
            .get(&id.0)
            .ok_or_else(|| "unknown SDL shell surface".to_owned())?;
        let window = self.surfaces[index].window();
        let logical_width = window.size().0.max(1);
        let pixel_width = window.size_in_pixels().0;
        let scale = pixel_width as f32 / logical_width as f32;
        self.surfaces[index]
            .ensure_presenter()?
            .present_accelerated(commands, scale)
    }

    pub fn show(&mut self, id: SurfaceId) -> bool {
        self.surface_mut(id)
            .is_some_and(|surface| surface.window_mut().show())
    }

    pub fn hide(&mut self, id: SurfaceId) -> bool {
        self.surface_mut(id)
            .is_some_and(|surface| surface.window_mut().hide())
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
        let event = self.events.wait_event_timeout(timeout)?;
        self.translate_event(event)
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
        let (title, x, y, width, height, hidden) = match role {
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
                geometry.y + geometry.height.saturating_sub(632) as i32,
                760.min(geometry.width),
                560.min(geometry.height),
                true,
            ),
            SurfaceRole::ControlCenter => (
                CONTROL_CENTER_TITLE,
                geometry.x + geometry.width.saturating_sub(438) as i32,
                geometry.y + geometry.height.saturating_sub(672) as i32,
                420.min(geometry.width),
                600.min(geometry.height),
                true,
            ),
        };
        let mut builder = self.video.window(title, width, height);
        builder.position(x, y).borderless().high_pixel_density();
        if hidden {
            builder.hidden();
        }
        let window = builder.build().map_err(|error| error.to_string())?;
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
            window: Some(window),
            presenter: None,
        });
        Ok(())
    }

    fn translate_event(&self, event: Event) -> Option<ShellEvent> {
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
            Event::TextEditing { text, .. } => Some(ShellEvent::Text {
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
