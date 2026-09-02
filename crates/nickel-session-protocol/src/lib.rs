use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const PROTOCOL_VERSION: u16 = 17;
pub const MAX_FRAME_BYTES: usize = 196_608;
pub const MAX_PREVIEW_WIDTH: u16 = 256;
pub const MAX_PREVIEW_HEIGHT: u16 = 144;
pub const MAX_SUBSCRIBERS: usize = 8;
pub const MAX_WINDOWS: usize = 128;
pub const MAX_WINDOW_TITLE_BYTES: usize = 384;
pub const MAX_WINDOW_APP_ID_BYTES: usize = 96;
pub const MAX_OUTPUTS: usize = 32;
pub const MAX_WORKSPACES: usize = 32;
pub const MAX_RUNTIME_PERFORMANCE_SAMPLES: usize = 64;

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
    ReloadShellSettings,
    ToggleLauncher,
    SetLauncherVisible {
        visible: bool,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    PanelApplication {
        application_id: String,
        output: Option<String>,
        interaction: PointerInteraction,
    },
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
    CacheDiagnostics(CacheDiagnostics),
    ShellRuntimeDiagnostics(ShellRuntimeDiagnostics),
    Workspaces(WorkspaceState),
    Preview(PreviewFrame),
    ShellSemanticTarget(ResolvedShellTarget),
    Event(Event),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheDiagnostics {
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
    ShellSettingsChanged,
    Snapshot(Snapshot),
    LauncherVisibility { visible: bool },
    Windows(Vec<WindowSnapshot>),
    Outputs(Vec<OutputSnapshot>),
    Focus { window: Option<WindowId> },
    Stacking { front_to_back: Vec<WindowId> },
    WindowRemoved { window: WindowId },
    Preview(PreviewFrame),
    OutputCaptureCompleted { path: String, result: CaptureResult },
    Workspaces(WorkspaceState),
    LockState { locked: bool },
    GlobalShortcut { action: ShortcutAction },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutAction {
    ShowRun,
    ShowScreenshotTool,
    CaptureActiveWindow,
    CaptureActiveWindowToFile,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellRole {
    Desktop,
    Panel,
    Launcher,
    ControlCenter,
    ContextMenu,
    Preview,
    Notification,
    ProjectMenu,
    Lock,
    Screenshot,
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
            Self::ProjectMenu => "io.nickel.shell.project-menu",
            Self::Lock => "io.nickel.shell.lock",
            Self::Screenshot => "io.nickel.shell.screenshot",
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
            Self::ProjectMenu,
            Self::Lock,
            Self::Screenshot,
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
    use super::*;

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
        };
        assert_eq!(diagnostics.validate(), Ok(()));
        let json = serde_json::to_value(&diagnostics).unwrap();
        assert_eq!(json["warm_present_us"][0], 950);
        assert_eq!(json["input_to_frame_us"][0], 480);
        assert_eq!(json["frame_allocations"]["scope"], "process");
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
}
