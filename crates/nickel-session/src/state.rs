use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    os::unix::net::UnixDatagram,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU32, Ordering},
    },
};

use nickel_core::{
    focus::FocusTransactions,
    hotkeys::{HotkeyAction, HotkeyController},
    launcher::{LauncherPointerTarget, LauncherVisibility},
    task_switcher::{SwitchWindow, TaskSwitchEffect, TaskSwitcher},
};
use nickel_session_protocol::{
    ClientEnvelope, Command as SessionCommand, ErrorCode, Event as SessionEvent,
    Geometry as ProtocolGeometry, OutputSnapshot, PreviewFrame as ProtocolPreview, Query, Request,
    SecureStorageState as ProtocolSecureStorage, ServerEnvelope, ServerMessage, ShellRole,
    Snapshot as SessionSnapshot, WindowAction as ProtocolWindowAction,
    WindowId as ProtocolWindowId, WindowSnapshot, decode, encode,
};
use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState},
    reexports::{
        calloop::{EventLoop, Interest, LoopSignal, Mode, PostAction, channel, generic::Generic},
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
        xdg_activation::XdgActivationState,
    },
};

use crate::{
    shell_layout::{self, Geometry},
    window_registry::{WindowId, WindowRegistry},
};

use crate::CalloopData;

fn protocol_error(code: ErrorCode, message: impl Into<String>) -> ServerMessage {
    ServerMessage::Error {
        code,
        message: message.into(),
    }
}

fn process_uid(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("Uid:\t"))
        .and_then(|value| value.split_whitespace().next())
        .map(str::to_owned)
}

fn same_session_user(pid: u32) -> bool {
    process_uid(pid).is_some_and(|uid| process_uid(std::process::id()).as_deref() == Some(&uid))
}

