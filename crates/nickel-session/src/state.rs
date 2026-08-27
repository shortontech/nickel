use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    os::unix::net::UnixDatagram,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

use nickel_core::{hotkeys::HotkeyController, launcher::LauncherVisibility};
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
    utils::{Logical, Point, SERIAL_COUNTER, Size},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::{ToplevelSurface, XdgShellState, decoration::XdgDecorationState},
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};

use crate::{
    shell_layout::{self, Geometry},
    window_registry::{WindowId, WindowRegistry},
};

use crate::CalloopData;

fn parse_geometry_command(value: &str) -> Option<(i32, i32, i32)> {
    let mut fields = value.split('\t');
    Some((
        fields.next()?.parse().ok()?,
        fields.next()?.parse().ok()?,
        fields.next()?.parse().ok()?,
    ))
}

#[derive(Debug, Eq, PartialEq)]
struct OutputPlacement {
    name: String,
    x: i32,
    y: i32,
}

fn parse_output_layout(message: &str) -> Result<(String, Vec<OutputPlacement>), &'static str> {
    let mut lines = message.lines();
    if lines.next() != Some("apply-outputs") {
        return Err("invalid output command");
    }
    let primary = lines
        .next()
        .and_then(|line| line.strip_prefix("primary\t"))
        .filter(|name| !name.is_empty())
        .ok_or("missing primary output")?
        .to_owned();
    let mut placements = Vec::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let mut fields = line.split('\t');
        let name = fields
            .next()
            .filter(|name| !name.is_empty())
            .ok_or("missing output name")?;
        let x = fields
            .next()
            .and_then(|value| value.parse().ok())
            .ok_or("invalid output x")?;
        let y = fields
            .next()
            .and_then(|value| value.parse().ok())
            .ok_or("invalid output y")?;
        if fields.next().is_some() {
            return Err("too many output fields");
        }
        placements.push(OutputPlacement {
            name: name.to_owned(),
            x,
            y,
        });
    }
    if placements.is_empty() {
        return Err("output layout is empty");
    }
    Ok((primary, placements))
}

pub struct NickelSession {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,

    pub space: Space<Window>,
    pub loop_signal: LoopSignal,

    // Smithay State
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub decoration_state: XdgDecorationState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<NickelSession>,
    pub data_device_state: DataDeviceState,
    pub popups: PopupManager,

    pub seat: Seat<Self>,
    pub windows: WindowRegistry,
    pub surface_windows: HashMap<ObjectId, WindowId>,
    pub launcher_window: Option<Window>,
    pub launcher_visibility: LauncherVisibility,
    pub desktop_windows: Vec<Window>,
    pub panel_windows: Vec<Window>,
    pub utility_windows: Vec<Window>,
    pub context_menu_window: Option<Window>,
    pub server_decorated: HashSet<ObjectId>,
    pub primary_output_name: Option<String>,
    pub preview_frames: HashMap<WindowId, PreviewFrame>,
    pub preview_requests: HashSet<WindowId>,
    pub hotkeys: HotkeyController,
    pub alt_tab_order: Vec<WindowId>,
    pub alt_tab_index: usize,
    pub preview_highlight: Option<WindowId>,
    pub minimized_windows: HashMap<WindowId, (Window, Point<i32, Logical>)>,
    maximized_restore: HashMap<ObjectId, Geometry>,
    fullscreen_restore: HashMap<ObjectId, Geometry>,
    pub last_titlebar_click: Option<(ObjectId, u32, Point<f64, Logical>)>,
    pub suppress_left_button_release: bool,
    pub frame_cursor: crate::window_frame::FrameCursor,
    pub buffer_commit_tx: Option<smithay::reexports::calloop::channel::Sender<SurfaceBufferCommit>>,
    pub identify_outputs_until: Option<std::time::Instant>,
    pub output_capture_path: Option<PathBuf>,
    pub output_capture_reply_path: Option<PathBuf>,
    control_socket_path: PathBuf,
    secure_storage_state: Arc<AtomicU8>,
    secure_storage_retry: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone)]
