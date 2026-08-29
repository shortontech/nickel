mod bundle;
mod client;
mod protocol;
mod replay;
mod selection;
mod settings;

pub use client::{CodexClient, ConnectionState};
pub use protocol::{
    AccountState, CodexBackend, CodexError, CodexEvent, CommandAction, CommandDecision, EventKind,
    FileChangeDecision, ImportProject, InteractionResponse, Model, NetworkPolicyAction,
    NetworkPolicyAmendment, Project, ProjectPage, ProjectPageResult, ProjectedItem,
    ProjectedThread, Projection, ReasoningEffortOption, ServerRequestId, StartThread, StartTurn,
    Thread, ThreadHistoryItem, ThreadHistoryTurn, ThreadId, ThreadPage, ThreadPageResult,
    ThreadRuntime, ThreadRuntimeStatus, Turn, TurnId, UserInputAnswer,
};
pub use replay::{ReplayBackend, ReplayScenario};
pub use selection::{
    BackendChoice, Candidate, CandidateSource, Compatibility, ProbeLimits, Selection, Selector,
};
pub use settings::{CodexSettings, RemoteHost, SettingsError};

pub const REQUIRED_PROFILE: &str = include_str!("../protocol/required-profile.json");
pub use bundle::{BundleArtifact, BundleManifest, stage_bundle};
