use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    os::fd::AsFd,
    os::fd::AsRawFd,
    os::unix::net::UnixDatagram,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU32, Ordering},
    },
};

use nickel_core::{
    focus::FocusTransactions,
    hotkeys::{HotkeyAction, HotkeyController},
    idle::{IdleController, IdleEffect, IdlePolicy},
    launcher::{LauncherPointerTarget, LauncherVisibility},
    shell_settings::ShellSettings,
    task_switcher::{SwitchWindow, TaskSwitchEffect, TaskSwitcher},
    workspaces::{WorkspaceError, WorkspaceId, WorkspaceTransition, Workspaces},
};
use nickel_session_protocol::{
    ClientEnvelope, Command as SessionCommand, ErrorCode, Event as SessionEvent,
    Geometry as ProtocolGeometry, OutputSnapshot, OutputTransform, PreviewFrame as ProtocolPreview,
    Query, Request, SecureStorageState as ProtocolSecureStorage, ServerEnvelope, ServerMessage,
    ShellRole, ShellSurfaceSnapshot, Snapshot as SessionSnapshot, TestOutput,
    WindowAction as ProtocolWindowAction, WindowId as ProtocolWindowId, WindowSnapshot,
    WorkspaceId as ProtocolWorkspaceId, WorkspaceSnapshot, WorkspaceState, decode, encode,
};
use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState},
    output::{Mode as OutputMode, Output, PhysicalProperties, Scale as OutputScale, Subpixel},
    reexports::{
        calloop::{EventLoop, Interest, LoopSignal, Mode, PostAction, channel, generic::Generic},
        wayland_server::{
            Display, DisplayHandle, Resource,
            backend::{ClientData, ClientId, DisconnectReason, GlobalId, ObjectId},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Point, SERIAL_COUNTER, Size, Transform},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        idle_inhibit::IdleInhibitManagerState,
        input_method::InputMethodManagerState,
        output::OutputManagerState,
        pointer_constraints::PointerConstraintsState,
        relative_pointer::RelativePointerManagerState,
        seat::WaylandFocus,
        selection::{data_device::DataDeviceState, primary_selection::PrimarySelectionState},
        shell::xdg::{ToplevelSurface, XdgShellState, decoration::XdgDecorationState},
        shm::ShmState,
        socket::ListeningSocketSource,
        xdg_activation::XdgActivationState,
        xwayland_shell::XWaylandShellState,
    },
    xwayland::{X11Wm, xwm::XwmId},
};

use crate::{
    shell_layout::{self, Geometry},
    window_registry::{WindowId, WindowRegistry},
};

fn protocol_error(code: ErrorCode, message: impl Into<String>) -> ServerMessage {
    ServerMessage::Error {
        code,
        message: message.into(),
    }
}

fn workspace_error(error: WorkspaceError) -> &'static str {
    match error {
        WorkspaceError::UnknownWorkspace => "unknown workspace",
        WorkspaceError::LastWorkspace => "cannot remove the last workspace",
        WorkspaceError::LimitReached => "workspace limit reached",
        WorkspaceError::UnknownWindow => "window has no workspace",
    }
}

fn clamp_window_location(
    location: Point<i32, Logical>,
    size: Size<i32, Logical>,
    work_area: Geometry,
) -> Point<i32, Logical> {
    (
        location
            .x
            .clamp(work_area.x, work_area.x + (work_area.width - size.w).max(0)),
        location.y.clamp(
            work_area.y,
            work_area.y + (work_area.height - size.h).max(0),
        ),
    )
        .into()
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

fn command_requires_shell_identity(command: &SessionCommand) -> bool {
    matches!(
        command,
        SessionCommand::LogOut | SessionCommand::Unlock | SessionCommand::SessionAction { .. }
    )
}

fn test_control_may_invoke(command: &SessionCommand) -> bool {
    matches!(
        command,
        SessionCommand::Unlock
            | SessionCommand::SessionAction {
                action: nickel_session_protocol::SessionAction::Lock
            }
    )
}

fn recv_control_frame(
    socket: &UnixDatagram,
    frame: &mut [u8],
) -> Result<(usize, Option<PathBuf>, u32), nix::errno::Errno> {
    use nix::sys::socket::{ControlMessageOwned, MsgFlags, UnixAddr, UnixCredentials, recvmsg};
    use std::io::IoSliceMut;

    let mut slices = [IoSliceMut::new(frame)];
    let mut credentials = nix::cmsg_space!(UnixCredentials);
    let message = recvmsg::<UnixAddr>(
        socket.as_raw_fd(),
        &mut slices,
        Some(&mut credentials),
        MsgFlags::MSG_DONTWAIT,
    )?;
    let length = message.bytes;
    let source = message
        .address
        .as_ref()
        .and_then(UnixAddr::path)
        .map(Path::to_path_buf);
    let peer_pid = message
        .cmsgs()?
        .find_map(|message| match message {
            ControlMessageOwned::ScmCredentials(credentials) => {
                u32::try_from(credentials.pid()).ok()
            }
            _ => None,
        })
        .ok_or(nix::errno::Errno::EACCES)?;
    Ok((length, source, peer_pid))
}

pub struct NickelSession {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub event_loop_handle: smithay::reexports::calloop::LoopHandle<'static, NickelSession>,
    #[cfg(feature = "backend-udev")]
    pub native: Option<crate::backend::udev::UdevData>,

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
    pub primary_selection_state: PrimarySelectionState,
    pub dnd_icon: Option<WlSurface>,
    pub relative_pointer_state: RelativePointerManagerState,
    pub pointer_constraints_state: PointerConstraintsState,
    pub(crate) pointer_lock_hints: HashMap<ObjectId, Point<f64, Logical>>,
    pub(crate) active_pointer_locks: HashSet<ObjectId>,
    pub idle_inhibit_state: IdleInhibitManagerState,
    pub input_method_state: InputMethodManagerState,
    pub xwayland_shell_state: XWaylandShellState,
    pub xwm: Option<(XwmId, X11Wm)>,
    pub xwayland_restart_pending: bool,
    pub xwayland_display: Option<u32>,
    pub xwayland_registration: Option<smithay::reexports::calloop::RegistrationToken>,
    pub popups: PopupManager,

    pub seat: Seat<Self>,
    pub windows: WindowRegistry,
    pub surface_windows: HashMap<ObjectId, WindowId>,
    pub x11_windows: HashMap<u32, WindowId>,
    pub launcher_window: Option<Window>,
    pub launcher_visibility: LauncherVisibility,
    launcher_output_name: Option<String>,
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
    pub lock_windows: Vec<Window>,
    pub locked: bool,
    lock_restore_window: Option<WindowId>,
    pub utility_windows: Vec<Window>,
    pub context_menu_window: Option<Window>,
    pub preview_window: Option<Window>,
    pub server_decorated: HashSet<ObjectId>,
    pub primary_output_name: Option<String>,
    virtual_test_outputs: HashMap<String, (Output, GlobalId)>,
    pub preview_frames: HashMap<WindowId, PreviewFrame>,
    pub preview_requests: HashSet<WindowId>,
    pub hotkeys: HotkeyController,
    pub task_switcher: TaskSwitcher<WindowId>,
    pub workspaces: Workspaces<WindowId>,
    pub workspace_hidden_windows: HashMap<WindowId, (Window, Point<i32, Logical>)>,
    displaced_output_windows: HashMap<String, Vec<DisplacedWindow>>,
    pub preview_highlight: Option<WindowId>,
    pub minimized_windows: HashMap<WindowId, (Window, Point<i32, Logical>)>,
    maximized_restore: HashMap<ObjectId, Geometry>,
    x11_maximized_restore: HashMap<u32, smithay::utils::Rectangle<i32, Logical>>,
    fullscreen_restore: HashMap<ObjectId, Geometry>,
    x11_fullscreen_restore: HashMap<u32, smithay::utils::Rectangle<i32, Logical>>,
    pub last_titlebar_click: Option<(ObjectId, u32, Point<f64, Logical>)>,
    pub suppress_left_button_release: bool,
    pub idle_inhibitors: HashMap<ObjectId, usize>,
    pub(crate) active_touch_slots: HashSet<smithay::backend::input::TouchSlot>,
    idle_controller: IdleController,
    pub dimmed: bool,
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
    shell_supervisor: Option<std::sync::mpsc::Sender<crate::ShellSupervisorCommand>>,
    #[cfg(feature = "backend-winit")]
    winit_redraw_window: Option<*const dyn smithay::reexports::winit::window::Window>,
}

#[derive(Clone)]
pub struct PreviewFrame {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy)]
struct DisplacedWindow {
    id: WindowId,
    relative_location: Point<i32, Logical>,
    rescue_location: Point<i32, Logical>,
}

