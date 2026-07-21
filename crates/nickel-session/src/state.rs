use std::{
    collections::HashMap, ffi::OsString, os::unix::net::UnixDatagram, path::PathBuf, sync::Arc,
};

use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState},
    reexports::{
        calloop::{EventLoop, Interest, LoopSignal, Mode, PostAction, generic::Generic},
        wayland_server::{
            Display, DisplayHandle, Resource,
            backend::{ClientData, ClientId, DisconnectReason, ObjectId},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Point, Size},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::{ToplevelSurface, XdgShellState},
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};

use crate::{
    shell_layout::{self, Geometry},
    window_registry::{WindowId, WindowRegistry},
};

use crate::CalloopData;

pub struct NickelSession {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,

    pub space: Space<Window>,
    pub loop_signal: LoopSignal,

    // Smithay State
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<NickelSession>,
    pub data_device_state: DataDeviceState,
    pub popups: PopupManager,

    pub seat: Seat<Self>,
    pub windows: WindowRegistry,
    pub surface_windows: HashMap<ObjectId, WindowId>,
    pub launcher_window: Option<Window>,
    pub launcher_visible: bool,
    pub panel_window: Option<Window>,
    maximized_restore: HashMap<ObjectId, Geometry>,
    pub last_titlebar_click: Option<(ObjectId, u32, Point<f64, Logical>)>,
    pub suppress_left_button_release: bool,
    control_socket_path: PathBuf,
}

impl NickelSession {
    pub fn new(event_loop: &mut EventLoop<CalloopData>, display: Display<Self>) -> Self {
        let start_time = std::time::Instant::now();

        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let mut seat_state = SeatState::new();
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let popups = PopupManager::default();

        // A seat is a group of keyboards, pointer and touch devices.
        // A seat typically has a pointer and maintains a keyboard focus and a pointer focus.
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "winit");

        // Notify clients that we have a keyboard, for the sake of the example we assume that keyboard is always present.
        // You may want to track keyboard hot-plug in real compositor.
        seat.add_keyboard(Default::default(), 200, 25).unwrap();

        // Notify clients that we have a pointer (mouse)
        // Here we assume that there is always pointer plugged in
        seat.add_pointer();

        // A space represents a two-dimensional plane. Windows and Outputs can be mapped onto it.
        //
        // Windows get a position and stacking order through mapping.
        // Outputs become views of a part of the Space and can be rendered via Space::render_output.
        let space = Space::default();

        let control_socket_path = Self::init_control_socket(event_loop);

        let socket_name = Self::init_wayland_listener(display, event_loop);

        // Get the loop signal, used to stop the event loop
        let loop_signal = event_loop.get_signal();