fn shell_registration_allowed(expected_pid: u32, claimed_pid: u32) -> bool {
    expected_pid != 0 && expected_pid == claimed_pid && same_session_user(claimed_pid)
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
    pub activation_state: XdgActivationState,
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
    launcher_focus: FocusTransactions<ObjectId>,
    launcher_restore_window: Option<WindowId>,
    launcher_subscribers: Vec<PathBuf>,
    protocol_token: String,
    authenticated_shell_pids: HashSet<u32>,
    test_control_enabled: bool,
    expected_shell_pid: Arc<AtomicU32>,
    pub launcher_show_requested_at: Option<std::time::Instant>,
    pub desktop_windows: Vec<Window>,
    pub panel_windows: Vec<Window>,
    pub utility_windows: Vec<Window>,
    pub context_menu_window: Option<Window>,
    pub server_decorated: HashSet<ObjectId>,
    pub primary_output_name: Option<String>,
    pub preview_frames: HashMap<WindowId, PreviewFrame>,
    pub preview_requests: HashSet<WindowId>,
    pub hotkeys: HotkeyController,
    pub task_switcher: TaskSwitcher<WindowId>,
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
    pub output_capture_request_id: Option<u64>,
    pub shell_failure_count: u8,
    control_socket_path: PathBuf,
    secure_storage_state: Arc<AtomicU8>,
    secure_storage_retry: Arc<std::sync::atomic::AtomicBool>,
    deferred_focus_restore: channel::Sender<WindowId>,
    #[cfg(feature = "backend-winit")]
    winit_redraw_window: Option<usize>,
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
    pub(crate) fn is_authenticated_shell_pid(&self, pid: u32) -> bool {
        self.expected_shell_pid.load(Ordering::Acquire) == pid
            && self.authenticated_shell_pids.contains(&pid)
    }

    pub fn new(
        event_loop: &mut EventLoop<CalloopData>,
        display: Display<Self>,
        test_control_enabled: bool,
    ) -> Self {
        let start_time = std::time::Instant::now();

        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let activation_state = XdgActivationState::new::<Self>(&dh);
        let decoration_state = XdgDecorationState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let _text_input_manager_state =
            smithay::wayland::text_input::TextInputManagerState::new::<Self>(&dh);
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

        let protocol_token = format!(
            "{:x}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        // SAFETY: session initialization is single-threaded and precedes shell launch.
        unsafe { std::env::set_var("NICKEL_SESSION_TOKEN", &protocol_token) };
        let control_socket_path = Self::init_control_socket(event_loop);
        let secure_storage_state = Arc::new(AtomicU8::new(
            crate::login_services::SecureStorageState::Starting as u8,
        ));
        let secure_storage_retry = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (deferred_focus_restore, deferred_focus_restore_rx) = channel::channel();
        event_loop
            .handle()
            .insert_source(deferred_focus_restore_rx, |event, _, data| {
                if let channel::Event::Msg(window) = event {
                    data.state.activate_window(window);
                    data.state.request_output_redraw();
                }
            })
            .expect("failed to register deferred focus restoration");

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
            activation_state,
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
            launcher_focus: FocusTransactions::default(),
            launcher_restore_window: None,
            launcher_subscribers: Vec::new(),
            protocol_token,
            authenticated_shell_pids: HashSet::new(),
            test_control_enabled,
            expected_shell_pid: Arc::new(AtomicU32::new(0)),
            launcher_show_requested_at: None,
            desktop_windows: Vec::new(),
            panel_windows: Vec::new(),
            utility_windows: Vec::new(),
            context_menu_window: None,
            server_decorated: HashSet::new(),
            primary_output_name: None,
            preview_frames: HashMap::new(),
            preview_requests: HashSet::new(),
            hotkeys: HotkeyController::default(),
            task_switcher: TaskSwitcher::default(),
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
            output_capture_request_id: None,
            shell_failure_count: 0,
            control_socket_path,
            secure_storage_state,
            secure_storage_retry,
            deferred_focus_restore,
            #[cfg(feature = "backend-winit")]
            winit_redraw_window: None,
        }
    }

    #[cfg(feature = "backend-winit")]
    pub fn set_winit_redraw_window(&mut self, window: &winit::window::Window) {
        self.winit_redraw_window = Some(std::ptr::from_ref(window) as usize);
    }

    #[cfg(feature = "backend-winit")]
    pub fn request_output_redraw(&self) {
        let Some(address) = self.winit_redraw_window else {
            return;
        };
        // SAFETY: Smithay owns this window in an Arc for exactly the lifetime
        // of the winit backend and this session state. Moving the backend does
        // not move the Arc allocation, and all calls occur on the event thread.
        unsafe { &*(address as *const winit::window::Window) }.request_redraw();
    }

    #[cfg(not(feature = "backend-winit"))]
    pub fn request_output_redraw(&self) {}

    pub fn secure_storage_state_handle(&self) -> Arc<AtomicU8> {
        Arc::clone(&self.secure_storage_state)
    }

    pub fn secure_storage_retry_handle(&self) -> Arc<std::sync::atomic::AtomicBool> {
        Arc::clone(&self.secure_storage_retry)
    }

    pub fn expected_shell_pid_handle(&self) -> Arc<AtomicU32> {
        self.expected_shell_pid.clone()
    }

    pub fn shell_recovery_visible(&self) -> bool {
        crate::shell_recovery_visible_for(self.shell_failure_count)
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
                    let mut frame = vec![0_u8; nickel_session_protocol::MAX_FRAME_BYTES];
                    while let Ok((length, source)) = socket.as_ref().recv_from(&mut frame) {
                        let request = decode::<ClientEnvelope>(&frame[..length]);
                        let (request_id, message) = match request {
                            Ok(envelope) => {
                                let request_id = envelope.request_id;
                                let message = data
                                    .state
                                    .handle_protocol_request(envelope, source.as_pathname());
                                (request_id, message)
                            }
                            Err(error) => (
                                0,
                                ServerMessage::Error {
                                    code: ErrorCode::IncompatibleVersion,
                                    message: error.to_string(),
                                },
                            ),
                        };
                        if let Some(path) = source.as_pathname()
                            && let Ok(response) = encode(&ServerEnvelope {
                                request_id,
                                message,
                            })
                        {
                            let _ = socket.as_ref().send_to(&response, path);
                        }
                        data.state.request_output_redraw();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .expect("failed to register Nickel session control socket");
        path
    }

    fn handle_protocol_request(
        &mut self,
        envelope: ClientEnvelope,
        source: Option<&std::path::Path>,
    ) -> ServerMessage {
        if envelope.token != self.protocol_token {
            return protocol_error(ErrorCode::Unauthorized, "invalid session capability");
        }
        match envelope.request {
            Request::RegisterShell { pid } => {
                if !shell_registration_allowed(self.expected_shell_pid.load(Ordering::Acquire), pid)
                {
                    return protocol_error(
                        ErrorCode::Unauthorized,
                        "shell process is outside the active user session",
                    );
                }
                self.authenticated_shell_pids.insert(pid);
                ServerMessage::Snapshot(self.protocol_snapshot())
            }
            Request::Subscribe => {
                let Some(path) = source else {
                    return protocol_error(ErrorCode::InvalidRequest, "subscriber has no path");
                };
                if !self.launcher_subscribers.iter().any(|entry| entry == path) {
                    if self.launcher_subscribers.len() >= nickel_session_protocol::MAX_SUBSCRIBERS {
                        return protocol_error(
                            ErrorCode::ResourceLimit,
                            "subscriber limit reached",
                        );
                    }
                    self.launcher_subscribers.push(path.to_path_buf());
                }
                ServerMessage::Event(SessionEvent::Snapshot(self.protocol_snapshot()))
            }
            Request::Query(query) => self.handle_protocol_query(query),
            Request::Command(command) => {
                self.handle_protocol_command(command, source, envelope.request_id)
            }
        }
    }

    fn handle_protocol_query(&mut self, query: Query) -> ServerMessage {
        match query {
            Query::Snapshot => ServerMessage::Snapshot(self.protocol_snapshot()),
            Query::Windows => ServerMessage::Windows(self.protocol_windows()),
            Query::Outputs => ServerMessage::Outputs(self.protocol_outputs()),
            Query::LauncherVisibility => ServerMessage::LauncherVisibility {
                visible: self.launcher_visibility.is_visible(),
            },
            Query::SecureStorage => ServerMessage::SecureStorage {
                state: self.protocol_secure_storage_state(),
            },
            Query::Preview { window } => {
                let id = WindowId(window.0);
                if !self.windows.snapshot().iter().any(|entry| entry.id == id) {
                    return protocol_error(ErrorCode::InvalidWindow, "unknown window id");
                }
                self.preview_requests.insert(id);
                let Some(frame) = self.preview_frames.get(&id) else {
                    return protocol_error(ErrorCode::InvalidRequest, "preview is not ready");
                };
                let preview = ProtocolPreview {
                    window,
                    width: frame.width,
                    height: frame.height,
                    rgba: frame.rgba.clone(),
                };
                if preview.validate().is_err() {
                    return protocol_error(ErrorCode::ResourceLimit, "preview exceeds bounds");
                }
                ServerMessage::Preview(preview)
            }
        }
    }

    fn handle_protocol_command(
        &mut self,
        command: SessionCommand,
        source: Option<&std::path::Path>,
        request_id: u64,
    ) -> ServerMessage {
        match command {
            SessionCommand::ToggleLauncher => self.toggle_launcher(),
            SessionCommand::SetLauncherVisible { visible } => self.set_launcher_visible(visible),
            SessionCommand::LogOut => self.loop_signal.stop(),
            SessionCommand::RetrySecureStorage => self
                .secure_storage_retry
                .store(true, std::sync::atomic::Ordering::Release),
            SessionCommand::HideOverlay => self.hide_context_menu(),
            SessionCommand::ShowOverlay { role, geometry } => match role {
                ShellRole::ContextMenu => {
                    self.show_context_menu(geometry.x, geometry.width, geometry.height, true)
                }
                ShellRole::Preview => {
                    self.show_context_menu(geometry.x, geometry.width, geometry.height, false)
                }
                _ => {
                    return protocol_error(
                        ErrorCode::InvalidRequest,
                        "role is not a transient overlay",
                    );
                }
            },
            SessionCommand::IdentifyOutputs => {
                self.identify_outputs_until =
                    Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
            }
            SessionCommand::CaptureOutput { path } => {
                if path.is_empty() {
                    return protocol_error(ErrorCode::InvalidRequest, "capture path is empty");
                }
                self.output_capture_path = Some(PathBuf::from(path));
                self.output_capture_reply_path = source.map(PathBuf::from);
                self.output_capture_request_id = Some(request_id);
            }
            SessionCommand::ApplyOutputs { layout } => {
                if let Err(error) = self.apply_output_layout(layout) {
                    return protocol_error(ErrorCode::InvalidRequest, error);
                }
            }
            SessionCommand::HighlightWindow { window } => {
                if let Some(window) = window
                    && !self.window_exists(window)
                {
                    return protocol_error(ErrorCode::InvalidWindow, "unknown window id");
                }
                self.preview_highlight = window.map(|id| WindowId(id.0));
            }
            SessionCommand::WindowAction { window, action } => {
                if !self.window_exists(window) {
                    return protocol_error(ErrorCode::InvalidWindow, "unknown window id");
                }
                let id = WindowId(window.0);
                match action {
                    ProtocolWindowAction::Activate => self.activate_window(id),
                    ProtocolWindowAction::Close => self.close_window(id),
                    ProtocolWindowAction::Minimize => self.minimize_window(id),
                    ProtocolWindowAction::MaximizeRestore => self.maximize_window(id),
                }
            }
            SessionCommand::TestInput { input } => {
                if !self.test_control_enabled {
                    return protocol_error(
                        ErrorCode::Unauthorized,
                        "test input control is not enabled for this session",
                    );
                }
                if let Err(error) = self.inject_test_input(input) {
                    return protocol_error(ErrorCode::InvalidRequest, error);
                }
            }
        }
        ServerMessage::Ack
    }

    pub fn complete_output_capture(
        &mut self,
        path: &std::path::Path,
        result: nickel_session_protocol::CaptureResult,
    ) {
        let Some(reply_path) = self.output_capture_reply_path.take() else {
            return;
        };
        let request_id = self.output_capture_request_id.take().unwrap_or_default();
        let message = ServerEnvelope {
            request_id,
            message: ServerMessage::Event(SessionEvent::OutputCaptureCompleted {
                path: path.to_string_lossy().into_owned(),
                result,
            }),
        };
        if let Ok(frame) = encode(&message)
            && let Ok(socket) = UnixDatagram::unbound()
        {
            let _ = socket.send_to(&frame, reply_path);
        }
    }

    fn window_exists(&self, id: ProtocolWindowId) -> bool {
        self.windows
            .snapshot()
            .iter()
            .any(|window| window.id.0 == id.0)
    }

    fn protocol_windows(&self) -> Vec<WindowSnapshot> {
        let shell_ids = self
            .shell_windows()
            .filter_map(|window| {
                self.surface_windows
                    .get(&window.toplevel()?.wl_surface().id())
            })
            .copied()
            .collect::<HashSet<_>>();
        self.windows
            .snapshot()
            .into_iter()
            .filter(|window| !shell_ids.contains(&window.id))
            .take(nickel_session_protocol::MAX_WINDOWS)
            .map(|window| WindowSnapshot {
                id: ProtocolWindowId(window.id.0),
                application_id: window.app_id.clone(),
                title: window.title.clone(),
                active: window.active,
                minimized: self.minimized_windows.contains_key(&window.id),
                maximized: false,
            })
            .collect()
    }

    fn protocol_outputs(&self) -> Vec<OutputSnapshot> {
        self.space
            .outputs()
            .filter_map(|output| {
                let geometry = self.space.output_geometry(output)?;
                let physical = output.physical_properties();
                Some(OutputSnapshot {
                    name: output.name(),
                    model: physical.model,
                    geometry: ProtocolGeometry {
                        x: geometry.loc.x,
                        y: geometry.loc.y,
                        width: geometry.size.w,
                        height: geometry.size.h,
                    },
                    work_area: {
                        let area = shell_layout::work_area(Geometry {
                            x: geometry.loc.x,
                            y: geometry.loc.y,
                            width: geometry.size.w,
                            height: geometry.size.h,
                        });
                        ProtocolGeometry {
                            x: area.x,
                            y: area.y,
                            width: area.width,
                            height: area.height,
                        }
                    },
                    physical_width_mm: physical.size.w,
                    physical_height_mm: physical.size.h,
                    primary: self.primary_output_name.as_deref() == Some(output.name().as_str()),
                })
            })
            .take(nickel_session_protocol::MAX_OUTPUTS)
            .collect()
    }

    fn protocol_snapshot(&self) -> SessionSnapshot {
        let windows = self.protocol_windows();
        SessionSnapshot {
            focused: windows
                .iter()
                .find(|window| window.active)
                .map(|window| window.id),
            stacking_front_to_back: windows.iter().map(|window| window.id).collect(),
            outputs: self.protocol_outputs(),
            windows,
            launcher_visible: self.launcher_visibility.is_visible(),
        }
    }

    fn protocol_secure_storage_state(&self) -> ProtocolSecureStorage {
        match self.secure_storage_state.load(Ordering::Acquire) {
            value if value == crate::login_services::SecureStorageState::Starting as u8 => {
                ProtocolSecureStorage::Starting
            }
            value if value == crate::login_services::SecureStorageState::Locked as u8 => {
                ProtocolSecureStorage::Locked
            }
            value if value == crate::login_services::SecureStorageState::PromptRequired as u8 => {
                ProtocolSecureStorage::PromptRequired
            }
            value if value == crate::login_services::SecureStorageState::Ready as u8 => {
                ProtocolSecureStorage::Ready
            }
            _ => ProtocolSecureStorage::Unavailable,
        }
    }

    pub fn toggle_launcher(&mut self) {
        let visible = self.launcher_visibility.toggle();
        self.hotkeys.launcher_visibility_applied(visible);
        self.apply_launcher_visibility(visible);
        if !visible {
            self.restore_launcher_focus();
        }
        self.notify_launcher_visibility(visible);
    }

    fn apply_output_layout(
        &mut self,
        layout: nickel_session_protocol::OutputLayout,
    ) -> Result<(), &'static str> {
        let primary = layout.primary;
        let mut placements = layout.placements;
        if primary.is_empty() {
            return Err("missing primary output");
        }
        if placements.is_empty() {
            return Err("output layout is empty");
        }
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
        self.notify_protocol_snapshot();
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
        let changed = self.launcher_visibility.is_visible() != visible;
        if changed && visible {
            self.launcher_show_requested_at = Some(std::time::Instant::now());
        }
        self.launcher_visibility.set(visible);
        self.hotkeys.launcher_visibility_applied(visible);
        self.apply_launcher_visibility(visible);
        if changed {
            if !visible {
                self.restore_launcher_focus();
            }
            self.notify_launcher_visibility(visible);
        }
    }

    pub fn launcher_pointer_press(
        &mut self,
        target: LauncherPointerTarget,
        restore_window_focus: bool,
    ) -> bool {
        if self.launcher_visibility.pointer_press(target) {
            self.hotkeys.launcher_visibility_applied(false);
            self.apply_launcher_visibility(false);
            if restore_window_focus {
                self.restore_launcher_focus();
            } else {
                self.launcher_restore_window = None;
            }
            self.notify_launcher_visibility(false);
            restore_window_focus
        } else {
            false
        }
    }

    pub fn launcher_keyboard_focus_changed(&mut self, focused: Option<&WlSurface>) {
        if let Some(focused) = focused
            && let Some(request) = self.launcher_focus.requested().cloned()
            && focused.id() == request.surface
        {
            let _ = self.launcher_focus.acknowledge(&request);
            return;
        }
        let Some(acknowledged) = self.launcher_focus.acknowledged().cloned() else {
            return;
        };
        if self.launcher_visibility.is_visible() && self.launcher_focus.loses_current(&acknowledged)
        {
            self.launcher_visibility.set(false);
            self.hotkeys.launcher_visibility_applied(false);
            self.apply_launcher_visibility(false);
            if let Some(window) = self.launcher_restore_window.take() {
                let _ = self.deferred_focus_restore.send(window);
            }
            self.notify_launcher_visibility(false);
        }
    }

    fn notify_launcher_visibility(&mut self, visible: bool) {
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::LauncherVisibility { visible }),
        }) else {
            return;
        };
        let Ok(socket) = UnixDatagram::unbound() else {
            return;
        };
        self.launcher_subscribers
            .retain(|path| socket.send_to(&event, path).is_ok());
        self.notify_protocol_snapshot();
    }

    pub(crate) fn notify_protocol_snapshot(&mut self) {
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::Snapshot(self.protocol_snapshot())),
        }) else {
            return;
        };
        let Ok(socket) = UnixDatagram::unbound() else {
            return;
        };
        self.launcher_subscribers
            .retain(|path| socket.send_to(&event, path).is_ok());
    }

    pub(crate) fn notify_preview_frame(&mut self, window: WindowId) {
        let Some(frame) = self.preview_frames.get(&window) else {
            return;
        };
        let preview = ProtocolPreview {
            window: ProtocolWindowId(window.0),
            width: frame.width,
            height: frame.height,
            rgba: frame.rgba.clone(),
        };
        if preview.validate().is_err() {
            return;
        }
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::Preview(preview)),
        }) else {
            return;
        };
        let Ok(socket) = UnixDatagram::unbound() else {
            return;
        };
        self.launcher_subscribers
            .retain(|path| socket.send_to(&event, path).is_ok());
    }

    fn apply_launcher_visibility(&mut self, visible: bool) {
        let Some(window) = self.launcher_window.clone() else {
            return;
        };
        if visible {
            if self.launcher_restore_window.is_none() {
                self.launcher_restore_window = self
                    .windows
                    .snapshot()
                    .into_iter()
                    .find(|window| window.active)
                    .map(|window| window.id);
            }
            let geometry = self.launcher_geometry(&window);
            self.space
                .map_element(window.clone(), (geometry.x, geometry.y), true);
            let surface = window.toplevel().unwrap().wl_surface().clone();
            let _request = self.launcher_focus.request(surface.id());
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

    fn restore_launcher_focus(&mut self) {
        if let Some(window) = self.launcher_restore_window.take() {
            self.activate_window(window);
        }
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
        self.notify_protocol_snapshot();
    }

    pub fn cycle_windows(&mut self, forward: bool) {
        self.apply_task_switch_action(if forward {
            HotkeyAction::SwitchNext
        } else {
            HotkeyAction::SwitchPrevious
        });
    }

    pub fn commit_window_cycle(&mut self) {
        self.apply_task_switch_action(HotkeyAction::CommitSwitch);
    }

    pub fn apply_task_switch_action(&mut self, action: HotkeyAction) {
        let shell_ids = self
            .shell_windows()
            .filter_map(|window| {
                self.surface_windows
                    .get(&window.toplevel()?.wl_surface().id())
            })
            .copied()
            .collect::<HashSet<_>>();
        let windows = self
            .windows
            .snapshot()
            .into_iter()
            .rev()
            .filter(|window| !shell_ids.contains(&window.id))
            .map(|window| SwitchWindow {
                id: window.id,
                application_id: window.app_id.clone(),
                active: window.active,
            })
            .collect::<Vec<_>>();
        let effects = self.task_switcher.apply(action, &windows);
        self.apply_task_switch_effects(effects);
    }

    pub(crate) fn remove_window_from_switcher(&mut self, id: WindowId) {
        self.preview_requests.remove(&id);
        self.preview_frames.remove(&id);
        let effects = self.task_switcher.remove_candidate(&id);
        self.apply_task_switch_effects(effects);
    }

    fn apply_task_switch_effects(&mut self, effects: Vec<TaskSwitchEffect<WindowId>>) {
        for effect in effects {
            match effect {
                TaskSwitchEffect::RequestPreviews(ids) => self.preview_requests.extend(ids),
                TaskSwitchEffect::ActivateWindow(id) => self.activate_window(id),
                TaskSwitchEffect::ShowFlip { .. }
                | TaskSwitchEffect::SelectPreview(_)
                | TaskSwitchEffect::HideFlip { .. } => {}
            }
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
        self.notify_protocol_snapshot();
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
        self.notify_protocol_snapshot();
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
        self.notify_protocol_snapshot();
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
        let width = if requested.w > 1 {
            requested.w.min(920)
        } else {
            920
        };
        let height = if requested.h > 1 {
            requested.h.min(680)
        } else {
            680
        };
        let work_area = shell_layout::work_area(self.output_geometry().unwrap_or(Geometry {
            x: 0,
            y: 0,
            width,
            height: height + shell_layout::PANEL_HEIGHT + 8,
        }));
        shell_layout::bottom_left_in(work_area, (width, height), 18, 8)
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
mod protocol_tests {
    use super::shell_registration_allowed;

    #[test]
    fn only_the_exact_supervised_shell_pid_can_register() {
        let current = std::process::id();
        assert!(shell_registration_allowed(current, current));
        assert!(!shell_registration_allowed(0, current));
        assert!(!shell_registration_allowed(
            current.saturating_add(1),
            current
        ));
        assert!(!shell_registration_allowed(current, u32::MAX));
    }
}