pub struct SurfaceBufferCommit {
    pub surface: WlSurface,
    pub render_visible: bool,
}

impl NickelSession {
    pub(crate) fn note_input_activity(&mut self) {
        if self
            .idle_controller
            .note_activity(self.start_time.elapsed())
            == Some(IdleEffect::Undim)
        {
            self.dimmed = false;
            self.request_output_redraw();
            #[cfg(feature = "backend-udev")]
            if self.native.is_some() {
                self.render_all_outputs();
            }
        }
    }

    pub(crate) fn poll_idle_policy(&mut self) {
        let effects = self.idle_controller.poll(
            self.start_time.elapsed(),
            !self.idle_inhibitors.is_empty(),
            self.locked,
        );
        for effect in effects {
            match effect {
                IdleEffect::Dim => {
                    self.dimmed = true;
                    self.request_output_redraw();
                    #[cfg(feature = "backend-udev")]
                    if self.native.is_some() {
                        self.render_all_outputs();
                    }
                }
                IdleEffect::Undim => {
                    self.dimmed = false;
                    self.request_output_redraw();
                    #[cfg(feature = "backend-udev")]
                    if self.native.is_some() {
                        self.render_all_outputs();
                    }
                }
                IdleEffect::Lock => {
                    self.dimmed = false;
                    self.lock_session();
                }
                IdleEffect::Suspend => {
                    crate::session_services::request(
                        crate::session_services::SystemAction::Suspend,
                    );
                }
            }
        }
    }

    pub(crate) fn is_authenticated_shell_pid(&self, pid: u32) -> bool {
        self.expected_shell_pid.load(Ordering::Acquire) == pid
            && self.authenticated_shell_pids.contains(&pid)
    }

