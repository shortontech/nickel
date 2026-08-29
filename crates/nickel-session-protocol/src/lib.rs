use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 196_608;
pub const MAX_PREVIEW_WIDTH: u16 = 256;
pub const MAX_PREVIEW_HEIGHT: u16 = 144;
pub const MAX_SUBSCRIBERS: usize = 8;
pub const MAX_WINDOWS: usize = 1_024;
pub const MAX_OUTPUTS: usize = 32;

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
    LauncherVisibility,
    SecureStorage,
    Preview { window: WindowId },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    ToggleLauncher,
    SetLauncherVisible {
        visible: bool,
    },
    LogOut,
    RetrySecureStorage,
    HideOverlay,
    ShowOverlay {
        role: ShellRole,
        geometry: Geometry,
    },
    IdentifyOutputs,
    CaptureOutput {
        path: String,
    },
    ApplyOutputs {
        layout: OutputLayout,
    },
    HighlightWindow {
        window: Option<WindowId>,
    },
    WindowAction {
        window: WindowId,
        action: WindowAction,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "response", rename_all = "snake_case")]
pub enum ServerMessage {
    Ack,
    Error { code: ErrorCode, message: String },
    Snapshot(Snapshot),
    Windows(Vec<WindowSnapshot>),
    Outputs(Vec<OutputSnapshot>),
    LauncherVisibility { visible: bool },
    SecureStorage { state: SecureStorageState },
    Preview(PreviewFrame),
    Event(Event),
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
#[serde(tag = "event", rename_all = "snake_case")]
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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputSnapshot {
    pub name: String,
    pub model: String,
    pub geometry: Geometry,
    pub work_area: Geometry,
    pub physical_width_mm: i32,
    pub physical_height_mm: i32,
    pub primary: bool,
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
    pub rgba: Vec<u8>,
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
                physical_width_mm: 300,
                physical_height_mm: 170,
                primary: true,
            }],
            windows: vec![WindowSnapshot {
                id: WindowId(9),
                application_id: "org.example.Editor".into(),
                title: "notes".into(),
                active: true,
                minimized: false,
                maximized: false,
            }],
            focused: Some(WindowId(9)),
            stacking_front_to_back: vec![WindowId(9)],
            launcher_visible: false,
        };
        let envelope = ServerEnvelope {
            request_id: 44,
            message: ServerMessage::Snapshot(snapshot.clone()),
        };
        let restored = decode::<ServerEnvelope>(&encode(&envelope).unwrap()).unwrap();
        assert_eq!(restored.message, ServerMessage::Snapshot(snapshot));
    }
}
