use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const PROTOCOL_VERSION: u16 = 8;
pub const MAX_FRAME_BYTES: usize = 196_608;
pub const MAX_PREVIEW_WIDTH: u16 = 256;
pub const MAX_PREVIEW_HEIGHT: u16 = 144;
pub const MAX_SUBSCRIBERS: usize = 8;
pub const MAX_WINDOWS: usize = 1_024;
pub const MAX_OUTPUTS: usize = 32;
pub const MAX_WORKSPACES: usize = 32;

const MAGIC: [u8; 4] = *b"NIKL";
const HEADER_BYTES: usize = 10;

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
    LauncherVisibility,
    SecureStorage,
    IdleInhibition,
    CacheDiagnostics,
    Workspaces,
    Preview {
        window: WindowId,
    },
    /// Resolve a semantic shell target through the live renderer records. This
    /// query is served by the shell's capability-gated nested test endpoint,
    /// not by the compositor control socket.
    ShellSemanticTarget {
        target: ShellSemanticTarget,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
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
    /// Dispatch a renderer-resolved shell-local pointer interaction through
    /// the compositor's ordinary absolute-motion and button paths.
    ShellPointer {
        target: ResolvedShellTarget,
    },
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
    Error { code: ErrorCode, message: String },
    Snapshot(Snapshot),
    Windows(Vec<WindowSnapshot>),
    Outputs(Vec<OutputSnapshot>),
    ShellSurfaces(Vec<ShellSurfaceSnapshot>),
    LauncherVisibility { visible: bool },
    SecureStorage { state: SecureStorageState },
    IdleInhibition { surfaces: u16 },
    CacheDiagnostics(CacheDiagnostics),
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
            TestInput::Key {
                key: TestKey::LeftAlt,
                state: InputState::Pressed,
            },
            TestInput::PointerMove { x: 640, y: 360 },
            TestInput::PointerMoveRelative { dx: 12, dy: -7 },
            TestInput::PointerButton {
                button: TestPointerButton::Left,
                state: InputState::Released,
            },
            TestInput::ShellPointer {
                target: ResolvedShellTarget {
                    role: ShellRole::Preview,
                    output: Some("DP-1".into()),
                    x: 144,
                    y: 96,
                    interaction: PointerInteraction::RightClick,
                },
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
}
