#[cfg(unix)]
pub mod client;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const PROTOCOL_VERSION: u16 = 27;
pub const MAX_FRAME_BYTES: usize = 196_608;
pub const MAX_PREVIEW_WIDTH: u16 = 256;
pub const MAX_PREVIEW_HEIGHT: u16 = 144;
pub const MAX_SUBSCRIBERS: usize = 8;
pub const MAX_PENDING_LAUNCHES: usize = 32;
pub const MAX_WINDOWS: usize = 128;
pub const MAX_WINDOW_TITLE_BYTES: usize = 384;
pub const MAX_WINDOW_APP_ID_BYTES: usize = 96;
pub const MAX_OUTPUTS: usize = 32;
pub const MAX_WORKSPACES: usize = 32;
pub const MAX_RUNTIME_PERFORMANCE_SAMPLES: usize = 64;
pub const SHELL_SURFACE_APPLICATION_ID_PREFIX: &str = "io.nickel.shell.surface.";

const MAGIC: [u8; 4] = *b"NIKL";
pub const FRAME_HEADER_BYTES: usize = 10;
const HEADER_BYTES: usize = FRAME_HEADER_BYTES;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientEnvelope {
    pub token: String,
    pub request_id: u64,
    pub request: Request,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerEnvelope {
    pub request_id: u64,
    pub message: ServerMessage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum Request {
    RegisterShell { pid: u32 },
    Subscribe,
    Query(Query),
    Command(Command),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "query", rename_all = "snake_case")]
pub enum Query {
    OnScreenKeyboard,
    Snapshot,
    Windows,
    Outputs,
    ShellSurfaces,
    ShellReadiness,
    LauncherVisibility,
    SecureStorage,
    IdleInhibition,
    CacheDiagnostics,
    /// Read bounded renderer timing and allocation telemetry from the shell's
    /// capability-gated nested test endpoint.
    ShellRuntimeDiagnostics,
    Workspaces,
    ShellBehavior,
    RemoteControl,
    Preview {
        window: WindowId,
    },
    /// Resolve a semantic shell target through the live renderer records, or
    /// dispatch a screenshot action through its application host. This query
    /// is served by the shell's capability-gated nested test endpoint, not by
    /// the compositor control socket.
    ShellSemanticTarget {
        target: ShellSemanticTarget,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Observe qualifying application windows created by one process lineage. Only the authenticated
    /// Nickel shell may establish this short-lived attribution authority.
    ObservePendingLaunch {
        generation: u64,
        root_pid: u32,
        root_start_time: u64,
        deadline_ms: u16,
    },
    CancelPendingLaunch {
        generation: u64,
    },
    /// Bind an opaque XDG application identity to an authenticated shell
    /// role. Output-scoped roles carry the monitor identity separately from
    /// presentation metadata such as the window title.
    RegisterShellSurface {
        identity: ShellSurfaceIdentity,
    },
    RequestOnScreenKeyboard,
    ConfigureOnScreenKeyboard {
        height: u32,
        dock_top: bool,
        enabled: bool,
        visible: bool,
        generation: u64,
        environment_override: bool,
    },
    OnScreenKeyboardInput {
        epoch: u64,
        input: OnScreenKeyboardInput,
    },
    ReloadShellSettings,
    ApplyShellBehavior {
        transaction: ShellBehaviorTransaction,
    },
    ApplyRemoteControl {
        requested_enabled: bool,
        generation: u64,
    },
    StartRemotePairing {
        now_unix_secs: u64,
    },
    CancelRemotePairing,
    EmergencyStopRemoteControl,
    DecideRemoteClient {
        client_id: String,
        decision: RemoteClientDecision,
        /// Legacy compatibility field. Identity approval grants no desktop authority; current
        /// clients send an empty vector and resource access requires a separate lease.
        capabilities: Vec<RemoteCapability>,
    },
    RevokeRemoteClient {
        client_id: String,
    },
    BlockRemoteClient {
        client_id: String,
        blocked: bool,
    },
    DecideRemoteLease {
        pending_generation: u64,
        client_id: String,
        #[serde(rename = "lease_request")]
        request: RemoteLeaseRequest,
        allow: bool,
    },
    ManageRemoteLease {
        lease_id: u64,
        action: RemoteLeaseAction,
    },
    ApproveRemoteLeaseDuration {
        pending_generation: u64,
        client_id: String,
        #[serde(rename = "lease_request")]
        request: RemoteLeaseRequest,
        #[serde(deserialize_with = "Option::deserialize")]
        duration_seconds: Option<u64>,
    },
    ToggleLauncher,
    SetLauncherVisible {
        visible: bool,
    },
    SetLauncherVisibleFromController {
        visible: bool,
    },
    SetShellRoleVisible {
        role: ShellRole,
        visible: bool,
    },
    ShowAnchoredShellRole {
        role: ShellRole,
        anchor: ShellPopoverAnchor,
    },
    LogOut,
    SessionAction {
        action: SessionAction,
    },
    Unlock,
    RetrySecureStorage,
    HideOverlay,
    ShowOverlay {
        role: ShellRole,
        geometry: Geometry,
        windows: Vec<WindowId>,
    },
    FocusShellRole {
        role: ShellRole,
    },
    RestoreApplicationFocus,
    IdentifyOutputs,
    CaptureOutput {
        path: String,
        output: Option<String>,
    },
    ApplyOutputs {
        layout: OutputLayout,
    },
    CreateWorkspace,
    ToggleShowDesktop,
    RemoveWorkspace {
        workspace: WorkspaceId,
    },
    SwitchWorkspace {
        workspace: WorkspaceId,
        output: Option<String>,
    },
    MoveWindowToWorkspace {
        window: WindowId,
        workspace: WorkspaceId,
    },
    MoveWindowToOutput {
        window: WindowId,
        output: String,
    },
    HighlightWindow {
        window: Option<WindowId>,
    },
    WindowAction {
        window: WindowId,
        action: WindowAction,
    },
    /// Inject an ordinary compositor input event when the nested session was
    /// explicitly started with its test-control capability enabled.
    TestInput {
        input: TestInput,
    },
    /// Add or remove an output through the nested compositor's explicit test
    /// capability. The native backend rejects this capability at startup.
    TestOutput {
        output: TestOutput,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellSurfaceIdentity {
    pub application_id: String,
    pub role: ShellRole,
    pub output: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum TestOutput {
    Connect {
        name: String,
        logical_width: i32,
        logical_height: i32,
        scale_120: u32,
        transform: OutputTransform,
    },
    Disconnect {
        name: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionAction {
    RestartShell,
    Lock,
    Suspend,
    Reboot,
    PowerOff,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "input", rename_all = "snake_case")]
pub enum TestInput {
    TouchDown {
        slot: u32,
        x: i32,
        y: i32,
    },
    TouchMotion {
        slot: u32,
        x: i32,
        y: i32,
    },
    TouchUp {
        slot: u32,
    },
    TouchCancel {
        slot: u32,
    },
    TouchFrame,
    ControllerConnect,
    ControllerDisconnect,
    ControllerButton {
        button: TestControllerButton,
        state: InputState,
    },
    ControllerTap {
        button: TestControllerButton,
    },
    ControllerAxis {
        axis: TestControllerAxis,
        value: i16,
    },
    Key {
        key: TestKey,
        state: InputState,
    },
    /// Exercise the emergency-key path with explicit source attribution. This
    /// is accepted only by a session started with its private test-control
    /// capability and never creates or reads a host input device.
    EmergencyControl {
        source: TestEmergencyControlSource,
        side: TestEmergencyControlSide,
        state: InputState,
    },
    PointerMove {
        x: i32,
        y: i32,
    },
    PointerMoveRelative {
        dx: i32,
        dy: i32,
    },
    PointerButton {
        button: TestPointerButton,
        state: InputState,
    },
    PointerAxis {
        horizontal_v120: i32,
        vertical_v120: i32,
    },
    /// Dispatch a renderer-resolved shell-local pointer interaction through
    /// the compositor's ordinary absolute-motion and button paths.
    ShellPointer {
        target: ResolvedShellTarget,
    },
    /// Resolve a compositor-owned recovery action through the production
    /// panel layout, then dispatch an ordinary pointer click.
    RecoveryPointer {
        action: RecoveryTargetAction,
        output: Option<String>,
    },
    /// Resolve a managed window through the compositor's live registry and
    /// geometry, then dispatch an ordinary pointer interaction.
    WindowPointer {
        window: WindowId,
        interaction: PointerInteraction,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestEmergencyControlSource {
    /// No syspath, matching remote and ordinary test input.
    Synthetic,
    /// A non-virtual `/sys/devices` attribution fixture delivered through the
    /// production compositor input handler. This does not access hardware.
    PhysicalFixture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestEmergencyControlSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestControllerButton {
    South,
    East,
    West,
    North,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    LeftShoulder,
    RightShoulder,
    Select,
    Start,
    Guide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestControllerAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryTargetAction {
    Retry,
    Exit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum ShellSemanticTarget {
    OnScreenKeyboard {
        key: String,
    },
    OnScreenKeyboardToggle,
    PanelApplication {
        application_id: String,
        output: Option<String>,
        interaction: PointerInteraction,
    },
    PanelControlCenter {
        output: Option<String>,
    },
    ControlCenterLock,
    PreviewWindow {
        window: WindowId,
        action: PreviewTargetAction,
    },
    WindowMenu {
        window: WindowId,
        action: WindowMenuTargetAction,
    },
    Screenshot {
        action: ScreenshotTargetAction,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotTargetAction {
    SelectionStart,
    SelectionEnd,
    Confirm,
    CopyImage,
    SaveImage,
    CopyTemporaryPath,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewTargetAction {
    Hover,
    Activate,
    Close,
    OpenMenu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowMenuTargetAction {
    Close,
    MaximizeRestore,
    Minimize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerInteraction {
    Hover,
    LeftClick,
    RightClick,
    LeftPress,
    LeftRelease,
    LeftDoubleClick,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedShellTarget {
    pub role: ShellRole,
    pub output: Option<String>,
    pub x: i32,
    pub y: i32,
    pub interaction: PointerInteraction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputState {
    Pressed,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestKey {
    A,
    C,
    P,
    V,
    X,
    Enter,
    Escape,
    Tab,
    LeftAlt,
    LeftShift,
    LeftControl,
    LeftMeta,
    Left,
    Right,
    Up,
    Down,
    Space,
    Backspace,
    Delete,
    F11,
    PrintScreen,
    VolumeUp,
    VolumeDown,
    VolumeMute,
    MediaPlayPause,
    MediaPlay,
    MediaPause,
    MediaStop,
    MediaNext,
    MediaPrevious,
    MediaFastForward,
    MediaRewind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestPointerButton {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "response", content = "data", rename_all = "snake_case")]
pub enum ServerMessage {
    OnScreenKeyboard(OnScreenKeyboardSnapshot),
    Ack,
    Error {
        code: ErrorCode,
        message: String,
    },
    Snapshot(Snapshot),
    Windows(Vec<WindowSnapshot>),
    Outputs(Vec<OutputSnapshot>),
    ShellSurfaces(Vec<ShellSurfaceSnapshot>),
    ShellReadiness(ShellReadinessSnapshot),
    LauncherVisibility {
        visible: bool,
    },
    SecureStorage {
        state: SecureStorageState,
        reason: Option<SecureStorageUnavailableReason>,
    },
    IdleInhibition {
        surfaces: u16,
    },
    CacheDiagnostics(Box<CacheDiagnostics>),
    ShellRuntimeDiagnostics(ShellRuntimeDiagnostics),
    Workspaces(WorkspaceState),
    ShellBehavior(ShellBehaviorSnapshot),
    RemoteControl(RemoteControlSnapshot),
    RemotePairing(RemotePairingSnapshot),
    Preview(PreviewFrame),
    ShellSemanticTarget(ResolvedShellTarget),
    Event(Event),
}

/// Native optional-preview work only. Byte counts are logical texture/PBO
/// payload, not driver allocations or process RSS. Times are CPU wall time;
/// completion age includes scheduling delay and is not a GPU timer query.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NativePreviewWorkDiagnostics {
    pub pending_count: u16,
    pub pending_texture_bytes: u64,
    pub pending_readback_bytes: u64,
    pub peak_pending_payload_bytes: u64,
    pub turns: u64,
    pub pending_polls: u64,
    pub submissions: u64,
    pub submission_failures: u64,
    pub readback_failures: u64,
    pub completions: u64,
    pub cancellations: u64,
    pub timeouts: u64,
    pub submit_cpu_us: u64,
    pub map_copy_cpu_us: u64,
    pub completion_age_us: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheDiagnostics {
    #[serde(default)]
    pub native_preview_work: NativePreviewWorkDiagnostics,
    #[serde(default)]
    pub internal_ui_surfaces: u16,
    #[serde(default)]
    pub internal_ui_gpu_frames: u64,
    #[serde(default)]
    pub internal_ui_fallback_frames: u64,
    #[serde(default)]
    pub internal_ui_software_frame_bytes: u64,
    #[serde(default)]
    pub internal_ui_fallback_raster_bytes: u64,
    /// Private text renderer pixel allocation capacity, separate from shared textures.
    #[serde(default)]
    pub internal_ui_text_scratch_bytes: u64,
    #[serde(default)]
    pub internal_ui_text_private_cache_bytes: u64,
    #[serde(default)]
    pub internal_ui_fallback_buffer_creations: u64,
    #[serde(default)]
    pub internal_ui_fallback_buffer_reuses: u64,
    #[serde(default)]
    pub internal_ui_fallback_converted_bytes: u64,
    /// Submitted damage payload estimate; not measured driver upload traffic.
    #[serde(default)]
    pub internal_ui_fallback_upload_damage_bytes: u64,
    #[serde(default)]
    pub internal_ui_fallback_full_repaints: u64,
    #[serde(default)]
    pub internal_ui_fallback_partial_repaints: u64,
    #[serde(default)]
    pub internal_ui_image_cache_entries: u16,
    #[serde(default)]
    pub internal_ui_image_cache_bytes: u64,
    #[serde(default)]
    pub internal_ui_text_cache_entries: u16,
    #[serde(default)]
    pub internal_ui_text_cache_bytes: u64,
    #[serde(default)]
    pub internal_ui_texture_import_failures: u64,
    #[serde(default)]
    pub internal_ui_fallback_import_failures: u64,
    #[serde(default)]
    pub internal_shell_wallpaper_entries: u16,
    #[serde(default)]
    pub internal_shell_wallpaper_bytes: u64,
    pub preview_entries: u16,
    pub preview_capacity: u16,
    pub preview_bytes: u64,
    #[serde(default)]
    pub preview_byte_capacity: u64,
    #[serde(default)]
    pub preview_peak_bytes: u64,
    #[serde(default)]
    pub preview_admissions: u64,
    #[serde(default)]
    pub preview_evictions: u64,
    #[serde(default)]
    pub preview_invalidations: u64,
    #[serde(default)]
    pub preview_captures: u64,
    #[serde(default)]
    pub preview_skipped_unchanged: u64,
    #[serde(default)]
    pub preview_readback_bytes: u64,
    #[serde(default)]
    /// Aggregate transient bytes allocated/copied for raw clone, base64 string, JSON payload,
    /// and framed response. Explicit fields below partition this value.
    pub preview_protocol_copy_bytes: u64,
    /// Raw RGBA bytes cloned into protocol response values.
    #[serde(default)]
    pub preview_protocol_raw_copy_bytes: u64,
    /// Base64 payload bytes expected during JSON serialization; framing overhead is excluded.
    #[serde(default)]
    pub preview_protocol_base64_bytes: u64,
    /// Exact serialized JSON payload bytes produced for preview responses.
    #[serde(default)]
    pub preview_protocol_json_payload_bytes: u64,
    /// Exact framed response bytes copied into the datagram send buffer.
    #[serde(default)]
    pub preview_protocol_framed_copy_bytes: u64,
    #[serde(default)]
    pub preview_capture_failures: u64,
    #[serde(default)]
    /// Completed preview presentation revision; source commits awaiting capture
    /// do not advance this value while the last completed pixels remain visible.
    pub preview_cache_generation: u64,
    pub metadata_entries: u16,
    pub metadata_title_bytes: u64,
    pub metadata_peak_title_bytes: u64,
    pub metadata_app_id_bytes: u64,
    pub metadata_peak_app_id_bytes: u64,
    pub metadata_truncations: u64,
    pub metadata_canonicalizations: u64,
    pub metadata_updates: u64,
    pub metadata_live_snapshot_bytes: u64,
    pub metadata_peak_snapshot_bytes: u64,
    #[serde(default)]
    pub titlebar_entries: u16,
    #[serde(default)]
    pub titlebar_live_bytes: u64,
    #[serde(default)]
    pub titlebar_peak_bytes: u64,
    #[serde(default)]
    pub titlebar_hits: u64,
    #[serde(default)]
    pub titlebar_misses: u64,
    #[serde(default)]
    pub titlebar_rasterizations: u64,
    #[serde(default)]
    pub titlebar_avoided_rasterizations: u64,
    #[serde(default)]
    pub titlebar_evictions: u64,
    #[serde(default)]
    pub titlebar_generation: u64,
    #[serde(default)]
    pub titlebar_font_database_loads: u64,
    #[serde(default)]
    pub titlebar_renderer_bytes: Option<u64>,
    #[serde(default)]
    pub recovery_entries: u16,
    #[serde(default)]
    pub recovery_live_bytes: u64,
    #[serde(default)]
    pub recovery_peak_bytes: u64,
    #[serde(default)]
    pub recovery_rasterizations: u64,
    #[serde(default)]
    pub recovery_avoided_rasterizations: u64,
    #[serde(default)]
    pub recovery_evictions: u64,
    #[serde(default)]
    pub recovery_generation: u64,
    #[serde(default)]
    pub recovery_renderer_bytes: Option<u64>,
    #[serde(default)]
    pub identify_entries: u16,
    #[serde(default)]
    pub identify_live_bytes: u64,
    #[serde(default)]
    pub identify_peak_bytes: u64,
    #[serde(default)]
    pub identify_rasterizations: u64,
    #[serde(default)]
    pub identify_avoided_rasterizations: u64,
    #[serde(default)]
    pub identify_evictions: u64,
    #[serde(default)]
    pub identify_renderer_bytes: Option<u64>,
}

/// Bounded runtime evidence retained by a shell presenter.
///
/// Durations are represented as integer microseconds so evidence is stable
/// across JSON encoders without claiming sub-microsecond precision. Each
/// sample vector is capped by [`MAX_RUNTIME_PERFORMANCE_SAMPLES`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellRuntimeDiagnostics {
    #[serde(default)]
    pub input_to_message_us: Vec<u64>,
    #[serde(default)]
    pub input_to_frame_us: Vec<u64>,
    #[serde(default)]
    pub layout_us: Vec<u64>,
    #[serde(default)]
    pub paint_list_us: Vec<u64>,
    pub warm_present_us: Vec<u64>,
    pub input_to_visible_us: Vec<u64>,
    #[serde(default)]
    pub scheduled_wakeups: u64,
    #[serde(default)]
    pub host_phase_samples_available: bool,
    pub retained_presenter_bytes: u64,
    pub frame_allocations: AllocationMeasurement,
    /// Rows are likely-graphical, likely-terminal, unknown, and unavailable;
    /// columns are no qualifying window and a qualifying window.
    #[serde(default)]
    pub executable_prediction_observations: [[u64; 2]; 4],
    /// Qualifying-window observations attributed through a child process.
    #[serde(default)]
    pub executable_prediction_descendant_windows: u64,
}

impl ShellRuntimeDiagnostics {
    pub fn validate(&self) -> Result<(), FrameError> {
        if [
            &self.input_to_message_us,
            &self.input_to_frame_us,
            &self.layout_us,
            &self.paint_list_us,
            &self.warm_present_us,
            &self.input_to_visible_us,
        ]
        .into_iter()
        .any(|samples| samples.len() > MAX_RUNTIME_PERFORMANCE_SAMPLES)
        {
            return Err(FrameError::TooLarge);
        }
        self.frame_allocations.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocationMeasurement {
    /// P95 allocation operations across the reported warm-frame samples.
    pub count: Option<u64>,
    pub sample_count: usize,
    pub scope: AllocationScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

impl Default for AllocationMeasurement {
    fn default() -> Self {
        Self {
            count: None,
            sample_count: 0,
            scope: AllocationScope::Unavailable,
            unavailable_reason: Some("allocation instrumentation is not installed".into()),
        }
    }
}

impl AllocationMeasurement {
    pub fn validate(&self) -> Result<(), FrameError> {
        match self.scope {
            AllocationScope::Unavailable
                if self.count.is_some()
                    || self.sample_count != 0
                    || self
                        .unavailable_reason
                        .as_deref()
                        .is_none_or(|reason| reason.trim().is_empty()) =>
            {
                Err(FrameError::InvalidPayload(
                    "unavailable allocation evidence cannot contain measurements".into(),
                ))
            }
            AllocationScope::Unavailable => Ok(()),
            _ if self.count.is_some()
                && self.sample_count > 0
                && self.unavailable_reason.is_none() =>
            {
                Ok(())
            }
            _ if self.count.is_none()
                && self.sample_count == 0
                && self
                    .unavailable_reason
                    .as_deref()
                    .is_some_and(|reason| !reason.trim().is_empty()) =>
            {
                Ok(())
            }
            _ => Err(FrameError::InvalidPayload(
                "allocation evidence has inconsistent count, samples, or reason".into(),
            )),
        }
    }
}

/// Scope covered by an allocator-visible measurement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllocationScope {
    /// All allocations made by the instrumented shell process while sampling.
    Process,
    /// Allocations made by the thread executing the presenter while sampling.
    Thread,
    /// Allocations explicitly owned by the presenter implementation.
    Presenter,
    /// The runtime does not currently expose an allocation counter.
    #[default]
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Unauthorized,
    IncompatibleVersion,
    InvalidRequest,
    InvalidWindow,
    ResourceLimit,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum Event {
    PendingLaunchWindow {
        generation: u64,
        observed_after_ms: u16,
        descendant: bool,
    },
    PendingLaunchExpired {
        generation: u64,
    },
    ShellSettingsChanged,
    ShellBehaviorChanged(ShellBehaviorSnapshot),
    Snapshot(Snapshot),
    LauncherVisibility {
        visible: bool,
    },
    Windows(Vec<WindowSnapshot>),
    Outputs(Vec<OutputSnapshot>),
    Focus {
        window: Option<WindowId>,
    },
    Stacking {
        front_to_back: Vec<WindowId>,
    },
    WindowRemoved {
        window: WindowId,
    },
    Preview(PreviewFrame),
    OutputCaptureCompleted {
        path: String,
        result: CaptureResult,
    },
    Workspaces(WorkspaceState),
    LockState {
        locked: bool,
    },
    GlobalShortcut {
        action: ShortcutAction,
    },
    ConsumerControl {
        control: ConsumerControl,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShellBehaviorSetting {
    BarDisplayScope,
    BarWindowScope,
    DesktopCount,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum ShellBehaviorValue {
    Toggle(bool),
    Count(u8),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellBehaviorTransaction {
    pub setting: ShellBehaviorSetting,
    pub prior: ShellBehaviorValue,
    pub requested: ShellBehaviorValue,
    pub topology_generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellBehaviorSnapshot {
    pub bar_on_all_displays: bool,
    pub all_windows_on_every_bar: bool,
    pub desktop_count: u8,
    pub topology_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteControlEffectiveState {
    Disabled,
    Enabled,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteControlSnapshot {
    pub requested_enabled: bool,
    pub effective: RemoteControlEffectiveState,
    pub generation: u64,
    pub acknowledged_generation: u64,
    pub endpoint: String,
    #[serde(default)]
    pub host_fingerprint: Option<String>,
    #[serde(default)]
    pub environment_override: bool,
    pub diagnostic: Option<String>,
    pub pending_clients: Vec<RemotePendingClientSnapshot>,
    pub granted_clients: Vec<RemoteGrantedClientSnapshot>,
    #[serde(default)]
    pub pending_leases: Vec<RemotePendingLease>,
    #[serde(default)]
    pub active_leases: Vec<RemoteActiveLease>,
    #[serde(default)]
    pub lease_audit: Vec<RemoteLeaseAuditEvent>,
    #[serde(default)]
    pub lease_audit_evicted: u64,
    #[serde(default)]
    pub permission_audit: Vec<RemotePermissionAuditEvent>,
    #[serde(default)]
    pub permission_audit_evicted: u64,
    #[serde(default)]
    pub trace_audit: Vec<RemoteTraceAuditEvent>,
    #[serde(default)]
    pub trace_audit_evicted: u64,
    #[serde(default)]
    pub operation_audit: Vec<RemoteOperationAuditEvent>,
    #[serde(default)]
    pub operation_audit_evicted: u64,
    #[serde(default)]
    pub connection_audit: Vec<RemoteConnectionAuditEvent>,
    #[serde(default)]
    pub connection_audit_evicted: u64,
}

/// Local control protocol only; excluded from agent discovery and diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteConnectionAuditEvent {
    pub generation: u64,
    pub observed_at_us: u64,
    pub client_id: String,
    pub address: std::net::IpAddr,
    pub tls: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RemoteTraceCategory {
    NestedFrameDispatch,
    DrmFrameDispatch,
}

/// Fixed trace lifecycle outcomes for trusted local inspection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteTraceTransition {
    Started,
    Stopped,
    TimedOut,
    Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteTraceAuditEvent {
    pub generation: u64,
    pub observed_at_us: u64,
    pub client_id: u64,
    pub lease_id: u64,
    pub trace_id: u64,
    pub category: RemoteTraceCategory,
    pub transition: RemoteTraceTransition,
    pub duration_limit_seconds: u16,
    pub elapsed_us: u64,
}

/// Fixed outcomes for the trusted local operation history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteOperationOutcome {
    Success,
    Error,
    Cancelled,
}

/// Trusted local Settings projection, excluded from MCP tools and diagnostic snapshots.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteOperationAuditEvent {
    pub generation: u64,
    pub observed_at_us: u64,
    /// Fixed server-owned method name, never supplied by a caller.
    pub method: String,
    /// First lease that authorized the operation. None means authorization never succeeded.
    pub matched_lease_id: Option<u64>,
    pub duration_us: u64,
    pub outcome: RemoteOperationOutcome,
}

/// Fixed permission outcomes; no caller-controlled labels or request payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(usize)]
pub enum RemotePermissionOutcome {
    Submitted,
    Coalesced,
    Approved,
    Denied,
    Cancelled,
    Blocked,
    Invalid,
    Capacity,
    Cooldown,
    BlockedRequest,
    Unauthorized,
}

/// Trusted local projection, excluded from remote diagnostic snapshots.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePermissionAuditEvent {
    pub generation: u64,
    pub observed_at_us: u64,
    pub client_id: Option<u64>,
    pub outcome: RemotePermissionOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteLeaseTransition {
    Approved,
    Renewed,
    Paused,
    Resumed,
    Expired,
    Revoked,
    Disconnected,
    Reconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteLeaseScopeKind {
    Surface,
    Window,
    Application,
    Output,
    FullSession,
}

/// Trusted local Settings projection, excluded from MCP diagnostic snapshots.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteLeaseAuditEvent {
    pub generation: u64,
    pub observed_at_us: u64,
    pub lease_id: u64,
    pub transition: RemoteLeaseTransition,
    pub scope: RemoteLeaseScopeKind,
    pub lifetime_limit_seconds: Option<u64>,
    pub full_debug: bool,
    pub allow_resumption: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteLeaseAction {
    Pause,
    Resume,
    Revoke,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteActiveLease {
    pub lease_id: u64,
    pub client_label: String,
    pub scope: RemoteResourceScope,
    /// Presentation resolved by the local session; never an authorization key.
    #[serde(default)]
    pub resource_label: Option<String>,
    pub remaining_seconds: Option<u64>,
    pub suspended: bool,
    pub full_debug: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RemoteResourceId {
    pub id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", content = "resource", rename_all = "snake_case")]
pub enum RemoteResourceScope {
    Surface(RemoteResourceId),
    Window(RemoteResourceId),
    /// Use verified_application from an authorized window or installed-app inventory.
    /// A display label, self-reported app ID, or WM_CLASS does not establish scope.
    Application(String),
    Output(RemoteResourceId),
    FullSession,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteLeaseRequest {
    #[serde(default)]
    pub renewal: Option<RemoteLeaseRenewal>,
    pub scope: RemoteResourceScope,
    pub duration_seconds: Option<u64>,
    pub allow_resumption: bool,
    pub full_debug: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteLeaseRenewal {
    pub lease_id: u64,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteLeaseRequestChanges {
    pub access_changed: bool,
    pub duration_increased: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemotePendingLease {
    /// Incarnation of this pending card, preserved only for equivalent coalesced requests.
    pub pending_generation: u64,
    pub client_id: String,
    pub client_label: String,
    pub request: RemoteLeaseRequest,
    /// Presentation resolved by the local session; never supplied by the client.
    #[serde(default)]
    pub resource_label: Option<String>,
    /// Local comparison with previous versions of this pending request.
    #[serde(default)]
    pub changes: RemoteLeaseRequestChanges,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteClientDecision {
    Deny,
    AllowOnce,
    Remember,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteCapability {
    Observe,
    WindowManagement,
    SettingsRead,
    SettingsChange,
    ApplicationLaunch,
    PointerInput,
    KeyboardInput,
    ScreenCapture,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePendingClientSnapshot {
    pub id: String,
    pub label: String,
    pub requested: Vec<RemoteCapability>,
    pub connected_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteClientOrigin {
    pub address: String,
    pub tls: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteGrantedClientSnapshot {
    pub id: String,
    pub label: String,
    /// Legacy compatibility field, always empty. See the active and pending lease collections
    /// for desktop authority.
    #[serde(default)]
    pub capabilities: Vec<RemoteCapability>,
    pub remembered: bool,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub origin: Option<RemoteClientOrigin>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePairingSnapshot {
    pub ceremony_id: String,
    pub qr_payload: String,
    pub short_code: String,
    pub expires_at: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutAction {
    ShowRun,
    OpenFiles,
    OpenSettings,
    ShowControlCenter,
    ShowNotifications,
    ShowDesktop,
    ProjectDisplays,
    ShowWindowMenu,
    ShowScreenshotTool,
    CaptureActiveWindow,
    CaptureActiveWindowToFile,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerControl {
    VolumeUp,
    VolumeDown,
    VolumeMute,
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    FastForward,
    Rewind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CaptureResult {
    Saved { backend: CaptureBackend },
    Failed { message: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureBackend {
    Nested,
    Native,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub outputs: Vec<OutputSnapshot>,
    pub windows: Vec<WindowSnapshot>,
    pub focused: Option<WindowId>,
    pub stacking_front_to_back: Vec<WindowId>,
    pub launcher_visible: bool,
    pub locked: bool,
    pub workspaces: WorkspaceState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WindowId(pub u64);

/// No surrounding text or typed content is included in recipient diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnScreenKeyboardSnapshot {
    pub height: u32,
    /// Discard controller events produced at or before the last keyboard ownership transition.
    pub controller_barrier_unix_ms: u64,
    pub dock_top: bool,
    pub auto_show_requested: bool,
    pub touchscreen_present: bool,
    pub generation: u64,
    pub environment_override: bool,
    pub epoch: u64,
    pub recipient: Option<WindowId>,
    /// Opaque compositor-hosted surface identity, separate from Wayland window
    /// IDs. No field contents are exposed; the epoch remains the delivery lease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internal_recipient: Option<u64>,
    pub text_input_active: bool,
    pub enabled: bool,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OnScreenKeyboardInput {
    Text {
        text: String,
    },
    /// XKB keysyms, not hardware scan codes. Modifiers apply to this tap only.
    Key {
        keysym: u32,
        modifiers: Vec<u32>,
    },
}

impl OnScreenKeyboardSnapshot {
    pub fn has_recipient(&self) -> bool {
        self.recipient.is_some() || self.internal_recipient.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSnapshot {
    pub id: WindowId,
    pub application_id: String,
    pub title: String,
    pub active: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub geometry: Option<Geometry>,
    pub workspace: WorkspaceId,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkspaceId(pub u64);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub active: WorkspaceId,
    pub active_output: Option<String>,
    pub ordered: Vec<WorkspaceSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub id: WorkspaceId,
    pub windows: Vec<WindowId>,
    pub focused: Option<WindowId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputSnapshot {
    pub name: String,
    pub model: String,
    pub geometry: Geometry,
    pub work_area: Geometry,
    /// Fractional scale in Wayland protocol units (120 == 1.0).
    pub scale_120: u32,
    pub transform: OutputTransform,
    pub physical_width_mm: i32,
    pub physical_height_mm: i32,
    pub primary: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellSurfaceSnapshot {
    pub role: ShellRole,
    pub geometry: Option<Geometry>,
    pub output: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellReadinessSnapshot {
    pub expected_shell_pid: Option<u32>,
    pub authenticated_shell_pid: Option<u32>,
    pub outputs: u16,
    pub desktops: u16,
    pub panels: u16,
    pub locks: u16,
    pub launchers: u16,
    pub required_singletons_ready: bool,
    pub output_roles_ready: bool,
    pub reserved_ordinary_windows: u16,
    pub ready: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputTransform {
    #[default]
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorSide {
    Above,
    Below,
    Left,
    Right,
}

/// A semantic control anchor expressed in the invoking output's logical
/// coordinate space. Output identity prevents identical per-output controls
/// from being confused during placement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellPopoverAnchor {
    pub control: String,
    pub output: String,
    pub bounds: Geometry,
    pub preferred: AnchorSide,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputLayout {
    pub primary: String,
    pub placements: Vec<OutputPlacement>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputPlacement {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub enabled: bool,
    /// Exact desired scale in Wayland fractional-scale units (120 == 100%).
    #[serde(default = "default_output_scale_120")]
    pub scale_120: u32,
}

const fn default_output_scale_120() -> u32 {
    120
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellRole {
    Desktop,
    Panel,
    Launcher,
    ControlCenter,
    ContextMenu,
    Preview,
    Notification,
    VolumeOsd,
    ProjectMenu,
    Lock,
    Screenshot,
    OnScreenKeyboard,
    Recovery,
}

impl ShellRole {
    pub fn application_id(self) -> &'static str {
        match self {
            Self::Desktop => "io.nickel.shell.desktop",
            Self::Panel => "io.nickel.shell.panel",
            Self::Launcher => "io.nickel.shell.launcher",
            Self::ControlCenter => "io.nickel.shell.control-center",
            Self::ContextMenu => "io.nickel.shell.context-menu",
            Self::Preview => "io.nickel.shell.preview",
            Self::Notification => "io.nickel.shell.notification",
            Self::VolumeOsd => "io.nickel.shell.volume-osd",
            Self::ProjectMenu => "io.nickel.shell.project-menu",
            Self::Lock => "io.nickel.shell.lock",
            Self::Screenshot => "io.nickel.shell.screenshot",
            Self::OnScreenKeyboard => "io.nickel.shell.on-screen-keyboard",
            Self::Recovery => "io.nickel.shell.recovery",
        }
    }

    pub fn from_application_id(value: &str) -> Option<Self> {
        [
            Self::Desktop,
            Self::Panel,
            Self::Launcher,
            Self::ControlCenter,
            Self::ContextMenu,
            Self::Preview,
            Self::Notification,
            Self::VolumeOsd,
            Self::ProjectMenu,
            Self::Lock,
            Self::Screenshot,
            Self::OnScreenKeyboard,
            Self::Recovery,
        ]
        .into_iter()
        .find(|role| role.application_id() == value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowAction {
    Activate,
    Close,
    Minimize,
    MaximizeRestore,
    FullscreenRestore,
    SnapLeading,
    SnapTrailing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecureStorageState {
    Starting,
    Locked,
    PromptRequired,
    Ready,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecureStorageUnavailableReason {
    Connection,
    Protocol,
    MissingDefaultCollection,
    PromptTimedOut,
    ProviderDisappeared,
    ProviderConfiguration,
    UnexpectedProvider,
    ReadinessCheck,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewFrame {
    pub window: WindowId,
    pub width: u16,
    pub height: u16,
    #[serde(with = "base64_bytes")]
    pub rgba: Vec<u8>,
}

mod base64_bytes {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        STANDARD.decode(encoded).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("frame is too short")]
    TooShort,
    #[error("invalid frame magic")]
    InvalidMagic,
    #[error("incompatible protocol version {0}")]
    IncompatibleVersion(u16),
    #[error("frame exceeds the size limit")]
    TooLarge,
    #[error("frame length does not match its header")]
    LengthMismatch,
    #[error("invalid frame payload: {0}")]
    InvalidPayload(String),
}

pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, FrameError> {
    let payload =
        serde_json::to_vec(value).map_err(|error| FrameError::InvalidPayload(error.to_string()))?;
    if payload.len() + HEADER_BYTES > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    let length = u32::try_from(payload.len()).map_err(|_| FrameError::TooLarge)?;
    let mut frame = Vec::with_capacity(HEADER_BYTES + payload.len());
    frame.extend_from_slice(&MAGIC);
    frame.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn decode<T: DeserializeOwned>(frame: &[u8]) -> Result<T, FrameError> {
    if frame.len() < HEADER_BYTES {
        return Err(FrameError::TooShort);
    }
    if frame[..4] != MAGIC {
        return Err(FrameError::InvalidMagic);
    }
    let version = u16::from_le_bytes([frame[4], frame[5]]);
    if version != PROTOCOL_VERSION {
        return Err(FrameError::IncompatibleVersion(version));
    }
    let length = u32::from_le_bytes(frame[6..10].try_into().expect("fixed header")) as usize;
    if length + HEADER_BYTES > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    if frame.len() != length + HEADER_BYTES {
        return Err(FrameError::LengthMismatch);
    }
    serde_json::from_slice(&frame[HEADER_BYTES..])
        .map_err(|error| FrameError::InvalidPayload(error.to_string()))
}

impl PreviewFrame {
    pub fn validate(&self) -> Result<(), FrameError> {
        if self.width == 0 || self.height == 0 {
            return Err(FrameError::InvalidPayload(
                "preview dimensions must be non-zero".into(),
            ));
        }
        if self.width > MAX_PREVIEW_WIDTH || self.height > MAX_PREVIEW_HEIGHT {
            return Err(FrameError::TooLarge);
        }
        let expected = usize::from(self.width) * usize::from(self.height) * 4;
        if self.rgba.len() != expected {
            return Err(FrameError::LengthMismatch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lease_management_uses_the_protocols_snake_case_wire_actions() {
        use super::*;
        for (action, name) in [
            (RemoteLeaseAction::Pause, "pause"),
            (RemoteLeaseAction::Resume, "resume"),
            (RemoteLeaseAction::Revoke, "revoke"),
        ] {
            let value = serde_json::json!({
                "token": "test-only", "request_id": 42,
                "request": {"request": "command", "command": "manage_remote_lease",
                    "lease_id": 7, "action": name}
            });
            let envelope: ClientEnvelope = serde_json::from_value(value).unwrap();
            assert_eq!(
                envelope.request,
                Request::Command(Command::ManageRemoteLease {
                    lease_id: 7,
                    action,
                })
            );
            assert_eq!(
                decode::<ClientEnvelope>(&encode(&envelope).unwrap()).unwrap(),
                envelope
            );
        }
    }

    #[test]
    fn pending_lease_change_metadata_defaults_with_explicit_incarnation() {
        use super::*;
        let pending: RemotePendingLease = serde_json::from_value(serde_json::json!({
            "pending_generation": 1, "client_id": "agent", "client_label": "Agent", "resource_label": null,
            "request": {"scope": {"kind": "full_session"}, "duration_seconds": 30,
                "allow_resumption": false, "full_debug": false}
        }))
        .unwrap();
        assert_eq!(pending.changes, RemoteLeaseRequestChanges::default());
    }

    #[test]
    fn local_duration_approval_requires_an_explicit_choice() {
        use super::*;
        for duration_seconds in [Some(1200), Some(7200), Some(420), None] {
            let command = Command::ApproveRemoteLeaseDuration {
                pending_generation: 1,
                client_id: "agent".into(),
                request: RemoteLeaseRequest {
                    renewal: None,
                    scope: RemoteResourceScope::FullSession,
                    duration_seconds: Some(30),
                    allow_resumption: false,
                    full_debug: false,
                },
                duration_seconds,
            };
            let mut json = serde_json::to_value(&command).unwrap();
            assert_eq!(
                serde_json::from_value::<Command>(json.clone()).unwrap(),
                command
            );
            json.as_object_mut().unwrap().remove("duration_seconds");
            assert!(serde_json::from_value::<Command>(json).is_err());
        }
    }

    #[test]
    fn lease_decision_round_trips_inside_the_authenticated_request_envelope() {
        use super::*;
        let envelope = ClientEnvelope {
            token: "test-only".into(),
            request_id: 42,
            request: Request::Command(Command::DecideRemoteLease {
                pending_generation: 1,
                client_id: "agent".into(),
                allow: true,
                request: RemoteLeaseRequest {
                    renewal: None,
                    scope: RemoteResourceScope::FullSession,
                    duration_seconds: Some(1200),
                    allow_resumption: false,
                    full_debug: false,
                },
            }),
        };
        let encoded = encode(&envelope).unwrap();
        assert_eq!(decode::<ClientEnvelope>(&encoded).unwrap(), envelope);
        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(json["request"]["request"], "command");
        assert!(json["request"]["lease_request"].is_object());
        let mut missing = json;
        missing["request"]
            .as_object_mut()
            .unwrap()
            .remove("pending_generation");
        assert!(serde_json::from_value::<ClientEnvelope>(missing).is_err());
    }
    #[test]
    fn native_preview_work_diagnostics_preserve_old_payload_compatibility() {
        let mut old = serde_json::to_value(super::CacheDiagnostics::default()).unwrap();
        old.as_object_mut().unwrap().remove("native_preview_work");
        let decoded: super::CacheDiagnostics = serde_json::from_value(old).unwrap();
        assert_eq!(decoded.native_preview_work, Default::default());

        let work = super::NativePreviewWorkDiagnostics {
            pending_count: 1,
            pending_texture_bytes: 129_600,
            pending_readback_bytes: 129_600,
            peak_pending_payload_bytes: 259_200,
            submissions: 3,
            completions: 2,
            ..Default::default()
        };
        let encoded = serde_json::to_vec(&work).unwrap();
        assert_eq!(
            serde_json::from_slice::<super::NativePreviewWorkDiagnostics>(&encoded).unwrap(),
            work
        );
        assert_eq!(
            serde_json::from_str::<super::NativePreviewWorkDiagnostics>("{}").unwrap(),
            Default::default()
        );
    }

    #[test]
    fn keyboard_internal_recipient_is_optional_and_distinct_from_window_identity() {
        let legacy = super::OnScreenKeyboardSnapshot {
            recipient: Some(super::WindowId(7)),
            epoch: 19,
            ..Default::default()
        };
        let wire = serde_json::to_value(&legacy).unwrap();
        assert!(wire.get("internal_recipient").is_none());
        let decoded: super::OnScreenKeyboardSnapshot = serde_json::from_value(wire).unwrap();
        assert_eq!(decoded, legacy);
        assert!(decoded.has_recipient());

        let native = super::OnScreenKeyboardSnapshot {
            internal_recipient: Some(7),
            epoch: 20,
            ..Default::default()
        };
        let decoded: super::OnScreenKeyboardSnapshot =
            serde_json::from_value(serde_json::to_value(&native).unwrap()).unwrap();
        assert_eq!(decoded, native);
        assert!(decoded.recipient.is_none());
        assert!(decoded.has_recipient());
        assert!(!super::OnScreenKeyboardSnapshot::default().has_recipient());
    }

    use super::*;

    #[test]
    fn presentation_memory_counters_round_trip_and_default_for_older_peers() {
        let counters = CacheDiagnostics {
            internal_ui_text_scratch_bytes: 4096,
            internal_ui_text_private_cache_bytes: 8192,
            internal_ui_fallback_buffer_creations: 1,
            internal_ui_fallback_buffer_reuses: 7,
            internal_ui_fallback_converted_bytes: 128,
            internal_ui_fallback_upload_damage_bytes: 256,
            internal_ui_fallback_full_repaints: 1,
            internal_ui_fallback_partial_repaints: 7,
            ..Default::default()
        };
        let envelope = ServerEnvelope {
            request_id: 42,
            message: ServerMessage::CacheDiagnostics(Box::new(counters)),
        };
        assert_eq!(
            decode::<ServerEnvelope>(&encode(&envelope).unwrap()).unwrap(),
            envelope
        );
        let mut legacy = serde_json::to_value(CacheDiagnostics::default()).unwrap();
        let fields = legacy.as_object_mut().unwrap();
        for key in [
            "internal_ui_text_scratch_bytes",
            "internal_ui_text_private_cache_bytes",
            "internal_ui_fallback_buffer_creations",
            "internal_ui_fallback_buffer_reuses",
            "internal_ui_fallback_converted_bytes",
            "internal_ui_fallback_upload_damage_bytes",
            "internal_ui_fallback_full_repaints",
            "internal_ui_fallback_partial_repaints",
        ] {
            fields.remove(key);
        }
        assert_eq!(
            serde_json::from_value::<CacheDiagnostics>(legacy).unwrap(),
            CacheDiagnostics::default()
        );
    }

    #[test]
    fn round_trip_is_versioned_and_bounded() {
        let envelope = ClientEnvelope {
            token: "session-token".into(),
            request_id: 7,
            request: Request::Query(Query::Snapshot),
        };
        let frame = encode(&envelope).unwrap();
        assert_eq!(decode::<ClientEnvelope>(&frame).unwrap(), envelope);

        let mut incompatible = frame.clone();
        incompatible[4..6].copy_from_slice(&(PROTOCOL_VERSION + 1).to_le_bytes());
        assert_eq!(
            decode::<ClientEnvelope>(&incompatible),
            Err(FrameError::IncompatibleVersion(PROTOCOL_VERSION + 1))
        );

        let oversized = ServerEnvelope {
            request_id: 1,
            message: ServerMessage::Error {
                code: ErrorCode::Internal,
                message: "x".repeat(MAX_FRAME_BYTES),
            },
        };
        assert_eq!(encode(&oversized), Err(FrameError::TooLarge));
    }

    #[test]
    fn preview_dimensions_and_payload_are_bounded() {
        let valid = PreviewFrame {
            window: WindowId(1),
            width: 2,
            height: 2,
            rgba: vec![0; 16],
        };
        assert_eq!(valid.validate(), Ok(()));
        for (width, height) in [(0, 2), (2, 0), (0, 0)] {
            assert!(matches!(
                PreviewFrame {
                    width,
                    height,
                    rgba: Vec::new(),
                    ..valid.clone()
                }
                .validate(),
                Err(FrameError::InvalidPayload(_))
            ));
        }
        assert_eq!(
            PreviewFrame {
                width: 257,
                ..valid.clone()
            }
            .validate(),
            Err(FrameError::TooLarge)
        );
        assert_eq!(
            PreviewFrame {
                rgba: vec![0; 15],
                ..valid
            }
            .validate(),
            Err(FrameError::LengthMismatch)
        );
    }

    #[test]
    fn typed_shell_surface_identity_round_trips_with_output() {
        let request = ClientEnvelope {
            token: "capability".into(),
            request_id: 41,
            request: Request::Command(Command::RegisterShellSurface {
                identity: ShellSurfaceIdentity {
                    application_id: format!("{SHELL_SURFACE_APPLICATION_ID_PREFIX}42.7"),
                    role: ShellRole::Panel,
                    output: Some("Unknown - Display - DP-2".into()),
                },
            }),
        };
        assert_eq!(
            decode::<ClientEnvelope>(&encode(&request).unwrap()).unwrap(),
            request
        );
    }

    #[test]
    fn nested_test_input_commands_round_trip() {
        for input in [
            TestInput::ControllerConnect,
            TestInput::ControllerButton {
                button: TestControllerButton::South,
                state: InputState::Pressed,
            },
            TestInput::ControllerTap {
                button: TestControllerButton::Guide,
            },
            TestInput::ControllerAxis {
                axis: TestControllerAxis::LeftX,
                value: i16::MAX,
            },
            TestInput::ControllerDisconnect,
            TestInput::Key {
                key: TestKey::LeftAlt,
                state: InputState::Pressed,
            },
            TestInput::Key {
                key: TestKey::V,
                state: InputState::Released,
            },
            TestInput::Key {
                key: TestKey::X,
                state: InputState::Pressed,
            },
            TestInput::PointerMove { x: 640, y: 360 },
            TestInput::PointerMoveRelative { dx: 12, dy: -7 },
            TestInput::PointerButton {
                button: TestPointerButton::Left,
                state: InputState::Released,
            },
            TestInput::PointerAxis {
                horizontal_v120: 120,
                vertical_v120: -240,
            },
            TestInput::ShellPointer {
                target: ResolvedShellTarget {
                    role: ShellRole::Screenshot,
                    output: None,
                    x: 144,
                    y: 96,
                    interaction: PointerInteraction::LeftPress,
                },
            },
            TestInput::RecoveryPointer {
                action: RecoveryTargetAction::Retry,
                output: Some("DP-1".into()),
            },
            TestInput::WindowPointer {
                window: WindowId(7),
                interaction: PointerInteraction::LeftDoubleClick,
            },
        ] {
            let envelope = ClientEnvelope {
                token: "test-capability".into(),
                request_id: 9,
                request: Request::Command(Command::TestInput { input }),
            };
            assert_eq!(
                decode::<ClientEnvelope>(&encode(&envelope).unwrap()).unwrap(),
                envelope
            );
        }
    }

    #[test]
    fn semantic_shell_target_query_and_response_round_trip() {
        let target = ShellSemanticTarget::PreviewWindow {
            window: WindowId(11),
            action: PreviewTargetAction::Close,
        };
        let request = ClientEnvelope {
            token: "test-capability".into(),
            request_id: 12,
            request: Request::Query(Query::ShellSemanticTarget { target }),
        };
        assert_eq!(
            decode::<ClientEnvelope>(&encode(&request).unwrap()).unwrap(),
            request
        );
        let response = ServerEnvelope {
            request_id: 12,
            message: ServerMessage::ShellSemanticTarget(ResolvedShellTarget {
                role: ShellRole::Preview,
                output: None,
                x: 24,
                y: 32,
                interaction: PointerInteraction::LeftClick,
            }),
        };
        assert_eq!(
            decode::<ServerEnvelope>(&encode(&response).unwrap()).unwrap(),
            response
        );

        let screenshot = ClientEnvelope {
            token: "test-capability".into(),
            request_id: 13,
            request: Request::Query(Query::ShellSemanticTarget {
                target: ShellSemanticTarget::Screenshot {
                    action: ScreenshotTargetAction::SelectionStart,
                },
            }),
        };
        assert_eq!(
            decode::<ClientEnvelope>(&encode(&screenshot).unwrap()).unwrap(),
            screenshot
        );
        let screenshot_response = ServerEnvelope {
            request_id: 13,
            message: ServerMessage::Ack,
        };
        assert_eq!(
            decode::<ServerEnvelope>(&encode(&screenshot_response).unwrap()).unwrap(),
            screenshot_response
        );
    }

    #[test]
    fn shell_runtime_diagnostics_are_versioned_bounded_and_explicit() {
        let request = ClientEnvelope {
            token: "test-capability".into(),
            request_id: 14,
            request: Request::Query(Query::ShellRuntimeDiagnostics),
        };
        assert_eq!(
            decode::<ClientEnvelope>(&encode(&request).unwrap()).unwrap(),
            request
        );

        let diagnostics = ShellRuntimeDiagnostics {
            input_to_message_us: vec![120; MAX_RUNTIME_PERFORMANCE_SAMPLES],
            input_to_frame_us: vec![480; MAX_RUNTIME_PERFORMANCE_SAMPLES],
            layout_us: vec![210; MAX_RUNTIME_PERFORMANCE_SAMPLES],
            paint_list_us: vec![90; MAX_RUNTIME_PERFORMANCE_SAMPLES],
            warm_present_us: vec![950; MAX_RUNTIME_PERFORMANCE_SAMPLES],
            input_to_visible_us: vec![2_400; MAX_RUNTIME_PERFORMANCE_SAMPLES],
            scheduled_wakeups: 3,
            host_phase_samples_available: true,
            retained_presenter_bytes: 1_048_576,
            frame_allocations: AllocationMeasurement {
                count: Some(0),
                sample_count: MAX_RUNTIME_PERFORMANCE_SAMPLES,
                scope: AllocationScope::Process,
                unavailable_reason: None,
            },
            executable_prediction_observations: [[1, 2], [3, 4], [5, 6], [7, 8]],
            executable_prediction_descendant_windows: 9,
        };
        assert_eq!(diagnostics.validate(), Ok(()));
        let json = serde_json::to_value(&diagnostics).unwrap();
        assert_eq!(json["warm_present_us"][0], 950);
        assert_eq!(json["input_to_frame_us"][0], 480);
        assert_eq!(json["frame_allocations"]["scope"], "process");
        assert_eq!(json["executable_prediction_observations"][0][1], 2);
        assert_eq!(json["executable_prediction_descendant_windows"], 9);
        let response = ServerEnvelope {
            request_id: 14,
            message: ServerMessage::ShellRuntimeDiagnostics(diagnostics.clone()),
        };
        assert_eq!(
            decode::<ServerEnvelope>(&encode(&response).unwrap()).unwrap(),
            response
        );

        let mut oversized = diagnostics;
        oversized.warm_present_us.push(950);
        assert_eq!(oversized.validate(), Err(FrameError::TooLarge));
    }

    #[test]
    fn allocation_measurement_never_uses_a_zero_as_unavailable_evidence() {
        let missing_reason = AllocationMeasurement {
            count: None,
            sample_count: 0,
            scope: AllocationScope::Unavailable,
            unavailable_reason: None,
        };
        assert!(missing_reason.validate().is_err());
        let fake_zero = AllocationMeasurement {
            count: Some(0),
            sample_count: 0,
            scope: AllocationScope::Process,
            unavailable_reason: None,
        };
        assert!(fake_zero.validate().is_err());
        assert_eq!(AllocationMeasurement::default().validate(), Ok(()));
    }

    #[test]
    fn nested_test_output_commands_round_trip() {
        for output in [
            TestOutput::Connect {
                name: "DP-test".into(),
                logical_width: 1024,
                logical_height: 768,
                scale_120: 180,
                transform: OutputTransform::Rotate90,
            },
            TestOutput::Disconnect {
                name: "DP-test".into(),
            },
        ] {
            let envelope = ClientEnvelope {
                token: "test-capability".into(),
                request_id: 10,
                request: Request::Command(Command::TestOutput { output }),
            };
            assert_eq!(
                decode::<ClientEnvelope>(&encode(&envelope).unwrap()).unwrap(),
                envelope
            );
        }
    }

    #[test]
    fn output_layout_round_trips_exact_fractional_scale() {
        let request = Request::Command(Command::ApplyOutputs {
            layout: OutputLayout {
                primary: "edid-primary".into(),
                placements: vec![OutputPlacement {
                    name: "edid-primary".into(),
                    x: -1920,
                    y: 37,
                    enabled: true,
                    scale_120: 150,
                }],
            },
        });
        assert_eq!(
            decode::<Request>(&encode(&request).unwrap()).unwrap(),
            request
        );
    }

    #[test]
    fn workspace_commands_round_trip_with_stable_ids_and_output_identity() {
        for command in [
            Command::CreateWorkspace,
            Command::RemoveWorkspace {
                workspace: WorkspaceId(3),
            },
            Command::SwitchWorkspace {
                workspace: WorkspaceId(7),
                output: Some("DP-2".into()),
            },
            Command::MoveWindowToWorkspace {
                window: WindowId(11),
                workspace: WorkspaceId(7),
            },
            Command::MoveWindowToOutput {
                window: WindowId(11),
                output: "HDMI-A-1".into(),
            },
        ] {
            let envelope = ClientEnvelope {
                token: "session-token".into(),
                request_id: 12,
                request: Request::Command(command),
            };
            assert_eq!(
                decode::<ClientEnvelope>(&encode(&envelope).unwrap()).unwrap(),
                envelope
            );
        }
    }

    #[test]
    fn shell_role_ids_are_explicit_and_round_trip() {
        for role in [
            ShellRole::Desktop,
            ShellRole::Panel,
            ShellRole::Launcher,
            ShellRole::ControlCenter,
            ShellRole::ContextMenu,
            ShellRole::Preview,
            ShellRole::Notification,
            ShellRole::VolumeOsd,
            ShellRole::ProjectMenu,
            ShellRole::Lock,
            ShellRole::Screenshot,
            ShellRole::Recovery,
        ] {
            assert_eq!(
                ShellRole::from_application_id(role.application_id()),
                Some(role)
            );
        }
        assert_eq!(ShellRole::from_application_id("io.nickel.shell.fake"), None);
    }

    #[test]
    fn shell_focus_handoff_round_trips_with_typed_roles() {
        for command in [
            Command::FocusShellRole {
                role: ShellRole::ControlCenter,
            },
            Command::RestoreApplicationFocus,
        ] {
            let envelope = ClientEnvelope {
                token: "session-token".into(),
                request_id: 13,
                request: Request::Command(command),
            };
            assert_eq!(
                decode::<ClientEnvelope>(&encode(&envelope).unwrap()).unwrap(),
                envelope
            );
        }
    }

    #[test]
    fn shell_visibility_round_trips_with_a_typed_role() {
        for visible in [false, true] {
            let envelope = ClientEnvelope {
                token: "session-token".into(),
                request_id: 19,
                request: Request::Command(Command::SetShellRoleVisible {
                    role: ShellRole::VolumeOsd,
                    visible,
                }),
            };
            assert_eq!(
                decode::<ClientEnvelope>(&encode(&envelope).unwrap()).unwrap(),
                envelope
            );
        }
    }

    #[test]
    fn semantic_shell_popover_anchor_round_trips_with_output_scope() {
        let envelope = ClientEnvelope {
            token: "session-token".into(),
            request_id: 20,
            request: Request::Command(Command::ShowAnchoredShellRole {
                role: ShellRole::ControlCenter,
                anchor: ShellPopoverAnchor {
                    control: "panel-control".into(),
                    output: "HDMI-A-1".into(),
                    bounds: Geometry {
                        x: 1720,
                        y: 0,
                        width: 96,
                        height: 56,
                    },
                    preferred: AnchorSide::Above,
                },
            }),
        };
        assert_eq!(
            decode::<ClientEnvelope>(&encode(&envelope).unwrap()).unwrap(),
            envelope
        );
    }

    #[test]
    fn session_actions_round_trip_without_ui_specific_authorization_state() {
        for action in [
            SessionAction::RestartShell,
            SessionAction::Lock,
            SessionAction::Suspend,
            SessionAction::Reboot,
            SessionAction::PowerOff,
        ] {
            let envelope = ClientEnvelope {
                token: "session-token".into(),
                request_id: 14,
                request: Request::Command(Command::SessionAction { action }),
            };
            assert_eq!(
                decode::<ClientEnvelope>(&encode(&envelope).unwrap()).unwrap(),
                envelope
            );
        }
        let envelope = ClientEnvelope {
            token: "session-token".into(),
            request_id: 15,
            request: Request::Command(Command::Unlock),
        };
        assert_eq!(
            decode::<ClientEnvelope>(&encode(&envelope).unwrap()).unwrap(),
            envelope
        );
    }

    #[test]
    fn reconnect_snapshot_round_trips_without_native_objects() {
        let snapshot = Snapshot {
            outputs: vec![OutputSnapshot {
                name: "DP-1".into(),
                model: "Nested output".into(),
                geometry: Geometry {
                    x: 0,
                    y: 0,
                    width: 1280,
                    height: 720,
                },
                work_area: Geometry {
                    x: 0,
                    y: 0,
                    width: 1280,
                    height: 672,
                },
                scale_120: 180,
                transform: OutputTransform::Rotate90,
                physical_width_mm: 300,
                physical_height_mm: 170,
                primary: true,
                enabled: true,
            }],
            windows: vec![WindowSnapshot {
                id: WindowId(9),
                application_id: "org.example.Editor".into(),
                title: "notes".into(),
                active: true,
                minimized: false,
                maximized: false,
                fullscreen: false,
                geometry: Some(Geometry {
                    x: 32,
                    y: 32,
                    width: 800,
                    height: 600,
                }),
                workspace: WorkspaceId(1),
            }],
            focused: Some(WindowId(9)),
            stacking_front_to_back: vec![WindowId(9)],
            launcher_visible: false,
            locked: false,
            workspaces: WorkspaceState {
                active: WorkspaceId(1),
                active_output: Some("DP-1".into()),
                ordered: vec![WorkspaceSnapshot {
                    id: WorkspaceId(1),
                    windows: vec![WindowId(9)],
                    focused: Some(WindowId(9)),
                }],
            },
        };
        let envelope = ServerEnvelope {
            request_id: 44,
            message: ServerMessage::Snapshot(snapshot.clone()),
        };
        let restored = decode::<ServerEnvelope>(&encode(&envelope).unwrap()).unwrap();
        assert_eq!(restored.message, ServerMessage::Snapshot(snapshot));
    }

    #[test]
    fn sequence_response_and_event_variants_round_trip() {
        let window = WindowSnapshot {
            id: WindowId(7),
            application_id: "org.example.Editor".into(),
            title: "notes".into(),
            active: true,
            minimized: false,
            maximized: false,
            fullscreen: false,
            geometry: None,
            workspace: WorkspaceId(1),
        };
        for message in [
            ServerMessage::Windows(vec![window.clone()]),
            ServerMessage::Event(Event::Windows(vec![window.clone()])),
            ServerMessage::Event(Event::Stacking {
                front_to_back: vec![window.id],
            }),
            ServerMessage::Workspaces(WorkspaceState {
                active: WorkspaceId(1),
                active_output: None,
                ordered: vec![WorkspaceSnapshot {
                    id: WorkspaceId(1),
                    windows: vec![window.id],
                    focused: Some(window.id),
                }],
            }),
        ] {
            let envelope = ServerEnvelope {
                request_id: 3,
                message: message.clone(),
            };
            assert_eq!(
                decode::<ServerEnvelope>(&encode(&envelope).unwrap())
                    .unwrap()
                    .message,
                message
            );
        }
    }

    #[test]
    fn shell_surface_diagnostics_round_trip_authoritative_placement() {
        let message = ServerMessage::ShellSurfaces(vec![ShellSurfaceSnapshot {
            role: ShellRole::Launcher,
            geometry: Some(Geometry {
                x: 1298,
                y: 24,
                width: 920,
                height: 680,
            }),
            output: Some("DP-test".into()),
        }]);
        let envelope = ServerEnvelope {
            request_id: 18,
            message: message.clone(),
        };
        assert_eq!(
            decode::<ServerEnvelope>(&encode(&envelope).unwrap())
                .unwrap()
                .message,
            message
        );
    }

    #[test]
    fn shell_readiness_diagnostics_round_trip_generation_and_invariants() {
        let message = ServerMessage::ShellReadiness(ShellReadinessSnapshot {
            expected_shell_pid: Some(42),
            authenticated_shell_pid: Some(42),
            outputs: 2,
            desktops: 2,
            panels: 2,
            locks: 2,
            launchers: 1,
            required_singletons_ready: true,
            output_roles_ready: true,
            reserved_ordinary_windows: 0,
            ready: true,
        });
        let envelope = ServerEnvelope {
            request_id: 19,
            message: message.clone(),
        };
        assert_eq!(
            decode::<ServerEnvelope>(&encode(&envelope).unwrap())
                .unwrap()
                .message,
            message
        );
    }

    #[test]
    fn production_sized_preview_fits_the_wire_frame() {
        let preview = PreviewFrame {
            window: WindowId(11),
            width: 240,
            height: 135,
            rgba: vec![0xab; 240 * 135 * 4],
        };
        let envelope = ServerEnvelope {
            request_id: 7,
            message: ServerMessage::Preview(preview.clone()),
        };
        let encoded = encode(&envelope).expect("production preview fits the protocol frame");
        assert_eq!(
            decode::<ServerEnvelope>(&encoded).unwrap().message,
            ServerMessage::Preview(preview)
        );
    }

    #[test]
    fn maximum_bounded_window_metadata_population_fits_one_wire_response() {
        let windows = (0..MAX_WINDOWS)
            .map(|index| WindowSnapshot {
                id: WindowId(index as u64 + 1),
                // Backslash exercises the maximum expansion admitted by the
                // canonical projection (ASCII controls are normalized).
                application_id: "\\".repeat(MAX_WINDOW_APP_ID_BYTES),
                title: "\\".repeat(MAX_WINDOW_TITLE_BYTES),
                active: index == 0,
                minimized: false,
                maximized: false,
                fullscreen: false,
                geometry: Some(Geometry {
                    x: i32::MAX,
                    y: i32::MIN,
                    width: i32::MAX,
                    height: i32::MAX,
                }),
                workspace: WorkspaceId(u64::MAX),
            })
            .collect::<Vec<_>>();
        let ids = windows.iter().map(|window| window.id).collect::<Vec<_>>();
        let outputs = (0..MAX_OUTPUTS)
            .map(|index| OutputSnapshot {
                name: format!("connector-{index:02}"),
                model: "model".repeat(32),
                geometry: Geometry {
                    x: i32::MAX,
                    y: i32::MIN,
                    width: i32::MAX,
                    height: i32::MAX,
                },
                work_area: Geometry {
                    x: i32::MAX,
                    y: i32::MIN,
                    width: i32::MAX,
                    height: i32::MAX,
                },
                scale_120: u32::MAX,
                transform: OutputTransform::Flipped270,
                physical_width_mm: i32::MAX,
                physical_height_mm: i32::MAX,
                primary: index == 0,
                enabled: true,
            })
            .collect();
        let envelope = ServerEnvelope {
            request_id: u64::MAX,
            message: ServerMessage::Event(Event::Snapshot(Snapshot {
                outputs,
                windows: windows.clone(),
                focused: Some(windows[0].id),
                stacking_front_to_back: ids.clone(),
                launcher_visible: true,
                locked: true,
                workspaces: WorkspaceState {
                    active: WorkspaceId(1),
                    active_output: Some("connector-00".into()),
                    ordered: vec![WorkspaceSnapshot {
                        id: WorkspaceId(1),
                        windows: ids,
                        focused: Some(windows[0].id),
                    }],
                },
            })),
        };

        let encoded = encode(&envelope).expect("bounded metadata population fits one frame");
        assert!(encoded.len() <= MAX_FRAME_BYTES);
        assert_eq!(decode::<ServerEnvelope>(&encoded).unwrap(), envelope);
    }

    #[test]
    fn shell_behavior_transaction_and_effective_ack_round_trip() {
        let request = ClientEnvelope {
            token: "session-token".into(),
            request_id: 42,
            request: Request::Command(Command::ApplyShellBehavior {
                transaction: ShellBehaviorTransaction {
                    setting: ShellBehaviorSetting::DesktopCount,
                    prior: ShellBehaviorValue::Count(4),
                    requested: ShellBehaviorValue::Count(6),
                    topology_generation: 9,
                },
            }),
        };
        assert_eq!(
            decode::<ClientEnvelope>(&encode(&request).unwrap()).unwrap(),
            request
        );

        let response = ServerEnvelope {
            request_id: 42,
            message: ServerMessage::ShellBehavior(ShellBehaviorSnapshot {
                bar_on_all_displays: true,
                all_windows_on_every_bar: false,
                desktop_count: 6,
                topology_generation: 9,
            }),
        };
        assert_eq!(
            decode::<ServerEnvelope>(&encode(&response).unwrap()).unwrap(),
            response
        );
    }

    #[test]
    fn pending_launch_authority_round_trips_without_command_text() {
        let request = ClientEnvelope {
            token: "session-token".into(),
            request_id: 91,
            request: Request::Command(Command::ObservePendingLaunch {
                generation: 27,
                root_pid: 4_242,
                root_start_time: 91_337,
                deadline_ms: 100,
            }),
        };
        assert_eq!(
            decode::<ClientEnvelope>(&encode(&request).unwrap()).unwrap(),
            request
        );
        let event = ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(Event::PendingLaunchWindow {
                generation: 27,
                observed_after_ms: 99,
                descendant: true,
            }),
        };
        assert_eq!(
            decode::<ServerEnvelope>(&encode(&event).unwrap()).unwrap(),
            event
        );
        let expired = ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(Event::PendingLaunchExpired { generation: 27 }),
        };
        assert_eq!(
            decode::<ServerEnvelope>(&encode(&expired).unwrap()).unwrap(),
            expired
        );
    }
}