pub struct PreviewFrame {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

pub struct SurfaceBufferCommit {
    pub surface: WlSurface,
    pub render_visible: bool,
}

impl NickelSession {
    pub fn new(event_loop: &mut EventLoop<CalloopData>, display: Display<Self>) -> Self {
        let start_time = std::time::Instant::now();

        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let decoration_state = XdgDecorationState::new::<Self>(&dh);
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
        seat.add_touch();

        // A space represents a two-dimensional plane. Windows and Outputs can be mapped onto it.
        //
        // Windows get a position and stacking order through mapping.
        // Outputs become views of a part of the Space and can be rendered via Space::render_output.
        let space = Space::default();

        let control_socket_path = Self::init_control_socket(event_loop);
        let secure_storage_state = Arc::new(AtomicU8::new(
            crate::login_services::SecureStorageState::Starting as u8,
        ));
        let secure_storage_retry = Arc::new(std::sync::atomic::AtomicBool::new(false));

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
            decoration_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            popups,
            seat,
            windows: WindowRegistry::default(),
            surface_windows: HashMap::new(),
            launcher_window: None,
            launcher_visibility: LauncherVisibility::default(),
            desktop_windows: Vec::new(),
            panel_windows: Vec::new(),
            utility_windows: Vec::new(),
            context_menu_window: None,
            server_decorated: HashSet::new(),
            primary_output_name: None,
            preview_frames: HashMap::new(),
            preview_requests: HashSet::new(),
            hotkeys: HotkeyController::default(),
            alt_tab_order: Vec::new(),
            alt_tab_index: 0,
            preview_highlight: None,
            minimized_windows: HashMap::new(),
            maximized_restore: HashMap::new(),
            fullscreen_restore: HashMap::new(),
            last_titlebar_click: None,
            suppress_left_button_release: false,
            frame_cursor: crate::window_frame::FrameCursor::Arrow,
            buffer_commit_tx: None,
            identify_outputs_until: None,
            output_capture_path: None,
            output_capture_reply_path: None,
            control_socket_path,
            secure_storage_state,
            secure_storage_retry,
        }
    }

    pub fn secure_storage_state_handle(&self) -> Arc<AtomicU8> {
        Arc::clone(&self.secure_storage_state)
    }

    pub fn secure_storage_retry_handle(&self) -> Arc<std::sync::atomic::AtomicBool> {
        Arc::clone(&self.secure_storage_retry)
    }

