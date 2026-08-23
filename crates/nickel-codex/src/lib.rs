mod bundle;
mod client;
mod protocol;
mod replay;
mod selection;

pub use client::{CodexClient, ConnectionState};
pub use protocol::{
    AccountState, CodexBackend, CodexError, CodexEvent, CommandDecision, EventKind,
    FileChangeDecision, InteractionResponse, Model, NetworkPolicyAction, NetworkPolicyAmendment,
    ProjectedItem, ProjectedThread, Projection, ServerRequestId, StartThread, StartTurn, Thread,
    ThreadId, ThreadPage, ThreadPageResult, Turn, TurnId, UserInputAnswer,
};
pub use replay::{ReplayBackend, ReplayScenario};
pub use selection::{
    BackendChoice, Candidate, CandidateSource, Compatibility, ProbeLimits, Selection, Selector,
};

pub const REQUIRED_PROFILE: &str = include_str!("../protocol/required-profile.json");
pub use bundle::{BundleArtifact, BundleManifest, stage_bundle};
