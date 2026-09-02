use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::OsString,
    hash::Hash,
    os::fd::AsFd,
    os::fd::AsRawFd,
    os::unix::net::UnixDatagram,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

use nickel_core::{
    focus::FocusTransactions,
    hotkeys::{CompositorShortcutAdapter, HotkeyAction},
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
        calloop::{
            EventLoop, Interest, LoopSignal, Mode, PostAction, channel,
            generic::Generic,
            timer::{TimeoutAction, Timer},
        },
        wayland_server::{
            Display, DisplayHandle, Resource,
            backend::{ClientData, ClientId, DisconnectReason, GlobalId, ObjectId},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{IsAlive, Logical, Point, Rectangle, SERIAL_COUNTER, Size, Transform},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        idle_inhibit::IdleInhibitManagerState,
        image_capture_source::{ImageCaptureSourceState, OutputCaptureSourceState},
        image_copy_capture::{ImageCopyCaptureState, Session},
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

const OUTPUT_GLOBAL_BIND_SETTLE_GRACE: Duration = Duration::from_secs(3);
const OUTPUT_GLOBAL_DISABLED_GRACE: Duration = Duration::from_secs(3);
const MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS: usize = nickel_session_protocol::MAX_OUTPUTS;

fn output_global_capacity_available(pending: usize, live: usize) -> bool {
    pending.saturating_add(live) < MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS
}

struct DeferredGlobalRetirements<T> {
    pending: VecDeque<DeferredGlobalRetirement<T>>,
}

struct DeferredGlobalRetirement<T> {
    identity: String,
    deadline: Instant,
    disabled: bool,
    value: T,
}

#[derive(Debug, Eq, PartialEq)]
enum GlobalRetirementAction<T> {
    Disable { identity: String, value: T },
    Remove { identity: String, value: T },
}

impl<T> Default for DeferredGlobalRetirements<T> {
    fn default() -> Self {
        Self {
            pending: VecDeque::new(),
        }
    }
}

impl<T> DeferredGlobalRetirements<T> {
    fn has_capacity(&self) -> bool {
        self.pending.len() < MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS
    }

    fn defer(&mut self, now: Instant, identity: String, value: T) -> Result<(), T> {
        if !self.has_capacity() {
            return Err(value);
        }
        self.pending.push_back(DeferredGlobalRetirement {
            identity,
            deadline: now + OUTPUT_GLOBAL_BIND_SETTLE_GRACE,
            disabled: false,
            value,
        });
        Ok(())
    }

    fn advance(&mut self, now: Instant) -> Vec<GlobalRetirementAction<T>>
    where
        T: Clone,
    {
        let mut actions = Vec::new();
        let mut retained = VecDeque::with_capacity(self.pending.len());
        while let Some(mut pending) = self.pending.pop_front() {
            if pending.deadline > now {
                retained.push_back(pending);
            } else if pending.disabled {
                actions.push(GlobalRetirementAction::Remove {
                    identity: pending.identity,
                    value: pending.value,
                });
            } else {
                pending.disabled = true;
                pending.deadline = now + OUTPUT_GLOBAL_DISABLED_GRACE;
                actions.push(GlobalRetirementAction::Disable {
                    identity: pending.identity.clone(),
                    value: pending.value.clone(),
                });
                retained.push_back(pending);
            }
        }
        self.pending = retained;
        actions
    }

    fn len(&self) -> usize {
        self.pending.len()
    }

    fn has_enabled_identity(&self, identity: &str) -> bool {
        self.pending
            .iter()
            .any(|pending| !pending.disabled && pending.identity == identity)
    }
}

fn protocol_error(code: ErrorCode, message: impl Into<String>) -> ServerMessage {
    ServerMessage::Error {
        code,
        message: message.into(),
    }
}

fn retain_live_idle_inhibitors<K: Eq + std::hash::Hash>(
    inhibitors: &mut HashMap<K, usize>,
    mut is_alive: impl FnMut(&K) -> bool,
) {
    inhibitors.retain(|surface, _| is_alive(surface));
}

fn identification_expiry_is_current(current_generation: u64, scheduled_generation: u64) -> bool {
    current_generation == scheduled_generation
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

pub(crate) fn drag_icon_location(
    pointer: Point<f64, Logical>,
    output: Rectangle<i32, Logical>,
) -> Option<Point<i32, Logical>> {
    output
        .to_f64()
        .contains(pointer)
        .then(|| (pointer - output.loc.to_f64()).to_i32_round())
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellRegistrationRejection {
    ClaimedPeerMismatch,
    NoActiveGeneration,
    OutsideActiveGeneration,
    OutsideSessionUser,
}

fn shell_registration_rejection(
    expected_pid: u32,
    claimed_pid: u32,
    peer_pid: u32,
    same_user: bool,
) -> Option<ShellRegistrationRejection> {
    if claimed_pid != peer_pid {
        Some(ShellRegistrationRejection::ClaimedPeerMismatch)
    } else if expected_pid == 0 {
        Some(ShellRegistrationRejection::NoActiveGeneration)
    } else if expected_pid != claimed_pid {
        Some(ShellRegistrationRejection::OutsideActiveGeneration)
    } else if !same_user {
        Some(ShellRegistrationRejection::OutsideSessionUser)
    } else {
        None
    }
}

fn command_requires_shell_identity(command: &SessionCommand) -> bool {
    matches!(
        command,
        SessionCommand::LogOut
            | SessionCommand::Unlock
            | SessionCommand::SessionAction { .. }
            | SessionCommand::FocusShellRole { .. }
            | SessionCommand::RestoreApplicationFocus
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

const SHELL_OUTPUT_METADATA_MARKER: &str = "[output=";

fn shell_surface_output_from_title(title: &str) -> Option<String> {
    let output = title
        .strip_suffix(']')?
        .rsplit_once(SHELL_OUTPUT_METADATA_MARKER)?
        .1;
    (!output.is_empty()
        && output
            .chars()
            .all(|character| !character.is_control() && character != ']'))
    .then_some(output.to_owned())
}

fn output_index_for_shell_surface(output_name: &str, output_names: &[String]) -> Option<usize> {
    if let Some(index) = output_names.iter().position(|name| name == output_name) {
        return Some(index);
    }

    let mut matches = output_names
        .iter()
        .enumerate()
        .filter(|(_, name)| {
            output_name
                .strip_suffix(name.as_str())
                .is_some_and(|prefix| prefix.ends_with(" - "))
        })
        .map(|(index, _)| index);
    let index = matches.next()?;
    matches.next().is_none().then_some(index)
}

fn shell_role_accepts_ordinary_focus(role: ShellRole) -> bool {
    matches!(
        role,
        ShellRole::ControlCenter
            | ShellRole::ProjectMenu
            | ShellRole::Preview
            | ShellRole::ContextMenu
            | ShellRole::Screenshot
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
    pub(crate) active_pointer_constraint_origins: HashMap<ObjectId, Point<f64, Logical>>,
    pub idle_inhibit_state: IdleInhibitManagerState,
    pub input_method_state: InputMethodManagerState,
    pub xwayland_shell_state: XWaylandShellState,
    pub image_capture_source_state: ImageCaptureSourceState,
    pub output_capture_source_state: OutputCaptureSourceState,
    pub image_copy_capture_state: ImageCopyCaptureState,
    pub image_copy_sessions: Vec<Session>,
    pub(crate) pending_image_copy_frames: Vec<crate::handlers::PendingImageCopyFrame>,
    pub xwm: Option<(XwmId, X11Wm)>,
    pub xwayland_restart_pending: bool,
    pub xwayland_display: Option<u32>,
    pub xwayland_registration: Option<smithay::reexports::calloop::RegistrationToken>,
    pub popups: PopupManager,

    pub seat: Seat<Self>,
    pub windows: WindowRegistry,
    pub surface_windows: HashMap<ObjectId, WindowId>,
    pub(crate) shell_owned_windows: HashSet<WindowId>,
    pub x11_windows: HashMap<u32, WindowId>,
    pub launcher_window: Option<Window>,
    pub launcher_visibility: LauncherVisibility,
    launcher_output_name: Option<String>,
    launcher_focus: FocusTransactions<ObjectId>,
    launcher_restore_window: Option<WindowId>,
    launcher_subscribers: Vec<PathBuf>,
    protocol_token: String,
    authenticated_shell_pids: HashSet<u32>,
    registered_shell_role_slots: Vec<RegisteredShellRole>,
    last_logged_shell_readiness: Option<nickel_session_protocol::ShellReadinessSnapshot>,
    test_control_enabled: bool,
    #[cfg(target_os = "linux")]
    pub(crate) test_controller: Option<crate::test_input::TestController>,
    expected_shell_pid: Arc<AtomicU32>,
    pub launcher_show_requested_at: Option<std::time::Instant>,
    pub desktop_windows: Vec<Window>,
    pub panel_windows: Vec<Window>,
    pub lock_windows: Vec<Window>,
    pub locked: bool,
    lock_restore_window: Option<WindowId>,
    shell_focus_restore_window: Option<WindowId>,
    pub(crate) pending_shell_focus_role: Option<ShellRole>,
    pub utility_windows: Vec<Window>,
    screenshot_output_name: Option<String>,
    pub context_menu_window: Option<Window>,
    pub preview_window: Option<Window>,
    pub server_decorated: HashSet<ObjectId>,
    pub primary_output_name: Option<String>,
    virtual_test_outputs: HashMap<String, (Output, Option<GlobalId>)>,
    pending_output_global_retirements: DeferredGlobalRetirements<GlobalId>,
    pub preview_frames: HashMap<WindowId, PreviewFrame>,
    preview_spares: HashMap<WindowId, Vec<u8>>,
    preview_switcher_interest: Vec<WindowId>,
    preview_overlay_interest: Vec<WindowId>,
    preview_admitted: HashSet<WindowId>,
    preview_dirty: HashSet<WindowId>,
    preview_content_generation: HashMap<WindowId, u64>,
    preview_attempted: HashMap<WindowId, (u64, u64)>,
    preview_render_wave: u64,
    preview_retry_pending: HashSet<WindowId>,
    preview_retry_scheduled: Option<u64>,
    preview_retry_epoch: u64,
    preview_counters: PreviewCacheCounters,
    pub hotkeys: CompositorShortcutAdapter,
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
    pub idle_inhibitors: HashMap<WlSurface, usize>,
    pub(crate) active_touch_slots: HashSet<smithay::backend::input::TouchSlot>,
    idle_controller: IdleController,
    pub dimmed: bool,
    pub frame_cursor: crate::window_frame::FrameCursor,
    pub buffer_commit_tx: Option<smithay::reexports::calloop::channel::Sender<SurfaceBufferCommit>>,
    pub identify_outputs_until: Option<std::time::Instant>,
    identify_outputs_generation: u64,
    pub output_capture_path: Option<PathBuf>,
    pub output_capture_name: Option<String>,
    pub output_capture_reply_path: Option<PathBuf>,
    pub output_capture_request_id: Option<u64>,
    pub shell_failure_count: u8,
    pub(crate) recovery_ui: crate::recovery_ui::RecoveryUi,
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

pub const PREVIEW_WIDTH: usize = 240;
pub const PREVIEW_HEIGHT: usize = 135;
pub const PREVIEW_FRAME_BYTES: usize = PREVIEW_WIDTH * PREVIEW_HEIGHT * 4;
pub const PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER: usize = 7;
pub const PREVIEW_ENTRY_CAPACITY: usize = PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER * 2;
pub const PREVIEW_BYTE_CAPACITY: usize = PREVIEW_ENTRY_CAPACITY * PREVIEW_FRAME_BYTES;

fn bounded_preview_ids(ids: Vec<WindowId>, selected: usize) -> Vec<WindowId> {
    let start = selected
        .saturating_sub(PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER / 2)
        .min(
            ids.len()
                .saturating_sub(PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER),
        );
    ids.into_iter()
        .skip(start)
        .take(PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER)
        .collect()
}

fn admitted_preview_ids(switcher: &[WindowId], overlay: &[WindowId]) -> HashSet<WindowId> {
    switcher
        .iter()
        .chain(overlay)
        .copied()
        .fold(Vec::new(), |mut ids, id| {
            if ids.len() < PREVIEW_ENTRY_CAPACITY && !ids.contains(&id) {
                ids.push(id);
            }
            ids
        })
        .into_iter()
        .collect()
}

fn retired_preview_ids(current: &HashSet<WindowId>, admitted: &HashSet<WindowId>) -> Vec<WindowId> {
    current.difference(admitted).copied().collect()
}

fn advance_preview_content_generation(
    generations: &mut HashMap<WindowId, u64>,
    attempted: &mut HashMap<WindowId, (u64, u64)>,
    id: WindowId,
) -> u64 {
    let generation = generations
        .entry(id)
        .and_modify(|generation| *generation = generation.wrapping_add(1).max(1))
        .or_insert(1);
    attempted.remove(&id);
    *generation
}

fn protocol_preview_from_cached(
    window: nickel_session_protocol::WindowId,
    frame: Option<&PreviewFrame>,
) -> Option<ProtocolPreview> {
    let frame = frame?;
    Some(ProtocolPreview {
        window,
        width: frame.width,
        height: frame.height,
        rgba: frame.rgba.clone(),
    })
}

pub(crate) fn reuse_preview_pixels(mut rgba: Vec<u8>, mapped: &[u8]) -> Vec<u8> {
    rgba.clear();
    rgba.extend_from_slice(mapped);
    rgba
}

pub(crate) fn preview_mapping_has_exact_size(mapped: &[u8]) -> bool {
    mapped.len() == PREVIEW_FRAME_BYTES
}

fn record_preview_capture_attempt(
    attempted: &mut HashMap<WindowId, (u64, u64)>,
    id: WindowId,
    content_generation: u64,
    render_wave: u64,
) -> bool {
    if attempted
        .get(&id)
        .is_some_and(|(generation, _)| *generation == content_generation)
    {
        return false;
    }
    attempted.insert(id, (content_generation, render_wave));
    true
}

#[derive(Clone, Copy, Debug, Default)]
struct PreviewCacheCounters {
    peak_bytes: u64,
    admissions: u64,
    evictions: u64,
    invalidations: u64,
    captures: u64,
    skipped_unchanged: u64,
    readback_bytes: u64,
    protocol_copy_bytes: u64,
    protocol_raw_copy_bytes: u64,
    protocol_base64_bytes: u64,
    protocol_json_payload_bytes: u64,
    protocol_framed_copy_bytes: u64,
    capture_failures: u64,
    cache_generation: u64,
}

impl NickelSession {
    fn update_preview_peak_bytes(&mut self) {
        self.preview_counters.peak_bytes = self
            .preview_counters
            .peak_bytes
            .max(self.preview_bytes() as u64);
    }

    pub(crate) fn preview_bytes(&self) -> usize {
        self.preview_frames
            .values()
            .map(|frame| frame.rgba.len())
            .sum::<usize>()
            + self
                .preview_spares
                .values()
                .map(Vec::capacity)
                .sum::<usize>()
    }

    #[cfg(feature = "backend-udev")]
    pub(crate) fn preview_generation(&self) -> u64 {
        self.preview_counters.cache_generation
    }

    fn drop_preview_frame(&mut self, id: &WindowId) {
        self.preview_dirty.remove(id);
        self.preview_content_generation.remove(id);
        self.preview_attempted.remove(id);
        self.preview_retry_pending.remove(id);
        let released =
            self.preview_spares.remove(id).is_some() || self.preview_frames.remove(id).is_some();
        if released {
            self.preview_counters.evictions += 1;
            self.preview_counters.cache_generation = self
                .preview_counters
                .cache_generation
                .wrapping_add(1)
                .max(1);
        }
    }

    fn reconcile_preview_admission(&mut self) {
        let admitted = admitted_preview_ids(
            &self.preview_switcher_interest,
            &self.preview_overlay_interest,
        );
        if admitted != self.preview_admitted {
            self.preview_retry_epoch = self.preview_retry_epoch.wrapping_add(1).max(1);
            self.preview_retry_scheduled = None;
            self.preview_retry_pending
                .retain(|id| admitted.contains(id));
        }
        let retired = retired_preview_ids(&self.preview_admitted, &admitted);
        for id in retired {
            self.drop_preview_frame(&id);
        }
        for id in admitted.difference(&self.preview_admitted) {
            self.preview_counters.admissions += 1;
            self.preview_dirty.insert(*id);
            advance_preview_content_generation(
                &mut self.preview_content_generation,
                &mut self.preview_attempted,
                *id,
            );
        }
        self.preview_admitted = admitted;
        self.schedule_preview_retry();
    }

    fn set_switcher_preview_interest(&mut self, ids: Vec<WindowId>) {
        self.preview_switcher_interest = ids;
        self.reconcile_preview_admission();
    }

    fn set_overlay_preview_interest(&mut self, ids: Vec<WindowId>) {
        self.preview_overlay_interest = ids;
        self.reconcile_preview_admission();
    }

    fn clear_switcher_preview_interest(&mut self) {
        self.preview_switcher_interest.clear();
        self.reconcile_preview_admission();
    }

    fn clear_overlay_preview_interest(&mut self) {
        self.preview_overlay_interest.clear();
        self.reconcile_preview_admission();
    }

    pub(crate) fn retire_shell_preview_memory(&mut self) {
        self.clear_all_previews();
    }

    pub(crate) fn reassociate_preview_surface(&mut self, id: WindowId) {
        if self.preview_admitted.contains(&id) {
            self.preview_frames.remove(&id);
            self.preview_counters.invalidations += 1;
            self.preview_counters.cache_generation = self
                .preview_counters
                .cache_generation
                .wrapping_add(1)
                .max(1);
            self.preview_dirty.insert(id);
            advance_preview_content_generation(
                &mut self.preview_content_generation,
                &mut self.preview_attempted,
                id,
            );
        }
    }

    pub(crate) fn invalidate_preview_for_surface(&mut self, surface: &WlSurface) {
        let mut root = surface.clone();
        while let Some(parent) = smithay::wayland::compositor::get_parent(&root) {
            root = parent;
        }
        if let Some(id) = self.surface_windows.get(&root.id()).copied()
            && self.preview_admitted.contains(&id)
        {
            self.preview_dirty.insert(id);
            advance_preview_content_generation(
                &mut self.preview_content_generation,
                &mut self.preview_attempted,
                id,
            );
            self.preview_counters.invalidations += 1;
            self.preview_counters.cache_generation = self
                .preview_counters
                .cache_generation
                .wrapping_add(1)
                .max(1);
        }
    }

    pub(crate) fn begin_preview_render_wave(&mut self) -> u64 {
        self.preview_render_wave = self.preview_render_wave.wrapping_add(1).max(1);
        self.preview_render_wave
    }

    pub(crate) fn preview_capture_candidates(&mut self, wave: u64) -> Vec<(WindowId, Window)> {
        let admitted = self.preview_admitted.clone();
        let dirty = self.preview_dirty.clone();
        let mut candidates = Vec::new();
        let windows = self.space.elements().cloned().collect::<Vec<_>>();
        for window in windows {
            let Some(id) = window
                .wl_surface()
                .and_then(|surface| self.surface_windows.get(&surface.id()))
                .copied()
            else {
                continue;
            };
            if !admitted.contains(&id) {
                continue;
            }
            if dirty.contains(&id) || !self.preview_frames.contains_key(&id) {
                let generation = self
                    .preview_content_generation
                    .get(&id)
                    .copied()
                    .unwrap_or(1);
                if !record_preview_capture_attempt(
                    &mut self.preview_attempted,
                    id,
                    generation,
                    wave,
                ) {
                    continue;
                }
                candidates.push((id, window));
            } else {
                self.preview_counters.skipped_unchanged += 1;
            }
        }
        candidates
    }

    pub(crate) fn take_preview_capture_buffer(&mut self, id: WindowId) -> (Vec<u8>, bool) {
        self.preview_frames.remove(&id).map_or_else(
            || {
                (
                    self.preview_spares
                        .remove(&id)
                        .unwrap_or_else(|| vec![0; PREVIEW_FRAME_BYTES]),
                    false,
                )
            },
            |frame| (frame.rgba, true),
        )
    }

    pub(crate) fn preview_capture_failed(&mut self, id: WindowId, rgba: Vec<u8>, had_frame: bool) {
        self.preview_counters.capture_failures += 1;
        if had_frame {
            self.preview_frames.insert(
                id,
                PreviewFrame {
                    width: PREVIEW_WIDTH as u16,
                    height: PREVIEW_HEIGHT as u16,
                    rgba,
                },
            );
        } else {
            self.preview_spares.insert(id, rgba);
        }
        self.preview_retry_pending.insert(id);
        self.update_preview_peak_bytes();
    }

    pub(crate) fn preview_renderer_failed(&mut self, id: WindowId) {
        self.preview_counters.capture_failures += 1;
        self.preview_retry_pending.insert(id);
    }

    pub(crate) fn advance_preview_retry_generation(&mut self) -> bool {
        let pending = std::mem::take(&mut self.preview_retry_pending);
        self.preview_retry_scheduled = None;
        for id in pending.iter().copied() {
            advance_preview_content_generation(
                &mut self.preview_content_generation,
                &mut self.preview_attempted,
                id,
            );
        }
        !pending.is_empty()
    }

    pub(crate) fn schedule_preview_retry(&mut self) {
        self.schedule_preview_retry_after(std::time::Duration::from_millis(16));
    }

    pub(crate) fn schedule_preview_retry_after(&mut self, delay: std::time::Duration) {
        if self.preview_retry_pending.is_empty() || self.preview_retry_scheduled.is_some() {
            return;
        }
        let epoch = self.preview_retry_epoch;
        let timer = Timer::from_duration(delay);
        match self
            .event_loop_handle
            .insert_source(timer, move |_, _, data| {
                if data.preview_retry_epoch == epoch
                    && data.preview_retry_scheduled == Some(epoch)
                    && data.advance_preview_retry_generation()
                {
                    data.request_output_redraw();
                    #[cfg(feature = "backend-udev")]
                    if data.native.is_some() {
                        data.render_all_outputs_once();
                    }
                }
                TimeoutAction::Drop
            }) {
            Ok(_) => self.preview_retry_scheduled = Some(epoch),
            Err(error) => tracing::warn!(?error, "failed to schedule preview capture retry"),
        }
    }

    fn record_preview_protocol_encoding(&mut self, json_payload_bytes: usize, framed_bytes: usize) {
        self.preview_counters.protocol_json_payload_bytes += json_payload_bytes as u64;
        self.preview_counters.protocol_framed_copy_bytes += framed_bytes as u64;
        self.preview_counters.protocol_copy_bytes += (json_payload_bytes + framed_bytes) as u64;
    }

    pub(crate) fn store_preview(&mut self, id: WindowId, frame: PreviewFrame) {
        // Capture leases are created only for admitted IDs after the byte/entry ceiling has
        // already been reconciled. No event dispatch can change admission while the synchronous
        // renderer call owns the lease, so commit is intentionally infallible.
        assert!(self.preview_admitted.contains(&id));
        assert_eq!(frame.rgba.len(), PREVIEW_FRAME_BYTES);
        self.preview_counters.captures += 1;
        self.preview_counters.cache_generation = self
            .preview_counters
            .cache_generation
            .wrapping_add(1)
            .max(1);
        self.preview_counters.readback_bytes += frame.rgba.len() as u64;
        self.preview_frames.insert(id, frame);
        self.preview_spares.remove(&id);
        self.preview_dirty.remove(&id);
        self.preview_attempted.remove(&id);
        self.update_preview_peak_bytes();
    }

    fn clear_all_previews(&mut self) {
        self.preview_counters.evictions +=
            (self.preview_frames.len() + self.preview_spares.len()) as u64;
        self.preview_switcher_interest.clear();
        self.preview_overlay_interest.clear();
        self.preview_admitted.clear();
        self.preview_dirty.clear();
        self.preview_content_generation.clear();
        self.preview_attempted.clear();
        self.preview_frames.clear();
        self.preview_spares.clear();
        self.preview_retry_pending.clear();
        self.preview_retry_epoch = self.preview_retry_epoch.wrapping_add(1).max(1);
        self.preview_retry_scheduled = None;
        self.preview_frames.shrink_to_fit();
        self.preview_spares.shrink_to_fit();
        self.preview_switcher_interest.shrink_to_fit();
        self.preview_overlay_interest.shrink_to_fit();
        self.preview_admitted.shrink_to_fit();
        self.preview_dirty.shrink_to_fit();
        self.preview_content_generation.shrink_to_fit();
        self.preview_attempted.shrink_to_fit();
        self.preview_retry_pending.shrink_to_fit();
        #[cfg(feature = "backend-udev")]
        if let Some(native) = self.native.as_mut() {
            native.clear_task_switcher_cache();
        }
    }
}

#[derive(Clone, Copy)]
struct DisplacedWindow {
    id: WindowId,
    relative_location: Point<i32, Logical>,
    rescue_location: Point<i32, Logical>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RegisteredShellRole {
    role: ShellRole,
    output: Option<String>,
    surface: ObjectId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LifecycleCollectionCounts {
    pub pointer_hints: usize,
    pub pointer_locks: usize,
    pub pointer_origins: usize,
    pub displaced_outputs: usize,
    pub displaced_windows: usize,
    pub shell_roles: usize,
}

fn retire_pointer_surface<K: Eq + Hash>(
    hints: &mut HashMap<K, Point<f64, Logical>>,
    locks: &mut HashSet<K>,
    origins: &mut HashMap<K, Point<f64, Logical>>,
    surface: &K,
) -> Option<Point<f64, Logical>> {
    let hint = hints.remove(surface);
    let was_locked = locks.remove(surface);
    let origin = origins.remove(surface);
    if hints.is_empty() {
        hints.shrink_to_fit();
    }
    if locks.is_empty() {
        locks.shrink_to_fit();
    }
    if origins.is_empty() {
        origins.shrink_to_fit();
    }
    was_locked.then(|| origin.zip(hint).map(|(origin, hint)| origin + hint))?
}

fn retire_displaced_window(
    outputs: &mut HashMap<String, Vec<DisplacedWindow>>,
    window_id: WindowId,
) {
    for displaced in outputs.values_mut() {
        displaced.retain(|window| window.id != window_id);
    }
    outputs.retain(|_, displaced| !displaced.is_empty());
    if outputs.is_empty() {
        outputs.shrink_to_fit();
    }
}

fn retire_shell_surface(registrations: &mut Vec<RegisteredShellRole>, surface_id: &ObjectId) {
    registrations.retain(|registration| registration.surface != *surface_id);
    if registrations.is_empty() {
        registrations.shrink_to_fit();
    }
}

fn shell_registration_role_changed(
    registrations: &[RegisteredShellRole],
    surface_id: &ObjectId,
    next_role: Option<ShellRole>,
) -> bool {
    registrations
        .iter()
        .find(|registration| registration.surface == *surface_id)
        .is_some_and(|registration| Some(registration.role) != next_role)
}

pub struct SurfaceBufferCommit {
    pub surface: WlSurface,
    pub render_visible: bool,
}

impl NickelSession {
    fn expire_output_identification(&mut self, scheduled_generation: u64) -> bool {
        if !identification_expiry_is_current(self.identify_outputs_generation, scheduled_generation)
        {
            return false;
        }
        self.identify_outputs_until = None;
        #[cfg(feature = "backend-udev")]
        if let Some(native) = self.native.as_mut() {
            native.retire_identify_badges();
        }
        true
    }

    fn begin_output_identification(&mut self) {
        const IDENTIFY_DURATION: std::time::Duration = std::time::Duration::from_secs(3);
        self.identify_outputs_generation = self.identify_outputs_generation.wrapping_add(1);
        let generation = self.identify_outputs_generation;
        self.identify_outputs_until = Some(std::time::Instant::now() + IDENTIFY_DURATION);
        self.request_output_redraw();
        #[cfg(feature = "backend-udev")]
        if self.native.is_some() {
            self.render_all_outputs();
        }

        let timer = Timer::from_duration(IDENTIFY_DURATION);
        if let Err(error) = self
            .event_loop_handle
            .insert_source(timer, move |_, _, data| {
                if data.expire_output_identification(generation) {
                    data.request_output_redraw();
                    #[cfg(feature = "backend-udev")]
                    if data.native.is_some() {
                        data.render_all_outputs();
                    }
                }
                TimeoutAction::Drop
            })
        {
            tracing::warn!(
                ?error,
                "failed to schedule output-identification retirement"
            );
        }
    }

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
        self.reap_output_global_retirements(Instant::now());
        self.prune_dead_idle_inhibitors();
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

    pub(crate) fn output_global_admission_available(&mut self) -> bool {
        self.reap_output_global_retirements(Instant::now());
        output_global_capacity_available(
            self.pending_output_global_retirements.len(),
            self.space.outputs().count(),
        )
    }

    pub(crate) fn defer_output_global_retirement(&mut self, identity: String, global: GlobalId) {
        self.defer_output_global_retirement_at(identity, global, Instant::now());
    }

    fn defer_output_global_retirement_at(
        &mut self,
        identity: String,
        global: GlobalId,
        now: Instant,
    ) {
        self.pending_output_global_retirements
            .defer(now, identity, global)
            .expect("output-global admission keeps the retirement queue bounded");
    }

    pub(crate) fn output_global_identity_available(&self, identity: &str) -> bool {
        !self
            .pending_output_global_retirements
            .has_enabled_identity(identity)
    }

    fn reap_output_global_retirements(&mut self, now: Instant) {
        let mut disabled = false;
        let mut identities_to_publish = HashSet::new();
        for action in self.pending_output_global_retirements.advance(now) {
            match action {
                GlobalRetirementAction::Disable { identity, value } => {
                    self.display_handle.disable_global::<NickelSession>(value);
                    identities_to_publish.insert(identity);
                    disabled = true;
                }
                GlobalRetirementAction::Remove { value, .. } => {
                    self.display_handle.remove_global::<NickelSession>(value);
                }
            }
        }
        for identity in identities_to_publish {
            if self.output_global_identity_available(&identity) {
                self.publish_live_output_global(&identity);
            }
        }
        if disabled {
            let mut display = self.display_handle.clone();
            let _ = display.flush_clients();
        }
    }

    fn publish_live_output_global(&mut self, identity: &str) {
        let virtual_output = self
            .virtual_test_outputs
            .get(identity)
            .filter(|(_, global)| global.is_none())
            .map(|(output, _)| output.clone());
        #[cfg(feature = "backend-udev")]
        let native_output = self.native_output_without_global(identity);
        #[cfg(not(feature = "backend-udev"))]
        let native_output: Option<Output> = None;
        let Some(output) = virtual_output.or(native_output) else {
            return;
        };
        let global = output.create_global::<NickelSession>(&self.display_handle);
        if let Some((_, slot)) = self.virtual_test_outputs.get_mut(identity) {
            *slot = Some(global);
            return;
        }
        #[cfg(feature = "backend-udev")]
        let _ = self.set_native_output_global(identity, global);
    }

    pub(crate) fn prune_dead_idle_inhibitors(&mut self) {
        // Smithay calls `uninhibit` for an explicit protocol destroy, but the
        // protocol also destroys every inhibitor when its client disconnects.
        // Those resources do not produce an `uninhibit` callback, so retain
        // the surface proxy and discard entries whose Wayland resource died.
        retain_live_idle_inhibitors(&mut self.idle_inhibitors, Resource::is_alive);
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
        let image_capture_source_state = ImageCaptureSourceState::new();
        let output_capture_source_state = OutputCaptureSourceState::new_with_filter::<Self, _>(
            &dh,
            crate::handlers::is_portal_capture_client,
        );
        let image_copy_capture_state = ImageCopyCaptureState::new_with_filter::<Self, _>(
            &dh,
            crate::handlers::is_portal_capture_client,
        );
        let popups = PopupManager::default();

        // A seat is a group of keyboards, pointer and touch devices.
        // A seat typically has a pointer and maintains a keyboard focus and a pointer focus.
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "winit");

        // Notify clients that we have a keyboard, for the sake of the example we assume that keyboard is always present.
        // You may want to track keyboard hot-plug in real compositor.
        // Match ordinary desktop repeat behavior. A 200 ms delay caused normal
        // key presses to enter repeat before users could release the key.
        seat.add_keyboard(Default::default(), 600, 25).unwrap();

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
        if test_control_enabled {
            let shell_test_path = control_socket_path
                .with_file_name(format!("nickel-shell-test-{}.sock", std::process::id()));
            // SAFETY: session initialization is single-threaded and precedes shell launch.
            unsafe { std::env::set_var("NICKEL_SHELL_TEST_CONTROL", shell_test_path) };
        } else {
            // SAFETY: session initialization is single-threaded and precedes shell launch.
            unsafe { std::env::remove_var("NICKEL_SHELL_TEST_CONTROL") };
        }
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
            active_pointer_constraint_origins: HashMap::new(),
            idle_inhibit_state,
            input_method_state,
            xwayland_shell_state,
            image_capture_source_state,
            output_capture_source_state,
            image_copy_capture_state,
            image_copy_sessions: Vec::new(),
            pending_image_copy_frames: Vec::new(),
            xwm: None,
            xwayland_restart_pending: false,
            xwayland_display: None,
            xwayland_registration: None,
            popups,
            seat,
            windows: WindowRegistry::default(),
            surface_windows: HashMap::new(),
            shell_owned_windows: HashSet::new(),
            x11_windows: HashMap::new(),
            launcher_window: None,
            launcher_visibility: LauncherVisibility::default(),
            launcher_output_name: None,
            launcher_focus: FocusTransactions::default(),
            launcher_restore_window: None,
            launcher_subscribers: Vec::new(),
            protocol_token,
            authenticated_shell_pids: HashSet::new(),
            registered_shell_role_slots: Vec::new(),
            last_logged_shell_readiness: None,
            test_control_enabled,
            #[cfg(target_os = "linux")]
            test_controller: None,
            expected_shell_pid: Arc::new(AtomicU32::new(0)),
            launcher_show_requested_at: None,
            desktop_windows: Vec::new(),
            panel_windows: Vec::new(),
            lock_windows: Vec::new(),
            locked: false,
            lock_restore_window: None,
            shell_focus_restore_window: None,
            pending_shell_focus_role: None,
            utility_windows: Vec::new(),
            screenshot_output_name: None,
            context_menu_window: None,
            preview_window: None,
            server_decorated: HashSet::new(),
            primary_output_name: None,
            virtual_test_outputs: HashMap::new(),
            pending_output_global_retirements: DeferredGlobalRetirements::default(),
            preview_frames: HashMap::new(),
            preview_spares: HashMap::new(),
            preview_switcher_interest: Vec::new(),
            preview_overlay_interest: Vec::new(),
            preview_admitted: HashSet::new(),
            preview_dirty: HashSet::new(),
            preview_content_generation: HashMap::new(),
            preview_attempted: HashMap::new(),
            preview_render_wave: 0,
            preview_retry_pending: HashSet::new(),
            preview_retry_scheduled: None,
            preview_retry_epoch: 1,
            preview_counters: PreviewCacheCounters::default(),
            hotkeys: CompositorShortcutAdapter::default(),
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
            identify_outputs_generation: 0,
            output_capture_path: None,
            output_capture_name: None,
            output_capture_reply_path: None,
            output_capture_request_id: None,
            shell_failure_count: 0,
            recovery_ui: crate::recovery_ui::RecoveryUi::new(),
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

    pub(crate) fn secure_storage_state(&self) -> crate::login_services::SecureStorageState {
        crate::login_services::SecureStorageState::from_u8(
            self.secure_storage_state.load(Ordering::Acquire),
        )
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
                            let is_preview = matches!(&message, ServerMessage::Preview(_));
                            match encode(&ServerEnvelope {
                                request_id,
                                message,
                            }) {
                                Ok(response) => {
                                    if is_preview {
                                        data.record_preview_protocol_encoding(
                                            response.len().saturating_sub(
                                                nickel_session_protocol::FRAME_HEADER_BYTES,
                                            ),
                                            response.len(),
                                        );
                                    }
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
                        data.windows.finish_snapshot();
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
        let request_id = envelope.request_id;
        if envelope.token != self.protocol_token {
            return protocol_error(ErrorCode::Unauthorized, "invalid session capability");
        }
        match envelope.request {
            Request::RegisterShell { pid } => {
                let expected_pid = self.expected_shell_pid.load(Ordering::Acquire);
                if let Some(rejection) = shell_registration_rejection(
                    expected_pid,
                    pid,
                    peer_pid,
                    same_session_user(pid),
                ) {
                    tracing::warn!(
                        operation = "register-shell",
                        correlation_id = request_id,
                        claimed_pid = pid,
                        peer_pid,
                        expected_pid,
                        rejection_category = ?rejection,
                        "shell registration rejected"
                    );
                    return protocol_error(
                        ErrorCode::Unauthorized,
                        "shell process is outside the active user session",
                    );
                }
                if !self.authenticated_shell_pids.contains(&pid) {
                    self.retire_shell_surface_roles();
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
                self.handle_protocol_command(command, source, request_id)
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
            Query::ShellReadiness => ServerMessage::ShellReadiness(self.protocol_shell_readiness()),
            Query::LauncherVisibility => ServerMessage::LauncherVisibility {
                visible: self.launcher_visibility.is_visible(),
            },
            Query::SecureStorage => ServerMessage::SecureStorage {
                state: self.protocol_secure_storage_state(),
                reason: (self.secure_storage_state()
                    == crate::login_services::SecureStorageState::Unavailable)
                    .then(crate::login_services::secure_storage_unavailable_reason)
                    .flatten(),
            },
            Query::IdleInhibition => ServerMessage::IdleInhibition {
                surfaces: {
                    self.prune_dead_idle_inhibitors();
                    u16::try_from(self.idle_inhibitors.len()).unwrap_or(u16::MAX)
                },
            },
            Query::CacheDiagnostics => {
                let metadata = self.windows.metadata_diagnostics();
                let titlebar = crate::window_frame::titlebar_cache_diagnostics();
                let recovery = self.recovery_ui.raster_diagnostics();
                #[cfg(feature = "backend-udev")]
                let identify = self
                    .native
                    .as_ref()
                    .map(crate::backend::udev::UdevData::identify_badge_diagnostics)
                    .unwrap_or_default();
                ServerMessage::CacheDiagnostics(nickel_session_protocol::CacheDiagnostics {
                    preview_entries: u16::try_from(self.preview_frames.len()).unwrap_or(u16::MAX),
                    preview_capacity: u16::try_from(PREVIEW_ENTRY_CAPACITY).unwrap_or(u16::MAX),
                    preview_bytes: self.preview_bytes() as u64,
                    preview_byte_capacity: PREVIEW_BYTE_CAPACITY as u64,
                    preview_peak_bytes: self.preview_counters.peak_bytes,
                    preview_admissions: self.preview_counters.admissions,
                    preview_evictions: self.preview_counters.evictions,
                    preview_invalidations: self.preview_counters.invalidations,
                    preview_captures: self.preview_counters.captures,
                    preview_skipped_unchanged: self.preview_counters.skipped_unchanged,
                    preview_readback_bytes: self.preview_counters.readback_bytes,
                    preview_protocol_copy_bytes: self.preview_counters.protocol_copy_bytes,
                    preview_protocol_raw_copy_bytes: self.preview_counters.protocol_raw_copy_bytes,
                    preview_protocol_base64_bytes: self.preview_counters.protocol_base64_bytes,
                    preview_protocol_json_payload_bytes: self
                        .preview_counters
                        .protocol_json_payload_bytes,
                    preview_protocol_framed_copy_bytes: self
                        .preview_counters
                        .protocol_framed_copy_bytes,
                    preview_capture_failures: self.preview_counters.capture_failures,
                    preview_cache_generation: self.preview_counters.cache_generation,
                    metadata_entries: u16::try_from(metadata.entries).unwrap_or(u16::MAX),
                    metadata_title_bytes: metadata.title_bytes as u64,
                    metadata_peak_title_bytes: metadata.peak_title_bytes as u64,
                    metadata_app_id_bytes: metadata.app_id_bytes as u64,
                    metadata_peak_app_id_bytes: metadata.peak_app_id_bytes as u64,
                    metadata_truncations: metadata.truncations,
                    metadata_canonicalizations: metadata.canonicalizations,
                    metadata_updates: metadata.updates,
                    metadata_live_snapshot_bytes: metadata.live_snapshot_bytes as u64,
                    metadata_peak_snapshot_bytes: metadata.peak_snapshot_bytes as u64,
                    titlebar_entries: u16::try_from(titlebar.entries).unwrap_or(u16::MAX),
                    titlebar_live_bytes: titlebar.live_bytes as u64,
                    titlebar_peak_bytes: titlebar.peak_bytes as u64,
                    titlebar_hits: titlebar.hits,
                    titlebar_misses: titlebar.misses,
                    titlebar_rasterizations: titlebar.rasterizations,
                    titlebar_avoided_rasterizations: titlebar.avoided_rasterizations,
                    titlebar_evictions: titlebar.evictions,
                    titlebar_generation: titlebar.generation,
                    titlebar_font_database_loads: titlebar.font_database_loads,
                    titlebar_renderer_bytes: titlebar.renderer_bytes.map(|bytes| bytes as u64),
                    recovery_entries: u16::try_from(recovery.entries).unwrap_or(u16::MAX),
                    recovery_live_bytes: recovery.live_bytes as u64,
                    recovery_peak_bytes: recovery.peak_bytes as u64,
                    recovery_rasterizations: recovery.rasterizations,
                    recovery_avoided_rasterizations: recovery.avoided_rasterizations,
                    recovery_evictions: recovery.evictions,
                    recovery_generation: recovery.generation,
                    recovery_renderer_bytes: recovery.renderer_bytes.map(|bytes| bytes as u64),
                    #[cfg(feature = "backend-udev")]
                    identify_entries: u16::try_from(identify.entries).unwrap_or(u16::MAX),
                    #[cfg(not(feature = "backend-udev"))]
                    identify_entries: 0,
                    #[cfg(feature = "backend-udev")]
                    identify_live_bytes: identify.live_bytes as u64,
                    #[cfg(not(feature = "backend-udev"))]
                    identify_live_bytes: 0,
                    #[cfg(feature = "backend-udev")]
                    identify_peak_bytes: identify.peak_bytes as u64,
                    #[cfg(not(feature = "backend-udev"))]
                    identify_peak_bytes: 0,
                    #[cfg(feature = "backend-udev")]
                    identify_rasterizations: identify.rasterizations,
                    #[cfg(not(feature = "backend-udev"))]
                    identify_rasterizations: 0,
                    #[cfg(feature = "backend-udev")]
                    identify_avoided_rasterizations: identify.avoided_rasterizations,
                    #[cfg(not(feature = "backend-udev"))]
                    identify_avoided_rasterizations: 0,
                    #[cfg(feature = "backend-udev")]
                    identify_evictions: identify.evictions,
                    #[cfg(not(feature = "backend-udev"))]
                    identify_evictions: 0,
                    #[cfg(feature = "backend-udev")]
                    identify_renderer_bytes: identify.renderer_bytes.map(|bytes| bytes as u64),
                    #[cfg(not(feature = "backend-udev"))]
                    identify_renderer_bytes: None,
                })
            }
            Query::Workspaces => ServerMessage::Workspaces(self.protocol_workspaces()),
            Query::Preview { window } => {
                let id = WindowId(window.0);
                if !self.windows.snapshot().iter().any(|entry| entry.id == id) {
                    return protocol_error(ErrorCode::InvalidWindow, "unknown window id");
                }
                let Some(preview) =
                    protocol_preview_from_cached(window, self.preview_frames.get(&id))
                else {
                    return protocol_error(ErrorCode::InvalidRequest, "preview is not ready");
                };
                self.preview_counters.protocol_copy_bytes += preview.rgba.len() as u64;
                self.preview_counters.protocol_raw_copy_bytes += preview.rgba.len() as u64;
                let base64_bytes = 4 * preview.rgba.len().div_ceil(3);
                self.preview_counters.protocol_base64_bytes += base64_bytes as u64;
                self.preview_counters.protocol_copy_bytes += base64_bytes as u64;
                if preview.validate().is_err() {
                    return protocol_error(ErrorCode::ResourceLimit, "preview exceeds bounds");
                }
                ServerMessage::Preview(preview)
            }
            Query::ShellSemanticTarget { .. } | Query::ShellRuntimeDiagnostics => protocol_error(
                ErrorCode::InvalidRequest,
                "shell-only queries are resolved by the nested shell test endpoint",
            ),
        }
    }

    fn handle_protocol_command(
        &mut self,
        command: SessionCommand,
        source: Option<&std::path::Path>,
        request_id: u64,
    ) -> ServerMessage {
        match command {
            SessionCommand::ReloadShellSettings => {
                self.notify_shell_settings_changed();
            }
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
                    if windows.iter().any(|window| !self.window_exists(*window)) {
                        return protocol_error(ErrorCode::InvalidWindow, "unknown preview window");
                    }
                    self.set_overlay_preview_interest(
                        windows.iter().map(|window| WindowId(window.0)).collect(),
                    );
                    self.show_preview(geometry.x, geometry.width, geometry.height)
                }
                _ => {
                    return protocol_error(
                        ErrorCode::InvalidRequest,
                        "role is not a transient overlay",
                    );
                }
            },
            SessionCommand::FocusShellRole { role } => {
                if !shell_role_accepts_ordinary_focus(role) {
                    return protocol_error(
                        ErrorCode::InvalidRequest,
                        "role does not accept ordinary shell focus",
                    );
                }
                self.focus_shell_role(role);
            }
            SessionCommand::RestoreApplicationFocus => self.restore_application_focus(),
            SessionCommand::IdentifyOutputs => self.begin_output_identification(),
            SessionCommand::CaptureOutput { path, output } => {
                if path.is_empty() {
                    return protocol_error(ErrorCode::InvalidRequest, "capture path is empty");
                }
                if let Some(name) = output.as_deref()
                    && !self
                        .protocol_outputs()
                        .iter()
                        .any(|candidate| candidate.enabled && candidate.name == name)
                {
                    return protocol_error(
                        ErrorCode::InvalidRequest,
                        "capture output was not found",
                    );
                }
                self.output_capture_path = Some(PathBuf::from(path));
                self.output_capture_name = output;
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
            .any(|window| window.id.0 == id.0 && !self.shell_owned_windows.contains(&window.id))
    }

    fn protocol_windows(&mut self) -> Vec<WindowSnapshot> {
        let mut shell_ids = self
            .shell_windows()
            .filter_map(|window| {
                self.surface_windows
                    .get(&window.toplevel()?.wl_surface().id())
            })
            .copied()
            .collect::<HashSet<_>>();
        shell_ids.extend(self.shell_owned_windows.iter().copied());
        let windows = self
            .windows
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
                let geometry = self
                    .window_for_registry_id(window.id)
                    .and_then(|candidate| self.space.element_bbox(&candidate))
                    .map(|bounds| ProtocolGeometry {
                        x: bounds.loc.x,
                        y: bounds.loc.y,
                        width: bounds.size.w,
                        height: bounds.size.h,
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
            .collect::<Vec<_>>();
        let snapshot_bytes = windows
            .iter()
            .map(|window| window.title.len() + window.application_id.len())
            .sum();
        self.windows.begin_snapshot(snapshot_bytes);
        windows
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
                if !self.output_global_admission_available() {
                    return Err("output global retirement backlog is full");
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
                let global = self
                    .output_global_identity_available(&name)
                    .then(|| output.create_global::<NickelSession>(&self.display_handle));
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
                if let Some(global) = global {
                    self.defer_output_global_retirement(name.clone(), global);
                }
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
        let outputs = self
            .space
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
                    enabled: true,
                })
            })
            .take(nickel_session_protocol::MAX_OUTPUTS)
            .collect::<Vec<_>>();
        #[cfg(feature = "backend-udev")]
        let mut outputs = outputs;
        #[cfg(feature = "backend-udev")]
        if let Some(native) = self.native.as_ref() {
            for output in native.disabled_outputs() {
                if outputs.len() >= nickel_session_protocol::MAX_OUTPUTS {
                    break;
                }
                let Some(mode) = output.current_mode() else {
                    continue;
                };
                let physical = output.physical_properties();
                let location = output.current_location();
                outputs.push(OutputSnapshot {
                    name: output.name(),
                    model: physical.model,
                    geometry: ProtocolGeometry {
                        x: location.x,
                        y: location.y,
                        width: mode.size.w,
                        height: mode.size.h,
                    },
                    work_area: ProtocolGeometry {
                        x: location.x,
                        y: location.y,
                        width: mode.size.w,
                        height: mode.size.h,
                    },
                    scale_120: (output.current_scale().fractional_scale() * 120.0).round() as u32,
                    transform: OutputTransform::Normal,
                    physical_width_mm: physical.size.w,
                    physical_height_mm: physical.size.h,
                    primary: false,
                    enabled: false,
                });
            }
        }
        outputs
    }

    pub(crate) fn protocol_shell_surfaces(&self) -> Vec<ShellSurfaceSnapshot> {
        let registry = self.windows.snapshot();
        let outputs = self.space.outputs().cloned().collect::<Vec<_>>();
        let output_names = outputs.iter().map(Output::name).collect::<Vec<_>>();
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
                let output = self
                    .shell_surface_output_name(window)
                    .and_then(|name| output_index_for_shell_surface(&name, &output_names))
                    .map(|index| output_names[index].clone())
                    .or_else(|| {
                        bounds.and_then(|bounds| {
                            outputs
                                .iter()
                                .filter_map(|output| {
                                    let output_bounds = self.space.output_geometry(output)?;
                                    let left = bounds.loc.x.max(output_bounds.loc.x);
                                    let top = bounds.loc.y.max(output_bounds.loc.y);
                                    let right = (bounds.loc.x + bounds.size.w)
                                        .min(output_bounds.loc.x + output_bounds.size.w);
                                    let bottom = (bounds.loc.y + bounds.size.h)
                                        .min(output_bounds.loc.y + output_bounds.size.h);
                                    let area = i64::from((right - left).max(0))
                                        * i64::from((bottom - top).max(0));
                                    Some((area, output.name()))
                                })
                                .max_by_key(|(area, _)| *area)
                                .filter(|(area, _)| *area > 0)
                                .map(|(_, name)| name)
                        })
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

    pub(crate) fn protocol_shell_readiness(
        &self,
    ) -> nickel_session_protocol::ShellReadinessSnapshot {
        let expected_shell_pid = match self.expected_shell_pid.load(Ordering::Acquire) {
            0 => None,
            pid => Some(pid),
        };
        let authenticated_shell_pid = self.authenticated_shell_pids.iter().copied().next();
        let outputs = u16::try_from(self.space.outputs().count()).unwrap_or(u16::MAX);
        let surfaces = self.protocol_shell_surfaces();
        let role_count = |role| {
            u16::try_from(
                surfaces
                    .iter()
                    .filter(|surface| surface.role == role)
                    .count(),
            )
            .unwrap_or(u16::MAX)
        };
        let desktops = role_count(ShellRole::Desktop);
        let panels = role_count(ShellRole::Panel);
        let registered_role_count = |role| {
            u16::try_from(
                self.registered_shell_role_slots
                    .iter()
                    .filter(|registration| registration.role == role)
                    .count(),
            )
            .unwrap_or(u16::MAX)
        };
        let locks = registered_role_count(ShellRole::Lock);
        let launchers = u16::from(
            self.launcher_window
                .as_ref()
                .is_some_and(|window| window.alive()),
        );
        let reserved_ordinary_windows = u16::try_from(
            self.windows
                .snapshot()
                .iter()
                .filter(|window| {
                    ShellRole::from_application_id(&window.app_id).is_some()
                        && !self.shell_owned_windows.contains(&window.id)
                })
                .count(),
        )
        .unwrap_or(u16::MAX);
        let required_singletons_ready = [
            ShellRole::Launcher,
            ShellRole::ControlCenter,
            ShellRole::ContextMenu,
            ShellRole::Preview,
            ShellRole::Notification,
            ShellRole::ProjectMenu,
            ShellRole::Screenshot,
        ]
        .into_iter()
        .all(|role| registered_role_count(role) == 1 && role_count(role) <= 1);
        let output_names = self
            .space
            .outputs()
            .map(|output| output.name())
            .collect::<HashSet<_>>();
        let registered_role_outputs = |role| {
            self.registered_shell_role_slots
                .iter()
                .filter(|registration| registration.role == role)
                .filter_map(|registration| registration.output.clone())
                .collect::<HashSet<_>>()
        };
        let live_role_outputs = |role| {
            surfaces
                .iter()
                .filter(|surface| surface.role == role)
                .filter_map(|surface| surface.output.clone())
                .collect::<HashSet<_>>()
        };
        let output_roles_ready = live_role_outputs(ShellRole::Desktop) == output_names
            && live_role_outputs(ShellRole::Panel) == output_names
            && registered_role_outputs(ShellRole::Lock) == output_names;
        let ready = expected_shell_pid.is_some()
            && expected_shell_pid == authenticated_shell_pid
            && desktops == outputs
            && panels == outputs
            && locks == outputs
            && launchers == 1
            && required_singletons_ready
            && output_roles_ready
            && reserved_ordinary_windows == 0;
        nickel_session_protocol::ShellReadinessSnapshot {
            expected_shell_pid,
            authenticated_shell_pid,
            outputs,
            desktops,
            panels,
            locks,
            launchers,
            required_singletons_ready,
            output_roles_ready,
            reserved_ordinary_windows,
            ready,
        }
    }

    pub(crate) fn log_shell_readiness_if_changed(&mut self) {
        let readiness = self.protocol_shell_readiness();
        if self.last_logged_shell_readiness.as_ref() == Some(&readiness) {
            return;
        }
        tracing::info!(
            expected_shell_pid = ?readiness.expected_shell_pid,
            authenticated_shell_pid = ?readiness.authenticated_shell_pid,
            outputs = readiness.outputs,
            desktops = readiness.desktops,
            panels = readiness.panels,
            locks = readiness.locks,
            launchers = readiness.launchers,
            required_singletons_ready = readiness.required_singletons_ready,
            output_roles_ready = readiness.output_roles_ready,
            reserved_ordinary_windows = readiness.reserved_ordinary_windows,
            control_channel_health = "available",
            ready = readiness.ready,
            "shell readiness changed"
        );
        self.last_logged_shell_readiness = Some(readiness);
    }

    fn protocol_snapshot(&mut self) -> SessionSnapshot {
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
        match self.secure_storage_state() {
            crate::login_services::SecureStorageState::Starting => ProtocolSecureStorage::Starting,
            crate::login_services::SecureStorageState::Locked => ProtocolSecureStorage::Locked,
            crate::login_services::SecureStorageState::PromptRequired => {
                ProtocolSecureStorage::PromptRequired
            }
            crate::login_services::SecureStorageState::Ready => ProtocolSecureStorage::Ready,
            crate::login_services::SecureStorageState::Unavailable => {
                ProtocolSecureStorage::Unavailable
            }
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
        #[cfg(feature = "backend-udev")]
        let mut connected = connected;
        #[cfg(feature = "backend-udev")]
        for (name, size) in self.native_output_inventory() {
            connected.entry(name.clone()).or_insert_with(|| {
                let output = self
                    .native
                    .as_ref()
                    .and_then(|native| {
                        native
                            .disabled_outputs()
                            .find(|output| output.name() == name)
                            .cloned()
                    })
                    .expect("disabled inventory retains its output");
                (output, size)
            });
        }
        if placements.len() != connected.len() {
            return Err("layout must include every connected output");
        }
        let mut names = HashSet::new();
        if placements.iter().any(|placement| {
            !connected.contains_key(&placement.name) || !names.insert(&placement.name)
        }) {
            return Err("layout contains an unknown or duplicate output");
        }
        if !placements
            .iter()
            .any(|placement| placement.name == primary && placement.enabled)
        {
            return Err("primary output must be enabled");
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
        for (index, left) in placements
            .iter()
            .enumerate()
            .filter(|(_, output)| output.enabled)
        {
            let left_size = connected[&left.name].1;
            for right in placements
                .iter()
                .skip(index + 1)
                .filter(|output| output.enabled)
            {
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

        #[cfg(feature = "backend-udev")]
        for placement in placements.iter().filter(|placement| placement.enabled) {
            self.set_native_output_enabled(&placement.name, true)?;
        }
        #[cfg(not(feature = "backend-udev"))]
        if placements.iter().any(|placement| !placement.enabled) {
            return Err("output disabling requires the native DRM backend");
        }
        self.primary_output_name = Some(primary.clone());
        #[cfg(feature = "backend-udev")]
        for placement in placements.iter().filter(|placement| !placement.enabled) {
            self.set_native_output_enabled(&placement.name, false)?;
        }
        for placement in placements.iter().filter(|placement| placement.enabled) {
            let output = self
                .space
                .outputs()
                .find(|output| output.name() == placement.name)
                .cloned()
                .ok_or("enabled output did not become active")?;
            let location = (placement.x, placement.y).into();
            output.change_current_state(None, None, None, Some(location));
            self.space.map_output(&output, location);
        }
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
            .filter(|window| !self.is_shell_owned_window(window))
            .filter(|window| {
                self.space
                    .element_bbox(window)
                    .is_some_and(|bounds| !outputs.iter().any(|output| output.overlaps(bounds)))
            })
            .cloned()
            .collect();
        for window in stranded {
            self.map_compositor_moved_window(window, fallback.into(), false);
        }
    }

    pub(crate) fn stage_output_removal(&mut self, output: &Output) {
        self.fail_image_copy_frames(
            output,
            smithay::wayland::image_copy_capture::CaptureFailureReason::Stopped,
        );
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
            self.map_compositor_moved_window(window, rescue_location, false);
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
                self.map_compositor_moved_window(window, location, false);
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

    pub fn toggle_launcher_visibility(&mut self) {
        self.set_launcher_visible(!self.launcher_visibility.is_visible());
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

    fn notify_shell_settings_changed(&mut self) {
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::ShellSettingsChanged),
        }) else {
            return;
        };
        let Ok(socket) = UnixDatagram::unbound() else {
            return;
        };
        self.launcher_subscribers
            .retain(|path| socket.send_to(&event, path).is_ok());
    }

    pub(crate) fn notify_global_shortcut(
        &mut self,
        action: nickel_session_protocol::ShortcutAction,
    ) {
        tracing::info!(?action, "global shortcut activated");
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::GlobalShortcut { action }),
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
        let event = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::Snapshot(self.protocol_snapshot())),
        });
        self.windows.finish_snapshot();
        let Ok(event) = event else {
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
            let location = Self::shell_surface_location(&window, geometry);
            self.space.map_element(window.clone(), location, true);
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

    pub(crate) fn focus_shell_role(&mut self, role: ShellRole) -> bool {
        if role == ShellRole::Screenshot {
            self.screenshot_output_name = self.preferred_interaction_output_name();
        }
        self.pending_shell_focus_role = Some(role);
        let registry = self.windows.snapshot();
        let target = self.shell_windows().find_map(|window| {
            let id = window
                .wl_surface()
                .and_then(|surface| self.surface_windows.get(&surface.id()))?;
            let app_id = registry
                .iter()
                .find(|entry| entry.id == *id)?
                .app_id
                .as_str();
            (ShellRole::from_application_id(app_id) == Some(role)).then(|| window.clone())
        });
        let Some(target) = target else {
            return false;
        };
        if self.shell_focus_restore_window.is_none() {
            let shell_ids = self
                .shell_windows()
                .filter_map(|window| {
                    window
                        .wl_surface()
                        .and_then(|surface| self.surface_windows.get(&surface.id()))
                        .copied()
                })
                .collect::<HashSet<_>>();
            let focused = self
                .seat
                .get_keyboard()
                .and_then(|keyboard| keyboard.current_focus())
                .and_then(|focus| match focus {
                    crate::focus::KeyboardFocusTarget::Wayland(surface) => {
                        self.surface_windows.get(&surface.id()).copied()
                    }
                    crate::focus::KeyboardFocusTarget::X11(surface) => {
                        self.x11_windows.get(&surface.window_id()).copied()
                    }
                })
                .filter(|window| !shell_ids.contains(window));
            self.shell_focus_restore_window = focused.or_else(|| {
                registry
                    .iter()
                    .find(|window| window.active && !shell_ids.contains(&window.id))
                    .map(|window| window.id)
            });
        }
        if role == ShellRole::Screenshot {
            self.place_screenshot_surface(&target);
        }
        self.space.raise_element(&target, true);
        self.seat.get_keyboard().unwrap().set_focus(
            self,
            crate::focus::KeyboardFocusTarget::for_window(&target),
            SERIAL_COUNTER.next_serial(),
        );
        self.space.elements().for_each(|window| {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_pending_configure();
            }
        });
        true
    }

    fn restore_application_focus(&mut self) {
        if self.pending_shell_focus_role == Some(ShellRole::Screenshot) {
            self.screenshot_output_name = None;
        }
        self.pending_shell_focus_role = None;
        if let Some(window) = self.shell_focus_restore_window.take() {
            self.activate_window(window);
        } else {
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                Option::<crate::focus::KeyboardFocusTarget>::None,
                SERIAL_COUNTER.next_serial(),
            );
        }
    }

    pub fn register_launcher(&mut self, window: Window) {
        if let Some(previous) = self.launcher_window.clone()
            && previous != window
        {
            self.retire_replaced_shell_window(previous);
        }
        self.space.unmap_elem(&window);
        self.launcher_window = Some(window);
        self.apply_launcher_visibility(self.launcher_visibility.is_visible());
    }

    fn retire_replaced_shell_window(&mut self, window: Window) {
        self.space.unmap_elem(&window);
        if let Some(toplevel) = window.toplevel() {
            toplevel.send_close();
        }
        let surface_id = window.wl_surface().map(|surface| surface.id());
        let window_id = surface_id
            .as_ref()
            .and_then(|surface| self.surface_windows.get(surface))
            .copied();
        self.retire_surface_window_references(surface_id.as_ref(), window_id);
    }

    fn retire_shell_surface_roles(&mut self) {
        // A replacement shell generation must never expose the previous
        // generation through the ordinary Space render/input paths or retain
        // its derived identity until a later destruction callback.
        let retired = self.shell_windows().cloned().collect::<Vec<_>>();
        for window in retired {
            self.space.unmap_elem(&window);
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_close();
            }
            let surface_id = window.wl_surface().map(|surface| surface.id());
            let window_id = surface_id
                .as_ref()
                .and_then(|surface| self.surface_windows.get(surface))
                .copied();
            self.retire_surface_window_references(surface_id.as_ref(), window_id);
        }
        self.launcher_window = None;
        self.desktop_windows.clear();
        self.panel_windows.clear();
        self.lock_windows.clear();
        self.utility_windows.clear();
        self.context_menu_window = None;
        self.preview_window = None;
        self.registered_shell_role_slots.clear();
        self.clear_all_previews();
    }

    /// Remove every derived reference owned for a surface/window identity.
    ///
    /// Destruction callbacks must use this operation instead of independently
    /// editing lifecycle collections. X11 windows can lack a Wayland surface,
    /// while shell surfaces can be retired before their client destroys the
    /// underlying object, so both halves of the identity are optional.
    pub(crate) fn retire_surface_window_references(
        &mut self,
        surface_id: Option<&ObjectId>,
        window_id: Option<WindowId>,
    ) {
        if let Some(surface_id) = surface_id {
            let _ = retire_pointer_surface(
                &mut self.pointer_lock_hints,
                &mut self.active_pointer_locks,
                &mut self.active_pointer_constraint_origins,
                surface_id,
            );
            self.server_decorated.remove(surface_id);
            self.maximized_restore.remove(surface_id);
            self.fullscreen_restore.remove(surface_id);
            self.surface_windows.remove(surface_id);
            retire_shell_surface(&mut self.registered_shell_role_slots, surface_id);
            self.idle_inhibitors
                .retain(|surface, _| surface.id() != *surface_id);
            if self
                .last_titlebar_click
                .as_ref()
                .is_some_and(|(surface, _, _)| surface == surface_id)
            {
                self.last_titlebar_click = None;
            }

            let matches_surface = |window: &Window| {
                window
                    .wl_surface()
                    .is_some_and(|surface| surface.id() == *surface_id)
            };
            if self.launcher_window.as_ref().is_some_and(&matches_surface) {
                self.launcher_window = None;
                self.launcher_visibility.set(false);
            }
            self.desktop_windows
                .retain(|window| !matches_surface(window));
            self.panel_windows.retain(|window| !matches_surface(window));
            self.lock_windows.retain(|window| !matches_surface(window));
            self.utility_windows
                .retain(|window| !matches_surface(window));
            if self
                .context_menu_window
                .as_ref()
                .is_some_and(&matches_surface)
            {
                self.context_menu_window = None;
            }
            if self.preview_window.as_ref().is_some_and(&matches_surface) {
                self.preview_window = None;
                self.clear_overlay_preview_interest();
            }
        }

        if let Some(window_id) = window_id {
            self.surface_windows
                .retain(|_, retained| *retained != window_id);
            self.shell_owned_windows.remove(&window_id);
            self.minimized_windows.remove(&window_id);
            self.workspace_hidden_windows.remove(&window_id);
            self.workspaces.remove_window(&window_id);
            self.remove_window_from_switcher(window_id);
            self.windows.remove(window_id);
            self.preview_highlight = self
                .preview_highlight
                .filter(|candidate| *candidate != window_id);
            self.launcher_restore_window = self
                .launcher_restore_window
                .filter(|candidate| *candidate != window_id);
            self.lock_restore_window = self
                .lock_restore_window
                .filter(|candidate| *candidate != window_id);
            self.shell_focus_restore_window = self
                .shell_focus_restore_window
                .filter(|candidate| *candidate != window_id);
            retire_displaced_window(&mut self.displaced_output_windows, window_id);
        }

        // Churn of dead identities must not retain admission-sized backing
        // allocations once the logical collections return to baseline.
        let counts = self.lifecycle_collection_counts();
        tracing::trace!(
            pointer_hints = counts.pointer_hints,
            pointer_locks = counts.pointer_locks,
            pointer_origins = counts.pointer_origins,
            displaced_outputs = counts.displaced_outputs,
            displaced_windows = counts.displaced_windows,
            shell_roles = counts.shell_roles,
            "retired session identity references"
        );
        debug_assert!(self.lifecycle_references_are_live());
    }

    pub(crate) fn retire_pointer_constraint_references(
        &mut self,
        surface_id: &ObjectId,
    ) -> Option<Point<f64, Logical>> {
        retire_pointer_surface(
            &mut self.pointer_lock_hints,
            &mut self.active_pointer_locks,
            &mut self.active_pointer_constraint_origins,
            surface_id,
        )
    }

    pub(crate) fn clear_changed_shell_surface_role(
        &mut self,
        surface_id: &ObjectId,
        next_role: Option<ShellRole>,
    ) {
        let role_changed = shell_registration_role_changed(
            &self.registered_shell_role_slots,
            surface_id,
            next_role,
        );
        if !role_changed {
            return;
        }
        retire_shell_surface(&mut self.registered_shell_role_slots, surface_id);
        let matches_surface = |window: &Window| {
            window
                .wl_surface()
                .is_some_and(|surface| surface.id() == *surface_id)
        };
        if self.launcher_window.as_ref().is_some_and(&matches_surface) {
            self.launcher_window = None;
            self.launcher_visibility.set(false);
        }
        self.desktop_windows
            .retain(|window| !matches_surface(window));
        self.panel_windows.retain(|window| !matches_surface(window));
        self.lock_windows.retain(|window| !matches_surface(window));
        self.utility_windows
            .retain(|window| !matches_surface(window));
        if self
            .context_menu_window
            .as_ref()
            .is_some_and(&matches_surface)
        {
            self.context_menu_window = None;
        }
        if self.preview_window.as_ref().is_some_and(&matches_surface) {
            self.preview_window = None;
            self.clear_overlay_preview_interest();
        }
    }

    pub(crate) fn lifecycle_collection_counts(&self) -> LifecycleCollectionCounts {
        LifecycleCollectionCounts {
            pointer_hints: self.pointer_lock_hints.len(),
            pointer_locks: self.active_pointer_locks.len(),
            pointer_origins: self.active_pointer_constraint_origins.len(),
            displaced_outputs: self.displaced_output_windows.len(),
            displaced_windows: self.displaced_output_windows.values().map(Vec::len).sum(),
            shell_roles: self.registered_shell_role_slots.len(),
        }
    }

    fn lifecycle_references_are_live(&self) -> bool {
        let backend = self.display_handle.backend_handle();
        let live_surface = |surface: &ObjectId| backend.object_info(surface.clone()).is_ok();
        self.pointer_lock_hints.keys().all(live_surface)
            && self.active_pointer_locks.iter().all(live_surface)
            && self
                .active_pointer_constraint_origins
                .keys()
                .all(live_surface)
            && self
                .registered_shell_role_slots
                .iter()
                .all(|registration| live_surface(&registration.surface))
            && self
                .surface_windows
                .iter()
                .all(|(surface, window)| live_surface(surface) && self.windows.contains(*window))
            && self
                .displaced_output_windows
                .values()
                .flatten()
                .all(|displaced| self.windows.contains(displaced.id))
    }

    pub(crate) fn record_shell_role_registration(&mut self, window: &Window, role: ShellRole) {
        let output = matches!(
            role,
            ShellRole::Desktop | ShellRole::Panel | ShellRole::Lock
        )
        .then(|| self.shell_surface_output_name(window))
        .flatten()
        .and_then(|name| {
            let output_names = self.space.outputs().map(Output::name).collect::<Vec<_>>();
            output_index_for_shell_surface(&name, &output_names)
                .map(|index| output_names[index].clone())
        });
        if matches!(
            role,
            ShellRole::Desktop | ShellRole::Panel | ShellRole::Lock
        ) && output.is_none()
        {
            return;
        }
        let Some(surface) = window.wl_surface() else {
            return;
        };
        let registration = RegisteredShellRole {
            role,
            output,
            surface: surface.id(),
        };
        // A title/app-id update on the same live surface may change its output
        // slot. Replace that registration rather than treating the original
        // tuple as permanent history.
        retire_shell_surface(&mut self.registered_shell_role_slots, &registration.surface);
        let replacements = self
            .registered_shell_role_slots
            .iter()
            .filter(|existing| {
                existing.role == registration.role
                    && existing.output == registration.output
                    && existing.surface != registration.surface
            })
            .map(|existing| existing.surface.clone())
            .collect::<Vec<_>>();
        for replaced_surface in replacements {
            let window_id = self.surface_windows.get(&replaced_surface).copied();
            let replaced_window = self
                .shell_windows()
                .find(|candidate| {
                    candidate
                        .wl_surface()
                        .is_some_and(|surface| surface.id() == replaced_surface)
                })
                .cloned();
            if let Some(replaced_window) = replaced_window {
                self.retire_replaced_shell_window(replaced_window);
            } else {
                self.retire_surface_window_references(Some(&replaced_surface), window_id);
            }
        }
        if !self
            .registered_shell_role_slots
            .iter()
            .any(|existing| existing.surface == registration.surface)
        {
            self.registered_shell_role_slots.push(registration);
        }
    }

    pub fn register_panel(&mut self, window: Window) {
        // Smithay's ordinary xdg windows use z-index 30. Keep the Nickel panel
        // in its top shell layer so later application maps cannot cover it.
        window.override_z_index(40);
        self.panel_windows.retain(IsAlive::alive);
        if !self.panel_windows.contains(&window) {
            self.panel_windows.push(window);
        }
        self.relayout_shell_surfaces();
    }

    pub fn register_desktop(&mut self, window: Window) {
        window.override_z_index(0);
        self.desktop_windows.retain(IsAlive::alive);
        if !self.desktop_windows.contains(&window) {
            self.desktop_windows.push(window);
        }
        self.relayout_shell_surfaces();
    }

    pub fn is_panel_window(&self, window: &Window) -> bool {
        self.panel_windows.contains(window)
    }

    pub fn is_shell_owned_window(&self, window: &Window) -> bool {
        window
            .wl_surface()
            .and_then(|surface| self.surface_windows.get(&surface.id()))
            .is_some_and(|id| self.shell_owned_windows.contains(id))
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
        window.x11_surface().is_some_and(|surface| {
            self.x11_windows.contains_key(&surface.window_id()) && !surface.is_decorated()
        }) || window
            .toplevel()
            .is_some_and(|surface| self.server_decorated.contains(&surface.wl_surface().id()))
    }

    pub(crate) fn clamp_initial_managed_x11_geometry(
        &self,
        geometry: Rectangle<i32, Logical>,
    ) -> Rectangle<i32, Logical> {
        let content = Geometry {
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w.max(1),
            height: geometry.size.h.max(1),
        };
        let outputs = self
            .space
            .outputs()
            .filter_map(|output| self.space.output_geometry(output))
            .map(|output| Geometry {
                x: output.loc.x,
                y: output.loc.y,
                width: output.size.w,
                height: output.size.h,
            })
            .collect::<Vec<_>>();
        let Some(output) = shell_layout::output_for_window(content, &outputs) else {
            return geometry;
        };
        let content =
            clamp_decorated_content_to_work_area(content, self.work_area_for_output(output));
        Rectangle::new(
            (content.x, content.y).into(),
            (content.width, content.height).into(),
        )
    }

    pub(crate) fn map_compositor_moved_window(
        &mut self,
        window: Window,
        location: Point<i32, Logical>,
        activate: bool,
    ) {
        if let Some(surface) = window.x11_surface()
            && self.x11_windows.contains_key(&surface.window_id())
        {
            let configured = surface.last_configure();
            let size = if configured.size.w > 0 && configured.size.h > 0 {
                configured.size
            } else {
                window.geometry().size
            };
            let _ = surface.configure(Rectangle::new(location, size));
        }
        self.space.map_element(window, location, activate);
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
            .filter(|window| window.alive())
    }

    pub fn register_utility_window(&mut self, window: Window, role: ShellRole) {
        self.utility_windows.retain(IsAlive::alive);
        if !self.utility_windows.contains(&window) {
            self.utility_windows.push(window.clone());
        }
        if role == ShellRole::Screenshot {
            self.place_screenshot_surface(&window);
        }
    }

    fn place_screenshot_surface(&mut self, window: &Window) {
        let output_name = self
            .screenshot_output_name
            .clone()
            .or_else(|| self.preferred_interaction_output_name());
        let Some(output) = output_name
            .as_deref()
            .and_then(|name| self.output_geometry_named(name))
            .or_else(|| self.output_geometry_for_shell())
        else {
            return;
        };
        if self.screenshot_output_name.is_none() {
            self.screenshot_output_name = output_name;
        }
        let size = window.geometry().size;
        if size.w <= 0 || size.h <= 0 {
            return;
        }
        let target = shell_layout::centered_in(output, (size.w, size.h));
        if size.w != target.width || size.h != target.height {
            Self::configure_window(window, target);
        }
        let location = Self::shell_surface_location(window, target);
        self.space.map_element(window.clone(), location, true);
    }

    pub(crate) fn relayout_committed_shell_window(&mut self, window: &Window) {
        let is_screenshot = {
            let registry = self.windows.snapshot();
            window
                .wl_surface()
                .and_then(|surface| self.surface_windows.get(&surface.id()))
                .and_then(|id| registry.iter().find(|entry| entry.id == *id))
                .and_then(|entry| ShellRole::from_application_id(&entry.app_id))
                == Some(ShellRole::Screenshot)
        };
        if is_screenshot {
            self.place_screenshot_surface(window);
        }
    }

    pub fn register_lock(&mut self, window: Window) {
        window.override_z_index(100);
        self.lock_windows.retain(IsAlive::alive);
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
                self.retire_replaced_shell_window(stale);
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
        let output_names = outputs.iter().map(Output::name).collect::<Vec<_>>();
        for lock in self.lock_windows.clone() {
            let Some(output_name) = self.shell_surface_output_name(&lock) else {
                continue;
            };
            let Some(output_index) = output_index_for_shell_surface(&output_name, &output_names)
            else {
                continue;
            };
            let output = &outputs[output_index];
            let Some(geometry) = self.space.output_geometry(output) else {
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
        if let Some(previous) = self.context_menu_window.clone()
            && previous != window
        {
            self.retire_replaced_shell_window(previous);
        }
        window.override_z_index(50);
        self.context_menu_window = Some(window);
    }

    pub fn register_preview(&mut self, window: Window) {
        if let Some(previous) = self.preview_window.clone()
            && previous != window
        {
            self.retire_replaced_shell_window(previous);
        }
        window.override_z_index(49);
        if self
            .preview_window
            .as_ref()
            .is_some_and(|registered| registered != &window)
        {
            self.clear_overlay_preview_interest();
        }
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
        // Passive previews must not deactivate the current application merely
        // because their shell surface became mapped. Explicit keyboard/menu
        // focus remains authoritative through the `focus` argument.
        self.space.map_element(window.clone(), (x, y), focus);
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
        self.space.raise_element(&window, focus);
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
        self.clear_overlay_preview_interest();
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
        let Some(window) = self.window_for_registry_id(id) else {
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
        self.preview_switcher_interest
            .retain(|candidate| *candidate != id);
        self.preview_overlay_interest
            .retain(|candidate| *candidate != id);
        self.preview_admitted.remove(&id);
        self.drop_preview_frame(&id);
        self.reconcile_preview_admission();
        let effects = self.task_switcher.remove_candidate(&id);
        self.apply_task_switch_effects(effects);
    }

    pub(crate) fn restore_focus_after_window_removal(&mut self, restore: bool) {
        if !restore || self.locked {
            return;
        }
        let replacement = self
            .workspaces
            .ordered()
            .iter()
            .find(|workspace| workspace.id == self.workspaces.active())
            .and_then(|workspace| workspace.last_focused)
            .filter(|candidate| !self.minimized_windows.contains_key(candidate));
        if let Some(replacement) = replacement {
            self.activate_window(replacement);
        } else {
            self.windows.deactivate_all();
            self.seat
                .get_keyboard()
                .unwrap()
                .set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
    }

    fn apply_task_switch_effects(&mut self, effects: Vec<TaskSwitchEffect<WindowId>>) {
        for effect in effects {
            match effect {
                TaskSwitchEffect::RequestPreviews(ids) => {
                    let ids = bounded_preview_ids(ids, self.task_switcher.selected_index());
                    self.set_switcher_preview_interest(ids);
                }
                TaskSwitchEffect::ActivateWindow(id) => self.activate_window(id),
                TaskSwitchEffect::ShowFlip { .. } => {}
                TaskSwitchEffect::SelectPreview(_) => {
                    let ids = bounded_preview_ids(
                        self.task_switcher.candidates().to_vec(),
                        self.task_switcher.selected_index(),
                    );
                    self.set_switcher_preview_interest(ids);
                }
                TaskSwitchEffect::HideFlip { .. } => self.clear_switcher_preview_interest(),
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
        let x11_window = self.window_for_registry_id(id);
        if let Some(window) = x11_window
            && let Some(surface) = window.x11_surface()
        {
            let maximize = !surface.is_maximized();
            let _ = surface.set_maximized(maximize);
            if maximize {
                self.apply_maximized_x11_geometry(&window, surface, true);
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
        let window = self.window_for_registry_id(id);
        if let Some(surface) = window.as_ref().and_then(Window::x11_surface).cloned() {
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
        let surface = window.as_ref().and_then(Window::toplevel).cloned();
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

    fn shell_surface_output_name(&self, window: &Window) -> Option<String> {
        let id = window
            .wl_surface()
            .and_then(|surface| self.surface_windows.get(&surface.id()))?;
        let title = self
            .windows
            .snapshot()
            .into_iter()
            .find(|entry| entry.id == *id)?
            .title
            .clone();
        shell_surface_output_from_title(&title)
    }

    pub fn relayout_shell_surfaces(&mut self) {
        if self.output_geometry().is_none() {
            return;
        }
        let outputs = self.space.outputs().cloned().collect::<Vec<_>>();
        let output_names = outputs.iter().map(Output::name).collect::<Vec<_>>();
        for desktop in self.desktop_windows.clone() {
            let Some(output_name) = self.shell_surface_output_name(&desktop) else {
                continue;
            };
            let Some(output_index) = output_index_for_shell_surface(&output_name, &output_names)
            else {
                continue;
            };
            let output = &outputs[output_index];
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
            let location = Self::shell_surface_location(&desktop, geometry);
            self.space.map_element(desktop, location, false);
        }
        for panel in self.panel_windows.clone() {
            let Some(output_name) = self.shell_surface_output_name(&panel) else {
                continue;
            };
            let Some(output_index) = output_index_for_shell_surface(&output_name, &output_names)
            else {
                continue;
            };
            let output = &outputs[output_index];
            let Some(output) = self.space.output_geometry(output) else {
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
            let location = Self::shell_surface_location(&panel, geometry);
            self.space.map_element(panel.clone(), location, false);
            self.space.raise_element(&panel, false);
        }
        if self.launcher_visibility.is_visible()
            && let Some(launcher) = self.launcher_window.clone()
        {
            let geometry = self.launcher_geometry(&launcher);
            let location = Self::shell_surface_location(&launcher, geometry);
            self.space.map_element(launcher, location, true);
            self.raise_panels();
        }
        let screenshot_utilities = {
            let registry = self.windows.snapshot();
            self.utility_windows
                .iter()
                .filter(|utility| self.space.elements().any(|mapped| mapped == *utility))
                .filter(|utility| {
                    utility
                        .wl_surface()
                        .and_then(|surface| self.surface_windows.get(&surface.id()))
                        .and_then(|id| registry.iter().find(|entry| entry.id == *id))
                        .and_then(|entry| ShellRole::from_application_id(&entry.app_id))
                        == Some(ShellRole::Screenshot)
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        for utility in screenshot_utilities {
            self.place_screenshot_surface(&utility);
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
        let geometry = maximized_content_geometry(
            work_area,
            self.server_decorated.contains(&surface.wl_surface().id()),
        );
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

    pub(crate) fn reconcile_maximized_toplevel_geometry(
        &mut self,
        surface: &ToplevelSurface,
    ) -> bool {
        if !self
            .maximized_restore
            .contains_key(&surface.wl_surface().id())
        {
            return false;
        }
        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            return false;
        };
        let Some(output) = self.output_geometry_for_window(&window) else {
            return false;
        };
        let geometry = maximized_content_geometry(
            self.work_area_for_output(output),
            self.server_decorated.contains(&surface.wl_surface().id()),
        );
        surface.with_pending_state(|state| {
            state.size = Some(Size::from((geometry.width, geometry.height)));
        });
        self.space
            .map_element(window, (geometry.x, geometry.y), true);
        self.raise_panels();
        self.notify_protocol_snapshot();
        true
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

    pub(crate) fn forget_all_x11_geometry(&mut self) {
        self.x11_maximized_restore.clear();
        self.x11_maximized_restore.shrink_to_fit();
        self.x11_fullscreen_restore.clear();
        self.x11_fullscreen_restore.shrink_to_fit();
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
            let geometry = maximized_content_geometry(
                work_area,
                self.server_decorated.contains(&surface.wl_surface().id()),
            );
            Self::configure_window(&window, geometry);
            self.space
                .map_element(window, (geometry.x, geometry.y), true);
            surface.send_pending_configure();
        }
        let maximized_x11 = self
            .space
            .elements()
            .filter_map(|window| {
                let surface = window.x11_surface()?;
                self.x11_maximized_restore
                    .contains_key(&surface.window_id())
                    .then_some((window.clone(), surface.clone()))
            })
            .collect::<Vec<_>>();
        for (window, surface) in maximized_x11 {
            self.apply_maximized_x11_geometry(&window, &surface, false);
        }
        self.raise_panels();
    }

    pub(crate) fn apply_maximized_x11_geometry(
        &mut self,
        window: &Window,
        surface: &smithay::xwayland::X11Surface,
        preserve_restore: bool,
    ) {
        let Some(output) = self.output_geometry_for_window(window) else {
            return;
        };
        if preserve_restore {
            let location = self.space.element_location(window).unwrap_or_default();
            let size = window.geometry().size;
            self.x11_maximized_restore
                .entry(surface.window_id())
                .or_insert_with(|| {
                    smithay::utils::Rectangle::new(location, (size.w.max(1), size.h.max(1)).into())
                });
        }
        let geometry = maximized_content_geometry(
            self.work_area_for_output(output),
            self.is_server_decorated(window),
        );
        let geometry = smithay::utils::Rectangle::new(
            (geometry.x, geometry.y).into(),
            (geometry.width, geometry.height).into(),
        );
        let _ = surface.configure(geometry);
        self.space.map_element(window.clone(), geometry.loc, true);
    }

    pub(crate) fn restore_maximized_window_for_drag(
        &mut self,
        window: &Window,
        pointer: Point<f64, Logical>,
    ) -> Option<Point<i32, Logical>> {
        let current_location = self.space.element_location(window)?;
        let current_size = window.geometry().size;
        let current = Geometry {
            x: current_location.x,
            y: current_location.y,
            width: current_size.w.max(1),
            height: current_size.h.max(1),
        };
        let output = self.space.outputs().find_map(|output| {
            self.space
                .output_geometry(output)
                .filter(|geometry| geometry.to_f64().contains(pointer))
                .map(|geometry| Geometry {
                    x: geometry.loc.x,
                    y: geometry.loc.y,
                    width: geometry.size.w,
                    height: geometry.size.h,
                })
        })?;
        let decorated = self.is_server_decorated(window);

        if let Some(surface) = window.x11_surface() {
            let restore = self.x11_maximized_restore.remove(&surface.window_id())?;
            let restore = Geometry {
                x: restore.loc.x,
                y: restore.loc.y,
                width: restore.size.w,
                height: restore.size.h,
            };
            let geometry = restored_drag_content_geometry(
                current,
                restore,
                pointer,
                decorated,
                self.work_area_for_output(output),
            );
            let rectangle = Rectangle::new(
                (geometry.x, geometry.y).into(),
                (geometry.width, geometry.height).into(),
            );
            let _ = surface.set_maximized(false);
            let _ = surface.configure(rectangle);
            self.space.map_element(window.clone(), rectangle.loc, true);
            self.notify_protocol_snapshot();
            return Some(rectangle.loc);
        }

        let surface = window.toplevel()?.clone();
        let restore = self.maximized_restore.remove(&surface.wl_surface().id())?;
        let geometry = restored_drag_content_geometry(
            current,
            restore,
            pointer,
            decorated,
            self.work_area_for_output(output),
        );
        tracing::info!(
            surface = ?surface.wl_surface().id(),
            current = ?current,
            restore = ?restore,
            decorated,
            pointer = ?pointer,
            result = ?geometry,
            "diagnostic: restoring maximized Wayland window for drag"
        );
        surface.with_pending_state(|state| {
            state
                .states
                .unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
            state.size = Some((geometry.width, geometry.height).into());
        });
        self.space
            .map_element(window.clone(), (geometry.x, geometry.y), true);
        surface.send_pending_configure();
        self.notify_protocol_snapshot();
        Some((geometry.x, geometry.y).into())
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

    pub(crate) fn window_for_registry_id(&self, id: WindowId) -> Option<Window> {
        self.space
            .elements()
            .find(|window| {
                window
                    .wl_surface()
                    .and_then(|surface| self.surface_windows.get(&surface.id()))
                    .copied()
                    == Some(id)
                    || window
                        .x11_surface()
                        .and_then(|surface| self.x11_windows.get(&surface.window_id()))
                        .copied()
                        == Some(id)
            })
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

    pub(crate) fn output_geometry_named(&self, name: &str) -> Option<Geometry> {
        let output = self.space.outputs().find(|output| output.name() == name)?;
        let geometry = self.space.output_geometry(output)?;
        Some(Geometry {
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w,
            height: geometry.size.h,
        })
    }

    pub(crate) fn preferred_interaction_output_name(&self) -> Option<String> {
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

    fn shell_surface_location(window: &Window, target: Geometry) -> (i32, i32) {
        let surface = window.geometry();
        shell_layout::space_location_for_bounds(
            target,
            Geometry {
                x: surface.loc.x,
                y: surface.loc.y,
                width: surface.size.w,
                height: surface.size.h,
            },
        )
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
                let portal_capture_allowed = nix::sys::socket::getsockopt(
                    &client_stream,
                    nix::sys::socket::sockopt::PeerCredentials,
                )
                .ok()
                .is_some_and(|credentials| {
                    crate::handlers::portal_capture_pid_allowed(credentials.pid())
                });
                state
                    .display_handle
                    .insert_client(
                        client_stream,
                        Arc::new(ClientState {
                            compositor_state: CompositorClientState::default(),
                            portal_capture_allowed,
                        }),
                    )
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

fn maximized_content_geometry(frame: Geometry, server_decorated: bool) -> Geometry {
    if server_decorated {
        Geometry {
            x: frame.x + crate::window_frame::RESIZE_BORDER,
            y: frame.y + crate::window_frame::TITLEBAR_HEIGHT + crate::window_frame::RESIZE_BORDER,
            width: (frame.width - crate::window_frame::RESIZE_BORDER * 2).max(1),
            height: (frame.height
                - crate::window_frame::TITLEBAR_HEIGHT
                - crate::window_frame::RESIZE_BORDER * 2)
                .max(1),
        }
    } else {
        frame
    }
}

fn clamp_decorated_content_to_work_area(content: Geometry, work_area: Geometry) -> Geometry {
    let outer = crate::window_frame::outer_geometry(content);
    let location = clamp_window_location(
        (outer.x, outer.y).into(),
        (outer.width, outer.height).into(),
        work_area,
    );
    Geometry {
        x: location.x + (content.x - outer.x),
        y: location.y + (content.y - outer.y),
        ..content
    }
}

fn restored_drag_content_geometry(
    current_content: Geometry,
    restore_content: Geometry,
    pointer: Point<f64, Logical>,
    server_decorated: bool,
    work_area: Geometry,
) -> Geometry {
    let current_outer = if server_decorated {
        crate::window_frame::outer_geometry(current_content)
    } else {
        current_content
    };
    let restored_outer_size = if server_decorated {
        crate::window_frame::outer_geometry(Geometry {
            x: 0,
            y: 0,
            ..restore_content
        })
    } else {
        Geometry {
            x: 0,
            y: 0,
            ..restore_content
        }
    };
    let horizontal = ((pointer.x - f64::from(current_outer.x))
        / f64::from(current_outer.width.max(1)))
    .clamp(0.0, 1.0);
    let titlebar_offset = (pointer.y - f64::from(current_outer.y))
        .clamp(0.0, f64::from(crate::window_frame::TITLEBAR_HEIGHT.max(1)));
    let minimum_visible = 32.min(restored_outer_size.width.max(1));
    let outer_x = (pointer.x - horizontal * f64::from(restored_outer_size.width)).round() as i32;
    let outer_y = (pointer.y - titlebar_offset).round() as i32;
    let outer_x = outer_x.clamp(
        work_area.x - restored_outer_size.width + minimum_visible,
        work_area.x + work_area.width - minimum_visible,
    );
    let outer_y = outer_y.clamp(
        work_area.y,
        work_area.y + work_area.height - crate::window_frame::TITLEBAR_HEIGHT.max(1),
    );

    if server_decorated {
        Geometry {
            x: outer_x + crate::window_frame::RESIZE_BORDER,
            y: outer_y + crate::window_frame::TITLEBAR_HEIGHT + crate::window_frame::RESIZE_BORDER,
            width: restore_content.width,
            height: restore_content.height,
        }
    } else {
        Geometry {
            x: outer_x,
            y: outer_y,
            ..restore_content
        }
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
    pub portal_capture_allowed: bool,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

#[cfg(test)]
mod protocol_tests {
    use super::{
        DeferredGlobalRetirements, DisplacedWindow, GlobalRetirementAction,
        MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS, OUTPUT_GLOBAL_BIND_SETTLE_GRACE,
        OUTPUT_GLOBAL_DISABLED_GRACE, PREVIEW_BYTE_CAPACITY, PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER,
        PREVIEW_ENTRY_CAPACITY, PREVIEW_FRAME_BYTES, RegisteredShellRole,
        ShellRegistrationRejection, admitted_preview_ids, advance_preview_content_generation,
        bounded_preview_ids, clamp_decorated_content_to_work_area, clamp_window_location,
        command_requires_shell_identity, drag_icon_location, identification_expiry_is_current,
        maximized_content_geometry, output_global_capacity_available,
        output_index_for_shell_surface, preview_mapping_has_exact_size,
        protocol_preview_from_cached, record_preview_capture_attempt,
        restored_drag_content_geometry, retain_live_idle_inhibitors, retire_displaced_window,
        retire_pointer_surface, retire_shell_surface, reuse_preview_pixels,
        shell_registration_rejection, shell_registration_role_changed,
        shell_role_accepts_ordinary_focus, shell_surface_output_from_title,
        test_control_may_invoke,
    };
    use crate::shell_layout::Geometry;
    use nickel_session_protocol::{
        Command, OutputTransform, Query, ServerEnvelope, ServerMessage, SessionAction, ShellRole,
        TestOutput,
    };
    use smithay::{
        output::{Output, PhysicalProperties, Subpixel},
        reexports::{
            calloop::EventLoop,
            wayland_server::{Display, backend::ObjectId},
        },
        utils::Point,
    };
    use std::collections::{HashMap, HashSet};
    use std::time::{Duration, Instant};

    #[test]
    fn pointer_identity_churn_returns_all_collections_to_baseline() {
        let mut hints = HashMap::new();
        let mut locks = HashSet::new();
        let mut origins = HashMap::new();

        for surface in 0_u16..300 {
            hints.insert(surface, Point::from((1.0, 2.0)));
            locks.insert(surface);
            origins.insert(surface, Point::from((3.0, 4.0)));
            let restored = retire_pointer_surface(&mut hints, &mut locks, &mut origins, &surface);
            assert_eq!(restored, Some(Point::from((4.0, 6.0))));
            assert!(hints.is_empty());
            assert!(locks.is_empty());
            assert!(origins.is_empty());
        }

        assert_eq!(hints.capacity(), 0);
        assert_eq!(locks.capacity(), 0);
        assert_eq!(origins.capacity(), 0);
        hints.insert(301, Point::from((5.0, 6.0)));
        assert_eq!(hints.len(), 1, "new hints remain admissible after churn");
    }

    #[test]
    fn closed_displaced_windows_do_not_preserve_output_history() {
        let mut outputs = HashMap::new();
        for index in 0_u64..300 {
            let id = super::WindowId(index + 1);
            outputs.insert(
                format!("virtual-{index}"),
                vec![DisplacedWindow {
                    id,
                    relative_location: Point::from((10, 20)),
                    rescue_location: Point::from((30, 40)),
                }],
            );
            retire_displaced_window(&mut outputs, id);
            assert!(outputs.is_empty());
        }
        assert_eq!(outputs.capacity(), 0);
    }

    #[test]
    fn displaced_mapped_minimized_and_hidden_windows_retire_independently() {
        let mapped = super::WindowId(1);
        let minimized = super::WindowId(2);
        let hidden = super::WindowId(3);
        let displaced = |id| DisplacedWindow {
            id,
            relative_location: Point::from((10, 20)),
            rescue_location: Point::from((30, 40)),
        };
        let mut outputs = HashMap::from([(
            "removed-output".to_owned(),
            vec![displaced(mapped), displaced(minimized), displaced(hidden)],
        )]);

        retire_displaced_window(&mut outputs, mapped);
        assert_eq!(outputs["removed-output"].len(), 2);
        retire_displaced_window(&mut outputs, minimized);
        assert_eq!(outputs["removed-output"].len(), 1);
        retire_displaced_window(&mut outputs, hidden);
        assert!(outputs.is_empty());
        assert_eq!(outputs.capacity(), 0);
    }

    #[test]
    fn destroying_a_shell_surface_retires_only_its_registration() {
        let retired = ObjectId::null();
        let retained = retired.clone();
        let mut registrations = vec![RegisteredShellRole {
            role: ShellRole::Launcher,
            output: None,
            surface: retired.clone(),
        }];
        retire_shell_surface(&mut registrations, &retired);
        assert!(registrations.is_empty());
        assert_eq!(registrations.capacity(), 0);

        // Re-registration after independent destruction is not blocked by a
        // historical singleton slot.
        registrations.push(RegisteredShellRole {
            role: ShellRole::Launcher,
            output: None,
            surface: retained,
        });
        assert_eq!(registrations.len(), 1);
    }

    #[test]
    fn live_surface_role_transitions_invalidate_historical_readiness() {
        let surface = ObjectId::null();
        let registrations = vec![RegisteredShellRole {
            role: ShellRole::Launcher,
            output: None,
            surface: surface.clone(),
        }];

        assert!(!shell_registration_role_changed(
            &registrations,
            &surface,
            Some(ShellRole::Launcher)
        ));
        assert!(shell_registration_role_changed(
            &registrations,
            &surface,
            Some(ShellRole::ControlCenter)
        ));
        assert!(shell_registration_role_changed(
            &registrations,
            &surface,
            None
        ));
    }

    #[test]
    fn only_the_latest_output_identification_generation_may_expire() {
        assert!(identification_expiry_is_current(7, 7));
        assert!(!identification_expiry_is_current(8, 7));
        assert!(!identification_expiry_is_current(7, 8));
    }

    static PREVIEW_SESSION_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn preview_test_session() -> (
        EventLoop<'static, super::NickelSession>,
        super::NickelSession,
    ) {
        let mut event_loop = EventLoop::try_new().unwrap();
        let display = Display::new().unwrap();
        let session = super::NickelSession::new(&mut event_loop, display, true);
        (event_loop, session)
    }

    #[test]
    fn deferred_global_queue_is_bounded_and_advances_through_both_grace_periods() {
        let started = Instant::now();
        let mut retirements = DeferredGlobalRetirements::default();
        for value in 0..MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS {
            assert_eq!(
                retirements.defer(started, format!("output-{value}"), value),
                Ok(())
            );
        }
        assert_eq!(retirements.len(), MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS);
        assert!(!output_global_capacity_available(
            MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS - 1,
            1,
        ));
        assert_eq!(
            retirements.defer(
                started,
                "overflow".into(),
                MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS,
            ),
            Err(MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS)
        );
        assert!(
            retirements
                .advance(started + OUTPUT_GLOBAL_BIND_SETTLE_GRACE - Duration::from_millis(1))
                .is_empty()
        );
        assert_eq!(retirements.len(), MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS);
        assert_eq!(
            retirements.advance(started + OUTPUT_GLOBAL_BIND_SETTLE_GRACE),
            (0..MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS)
                .map(|value| GlobalRetirementAction::Disable {
                    identity: format!("output-{value}"),
                    value,
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(retirements.len(), MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS);
        assert!(
            retirements
                .advance(
                    started + OUTPUT_GLOBAL_BIND_SETTLE_GRACE + OUTPUT_GLOBAL_DISABLED_GRACE
                        - Duration::from_millis(1),
                )
                .is_empty()
        );
        assert_eq!(
            retirements
                .advance(started + OUTPUT_GLOBAL_BIND_SETTLE_GRACE + OUTPUT_GLOBAL_DISABLED_GRACE,),
            (0..MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS)
                .map(|value| GlobalRetirementAction::Remove {
                    identity: format!("output-{value}"),
                    value,
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(retirements.len(), 0);
    }

    #[test]
    fn rapid_unique_global_churn_returns_to_baseline_each_grace_window() {
        let started = Instant::now();
        let mut retirements = DeferredGlobalRetirements::default();
        for generation in 0..64 {
            let cycle = started
                + (OUTPUT_GLOBAL_BIND_SETTLE_GRACE + OUTPUT_GLOBAL_DISABLED_GRACE) * generation;
            for output in 0..MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS {
                retirements
                    .defer(cycle, format!("output-{output}"), (generation, output))
                    .expect("one admitted window of churn fits the bound");
            }
            assert!(!retirements.has_capacity());
            assert_eq!(
                retirements
                    .advance(cycle + OUTPUT_GLOBAL_BIND_SETTLE_GRACE)
                    .len(),
                MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS
            );
            assert_eq!(retirements.len(), MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS);
            assert_eq!(
                retirements
                    .advance(
                        cycle + OUTPUT_GLOBAL_BIND_SETTLE_GRACE + OUTPUT_GLOBAL_DISABLED_GRACE,
                    )
                    .len(),
                MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS
            );
            assert_eq!(retirements.len(), 0);
        }
    }

    #[test]
    fn output_global_settles_before_disable_and_remains_until_final_grace_expires() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let output = Output::new(
            "deferred-test".into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Nickel".into(),
                model: "Deferred test output".into(),
                serial_number: "deferred-test".into(),
            },
        );
        let global = output.create_global::<super::NickelSession>(&session.display_handle);
        let retained = global.clone();
        let started = Instant::now();

        session.defer_output_global_retirement_at("deferred-test".into(), global, started);
        let active = session
            .display_handle
            .backend_handle()
            .global_info(retained.clone())
            .expect("settling global remains advertised");
        assert!(!active.disabled);

        session.reap_output_global_retirements(
            started + OUTPUT_GLOBAL_BIND_SETTLE_GRACE - Duration::from_millis(1),
        );
        assert!(
            !session
                .display_handle
                .backend_handle()
                .global_info(retained.clone())
                .unwrap()
                .disabled
        );
        session.reap_output_global_retirements(started + OUTPUT_GLOBAL_BIND_SETTLE_GRACE);
        assert!(
            session
                .display_handle
                .backend_handle()
                .global_info(retained.clone())
                .expect("disabled global remains bindable during final grace")
                .disabled
        );
        assert!(
            session
                .display_handle
                .backend_handle()
                .global_info(retained.clone())
                .is_ok()
        );
        session.reap_output_global_retirements(
            started + OUTPUT_GLOBAL_BIND_SETTLE_GRACE + OUTPUT_GLOBAL_DISABLED_GRACE,
        );
        assert!(
            session
                .display_handle
                .backend_handle()
                .global_info(retained)
                .is_err()
        );
    }

    #[test]
    fn same_name_reconnect_waits_to_publish_until_the_old_global_is_disabled() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let connect = || TestOutput::Connect {
            name: "same".into(),
            logical_width: 640,
            logical_height: 480,
            scale_120: 120,
            transform: OutputTransform::Normal,
        };

        session.apply_test_output(connect()).unwrap();
        let old = session.virtual_test_outputs["same"].1.clone().unwrap();
        session
            .apply_test_output(TestOutput::Disconnect {
                name: "same".into(),
            })
            .unwrap();
        session.apply_test_output(connect()).unwrap();

        assert!(session.virtual_test_outputs["same"].1.is_none());
        assert!(
            !session
                .display_handle
                .backend_handle()
                .global_info(old.clone())
                .unwrap()
                .disabled
        );

        session.reap_output_global_retirements(Instant::now() + OUTPUT_GLOBAL_BIND_SETTLE_GRACE);
        assert!(
            session
                .display_handle
                .backend_handle()
                .global_info(old)
                .unwrap()
                .disabled
        );
        let replacement = session.virtual_test_outputs["same"].1.clone().unwrap();
        assert!(
            !session
                .display_handle
                .backend_handle()
                .global_info(replacement)
                .unwrap()
                .disabled
        );
    }

    #[test]
    fn rapid_same_name_reconnect_keeps_only_the_latest_live_generation_unpublished() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let connect = || TestOutput::Connect {
            name: "repeat".into(),
            logical_width: 640,
            logical_height: 480,
            scale_120: 120,
            transform: OutputTransform::Normal,
        };

        session.apply_test_output(connect()).unwrap();
        for _ in 0..64 {
            session
                .apply_test_output(TestOutput::Disconnect {
                    name: "repeat".into(),
                })
                .unwrap();
            session.apply_test_output(connect()).unwrap();
            assert!(session.virtual_test_outputs["repeat"].1.is_none());
            assert_eq!(session.pending_output_global_retirements.len(), 1);
        }

        let first_disable = Instant::now() + OUTPUT_GLOBAL_BIND_SETTLE_GRACE;
        session.reap_output_global_retirements(first_disable);
        assert!(session.virtual_test_outputs["repeat"].1.is_some());
        assert_eq!(session.pending_output_global_retirements.len(), 1);

        session
            .apply_test_output(TestOutput::Disconnect {
                name: "repeat".into(),
            })
            .unwrap();
        session.apply_test_output(connect()).unwrap();
        assert!(session.virtual_test_outputs["repeat"].1.is_none());
        assert_eq!(session.pending_output_global_retirements.len(), 2);

        let second_disable = first_disable + OUTPUT_GLOBAL_DISABLED_GRACE;
        session.reap_output_global_retirements(second_disable);
        assert!(session.virtual_test_outputs["repeat"].1.is_some());
        assert_eq!(session.pending_output_global_retirements.len(), 1);
        session.reap_output_global_retirements(second_disable + OUTPUT_GLOBAL_DISABLED_GRACE);
        assert_eq!(session.pending_output_global_retirements.len(), 0);
    }

    #[test]
    fn shutdown_drops_pending_and_unpublished_same_name_generations() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (event_loop, mut session) = preview_test_session();
        let connect = || TestOutput::Connect {
            name: "shutdown".into(),
            logical_width: 640,
            logical_height: 480,
            scale_120: 120,
            transform: OutputTransform::Normal,
        };
        session.apply_test_output(connect()).unwrap();
        session
            .apply_test_output(TestOutput::Disconnect {
                name: "shutdown".into(),
            })
            .unwrap();
        session.apply_test_output(connect()).unwrap();
        assert_eq!(session.pending_output_global_retirements.len(), 1);
        assert!(session.virtual_test_outputs["shutdown"].1.is_none());
        drop(session);
        drop(event_loop);
    }

    #[test]
    fn rapid_virtual_output_churn_applies_backpressure_until_reap() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let connect = |name: String| TestOutput::Connect {
            name,
            logical_width: 640,
            logical_height: 480,
            scale_120: 120,
            transform: OutputTransform::Normal,
        };

        for generation in 0..MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS {
            let name = format!("rapid-{generation}");
            session
                .apply_test_output(connect(name.clone()))
                .expect("churn within the deferred-global bound is admitted");
            session
                .apply_test_output(TestOutput::Disconnect { name })
                .expect("admitted output disconnects into the grace queue");
        }
        assert_eq!(
            session.pending_output_global_retirements.len(),
            MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS
        );
        assert_eq!(
            session.apply_test_output(connect("backpressured".into())),
            Err("output global retirement backlog is full")
        );

        let disable_at = Instant::now() + OUTPUT_GLOBAL_BIND_SETTLE_GRACE;
        session.reap_output_global_retirements(disable_at);
        assert_eq!(
            session.apply_test_output(connect("still-backpressured".into())),
            Err("output global retirement backlog is full")
        );
        session.reap_output_global_retirements(disable_at + OUTPUT_GLOBAL_DISABLED_GRACE);
        session
            .apply_test_output(connect("after-reap".into()))
            .expect("global admission resumes after the grace queue drains");
    }

    #[test]
    fn failed_capture_rolls_the_real_frame_and_allocation_back_into_session() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let id = session
            .windows
            .insert(crate::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.preview_admitted.insert(id);
        session.preview_frames.insert(
            id,
            super::PreviewFrame {
                width: super::PREVIEW_WIDTH as u16,
                height: super::PREVIEW_HEIGHT as u16,
                rgba: vec![41; PREVIEW_FRAME_BYTES],
            },
        );
        let allocation = session.preview_frames[&id].rgba.as_ptr();

        let (rgba, had_frame) = session.take_preview_capture_buffer(id);
        session.preview_capture_failed(id, rgba, had_frame);

        assert_eq!(session.preview_frames[&id].rgba.as_ptr(), allocation);
        assert_eq!(session.preview_frames[&id].rgba[0], 41);
        assert_eq!(session.preview_counters.evictions, 0);
        assert_eq!(session.preview_counters.capture_failures, 1);
    }

    #[test]
    fn fourteen_first_capture_failures_retain_exactly_the_declared_capacity() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let ids = (0..PREVIEW_ENTRY_CAPACITY)
            .map(|_| {
                session
                    .windows
                    .insert(crate::window_registry::WindowAdmission::Ordinary)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        session.preview_admitted.extend(ids.iter().copied());
        for id in ids {
            let (rgba, had_frame) = session.take_preview_capture_buffer(id);
            assert!(!had_frame);
            session.preview_capture_failed(id, rgba, false);
        }

        assert_eq!(session.preview_bytes(), PREVIEW_BYTE_CAPACITY);
        assert_eq!(
            session.preview_counters.peak_bytes,
            PREVIEW_BYTE_CAPACITY as u64
        );
        assert_eq!(session.preview_spares.len(), PREVIEW_ENTRY_CAPACITY);
    }

    #[test]
    fn invalid_mapped_length_leaves_the_capture_lease_untouched() {
        let pixels = vec![23; PREVIEW_FRAME_BYTES];
        let allocation = pixels.as_ptr();
        assert!(!preview_mapping_has_exact_size(&vec![
            0;
            PREVIEW_FRAME_BYTES
                - 1
        ]));
        assert_eq!(pixels.as_ptr(), allocation);
        assert_eq!(pixels[0], 23);
    }

    #[test]
    fn stale_retry_epoch_cannot_consume_new_generation_pending_work() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (mut event_loop, mut session) = preview_test_session();
        let old = session
            .windows
            .insert(crate::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.preview_retry_pending.insert(old);
        session.schedule_preview_retry();
        let stale_epoch = session.preview_retry_scheduled.unwrap();

        session.clear_all_previews();
        let current = session
            .windows
            .insert(crate::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.preview_content_generation.insert(current, 10);
        session.preview_retry_pending.insert(current);
        session.schedule_preview_retry();
        assert_ne!(session.preview_retry_scheduled, Some(stale_epoch));

        event_loop
            .dispatch(std::time::Duration::from_millis(25), &mut session)
            .unwrap();
        assert_eq!(session.preview_content_generation[&current], 11);
        assert!(session.preview_retry_pending.is_empty());
    }

    #[test]
    fn nested_retry_waits_for_capture_cadence_and_fires_only_once() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (mut event_loop, mut session) = preview_test_session();
        let id = session
            .windows
            .insert(crate::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.preview_content_generation.insert(id, 4);
        session.preview_retry_pending.insert(id);
        session.schedule_preview_retry_after(std::time::Duration::from_millis(200));

        event_loop
            .dispatch(std::time::Duration::from_millis(30), &mut session)
            .unwrap();
        assert_eq!(session.preview_content_generation[&id], 4);
        assert_eq!(session.preview_retry_pending.len(), 1);
        assert!(session.preview_retry_scheduled.is_some());

        event_loop
            .dispatch(std::time::Duration::from_millis(220), &mut session)
            .unwrap();
        assert_eq!(session.preview_content_generation[&id], 5);
        assert!(session.preview_retry_pending.is_empty());
        assert!(session.preview_retry_scheduled.is_none());

        event_loop
            .dispatch(std::time::Duration::from_millis(30), &mut session)
            .unwrap();
        assert_eq!(session.preview_content_generation[&id], 5);
        assert!(session.preview_retry_scheduled.is_none());
    }

    #[test]
    fn real_session_reconcile_preserves_seven_frames_for_each_visible_consumer() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let switcher = (0..7)
            .map(|_| {
                session
                    .windows
                    .insert(crate::window_registry::WindowAdmission::Ordinary)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let overlay = (0..7)
            .map(|_| {
                session
                    .windows
                    .insert(crate::window_registry::WindowAdmission::Ordinary)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        session.set_switcher_preview_interest(switcher.clone());
        session.set_overlay_preview_interest(overlay.clone());
        for id in switcher.iter().chain(&overlay).copied() {
            session.preview_frames.insert(
                id,
                super::PreviewFrame {
                    width: super::PREVIEW_WIDTH as u16,
                    height: super::PREVIEW_HEIGHT as u16,
                    rgba: vec![id.0 as u8; PREVIEW_FRAME_BYTES],
                },
            );
        }

        session.clear_switcher_preview_interest();

        assert!(
            overlay
                .iter()
                .all(|id| session.preview_frames.contains_key(id))
        );
        assert_eq!(session.preview_bytes(), 7 * PREVIEW_FRAME_BYTES);
    }

    #[test]
    fn real_preview_query_and_encode_counters_partition_the_aggregate() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let id = session
            .windows
            .insert(crate::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.preview_admitted.insert(id);
        session.preview_frames.insert(
            id,
            super::PreviewFrame {
                width: super::PREVIEW_WIDTH as u16,
                height: super::PREVIEW_HEIGHT as u16,
                rgba: vec![5; PREVIEW_FRAME_BYTES],
            },
        );
        let message = session.handle_protocol_query(Query::Preview {
            window: nickel_session_protocol::WindowId(id.0),
        });
        assert!(matches!(message, ServerMessage::Preview(_)));
        let framed = nickel_session_protocol::encode(&ServerEnvelope {
            request_id: 9,
            message,
        })
        .unwrap();
        session.record_preview_protocol_encoding(
            framed.len() - nickel_session_protocol::FRAME_HEADER_BYTES,
            framed.len(),
        );

        let counters = session.preview_counters;
        assert_eq!(
            counters.protocol_copy_bytes,
            counters.protocol_raw_copy_bytes
                + counters.protocol_base64_bytes
                + counters.protocol_json_payload_bytes
                + counters.protocol_framed_copy_bytes
        );
        assert_eq!(counters.protocol_raw_copy_bytes, PREVIEW_FRAME_BYTES as u64);
    }

    #[test]
    fn real_session_attempt_generation_blocks_other_nodes_until_explicit_retry() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let id = session
            .windows
            .insert(crate::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.preview_content_generation.insert(id, 6);
        let first_node_wave = session.begin_preview_render_wave();
        assert!(record_preview_capture_attempt(
            &mut session.preview_attempted,
            id,
            6,
            first_node_wave
        ));
        let second_node_wave = session.begin_preview_render_wave();
        assert!(!record_preview_capture_attempt(
            &mut session.preview_attempted,
            id,
            6,
            second_node_wave
        ));

        session.preview_renderer_failed(id);
        assert!(session.advance_preview_retry_generation());
        let retry_wave = session.begin_preview_render_wave();
        assert!(record_preview_capture_attempt(
            &mut session.preview_attempted,
            id,
            7,
            retry_wave
        ));
    }

    #[test]
    fn preview_workload_is_bounded_around_the_selected_window() {
        let ids = (0..nickel_session_protocol::MAX_WINDOWS as u64)
            .map(super::WindowId)
            .collect::<Vec<_>>();
        let selected = nickel_session_protocol::MAX_WINDOWS / 2;
        let admitted = bounded_preview_ids(ids, selected);

        assert_eq!(admitted.len(), PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER);
        assert!(admitted.contains(&super::WindowId(selected as u64)));
        assert_eq!(
            PREVIEW_BYTE_CAPACITY,
            PREVIEW_ENTRY_CAPACITY * PREVIEW_FRAME_BYTES
        );
        assert_eq!(PREVIEW_BYTE_CAPACITY, 1_814_400);
    }

    #[test]
    fn preview_workload_clamps_at_both_candidate_edges() {
        let ids = (0..12).map(super::WindowId).collect::<Vec<_>>();
        assert_eq!(bounded_preview_ids(ids.clone(), 0), ids[..7]);
        assert_eq!(bounded_preview_ids(ids.clone(), 11), ids[5..]);
    }

    #[test]
    fn independent_preview_consumers_share_the_budget_without_destroying_interest() {
        let switcher = (1..=7).map(super::WindowId).collect::<Vec<_>>();
        let overlay = (8..=(nickel_session_protocol::MAX_WINDOWS as u64 + 8))
            .map(super::WindowId)
            .collect::<Vec<_>>();

        let overlapping = admitted_preview_ids(&switcher, &overlay);
        assert_eq!(overlapping.len(), PREVIEW_ENTRY_CAPACITY);
        assert!(switcher.iter().all(|id| overlapping.contains(id)));
        let mut frames = overlapping
            .iter()
            .map(|id| {
                (
                    *id,
                    super::PreviewFrame {
                        width: super::PREVIEW_WIDTH as u16,
                        height: super::PREVIEW_HEIGHT as u16,
                        rgba: vec![id.0 as u8; PREVIEW_FRAME_BYTES],
                    },
                )
            })
            .collect::<HashMap<_, _>>();

        let after_overlay_dismissal = admitted_preview_ids(&switcher, &[]);
        assert_eq!(after_overlay_dismissal, switcher.iter().copied().collect());
        let after_switcher_dismissal = admitted_preview_ids(&[], &overlay);
        assert_eq!(after_switcher_dismissal.len(), PREVIEW_ENTRY_CAPACITY);
        frames.retain(|id, _| after_switcher_dismissal.contains(id));
        assert_eq!(frames.len(), PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER);
        assert!(
            overlay[..PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER]
                .iter()
                .all(|id| protocol_preview_from_cached(
                    nickel_session_protocol::WindowId(id.0),
                    frames.get(id)
                )
                .is_some())
        );
    }

    #[test]
    fn failed_capture_is_attempted_once_per_generation_across_outputs_and_nodes() {
        let id = super::WindowId(7);
        let mut attempted = HashMap::new();
        assert!(record_preview_capture_attempt(&mut attempted, id, 3, 11));
        for _ in 0..3 {
            assert!(!record_preview_capture_attempt(&mut attempted, id, 3, 11));
        }
        assert!(!record_preview_capture_attempt(&mut attempted, id, 3, 12));
        assert!(record_preview_capture_attempt(&mut attempted, id, 4, 12));
    }

    #[test]
    fn preview_churn_never_admits_more_than_the_declared_process_ceiling() {
        for generation in 0..(nickel_session_protocol::MAX_WINDOWS as u64 * 2) {
            let switcher = (generation..generation + 7)
                .map(super::WindowId)
                .collect::<Vec<_>>();
            let overlay = (generation + 7..generation + 1_031)
                .map(super::WindowId)
                .collect::<Vec<_>>();
            let admitted = admitted_preview_ids(&switcher, &overlay);
            assert!(admitted.len() <= PREVIEW_ENTRY_CAPACITY);
            assert!(admitted.len() * PREVIEW_FRAME_BYTES <= PREVIEW_BYTE_CAPACITY);
        }
    }

    #[test]
    fn damage_advances_only_the_authoritative_window_content_generation() {
        let damaged = super::WindowId(1);
        let unchanged = super::WindowId(2);
        let mut generations = HashMap::from([(damaged, 3), (unchanged, 8)]);
        let mut attempted = HashMap::from([(damaged, (3, 4)), (unchanged, (8, 4))]);

        assert_eq!(
            advance_preview_content_generation(&mut generations, &mut attempted, damaged),
            4
        );
        assert_eq!(generations[&unchanged], 8);
        assert!(!attempted.contains_key(&damaged));
        assert_eq!(attempted[&unchanged], (8, 4));
    }

    #[test]
    fn not_ready_query_has_no_interest_side_effect() {
        let switcher = vec![super::WindowId(1)];
        let overlay = vec![super::WindowId(2)];
        let before = admitted_preview_ids(&switcher, &overlay);
        assert!(
            protocol_preview_from_cached(nickel_session_protocol::WindowId(99), None).is_none()
        );
        assert_eq!(admitted_preview_ids(&switcher, &overlay), before);
    }

    #[test]
    fn replacement_reuses_the_retired_frame_allocation() {
        let pixels = vec![7; PREVIEW_FRAME_BYTES];
        let allocation = pixels.as_ptr();
        let replacement = reuse_preview_pixels(pixels, &vec![9; PREVIEW_FRAME_BYTES]);
        assert_eq!(replacement.as_ptr(), allocation);
        assert_eq!(replacement.len(), PREVIEW_FRAME_BYTES);
        assert!(replacement.iter().all(|pixel| *pixel == 9));
    }

    #[test]
    fn only_the_exact_supervised_shell_pid_can_register() {
        let current = std::process::id();
        assert_eq!(
            shell_registration_rejection(current, current, current + 1, true),
            Some(ShellRegistrationRejection::ClaimedPeerMismatch)
        );
        assert_eq!(
            shell_registration_rejection(0, current, current, true),
            Some(ShellRegistrationRejection::NoActiveGeneration)
        );
        assert_eq!(
            shell_registration_rejection(current + 1, current, current, true),
            Some(ShellRegistrationRejection::OutsideActiveGeneration)
        );
        assert_eq!(
            shell_registration_rejection(current, current, current, false),
            Some(ShellRegistrationRejection::OutsideSessionUser)
        );
        assert_eq!(
            shell_registration_rejection(current, current, current, true),
            None
        );
    }

    #[test]
    fn privileged_shell_commands_require_the_registered_shell_pid() {
        for command in [
            Command::LogOut,
            Command::Unlock,
            Command::SessionAction {
                action: SessionAction::Lock,
            },
            Command::SessionAction {
                action: SessionAction::PowerOff,
            },
            Command::FocusShellRole {
                role: ShellRole::ControlCenter,
            },
            Command::RestoreApplicationFocus,
        ] {
            assert!(command_requires_shell_identity(&command));
        }
        assert!(!command_requires_shell_identity(&Command::ToggleLauncher));
    }

    #[test]
    fn ordinary_shell_focus_includes_the_interactive_screenshot_overlay() {
        for role in [
            ShellRole::ControlCenter,
            ShellRole::ProjectMenu,
            ShellRole::Preview,
            ShellRole::ContextMenu,
            ShellRole::Screenshot,
        ] {
            assert!(shell_role_accepts_ordinary_focus(role));
        }
        for role in [
            ShellRole::Desktop,
            ShellRole::Panel,
            ShellRole::Launcher,
            ShellRole::Lock,
            ShellRole::Notification,
        ] {
            assert!(!shell_role_accepts_ordinary_focus(role));
        }
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
    fn disconnected_surfaces_stop_inhibiting_idle_policy() {
        let mut inhibitors = HashMap::from([("alive", 2), ("disconnected", 1)]);
        retain_live_idle_inhibitors(&mut inhibitors, |surface| *surface == "alive");
        assert_eq!(inhibitors, HashMap::from([("alive", 2)]));
    }

    #[test]
    fn shell_surface_output_identity_survives_reversed_registration_order() {
        let outputs = vec!["DP-2".into(), "DP-1".into()];
        let registered_surfaces = [
            "Nickel Panel [output=DP-1]",
            "Nickel Desktop [output=DP-1]",
            "Nickel Panel [output=DP-2]",
            "Nickel Desktop [output=DP-2]",
        ];
        let names = registered_surfaces
            .iter()
            .map(|title| shell_surface_output_from_title(title).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(
            names
                .iter()
                .map(|name| output_index_for_shell_surface(name, &outputs))
                .collect::<Vec<_>>(),
            [Some(1), Some(1), Some(0), Some(0)]
        );
    }

    #[test]
    fn descriptive_sdl_output_names_resolve_to_authoritative_connector_names() {
        let outputs = vec!["DVI-I-1".into(), "DP-3".into()];

        assert_eq!(
            output_index_for_shell_surface("Unknown - Odyssey G40B - DP-3", &outputs),
            Some(1)
        );
        assert_eq!(
            output_index_for_shell_surface("Unknown - MB16A - DVI-I-1", &outputs),
            Some(0)
        );
    }

    #[test]
    fn descriptive_output_matching_rejects_missing_or_ambiguous_connectors() {
        assert_eq!(
            output_index_for_shell_surface("Unknown - DisplayPort-1", &["DP-1".into()]),
            None
        );
        assert_eq!(
            output_index_for_shell_surface(
                "Unknown - Display - DP-1",
                &["DP-1".into(), "Display - DP-1".into()]
            ),
            None
        );
    }

    #[test]
    fn shell_surface_output_metadata_rejects_ambiguous_titles() {
        assert_eq!(shell_surface_output_from_title("Nickel Panel"), None);
        assert_eq!(
            shell_surface_output_from_title("Nickel Panel [output=]"),
            None
        );
        assert_eq!(
            shell_surface_output_from_title("Nickel Panel [output=DP-1]extra"),
            None
        );
        assert_eq!(
            shell_surface_output_from_title("Nickel Panel [output=DP-1]"),
            Some("DP-1".into())
        );
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

    #[test]
    fn maximized_server_frame_exactly_fits_the_work_area() {
        let work_area = Geometry {
            x: -1920,
            y: 30,
            width: 1920,
            height: 1010,
        };

        let content = maximized_content_geometry(work_area, true);

        assert_eq!(crate::window_frame::outer_geometry(content), work_area);
    }

    #[test]
    fn initial_managed_x11_content_keeps_its_frame_inside_the_work_area() {
        let work_area = Geometry {
            x: 0,
            y: 0,
            width: 1920,
            height: 1024,
        };
        let content = clamp_decorated_content_to_work_area(
            Geometry {
                x: 0,
                y: 0,
                width: 1200,
                height: 800,
            },
            work_area,
        );

        let outer = crate::window_frame::outer_geometry(content);
        assert_eq!(outer.x, work_area.x);
        assert_eq!(outer.y, work_area.y);
        assert_eq!(content.width, 1200);
        assert_eq!(content.height, 800);
    }

    #[test]
    fn maximized_client_decorated_window_receives_the_whole_work_area() {
        let work_area = Geometry {
            x: 1920,
            y: -200,
            width: 1280,
            height: 700,
        };

        assert_eq!(maximized_content_geometry(work_area, false), work_area);
    }

    #[test]
    fn maximized_content_geometry_clamps_undersized_work_areas() {
        let work_area = Geometry {
            x: 7,
            y: 11,
            width: 1,
            height: 1,
        };

        let content = maximized_content_geometry(work_area, true);

        assert_eq!(content.width, 1);
        assert_eq!(content.height, 1);
    }

    #[test]
    fn restored_drag_preserves_horizontal_pointer_proportion() {
        let current = maximized_content_geometry(
            Geometry {
                x: 0,
                y: 0,
                width: 1200,
                height: 700,
            },
            true,
        );
        let restore = Geometry {
            x: 80,
            y: 90,
            width: 600,
            height: 400,
        };
        let work_area = Geometry {
            x: 0,
            y: 0,
            width: 1200,
            height: 700,
        };

        let left = restored_drag_content_geometry(
            current,
            restore,
            Point::from((120.0, 18.0)),
            true,
            work_area,
        );
        let center = restored_drag_content_geometry(
            current,
            restore,
            Point::from((600.0, 18.0)),
            true,
            work_area,
        );
        let right = restored_drag_content_geometry(
            current,
            restore,
            Point::from((1080.0, 18.0)),
            true,
            work_area,
        );

        assert!(left.x < center.x);
        assert!(center.x < right.x);
        assert_eq!(left.width, restore.width);
        assert_eq!(center.height, restore.height);
    }

    #[test]
    fn restored_drag_keeps_titlebar_reachable_on_negative_output() {
        let work_area = Geometry {
            x: -1920,
            y: -200,
            width: 1920,
            height: 1000,
        };
        let geometry = restored_drag_content_geometry(
            maximized_content_geometry(work_area, true),
            Geometry {
                x: 10,
                y: 10,
                width: 900,
                height: 700,
            },
            Point::from((-1910.0, -195.0)),
            true,
            work_area,
        );
        let outer = crate::window_frame::outer_geometry(geometry);

        assert!(outer.x + outer.width >= work_area.x + 32);
        assert!(outer.y >= work_area.y);
        assert!(outer.y < work_area.y + work_area.height);
    }

    #[test]
    fn drag_icon_uses_pointer_output_and_output_local_coordinates() {
        let left = smithay::utils::Rectangle::new((0, 0).into(), (1920, 1080).into());
        let right = smithay::utils::Rectangle::new((1920, 0).into(), (1920, 1080).into());
        let pointer = smithay::utils::Point::from((2012.4, 84.6));

        assert_eq!(drag_icon_location(pointer, left), None);
        assert_eq!(drag_icon_location(pointer, right), Some((92, 85).into()));
    }
}