    fn secure_storage_state_payload(&self) -> &'static [u8] {
        match self.secure_storage_state.load(Ordering::Acquire) {
            value if value == crate::login_services::SecureStorageState::Starting as u8 => {
                b"starting"
            }
            value if value == crate::login_services::SecureStorageState::Locked as u8 => b"locked",
            value if value == crate::login_services::SecureStorageState::PromptRequired as u8 => {
                b"prompt-required"
            }
            value if value == crate::login_services::SecureStorageState::Ready as u8 => b"ready",
            _ => b"unavailable",
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
                    let mut command = [0_u8; 4096];
                    while let Ok((length, source)) = socket.as_ref().recv_from(&mut command) {
                        let message = &command[..length];
                        match message {
                            b"toggle-launcher" => data.state.toggle_launcher(),
                            b"hide-launcher" => data.state.set_launcher_visible(false),
                            b"show-launcher" => data.state.set_launcher_visible(true),
                            b"logout" => data.state.loop_signal.stop(),
                            b"launcher-visible" => {
                                if let Some(path) = source.as_pathname() {
                                    let visible = if data.state.launcher_visibility.is_visible() {
                                        b"1"
                                    } else {
                                        b"0"
                                    };
                                    let _ = socket.as_ref().send_to(visible, path);
                                }
                            }
                            b"secure-storage-state" => {
                                if let Some(path) = source.as_pathname() {
                                    let state = data.state.secure_storage_state_payload();
                                    let _ = socket.as_ref().send_to(state, path);
                                }
                            }
                            b"retry-secure-storage" => data
                                .state
                                .secure_storage_retry
                                .store(true, std::sync::atomic::Ordering::Release),
                            b"hide-context-menu" => data.state.hide_context_menu(),
                            b"list-windows" => {
                                if let Some(path) = source.as_pathname() {
                                    let snapshot = data.state.window_snapshot_payload();
                                    let _ = socket.as_ref().send_to(snapshot.as_bytes(), path);
                                }
                            }
                            b"list-outputs" => {
                                if let Some(path) = source.as_pathname() {
                                    let snapshot = data.state.output_snapshot_payload();
                                    let _ = socket.as_ref().send_to(snapshot.as_bytes(), path);
                                }
                            }
                            b"identify-outputs" => {
                                data.state.identify_outputs_until = Some(
                                    std::time::Instant::now() + std::time::Duration::from_secs(3),
                                );
                                #[cfg(feature = "backend-udev")]
                                data.render_all_outputs();
                                if let Some(path) = source.as_pathname() {
                                    let _ = socket.as_ref().send_to(b"ok", path);
                                }
                            }
                            _ => {
                                if let Ok(message) = std::str::from_utf8(message)
                                    && let Some(path) = message.strip_prefix("capture-output\t")
                                    && !path.is_empty()
                                {
                                    data.state.output_capture_path = Some(PathBuf::from(path));
                                    data.state.output_capture_reply_path =
                                        source.as_pathname().map(PathBuf::from);
                                    #[cfg(feature = "backend-udev")]
                                    data.render_all_outputs();
                                } else if let Ok(message) = std::str::from_utf8(message)
                                    && message.starts_with("apply-outputs\n")
                                {
                                    let response = match data.state.apply_output_layout(message) {
                                        Ok(()) => "ok".to_owned(),
                                        Err(error) => format!("error\t{error}"),
                                    };
                                    if let Some(path) = source.as_pathname() {
                                        let _ = socket.as_ref().send_to(response.as_bytes(), path);
                                    }
                                } else if let Ok(message) = std::str::from_utf8(message)
                                    && let Some((x, width, height)) = message
                                        .strip_prefix("show-context-menu\t")
                                        .and_then(parse_geometry_command)
                                {
                                    data.state.show_context_menu(x, width, height, true);
                                } else if let Ok(message) = std::str::from_utf8(message)
                                    && let Some((x, width, height)) = message
                                        .strip_prefix("show-preview\t")
                                        .and_then(parse_geometry_command)
                                {
                                    data.state.show_context_menu(x, width, height, false);
                                } else if let Ok(message) = std::str::from_utf8(message)
                                    && let Some(id) = message
                                        .strip_prefix("highlight-window\t")
                                        .and_then(|value| value.parse().ok())
                                {
                                    data.state.preview_highlight = Some(WindowId(id));
                                } else if message == b"clear-window-highlight" {
                                    data.state.preview_highlight = None;
                                } else if let Ok(message) = std::str::from_utf8(message)
                                    && let Some(id) = message
                                        .strip_prefix("get-preview\t")
                                        .and_then(|value| value.parse().ok())
                                {
                                    data.state.preview_requests.insert(WindowId(id));
                                    if let (Some(path), Some(frame)) = (
                                        source.as_pathname(),
                                        data.state.preview_frames.get(&WindowId(id)),
                                    ) {
                                        let mut payload = Vec::with_capacity(12 + frame.rgba.len());
                                        payload.extend_from_slice(&id.to_le_bytes());
                                        payload.extend_from_slice(&frame.width.to_le_bytes());
                                        payload.extend_from_slice(&frame.height.to_le_bytes());
                                        payload.extend_from_slice(&frame.rgba);
                                        let _ = socket.as_ref().send_to(&payload, path);
                                    }
                                } else if let Ok(message) = std::str::from_utf8(message)
                                    && let Some(id) = message
                                        .strip_prefix("activate-window\t")
                                        .and_then(|value| value.parse().ok())
                                {
                                    data.state.activate_window(WindowId(id));
                                } else if let Ok(message) = std::str::from_utf8(message)
                                    && let Some(id) = message
                                        .strip_prefix("maximize-window\t")
                                        .and_then(|value| value.parse().ok())
                                {
                                    data.state.maximize_window(WindowId(id));
                                } else if let Ok(message) = std::str::from_utf8(message)
                                    && let Some(id) = message
                                        .strip_prefix("minimize-window\t")
                                        .and_then(|value| value.parse().ok())
                                {
                                    data.state.minimize_window(WindowId(id));
                                } else if let Ok(message) = std::str::from_utf8(message)
                                    && let Some(id) = message
                                        .strip_prefix("close-window\t")
                                        .and_then(|value| value.parse().ok())
                                {
                                    data.state.close_window(WindowId(id));
                                }
                            }
                        }
                    }
                    Ok(PostAction::Continue)
                },
            )
            .expect("failed to register Nickel session control socket");
        path
    }

    pub fn toggle_launcher(&mut self) {
        let visible = self.launcher_visibility.toggle();
        self.hotkeys.launcher_visibility_applied(visible);
        self.apply_launcher_visibility(visible);
    }

    fn window_snapshot_payload(&self) -> String {
        let shell_ids = self
            .shell_windows()
            .filter_map(|window| {
                self.surface_windows
                    .get(&window.toplevel()?.wl_surface().id())
            })
            .copied()
            .collect::<Vec<_>>();
        self.windows
            .snapshot()
            .into_iter()
            .filter(|window| !shell_ids.contains(&window.id))
            .map(|window| {
                format!(
                    "{}\t{}\t{}\t{}\n",
                    window.id.0,
                    u8::from(window.active),
                    window.app_id.replace(['\t', '\n', '\r'], ""),
                    window.title.replace(['\t', '\n', '\r'], " ")
                )
            })
            .collect()
    }

    fn output_snapshot_payload(&self) -> String {
        self.space
            .outputs()
            .filter_map(|output| {
                let geometry = self.space.output_geometry(output)?;
                let physical = output.physical_properties();
                Some(format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    output.name(),
                    physical.model.replace(['\t', '\n', '\r'], " "),
                    geometry.loc.x,
                    geometry.loc.y,
                    geometry.size.w,
                    geometry.size.h,
                    physical.size.w,
                    physical.size.h,
                    u8::from(self.primary_output_name.as_deref() == Some(output.name().as_str()))
                ))
            })
            .collect()
    }

    fn apply_output_layout(&mut self, message: &str) -> Result<(), &'static str> {
        let (primary, mut placements) = parse_output_layout(message)?;
        let connected: HashMap<_, _> = self
            .space
            .outputs()
            .filter_map(|output| {
                self.space
                    .output_geometry(output)
                    .map(|geometry| (output.name(), (output.clone(), geometry.size)))
            })
            .collect();
        if placements.len() != connected.len() {
            return Err("layout must include every connected output");
        }
        let mut names = HashSet::new();
        if placements.iter().any(|placement| {
            !connected.contains_key(&placement.name) || !names.insert(&placement.name)
        }) {
            return Err("layout contains an unknown or duplicate output");
        }
        if !names.contains(&primary) {
            return Err("primary output is not connected");
        }
        let minimum_x = placements
            .iter()
            .map(|placement| placement.x)
            .min()
            .unwrap_or(0);
        let minimum_y = placements
            .iter()
            .map(|placement| placement.y)
            .min()
            .unwrap_or(0);
        for placement in &mut placements {
            placement.x -= minimum_x;
            placement.y -= minimum_y;
        }
        for (index, left) in placements.iter().enumerate() {
            let left_size = connected[&left.name].1;
            for right in placements.iter().skip(index + 1) {
                let right_size = connected[&right.name].1;
                let overlaps = left.x < right.x + right_size.w
                    && left.x + left_size.w > right.x
                    && left.y < right.y + right_size.h
                    && left.y + left_size.h > right.y;
                if overlaps {
                    return Err("outputs may touch but cannot overlap");
                }
            }
        }

        for placement in &placements {
            let (output, _) = &connected[&placement.name];
            let location = (placement.x, placement.y).into();
            output.change_current_state(None, None, None, Some(location));
            self.space.map_output(output, location);
        }
        self.primary_output_name = Some(primary);
        self.rescue_stranded_windows();
        self.relayout_shell_surfaces();
        Ok(())
    }

    fn rescue_stranded_windows(&mut self) {
        let outputs: Vec<_> = self
            .space
            .outputs()
            .filter_map(|output| self.space.output_geometry(output))
            .collect();
        let Some(fallback) = self
            .output_geometry()
            .map(|geometry| (geometry.x, geometry.y))
        else {
            return;
        };
        let stranded: Vec<_> = self
            .space
            .elements()
            .filter(|window| {
                self.space
                    .element_bbox(window)
                    .is_some_and(|bounds| !outputs.iter().any(|output| output.overlaps(bounds)))
            })
            .cloned()
            .collect();
        for window in stranded {
            self.space.map_element(window, fallback, false);
        }
    }

    pub fn set_launcher_visible(&mut self, visible: bool) {
        self.launcher_visibility.set(visible);
        self.hotkeys.launcher_visibility_applied(visible);
        self.apply_launcher_visibility(visible);
    }

    fn apply_launcher_visibility(&mut self, visible: bool) {
        let Some(window) = self.launcher_window.clone() else {
            return;
        };
        if visible {
            let geometry = self.launcher_geometry(&window);
            self.space
                .map_element(window.clone(), (geometry.x, geometry.y), true);
            let surface = window.toplevel().unwrap().wl_surface().clone();
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                Some(surface),
                SERIAL_COUNTER.next_serial(),
            );
            self.space.elements().for_each(|window| {
                window.toplevel().unwrap().send_pending_configure();
            });
            self.raise_panels();
        } else {
            self.space.unmap_elem(&window);
        }
        eprintln!(
            "nickel-session: launcher {}",
            if visible { "shown" } else { "hidden" }
        );
    }

    pub fn register_launcher(&mut self, window: Window) {
        self.space.unmap_elem(&window);
        self.launcher_window = Some(window);
        self.apply_launcher_visibility(self.launcher_visibility.is_visible());
    }

    pub fn register_panel(&mut self, window: Window) {
        // Smithay's ordinary xdg windows use z-index 30. Keep the Nickel panel
        // in its top shell layer so later application maps cannot cover it.
        window.override_z_index(40);
        if !self.panel_windows.contains(&window) {
            self.panel_windows.push(window);
        }
        self.relayout_shell_surfaces();
    }

    pub fn register_desktop(&mut self, window: Window) {
        window.override_z_index(0);
        if !self.desktop_windows.contains(&window) {
            self.desktop_windows.push(window);
        }
        self.relayout_shell_surfaces();
    }

    pub fn is_panel_window(&self, window: &Window) -> bool {
        self.panel_windows.contains(window)
    }

    pub fn is_fullscreen_window(&self, window: &Window) -> bool {
        window.toplevel().is_some_and(|surface| {
            self.fullscreen_restore
                .contains_key(&surface.wl_surface().id())
        })
    }

    pub fn is_maximized_window(&self, window: &Window) -> bool {
        window.toplevel().is_some_and(|surface| {
            self.maximized_restore
                .contains_key(&surface.wl_surface().id())
        })
    }

    pub fn is_server_decorated(&self, window: &Window) -> bool {
        window
            .toplevel()
            .is_some_and(|surface| self.server_decorated.contains(&surface.wl_surface().id()))
    }

    pub fn shell_windows(&self) -> impl Iterator<Item = &Window> {
        self.launcher_window
            .iter()
            .chain(self.desktop_windows.iter())
            .chain(self.panel_windows.iter())
            .chain(self.utility_windows.iter())
            .chain(self.context_menu_window.iter())
    }

    pub fn register_utility_window(&mut self, window: Window) {
        if !self.utility_windows.contains(&window) {
            self.utility_windows.push(window);
        }
    }

    pub fn register_context_menu(&mut self, window: Window) {
        window.override_z_index(50);
        self.context_menu_window = Some(window);
    }

    pub fn show_context_menu(
        &mut self,
        x: i32,
        requested_width: i32,
        requested_height: i32,
        focus: bool,
    ) {
        let (Some(window), Some(output)) =
            (self.context_menu_window.clone(), self.output_geometry())
        else {
            return;
        };
        let width = requested_width.clamp(120, output.width.max(120));
        let maximum_height = (output.height - shell_layout::PANEL_HEIGHT).max(52);
        let height = requested_height.clamp(52, maximum_height);
        let x = (output.x + x).clamp(output.x, output.x + output.width - width);
        let y = output.y + output.height - shell_layout::PANEL_HEIGHT - height - 4;
        Self::configure_window(
            &window,
            Geometry {
                x,
                y,
                width,
                height,
            },
        );
        self.space.map_element(window.clone(), (x, y), true);
        if focus {
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                Some(window.toplevel().unwrap().wl_surface().clone()),
                SERIAL_COUNTER.next_serial(),
            );
        }
        self.space.elements().for_each(|window| {
            window.toplevel().unwrap().send_pending_configure();
        });
        self.space.raise_element(&window, true);
        self.raise_panels();
        eprintln!("nickel-session: context menu shown at {x},{y}");
    }

    pub fn hide_context_menu(&mut self) {
        if let Some(window) = self.context_menu_window.clone() {
            self.space.unmap_elem(&window);
        }
        self.preview_highlight = None;
        eprintln!("nickel-session: context menu hidden");
    }

    pub fn close_window(&mut self, id: WindowId) {
        let surface_id = self
            .surface_windows
            .iter()
            .find_map(|(surface, window)| (*window == id).then_some(surface.clone()));
        let Some(surface_id) = surface_id else {
            return;
        };
        if let Some((window, _)) = self.minimized_windows.remove(&id) {
            if let Some(surface) = window.toplevel() {
                surface.send_close();
            }
            self.hide_context_menu();
            return;
        }
        if let Some(window) = self.space.elements().find(|window| {
            window
                .toplevel()
                .is_some_and(|surface| surface.wl_surface().id() == surface_id)
        }) && let Some(surface) = window.toplevel()
        {
            surface.send_close();
        }
        self.hide_context_menu();
    }

    pub fn activate_window(&mut self, id: WindowId) {
        if let Some((window, location)) = self.minimized_windows.remove(&id) {
            self.space.map_element(window, location, true);
        }
        let Some(surface_id) = self
            .surface_windows
            .iter()
            .find_map(|(surface, window)| (*window == id).then_some(surface.clone()))
        else {
            return;
        };
        let Some(window) = self
            .space
            .elements()
            .find(|window| {
                window
                    .toplevel()
                    .is_some_and(|surface| surface.wl_surface().id() == surface_id)
            })
            .cloned()
        else {
            return;
        };
        self.space.raise_element(&window, true);
        self.windows.raise(id);
        self.seat.get_keyboard().unwrap().set_focus(
            self,
            Some(window.toplevel().unwrap().wl_surface().clone()),
            SERIAL_COUNTER.next_serial(),
        );
        self.space.elements().for_each(|window| {
            window.toplevel().unwrap().send_pending_configure();
        });
        self.raise_panels();
    }

    pub fn cycle_windows(&mut self, forward: bool) {
        if self.alt_tab_order.is_empty() {
            let shell_ids = self
                .shell_windows()
                .filter_map(|window| {
                    self.surface_windows
                        .get(&window.toplevel()?.wl_surface().id())
                })
                .copied()
                .collect::<HashSet<_>>();
            self.alt_tab_order = self
                .windows
                .snapshot()
                .into_iter()
                .rev()
                .map(|window| window.id)
                .filter(|id| !shell_ids.contains(id))
                .collect();
            self.preview_requests
                .extend(self.alt_tab_order.iter().copied());
            self.alt_tab_index = if forward {
                usize::from(self.alt_tab_order.len() > 1)
            } else {
                self.alt_tab_order.len().saturating_sub(1)
            };
        } else if !self.alt_tab_order.is_empty() {
            self.alt_tab_index = if forward {
                (self.alt_tab_index + 1) % self.alt_tab_order.len()
            } else {
                self.alt_tab_index
                    .checked_sub(1)
                    .unwrap_or(self.alt_tab_order.len() - 1)
            };
        }
    }

    pub fn commit_window_cycle(&mut self) {
        let target = self.alt_tab_order.get(self.alt_tab_index).copied();
        self.alt_tab_order.clear();
        self.alt_tab_index = 0;
        if let Some(id) = target {
            self.activate_window(id);
        }
    }

    pub fn minimize_window(&mut self, id: WindowId) {
        let Some(window) = self
            .space
            .elements()
            .find(|window| {
                window
                    .toplevel()
                    .and_then(|surface| self.surface_windows.get(&surface.wl_surface().id()))
                    .copied()
                    == Some(id)
            })
            .cloned()
        else {
            return;
        };
        let location = self.space.element_location(&window).unwrap_or_default();
        self.space.unmap_elem(&window);
        self.minimized_windows.insert(id, (window, location));
        self.raise_panels();
    }

    pub fn maximize_window(&mut self, id: WindowId) {
        self.activate_window(id);
        let surface = self.space.elements().find_map(|window| {
            let surface = window.toplevel()?;
            (self
                .surface_windows
                .get(&surface.wl_surface().id())
                .copied()
                == Some(id))
            .then(|| surface.clone())
        });
        if let Some(surface) = surface {
            self.toggle_maximized_toplevel(&surface);
        }
    }

    pub fn relayout_shell_surfaces(&mut self) {
        if self.output_geometry().is_none() {
            return;
        }
        let mut outputs = self.space.outputs().cloned().collect::<Vec<_>>();
        // nickel creates panel surfaces in monitor-name order. Preserve the
        // same stable order here so differently sized outputs receive their
        // own panel rather than whichever surface happened to commit last.
        outputs.sort_by_key(|output| output.name());
        for (desktop, output) in self.desktop_windows.clone().into_iter().zip(&outputs) {
            let Some(geometry) = self.space.output_geometry(output) else {
                continue;
            };
            let geometry = Geometry {
                x: geometry.loc.x,
                y: geometry.loc.y,
                width: geometry.size.w,
                height: geometry.size.h,
            };
            Self::configure_window(&desktop, geometry);
            self.space
                .map_element(desktop, (geometry.x, geometry.y), false);
        }
        for (panel, output) in self.panel_windows.clone().into_iter().zip(outputs) {
            let Some(output) = self.space.output_geometry(&output) else {
                continue;
            };
            let output = Geometry {
                x: output.loc.x,
                y: output.loc.y,
                width: output.size.w,
                height: output.size.h,
            };
            let geometry = shell_layout::panel(output);
            Self::configure_window(&panel, geometry);
            self.space
                .map_element(panel.clone(), (geometry.x, geometry.y), false);
            self.space.raise_element(&panel, false);
        }
        if self.launcher_visibility.is_visible()
            && let Some(launcher) = self.launcher_window.clone()
        {
            let geometry = self.launcher_geometry(&launcher);
            self.space
                .map_element(launcher, (geometry.x, geometry.y), true);
            self.raise_panels();
        }
        self.relayout_maximized_windows();
    }

    pub fn maximize_toplevel(&mut self, surface: &ToplevelSurface) {
        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            surface.send_configure();
            return;
        };
        let Some(output) = self.output_geometry_for_window(&window) else {
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

        let work_area = self.work_area_for_output(output);
        let geometry = if self.server_decorated.contains(&surface.wl_surface().id()) {
            decorated_content_geometry(work_area)
        } else {
            work_area
        };
        surface.with_pending_state(|state| {
            state
                .states
                .set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
            state.size = Some(Size::from((geometry.width, geometry.height)));
        });
        self.space
            .map_element(window, (geometry.x, geometry.y), true);
        self.raise_panels();
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
        self.raise_panels();
        surface.send_pending_configure();
    }

    pub fn fullscreen_toplevel(&mut self, surface: &ToplevelSurface) {
        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            surface.send_configure();
            return;
        };
        let Some(output) = self.output_geometry_for_window(&window) else {
            surface.send_configure();
            return;
        };
        let location = self.space.element_location(&window).unwrap_or_default();
        let size = window.geometry().size;
        self.fullscreen_restore
            .entry(surface.wl_surface().id())
            .or_insert(Geometry {
                x: location.x,
                y: location.y,
                width: size.w.max(1),
                height: size.h.max(1),
            });
        surface.with_pending_state(|state| {
            state
                .states
                .set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen);
            state.size = Some(Size::from((output.width, output.height)));
        });
        window.override_z_index(45);
        self.space.map_element(window, (output.x, output.y), true);
        surface.send_pending_configure();
    }

    pub fn unfullscreen_toplevel(&mut self, surface: &ToplevelSurface) {
        let Some(restore) = self.fullscreen_restore.remove(&surface.wl_surface().id()) else {
            return;
        };
        surface.with_pending_state(|state| {
            state
                .states
                .unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen);
            state.size = Some(Size::from((restore.width, restore.height)));
        });
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            window.override_z_index(30);
            self.space.map_element(window, (restore.x, restore.y), true);
        }
        self.raise_panels();
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
        self.fullscreen_restore.remove(&surface.wl_surface().id());
    }

    fn relayout_maximized_windows(&mut self) {
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
            let Some(output) = self.output_geometry_for_window(&window) else {
                continue;
            };
            let work_area = self.work_area_for_output(output);
            let geometry = if self.server_decorated.contains(&surface.wl_surface().id()) {
                decorated_content_geometry(work_area)
            } else {
                work_area
            };
            Self::configure_window(&window, geometry);
            self.space
                .map_element(window, (geometry.x, geometry.y), true);
            surface.send_pending_configure();
        }
        self.raise_panels();
    }

    fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|window| window.toplevel().unwrap().wl_surface() == surface)
            .cloned()
    }

    fn raise_panels(&mut self) {
        for panel in self.panel_windows.clone() {
            self.space.raise_element(&panel, false);
        }
    }

    fn output_geometry(&self) -> Option<Geometry> {
        let output = self
            .primary_output_name
            .as_ref()
            .and_then(|name| self.space.outputs().find(|output| output.name() == *name))
            .or_else(|| self.space.outputs().next())?;
        let geometry = self.space.output_geometry(output)?;
        Some(Geometry {
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w,
            height: geometry.size.h,
        })
    }

    fn output_geometry_for_window(&self, window: &Window) -> Option<Geometry> {
        let bounds = self.space.element_bbox(window)?;
        let window_geometry = Geometry {
            x: bounds.loc.x,
            y: bounds.loc.y,
            width: bounds.size.w,
            height: bounds.size.h,
        };
        let outputs: Vec<_> = self
            .space
            .outputs()
            .filter_map(|output| self.space.output_geometry(output))
            .map(|geometry| Geometry {
                x: geometry.loc.x,
                y: geometry.loc.y,
                width: geometry.size.w,
                height: geometry.size.h,
            })
            .collect();
        shell_layout::output_for_window(window_geometry, &outputs)
    }

    fn work_area_for_output(&self, output: Geometry) -> Geometry {
        if self.output_geometry() == Some(output) {
            shell_layout::work_area(output)
        } else {
            output
        }
    }

    pub(crate) fn output_geometry_for_shell(&self) -> Option<Geometry> {
        self.output_geometry()
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

fn decorated_content_geometry(frame: Geometry) -> Geometry {
    Geometry {
        x: frame.x + crate::window_frame::RESIZE_BORDER,
        y: frame.y + crate::window_frame::TITLEBAR_HEIGHT + crate::window_frame::RESIZE_BORDER,
        width: (frame.width - crate::window_frame::RESIZE_BORDER * 2).max(1),
        height: (frame.height
            - crate::window_frame::TITLEBAR_HEIGHT
            - crate::window_frame::RESIZE_BORDER * 2)
            .max(1),
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

#[cfg(test)]
mod tests {
    use super::{OutputPlacement, parse_output_layout};

    #[test]
    fn output_layout_command_parses_atomically() {
        let (primary, outputs) =
            parse_output_layout("apply-outputs\nprimary\tDP-3\nDVI-I-1\t0\t0\nDP-3\t1920\t120\n")
                .unwrap();

        assert_eq!(primary, "DP-3");
        assert_eq!(
            outputs,
            vec![
                OutputPlacement {
                    name: "DVI-I-1".into(),
                    x: 0,
                    y: 0,
                },
                OutputPlacement {
                    name: "DP-3".into(),
                    x: 1920,
                    y: 120,
                },
            ]
        );
    }

    #[test]
    fn output_layout_command_rejects_missing_primary() {
        assert!(parse_output_layout("apply-outputs\nDVI-I-1\t0\t0\n").is_err());
    }
}
