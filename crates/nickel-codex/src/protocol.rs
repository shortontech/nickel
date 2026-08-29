use std::{path::PathBuf, sync::mpsc::Receiver};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CodexError {
    #[error("Codex unavailable: {0}")]
    Unavailable(String),
    #[error("incompatible Codex CLI: {0}")]
    Incompatible(String),
    #[error("Codex protocol error: {0}")]
    Protocol(String),
    #[error("Codex operation timed out: {0}")]
    Timeout(String),
    #[error("Codex process stopped: {0}")]
    Stopped(String),
    #[error("invalid interaction response: {0}")]
    InvalidInteraction(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ThreadId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServerRequestId(pub String);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AccountState {
    pub authenticated: bool,
    pub account_type: Option<String>,
    pub email: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub default_reasoning_effort: Option<String>,
    #[serde(default)]
    pub supported_reasoning_efforts: Vec<ReasoningEffortOption>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningEffortOption {
    pub reasoning_effort: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub roots: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreadRuntimeStatus {
    NotLoaded,
    Idle,
    Active,
    SystemError,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRuntime {
    pub project_id: Option<String>,
    pub status: ThreadRuntimeStatus,
    pub active_flags: Vec<String>,
    pub can_accept_direct_input: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Thread {
    pub id: ThreadId,
    pub title: Option<String>,
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub last_used_at: Option<i64>,
    #[serde(default)]
    pub turns: Vec<ThreadHistoryTurn>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThreadHistoryTurn {
    pub id: TurnId,
    pub status: String,
    pub items: Vec<ThreadHistoryItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThreadHistoryItem {
    pub id: String,
    pub item_type: String,
    pub text: String,
    pub command_actions: Vec<CommandAction>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CommandAction {
    Read {
        name: String,
        path: String,
    },
    ListFiles {
        path: Option<String>,
    },
    Search {
        query: Option<String>,
        path: Option<String>,
    },
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Turn {
    pub id: TurnId,
    pub thread_id: ThreadId,
    pub status: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Projection {
    pub threads: HashMap<ThreadId, ProjectedThread>,
    pub active_turn: Option<TurnId>,
    pub items: HashMap<String, ProjectedItem>,
    pub terminal_error: Option<String>,
}

use std::collections::HashMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProjectedThread {
    pub active_turn: Option<TurnId>,
    pub terminal_turns: Vec<TurnId>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProjectedItem {
    pub item_type: String,
    pub text: String,
    pub completed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ThreadPage {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct ThreadPageResult {
    pub threads: Vec<Thread>,
    pub next_cursor: Option<String>,
    pub runtime: HashMap<ThreadId, ThreadRuntime>,
}

#[derive(Clone, Debug, Default)]
pub struct ProjectPage {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct ProjectPageResult {
    pub projects: Vec<Project>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ImportProject {
    pub idempotency_key: String,
    pub name: String,
    pub roots: Vec<PathBuf>,
    pub threads: Vec<ThreadId>,
}

#[derive(Clone, Debug)]
pub struct StartThread {
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub project_id: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Clone, Debug)]
pub struct StartTurn {
    pub thread_id: ThreadId,
    pub text: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InteractionResponse {
    CommandApproval { decision: CommandDecision },
    FileChangeApproval { decision: FileChangeDecision },
    UserInput { answers: Vec<UserInputAnswer> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommandDecision {
    Accept,
    AcceptForSession,
    AcceptWithExecpolicyAmendment {
        execpolicy_amendment: Vec<String>,
    },
    ApplyNetworkPolicyAmendment {
        network_policy_amendment: NetworkPolicyAmendment,
    },
    Decline,
    Cancel,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileChangeDecision {
    Accept,
    AcceptForSession,
    Decline,
    Cancel,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkPolicyAmendment {
    pub host: String,
    pub action: NetworkPolicyAction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkPolicyAction {
    Allow,
    Deny,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserInputAnswer {
    pub question_id: String,
    pub answer: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodexEvent {
    pub sequence: u64,
    pub kind: EventKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    Connection {
        state: String,
    },
    ThreadStarted {
        thread_id: ThreadId,
    },
    TurnStarted {
        thread_id: ThreadId,
        turn_id: TurnId,
    },
    TurnCompleted {
        thread_id: ThreadId,
        turn_id: TurnId,
        status: String,
    },
    ItemStarted {
        thread_id: Option<ThreadId>,
        turn_id: Option<TurnId>,
        item_id: String,
        item_type: String,
        command_actions: Vec<CommandAction>,
        initial_text: String,
    },
    ItemCompleted {
        item_id: String,
    },
    AgentMessageDelta {
        item_id: String,
        delta: String,
    },
    CommandOutputDelta {
        item_id: String,
        delta: String,
    },
    FileChangeDelta {
        item_id: String,
        delta: String,
    },
    PlanDelta {
        item_id: String,
        delta: String,
    },
    ReasoningDelta {
        item_id: String,
        delta: String,
    },
    ApprovalRequested {
        request_id: ServerRequestId,
        approval_type: String,
        summary: Option<String>,
    },
    UserInputRequested {
        request_id: ServerRequestId,
        question_ids: Vec<String>,
    },
    AccountUpdated,
    Error {
        message: String,
    },
    UnsupportedEvent {
        method: String,
    },
    Inconsistency {
        message: String,
    },
}

pub trait CodexBackend {
    fn account(&self) -> Result<AccountState, CodexError>;
    fn models(&self) -> Result<Vec<Model>, CodexError>;
    fn list_projects(&self, page: ProjectPage) -> Result<ProjectPageResult, CodexError>;
    fn import_project(&self, project: ImportProject) -> Result<Project, CodexError>;
    fn list_threads(&self, page: ThreadPage) -> Result<ThreadPageResult, CodexError>;
    fn start_thread(&self, request: StartThread) -> Result<Thread, CodexError>;
    fn resume_thread(&self, id: ThreadId) -> Result<Thread, CodexError>;
    fn start_turn(&self, request: StartTurn) -> Result<Turn, CodexError>;
    fn shell_command(&self, thread: ThreadId, command: String) -> Result<(), CodexError>;
    fn interrupt_turn(&self, thread: ThreadId, turn: TurnId) -> Result<(), CodexError>;
    fn respond(
        &self,
        request: ServerRequestId,
        response: InteractionResponse,
    ) -> Result<(), CodexError>;
    fn subscribe(&self) -> Receiver<CodexEvent>;
}

pub(crate) fn request_id(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| value.as_i64().map(|id| id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_approval_variant_matches_app_server_shape() {
        let decisions = [
            CommandDecision::Accept,
            CommandDecision::AcceptForSession,
            CommandDecision::AcceptWithExecpolicyAmendment {
                execpolicy_amendment: vec!["prefix_rule(pattern=[\"cargo\", \"test\"])".into()],
            },
            CommandDecision::ApplyNetworkPolicyAmendment {
                network_policy_amendment: NetworkPolicyAmendment {
                    host: "example.invalid".into(),
                    action: NetworkPolicyAction::Deny,
                },
            },
            CommandDecision::Decline,
            CommandDecision::Cancel,
        ];
        let encoded: Vec<_> = decisions
            .into_iter()
            .map(|decision| serde_json::to_value(decision).unwrap())
            .collect();
        assert_eq!(encoded[0], "accept");
        assert_eq!(encoded[1], "acceptForSession");
        assert!(encoded[2].get("acceptWithExecpolicyAmendment").is_some());
        assert!(encoded[3].get("applyNetworkPolicyAmendment").is_some());
        assert_eq!(encoded[4], "decline");
        assert_eq!(encoded[5], "cancel");

        for decision in [
            FileChangeDecision::Accept,
            FileChangeDecision::AcceptForSession,
            FileChangeDecision::Decline,
            FileChangeDecision::Cancel,
        ] {
            assert!(serde_json::to_value(decision).unwrap().is_string());
        }
    }
}
