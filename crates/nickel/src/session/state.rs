use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    hash::Hash,
    os::fd::AsFd,
    os::fd::AsRawFd,
    os::unix::net::UnixDatagram,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

use nickel_core::{
    active_output::{
        ActiveOutputContext, InvocationSource, resolve_active_output, resolve_new_window_output,
    },
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
    ShellBehaviorSetting, ShellBehaviorSnapshot, ShellBehaviorTransaction, ShellBehaviorValue,
    ShellPopoverAnchor, ShellRole, ShellSurfaceIdentity, ShellSurfaceSnapshot,
    Snapshot as SessionSnapshot, TestOutput, WindowAction as ProtocolWindowAction,
    WindowId as ProtocolWindowId, WindowSnapshot, WorkspaceId as ProtocolWorkspaceId,
    WorkspaceSnapshot, WorkspaceState, decode, encode,
};
use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType, find_popup_root_surface},
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
        compositor::{
            CompositorClientState, CompositorState, get_parent, send_surface_state, with_states,
        },
        fractional_scale::{FractionalScaleManagerState, with_fractional_scale},
        idle_inhibit::IdleInhibitManagerState,
        image_capture_source::{ImageCaptureSourceState, OutputCaptureSourceState},
        image_copy_capture::{ImageCopyCaptureState, Session},
        input_method::InputMethodManagerState,
        output::OutputManagerState,
        pointer_constraints::PointerConstraintsState,
        relative_pointer::RelativePointerManagerState,
        seat::WaylandFocus,
        selection::{data_device::DataDeviceState, primary_selection::PrimarySelectionState},
        shell::xdg::{
            ToplevelSurface, XdgShellState, decoration::XdgDecorationState, dialog::XdgDialogState,
        },
        shm::ShmState,
        socket::ListeningSocketSource,
        viewporter::ViewporterState,
        xdg_activation::XdgActivationState,
        xwayland_shell::XWaylandShellState,
    },
    xwayland::{X11Wm, xwm::XwmId},
};

/// Nickel-owned overlay color used to dim shell content without painting a
/// pure-black translucent background.
pub(crate) const fn shell_scrim(alpha: f32) -> [f32; 4] {
    [0.035, 0.043, 0.055, alpha]
}

#[cfg(test)]
mod internal_shell_placement_tests {
    use super::{
        internal_codex_chat_placement, internal_codex_project_menu_placement,
        internal_shell_surface_placement,
    };
    use crate::{internal_shell::InternalOutput, winit_shell::SurfaceRole};
    use nickel_session_protocol::{AnchorSide, Geometry, ShellPopoverAnchor};

    fn outputs() -> Vec<(InternalOutput, i32, i32)> {
        vec![
            (
                InternalOutput {
                    name: "left".into(),
                    width: 1920,
                    height: 1080,
                    scale: 1.0,
                },
                -1920,
                -120,
            ),
            (
                InternalOutput {
                    name: "right".into(),
                    width: 2560,
                    height: 1440,
                    scale: 1.0,
                },
                0,
                240,
            ),
        ]
    }

    #[test]
    fn launcher_uses_active_output_global_origin() {
        let placement = internal_shell_surface_placement(
            SurfaceRole::Launcher,
            None,
            (960, 720),
            &outputs(),
            Some("right"),
        );

        assert_eq!(placement.output.as_deref(), Some("right"));
        assert_eq!(placement.geometry, (18, 896, 960, 720));
    }

    #[test]
    fn switching_active_output_relocates_one_launcher_to_negative_origin() {
        let right = internal_shell_surface_placement(
            SurfaceRole::Launcher,
            None,
            (960, 720),
            &outputs(),
            Some("right"),
        );
        let left = internal_shell_surface_placement(
            SurfaceRole::Launcher,
            None,
            (960, 720),
            &outputs(),
            Some("left"),
        );

        assert_eq!(right.output.as_deref(), Some("right"));
        assert_eq!(left.output.as_deref(), Some("left"));
        assert_eq!(left.geometry, (-1902, 176, 960, 720));
        assert_ne!(right.geometry, left.geometry);
    }

    #[test]
    fn codex_menu_uses_clicked_panel_output_global_coordinates() {
        let anchor = ShellPopoverAnchor {
            control: "panel-codex".into(),
            output: "right".into(),
            bounds: Geometry {
                x: 2200,
                y: 1392,
                width: 48,
                height: 48,
            },
            preferred: AnchorSide::Above,
        };

        let placement =
            internal_codex_project_menu_placement(Some(&anchor), &outputs(), Some("left"));

        assert_eq!(placement.output.as_deref(), Some("right"));
        assert_eq!(placement.origin, (1964, 936));
        assert_eq!(placement.scale, 1.0);
    }

    #[test]
    fn codex_menu_fallback_includes_negative_output_origin() {
        let placement = internal_codex_project_menu_placement(None, &outputs(), Some("left"));

        assert_eq!(placement.output.as_deref(), Some("left"));
        assert_eq!(placement.origin, (-1920, 216));
    }

    #[test]
    fn codex_chat_frame_is_centered_inside_nonzero_output_work_area() {
        let placement = internal_codex_chat_placement(&outputs(), Some("right"));

        assert_eq!(placement.output.as_deref(), Some("right"));
        assert_eq!(placement.origin, (720, 572));
        assert_eq!(placement.scale, 1.0);
        let outer =
            crate::session::window_frame::outer_geometry(crate::session::shell_layout::Geometry {
                x: placement.origin.0,
                y: placement.origin.1,
                width: crate::internal_codex::CHAT_SIZE.0 as i32,
                height: crate::internal_codex::CHAT_SIZE.1 as i32,
            });
        assert!(outer.x >= 0);
        assert!(outer.y >= 240);
        assert!(outer.x + outer.width <= 2560);
        assert!(outer.y + outer.height <= 240 + 1440 - crate::winit_shell::PANEL_HEIGHT as i32);
    }

    #[test]
    fn codex_chat_frame_preserves_negative_output_origin() {
        let placement = internal_codex_chat_placement(&outputs(), Some("left"));

        assert_eq!(placement.output.as_deref(), Some("left"));
        assert_eq!(placement.origin, (-1520, 32));
        let outer =
            crate::session::window_frame::outer_geometry(crate::session::shell_layout::Geometry {
                x: placement.origin.0,
                y: placement.origin.1,
                width: crate::internal_codex::CHAT_SIZE.0 as i32,
                height: crate::internal_codex::CHAT_SIZE.1 as i32,
            });
        assert!(outer.x >= -1920);
        assert!(outer.y >= -120);
        assert!(outer.x + outer.width <= 0);
        assert!(outer.y + outer.height <= -120 + 1080 - crate::winit_shell::PANEL_HEIGHT as i32);
    }
}

use crate::session::{
    output_retirement::{DeferredRetirements, RetirementAction, capacity_available},
    shell_layout::{self, Geometry},
    window_registry::{WindowId, WindowRegistry},
};

fn stable_output_identity(output: &Output) -> String {
    let physical = output.physical_properties();
    let hardware = format!(
        "{}|{}|{}|{}x{}",
        physical.make, physical.model, physical.serial_number, physical.size.w, physical.size.h
    );
    if physical.serial_number.is_empty() {
        format!("{hardware}|{}", output.name())
    } else {
        hardware
    }
}

fn protocol_error(code: ErrorCode, message: impl Into<String>) -> ServerMessage {
    ServerMessage::Error {
        code,
        message: message.into(),
    }
}

fn shell_behavior_value(
    settings: &ShellSettings,
    setting: ShellBehaviorSetting,
) -> ShellBehaviorValue {
    match setting {
        ShellBehaviorSetting::BarDisplayScope => {
            ShellBehaviorValue::Toggle(settings.bar_on_all_displays)
        }
        ShellBehaviorSetting::BarWindowScope => {
            ShellBehaviorValue::Toggle(settings.all_windows_on_every_bar)
        }
        ShellBehaviorSetting::DesktopCount => ShellBehaviorValue::Count(settings.desktop_count),
    }
}

fn apply_shell_behavior_value(
    settings: &mut ShellSettings,
    setting: ShellBehaviorSetting,
    value: ShellBehaviorValue,
) -> Result<(), &'static str> {
    match (setting, value) {
        (ShellBehaviorSetting::BarDisplayScope, ShellBehaviorValue::Toggle(value)) => {
            settings.bar_on_all_displays = value;
        }
        (ShellBehaviorSetting::BarWindowScope, ShellBehaviorValue::Toggle(value)) => {
            settings.all_windows_on_every_bar = value;
        }
        (ShellBehaviorSetting::DesktopCount, ShellBehaviorValue::Count(value))
            if (1..=nickel_core::shell_settings::MAX_CONFIGURED_WORKSPACES).contains(&value) =>
        {
            settings.desktop_count = value;
        }
        (ShellBehaviorSetting::DesktopCount, ShellBehaviorValue::Count(_)) => {
            return Err("desktop count is outside the supported range");
        }
        _ => return Err("setting and value types do not match"),
    }
    Ok(())
}

fn prepare_shell_behavior_update(
    current: &ShellSettings,
    topology_generation: u64,
    transaction: &ShellBehaviorTransaction,
) -> Result<ShellSettings, &'static str> {
    if transaction.topology_generation != topology_generation {
        return Err("stale output topology generation");
    }
    if transaction.prior != shell_behavior_value(current, transaction.setting) {
        return Err("shell setting changed before this transaction was applied");
    }
    let mut requested = current.clone();
    apply_shell_behavior_value(&mut requested, transaction.setting, transaction.requested)?;
    Ok(requested)
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

fn output_contains_logical_point(output: Rectangle<i32, Logical>, x: i32, y: i32) -> bool {
    output.contains(Point::from((x, y)))
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

#[derive(Clone, Debug)]
struct PendingLaunchObservation {
    generation: u64,
    root_pid: u32,
    root_start_time: u64,
    registered_at: Instant,
    deadline: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingLaunchWindowDisposition {
    AwaitExpiry,
    Unrelated,
    Attributed { descendant: bool },
}

fn pending_launch_window_disposition(
    pending: &PendingLaunchObservation,
    now: Instant,
    client_pid: u32,
) -> PendingLaunchWindowDisposition {
    if now.saturating_duration_since(pending.registered_at) > pending.deadline {
        return PendingLaunchWindowDisposition::AwaitExpiry;
    }
    if process_descends_from(client_pid, pending.root_pid, pending.root_start_time) {
        PendingLaunchWindowDisposition::Attributed {
            descendant: client_pid != pending.root_pid,
        }
    } else {
        PendingLaunchWindowDisposition::Unrelated
    }
}

fn linux_process_parent(pid: u32) -> Option<u32> {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("PPid:\t"))?
        .trim()
        .parse()
        .ok()
}

fn linux_process_start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

fn process_descends_from(mut pid: u32, root_pid: u32, root_start_time: u64) -> bool {
    for _ in 0..64 {
        if pid == root_pid {
            return linux_process_start_time(pid) == Some(root_start_time);
        }
        let Some(parent) = linux_process_parent(pid) else {
            return false;
        };
        if parent == 0 || parent == pid {
            return false;
        }
        pid = parent;
    }
    false
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
            | SessionCommand::ObservePendingLaunch { .. }
            | SessionCommand::CancelPendingLaunch { .. }
            | SessionCommand::Unlock
            | SessionCommand::SessionAction { .. }
            | SessionCommand::FocusShellRole { .. }
            | SessionCommand::RestoreApplicationFocus
            | SessionCommand::ConfigureOnScreenKeyboard { .. }
            | SessionCommand::OnScreenKeyboardInput { .. }
            | SessionCommand::RegisterShellSurface { .. }
    )
}

fn test_control_may_invoke(command: &SessionCommand) -> bool {
    matches!(
        command,
        SessionCommand::LogOut
            | SessionCommand::Unlock
            | SessionCommand::SessionAction {
                action: nickel_session_protocol::SessionAction::Lock
            }
    )
}