    pub fn new(
        event_loop: &mut EventLoop<'static, NickelSession>,
        display: Display<Self>,
        test_control_enabled: bool,
    ) -> Self {
        let start_time = std::time::Instant::now();
        let shell_settings = ShellSettings::load_default();
        let idle_controller = IdleController::new(
            IdlePolicy::from_seconds(
                shell_settings.idle_dim_seconds,
                shell_settings.idle_lock_seconds,
                shell_settings.idle_suspend_seconds,
            ),
            std::time::Duration::ZERO,
        );

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
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let relative_pointer_state = RelativePointerManagerState::new::<Self>(&dh);
        let pointer_constraints_state = PointerConstraintsState::new::<Self>(&dh);
        let idle_inhibit_state = IdleInhibitManagerState::new::<Self>(&dh);
        // The protocol itself permits only one active input method per seat.
        // Visibility is session-local: every connected client already passed
        // the compositor socket's same-user boundary.
        let input_method_state = InputMethodManagerState::new::<Self, _>(&dh, |_| true);
        let xwayland_shell_state = XWaylandShellState::new::<Self>(&dh);
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
                    data.activate_window(window);
                    data.request_output_redraw();
                }
            })
            .expect("failed to register deferred focus restoration");

        let socket_name = Self::init_wayland_listener(display, event_loop);

        // Get the loop signal, used to stop the event loop
        let loop_signal = event_loop.get_signal();

        Self {
            start_time,
            display_handle: dh,
            event_loop_handle: event_loop.handle(),
            #[cfg(feature = "backend-udev")]
            native: None,

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
            primary_selection_state,
            dnd_icon: None,
            relative_pointer_state,
            pointer_constraints_state,
            pointer_lock_hints: HashMap::new(),
            active_pointer_locks: HashSet::new(),
            idle_inhibit_state,
            input_method_state,
            xwayland_shell_state,
            xwm: None,
            xwayland_restart_pending: false,
            xwayland_display: None,
            xwayland_registration: None,
            popups,
            seat,
            windows: WindowRegistry::default(),
            surface_windows: HashMap::new(),
            x11_windows: HashMap::new(),
            launcher_window: None,
            launcher_visibility: LauncherVisibility::default(),
            launcher_output_name: None,
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
            lock_windows: Vec::new(),
            locked: false,
            lock_restore_window: None,
            utility_windows: Vec::new(),
            context_menu_window: None,
            preview_window: None,
            server_decorated: HashSet::new(),
            primary_output_name: None,
            virtual_test_outputs: HashMap::new(),
            preview_frames: HashMap::new(),
            preview_requests: HashSet::new(),
            hotkeys: HotkeyController::default(),
            task_switcher: TaskSwitcher::default(),
            workspaces: Workspaces::default(),
            workspace_hidden_windows: HashMap::new(),
            displaced_output_windows: HashMap::new(),
            preview_highlight: None,
            minimized_windows: HashMap::new(),
            maximized_restore: HashMap::new(),
            x11_maximized_restore: HashMap::new(),
            fullscreen_restore: HashMap::new(),
            x11_fullscreen_restore: HashMap::new(),
            last_titlebar_click: None,
            suppress_left_button_release: false,
            idle_inhibitors: HashMap::new(),
            active_touch_slots: HashSet::new(),
            idle_controller,
            dimmed: false,
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
            shell_supervisor: None,
            #[cfg(feature = "backend-winit")]
            winit_redraw_window: None,
        }
    }

    #[cfg(feature = "backend-winit")]
    pub fn set_winit_redraw_window(
        &mut self,
        window: &dyn smithay::reexports::winit::window::Window,
    ) {
        self.winit_redraw_window = Some(std::ptr::from_ref(window));
    }

    #[cfg(feature = "backend-winit")]
    pub fn request_output_redraw(&self) {
        let Some(window) = self.winit_redraw_window else {
            return;
        };
        // SAFETY: Smithay owns this window in an Arc for exactly the lifetime
        // of the winit backend and this session state. Moving the backend does
        // not move the Arc allocation, and all calls occur on the event thread.
        unsafe { &*window }.request_redraw();
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

    pub(crate) fn set_shell_supervisor(
        &mut self,
        supervisor: std::sync::mpsc::Sender<crate::ShellSupervisorCommand>,
    ) {
        self.shell_supervisor = Some(supervisor);
    }

    pub fn shell_recovery_visible(&self) -> bool {
        crate::shell_recovery_visible_for(self.shell_failure_count)
    }

    pub(crate) fn retry_shell_from_recovery(&mut self) -> bool {
        if !self.shell_recovery_visible() {
            return false;
        }
        let Some(supervisor) = &self.shell_supervisor else {
            return false;
        };
        if supervisor
            .send(crate::ShellSupervisorCommand::Restart)
            .is_err()
        {
            return false;
        }
        self.shell_failure_count = 0;
        self.request_output_redraw();
        true
    }

    pub(crate) fn exit_from_recovery(&mut self) -> bool {
        if !self.shell_recovery_visible() {
            return false;
        }
        self.loop_signal.stop();
        true
    }

    fn init_control_socket(event_loop: &mut EventLoop<'static, NickelSession>) -> PathBuf {
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
        nix::sys::socket::setsockopt(&socket, nix::sys::socket::sockopt::PassCred, &true)
            .expect("failed to enable Nickel session peer credentials");

        // SAFETY: session initialization is single-threaded and happens before
        // the shell child is spawned.
        unsafe { std::env::set_var("NICKEL_SESSION_CONTROL", &path) };

        event_loop
            .handle()
            .insert_source(
                Generic::new(socket, Interest::READ, Mode::Level),
                |_, socket, data| {
                    let mut frame = vec![0_u8; nickel_session_protocol::MAX_FRAME_BYTES];
                    while let Ok((length, source, peer_pid)) =
                        recv_control_frame(socket.as_ref(), &mut frame)
                    {
                        let request = decode::<ClientEnvelope>(&frame[..length]);
                        let (request_id, message) = match request {
                            Ok(envelope) => {
                                let request_id = envelope.request_id;
                                let message = data.handle_protocol_request(
                                    envelope,
                                    source.as_deref(),
                                    peer_pid,
                                );
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
                        if let Some(path) = source.as_deref() {
                            match encode(&ServerEnvelope {
                                request_id,
                                message,
                            }) {
                                Ok(response) => {
                                    if let Err(error) = socket.as_ref().send_to(&response, path) {
                                        tracing::warn!(
                                            ?error,
                                            ?path,
                                            "failed to reply on session control socket"
                                        );
                                    }
                                }
                                Err(error) => tracing::warn!(
                                    ?error,
                                    request_id,
                                    "failed to encode session control response"
                                ),
                            }
                        }
                        data.request_output_redraw();
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
        peer_pid: u32,
    ) -> ServerMessage {
        if envelope.token != self.protocol_token {
            return protocol_error(ErrorCode::Unauthorized, "invalid session capability");
        }
        match envelope.request {
            Request::RegisterShell { pid } => {
                if pid != peer_pid
                    || !shell_registration_allowed(
                        self.expected_shell_pid.load(Ordering::Acquire),
                        pid,
                    )
                {
                    return protocol_error(
                        ErrorCode::Unauthorized,
                        "shell process is outside the active user session",
                    );
                }
                self.authenticated_shell_pids.clear();
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
                if command_requires_shell_identity(&command)
                    && !self.authenticated_shell_pids.contains(&peer_pid)
                    && !(self.test_control_enabled && test_control_may_invoke(&command))
                {
                    return protocol_error(
                        ErrorCode::Unauthorized,
                        "command requires the authenticated Nickel shell process",
                    );
                }
                self.handle_protocol_command(command, source, envelope.request_id)
            }
        }
    }

    fn handle_protocol_query(&mut self, query: Query) -> ServerMessage {
        match query {
            Query::Snapshot => ServerMessage::Snapshot(self.protocol_snapshot()),
            Query::Windows => ServerMessage::Windows(
                self.protocol_windows()
                    .into_iter()
                    .filter(|window| window.workspace.0 == self.workspaces.active().0)
                    .collect(),
            ),
            Query::Outputs => ServerMessage::Outputs(self.protocol_outputs()),
            Query::ShellSurfaces => ServerMessage::ShellSurfaces(self.protocol_shell_surfaces()),
            Query::LauncherVisibility => ServerMessage::LauncherVisibility {
                visible: self.launcher_visibility.is_visible(),
            },
            Query::SecureStorage => ServerMessage::SecureStorage {
                state: self.protocol_secure_storage_state(),
            },
            Query::IdleInhibition => ServerMessage::IdleInhibition {
                surfaces: u16::try_from(self.idle_inhibitors.len()).unwrap_or(u16::MAX),
            },
            Query::CacheDiagnostics => {
                ServerMessage::CacheDiagnostics(nickel_session_protocol::CacheDiagnostics {
                    preview_entries: u16::try_from(self.preview_frames.len()).unwrap_or(u16::MAX),
                    preview_capacity: u16::try_from(nickel_session_protocol::MAX_WINDOWS)
                        .unwrap_or(u16::MAX),
                    preview_bytes: self
                        .preview_frames
                        .values()
                        .map(|frame| frame.rgba.len() as u64)
                        .sum(),
                })
            }
            Query::Workspaces => ServerMessage::Workspaces(self.protocol_workspaces()),
            Query::Preview { window } => {
                let id = WindowId(window.0);
                if !self.windows.snapshot().iter().any(|entry| entry.id == id) {
                    return protocol_error(ErrorCode::InvalidWindow, "unknown window id");
                }
                if !self.preview_requests.contains(&id)
                    && self.preview_requests.len() >= nickel_session_protocol::MAX_WINDOWS
                {
                    return protocol_error(ErrorCode::ResourceLimit, "preview cache is full");
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
            SessionCommand::SessionAction { action } => match action {
                nickel_session_protocol::SessionAction::RestartShell => {
                    let Some(supervisor) = &self.shell_supervisor else {
                        return protocol_error(
                            ErrorCode::InvalidRequest,
                            "shell supervisor is unavailable",
                        );
                    };
                    if supervisor
                        .send(crate::ShellSupervisorCommand::Restart)
                        .is_err()
                    {
                        return protocol_error(
                            ErrorCode::InvalidRequest,
                            "shell supervisor stopped",
                        );
                    }
                }
                nickel_session_protocol::SessionAction::Lock => {
                    self.lock_session();
                }
                nickel_session_protocol::SessionAction::Suspend => {
                    crate::session_services::request(
                        crate::session_services::SystemAction::Suspend,
                    );
                }
                nickel_session_protocol::SessionAction::Reboot => {
                    crate::session_services::request(crate::session_services::SystemAction::Reboot);
                }
                nickel_session_protocol::SessionAction::PowerOff => {
                    crate::session_services::request(
                        crate::session_services::SystemAction::PowerOff,
                    );
                }
            },
            SessionCommand::Unlock => self.unlock_session(),
            SessionCommand::RetrySecureStorage => self
                .secure_storage_retry
                .store(true, std::sync::atomic::Ordering::Release),
            SessionCommand::HideOverlay => self.hide_overlays(),
            SessionCommand::ShowOverlay {
                role,
                geometry,
                windows,
            } => match role {
                ShellRole::ContextMenu => {
                    if !windows.is_empty() {
                        return protocol_error(
                            ErrorCode::InvalidRequest,
                            "context menu does not accept preview windows",
                        );
                    }
                    self.show_context_menu(geometry.x, geometry.width, geometry.height, true)
                }
                ShellRole::Preview => {
                    if windows.len() > nickel_session_protocol::MAX_WINDOWS {
                        return protocol_error(
                            ErrorCode::ResourceLimit,
                            "too many preview windows",
                        );
                    }
                    if windows.iter().any(|window| !self.window_exists(*window)) {
                        return protocol_error(ErrorCode::InvalidWindow, "unknown preview window");
                    }
                    self.preview_requests =
                        windows.iter().map(|window| WindowId(window.0)).collect();
                    self.preview_frames
                        .retain(|window, _| self.preview_requests.contains(window));
                    self.show_preview(geometry.x, geometry.width, geometry.height)
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
            SessionCommand::CreateWorkspace => {
                if let Err(error) = self.workspaces.create() {
                    return protocol_error(ErrorCode::ResourceLimit, workspace_error(error));
                }
                self.notify_workspace_state();
                return ServerMessage::Workspaces(self.protocol_workspaces());
            }
            SessionCommand::RemoveWorkspace { workspace } => {
                let transition = match self.workspaces.remove(WorkspaceId(workspace.0)) {
                    Ok(transition) => transition,
                    Err(error) => {
                        return protocol_error(ErrorCode::InvalidRequest, workspace_error(error));
                    }
                };
                self.apply_workspace_transition(transition);
            }
            SessionCommand::SwitchWorkspace { workspace, output } => {
                if output.as_ref().is_some_and(|name| {
                    !self
                        .space
                        .outputs()
                        .any(|candidate| candidate.name() == *name)
                }) {
                    return protocol_error(ErrorCode::InvalidRequest, "unknown output");
                }
                let transition = match self.workspaces.switch_to(WorkspaceId(workspace.0), output) {
                    Ok(transition) => transition,
                    Err(error) => {
                        return protocol_error(ErrorCode::InvalidRequest, workspace_error(error));
                    }
                };
                self.apply_workspace_transition(transition);
            }
            SessionCommand::MoveWindowToWorkspace { window, workspace } => {
                let id = WindowId(window.0);
                let transition = match self.workspaces.move_window(&id, WorkspaceId(workspace.0)) {
                    Ok(transition) => transition,
                    Err(error) => {
                        return protocol_error(ErrorCode::InvalidRequest, workspace_error(error));
                    }
                };
                self.apply_workspace_transition(transition);
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
                    ProtocolWindowAction::FullscreenRestore => self.toggle_fullscreen_window(id),
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
            SessionCommand::TestOutput { output } => {
                if !self.test_control_enabled {
                    return protocol_error(
                        ErrorCode::Unauthorized,
                        "test output control is not enabled for this session",
                    );
                }
                if let Err(error) = self.apply_test_output(output) {
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
            .filter(|window| self.workspaces.is_visible(&window.id))
            .take(nickel_session_protocol::MAX_WINDOWS)
            .map(|window| {
                let surface = self
                    .surface_windows
                    .iter()
                    .find_map(|(surface, id)| (*id == window.id).then_some(surface));
                let geometry = surface
                    .and_then(|surface| {
                        self.space
                            .elements()
                            .find(|candidate| {
                                candidate
                                    .wl_surface()
                                    .is_some_and(|candidate| candidate.id() == *surface)
                            })
                            .and_then(|candidate| self.space.element_bbox(candidate))
                            .map(|bounds| ProtocolGeometry {
                                x: bounds.loc.x,
                                y: bounds.loc.y,
                                width: bounds.size.w,
                                height: bounds.size.h,
                            })
                    })
                    .or_else(|| {
                        self.workspace_hidden_windows
                            .get(&window.id)
                            .map(|(hidden, location)| {
                                let size = hidden.geometry().size;
                                ProtocolGeometry {
                                    x: location.x,
                                    y: location.y,
                                    width: size.w,
                                    height: size.h,
                                }
                            })
                    })
                    .or_else(|| {
                        self.minimized_windows
                            .get(&window.id)
                            .map(|(hidden, location)| {
                                let size = hidden.geometry().size;
                                ProtocolGeometry {
                                    x: location.x,
                                    y: location.y,
                                    width: size.w,
                                    height: size.h,
                                }
                            })
                    });
                WindowSnapshot {
                    id: ProtocolWindowId(window.id.0),
                    application_id: window.app_id.clone(),
                    title: window.title.clone(),
                    active: window.active,
                    minimized: self.minimized_windows.contains_key(&window.id),
                    maximized: surface
                        .is_some_and(|surface| self.maximized_restore.contains_key(surface))
                        || self.space.elements().any(|candidate| {
                            candidate.x11_surface().is_some_and(|x11| {
                                self.x11_windows.get(&x11.window_id()).copied() == Some(window.id)
                                    && x11.is_maximized()
                            })
                        }),
                    fullscreen: surface
                        .is_some_and(|surface| self.fullscreen_restore.contains_key(surface))
                        || self.space.elements().any(|candidate| {
                            candidate.x11_surface().is_some_and(|x11| {
                                self.x11_windows.get(&x11.window_id()).copied() == Some(window.id)
                                    && self.x11_fullscreen_restore.contains_key(&x11.window_id())
                            })
                        }),
                    geometry,
                    workspace: ProtocolWorkspaceId(
                        self.workspaces
                            .workspace_for(&window.id)
                            .unwrap_or(WorkspaceId(1))
                            .0,
                    ),
                }
            })
            .collect()
    }

    fn apply_test_output(&mut self, operation: TestOutput) -> Result<(), &'static str> {
        match operation {
            TestOutput::Connect {
                name,
                logical_width,
                logical_height,
                scale_120,
                transform,
            } => {
                if name.trim().is_empty() || name == "winit" {
                    return Err("test output needs a distinct non-empty name");
                }
                if self.space.outputs().any(|output| output.name() == name) {
                    return Err("output is already connected");
                }
                if self.space.outputs().count() >= nickel_session_protocol::MAX_OUTPUTS {
                    return Err("output limit reached");
                }
                if logical_width < 320 || logical_height < 240 || !(60..=480).contains(&scale_120) {
                    return Err("invalid test output geometry or scale");
                }
                let smithay_transform = match transform {
                    OutputTransform::Normal => Transform::Normal,
                    OutputTransform::Rotate90 => Transform::_90,
                    OutputTransform::Rotate180 => Transform::_180,
                    OutputTransform::Rotate270 => Transform::_270,
                    OutputTransform::Flipped => Transform::Flipped,
                    OutputTransform::Flipped90 => Transform::Flipped90,
                    OutputTransform::Flipped180 => Transform::Flipped180,
                    OutputTransform::Flipped270 => Transform::Flipped270,
                };
                let scale = f64::from(scale_120) / 120.0;
                let scaled_width = (f64::from(logical_width) * scale).round() as i32;
                let scaled_height = (f64::from(logical_height) * scale).round() as i32;
                let rotated = matches!(
                    smithay_transform,
                    Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270
                );
                let mode = OutputMode {
                    size: if rotated {
                        (scaled_height, scaled_width).into()
                    } else {
                        (scaled_width, scaled_height).into()
                    },
                    refresh: 60_000,
                };
                let x = self
                    .space
                    .outputs()
                    .filter_map(|output| self.space.output_geometry(output))
                    .map(|geometry| geometry.loc.x + geometry.size.w)
                    .max()
                    .unwrap_or(0);
                let output = Output::new(
                    name.clone(),
                    PhysicalProperties {
                        size: (0, 0).into(),
                        subpixel: Subpixel::Unknown,
                        make: "Nickel".into(),
                        model: "Nested test output".into(),
                        serial_number: name.clone(),
                    },
                );
                output.set_preferred(mode);
                output.change_current_state(
                    Some(mode),
                    Some(smithay_transform),
                    Some(OutputScale::Fractional(scale)),
                    Some((x, 0).into()),
                );
                let global = output.create_global::<NickelSession>(&self.display_handle);
                self.space.map_output(&output, (x, 0));
                self.restore_output_windows(&output);
                self.virtual_test_outputs.insert(name, (output, global));
            }
            TestOutput::Disconnect { name } => {
                let Some((output, global)) = self.virtual_test_outputs.remove(&name) else {
                    return Err("unknown virtual test output");
                };
                self.stage_output_removal(&output);
                self.space.unmap_output(&output);
                output.leave_all();
                self.display_handle.remove_global::<NickelSession>(global);
                self.reconcile_output_removal(&name);
                self.rescue_stranded_windows();
                self.relayout_maximized_windows();
                self.relayout_fullscreen_windows();
            }
        }
        self.relayout_shell_surfaces();
        self.request_output_redraw();
        self.notify_protocol_snapshot();
        Ok(())
    }

    pub(crate) fn reconcile_output_removal(&mut self, name: &str) {
        if self.primary_output_name.as_deref() == Some(name) {
            self.primary_output_name = self.space.outputs().next().map(|output| output.name());
        }
        if self.launcher_output_name.as_deref() == Some(name) {
            self.launcher_output_name = self.primary_output_name.clone();
        }
        self.workspaces
            .output_disconnected(name, self.primary_output_name.clone());
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
                    scale_120: (output.current_scale().fractional_scale() * 120.0).round() as u32,
                    transform: match output.current_transform() {
                        Transform::Normal => OutputTransform::Normal,
                        Transform::_90 => OutputTransform::Rotate90,
                        Transform::_180 => OutputTransform::Rotate180,
                        Transform::_270 => OutputTransform::Rotate270,
                        Transform::Flipped => OutputTransform::Flipped,
                        Transform::Flipped90 => OutputTransform::Flipped90,
                        Transform::Flipped180 => OutputTransform::Flipped180,
                        Transform::Flipped270 => OutputTransform::Flipped270,
                    },
                    physical_width_mm: physical.size.w,
                    physical_height_mm: physical.size.h,
                    primary: self.primary_output_name.as_deref() == Some(output.name().as_str()),
                })
            })
            .take(nickel_session_protocol::MAX_OUTPUTS)
            .collect()
    }

    fn protocol_shell_surfaces(&self) -> Vec<ShellSurfaceSnapshot> {
        let registry = self.windows.snapshot();
        self.shell_windows()
            .filter_map(|window| {
                let id = window
                    .wl_surface()
                    .and_then(|surface| self.surface_windows.get(&surface.id()))?;
                let app_id = registry
                    .iter()
                    .find(|entry| entry.id == *id)
                    .map(|entry| entry.app_id.as_str())?;
                let role = ShellRole::from_application_id(app_id)?;
                let bounds = self.space.element_bbox(window);
                let geometry = bounds.map(|bounds| ProtocolGeometry {
                    x: bounds.loc.x,
                    y: bounds.loc.y,
                    width: bounds.size.w,
                    height: bounds.size.h,
                });
                let output = bounds.and_then(|bounds| {
                    self.space
                        .outputs()
                        .filter_map(|output| {
                            let output_bounds = self.space.output_geometry(output)?;
                            let left = bounds.loc.x.max(output_bounds.loc.x);
                            let top = bounds.loc.y.max(output_bounds.loc.y);
                            let right = (bounds.loc.x + bounds.size.w)
                                .min(output_bounds.loc.x + output_bounds.size.w);
                            let bottom = (bounds.loc.y + bounds.size.h)
                                .min(output_bounds.loc.y + output_bounds.size.h);
                            let area =
                                i64::from((right - left).max(0)) * i64::from((bottom - top).max(0));
                            Some((area, output.name()))
                        })
                        .max_by_key(|(area, _)| *area)
                        .filter(|(area, _)| *area > 0)
                        .map(|(_, name)| name)
                });
                Some(ShellSurfaceSnapshot {
                    role,
                    geometry,
                    output,
                })
            })
            .take(nickel_session_protocol::MAX_WINDOWS)
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
            locked: self.locked,
            workspaces: self.protocol_workspaces(),
        }
    }

    fn protocol_workspaces(&self) -> WorkspaceState {
        let shell_ids = self
            .shell_windows()
            .filter_map(|window| {
                self.surface_windows
                    .get(&window.toplevel()?.wl_surface().id())
            })
            .copied()
            .collect::<HashSet<_>>();
        WorkspaceState {
            active: ProtocolWorkspaceId(self.workspaces.active().0),
            active_output: self.workspaces.active_output().map(str::to_owned),
            ordered: self
                .workspaces
                .ordered()
                .iter()
                .take(nickel_session_protocol::MAX_WORKSPACES)
                .map(|workspace| WorkspaceSnapshot {
                    id: ProtocolWorkspaceId(workspace.id.0),
                    windows: workspace
                        .windows
                        .iter()
                        .filter(|window| !shell_ids.contains(window))
                        .map(|window| ProtocolWindowId(window.0))
                        .collect(),
                    focused: workspace
                        .last_focused
                        .filter(|window| !shell_ids.contains(window))
                        .map(|window| ProtocolWindowId(window.0)),
                })
                .collect(),
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
        if visible {
            self.launcher_output_name = self.preferred_interaction_output_name();
        }
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

    pub(crate) fn stage_output_removal(&mut self, output: &Output) {
        let Some(removed) = self.space.output_geometry(output) else {
            return;
        };
        let Some(fallback) = self
            .space
            .outputs()
            .filter(|candidate| *candidate != output)
            .find_map(|candidate| self.space.output_geometry(candidate))
        else {
            return;
        };
        let removed_geometry = Geometry {
            x: removed.loc.x,
            y: removed.loc.y,
            width: removed.size.w,
            height: removed.size.h,
        };
        let fallback_geometry = shell_layout::work_area(Geometry {
            x: fallback.loc.x,
            y: fallback.loc.y,
            width: fallback.size.w,
            height: fallback.size.h,
        });
        let output_geometries = self
            .space
            .outputs()
            .filter_map(|output| self.space.output_geometry(output))
            .map(|geometry| Geometry {
                x: geometry.loc.x,
                y: geometry.loc.y,
                width: geometry.size.w,
                height: geometry.size.h,
            })
            .collect::<Vec<_>>();
        let mut displaced = Vec::new();
        let mapped = self
            .space
            .elements()
            .filter_map(|window| {
                let id = window
                    .wl_surface()
                    .and_then(|surface| self.surface_windows.get(&surface.id()))
                    .copied()?;
                self.workspaces.workspace_for(&id)?;
                let bounds = self.space.element_bbox(window)?;
                let geometry = Geometry {
                    x: bounds.loc.x,
                    y: bounds.loc.y,
                    width: bounds.size.w,
                    height: bounds.size.h,
                };
                (shell_layout::output_for_window(geometry, &output_geometries)
                    == Some(removed_geometry))
                .then(|| (id, window.clone(), bounds.loc, bounds.size))
            })
            .collect::<Vec<_>>();
        for (id, window, location, size) in mapped {
            let relative_location: Point<i32, Logical> =
                (location.x - removed.loc.x, location.y - removed.loc.y).into();
            let rescue_location = clamp_window_location(
                (
                    fallback_geometry.x + relative_location.x,
                    fallback_geometry.y + relative_location.y,
                )
                    .into(),
                size,
                fallback_geometry,
            );
            self.space.map_element(window, rescue_location, false);
            displaced.push(DisplacedWindow {
                id,
                relative_location,
                rescue_location,
            });
        }
        for (id, (window, location)) in &mut self.minimized_windows {
            let size = window.geometry().size;
            let geometry = Geometry {
                x: location.x,
                y: location.y,
                width: size.w,
                height: size.h,
            };
            if shell_layout::output_for_window(geometry, &output_geometries)
                != Some(removed_geometry)
            {
                continue;
            }
            let relative_location: Point<i32, Logical> =
                (location.x - removed.loc.x, location.y - removed.loc.y).into();
            let rescue_location = clamp_window_location(
                (
                    fallback_geometry.x + relative_location.x,
                    fallback_geometry.y + relative_location.y,
                )
                    .into(),
                size,
                fallback_geometry,
            );
            *location = rescue_location;
            displaced.push(DisplacedWindow {
                id: *id,
                relative_location,
                rescue_location,
            });
        }
        for (id, (window, location)) in &mut self.workspace_hidden_windows {
            let size = window.geometry().size;
            let geometry = Geometry {
                x: location.x,
                y: location.y,
                width: size.w,
                height: size.h,
            };
            if shell_layout::output_for_window(geometry, &output_geometries)
                != Some(removed_geometry)
            {
                continue;
            }
            let relative_location: Point<i32, Logical> =
                (location.x - removed.loc.x, location.y - removed.loc.y).into();
            let rescue_location = clamp_window_location(
                (
                    fallback_geometry.x + relative_location.x,
                    fallback_geometry.y + relative_location.y,
                )
                    .into(),
                size,
                fallback_geometry,
            );
            *location = rescue_location;
            displaced.push(DisplacedWindow {
                id: *id,
                relative_location,
                rescue_location,
            });
        }
        self.displaced_output_windows
            .insert(output.name(), displaced);
    }

    pub(crate) fn restore_output_windows(&mut self, output: &Output) {
        let Some(displaced) = self.displaced_output_windows.remove(&output.name()) else {
            return;
        };
        let Some(geometry) = self.space.output_geometry(output) else {
            return;
        };
        let work_area = shell_layout::work_area(Geometry {
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w,
            height: geometry.size.h,
        });
        for displaced in displaced {
            let desired = (
                geometry.loc.x + displaced.relative_location.x,
                geometry.loc.y + displaced.relative_location.y,
            )
                .into();
            if let Some((window, location)) = self.minimized_windows.get_mut(&displaced.id) {
                if *location == displaced.rescue_location {
                    *location = clamp_window_location(desired, window.geometry().size, work_area);
                }
                continue;
            }
            if let Some((window, location)) = self.workspace_hidden_windows.get_mut(&displaced.id) {
                if *location == displaced.rescue_location {
                    *location = clamp_window_location(desired, window.geometry().size, work_area);
                }
                continue;
            }
            let window = self.space.elements().find(|window| {
                window
                    .wl_surface()
                    .and_then(|surface| self.surface_windows.get(&surface.id()))
                    .copied()
                    == Some(displaced.id)
            });
            if let Some(window) = window.cloned()
                && self.space.element_location(&window) == Some(displaced.rescue_location)
            {
                let location = clamp_window_location(desired, window.geometry().size, work_area);
                self.space.map_element(window, location, false);
            }
        }
        self.relayout_maximized_windows();
        self.relayout_fullscreen_windows();
    }

    fn apply_workspace_transition(&mut self, transition: WorkspaceTransition<WindowId>) {
        self.hide_overlays();
        for id in transition.hide {
            if self.minimized_windows.contains_key(&id)
                || self.workspace_hidden_windows.contains_key(&id)
            {
                continue;
            }
            let window = self.space.elements().find(|window| {
                window
                    .wl_surface()
                    .and_then(|surface| self.surface_windows.get(&surface.id()))
                    .copied()
                    == Some(id)
            });
            if let Some(window) = window.cloned() {
                let location = self.space.element_location(&window).unwrap_or_default();
                window.set_activated(false);
                self.space.unmap_elem(&window);
                self.workspace_hidden_windows.insert(id, (window, location));
            }
        }
        for id in transition.show {
            if self.minimized_windows.contains_key(&id) {
                continue;
            }
            if let Some((window, location)) = self.workspace_hidden_windows.remove(&id) {
                self.space.map_element(window, location, true);
            }
        }
        if let Some(focus) = transition.focus {
            self.activate_window(focus);
        } else {
            self.windows.deactivate_all();
            self.seat
                .get_keyboard()
                .unwrap()
                .set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        self.raise_panels();
        self.request_output_redraw();
        self.notify_workspace_state();
        self.notify_protocol_snapshot();
    }

    pub fn switch_workspace_direction(
        &mut self,
        direction: nickel_core::workspaces::WorkspaceDirection,
    ) {
        let target = self.workspaces.neighbor(direction);
        let output = self.output_name_at_pointer();
        if let Ok(transition) = self.workspaces.switch_to(target, output) {
            self.apply_workspace_transition(transition);
        }
    }

    pub fn move_active_window_to_workspace(
        &mut self,
        direction: nickel_core::workspaces::WorkspaceDirection,
    ) {
        let Some(window) = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)
            .map(|window| window.id)
        else {
            return;
        };
        let target = self.workspaces.neighbor(direction);
        if let Ok(transition) = self.workspaces.move_window(&window, target) {
            self.apply_workspace_transition(transition);
        }
    }

    fn output_name_at_pointer(&self) -> Option<String> {
        let location = self.seat.get_pointer()?.current_location();
        self.space.outputs().find_map(|output| {
            self.space
                .output_geometry(output)
                .filter(|geometry| geometry.to_f64().contains(location))
                .map(|_| output.name())
        })
    }

    pub fn set_launcher_visible(&mut self, visible: bool) {
        let changed = self.launcher_visibility.is_visible() != visible;
        if changed && visible {
            self.launcher_show_requested_at = Some(std::time::Instant::now());
            self.launcher_output_name = self.preferred_interaction_output_name();
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

    fn notify_workspace_state(&mut self) {
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::Workspaces(self.protocol_workspaces())),
        }) else {
            return;
        };
        let Ok(socket) = UnixDatagram::unbound() else {
            return;
        };
        self.launcher_subscribers
            .retain(|path| socket.send_to(&event, path).is_ok());
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
                Some(crate::focus::KeyboardFocusTarget::Wayland(surface)),
                SERIAL_COUNTER.next_serial(),
            );
            self.space.elements().for_each(|window| {
                if let Some(toplevel) = window.toplevel() {
                    toplevel.send_pending_configure();
                }
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
        window.x11_surface().is_some_and(|surface| {
            self.x11_fullscreen_restore
                .contains_key(&surface.window_id())
        }) || window.toplevel().is_some_and(|surface| {
            self.fullscreen_restore
                .contains_key(&surface.wl_surface().id())
        })
    }

    pub fn is_maximized_window(&self, window: &Window) -> bool {
        window.x11_surface().is_some_and(|surface| {
            self.x11_maximized_restore
                .contains_key(&surface.window_id())
        }) || window.toplevel().is_some_and(|surface| {
            self.maximized_restore
                .contains_key(&surface.wl_surface().id())
        })
    }

    pub fn is_server_decorated(&self, window: &Window) -> bool {
        window
            .x11_surface()
            .is_some_and(|surface| self.x11_windows.contains_key(&surface.window_id()))
            || window
                .toplevel()
                .is_some_and(|surface| self.server_decorated.contains(&surface.wl_surface().id()))
    }

    pub fn shell_windows(&self) -> impl Iterator<Item = &Window> {
        self.launcher_window
            .iter()
            .chain(self.desktop_windows.iter())
            .chain(self.panel_windows.iter())
            .chain(self.lock_windows.iter())
            .chain(self.utility_windows.iter())
            .chain(self.context_menu_window.iter())
            .chain(self.preview_window.iter())
    }

    pub fn register_utility_window(&mut self, window: Window) {
        if !self.utility_windows.contains(&window) {
            self.utility_windows.push(window);
        }
    }

    pub fn register_lock(&mut self, window: Window) {
        window.override_z_index(100);
        self.lock_windows
            .retain(|candidate| self.space.elements().any(|mapped| mapped == candidate));
        if !self.lock_windows.contains(&window) {
            // SDL recreates native Wayland surfaces when a hidden window is
            // shown. The replacement may register before the old surface's
            // unmap reaches Smithay, so mapped-state alone cannot identify the
            // stale identity. There is exactly one lock surface per output;
            // once that capacity is full, a newly registered identity replaces
            // the oldest one and is the surface that receives configuration.
            let output_count = self.space.outputs().count().max(1);
            while self.lock_windows.len() >= output_count {
                let stale = self.lock_windows.remove(0);
                self.space.unmap_elem(&stale);
            }
            self.lock_windows.push(window.clone());
        }
        self.relayout_lock_surfaces();
        if self.locked {
            let focus = window
                .wl_surface()
                .map(|surface| crate::focus::KeyboardFocusTarget::Wayland(surface.into_owned()));
            self.seat
                .get_keyboard()
                .unwrap()
                .set_focus(self, focus, SERIAL_COUNTER.next_serial());
            let pointer = self.seat.get_pointer().unwrap();
            let location = pointer.current_location();
            pointer.motion(
                self,
                self.pointer_surface_under(location),
                &smithay::input::pointer::MotionEvent {
                    location,
                    serial: SERIAL_COUNTER.next_serial(),
                    time: smithay::backend::input::InputTime::now(),
                },
            );
            pointer.frame(self);
        }
    }

    fn relayout_lock_surfaces(&mut self) {
        let outputs = self.space.outputs().cloned().collect::<Vec<_>>();
        for (lock, output) in self.lock_windows.clone().into_iter().zip(outputs) {
            let Some(geometry) = self.space.output_geometry(&output) else {
                continue;
            };
            Self::configure_window(
                &lock,
                Geometry {
                    x: geometry.loc.x,
                    y: geometry.loc.y,
                    width: geometry.size.w,
                    height: geometry.size.h,
                },
            );
            if self.locked {
                self.space.map_element(lock.clone(), geometry.loc, true);
                self.space.raise_element(&lock, true);
            } else {
                self.space.unmap_elem(&lock);
            }
        }
    }

    fn lock_session(&mut self) {
        if self.locked {
            return;
        }
        self.locked = true;
        let shell_ids = self
            .shell_windows()
            .filter_map(|window| self.surface_windows.get(&window.wl_surface()?.id()))
            .copied()
            .collect::<HashSet<_>>();
        self.lock_restore_window = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active && !shell_ids.contains(&window.id))
            .map(|window| window.id);
        self.hide_overlays();
        self.relayout_lock_surfaces();
        let pointer = self.seat.get_pointer().unwrap();
        let pointer_location = pointer.current_location();
        pointer.motion(
            self,
            self.pointer_surface_under(pointer_location),
            &smithay::input::pointer::MotionEvent {
                location: pointer_location,
                serial: SERIAL_COUNTER.next_serial(),
                time: smithay::backend::input::InputTime::now(),
            },
        );
        pointer.frame(self);
        if !self.active_touch_slots.is_empty() {
            self.active_touch_slots.clear();
            self.seat.get_touch().unwrap().cancel(self);
        }
        let focus = self
            .lock_windows
            .first()
            .and_then(Window::wl_surface)
            .map(|surface| crate::focus::KeyboardFocusTarget::Wayland(surface.into_owned()));
        self.seat
            .get_keyboard()
            .unwrap()
            .set_focus(self, focus, SERIAL_COUNTER.next_serial());
        self.notify_lock_state();
    }

    fn unlock_session(&mut self) {
        if !self.locked {
            return;
        }
        self.locked = false;
        self.note_input_activity();
        self.relayout_lock_surfaces();
        if let Some(window) = self.lock_restore_window.take() {
            self.activate_window(window);
        } else {
            self.seat
                .get_keyboard()
                .unwrap()
                .set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        self.notify_lock_state();
    }

    fn notify_lock_state(&mut self) {
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::LockState {
                locked: self.locked,
            }),
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

    pub fn register_context_menu(&mut self, window: Window) {
        window.override_z_index(50);
        self.context_menu_window = Some(window);
    }

    pub fn register_preview(&mut self, window: Window) {
        window.override_z_index(49);
        self.preview_window = Some(window);
    }

    pub fn show_context_menu(
        &mut self,
        x: i32,
        requested_width: i32,
        requested_height: i32,
        focus: bool,
    ) {
        self.show_transient(
            self.context_menu_window.clone(),
            x,
            requested_width,
            requested_height,
            focus,
            "context menu",
        );
    }

    pub fn show_preview(&mut self, x: i32, requested_width: i32, requested_height: i32) {
        self.show_transient(
            self.preview_window.clone(),
            x,
            requested_width,
            requested_height,
            false,
            "preview",
        );
    }

    fn show_transient(
        &mut self,
        window: Option<Window>,
        x: i32,
        requested_width: i32,
        requested_height: i32,
        focus: bool,
        label: &str,
    ) {
        let Some(window) = window else {
            return;
        };
        let output = self
            .space
            .outputs()
            .filter_map(|output| self.space.output_geometry(output))
            .find(|geometry| x >= geometry.loc.x && x < geometry.loc.x + geometry.size.w)
            .map(|geometry| Geometry {
                x: geometry.loc.x,
                y: geometry.loc.y,
                width: geometry.size.w,
                height: geometry.size.h,
            })
            .or_else(|| self.output_geometry());
        let Some(output) = output else {
            return;
        };
        let width = requested_width.clamp(120, output.width.max(120));
        let maximum_height = (output.height - shell_layout::PANEL_HEIGHT).max(52);
        let height = requested_height.clamp(52, maximum_height);
        let x = x.clamp(output.x, output.x + output.width - width);
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
                crate::focus::KeyboardFocusTarget::for_window(&window),
                SERIAL_COUNTER.next_serial(),
            );
        }
        self.space.elements().for_each(|element| {
            if let Some(toplevel) = element.toplevel() {
                toplevel.send_pending_configure();
            }
        });
        self.space.raise_element(&window, true);
        self.raise_panels();
        eprintln!("nickel-session: {label} shown at {x},{y}");
    }

    pub fn hide_context_menu(&mut self) {
        if let Some(window) = self.context_menu_window.clone() {
            self.space.unmap_elem(&window);
        }
        self.preview_highlight = None;
        eprintln!("nickel-session: context menu hidden");
    }

    pub fn hide_overlays(&mut self) {
        if let Some(window) = self.context_menu_window.clone() {
            self.space.unmap_elem(&window);
        }
        if let Some(window) = self.preview_window.clone() {
            self.space.unmap_elem(&window);
        }
        self.preview_highlight = None;
        self.preview_requests.clear();
        self.preview_frames.clear();
        eprintln!("nickel-session: transient overlays hidden");
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
            } else if let Some(surface) = window.x11_surface() {
                let _ = surface.close();
            }
            self.hide_context_menu();
            return;
        }
        if let Some((window, _)) = self.workspace_hidden_windows.remove(&id) {
            if let Some(surface) = window.toplevel() {
                surface.send_close();
            } else if let Some(surface) = window.x11_surface() {
                let _ = surface.close();
            }
            self.hide_context_menu();
            return;
        }
        if let Some(window) = self.space.elements().find(|window| {
            window
                .wl_surface()
                .is_some_and(|surface| surface.id() == surface_id)
        }) {
            if let Some(surface) = window.toplevel() {
                surface.send_close();
            } else if let Some(surface) = window.x11_surface() {
                let _ = surface.close();
            }
        }
        self.hide_context_menu();
    }

    pub fn activate_window(&mut self, id: WindowId) {
        if let Some(workspace) = self.workspaces.workspace_for(&id)
            && workspace != self.workspaces.active()
            && let Ok(mut transition) = self.workspaces.switch_to(workspace, None)
        {
            transition.focus = Some(id);
            self.apply_workspace_transition(transition);
            return;
        }
        if let Some((window, location)) = self.minimized_windows.remove(&id) {
            if let Some(surface) = window.x11_surface() {
                let _ = surface.set_mapped(true);
            }
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
                    .wl_surface()
                    .is_some_and(|surface| surface.id() == surface_id)
            })
            .cloned()
        else {
            return;
        };
        self.space.raise_element(&window, true);
        if let Some(surface) = window.x11_surface() {
            self.raise_x11_surface(surface);
        }
        self.windows.raise(id);
        self.workspaces.focused(&id);
        self.space.elements().for_each(|candidate| {
            candidate.set_activated(candidate == &window);
        });
        self.seat.get_keyboard().unwrap().set_focus(
            self,
            crate::focus::KeyboardFocusTarget::for_window(&window),
            SERIAL_COUNTER.next_serial(),
        );
        self.space.elements().for_each(|window| {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_pending_configure();
            }
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
            .filter(|window| self.workspaces.is_visible(&window.id))
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
                    .wl_surface()
                    .and_then(|surface| self.surface_windows.get(&surface.id()))
                    .copied()
                    == Some(id)
            })
            .cloned()
        else {
            return;
        };
        let location = self.space.element_location(&window).unwrap_or_default();
        if let Some(surface) = window.x11_surface() {
            let _ = surface.set_mapped(false);
        }
        window.set_activated(false);
        self.space.unmap_elem(&window);
        self.minimized_windows.insert(id, (window, location));
        self.workspaces.unfocused(&id);
        let replacement = self
            .workspaces
            .ordered()
            .iter()
            .find(|workspace| workspace.id == self.workspaces.active())
            .and_then(|workspace| {
                workspace
                    .windows
                    .iter()
                    .rev()
                    .find(|candidate| {
                        **candidate != id && !self.minimized_windows.contains_key(candidate)
                    })
                    .copied()
            });
        if let Some(replacement) = replacement {
            self.activate_window(replacement);
        } else {
            self.windows.deactivate_all();
            self.seat
                .get_keyboard()
                .unwrap()
                .set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        self.raise_panels();
        self.notify_protocol_snapshot();
    }

    pub fn maximize_window(&mut self, id: WindowId) {
        self.activate_window(id);
        let x11_window = self
            .space
            .elements()
            .find(|window| {
                window
                    .wl_surface()
                    .and_then(|surface| self.surface_windows.get(&surface.id()))
                    .copied()
                    == Some(id)
            })
            .cloned();
        if let Some(window) = x11_window
            && let Some(surface) = window.x11_surface()
            && let Some(output) = self.space.outputs_for_element(&window).first()
            && let Some(geometry) = self.space.output_geometry(output)
        {
            let maximize = !surface.is_maximized();
            let _ = surface.set_maximized(maximize);
            if maximize {
                self.x11_maximized_restore
                    .insert(surface.window_id(), surface.geometry());
                let _ = surface.configure(geometry);
                self.space.map_element(window, geometry.loc, true);
            } else if let Some(restore) = self.x11_maximized_restore.remove(&surface.window_id()) {
                let _ = surface.configure(restore);
                self.space.map_element(window, restore.loc, true);
            }
            return;
        }
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

    pub fn toggle_fullscreen_window(&mut self, id: WindowId) {
        self.activate_window(id);
        let window = self.space.elements().find(|window| {
            window
                .wl_surface()
                .and_then(|surface| self.surface_windows.get(&surface.id()))
                .copied()
                == Some(id)
        });
        if let Some(surface) = window.and_then(Window::x11_surface).cloned() {
            if self
                .x11_fullscreen_restore
                .contains_key(&surface.window_id())
            {
                self.unfullscreen_x11(&surface);
            } else {
                self.fullscreen_x11(&surface);
            }
            return;
        }
        let surface = window.and_then(Window::toplevel).cloned();
        if let Some(surface) = surface {
            if self
                .fullscreen_restore
                .contains_key(&surface.wl_surface().id())
            {
                self.unfullscreen_toplevel(&surface);
            } else {
                self.fullscreen_toplevel(&surface);
            }
        }
    }

    pub fn relayout_shell_surfaces(&mut self) {
        if self.output_geometry().is_none() {
            return;
        }
        // SDL creates desktop and panel windows in the compositor-advertised
        // display order. Space preserves that output insertion order, so the
        // shell surfaces must use it rather than a separately sorted model.
        let outputs = self.space.outputs().cloned().collect::<Vec<_>>();
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
        self.relayout_lock_surfaces();
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

    pub fn fullscreen_x11(&mut self, surface: &smithay::xwayland::X11Surface) {
        let Some(window) = self
            .space
            .elements()
            .find(|window| window.x11_surface() == Some(surface))
            .cloned()
        else {
            return;
        };
        let Some(output) = self.space.outputs_for_element(&window).first().cloned() else {
            return;
        };
        let Some(geometry) = self.space.output_geometry(&output) else {
            return;
        };
        self.x11_fullscreen_restore
            .entry(surface.window_id())
            .or_insert_with(|| surface.geometry());
        let _ = surface.set_fullscreen(true);
        let _ = surface.configure(geometry);
        window.override_z_index(45);
        self.space.map_element(window, geometry.loc, true);
        self.request_output_redraw();
        self.notify_protocol_snapshot();
    }

    pub fn unfullscreen_x11(&mut self, surface: &smithay::xwayland::X11Surface) {
        let Some(restore) = self.x11_fullscreen_restore.remove(&surface.window_id()) else {
            return;
        };
        let _ = surface.set_fullscreen(false);
        let _ = surface.configure(restore);
        let window = {
            self.space
                .elements()
                .find(|window| window.x11_surface() == Some(surface))
                .cloned()
        };
        if let Some(window) = window {
            window.override_z_index(30);
            self.space.map_element(window, restore.loc, true);
        }
        self.raise_panels();
        self.request_output_redraw();
        self.notify_protocol_snapshot();
    }

    pub fn forget_x11_geometry(&mut self, surface: &smithay::xwayland::X11Surface) {
        self.x11_maximized_restore.remove(&surface.window_id());
        self.x11_fullscreen_restore.remove(&surface.window_id());
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

    pub(crate) fn relayout_maximized_windows(&mut self) {
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

    pub(crate) fn relayout_fullscreen_windows(&mut self) {
        let fullscreen = self
            .space
            .elements()
            .filter(|window| self.is_fullscreen_window(window))
            .cloned()
            .collect::<Vec<_>>();
        for window in fullscreen {
            let Some(output) = self.output_geometry_for_window(&window) else {
                continue;
            };
            if let Some(surface) = window.toplevel() {
                Self::configure_window(&window, output);
                window.override_z_index(45);
                self.space
                    .map_element(window.clone(), (output.x, output.y), true);
                surface.send_pending_configure();
            } else if let Some(surface) = window.x11_surface() {
                let geometry = smithay::utils::Rectangle::new(
                    (output.x, output.y).into(),
                    (output.width, output.height).into(),
                );
                let _ = surface.configure(geometry);
                window.override_z_index(45);
                self.space
                    .map_element(window.clone(), (output.x, output.y), true);
            }
        }
        self.raise_panels();
        self.request_output_redraw();
    }

    fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|window| window.wl_surface().as_deref() == Some(surface))
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

    fn output_geometry_named(&self, name: &str) -> Option<Geometry> {
        let output = self.space.outputs().find(|output| output.name() == name)?;
        let geometry = self.space.output_geometry(output)?;
        Some(Geometry {
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w,
            height: geometry.size.h,
        })
    }

    fn preferred_interaction_output_name(&self) -> Option<String> {
        self.output_name_at_pointer()
            .or_else(|| self.workspaces.active_output().map(str::to_owned))
            .or_else(|| self.primary_output_name.clone())
            .or_else(|| self.space.outputs().next().map(|output| output.name()))
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
        shell_layout::work_area(output)
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
        let output = self
            .launcher_output_name
            .as_deref()
            .and_then(|name| self.output_geometry_named(name))
            .or_else(|| self.output_geometry())
            .unwrap_or(Geometry {
                x: 0,
                y: 0,
                width,
                height: height + shell_layout::PANEL_HEIGHT + 8,
            });
        let work_area = shell_layout::work_area(output);
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
        event_loop: &mut EventLoop<'static, NickelSession>,
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
        // The Rust Wayland backend exposes an epoll descriptor. Polling that
        // epoll descriptor from calloop's epoll can lose readiness on some
        // nested-compositor stacks, so a tiny blocking poller converts it into
        // an ordinary calloop channel. Dispatch and all state mutation remain
        // on the compositor event-loop thread.
        let poll_fd = smithay::reexports::rustix::io::dup(display.as_fd())
            .expect("failed to duplicate Wayland backend poll fd");
        let (ready_tx, ready_rx) = channel::channel();
        let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel(0);
        std::thread::Builder::new()
            .name("nickel-wayland-poll".into())
            .spawn(move || {
                use smithay::reexports::rustix::event::{PollFd, PollFlags, poll};
                let mut descriptors = [PollFd::new(&poll_fd, PollFlags::IN)];
                loop {
                    if poll(&mut descriptors, None).is_err() || ready_tx.send(()).is_err() {
                        break;
                    }
                    if ack_rx.recv().is_err() {
                        break;
                    }
                    descriptors[0].clear_revents();
                }
            })
            .expect("failed to start Wayland backend poller");
        let mut display = display;
        loop_handle
            .insert_source(ready_rx, move |event, _, state| {
                if let channel::Event::Msg(()) = event {
                    if let Err(error) = display.dispatch_clients(state) {
                        tracing::warn!(%error, "Wayland client dispatch failed");
                    }
                    if let Err(error) = display.flush_clients() {
                        tracing::debug!(%error, "Wayland client flush deferred");
                    }
                    let _ = ack_tx.send(());
                }
            })
            .expect("failed to register Wayland backend dispatch channel");

        socket_name
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.space
            .element_under(pos)
            .filter(|(window, _)| !self.locked || self.lock_windows.contains(window))
            .and_then(|(window, location)| {
                window
                    .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + location).to_f64()))
            })
    }

    pub fn pointer_surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(crate::focus::PointerFocusTarget, Point<f64, Logical>)> {
        self.space
            .element_under(pos)
            .filter(|(window, _)| !self.locked || self.lock_windows.contains(window))
            .and_then(|(window, location)| {
                window
                    .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(surface, origin)| {
                        let target = window.x11_surface().map_or_else(
                            || crate::focus::PointerFocusTarget::Wayland(surface),
                            |x11| crate::focus::PointerFocusTarget::X11(x11.clone()),
                        );
                        (target, (origin + location).to_f64())
                    })
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
    use super::{
        clamp_window_location, command_requires_shell_identity, shell_registration_allowed,
        test_control_may_invoke,
    };
    use crate::shell_layout::Geometry;
    use nickel_session_protocol::{Command, SessionAction};

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

    #[test]
    fn destructive_and_unlock_commands_require_the_registered_shell_pid() {
        for command in [
            Command::LogOut,
            Command::Unlock,
            Command::SessionAction {
                action: SessionAction::Lock,
            },
            Command::SessionAction {
                action: SessionAction::PowerOff,
            },
        ] {
            assert!(command_requires_shell_identity(&command));
        }
        assert!(!command_requires_shell_identity(&Command::ToggleLauncher));
    }

    #[test]
    fn explicit_nested_test_control_can_only_cross_lock_boundaries() {
        assert!(test_control_may_invoke(&Command::Unlock));
        assert!(test_control_may_invoke(&Command::SessionAction {
            action: SessionAction::Lock,
        }));
        assert!(!test_control_may_invoke(&Command::SessionAction {
            action: SessionAction::PowerOff,
        }));
    }

    #[test]
    fn output_rescue_clamps_windows_to_the_authoritative_work_area() {
        let work_area = Geometry {
            x: 100,
            y: 40,
            width: 800,
            height: 500,
        };
        assert_eq!(
            clamp_window_location((850, 500).into(), (300, 200).into(), work_area),
            (600, 340).into()
        );
        assert_eq!(
            clamp_window_location((-20, -30).into(), (300, 200).into(), work_area),
            (100, 40).into()
        );
    }
}
