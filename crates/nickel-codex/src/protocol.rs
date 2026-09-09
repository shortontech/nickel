use crate::delivery::DeliveryReceiver as Receiver;
use std::path::PathBuf;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginMethod {
    Browser,
    DeviceCode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LoginChallenge {
    Browser {
        login_id: String,
        auth_url: String,
    },
    DeviceCode {
        login_id: String,
        user_code: String,
        verification_url: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginCompletion {
    pub login_id: Option<String>,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteControlConnectionStatus {
    Disabled,
    Connecting,
    Connected,
    Errored,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteControlStatus {
    pub status: RemoteControlConnectionStatus,
    pub server_name: String,
    pub installation_id: String,
    pub environment_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePairingChallenge {
    pub environment_id: String,
    pub expires_at: i64,
    /// Opaque payload intended for a locally rendered QR code. Never log it.
    pub pairing_code: String,
    pub manual_pairing_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteControlClient {
    pub client_id: String,
    pub display_name: Option<String>,
    pub device_model: Option<String>,
    pub device_type: Option<String>,
    pub platform: Option<String>,
    pub os_version: Option<String>,
    pub app_version: Option<String>,
    pub last_seen_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteControlClientPage {
    pub data: Vec<RemoteControlClient>,
    pub next_cursor: Option<String>,
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
    /// Compatibility field. Live projections retain lifecycle metadata, not transcript bodies.
    pub text: String,
    pub completed: bool,
}

pub const MAX_PROJECTED_ITEMS: usize = 2048;
pub const MAX_PROJECTED_THREADS: usize = 128;
pub const MAX_TERMINAL_TURNS: usize = 32;
pub const MAX_PROJECTION_BYTES: usize = 2 * 1024 * 1024;

impl Projection {
    /// Owned allocation estimate, including collection slots and string capacities.
    /// Transcript data lives in the UI or authoritative server history.
    pub fn retained_capacity(&self) -> usize {
        self.items.capacity() * (size_of::<String>() + size_of::<ProjectedItem>() + 1)
            + self.threads.capacity() * (size_of::<ThreadId>() + size_of::<ProjectedThread>() + 1)
            + self
                .items
                .iter()
                .map(|(id, item)| id.capacity() + item.item_type.capacity() + item.text.capacity())
                .sum::<usize>()
            + self
                .threads
                .iter()
                .map(|(id, thread)| {
                    id.0.capacity()
                        + thread.active_turn.as_ref().map_or(0, |id| id.0.capacity())
                        + thread.terminal_turns.capacity() * size_of::<TurnId>()
                        + thread
                            .terminal_turns
                            .iter()
                            .map(|id| id.0.capacity())
                            .sum::<usize>()
                })
                .sum::<usize>()
            + self.active_turn.as_ref().map_or(0, |id| id.0.capacity())
            + self.terminal_error.as_ref().map_or(0, String::capacity)
    }

    pub(crate) fn enforce_limits(&mut self) -> bool {
        for thread in self.threads.values_mut() {
            if thread.terminal_turns.len() > MAX_TERMINAL_TURNS {
                thread
                    .terminal_turns
                    .drain(..thread.terminal_turns.len() - MAX_TERMINAL_TURNS);
                thread.terminal_turns.shrink_to_fit();
            }
        }
        // Sorting avoids randomized HashMap iteration deciding what survives pressure.
        // Completed item metadata is always retired before active item metadata.
        while self.items.len() > MAX_PROJECTED_ITEMS
            || self.retained_capacity() > MAX_PROJECTION_BYTES
        {
            let key = self
                .items
                .iter()
                .filter(|(_, item)| item.completed)
                .map(|(id, _)| id)
                .min()
                .cloned();
            let Some(key) = key else { break };
            self.items.remove(&key);
            self.items.shrink_to_fit();
        }
        while self.threads.len() > MAX_PROJECTED_THREADS
            || self.retained_capacity() > MAX_PROJECTION_BYTES
        {
            let key = self
                .threads
                .iter()
                .filter(|(_, thread)| thread.active_turn.is_none())
                .map(|(id, _)| id)
                .min_by_key(|id| &id.0)
                .cloned();
            let Some(key) = key else { break };
            self.threads.remove(&key);
            self.threads.shrink_to_fit();
        }
        self.items.len() <= MAX_PROJECTED_ITEMS
            && self.threads.len() <= MAX_PROJECTED_THREADS
            && self.retained_capacity() <= MAX_PROJECTION_BYTES
    }
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

/// Stable app-server values controlling when Codex asks before taking an action.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalPolicy {
    Untrusted,
    OnFailure,
    #[default]
    OnRequest,
    Never,
}

#[derive(Clone, Debug)]
pub struct StartThread {
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub project_id: Option<String>,
    pub reasoning_effort: Option<String>,
    pub approval_policy: ApprovalPolicy,
}

#[derive(Clone, Debug)]
pub struct StartTurn {
    pub thread_id: ThreadId,
    pub text: String,
    /// In-memory images encoded as data URLs for the app-server `image` input.
    /// Callers must not persist or log these values.
    pub images: Vec<TurnImage>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub approval_policy: ApprovalPolicy,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnImage {
    pub data_url: String,
}

impl std::fmt::Debug for TurnImage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TurnImage")
            .field("encoded_bytes", &self.data_url.len())
            .finish_non_exhaustive()
    }
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
    AccountLoginCompleted {
        completion: LoginCompletion,
    },
    RemoteControlStatusChanged {
        status: RemoteControlStatus,
    },
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
    fn start_login(&self, _method: LoginMethod) -> Result<LoginChallenge, CodexError> {
        Err(CodexError::Unavailable(
            "account login is not supported by this backend".into(),
        ))
    }
    fn cancel_login(&self, _login_id: &str) -> Result<(), CodexError> {
        Err(CodexError::Unavailable(
            "account login is not supported by this backend".into(),
        ))
    }
    fn remote_control_status(&self) -> Result<RemoteControlStatus, CodexError> {
        Err(CodexError::Unavailable(
            "Codex phone access is not supported by this backend".into(),
        ))
    }
    fn enable_remote_control(&self, _ephemeral: bool) -> Result<RemoteControlStatus, CodexError> {
        Err(CodexError::Unavailable(
            "Codex phone access is not supported by this backend".into(),
        ))
    }
    fn disable_remote_control(&self, _ephemeral: bool) -> Result<RemoteControlStatus, CodexError> {
        Err(CodexError::Unavailable(
            "Codex phone access is not supported by this backend".into(),
        ))
    }
    fn start_remote_pairing(
        &self,
        _manual_code: bool,
    ) -> Result<RemotePairingChallenge, CodexError> {
        Err(CodexError::Unavailable(
            "Codex phone pairing is not supported by this backend".into(),
        ))
    }
    fn remote_pairing_claimed(
        &self,
        _pairing_code: Option<&str>,
        _manual_pairing_code: Option<&str>,
    ) -> Result<bool, CodexError> {
        Err(CodexError::Unavailable(
            "Codex phone pairing is not supported by this backend".into(),
        ))
    }
    fn remote_control_clients(
        &self,
        _environment_id: &str,
        _cursor: Option<&str>,
        _limit: usize,
    ) -> Result<RemoteControlClientPage, CodexError> {
        Err(CodexError::Unavailable(
            "Codex phone access is not supported by this backend".into(),
        ))
    }
    fn revoke_remote_control_client(
        &self,
        _environment_id: &str,
        _client_id: &str,
    ) -> Result<(), CodexError> {
        Err(CodexError::Unavailable(
            "Codex phone access is not supported by this backend".into(),
        ))
    }
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
    fn projection_churn_retires_completed_metadata_and_preserves_active_turn() {
        let mut projection = Projection {
            active_turn: Some(TurnId("active".into())),
            ..Projection::default()
        };
        for index in 0..5000 {
            projection.items.insert(
                format!("item-{index:05}"),
                ProjectedItem {
                    completed: true,
                    ..ProjectedItem::default()
                },
            );
            let thread = projection
                .threads
                .entry(ThreadId(format!("thread-{}", index / 64)))
                .or_default();
            thread.terminal_turns.push(TurnId(format!("turn-{index}")));
            assert!(projection.enforce_limits());
            assert!(projection.retained_capacity() <= MAX_PROJECTION_BYTES);
            assert!(projection.items.len() <= MAX_PROJECTED_ITEMS);
            assert!(
                projection
                    .threads
                    .values()
                    .all(|thread| thread.terminal_turns.len() <= MAX_TERMINAL_TURNS)
            );
        }
        assert_eq!(projection.active_turn, Some(TurnId("active".into())));
    }

    #[test]
    fn active_projection_capacity_requires_failure_instead_of_silent_eviction() {
        let mut projection = Projection::default();
        for index in 0..=MAX_PROJECTED_ITEMS {
            projection
                .items
                .insert(format!("item-{index}"), ProjectedItem::default());
        }
        assert!(!projection.enforce_limits());
        assert_eq!(projection.items.len(), MAX_PROJECTED_ITEMS + 1);
        assert!(projection.items.values().all(|item| !item.completed));
    }

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

    #[test]
    fn approval_policy_values_match_the_app_server_schema() {
        let values = [
            (ApprovalPolicy::Untrusted, "untrusted"),
            (ApprovalPolicy::OnFailure, "on-failure"),
            (ApprovalPolicy::OnRequest, "on-request"),
            (ApprovalPolicy::Never, "never"),
        ];
        for (policy, expected) in values {
            assert_eq!(serde_json::to_value(policy).unwrap(), expected);
            assert_eq!(
                serde_json::from_value::<ApprovalPolicy>(expected.into()).unwrap(),
                policy
            );
        }
        assert!(serde_json::from_value::<ApprovalPolicy>("sometimes".into()).is_err());
    }
}