fn production_control_token() -> String {
    use std::io::Read;

    let mut bytes = [0_u8; 32];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .is_err()
    {
        let fallback = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        bytes[..16].copy_from_slice(&fallback.to_ne_bytes());
        bytes[16..24].copy_from_slice(&u64::from(std::process::id()).to_ne_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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
            | ShellRole::VolumeOsd
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InternalShellTimerCounters {
    pub armed: u64,
    pub cancelled: u64,
    pub fired: u64,
    pub polls: u64,
    pub redraw_requests: u64,
}

#[derive(Debug, Default)]
struct InternalShellTimer {
    deadline: Option<Instant>,
    token: Option<smithay::reexports::calloop::RegistrationToken>,
    generation: u64,
    counters: InternalShellTimerCounters,
}

struct CompatibilityControlState {
    protocol_token: String,
    authenticated_shell_pids: HashSet<u32>,
    expected_shell_pid: u32,
    socket_path: PathBuf,
}

#[derive(Debug)]
pub(crate) enum InternalCaptureState {
    Idle,
    Pending(PathBuf),
    Complete(PathBuf, nickel_session_protocol::CaptureResult),
}

pub struct NickelSession {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub event_loop_handle: smithay::reexports::calloop::LoopHandle<'static, NickelSession>,
    #[cfg(feature = "backend-udev")]
    pub native: Option<crate::session::backend::udev::UdevData>,

    pub space: Space<Window>,
    /// UI applications hosted directly by the compositor, without a Wayland client.
    pub internal_ui: crate::session::InternalUiRuntime,
    /// Built-in shell state when no supervised shell client is requested.
    pub(crate) internal_shell: Option<crate::internal_shell::InternalShellCoordinator>,
    /// Codex menu/chat applications hosted in `internal_ui` on Linux.
    pub(crate) internal_codex: Option<crate::internal_codex::InternalCodexHost>,
    pub(crate) internal_shell_surfaces:
        HashMap<nickel_ui::InternalSurfaceId, nickel_ui::InternalSurfaceId>,
    internal_file_surfaces: HashMap<nickel_ui::InternalSurfaceId, nickel_ui::InternalSurfaceId>,
    internal_shell_timer: InternalShellTimer,
    pub loop_signal: LoopSignal,

    // Smithay State
    pub compositor_state: CompositorState,
    pub fractional_scale_manager_state: FractionalScaleManagerState,
    // Fractional-scale clients require wp_viewporter to submit buffers at the
    // advertised non-integer scale. Without it they fall back to wl_output's
    // ceil-rounded integer scale (125% therefore rendered as 200%).
    pub viewporter_state: ViewporterState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_dialog_state: XdgDialogState,
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
    pub(crate) pending_image_copy_frames: Vec<crate::session::handlers::PendingImageCopyFrame>,
    pub xwm: Option<(XwmId, X11Wm)>,
    pub xwayland_restart_pending: bool,
    pub xwayland_display: Option<u32>,
    pub xwayland_registration: Option<smithay::reexports::calloop::RegistrationToken>,
    pub popups: PopupManager,

    pub seat: Seat<Self>,
    pub(crate) on_screen_keyboard: crate::session::on_screen_keyboard::OnScreenKeyboardState,
    pub windows: WindowRegistry,
    pub surface_windows: HashMap<ObjectId, WindowId>,
    surface_effective_outputs: HashMap<ObjectId, String>,
    /// Live XDG protocol roles outlive their mapped compositor representation.
    pub(crate) xdg_toplevel_windows: HashMap<ObjectId, Window>,
    pub(crate) mapped_xdg_toplevels: HashSet<ObjectId>,
    pub(crate) restored_xdg_toplevels: HashSet<ObjectId>,
    pub(crate) xdg_toplevel_locations: HashMap<ObjectId, Point<i32, Logical>>,
    pub(crate) shell_owned_windows: HashSet<WindowId>,
    pub x11_windows: HashMap<u32, WindowId>,
    pub launcher_window: Option<Window>,
    pub launcher_visibility: LauncherVisibility,
    launcher_output_name: Option<String>,
    last_interaction_output_name: Option<String>,
    launcher_focus: FocusTransactions<ObjectId>,
    launcher_restore_window: Option<WindowId>,
    launcher_subscribers: Vec<PathBuf>,
    pending_launch_observations: Vec<PendingLaunchObservation>,
    /// Legacy datagram compatibility is absent from normal compositor-owned
    /// sessions. It exists only when an explicit external-control mode asks
    /// for it.
    compatibility_control: Option<CompatibilityControlState>,
    shell_surface_identities: HashMap<String, ShellSurfaceIdentity>,
    registered_shell_role_slots: Vec<RegisteredShellRole>,
    last_logged_shell_readiness: Option<nickel_session_protocol::ShellReadinessSnapshot>,
    test_control_enabled: bool,
    #[cfg(target_os = "linux")]
    pub(crate) test_controller: Option<crate::session::test_input::TestController>,
    pub launcher_show_requested_at: Option<std::time::Instant>,
    pub desktop_windows: Vec<Window>,
    pub panel_windows: Vec<Window>,
    pub lock_windows: Vec<Window>,
    pub locked: bool,
    pub(crate) held_consumer_controls: HashSet<nickel_session_protocol::ConsumerControl>,
    pub(crate) consumer_repeat_epoch: u64,
    lock_restore_window: Option<WindowId>,
    shell_focus_restore_window: Option<WindowId>,
    pub(crate) pending_shell_focus_role: Option<ShellRole>,
    pub utility_windows: Vec<Window>,
    hidden_shell_roles: HashSet<ShellRole>,
    hidden_shell_role_locations: HashMap<ShellRole, Point<i32, Logical>>,
    screenshot_output_name: Option<String>,
    pub context_menu_window: Option<Window>,
    pub preview_window: Option<Window>,
    pub server_decorated: HashSet<ObjectId>,
    pub primary_output_name: Option<String>,
    output_topology_generation: u64,
    last_protocol_outputs: Vec<OutputSnapshot>,
    output_scale_preferences: nickel_core::dpi::PersistedOutputScales,
    virtual_test_outputs: HashMap<String, (Output, Option<GlobalId>)>,
    pending_output_global_retirements: DeferredRetirements<GlobalId>,
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
    shortcut_desktop_windows: Vec<WindowId>,
    shortcut_desktop_focus: Option<WindowId>,
    shortcut_snap_restore: HashMap<WindowId, smithay::utils::Rectangle<i32, Logical>>,
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
    pub frame_cursor: crate::session::window_frame::FrameCursor,
    pub buffer_commit_tx: Option<smithay::reexports::calloop::channel::Sender<SurfaceBufferCommit>>,
    pub identify_outputs_until: Option<std::time::Instant>,
    identify_outputs_generation: u64,
    pub output_capture_path: Option<PathBuf>,
    pub output_capture_name: Option<String>,
    pub output_capture_reply_path: Option<PathBuf>,
    pub output_capture_request_id: Option<u64>,
    pub(crate) internal_capture: Arc<std::sync::Mutex<InternalCaptureState>>,
    pub(crate) internal_projection_outputs: Arc<std::sync::RwLock<Vec<OutputSnapshot>>>,
    pub shell_failure_count: u8,
    pub(crate) recovery_ui: crate::session::recovery_ui::RecoveryUi,
    secure_storage_state: Arc<AtomicU8>,
    secure_storage_retry: Arc<std::sync::atomic::AtomicBool>,
    deferred_focus_restore: channel::Sender<WindowId>,
    #[cfg(feature = "backend-winit")]
    winit_redraw_window: Option<*const dyn smithay::reexports::winit::window::Window>,
}

mod control_protocol;
mod preview;

#[allow(unused_imports)]
pub use preview::{
    PREVIEW_BYTE_CAPACITY, PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER, PREVIEW_ENTRY_CAPACITY,
    PREVIEW_FRAME_BYTES, PREVIEW_HEIGHT, PREVIEW_WIDTH, PreviewFrame,
};
pub(crate) use preview::{
    PreviewCacheCounters, bounded_preview_ids, preview_capture_dimensions,
    preview_mapping_has_exact_size, protocol_preview_from_cached, reuse_preview_pixels,
};
#[cfg(test)]
use preview::{
    admitted_preview_ids, advance_preview_content_generation, record_preview_capture_attempt,
};
impl NickelSession {
    pub(crate) fn enable_internal_shell(
        &mut self,
        host: std::sync::Arc<dyn crate::session_host::SessionHost>,
    ) -> Result<(), String> {
        self.enable_internal_shell_with_system_updates(
            host,
            crate::platform::system_status_receiver(),
        )
    }

    fn enable_internal_shell_with_system_updates(
        &mut self,
        host: std::sync::Arc<dyn crate::session_host::SessionHost>,
        platform_updates: std::sync::mpsc::Receiver<crate::platform::SystemStatusUpdate>,
    ) -> Result<(), String> {
        use crate::{internal_shell::InternalShellCoordinator, winit_shell::PanelEdge};

        let mut shell = InternalShellCoordinator::new(host, PanelEdge::Bottom)?;
        // Apply updates that were already available without delaying shell
        // construction. Later transitions remain calloop-driven.
        for update in platform_updates.try_iter() {
            let _ = shell.apply_system_status_update(update);
        }
        let (platform_update_tx, platform_update_rx) =
            smithay::reexports::calloop::channel::channel();
        std::thread::Builder::new()
            .name("nickel-internal-system-feed".into())
            .spawn(move || {
                while let Ok(update) = platform_updates.recv() {
                    if platform_update_tx.send(update).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| format!("could not start internal system feed: {error}"))?;
        self.event_loop_handle
            .insert_source(platform_update_rx, |event, _, state| {
                if let smithay::reexports::calloop::channel::Event::Msg(update) = event {
                    let changed = state
                        .internal_shell
                        .as_mut()
                        .is_some_and(|shell| shell.apply_system_status_update(update));
                    if changed {
                        state.sync_internal_shell();
                        state.request_output_redraw();
                    }
                }
            })
            .map_err(|error| format!("could not register internal system feed: {error}"))?;
        let feature_settings =
            nickel_core::optional_features::OptionalFeatureSettings::load_default();
        let codex_enabled = feature_settings.effective_codex_enabled();
        if codex_enabled {
            use nickel_core::optional_features::{
                CodexAvailabilityProjection, FeatureHealth, FeatureInstallation, FeatureSupport,
            };
            // The in-process host is the selected Codex installation. Publish a
            // recoverable loading projection before the first menu is opened;
            // otherwise LiveShell's unavailable default hides the only affordance
            // capable of starting the host and discovering its real state.
            shell.apply_codex_projection(CodexAvailabilityProjection::new(
                FeatureSupport::Supported,
                FeatureInstallation::Installed,
                true,
                FeatureHealth::Loading,
                feature_settings.codex_generation,
                Some("Checking the selected Codex backend…".into()),
            ));
        }
        self.internal_codex = codex_enabled.then(|| {
            crate::internal_codex::InternalCodexHost::new(
                feature_settings.codex_source,
                shell.semantic_theme(),
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/")),
            )
        });
        self.internal_shell = Some(shell);
        self.reconcile_internal_shell_outputs();
        Ok(())
    }

    /// Arm exactly one compositor-loop wakeup for the shell's earliest real
    /// deadline. Re-arming the same deadline is a no-op, so damage and input
    /// paths may call this freely without recreating a frame-rate poller.
    pub(crate) fn schedule_internal_shell_deadline(&mut self) {
        let shell_deadline = self
            .internal_shell
            .as_ref()
            .and_then(crate::internal_shell::InternalShellCoordinator::next_deadline);
        let codex_deadline = self
            .internal_codex
            .as_ref()
            .and_then(|host| host.next_deadline(&self.internal_ui));
        let deadline = shell_deadline.into_iter().chain(codex_deadline).min();
        self.arm_internal_shell_timer(deadline);
    }

    /// Wake the shell once after input or an externally-driven state change.
    /// The callback replaces this immediate wakeup with the next application
    /// deadline (if any).
    fn wake_internal_shell(&mut self) {
        if self.internal_shell.is_some() {
            self.arm_internal_shell_timer(Some(Instant::now()));
        }
    }

    fn arm_internal_shell_timer(&mut self, deadline: Option<Instant>) {
        if self.internal_shell_timer.deadline == deadline {
            return;
        }
        if let Some(token) = self.internal_shell_timer.token.take() {
            self.event_loop_handle.remove(token);
            self.internal_shell_timer.counters.cancelled = self
                .internal_shell_timer
                .counters
                .cancelled
                .saturating_add(1);
        }
        self.internal_shell_timer.deadline = deadline;
        self.internal_shell_timer.generation = self.internal_shell_timer.generation.wrapping_add(1);
        let generation = self.internal_shell_timer.generation;
        let Some(deadline) = deadline else {
            return;
        };
        match self.event_loop_handle.insert_source(
            smithay::reexports::calloop::timer::Timer::from_deadline(deadline),
            move |_, _, state| {
                if state.internal_shell_timer.generation != generation {
                    return smithay::reexports::calloop::timer::TimeoutAction::Drop;
                }
                state.internal_shell_timer.deadline = None;
                state.internal_shell_timer.token = None;
                state.internal_shell_timer.counters.fired =
                    state.internal_shell_timer.counters.fired.saturating_add(1);
                state.internal_shell_timer.counters.polls =
                    state.internal_shell_timer.counters.polls.saturating_add(1);
                state.poll_internal_shell(Instant::now());
                state.schedule_internal_shell_deadline();
                let counters = state.internal_shell_timer_counters();
                tracing::trace!(
                    armed = counters.armed,
                    cancelled = counters.cancelled,
                    fired = counters.fired,
                    polls = counters.polls,
                    redraw_requests = counters.redraw_requests,
                    next_deadline = ?state.internal_shell_timer.deadline,
                    "internal shell one-shot timer counters"
                );
                smithay::reexports::calloop::timer::TimeoutAction::Drop
            },
        ) {
            Ok(token) => {
                self.internal_shell_timer.token = Some(token);
                self.internal_shell_timer.counters.armed =
                    self.internal_shell_timer.counters.armed.saturating_add(1);
            }
            Err(error) => {
                self.internal_shell_timer.deadline = None;
                tracing::error!(%error, "could not schedule internal shell deadline");
            }
        }
    }

    pub(crate) fn internal_shell_timer_counters(&self) -> InternalShellTimerCounters {
        self.internal_shell_timer.counters
    }

    fn internal_outputs(&self) -> Vec<(crate::internal_shell::InternalOutput, i32, i32)> {
        let mut outputs = self
            .space
            .outputs()
            .filter_map(|output| {
                let geometry = self.space.output_geometry(output)?;
                Some((
                    crate::internal_shell::InternalOutput {
                        name: output.name(),
                        width: geometry.size.w.max(0) as u32,
                        height: geometry.size.h.max(0) as u32,
                        scale: output.current_scale().fractional_scale() as f32,
                    },
                    geometry.loc.x,
                    geometry.loc.y,
                ))
            })
            .collect::<Vec<_>>();
        if let Some(primary) = self.primary_output_name.as_deref()
            && let Some(index) = outputs
                .iter()
                .position(|(output, _, _)| output.name == primary)
        {
            outputs.swap(0, index);
        }
        outputs
    }

    pub(crate) fn reconcile_internal_shell_outputs(&mut self) {
        let outputs = self.internal_outputs();
        for id in self
            .internal_shell_surfaces
            .drain()
            .map(|(_, runtime)| runtime)
            .collect::<Vec<_>>()
        {
            self.internal_ui.remove(id);
        }
        let Some(shell) = self.internal_shell.as_mut() else {
            return;
        };
        shell.set_outputs(
            &outputs
                .iter()
                .map(|(output, _, _)| output.clone())
                .collect::<Vec<_>>(),
        );

        for surface in shell.surfaces().to_vec() {
            if !shell.visible(surface.id) {
                continue;
            }
            let Some(scene) = shell.scene(surface.id) else {
                continue;
            };
            let placement = internal_shell_surface_placement(
                surface.role,
                surface.output.as_deref(),
                surface.size,
                &outputs,
                self.launcher_output_name.as_deref(),
            );
            let scale = surface
                .output
                .as_deref()
                .and_then(|name| outputs.iter().find(|(output, _, _)| output.name == name))
                .map_or(1.0, |(output, _, _)| output.scale);
            let runtime_id = self.internal_ui.insert_scene(scene, placement, scale);
            self.internal_shell_surfaces.insert(surface.id, runtime_id);
        }
        self.schedule_internal_ui_frame();
        self.wake_internal_shell();
    }

    pub(crate) fn poll_internal_shell(&mut self, now: Instant) {
        if self.internal_shell.is_some() {
            let snapshot = self.protocol_snapshot();
            *self.internal_projection_outputs.write().unwrap() = snapshot.outputs.clone();
            let shell = self.internal_shell.as_mut().unwrap();
            shell.apply_session_snapshot(snapshot);
            let changed = shell.poll(now);
            let actions = shell.drain_file_actions();
            let shell_changed = !changed.is_empty();
            let codex_menu_visible = shell.codex_project_menu_visible();
            let codex_menu_anchor = shell
                .popover_anchor(nickel_session_protocol::AnchorSide::Above)
                .and_then(|(role, anchor)| {
                    (role == nickel_session_protocol::ShellRole::ProjectMenu).then_some(anchor)
                });
            let requested_codex_project = shell.take_requested_codex_project();
            let _ = shell;
            let outputs = self.internal_outputs();
            let menu_output = codex_menu_anchor
                .as_ref()
                .map(|anchor| anchor.output.as_str())
                .or_else(|| {
                    self.internal_codex
                        .as_ref()
                        .and_then(crate::internal_codex::InternalCodexHost::project_menu)
                        .and_then(|id| self.internal_ui.placement(id))
                        .and_then(|placement| placement.output.as_deref())
                })
                .map(str::to_owned);
            let fallback = self.resolve_interaction_output(InvocationSource::RecentInteraction);
            if shell_changed {
                self.sync_internal_shell();
            }
            for action in actions {
                self.apply_internal_file_action(action);
            }
            if codex_menu_visible {
                let placement = internal_codex_project_menu_placement(
                    codex_menu_anchor.as_ref(),
                    &outputs,
                    fallback.as_deref(),
                );
                if let Err(error) = self.show_internal_codex_project_menu(placement) {
                    tracing::warn!(%error, "could not host Codex project menu internally");
                }
            } else if let Some(mut host) = self.internal_codex.take() {
                if let Some(menu) = host.project_menu() {
                    host.close(&mut self.internal_ui, menu);
                }
                self.internal_codex = Some(host);
            }
            if let Some(project_id) = requested_codex_project
                && let Some(mut host) = self.internal_codex.take()
            {
                let placement = internal_codex_chat_placement(
                    &outputs,
                    menu_output.as_deref().or(fallback.as_deref()),
                );
                if let Err(error) =
                    host.open_project_by_id(&mut self.internal_ui, placement, &project_id)
                {
                    tracing::warn!(%error, %project_id, "could not open internal Codex project");
                }
                self.internal_codex = Some(host);
            }
        }
        let chat_output = self
            .internal_codex
            .as_ref()
            .and_then(crate::internal_codex::InternalCodexHost::project_menu)
            .and_then(|id| self.internal_ui.placement(id))
            .and_then(|placement| placement.output.as_deref())
            .map(str::to_owned)
            .or_else(|| self.resolve_interaction_output(InvocationSource::RecentInteraction));
        let chat_placement =
            internal_codex_chat_placement(&self.internal_outputs(), chat_output.as_deref());
        if let Some(mut codex) = self.internal_codex.take() {
            let changed = codex.poll_due(&mut self.internal_ui, now);
            let opened = codex
                .service_requests(&mut self.internal_ui, chat_placement)
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "could not service internal Codex request");
                    Vec::new()
                });
            self.internal_codex = Some(codex);
            if !changed.is_empty() || !opened.is_empty() {
                self.schedule_internal_ui_frame();
            }
        }
        let closing = self
            .internal_file_surfaces
            .iter()
            .filter_map(|(coordinator, runtime)| {
                self.internal_ui
                    .application::<nickel_file::FileApp>(*runtime)
                    .is_some_and(nickel_file::FileApp::close_requested)
                    .then_some(*coordinator)
            })
            .collect::<Vec<_>>();
        for id in closing {
            let action = self
                .internal_shell
                .as_mut()
                .unwrap()
                .file_windows_mut()
                .handle(nickel_file::FileWindowRequest::Close(id));
            self.apply_internal_file_action(action);
        }
        if self.internal_ui.has_damage() {
            self.schedule_internal_ui_frame();
        }
    }

    pub(crate) fn refresh_internal_shell_system(&mut self) {
        let changed = self
            .internal_shell
            .as_mut()
            .is_some_and(|shell| !shell.refresh_system().is_empty());
        if changed {
            self.sync_internal_shell();
        }
        self.schedule_internal_shell_deadline();
    }

    pub(crate) fn show_internal_codex_project_menu(
        &mut self,
        placement: crate::internal_codex::CodexSurfacePlacement,
    ) -> Result<nickel_ui::InternalSurfaceId, String> {
        let mut host = self
            .internal_codex
            .take()
            .ok_or_else(|| "Codex integration is disabled".to_owned())?;
        let previous = host
            .project_menu()
            .and_then(|id| self.internal_ui.placement(id).cloned());
        let result = host.ensure_project_menu(&mut self.internal_ui, placement);
        self.internal_codex = Some(host);
        if let Ok(id) = result {
            let presentation_changed = previous.as_ref() != self.internal_ui.placement(id);
            self.internal_ui.focus_surface(id);
            if presentation_changed {
                self.schedule_internal_ui_frame();
            }
        }
        result
    }

    fn apply_internal_file_action(&mut self, action: nickel_file::FileWindowAction) {
        use nickel_file::FileWindowAction;
        match action {
            FileWindowAction::Opened(id) => {
                let Some(shell) = self.internal_shell.as_mut() else {
                    return;
                };
                let Some(surface) = shell.file_windows_mut().take_surface(id) else {
                    return;
                };
                let Some((output, x, y)) = self.internal_outputs().into_iter().next() else {
                    return;
                };
                let size = surface.logical_size();
                let offset = (self.internal_file_surfaces.len() as i32 * 32) % 192;
                let runtime = self.internal_ui.insert_boxed(
                    surface,
                    crate::session::InternalSurfacePlacement {
                        role: crate::session::InternalSurfaceRole::Application,
                        geometry: (
                            x + 48 + offset,
                            y + 64 + offset,
                            size.0.min(output.width),
                            size.1.min(output.height),
                        ),
                        output: Some(output.name),
                    },
                    1.0,
                );
                self.internal_file_surfaces.insert(id, runtime);
                self.internal_ui.focus_surface(runtime);
            }
            FileWindowAction::Focused(id) => {
                if let Some(runtime) = self.internal_file_surfaces.get(&id).copied() {
                    self.internal_ui.focus_surface(runtime);
                }
            }
            FileWindowAction::Closed(id) => {
                if let Some(runtime) = self.internal_file_surfaces.remove(&id) {
                    self.internal_ui.remove(runtime);
                }
            }
            FileWindowAction::NotFound(_) => {}
        }
    }

    pub(crate) fn toggle_internal_launcher(&mut self) -> bool {
        let Some(was_visible) = self
            .internal_shell
            .as_ref()
            .map(crate::internal_shell::InternalShellCoordinator::launcher_visible)
        else {
            return false;
        };
        if !was_visible {
            self.launcher_output_name = self.resolve_interaction_output(InvocationSource::Keyboard);
        }
        let changed = self.internal_shell.as_mut().unwrap().toggle_launcher();
        if changed {
            self.sync_internal_shell();
            self.wake_internal_shell();
        }
        changed
    }

    pub(crate) fn flush_internal_shell_input(&mut self) {
        let events = self.internal_ui.drain_routed_events();
        if events.is_empty() || self.internal_shell.is_none() {
            return;
        }
        let reverse = self
            .internal_shell
            .as_ref()
            .unwrap()
            .surfaces()
            .iter()
            .filter_map(|surface| {
                self.internal_shell_surfaces
                    .get(&surface.id)
                    .map(|runtime| (*runtime, (surface.id, surface.role, surface.output.clone())))
            })
            .collect::<HashMap<_, _>>();
        let output_origins = self
            .internal_outputs()
            .into_iter()
            .map(|(output, x, y)| (output.name, (x, y)))
            .collect::<HashMap<_, _>>();
        let launcher_was_visible = self
            .internal_shell
            .as_ref()
            .is_some_and(crate::internal_shell::InternalShellCoordinator::launcher_visible);
        let shell = self.internal_shell.as_mut().unwrap();
        let mut changed = false;
        for (runtime_id, event) in events {
            let Some((shell_id, role, output)) = reverse.get(&runtime_id).cloned() else {
                continue;
            };
            if role == crate::winit_shell::SurfaceRole::Panel
                && let Some(output) = output
            {
                let origin = output_origins.get(&output).copied().unwrap_or_default();
                shell.set_panel_context(output, origin);
            }
            changed |= shell.step_slot(
                shell_id,
                nickel_ui::HostBatch {
                    events: vec![nickel_ui::HostEvent::Ui(event)],
                    ..Default::default()
                },
            );
        }
        let launcher_is_visible = shell.launcher_visible();
        let _ = shell;
        if !launcher_was_visible && launcher_is_visible {
            self.launcher_output_name =
                self.resolve_interaction_output(InvocationSource::RecentInteraction);
        }
        if changed {
            self.sync_internal_shell();
        }
        self.wake_internal_shell();
    }

    pub(crate) fn sync_internal_shell(&mut self) {
        let Some(mut shell) = self.internal_shell.take() else {
            return;
        };
        let entries = shell.surfaces().to_vec();
        for surface in entries {
            // The real ChatApplication host owns this role. LiveShell retains
            // only its visibility policy and must not paint a second shell
            // scene over the compositor-owned menu.
            if surface.role == crate::winit_shell::SurfaceRole::CodexProjectMenu {
                if let Some(runtime_id) = self.internal_shell_surfaces.remove(&surface.id) {
                    self.internal_ui.remove(runtime_id);
                }
                continue;
            }
            let visible = shell.visible(surface.id);
            if !visible {
                if let Some(runtime_id) = self.internal_shell_surfaces.remove(&surface.id) {
                    self.internal_ui.remove(runtime_id);
                }
                continue;
            }
            let Some(scene) = shell.scene(surface.id) else {
                continue;
            };
            let placement = internal_shell_surface_placement(
                surface.role,
                surface.output.as_deref(),
                surface.size,
                &self.internal_outputs(),
                self.launcher_output_name.as_deref(),
            );
            if let Some(runtime_id) = self.internal_shell_surfaces.get(&surface.id).copied() {
                self.internal_ui.update_scene(runtime_id, scene);
                self.internal_ui.relocate(runtime_id, placement);
                continue;
            }
            let output_scale = surface
                .output
                .as_deref()
                .and_then(|name| self.space.outputs().find(|output| output.name() == name))
                .map_or(1.0, |output| {
                    output.current_scale().fractional_scale() as f32
                });
            let runtime_id = self
                .internal_ui
                .insert_scene(scene, placement, output_scale);
            self.internal_shell_surfaces.insert(surface.id, runtime_id);
        }
        self.internal_shell = Some(shell);
        self.schedule_internal_ui_frame();
    }

    pub fn insert_internal_surface<A: nickel_ui::Application + 'static>(
        &mut self,
        application: A,
        placement: crate::session::InternalSurfacePlacement,
        scale: f32,
    ) -> nickel_ui::InternalSurfaceId {
        let id = self.internal_ui.insert(application, placement, scale);
        self.schedule_internal_ui_frame();
        id
    }

    pub fn remove_internal_surface(&mut self, id: nickel_ui::InternalSurfaceId) -> bool {
        let removed = self.internal_ui.remove(id);
        if removed {
            self.schedule_internal_ui_frame();
        }
        removed
    }

    pub fn step_internal_surface(
        &mut self,
        id: nickel_ui::InternalSurfaceId,
        batch: nickel_ui::HostBatch,
    ) -> bool {
        let changed = self.internal_ui.step(id, batch);
        if changed {
            self.schedule_internal_ui_frame();
        }
        changed
    }

    fn schedule_internal_ui_frame(&mut self) {
        self.internal_shell_timer.counters.redraw_requests = self
            .internal_shell_timer
            .counters
            .redraw_requests
            .saturating_add(1);
        self.request_output_redraw();
        #[cfg(feature = "backend-udev")]
        if self.native.is_some() {
            self.render_all_outputs();
        }
        self.schedule_internal_shell_deadline();
    }

    pub(crate) fn configured_output_scale(&self, output: &Output) -> OutputScale {
        self.output_scale_preferences
            .get(&stable_output_identity(output))
            .map(|scale| OutputScale::Fractional(scale.factor()))
            .unwrap_or(OutputScale::Integer(1))
    }
    /// Map a compositor-managed window only while its Wayland client has a
    /// buffer attached. X11 windows and non-XDG surfaces are unaffected.
    pub(crate) fn map_buffered_window(
        &mut self,
        window: Window,
        location: impl Into<Point<i32, Logical>>,
        activate: bool,
    ) -> bool {
        if let Some(surface) = window.wl_surface()
            && self.xdg_toplevel_windows.contains_key(&surface.id())
            && !self.mapped_xdg_toplevels.contains(&surface.id())
        {
            return false;
        }
        if self.panel_hidden_by_keyboard(&window) {
            self.space.unmap_elem(&window);
            return false;
        }
        self.space.map_element(window.clone(), location, activate);
        self.fit_window_above_keyboard(&window);
        true
    }
}
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

fn shell_registration_is_active(
    registration: &RegisteredShellRole,
    output_names: &HashSet<String>,
    expected_panel_outputs: &HashSet<String>,
) -> bool {
    match registration.role {
        ShellRole::Desktop | ShellRole::Lock => registration
            .output
            .as_ref()
            .is_some_and(|output| output_names.contains(output)),
        ShellRole::Panel => registration
            .output
            .as_ref()
            .is_some_and(|output| expected_panel_outputs.contains(output)),
        _ => true,
    }
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
                    crate::session::session_services::request(
                        crate::session::session_services::SystemAction::Suspend,
                    );
                }
            }
        }
    }

    pub(crate) fn output_global_admission_available(&mut self) -> bool {
        self.reap_output_global_retirements(Instant::now());
        capacity_available(
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
                RetirementAction::Disable { identity, value } => {
                    self.display_handle.disable_global::<NickelSession>(value);
                    identities_to_publish.insert(identity);
                    disabled = true;
                }
                RetirementAction::Remove { value, .. } => {
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
        self.compatibility_control.as_ref().is_some_and(|control| {
            control.expected_shell_pid == pid && control.authenticated_shell_pids.contains(&pid)
        })
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
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_dialog_state = XdgDialogState::new::<Self>(&dh);
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
            crate::session::handlers::is_portal_capture_client,
        );
        let image_copy_capture_state = ImageCopyCaptureState::new_with_filter::<Self, _>(
            &dh,
            crate::session::handlers::is_portal_capture_client,
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

        // The shell itself uses `InProcessSessionHost`; this authenticated
        // endpoint exists only for trusted out-of-process Nickel utilities
        // such as `nickel-settings`. Ordinary applications have both values
        // stripped from their launch environment.
        let compatibility_control = {
            let protocol_token = production_control_token();
            // SAFETY: session initialization is single-threaded and precedes clients.
            unsafe { std::env::set_var("NICKEL_SESSION_TOKEN", &protocol_token) };
            let control_socket_path = Self::init_control_socket(event_loop);
            crate::model::install_trusted_session_capability(
                control_socket_path.as_os_str().to_owned(),
                protocol_token.clone().into(),
            );
            let control_socket_name = control_socket_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("nickel");
            if test_control_enabled {
                let shell_test_path = control_socket_path
                    .with_file_name(format!("nickel-shell-test-{control_socket_name}"));
                // SAFETY: initialization still precedes all client launches.
                unsafe { std::env::set_var("NICKEL_SHELL_TEST_CONTROL", shell_test_path) };
            } else {
                unsafe {
                    std::env::remove_var("NICKEL_SESSION_CONTROL");
                    std::env::remove_var("NICKEL_SESSION_TOKEN");
                    std::env::remove_var("NICKEL_SHELL_TEST_CONTROL");
                }
            }
            Some(CompatibilityControlState {
                protocol_token,
                authenticated_shell_pids: HashSet::new(),
                expected_shell_pid: 0,
                socket_path: control_socket_path,
            })
        };
        let secure_storage_state = Arc::new(AtomicU8::new(
            crate::session::login_services::SecureStorageState::Starting as u8,
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
        let mut workspaces = Workspaces::default();
        let configured_desktops = ShellSettings::load_default().desktop_count;
        let _ = workspaces.set_count(usize::from(configured_desktops));

        Self {
            start_time,
            display_handle: dh,
            event_loop_handle: event_loop.handle(),
            #[cfg(feature = "backend-udev")]
            native: None,

            space,
            internal_ui: Default::default(),
            internal_shell: None,
            internal_codex: None,
            internal_shell_surfaces: HashMap::new(),
            internal_file_surfaces: HashMap::new(),
            internal_shell_timer: InternalShellTimer::default(),
            loop_signal,
            socket_name,

            compositor_state,
            fractional_scale_manager_state,
            viewporter_state,
            xdg_shell_state,
            xdg_dialog_state,
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
            on_screen_keyboard: Default::default(),
            windows: WindowRegistry::default(),
            surface_windows: HashMap::new(),
            surface_effective_outputs: HashMap::new(),
            xdg_toplevel_windows: HashMap::new(),
            mapped_xdg_toplevels: HashSet::new(),
            restored_xdg_toplevels: HashSet::new(),
            xdg_toplevel_locations: HashMap::new(),
            shell_owned_windows: HashSet::new(),
            x11_windows: HashMap::new(),
            launcher_window: None,
            launcher_visibility: LauncherVisibility::default(),
            launcher_output_name: None,
            last_interaction_output_name: None,
            launcher_focus: FocusTransactions::default(),
            launcher_restore_window: None,
            launcher_subscribers: Vec::new(),
            pending_launch_observations: Vec::new(),
            compatibility_control,
            shell_surface_identities: HashMap::new(),
            registered_shell_role_slots: Vec::new(),
            last_logged_shell_readiness: None,
            test_control_enabled,
            #[cfg(target_os = "linux")]
            test_controller: None,
            launcher_show_requested_at: None,
            desktop_windows: Vec::new(),
            panel_windows: Vec::new(),
            lock_windows: Vec::new(),
            locked: false,
            held_consumer_controls: HashSet::new(),
            consumer_repeat_epoch: 0,
            lock_restore_window: None,
            shell_focus_restore_window: None,
            pending_shell_focus_role: None,
            utility_windows: Vec::new(),
            hidden_shell_roles: HashSet::new(),
            hidden_shell_role_locations: HashMap::new(),
            screenshot_output_name: None,
            context_menu_window: None,
            preview_window: None,
            server_decorated: HashSet::new(),
            primary_output_name: None,
            output_topology_generation: 0,
            last_protocol_outputs: Vec::new(),
            output_scale_preferences: nickel_core::dpi::PersistedOutputScales::load_default()
                .unwrap_or_default(),
            virtual_test_outputs: HashMap::new(),
            pending_output_global_retirements: DeferredRetirements::default(),
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
            workspaces,
            workspace_hidden_windows: HashMap::new(),
            displaced_output_windows: HashMap::new(),
            preview_highlight: None,
            minimized_windows: HashMap::new(),
            shortcut_desktop_windows: Vec::new(),
            shortcut_desktop_focus: None,
            shortcut_snap_restore: HashMap::new(),
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
            frame_cursor: crate::session::window_frame::FrameCursor::Arrow,
            buffer_commit_tx: None,
            identify_outputs_until: None,
            identify_outputs_generation: 0,
            output_capture_path: None,
            output_capture_name: None,
            output_capture_reply_path: None,
            output_capture_request_id: None,
            internal_capture: Arc::new(std::sync::Mutex::new(InternalCaptureState::Idle)),
            internal_projection_outputs: Arc::new(std::sync::RwLock::new(Vec::new())),
            shell_failure_count: 0,
            recovery_ui: crate::session::recovery_ui::RecoveryUi::new(),
            secure_storage_state,
            secure_storage_retry,
            deferred_focus_restore,
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

    pub(crate) fn secure_storage_state(
        &self,
    ) -> crate::session::login_services::SecureStorageState {
        crate::session::login_services::SecureStorageState::from_u8(
            self.secure_storage_state.load(Ordering::Acquire),
        )
    }

    pub fn shell_recovery_visible(&self) -> bool {
        crate::session::shell_recovery_visible_for(self.shell_failure_count)
    }

    pub(crate) fn retry_shell_from_recovery(&mut self) -> bool {
        if !self.shell_recovery_visible() {
            return false;
        }
        false
    }

    pub(crate) fn exit_from_recovery(&mut self) -> bool {
        if !self.shell_recovery_visible() {
            return false;
        }
        self.loop_signal.stop();
        true
    }

    pub fn toggle_launcher(&mut self) {
        // IPC callers do not carry a native event object. Pointer and touch input
        // update interaction history before the shell requests this transition.
        self.toggle_launcher_from(InvocationSource::RecentInteraction);
    }

    fn toggle_launcher_from(&mut self, source: InvocationSource) {
        let visible = self.launcher_visibility.toggle();
        if visible {
            self.launcher_output_name = self.resolve_interaction_output(source);
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
            !connected.contains_key(&placement.name)
                || !names.insert(&placement.name)
                || !(60..=480).contains(&placement.scale_120)
        }) {
            return Err("layout contains an unknown, duplicate, or invalidly scaled output");
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
            let scaled_size = |placement: &nickel_session_protocol::OutputPlacement| {
                let (output, fallback) = &connected[&placement.name];
                output.current_mode().map_or(*fallback, |mode| {
                    output
                        .current_transform()
                        .transform_size(mode.size)
                        .to_f64()
                        .to_logical(f64::from(placement.scale_120) / 120.0)
                        .to_i32_round()
                })
            };
            let left_size = scaled_size(left);
            for right in placements
                .iter()
                .skip(index + 1)
                .filter(|output| output.enabled)
            {
                let right_size = scaled_size(right);
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
            output.change_current_state(
                None,
                None,
                Some(OutputScale::Fractional(
                    f64::from(placement.scale_120) / 120.0,
                )),
                Some(location),
            );
            self.space.map_output(&output, location);
            self.output_scale_preferences.set(
                stable_output_identity(&output),
                nickel_core::dpi::Scale120::new(placement.scale_120).unwrap_or_default(),
            );
        }
        if !self.test_control_enabled {
            self.output_scale_preferences
                .save_default()
                .map_err(|_| "could not persist output scales")?;
        }
        self.rescue_stranded_windows();
        self.relayout_shell_surfaces();
        self.reconstrain_all_reactive_popups();
        self.space.refresh();
        self.refresh_surface_scales();
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
                self.map_buffered_window(window, location, true);
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

    fn apply_configured_workspace_count(&mut self) {
        let requested = usize::from(ShellSettings::load_default().desktop_count);
        let Ok(transitions) = self.workspaces.set_count(requested) else {
            return;
        };
        for transition in transitions {
            self.apply_workspace_transition(transition);
        }
        self.notify_workspace_state();
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

    pub fn switch_workspace_number(&mut self, number: usize) {
        let Some(target) = self
            .workspaces
            .ordered()
            .get(number.saturating_sub(1))
            .map(|workspace| workspace.id)
        else {
            tracing::debug!(number, "workspace shortcut target is not configured");
            return;
        };
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

    pub fn create_workspace_and_switch(&mut self) {
        let Ok(target) = self.workspaces.create() else {
            return;
        };
        let output = self.output_name_at_pointer();
        if let Ok(transition) = self.workspaces.switch_to(target, output) {
            self.apply_workspace_transition(transition);
        }
    }

    pub fn remove_active_workspace(&mut self) {
        if let Ok(transition) = self.workspaces.remove(self.workspaces.active()) {
            self.apply_workspace_transition(transition);
        }
    }

    pub fn close_active_window(&mut self) {
        if let Some(id) = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)
            .map(|window| window.id)
        {
            self.close_window(id);
        }
    }

    pub fn maximize_active_window(&mut self) {
        if let Some(id) = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)
            .map(|window| window.id)
        {
            self.maximize_window(id);
        }
    }

    pub fn minimize_active_window(&mut self) {
        if let Some(id) = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)
            .map(|window| window.id)
        {
            self.minimize_window(id);
        }
    }

    pub fn toggle_show_desktop(&mut self) {
        if self.locked {
            return;
        }
        if self.shortcut_desktop_windows.is_empty() {
            let ids = self
                .windows
                .snapshot()
                .into_iter()
                .filter(|window| {
                    !self.minimized_windows.contains_key(&window.id)
                        && self.workspaces.is_visible(&window.id)
                })
                .map(|window| window.id)
                .collect::<Vec<_>>();
            self.shortcut_desktop_focus = self
                .windows
                .snapshot()
                .into_iter()
                .find(|window| window.active)
                .map(|window| window.id);
            for id in ids.iter().copied() {
                self.minimize_window(id);
            }
            self.shortcut_desktop_windows = ids;
        } else {
            let ids = std::mem::take(&mut self.shortcut_desktop_windows);
            let focus = self.shortcut_desktop_focus.take();
            for id in ids.iter().copied().filter(|id| Some(*id) != focus) {
                if self.window_exists(nickel_session_protocol::WindowId(id.0)) {
                    self.activate_window(id);
                }
            }
            if let Some(id) = focus.filter(|id| ids.contains(id))
                && self.window_exists(nickel_session_protocol::WindowId(id.0))
            {
                self.activate_window(id);
            }
        }
    }

    pub fn snap_active_window(&mut self, leading: bool) {
        let Some(id) = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)
            .map(|window| window.id)
        else {
            return;
        };
        if self
            .protocol_windows()
            .into_iter()
            .any(|candidate| candidate.id.0 == id.0 && candidate.fullscreen)
        {
            return;
        }
        let Some(window) = self.window_for_registry_id(id) else {
            return;
        };
        let Some(output) = self.space.outputs_for_element(&window).into_iter().next() else {
            return;
        };
        let Some(frame) = self.space.element_geometry(&window) else {
            return;
        };
        let output = self.space.output_geometry(&output).unwrap_or(frame);
        let area = self.work_area_for_output(Geometry {
            x: output.loc.x,
            y: output.loc.y,
            width: output.size.w,
            height: output.size.h,
        });
        self.shortcut_snap_restore.entry(id).or_insert(frame);
        let width = (area.width / 2).max(1);
        let leading = if locale_is_rtl() { !leading } else { leading };
        let x = if leading {
            area.x
        } else {
            area.x + area.width - width
        };
        let target =
            smithay::utils::Rectangle::new((x, area.y).into(), (width, area.height).into());
        if let Some(surface) = window.x11_surface() {
            let _ = surface.configure(target);
        }
        self.map_compositor_moved_window(window, target.loc, true);
        if let Some(surface) = self
            .window_for_registry_id(id)
            .and_then(|window| window.toplevel().cloned())
        {
            surface.with_pending_state(|state| state.size = Some((width, area.height).into()));
            surface.send_pending_configure();
        }
        self.notify_protocol_snapshot();
    }

    pub fn restore_or_minimize_active_window(&mut self) {
        let Some(snapshot) = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)
            .cloned()
        else {
            return;
        };
        if let Some(restore) = self.shortcut_snap_restore.remove(&snapshot.id) {
            let Some(window) = self.window_for_registry_id(snapshot.id) else {
                return;
            };
            let location = self
                .output_geometry_for_window(&window)
                .map(|output| {
                    clamp_window_location(
                        restore.loc,
                        restore.size,
                        self.work_area_for_output(output),
                    )
                })
                .unwrap_or(restore.loc);
            self.map_compositor_moved_window(window.clone(), location, true);
            if let Some(surface) = window.toplevel() {
                surface.with_pending_state(|state| state.size = Some(restore.size));
                surface.send_pending_configure();
            }
        } else if self
            .protocol_windows()
            .into_iter()
            .any(|window| window.id.0 == snapshot.id.0 && window.maximized)
        {
            self.maximize_window(snapshot.id);
        } else {
            self.minimize_window(snapshot.id);
        }
    }

    pub fn move_active_window_to_output_direction(&mut self, previous: bool) {
        let Some(id) = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)
            .map(|window| window.id)
        else {
            return;
        };
        let Some(window) = self.window_for_registry_id(id) else {
            return;
        };
        let current = self.output_name_for_window(&window);
        let mut outputs = self
            .space
            .outputs()
            .filter_map(|output| {
                self.space
                    .output_geometry(output)
                    .map(|geometry| (geometry.loc.x, output.name()))
            })
            .collect::<Vec<_>>();
        outputs.sort_by_key(|entry| entry.0);
        let Some(index) = outputs
            .iter()
            .position(|(_, name)| Some(name) == current.as_ref())
        else {
            return;
        };
        let target = if previous {
            index.checked_sub(1)
        } else if index + 1 < outputs.len() {
            Some(index + 1)
        } else {
            None
        };
        if let Some(target) = target {
            self.move_window_to_output(id, &outputs[target].1);
        }
    }

    fn output_name_at_pointer(&self) -> Option<String> {
        let location = self.seat.get_pointer()?.current_location();
        self.output_name_at(location)
    }

    /// Resolves a logical point through the compositor's single output hit
    /// authority. Input handlers record the result rather than reproducing the
    /// output geometry walk themselves.
    fn output_name_at(&self, location: Point<f64, Logical>) -> Option<String> {
        self.space.outputs().find_map(|output| {
            self.space
                .output_geometry(output)
                .filter(|geometry| geometry.to_f64().contains(location))
                .map(|_| output.name())
        })
    }

    pub(crate) fn record_interaction_output(&mut self, location: Point<f64, Logical>) {
        self.last_interaction_output_name = self.output_name_at(location);
    }

    pub fn set_launcher_visible(&mut self, visible: bool) {
        // Panel pointer/touch input has already updated interaction history.
        self.set_launcher_visible_from(visible, InvocationSource::RecentInteraction);
    }

    fn set_launcher_visible_from(&mut self, visible: bool, source: InvocationSource) {
        let changed = self.launcher_visibility.is_visible() != visible;
        if changed && visible {
            self.launcher_show_requested_at = Some(std::time::Instant::now());
            self.launcher_output_name = self.resolve_interaction_output(source);
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
        if self.toggle_internal_launcher() {
            return;
        }
        self.set_launcher_visible_from(
            !self.launcher_visibility.is_visible(),
            InvocationSource::Keyboard,
        );
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

    pub(crate) fn observe_pending_launch_window(&mut self, client_pid: u32) {
        let now = Instant::now();
        let mut observations = Vec::new();
        self.pending_launch_observations
            .retain(
                |pending| match pending_launch_window_disposition(pending, now, client_pid) {
                    PendingLaunchWindowDisposition::AwaitExpiry
                    | PendingLaunchWindowDisposition::Unrelated => true,
                    PendingLaunchWindowDisposition::Attributed { descendant } => {
                        let elapsed = now.saturating_duration_since(pending.registered_at);
                        observations.push((
                            pending.generation,
                            elapsed.as_millis().min(u128::from(u16::MAX)) as u16,
                            descendant,
                        ));
                        false
                    }
                },
            );
        if observations.is_empty() {
            return;
        }
        let Ok(socket) = UnixDatagram::unbound() else {
            return;
        };
        for (generation, observed_after_ms, descendant) in observations {
            let Ok(event) = encode(&ServerEnvelope {
                request_id: 0,
                message: ServerMessage::Event(SessionEvent::PendingLaunchWindow {
                    generation,
                    observed_after_ms,
                    descendant,
                }),
            }) else {
                continue;
            };
            self.launcher_subscribers
                .retain(|path| socket.send_to(&event, path).is_ok());
        }
    }

    fn notify_pending_launch_expired(&mut self, generation: u64) {
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::PendingLaunchExpired { generation }),
        }) else {
            return;
        };
        let Ok(socket) = UnixDatagram::unbound() else {
            return;
        };
        self.launcher_subscribers
            .retain(|path| socket.send_to(&event, path).is_ok());
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

    fn refresh_output_topology_generation(&mut self) -> bool {
        let outputs = self.protocol_outputs();
        if outputs != self.last_protocol_outputs {
            self.last_protocol_outputs = outputs;
            self.output_topology_generation =
                self.output_topology_generation.wrapping_add(1).max(1);
            if self.internal_shell.is_some() {
                self.reconcile_internal_shell_outputs();
            }
            true
        } else {
            false
        }
    }

    fn protocol_shell_behavior(&self) -> ShellBehaviorSnapshot {
        let settings = ShellSettings::load_default();
        ShellBehaviorSnapshot {
            bar_on_all_displays: settings.bar_on_all_displays,
            all_windows_on_every_bar: settings.all_windows_on_every_bar,
            desktop_count: settings.desktop_count,
            topology_generation: self.output_topology_generation,
        }
    }

    fn notify_shell_behavior_snapshot(&mut self, snapshot: ShellBehaviorSnapshot) {
        let event = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::ShellBehaviorChanged(snapshot)),
        });
        if let Ok(event) = event
            && let Ok(socket) = UnixDatagram::unbound()
        {
            self.launcher_subscribers
                .retain(|path| socket.send_to(&event, path).is_ok());
        }
    }

    fn apply_shell_behavior_transaction(
        &mut self,
        transaction: ShellBehaviorTransaction,
    ) -> ServerMessage {
        let _ = self.refresh_output_topology_generation();
        if transaction.topology_generation != self.output_topology_generation {
            return protocol_error(
                ErrorCode::InvalidRequest,
                format!(
                    "stale output topology generation {}; current generation is {}",
                    transaction.topology_generation, self.output_topology_generation
                ),
            );
        }
        let previous = ShellSettings::load_default();
        let requested = match prepare_shell_behavior_update(
            &previous,
            self.output_topology_generation,
            &transaction,
        ) {
            Ok(requested) => requested,
            Err(error) => return protocol_error(ErrorCode::InvalidRequest, error),
        };
        if let Err(error) = requested.save_default() {
            return protocol_error(
                ErrorCode::Internal,
                format!("could not persist shell setting: {error}"),
            );
        }
        let requested_count = usize::from(requested.desktop_count);
        let transitions = match self.workspaces.set_count(requested_count) {
            Ok(transitions) => transitions,
            Err(error) => {
                let rollback = previous.save_default();
                return protocol_error(
                    ErrorCode::Internal,
                    format!(
                        "could not apply shell setting: {error:?}; persistence rollback: {}",
                        if rollback.is_ok() {
                            "complete"
                        } else {
                            "failed"
                        }
                    ),
                );
            }
        };
        for transition in transitions {
            self.apply_workspace_transition(transition);
        }
        let effective = self.protocol_shell_behavior();
        if let Some(shell) = self.internal_shell.as_mut()
            && shell.set_bar_on_all_displays(effective.bar_on_all_displays)
        {
            self.reconcile_internal_shell_outputs();
        }
        self.notify_shell_behavior_snapshot(effective.clone());
        ServerMessage::ShellBehavior(effective)
    }

    pub(crate) fn notify_global_shortcut(
        &mut self,
        action: nickel_session_protocol::ShortcutAction,
    ) {
        tracing::info!(?action, "global shortcut activated");
        if let Some(shell) = self.internal_shell.as_mut() {
            let changed = shell.global_shortcut(action);
            if changed {
                self.sync_internal_shell();
                self.schedule_internal_ui_frame();
            }
            self.wake_internal_shell();
            return;
        }
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

    pub(crate) fn notify_consumer_control(
        &mut self,
        control: nickel_session_protocol::ConsumerControl,
    ) {
        tracing::info!(
            ?control,
            subscribers = self.launcher_subscribers.len(),
            "consumer control activated"
        );
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::ConsumerControl { control }),
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
        if self.refresh_output_topology_generation() {
            self.notify_shell_behavior_snapshot(self.protocol_shell_behavior());
        }
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
            if !self.map_buffered_window(window.clone(), location, true) {
                return;
            }
            let surface = window.toplevel().unwrap().wl_surface().clone();
            let _request = self.launcher_focus.request(surface.id());
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                Some(crate::session::focus::KeyboardFocusTarget::Wayland(surface)),
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
            "nickel: launcher {}",
            if visible { "shown" } else { "hidden" }
        );
    }

    fn restore_launcher_focus(&mut self) {
        if let Some(window) = self.launcher_restore_window.take() {
            self.activate_window(window);
        }
    }

    pub(crate) fn focus_shell_role(&mut self, role: ShellRole) -> bool {
        if !shell_role_accepts_ordinary_focus(role) {
            return false;
        }
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
                    crate::session::focus::KeyboardFocusTarget::Wayland(surface) => {
                        self.surface_windows.get(&surface.id()).copied()
                    }
                    crate::session::focus::KeyboardFocusTarget::X11(surface) => {
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
            crate::session::focus::KeyboardFocusTarget::for_window(&target),
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
                Option::<crate::session::focus::KeyboardFocusTarget>::None,
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
            self.mapped_xdg_toplevels.remove(surface_id);
            self.xdg_toplevel_windows.remove(surface_id);
            self.xdg_toplevel_locations.remove(surface_id);
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

    pub(crate) fn record_shell_role_registration(
        &mut self,
        window: &Window,
        role: ShellRole,
        output: Option<String>,
    ) {
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

    pub(crate) fn registered_shell_identity(
        &self,
        application_id: Option<&str>,
    ) -> Option<ShellSurfaceIdentity> {
        self.shell_surface_identities.get(application_id?).cloned()
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
        requested_position: bool,
        parent_output: Option<&str>,
        cascade: i32,
    ) -> Rectangle<i32, Logical> {
        let content = Geometry {
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w.max(1),
            height: geometry.size.h.max(1),
        };
        let active_output = self.new_window_active_output_name();
        let Some(decision) = shell_layout::resolve_window_output(
            &self.placement_outputs(),
            parent_output,
            requested_position.then_some(content),
            None,
            active_output.as_deref(),
        ) else {
            return geometry;
        };
        let content = if requested_position {
            clamp_decorated_content_to_work_area(content, decision.work_area)
        } else {
            shell_layout::initial_window_sized(
                decision.work_area,
                (content.width, content.height),
                cascade,
            )
        };
        tracing::info!(
            output = %decision.output_name,
            reason = ?decision.reason,
            requested_position,
            "diagnostic: captured X11 new-window placement"
        );
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
        let popup_root = window
            .toplevel()
            .map(|surface| surface.wl_surface().clone());
        self.map_buffered_window(window, location, activate);
        if let Some(root) = popup_root {
            self.reconstrain_reactive_popups(&root);
        }
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
            // Cropping is modal over the captured desktop, including the OSK.
            // Keep it below the lock surface (100), but above the keyboard (60).
            window.override_z_index(90);
            self.place_screenshot_surface(&window);
        }
        if role == ShellRole::OnScreenKeyboard {
            window.override_z_index(60);
            if self.on_screen_keyboard_snapshot().visible {
                self.place_on_screen_keyboard_surface(&window);
            } else {
                self.hidden_shell_roles.insert(role);
            }
        }
        if self.hidden_shell_roles.contains(&role) {
            if let Some(location) = self.space.element_location(&window) {
                self.hidden_shell_role_locations.insert(role, location);
            }
            self.space.unmap_elem(&window);
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
        self.map_buffered_window(window.clone(), location, true);
    }

    pub(crate) fn relayout_committed_shell_window(&mut self, window: &Window) {
        if self.is_on_screen_keyboard_window(window) && self.on_screen_keyboard_snapshot().visible {
            self.place_on_screen_keyboard_surface(window);
            return;
        }
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
            // A windowing runtime may recreate native Wayland surfaces when a
            // hidden window is shown. The replacement may register before the
            // old surface's unmap reaches Smithay, so mapped-state alone cannot
            // identify the stale identity. There is exactly one lock surface per output;
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
            let focus = window.wl_surface().map(|surface| {
                crate::session::focus::KeyboardFocusTarget::Wayland(surface.into_owned())
            });
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
                self.map_buffered_window(lock.clone(), geometry.loc, true);
                self.space.raise_element(&lock, true);
            } else {
                self.space.unmap_elem(&lock);
            }
        }
    }

    pub(crate) fn lock_session(&mut self) {
        if self.locked {
            return;
        }
        self.locked = true;
        // The focus transition to the compositor-owned lock surface terminates any
        // in-flight desktop chord. Do not let a missing physical release retain Alt,
        // Super, or the lock key across the secure-session boundary.
        self.hotkeys.reset_chord_state_preserving_owned_releases();
        self.cancel_consumer_control_repeats();
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
            .map(|surface| {
                crate::session::focus::KeyboardFocusTarget::Wayland(surface.into_owned())
            });
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
        self.hotkeys.reset_pressed_state();
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
        self.context_menu_window = Some(window.clone());
        if self.hidden_shell_roles.contains(&ShellRole::ContextMenu) {
            if let Some(location) = self.space.element_location(&window) {
                self.hidden_shell_role_locations
                    .insert(ShellRole::ContextMenu, location);
            }
            self.space.unmap_elem(&window);
        }
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
        self.preview_window = Some(window.clone());
        if self.hidden_shell_roles.contains(&ShellRole::Preview) {
            if let Some(location) = self.space.element_location(&window) {
                self.hidden_shell_role_locations
                    .insert(ShellRole::Preview, location);
            }
            self.space.unmap_elem(&window);
        }
    }

    pub fn show_context_menu(
        &mut self,
        x: i32,
        y: i32,
        requested_width: i32,
        requested_height: i32,
        focus: bool,
    ) {
        self.show_transient(
            self.context_menu_window.clone(),
            x,
            y,
            requested_width,
            requested_height,
            focus,
            "context menu",
        );
    }

    pub fn show_preview(&mut self, x: i32, y: i32, requested_width: i32, requested_height: i32) {
        self.show_transient(
            self.preview_window.clone(),
            x,
            y,
            requested_width,
            requested_height,
            false,
            "preview",
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn show_transient(
        &mut self,
        window: Option<Window>,
        x: i32,
        y: i32,
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
            .find(|geometry| output_contains_logical_point(*geometry, x, y))
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
        if !self.map_buffered_window(window.clone(), (x, y), focus) {
            return;
        }
        if focus {
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                crate::session::focus::KeyboardFocusTarget::for_window(&window),
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
        eprintln!("nickel: {label} shown at {x},{y}");
    }

    pub fn hide_context_menu(&mut self) {
        if let Some(window) = self.context_menu_window.clone() {
            self.space.unmap_elem(&window);
        }
        self.preview_highlight = None;
        eprintln!("nickel: context menu hidden");
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
        eprintln!("nickel: transient overlays hidden");
    }

    pub(crate) fn set_shell_role_visible(&mut self, role: ShellRole, visible: bool) {
        if matches!(
            role,
            ShellRole::Desktop | ShellRole::Panel | ShellRole::Lock | ShellRole::Launcher
        ) {
            return;
        }
        let registry = self.windows.snapshot();
        if visible {
            self.hidden_shell_roles.remove(&role);
        } else {
            self.hidden_shell_roles.insert(role);
        }
        let window = self.shell_windows().find_map(|window| {
            let id = window
                .wl_surface()
                .and_then(|surface| self.surface_windows.get(&surface.id()))?;
            (registry
                .iter()
                .find(|entry| entry.id == *id)
                .and_then(|entry| ShellRole::from_application_id(&entry.app_id))
                == Some(role))
            .then(|| window.clone())
        });
        let Some(window) = window else {
            return;
        };
        if visible {
            if role == ShellRole::OnScreenKeyboard {
                self.place_on_screen_keyboard_surface(&window);
                return;
            }
            if self.space.elements().any(|mapped| mapped == &window) {
                return;
            }
            let location = self
                .hidden_shell_role_locations
                .remove(&role)
                .unwrap_or_default();
            self.map_buffered_window(window, location, false);
        } else {
            if let Some(location) = self.space.element_location(&window) {
                self.hidden_shell_role_locations.insert(role, location);
            }
            self.space.unmap_elem(&window);
        }
    }

    fn place_on_screen_keyboard_surface(&mut self, window: &Window) {
        let Some(output) = self
            .on_screen_keyboard
            .output_name
            .as_deref()
            .and_then(|name| self.output_geometry_named(name))
            .or_else(|| self.output_geometry_for_shell())
        else {
            return;
        };
        let target = shell_layout::keyboard_area(
            output,
            self.on_screen_keyboard.dock_top,
            self.on_screen_keyboard.height,
        );
        Self::configure_window(window, target);
        let location = Self::shell_surface_location(window, target);
        self.map_buffered_window(window.clone(), location, false);
    }

    fn show_anchored_shell_role(&mut self, role: ShellRole, anchor: ShellPopoverAnchor) {
        use nickel_session_protocol::AnchorSide;

        if !matches!(role, ShellRole::ControlCenter | ShellRole::ProjectMenu) {
            return;
        }
        let Some(output) = self.output_geometry_named(&anchor.output) else {
            tracing::warn!(?role, output = %anchor.output, "popover anchor output disappeared");
            self.set_shell_role_visible(role, false);
            return;
        };
        let registry = self.windows.snapshot();
        let Some(window) = self.shell_windows().find_map(|window| {
            let id = window
                .wl_surface()
                .and_then(|surface| self.surface_windows.get(&surface.id()))?;
            (registry
                .iter()
                .find(|entry| entry.id == *id)
                .and_then(|entry| ShellRole::from_application_id(&entry.app_id))
                == Some(role))
            .then(|| window.clone())
        }) else {
            return;
        };
        let bounds = anchor.bounds;
        let anchor_geometry = match anchor.preferred {
            AnchorSide::Above => Geometry {
                x: output.x + bounds.x,
                y: output.y + output.height - shell_layout::PANEL_HEIGHT + bounds.y,
                width: bounds.width,
                height: bounds.height,
            },
            AnchorSide::Below => Geometry {
                x: output.x + bounds.x,
                y: output.y + bounds.y,
                width: bounds.width,
                height: bounds.height,
            },
            AnchorSide::Left => Geometry {
                x: output.x + bounds.x,
                y: output.y + bounds.y,
                width: bounds.width,
                height: bounds.height,
            },
            AnchorSide::Right => Geometry {
                x: output.x + output.width - shell_layout::PANEL_HEIGHT + bounds.x,
                y: output.y + bounds.y,
                width: bounds.width,
                height: bounds.height,
            },
        };
        let area = match anchor.preferred {
            AnchorSide::Above => shell_layout::work_area(output),
            AnchorSide::Below => Geometry {
                y: output.y + shell_layout::PANEL_HEIGHT,
                height: (output.height - shell_layout::PANEL_HEIGHT).max(0),
                ..output
            },
            AnchorSide::Left | AnchorSide::Right => output,
        };
        let size = window.geometry().size;
        let target = shell_layout::anchored_popover(
            area,
            anchor_geometry,
            (size.w.max(1), size.h.max(1)),
            anchor.preferred,
        );
        Self::configure_window(&window, target);
        let location = Self::shell_surface_location(&window, target);
        self.hidden_shell_roles.remove(&role);
        self.hidden_shell_role_locations
            .insert(role, location.into());
        self.map_buffered_window(window.clone(), location, true);
        self.space.raise_element(&window, true);
        let control = anchor.control.chars().take(80).collect::<String>();
        tracing::debug!(
            ?role,
            %control,
            output = %anchor.output,
            anchor_x = anchor_geometry.x,
            anchor_y = anchor_geometry.y,
            anchor_width = anchor_geometry.width,
            anchor_height = anchor_geometry.height,
            preferred = ?anchor.preferred,
            final_x = target.x,
            final_y = target.y,
            final_width = target.width,
            final_height = target.height,
            "placed anchored shell popover"
        );
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
        if self
            .window_for_registry_id(id)
            .is_some_and(|window| self.is_on_screen_keyboard_window(&window))
        {
            return;
        }
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
            self.map_buffered_window(window, location, true);
        }
        let Some(window) = self.window_for_registry_id(id) else {
            return;
        };
        self.last_interaction_output_name = self.output_name_for_window(&window);
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
            crate::session::focus::KeyboardFocusTarget::for_window(&window),
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
            .filter(|candidate| !self.minimized_windows.contains_key(candidate))
            .filter(|candidate| self.registry_window_is_mapped(*candidate));
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

    fn move_window_to_output(&mut self, id: WindowId, output_name: &str) -> bool {
        let Some(destination) = self.output_geometry_named(output_name) else {
            return false;
        };
        let destination = self.work_area_for_output(destination);
        if let Some((window, location)) = self.minimized_windows.get_mut(&id) {
            *location = clamp_window_location(
                (destination.x, destination.y).into(),
                window.geometry().size,
                destination,
            );
            self.notify_protocol_snapshot();
            return true;
        }
        if let Some((window, location)) = self.workspace_hidden_windows.get_mut(&id) {
            *location = clamp_window_location(
                (destination.x, destination.y).into(),
                window.geometry().size,
                destination,
            );
            self.notify_protocol_snapshot();
            return true;
        }
        let Some(window) = self.window_for_registry_id(id) else {
            return false;
        };
        let location = clamp_window_location(
            (destination.x, destination.y).into(),
            window.geometry().size,
            destination,
        );
        self.map_compositor_moved_window(window, location, false);
        self.notify_protocol_snapshot();
        true
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
                self.map_buffered_window(window, restore.loc, true);
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
        let surface = window.wl_surface()?;
        self.registered_shell_role_slots
            .iter()
            .find(|registration| registration.surface == surface.id())?
            .output
            .clone()
    }

    pub fn relayout_shell_surfaces(&mut self) {
        if self.on_screen_keyboard.visible
            && self
                .on_screen_keyboard
                .output_name
                .as_deref()
                .is_some_and(|name| self.output_geometry_named(name).is_none())
        {
            self.on_screen_keyboard.visible = false;
            self.set_shell_role_visible(ShellRole::OnScreenKeyboard, false);
        }
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
            self.map_buffered_window(desktop, location, false);
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
            if self.keyboard_reserves_output(output) {
                self.space.unmap_elem(&panel);
                continue;
            }
            let geometry = shell_layout::panel(output);
            Self::configure_window(&panel, geometry);
            let location = Self::shell_surface_location(&panel, geometry);
            self.map_buffered_window(panel.clone(), location, false);
            self.space.raise_element(&panel, false);
        }
        if self.launcher_visibility.is_visible()
            && let Some(launcher) = self.launcher_window.clone()
        {
            let geometry = self.launcher_geometry(&launcher);
            let location = Self::shell_surface_location(&launcher, geometry);
            self.map_buffered_window(launcher, location, true);
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
        let hidden = !self.on_screen_keyboard.visible || self.locked;
        let displaced = if hidden {
            std::mem::take(&mut self.on_screen_keyboard.displaced)
        } else {
            self.on_screen_keyboard.displaced.clone()
        };
        for (window, original) in displaced {
            if !window.alive()
                || self.space.element_location(&window).is_none()
                || self.is_fullscreen_window(&window)
                || self.is_maximized_window(&window)
            {
                continue;
            }
            let Some(output) = self.output_geometry_for_window(&window) else {
                continue;
            };
            let geometry = if hidden {
                original
            } else {
                if !self.keyboard_reserves_output(output) {
                    continue;
                }
                shell_layout::fit_keyboard_recipient(
                    original,
                    self.work_area_for_output(output),
                    self.is_server_decorated(&window),
                )
            };
            self.apply_keyboard_window_geometry(&window, geometry);
        }
        self.relayout_maximized_windows();
        self.relayout_fullscreen_windows();
        let windows = self.space.elements().cloned().collect::<Vec<_>>();
        for window in windows {
            self.fit_window_above_keyboard(&window);
        }
        let keyboard = self
            .utility_windows
            .iter()
            .find(|window| self.is_on_screen_keyboard_window(window))
            .cloned();
        if self.on_screen_keyboard.visible
            && !self.locked
            && let Some(window) = keyboard
        {
            self.place_on_screen_keyboard_surface(&window);
        }
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
            self.map_buffered_window(window, (restore.x, restore.y), true);
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
        let output = if self.keyboard_reserves_output(output) {
            self.work_area_for_output(output)
        } else {
            output
        };
        surface.with_pending_state(|state| {
            state
                .states
                .set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen);
            state.size = Some(Size::from((output.width, output.height)));
        });
        window.override_z_index(45);
        self.map_buffered_window(window, (output.x, output.y), true);
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
            self.map_buffered_window(window, (restore.x, restore.y), true);
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
        let bounds = Geometry {
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w,
            height: geometry.size.h,
        };
        let bounds = if self.keyboard_reserves_output(bounds) {
            self.work_area_for_output(bounds)
        } else {
            bounds
        };
        let geometry = smithay::utils::Rectangle::new(
            (bounds.x, bounds.y).into(),
            (bounds.width, bounds.height).into(),
        );
        let _ = surface.set_fullscreen(true);
        let _ = surface.configure(geometry);
        window.override_z_index(45);
        self.map_buffered_window(window, geometry.loc, true);
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
            self.map_buffered_window(window, restore.loc, true);
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
        self.map_buffered_window(window.clone(), geometry.loc, true);
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
            self.map_buffered_window(window.clone(), rectangle.loc, true);
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
            let output = if self.keyboard_reserves_output(output) {
                self.work_area_for_output(output)
            } else {
                output
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

    pub(crate) fn raise_panels(&mut self) {
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

    /// Reconcile every mapped top-level's effective output after Space has sent
    /// the complete enter/leave set. Geometry remains logical; only buffer
    /// preferences change while the client prepares its next commit.
    pub(crate) fn refresh_surface_scales(&mut self) {
        const HYSTERESIS_AREA: u64 = 4_096;
        let outputs = self
            .space
            .outputs()
            .filter_map(|output| {
                let geometry = self.space.output_geometry(output)?;
                Some((
                    nickel_core::dpi::OutputScale {
                        identity: output.name(),
                        geometry: nickel_core::dpi::LogicalRect {
                            x: geometry.loc.x,
                            y: geometry.loc.y,
                            width: geometry.size.w,
                            height: geometry.size.h,
                        },
                        scale: nickel_core::dpi::Scale120::new(
                            (output.current_scale().fractional_scale() * 120.0).round() as u32,
                        )
                        .unwrap_or_default(),
                    },
                    output.clone(),
                ))
            })
            .collect::<Vec<_>>();
        let candidates = outputs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect::<Vec<_>>();
        let active = self.last_interaction_output_name.as_deref();
        let windows = self.space.elements().cloned().collect::<Vec<_>>();
        for window in windows {
            let Some(surface) = window.wl_surface() else {
                continue;
            };
            let Some(bounds) = self.space.element_bbox(&window) else {
                continue;
            };
            let previous = self
                .surface_effective_outputs
                .get(&surface.id())
                .map(String::as_str);
            let selection = nickel_core::dpi::select_effective_output(
                nickel_core::dpi::LogicalRect {
                    x: bounds.loc.x,
                    y: bounds.loc.y,
                    width: bounds.size.w,
                    height: bounds.size.h,
                },
                &candidates,
                previous,
                active,
                HYSTERESIS_AREA,
            );
            let Some(identity) = selection.output else {
                self.surface_effective_outputs.remove(&surface.id());
                continue;
            };
            let Some((candidate, output)) = outputs
                .iter()
                .find(|(candidate, _)| candidate.identity == identity)
            else {
                continue;
            };
            self.surface_effective_outputs
                .insert(surface.id(), identity);
            let integer_scale = candidate.scale.integer_buffer_scale();
            let fractional_scale = candidate.scale.factor();
            let transform = output.current_transform();
            window.with_surfaces(|surface, states| {
                send_surface_state(surface, states, integer_scale, transform);
                with_fractional_scale(states, |state| state.set_preferred_scale(fractional_scale));
            });
            // Popups are separate surface trees rather than descendants visited by
            // `Window::with_surfaces`. Keep their buffer preference locked to the
            // owning top-level while that window crosses an output boundary.
            for (popup, _) in PopupManager::popups_for_surface(&surface) {
                with_states(popup.wl_surface(), |states| {
                    send_surface_state(popup.wl_surface(), states, integer_scale, transform);
                    with_fractional_scale(states, |state| {
                        state.set_preferred_scale(fractional_scale)
                    });
                });
            }
        }

        // A data-device icon has no xdg parent from which to derive an output.
        // The interaction output is the source-side scale and remains stable for
        // the lifetime of the drag even when the pointer straddles two outputs.
        if let Some(icon) = self.dnd_icon.as_ref() {
            let output = active
                .and_then(|name| self.space.outputs().find(|output| output.name() == name))
                .or_else(|| self.space.outputs().next());
            if let Some(output) = output {
                let fractional = output.current_scale().fractional_scale();
                let integer = fractional.ceil().max(1.0) as i32;
                with_states(icon, |states| {
                    send_surface_state(icon, states, integer, output.current_transform());
                    with_fractional_scale(states, |state| state.set_preferred_scale(fractional));
                });
            }
        }
    }

    pub(crate) fn refresh_new_surface_scale(&mut self, surface: &WlSurface) {
        let mut root = self
            .popups
            .find_popup(surface)
            .and_then(|popup| find_popup_root_surface(&popup).ok())
            .unwrap_or_else(|| surface.clone());
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        let identity = self.surface_effective_outputs.get(&root.id()).cloned();
        let output = identity
            .as_deref()
            .and_then(|identity| {
                self.space
                    .outputs()
                    .find(|output| output.name() == identity)
            })
            .cloned()
            .or_else(|| self.space.outputs().next().cloned());
        let Some(output) = output else { return };
        let fractional = output.current_scale().fractional_scale();
        let integer = fractional.ceil().max(1.0) as i32;
        with_states(surface, |states| {
            send_surface_state(surface, states, integer, output.current_transform());
            with_fractional_scale(states, |state| state.set_preferred_scale(fractional));
        });
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
        self.resolve_interaction_output(InvocationSource::Pointer)
    }

    pub(crate) fn keyboard_interaction_output_name(&self) -> Option<String> {
        self.resolve_interaction_output(InvocationSource::Keyboard)
    }

    fn focused_surface_output_name(&self) -> Option<String> {
        let id = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)?
            .id;
        self.output_name_for_window(&self.window_for_registry_id(id)?)
    }

    pub(crate) fn output_name_for_window(&self, window: &Window) -> Option<String> {
        let geometry = self.output_geometry_for_window(window)?;
        self.space.outputs().find_map(|output| {
            let candidate = self.space.output_geometry(output)?;
            (candidate.loc.x == geometry.x
                && candidate.loc.y == geometry.y
                && candidate.size.w == geometry.width
                && candidate.size.h == geometry.height)
                .then(|| output.name())
        })
    }

    fn resolve_interaction_output(&self, source: InvocationSource) -> Option<String> {
        let enabled = self
            .space
            .outputs()
            .map(|output| output.name())
            .collect::<Vec<_>>();
        let enabled_refs = enabled.iter().map(String::as_str).collect::<Vec<_>>();
        let pointer = self.output_name_at_pointer();
        let focused = self.focused_surface_output_name();
        resolve_active_output(
            source,
            ActiveOutputContext {
                pointer: pointer.as_deref(),
                focused_surface: focused.as_deref(),
                recent_interaction: self
                    .last_interaction_output_name
                    .as_deref()
                    .or_else(|| self.workspaces.active_output()),
                primary: self.primary_output_name.as_deref(),
                enabled: &enabled_refs,
            },
        )
    }

    pub(crate) fn new_window_active_output_name(&self) -> Option<String> {
        let focused_surface = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active && !self.shell_owned_windows.contains(&window.id))
            .and_then(|window| self.window_for_registry_id(window.id))
            .and_then(|window| self.output_name_for_window(&window));
        let enabled = self
            .space
            .outputs()
            .map(|output| output.name())
            .collect::<Vec<_>>();
        let enabled_refs = enabled.iter().map(String::as_str).collect::<Vec<_>>();
        let pointer = self.output_name_at_pointer();
        resolve_new_window_output(ActiveOutputContext {
            pointer: pointer.as_deref(),
            focused_surface: focused_surface.as_deref(),
            recent_interaction: self
                .last_interaction_output_name
                .as_deref()
                .or_else(|| self.workspaces.active_output()),
            primary: self.primary_output_name.as_deref(),
            enabled: &enabled_refs,
        })
    }

    pub(crate) fn placement_outputs(&self) -> Vec<shell_layout::PlacementOutput> {
        let primary = self.primary_output_name.as_deref();
        self.space
            .outputs()
            .filter_map(|output| {
                let geometry = self.space.output_geometry(output)?;
                let geometry = Geometry {
                    x: geometry.loc.x,
                    y: geometry.loc.y,
                    width: geometry.size.w,
                    height: geometry.size.h,
                };
                Some(shell_layout::PlacementOutput {
                    name: output.name(),
                    work_area: self.work_area_for_output(geometry),
                    primary: primary == Some(output.name().as_str()),
                })
            })
            .collect()
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

    fn keyboard_reserves_output(&self, output: Geometry) -> bool {
        self.on_screen_keyboard.visible
            && !self.locked
            && self
                .on_screen_keyboard
                .output_name
                .as_deref()
                .and_then(|name| self.output_geometry_named(name))
                == Some(output)
    }

    fn panel_hidden_by_keyboard(&self, window: &Window) -> bool {
        self.panel_windows.contains(window)
            && self
                .shell_surface_output_name(window)
                .as_deref()
                .and_then(|name| self.output_geometry_named(name))
                .is_some_and(|output| self.keyboard_reserves_output(output))
    }

    fn work_area_for_output(&self, output: Geometry) -> Geometry {
        if self.keyboard_reserves_output(output) {
            shell_layout::keyboard_work_area(
                output,
                self.on_screen_keyboard.dock_top,
                self.on_screen_keyboard.height,
            )
        } else {
            shell_layout::work_area(output)
        }
    }

    fn apply_keyboard_window_geometry(&mut self, window: &Window, geometry: Geometry) {
        Self::configure_window(window, geometry);
        if let Some(surface) = window.x11_surface() {
            let _ = surface.configure(smithay::utils::Rectangle::new(
                (geometry.x, geometry.y).into(),
                (geometry.width, geometry.height).into(),
            ));
        }
        self.space
            .map_element(window.clone(), (geometry.x, geometry.y), false);
    }

    pub(crate) fn fit_window_above_keyboard(&mut self, window: &Window) {
        if self.is_shell_owned_window(window)
            || self.is_fullscreen_window(window)
            || self.is_maximized_window(window)
        {
            return;
        }
        let Some(output) = self.output_geometry_for_window(window) else {
            return;
        };
        if !self.keyboard_reserves_output(output) {
            return;
        }
        let Some(location) = self.space.element_location(window) else {
            return;
        };
        let size = window.geometry().size;
        let content = Geometry {
            x: location.x,
            y: location.y,
            width: size.w,
            height: size.h,
        };
        let target = shell_layout::fit_keyboard_recipient(
            content,
            self.work_area_for_output(output),
            self.is_server_decorated(window),
        );
        if target != content {
            if !self
                .on_screen_keyboard
                .displaced
                .iter()
                .any(|(saved, _)| saved == window)
            {
                self.on_screen_keyboard
                    .displaced
                    .push((window.clone(), content));
            }
            self.apply_keyboard_window_geometry(window, target);
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
                    crate::session::handlers::portal_capture_pid_allowed(credentials.pid())
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

    /// Whether the ordinary client scene occupies `pos`, including the
    /// compositor-owned portion of a server-decorated window.
    ///
    /// Server-side titlebars live outside their client's input surface.  They
    /// must nevertheless mask the internal desktop, which is rendered and hit
    /// tested below ordinary windows.  Foreground internal surfaces still get
    /// their usual priority; this value is only used to make the desktop yield.
    pub(crate) fn client_scene_under(&self, pos: Point<f64, Logical>) -> bool {
        let frame = self.space.elements().rev().find_map(|window| {
            if (self.locked && !self.lock_windows.contains(window))
                || self.shell_windows().any(|shell| shell == window)
                || self.is_fullscreen_window(window)
                || !self.is_server_decorated(window)
            {
                return None;
            }
            let bounds = self.space.element_geometry(window)?;
            crate::session::window_frame::hit_test(
                crate::session::shell_layout::Geometry {
                    x: bounds.loc.x,
                    y: bounds.loc.y,
                    width: bounds.size.w,
                    height: bounds.size.h,
                },
                pos.x.round() as i32,
                pos.y.round() as i32,
            )
        });
        crate::session::window_frame::client_scene_occupies(
            self.surface_under(pos).is_some(),
            frame,
        )
    }

    pub fn pointer_surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(
        crate::session::focus::PointerFocusTarget,
        Point<f64, Logical>,
    )> {
        self.space
            .element_under(pos)
            .filter(|(window, _)| !self.locked || self.lock_windows.contains(window))
            .and_then(|(window, location)| {
                window
                    .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(surface, origin)| {
                        let target = window.x11_surface().map_or_else(
                            || crate::session::focus::PointerFocusTarget::Wayland(surface),
                            |x11| crate::session::focus::PointerFocusTarget::X11(x11.clone()),
                        );
                        (target, (origin + location).to_f64())
                    })
            })
    }
}

fn internal_shell_surface_placement(
    surface_role: crate::winit_shell::SurfaceRole,
    surface_output: Option<&str>,
    surface_size: (u32, u32),
    outputs: &[(crate::internal_shell::InternalOutput, i32, i32)],
    launcher_output: Option<&str>,
) -> crate::session::InternalSurfacePlacement {
    use crate::{session::InternalSurfaceRole, winit_shell::SurfaceRole};

    let requested_output = if surface_role == SurfaceRole::Launcher {
        launcher_output
    } else {
        surface_output
    };
    let selected = requested_output
        .and_then(|name| outputs.iter().find(|(output, _, _)| output.name == name))
        .or_else(|| outputs.first());
    let (output_name, origin_x, origin_y, output_width, output_height) = selected
        .map(|(output, x, y)| {
            (
                Some(output.name.clone()),
                *x,
                *y,
                output.width,
                output.height,
            )
        })
        .unwrap_or((None, 0, 0, surface_size.0, surface_size.1));

    let (x, y) = match surface_role {
        SurfaceRole::Panel => (
            origin_x,
            origin_y + output_height.saturating_sub(crate::winit_shell::PANEL_HEIGHT) as i32,
        ),
        SurfaceRole::Launcher => {
            let work_height = output_height.saturating_sub(crate::winit_shell::PANEL_HEIGHT);
            let x_margin = 18.min(output_width.saturating_sub(surface_size.0)) as i32;
            let y_margin = 8.min(work_height.saturating_sub(surface_size.1)) as i32;
            (
                origin_x + x_margin,
                origin_y + work_height.saturating_sub(surface_size.1) as i32 - y_margin,
            )
        }
        _ => (origin_x, origin_y),
    };
    let role = match surface_role {
        SurfaceRole::Desktop => InternalSurfaceRole::Desktop,
        SurfaceRole::Panel => InternalSurfaceRole::Panel,
        _ => InternalSurfaceRole::Overlay,
    };
    crate::session::InternalSurfacePlacement {
        role,
        geometry: (x, y, surface_size.0, surface_size.1),
        output: output_name,
    }
}

fn internal_codex_project_menu_placement(
    anchor: Option<&nickel_session_protocol::ShellPopoverAnchor>,
    outputs: &[(crate::internal_shell::InternalOutput, i32, i32)],
    fallback_output: Option<&str>,
) -> crate::internal_codex::CodexSurfacePlacement {
    let requested = anchor
        .map(|anchor| anchor.output.as_str())
        .or(fallback_output);
    let selected = requested
        .and_then(|name| outputs.iter().find(|(output, _, _)| output.name == name))
        .or_else(|| outputs.first());
    let Some((output, origin_x, origin_y)) = selected else {
        return crate::internal_codex::CodexSurfacePlacement::default();
    };
    let (menu_width, menu_height) = crate::internal_codex::MENU_SIZE;
    let max_x = output.width.saturating_sub(menu_width) as i32;
    let anchor_center = anchor
        .filter(|anchor| anchor.output == output.name)
        .map_or(24, |anchor| anchor.bounds.x + anchor.bounds.width / 2);
    let x = (anchor_center - menu_width as i32 / 2).clamp(0, max_x);
    let work_height = output
        .height
        .saturating_sub(crate::winit_shell::PANEL_HEIGHT);
    let y = work_height.saturating_sub(menu_height).saturating_sub(8) as i32;
    crate::internal_codex::CodexSurfacePlacement {
        output: Some(output.name.clone()),
        origin: (origin_x + x, origin_y + y),
        scale: output.scale,
    }
}

fn internal_codex_chat_placement(
    outputs: &[(crate::internal_shell::InternalOutput, i32, i32)],
    requested_output: Option<&str>,
) -> crate::internal_codex::CodexSurfacePlacement {
    let selected = requested_output
        .and_then(|name| outputs.iter().find(|(output, _, _)| output.name == name))
        .or_else(|| outputs.first());
    let Some((output, origin_x, origin_y)) = selected else {
        return crate::internal_codex::CodexSurfacePlacement::default();
    };
    let (content_width, content_height) = crate::internal_codex::CHAT_SIZE;
    let border = crate::session::window_frame::RESIZE_BORDER.max(0) as u32;
    let titlebar = crate::session::window_frame::TITLEBAR_HEIGHT.max(0) as u32;
    let outer_width = content_width.saturating_add(border.saturating_mul(2));
    let outer_height = content_height
        .saturating_add(titlebar)
        .saturating_add(border.saturating_mul(2));
    let work_height = output
        .height
        .saturating_sub(crate::winit_shell::PANEL_HEIGHT);
    let outer_x = output.width.saturating_sub(outer_width) / 2;
    let outer_y = work_height.saturating_sub(outer_height) / 2;
    crate::internal_codex::CodexSurfacePlacement {
        output: Some(output.name.clone()),
        origin: (
            origin_x + outer_x as i32 + border as i32,
            origin_y + outer_y as i32 + titlebar as i32 + border as i32,
        ),
        scale: output.scale,
    }
}

fn locale_is_rtl() -> bool {
    std::env::var("LC_ALL")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var("LANG").ok())
        .is_some_and(|locale| {
            ["ar", "fa", "he", "ur"].iter().any(|language| {
                locale == *language
                    || locale.starts_with(&format!("{language}_"))
                    || locale.starts_with(&format!("{language}-"))
            })
        })
}

fn maximized_content_geometry(frame: Geometry, server_decorated: bool) -> Geometry {
    if server_decorated {
        Geometry {
            x: frame.x + crate::session::window_frame::RESIZE_BORDER,
            y: frame.y
                + crate::session::window_frame::TITLEBAR_HEIGHT
                + crate::session::window_frame::RESIZE_BORDER,
            width: (frame.width - crate::session::window_frame::RESIZE_BORDER * 2).max(1),
            height: (frame.height
                - crate::session::window_frame::TITLEBAR_HEIGHT
                - crate::session::window_frame::RESIZE_BORDER * 2)
                .max(1),
        }
    } else {
        frame
    }
}

fn clamp_decorated_content_to_work_area(content: Geometry, work_area: Geometry) -> Geometry {
    let outer = crate::session::window_frame::outer_geometry(content);
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
        crate::session::window_frame::outer_geometry(current_content)
    } else {
        current_content
    };
    let restored_outer_size = if server_decorated {
        crate::session::window_frame::outer_geometry(Geometry {
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
    let titlebar_offset = (pointer.y - f64::from(current_outer.y)).clamp(
        0.0,
        f64::from(crate::session::window_frame::TITLEBAR_HEIGHT.max(1)),
    );
    let minimum_visible = 32.min(restored_outer_size.width.max(1));
    let outer_x = (pointer.x - horizontal * f64::from(restored_outer_size.width)).round() as i32;
    let outer_y = (pointer.y - titlebar_offset).round() as i32;
    let outer_x = outer_x.clamp(
        work_area.x - restored_outer_size.width + minimum_visible,
        work_area.x + work_area.width - minimum_visible,
    );
    let outer_y = outer_y.clamp(
        work_area.y,
        work_area.y + work_area.height - crate::session::window_frame::TITLEBAR_HEIGHT.max(1),
    );

    if server_decorated {
        Geometry {
            x: outer_x + crate::session::window_frame::RESIZE_BORDER,
            y: outer_y
                + crate::session::window_frame::TITLEBAR_HEIGHT
                + crate::session::window_frame::RESIZE_BORDER,
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
        if let Some(control) = &self.compatibility_control {
            let _ = std::fs::remove_file(&control.socket_path);
            crate::model::clear_trusted_session_capability(control.socket_path.as_os_str());
            if std::env::var_os("NICKEL_SESSION_CONTROL").as_deref()
                == Some(control.socket_path.as_os_str())
            {
                // SAFETY: production owns one session; serialized session tests
                // also tear down the capability they installed.
                unsafe {
                    std::env::remove_var("NICKEL_SESSION_CONTROL");
                    if std::env::var("NICKEL_SESSION_TOKEN").as_deref()
                        == Ok(control.protocol_token.as_str())
                    {
                        std::env::remove_var("NICKEL_SESSION_TOKEN");
                    }
                    std::env::remove_var("NICKEL_SHELL_TEST_CONTROL");
                }
            }
        }
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
        DisplacedWindow, PREVIEW_BYTE_CAPACITY, PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER,
        PREVIEW_ENTRY_CAPACITY, PREVIEW_FRAME_BYTES, PendingLaunchObservation,
        PendingLaunchWindowDisposition, RegisteredShellRole, ShellRegistrationRejection,
        admitted_preview_ids, advance_preview_content_generation, apply_shell_behavior_value,
        bounded_preview_ids, clamp_decorated_content_to_work_area, clamp_window_location,
        command_requires_shell_identity, drag_icon_location, identification_expiry_is_current,
        maximized_content_geometry, output_contains_logical_point, output_index_for_shell_surface,
        pending_launch_window_disposition, prepare_shell_behavior_update,
        preview_mapping_has_exact_size, protocol_preview_from_cached,
        record_preview_capture_attempt, restored_drag_content_geometry,
        retain_live_idle_inhibitors, retire_displaced_window, retire_pointer_surface,
        retire_shell_surface, reuse_preview_pixels, shell_behavior_value,
        shell_registration_is_active, shell_registration_rejection,
        shell_registration_role_changed, shell_role_accepts_ordinary_focus,
        test_control_may_invoke,
    };
    use crate::session::output_retirement::{
        BIND_SETTLE_GRACE as OUTPUT_GLOBAL_BIND_SETTLE_GRACE,
        DISABLED_GRACE as OUTPUT_GLOBAL_DISABLED_GRACE,
        MAX_PENDING as MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS,
    };
    use crate::session::shell_layout::Geometry;
    use crate::{
        platform::{SessionRequestError, ShellCommand},
        session_host::SessionHost,
    };
    use nickel_session_protocol::{
        Command, OutputTransform, Query, ServerEnvelope, ServerMessage, SessionAction,
        ShellBehaviorSetting, ShellBehaviorTransaction, ShellBehaviorValue, ShellRole, TestOutput,
    };
    use smithay::{
        output::{Output, PhysicalProperties, Subpixel},
        reexports::{
            calloop::EventLoop,
            wayland_server::{Display, backend::ObjectId},
        },
        utils::Point,
    };
    use std::time::{Duration, Instant};
    use std::{
        collections::{HashMap, HashSet},
        sync::Arc,
    };

    #[test]
    fn late_window_callback_leaves_expiry_owned_by_the_registered_timer() {
        let now = Instant::now();
        let pending = PendingLaunchObservation {
            generation: 7,
            root_pid: std::process::id(),
            root_start_time: 1,
            registered_at: now - Duration::from_millis(101),
            deadline: Duration::from_millis(100),
        };

        assert_eq!(
            pending_launch_window_disposition(&pending, now, std::process::id()),
            PendingLaunchWindowDisposition::AwaitExpiry
        );
    }

    #[test]
    fn shell_behavior_values_are_typed_and_desktop_counts_are_validated() {
        let mut settings = nickel_core::shell_settings::ShellSettings::default();
        assert_eq!(
            shell_behavior_value(&settings, ShellBehaviorSetting::BarDisplayScope),
            ShellBehaviorValue::Toggle(true)
        );
        assert!(
            apply_shell_behavior_value(
                &mut settings,
                ShellBehaviorSetting::BarWindowScope,
                ShellBehaviorValue::Toggle(false),
            )
            .is_ok()
        );
        assert!(!settings.all_windows_on_every_bar);
        assert!(
            apply_shell_behavior_value(
                &mut settings,
                ShellBehaviorSetting::DesktopCount,
                ShellBehaviorValue::Count(0),
            )
            .is_err()
        );
        assert!(
            apply_shell_behavior_value(
                &mut settings,
                ShellBehaviorSetting::BarDisplayScope,
                ShellBehaviorValue::Count(2),
            )
            .is_err()
        );
    }

    #[test]
    fn shell_behavior_transactions_reject_stale_topology_and_concurrent_writers() {
        let current = nickel_core::shell_settings::ShellSettings::default();
        let mut transaction = ShellBehaviorTransaction {
            setting: ShellBehaviorSetting::BarDisplayScope,
            prior: ShellBehaviorValue::Toggle(true),
            requested: ShellBehaviorValue::Toggle(false),
            topology_generation: 4,
        };
        assert_eq!(
            prepare_shell_behavior_update(&current, 5, &transaction),
            Err("stale output topology generation")
        );
        transaction.topology_generation = 5;
        transaction.prior = ShellBehaviorValue::Toggle(false);
        assert_eq!(
            prepare_shell_behavior_update(&current, 5, &transaction),
            Err("shell setting changed before this transaction was applied")
        );
        transaction.prior = ShellBehaviorValue::Toggle(true);
        let requested = prepare_shell_behavior_update(&current, 5, &transaction).unwrap();
        assert!(!requested.bar_on_all_displays);
        assert!(
            current.bar_on_all_displays,
            "planning must not mutate authority"
        );
    }

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
    fn disconnected_output_roles_are_dormant_until_the_output_returns() {
        let registrations = [
            RegisteredShellRole {
                role: ShellRole::Desktop,
                output: Some("winit".into()),
                surface: ObjectId::null(),
            },
            RegisteredShellRole {
                role: ShellRole::Desktop,
                output: Some("DP-test".into()),
                surface: ObjectId::null(),
            },
            RegisteredShellRole {
                role: ShellRole::Panel,
                output: Some("DP-test".into()),
                surface: ObjectId::null(),
            },
            RegisteredShellRole {
                role: ShellRole::Lock,
                output: Some("DP-test".into()),
                surface: ObjectId::null(),
            },
        ];
        let connected = HashSet::from(["winit".to_owned(), "DP-test".to_owned()]);
        assert!(registrations.iter().all(|registration| {
            shell_registration_is_active(registration, &connected, &connected)
        }));

        let after_disconnect = HashSet::from(["winit".to_owned()]);
        assert!(shell_registration_is_active(
            &registrations[0],
            &after_disconnect,
            &after_disconnect,
        ));
        assert!(registrations[1..].iter().all(|registration| {
            !shell_registration_is_active(registration, &after_disconnect, &after_disconnect)
        }));

        // Keeping the slots dormant preserves the shell's bounded reconnect
        // grace: the same native surfaces become authoritative again if the
        // named output returns instead of requiring an app-id transition.
        assert!(registrations.iter().all(|registration| {
            shell_registration_is_active(registration, &connected, &connected)
        }));
    }

    #[test]
    fn panel_registration_tracks_the_configured_output_set() {
        let registration = RegisteredShellRole {
            role: ShellRole::Panel,
            output: Some("DP-test".into()),
            surface: ObjectId::null(),
        };
        let connected = HashSet::from(["winit".to_owned(), "DP-test".to_owned()]);
        let primary_only = HashSet::from(["winit".to_owned()]);
        assert!(!shell_registration_is_active(
            &registration,
            &connected,
            &primary_only,
        ));
        assert!(shell_registration_is_active(
            &registration,
            &connected,
            &connected,
        ));
    }

    #[test]
    fn locked_test_output_disconnect_projects_readiness_to_live_topology() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        session
            .apply_test_output(TestOutput::Connect {
                name: "DP-test".into(),
                logical_width: 640,
                logical_height: 480,
                scale_120: 120,
                transform: OutputTransform::Normal,
            })
            .unwrap();
        for role in [ShellRole::Desktop, ShellRole::Panel, ShellRole::Lock] {
            session
                .registered_shell_role_slots
                .push(RegisteredShellRole {
                    role,
                    output: Some("DP-test".into()),
                    surface: ObjectId::null(),
                });
        }
        let connected = session.protocol_shell_readiness();
        assert_eq!((connected.outputs, connected.desktops), (1, 1));
        assert_eq!((connected.panels, connected.locks), (1, 1));

        session.locked = true;
        session
            .apply_test_output(TestOutput::Disconnect {
                name: "DP-test".into(),
            })
            .unwrap();

        let disconnected = session.protocol_shell_readiness();
        assert_eq!((disconnected.outputs, disconnected.desktops), (0, 0));
        assert_eq!((disconnected.panels, disconnected.locks), (0, 0));
        assert!(session.registered_shell_role_slots.len() == 3);
    }

    #[test]
    fn keyboard_reservation_resize_and_close_change_only_the_owner_output() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        for name in ["keyboard-owner", "unaffected"] {
            session
                .apply_test_output(TestOutput::Connect {
                    name: name.into(),
                    logical_width: 1280,
                    logical_height: 720,
                    scale_120: 120,
                    transform: OutputTransform::Normal,
                })
                .unwrap();
        }
        session.configure_on_screen_keyboard(true, true, 0, false, false, 368);
        let outputs = session.protocol_outputs();
        let owner = session.on_screen_keyboard.output_name.clone().unwrap();
        for output in &outputs {
            assert_eq!(
                output.work_area.height,
                if output.name == owner { 352 } else { 664 }
            );
        }
        session.configure_on_screen_keyboard(true, true, 0, false, true, 280);
        assert_eq!(
            session.on_screen_keyboard.output_name.as_deref(),
            Some(owner.as_str())
        );
        for output in session.protocol_outputs() {
            assert_eq!(
                output.work_area.height,
                if output.name == owner { 440 } else { 664 }
            );
            assert_eq!(
                output.work_area.y,
                if output.name == owner { 280 } else { 0 }
            );
        }
        session.configure_on_screen_keyboard(true, false, 0, false, true, 280);
        assert!(
            session
                .protocol_outputs()
                .iter()
                .all(|output| output.work_area.height == 664)
        );
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

    struct IdleInternalHost;

    impl SessionHost for IdleInternalHost {
        fn dispatch(&self, _command: ShellCommand) -> Result<(), SessionRequestError> {
            Ok(())
        }

        fn secure_storage_state(
            &self,
        ) -> Result<crate::platform::SecureStorageState, SessionRequestError> {
            Ok(crate::platform::SecureStorageState::Ready)
        }

        fn request_secure_storage_retry(&self) -> Result<(), SessionRequestError> {
            Ok(())
        }
    }

    #[test]
    fn unchanged_internal_desktop_has_no_sixty_hertz_poll_or_redraw_loop() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (mut event_loop, mut session) = preview_test_session();
        let (_system_tx, system_rx) = std::sync::mpsc::channel();
        session
            .enable_internal_shell_with_system_updates(Arc::new(IdleInternalHost), system_rx)
            .expect("headless internal shell");

        // Consume the intentionally immediate initialization wakeup. The
        // stable shell then owns a real application deadline well beyond a
        // frame interval (keyboard discovery currently supplies the nearest).
        event_loop
            .dispatch(Duration::from_millis(25), &mut session)
            .unwrap();
        let settled = session.internal_shell_timer_counters();

        // Damage/state paths may redundantly ask to maintain the schedule.
        // Sixty such calls must keep the one existing one-shot instead of
        // manufacturing a 60 Hz timer or redraw stream.
        for _ in 0..60 {
            session.schedule_internal_shell_deadline();
        }
        let after_rearm = session.internal_shell_timer_counters();
        assert_eq!(after_rearm.armed, settled.armed);
        assert_eq!(after_rearm.polls, settled.polls);
        assert_eq!(after_rearm.redraw_requests, settled.redraw_requests);

        event_loop
            .dispatch(Duration::from_millis(75), &mut session)
            .unwrap();
        let after_idle = session.internal_shell_timer_counters();
        assert!(after_idle.polls.saturating_sub(settled.polls) <= 1);
        assert_eq!(after_idle.redraw_requests, settled.redraw_requests);
    }

    #[test]
    fn applying_multi_output_fractional_scale_rebuilds_internal_surfaces_at_native_scale() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        for (name, width, height) in [("high", 1200, 900), ("normal", 1000, 800)] {
            session
                .apply_test_output(TestOutput::Connect {
                    name: name.into(),
                    logical_width: width,
                    logical_height: height,
                    scale_120: 120,
                    transform: OutputTransform::Normal,
                })
                .unwrap();
        }
        session
            .enable_internal_shell(Arc::new(IdleInternalHost))
            .unwrap();

        session
            .apply_output_layout(nickel_session_protocol::OutputLayout {
                primary: "high".into(),
                placements: vec![
                    nickel_session_protocol::OutputPlacement {
                        name: "high".into(),
                        x: 0,
                        y: 0,
                        enabled: true,
                        scale_120: 180,
                    },
                    nickel_session_protocol::OutputPlacement {
                        name: "normal".into(),
                        x: 800,
                        y: 0,
                        enabled: true,
                        scale_120: 120,
                    },
                ],
            })
            .unwrap();

        let outputs = session.protocol_outputs();
        let high = outputs.iter().find(|output| output.name == "high").unwrap();
        let normal = outputs
            .iter()
            .find(|output| output.name == "normal")
            .unwrap();
        assert_eq!((high.geometry.width, high.scale_120), (800, 180));
        assert_eq!((normal.geometry.x, normal.scale_120), (800, 120));

        let shell = session.internal_shell.as_ref().unwrap();
        for (name, expected) in [("high", 1.5_f32), ("normal", 1.0_f32)] {
            let surface = shell
                .surface(crate::winit_shell::SurfaceRole::Desktop, Some(name))
                .unwrap();
            let runtime = session.internal_shell_surfaces[&surface.id];
            assert_eq!(session.internal_ui.scale_factor(runtime), Some(expected));
        }
    }

    #[test]
    fn fractional_scale_is_published_with_required_viewporter_protocol() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, session) = preview_test_session();

        // Retaining both global handles is the Smithay contract that keeps the
        // paired protocols advertised for the session lifetime.
        assert_ne!(
            session.fractional_scale_manager_state.global(),
            session.viewporter_state.global()
        );
    }

    #[test]
    fn ordinary_session_exposes_a_restricted_settings_adapter_without_pid_authority() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let mut event_loop = EventLoop::try_new().unwrap();
        let display = Display::new().unwrap();
        let session = super::NickelSession::new(&mut event_loop, display, false);

        let control = session.compatibility_control.as_ref().unwrap();
        assert!(control.socket_path.exists());
        assert_eq!(control.protocol_token.len(), 64);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&control.socket_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(!session.is_authenticated_shell_pid(std::process::id()));
    }

    #[test]
    fn explicit_test_control_owns_compatibility_pid_state() {
        let (_event_loop, session) = preview_test_session();
        let control = session
            .compatibility_control
            .as_ref()
            .expect("test control should install the compatibility adapter");

        assert_ne!(control.protocol_token, "");
        assert_eq!(control.expected_shell_pid, 0);
        assert!(control.authenticated_shell_pids.is_empty());
        assert!(control.socket_path.exists());
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
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.preview_admitted.insert(id);
        session.preview_frames.insert(
            id,
            super::PreviewFrame {
                width: 75,
                height: super::PREVIEW_HEIGHT as u16,
                rgba: vec![41; 75 * super::PREVIEW_HEIGHT * 4],
            },
        );
        let allocation = session.preview_frames[&id].rgba.as_ptr();

        let (rgba, had_frame) = session.take_preview_capture_buffer(id);
        session.preview_capture_failed(id, rgba, had_frame);

        assert_eq!(session.preview_frames[&id].rgba.as_ptr(), allocation);
        assert_eq!(session.preview_frames[&id].rgba[0], 41);
        assert_eq!(session.preview_frames[&id].width, 75);
        assert_eq!(session.preview_frames[&id].height, 135);
        assert_eq!(session.preview_counters.evictions, 0);
        assert_eq!(session.preview_counters.capture_failures, 1);
    }

    #[test]
    fn fitted_preview_frame_is_stored_at_its_actual_dimensions() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let id = session
            .windows
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.preview_admitted.insert(id);
        let width = 210;
        let height = super::PREVIEW_HEIGHT as u16;

        session.store_preview(
            id,
            super::PreviewFrame {
                width,
                height,
                rgba: vec![17; usize::from(width) * usize::from(height) * 4],
            },
        );

        let stored = &session.preview_frames[&id];
        assert_eq!((stored.width, stored.height), (width, height));
        assert_eq!(stored.rgba.len(), 113_400);
        assert_eq!(session.preview_counters.captures, 1);
    }

    #[test]
    fn fourteen_first_capture_failures_retain_exactly_the_declared_capacity() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let ids = (0..PREVIEW_ENTRY_CAPACITY)
            .map(|_| {
                session
                    .windows
                    .insert(crate::session::window_registry::WindowAdmission::Ordinary)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        session.preview_admitted.extend(ids.iter().copied());
        for id in ids {
            let (rgba, had_frame) = session.take_preview_capture_buffer(id);
            assert!(had_frame.is_none());
            session.preview_capture_failed(id, rgba, None);
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
        assert!(!preview_mapping_has_exact_size(
            &vec![0; PREVIEW_FRAME_BYTES - 1],
            super::PREVIEW_WIDTH as u16,
            super::PREVIEW_HEIGHT as u16,
        ));
        assert_eq!(pixels.as_ptr(), allocation);
        assert_eq!(pixels[0], 23);
    }

    #[test]
    fn preview_capture_dimensions_are_bounded_and_preserve_window_aspect() {
        for (source, expected) in [
            ((3440, 1440), (240, 100)),
            ((1920, 1080), (240, 135)),
            ((1600, 1200), (180, 135)),
            ((1000, 1000), (135, 135)),
            ((900, 1600), (75, 135)),
            ((1919, 1079), (240, 134)),
        ] {
            let fitted = super::preview_capture_dimensions(source.0, source.1)
                .unwrap_or_else(|| panic!("positive source {source:?} has capture dimensions"));
            assert_eq!(fitted, expected, "source {source:?}");
            assert!(usize::from(fitted.0) <= super::PREVIEW_WIDTH);
            assert!(usize::from(fitted.1) <= super::PREVIEW_HEIGHT);
            if usize::from(fitted.0) == super::PREVIEW_WIDTH {
                let ideal_height = source.1 as f64 * f64::from(fitted.0) / source.0 as f64;
                assert!((ideal_height - f64::from(fitted.1)).abs() <= 1.0);
            } else {
                assert_eq!(usize::from(fitted.1), super::PREVIEW_HEIGHT);
                let ideal_width = source.0 as f64 * f64::from(fitted.1) / source.1 as f64;
                assert!((ideal_width - f64::from(fitted.0)).abs() <= 1.0);
            }
        }
        assert_eq!(super::preview_capture_dimensions(0, 1080), None);
        assert_eq!(super::preview_capture_dimensions(1920, -1), None);
    }

    #[test]
    fn stale_retry_epoch_cannot_consume_new_generation_pending_work() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (mut event_loop, mut session) = preview_test_session();
        let old = session
            .windows
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.preview_retry_pending.insert(old);
        session.schedule_preview_retry();
        let stale_epoch = session.preview_retry_scheduled.unwrap();

        session.clear_all_previews();
        let current = session
            .windows
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
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
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
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
                    .insert(crate::session::window_registry::WindowAdmission::Ordinary)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let overlay = (0..7)
            .map(|_| {
                session
                    .windows
                    .insert(crate::session::window_registry::WindowAdmission::Ordinary)
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
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
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
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
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
            Command::ObservePendingLaunch {
                generation: 7,
                root_pid: 42,
                root_start_time: 99,
                deadline_ms: 100,
            },
            Command::CancelPendingLaunch { generation: 7 },
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
    fn process_lineage_accepts_self_and_a_live_descendant_only() {
        let start = super::linux_process_start_time(std::process::id()).unwrap();
        assert!(super::process_descends_from(
            std::process::id(),
            std::process::id(),
            start,
        ));
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 2"])
            .spawn()
            .expect("spawn lineage fixture");
        assert!(super::process_descends_from(
            child.id(),
            std::process::id(),
            start,
        ));
        assert!(!super::process_descends_from(
            child.id(),
            std::process::id(),
            start.wrapping_add(1),
        ));
        assert!(!super::process_descends_from(
            std::process::id(),
            child.id(),
            super::linux_process_start_time(child.id()).unwrap(),
        ));
        let _ = child.kill();
        let _ = child.wait();
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
    fn explicit_nested_test_control_can_cross_lock_and_logout_boundaries() {
        assert!(test_control_may_invoke(&Command::LogOut));
        assert!(test_control_may_invoke(&Command::Unlock));
        assert!(test_control_may_invoke(&Command::SessionAction {
            action: SessionAction::Lock,
        }));
        assert!(!test_control_may_invoke(&Command::SessionAction {
            action: SessionAction::PowerOff,
        }));
    }

    #[test]
    fn surface_identity_registration_requires_the_authenticated_shell() {
        assert!(command_requires_shell_identity(
            &Command::RegisterShellSurface {
                identity: nickel_session_protocol::ShellSurfaceIdentity {
                    application_id: "io.nickel.shell.surface.42.1".into(),
                    role: ShellRole::Desktop,
                    output: Some("DP-1".into()),
                },
            }
        ));
    }

    #[test]
    fn disconnected_surfaces_stop_inhibiting_idle_policy() {
        let mut inhibitors = HashMap::from([("alive", 2), ("disconnected", 1)]);
        retain_live_idle_inhibitors(&mut inhibitors, |surface| *surface == "alive");
        assert_eq!(inhibitors, HashMap::from([("alive", 2)]));
    }

    #[test]
    fn descriptive_output_names_resolve_to_authoritative_connector_names() {
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

        assert_eq!(
            crate::session::window_frame::outer_geometry(content),
            work_area
        );
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

        let outer = crate::session::window_frame::outer_geometry(content);
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
        let outer = crate::session::window_frame::outer_geometry(geometry);

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

    #[test]
    fn transient_output_hit_testing_uses_both_global_axes() {
        let upper = smithay::utils::Rectangle::new((0, -1080).into(), (1920, 1080).into());
        let lower = smithay::utils::Rectangle::new((0, 0).into(), (1920, 1080).into());

        assert!(output_contains_logical_point(upper, 960, -40));
        assert!(!output_contains_logical_point(lower, 960, -40));
        assert!(!output_contains_logical_point(upper, 960, 1040));
        assert!(output_contains_logical_point(lower, 960, 1040));
    }
}