        Self {
            start_time,
            display_handle: dh,

            space,
            loop_signal,
            socket_name,

            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            popups,
            seat,
            windows: WindowRegistry::default(),
            surface_windows: HashMap::new(),
            launcher_window: None,
            launcher_visible: false,
            panel_window: None,
            maximized_restore: HashMap::new(),
            last_titlebar_click: None,
            suppress_left_button_release: false,
            control_socket_path,
        }
    }

    fn init_control_socket(event_loop: &mut EventLoop<CalloopData>) -> PathBuf {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = runtime.join(format!("nickel-session-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let socket =
            UnixDatagram::bind(&path).expect("failed to bind Nickel session control socket");
        socket
            .set_nonblocking(true)
            .expect("failed to make Nickel session control socket nonblocking");

        // SAFETY: session initialization is single-threaded and happens before
        // the shell child is spawned.
        unsafe { std::env::set_var("NICKEL_SESSION_CONTROL", &path) };

        event_loop
            .handle()
            .insert_source(
                Generic::new(socket, Interest::READ, Mode::Level),
                |_, socket, data| {
                    let mut command = [0_u8; 64];
                    while let Ok(length) = socket.as_ref().recv(&mut command) {
                        match &command[..length] {
                            b"toggle-launcher" => data.state.toggle_launcher(),
                            b"hide-launcher" => data.state.set_launcher_visible(false),
                            b"show-launcher" => data.state.set_launcher_visible(true),
                            _ => {}
                        }
                    }
                    Ok(PostAction::Continue)
                },
            )
            .expect("failed to register Nickel session control socket");
        path
    }

    pub fn toggle_launcher(&mut self) {
        self.set_launcher_visible(!self.launcher_visible);
    }

    pub fn set_launcher_visible(&mut self, visible: bool) {
        let Some(window) = self.launcher_window.clone() else {
            return;
        };
        if visible {
            let geometry = self.launcher_geometry(&window);
            self.space
                .map_element(window, (geometry.x, geometry.y), true);
            if let Some(panel) = self.panel_window.clone() {
                self.space.raise_element(&panel, false);
            }
        } else {
            self.space.unmap_elem(&window);
        }
        self.launcher_visible = visible;
        eprintln!(
            "nickel-session: launcher {}",
            if visible { "shown" } else { "hidden" }
        );
    }

    pub fn register_launcher(&mut self, window: Window) {
        self.space.unmap_elem(&window);
        self.launcher_window = Some(window);
        self.launcher_visible = false;
    }

    pub fn register_panel(&mut self, window: Window) {
        // Smithay's ordinary xdg windows use z-index 30. Keep the Nickel panel
        // in its top shell layer so later application maps cannot cover it.
        window.override_z_index(40);
        self.panel_window = Some(window);
        self.relayout_shell_surfaces();
    }

    pub fn relayout_shell_surfaces(&mut self) {
        let Some(output) = self.output_geometry() else {
            return;
        };
        if let Some(panel) = self.panel_window.clone() {
            let geometry = shell_layout::panel(output);
            Self::configure_window(&panel, geometry);
            self.space
                .map_element(panel.clone(), (geometry.x, geometry.y), false);
            self.space.raise_element(&panel, false);
        }
        if self.launcher_visible
            && let Some(launcher) = self.launcher_window.clone()
        {
            let geometry = self.launcher_geometry(&launcher);
            self.space
                .map_element(launcher, (geometry.x, geometry.y), true);
            if let Some(panel) = self.panel_window.clone() {
                self.space.raise_element(&panel, false);
            }
        }
        self.relayout_maximized_windows();
    }

    pub fn maximize_toplevel(&mut self, surface: &ToplevelSurface) {
        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            surface.send_configure();
            return;
        };
        let Some(output) = self.output_geometry() else {
            surface.send_configure();
            return;
        };
        let location = self.space.element_location(&window).unwrap_or_default();
        let size = window.geometry().size;
        self.maximized_restore
            .entry(surface.wl_surface().id())
            .or_insert(Geometry {
                x: location.x,
                y: location.y,
                width: size.w.max(1),
                height: size.h.max(1),
            });

        let geometry = shell_layout::work_area(output);
        surface.with_pending_state(|state| {
            state
                .states
                .set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
            state.size = Some(Size::from((geometry.width, geometry.height)));
        });
        self.space
            .map_element(window, (geometry.x, geometry.y), true);
        self.raise_panel();
        surface.send_pending_configure();
    }

    pub fn unmaximize_toplevel(&mut self, surface: &ToplevelSurface) {
        let restore = self.maximized_restore.remove(&surface.wl_surface().id());
        let Some(restore) = restore else {
            return;
        };
        surface.with_pending_state(|state| {
            state
                .states
                .unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
            state.size = Some(Size::from((restore.width, restore.height)));
        });
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.space.map_element(window, (restore.x, restore.y), true);
        }
        self.raise_panel();
        surface.send_pending_configure();
    }

    pub fn toggle_maximized_toplevel(&mut self, surface: &ToplevelSurface) {
        if self
            .maximized_restore
            .contains_key(&surface.wl_surface().id())
        {
            self.unmaximize_toplevel(surface);
        } else {
            self.maximize_toplevel(surface);
        }
    }

    pub fn forget_toplevel_geometry(&mut self, surface: &ToplevelSurface) {
        self.maximized_restore.remove(&surface.wl_surface().id());
    }

    fn relayout_maximized_windows(&mut self) {
        let Some(output) = self.output_geometry() else {
            return;
        };
        let geometry = shell_layout::work_area(output);
        let maximized: Vec<_> = self
            .space
            .elements()
            .filter_map(|window| {
                let surface = window.toplevel()?.wl_surface();
                self.maximized_restore
                    .contains_key(&surface.id())
                    .then_some((window.clone(), window.toplevel()?.clone()))
            })
            .collect();
        for (window, surface) in maximized {
            Self::configure_window(&window, geometry);
            self.space
                .map_element(window, (geometry.x, geometry.y), true);
            surface.send_pending_configure();
        }
        self.raise_panel();
    }

    fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|window| window.toplevel().unwrap().wl_surface() == surface)
            .cloned()
    }

    fn raise_panel(&mut self) {
        if let Some(panel) = self.panel_window.clone() {
            self.space.raise_element(&panel, false);
        }
    }

    fn output_geometry(&self) -> Option<Geometry> {
        let output = self.space.outputs().next()?;
        let geometry = self.space.output_geometry(output)?;
        Some(Geometry {
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w,
            height: geometry.size.h,
        })
    }

    fn launcher_geometry(&self, launcher: &Window) -> Geometry {
        let requested = launcher.geometry().size;
        let width = if requested.w > 1 { requested.w } else { 960 };
        let height = if requested.h > 1 { requested.h } else { 640 };
        shell_layout::centered_in(
            shell_layout::work_area(self.output_geometry().unwrap_or(Geometry {
                x: 0,
                y: 0,
                width,
                height: height + shell_layout::PANEL_HEIGHT,
            })),
            (width, height),
        )
    }

    fn configure_window(window: &Window, geometry: Geometry) {
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|state| {
                state.size = Some(Size::from((geometry.width, geometry.height)));
            });
            toplevel.send_pending_configure();
        }
    }

    fn init_wayland_listener(
        display: Display<NickelSession>,
        event_loop: &mut EventLoop<CalloopData>,
    ) -> OsString {
        // Creates a new listening socket, automatically choosing the next available `wayland` socket name.
        let listening_socket = ListeningSocketSource::new_auto().unwrap();

        // Get the name of the listening socket.
        // Clients will connect to this socket.
        let socket_name = listening_socket.socket_name().to_os_string();

        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                // Inside the callback, you should insert the client into the display.
                //
                // You may also associate some data with the client when inserting the client.
                state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
            })
            .expect("Failed to init the wayland event source.");

        // You also need to add the display itself to the event loop, so that client events will be processed by wayland-server.
        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // Safety: we don't drop the display
                    unsafe {
                        display
                            .get_mut()
                            .dispatch_clients(&mut state.state)
                            .unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        socket_name
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.space
            .element_under(pos)
            .and_then(|(window, location)| {
                window
                    .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + location).to_f64()))
            })
    }
}

impl Drop for NickelSession {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.control_socket_path);
    }
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
