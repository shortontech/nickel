mod bundle;
mod client;
pub mod delivery;
mod process;
mod protocol;
mod replay;
mod selection;
mod settings;

pub use client::{CodexClient, ConnectionState};
pub use protocol::{
    AccountState, ApprovalContext, ApprovalPolicy, CodexBackend, CodexError, CodexEvent,
    CommandAction, CommandDecision, CompletedItem, EventKind, FileChangeDecision, FilePatchChange,
    FileSearchMatch, ImportProject, InteractionResponse, LoginChallenge, LoginCompletion,
    LoginMethod, Model, NetworkPolicyAction, NetworkPolicyAmendment, Project, ProjectPage,
    ProjectPageResult, ProjectedItem, ProjectedThread, Projection, RateLimitBucket,
    RateLimitsStatus, ReasoningEffortOption, RemoteControlClient, RemoteControlClientPage,
    RemoteControlConnectionStatus, RemoteControlStatus, RemotePairingChallenge, ReviewSettings,
    SandboxPolicy, ServerRequestId, StartThread, StartTurn, Thread, ThreadHistoryItem,
    ThreadHistoryTurn, ThreadId, ThreadPage, ThreadPageResult, ThreadRuntime, ThreadRuntimeStatus,
    Turn, TurnId, TurnImage, TurnPlanStep, UserInputAnswer, UserInputOption, UserInputQuestion,
};
pub use replay::{ReplayBackend, ReplayScenario};
pub use selection::{
    BackendChoice, Candidate, CandidateSource, Compatibility, ProbeLimits, Selection, Selector,
};
pub use settings::{CodexSettings, RemoteHost, SettingsError};

pub const REQUIRED_PROFILE: &str = include_str!("../protocol/required-profile.json");
pub use bundle::{BundleArtifact, BundleManifest, stage_bundle};
