#[macro_use]
mod transcript;
mod application;
use transcript::*;
mod navigation;
#[cfg(test)]
pub(crate) use navigation::resume_preview;
use navigation::*;
mod composer;
use composer::*;
mod diagnostics;
use diagnostics::*;
mod connection;
use connection::*;

use std::path::PathBuf;

use nickel_codex::{
    ApprovalPolicy, BackendChoice, CodexSettings, CommandDecision, RemoteHost, SandboxPolicy,
    ServerRequestId,
};
use twinkle_ui::SemanticRole;
use twinkle_ui::approval::ApprovalPresentation;
use twinkle_ui::prelude::*;

use crate::model::is_collapsible_activity;
#[cfg(test)]
use crate::model::item_markdown_document;
#[cfg(test)]
use crate::model::{ActivityOutcome, item_label};
use crate::{
    BackendMode, ChatController, ChatState, ConnectionStatus, ControllerCommand, ControllerEvent,
    PendingInteraction, create_managed_workspace,
};
#[cfg(test)]
use crate::{ChatItem, ChatItemKind};

fn semantic_theme() -> SemanticTheme {
    // Standalone fallback only. Embedded surfaces receive the shell's resolved
    // semantic theme through `ChatApplication::set_theme`.
    SemanticTheme::from_tokens(twinkle_ui::SemanticTokenSet::standard(
        0x101318, 0x171b22, 0x202630, 0x343d4b, 0x343d4b, 0xe8edf4, 0x9ca8b8, 0x70a5ff, 0x1d3557,
        0x63d69a, 0x63d69a,
    ))
}
#[cfg(test)]
static DEFAULT_CODEX_SETTINGS: std::sync::LazyLock<CodexSettings> =
    std::sync::LazyLock::new(CodexSettings::default);

#[derive(Clone, Debug, PartialEq)]
pub enum ChatMessage {
    DraftChanged(String),
    PasteImage(Vec<u8>),
    RemoveAttachment(crate::AttachmentId),
    Send,
    ConfirmShell,
    CancelShell,
    NewChat,
    NewChatIn(PathBuf, String),
    Refresh,
    StartLogin(nickel_codex::LoginMethod),
    CancelLogin(String),
    OpenLoginUrl(String),
    CopyLoginText(String),
    OpenRemoteControl,
    CloseRemoteControl,
    RefreshRemoteControl,
    EnableRemoteControl,
    DisableRemoteControl,
    StartRemotePairing,
    CancelRemotePairing,
    RevokeRemoteClient(String, String),
    Reconnect,
    ToggleDiagnostics,
    ToggleDiagnosticDetails,
    CopyDiagnosticSummary,
    SelectThread(nickel_codex::ThreadId),
    ToggleModelPicker,
    ToggleReasoningPicker,
    ToggleApprovalPicker,
    ToggleSandboxPicker,
    ToggleRunSettings,
    CloseRunSettings,
    SelectModel(String),
    SelectReasoningEffort(String),
    SelectApprovalPolicy(ApprovalPolicy),
    SelectSandboxPolicy(Option<SandboxPolicy>),
    SelectCommand(String),
    SelectMention(String),
    SelectFeedbackCategory(String),
    FeedbackReasonChanged(String),
    SubmitFeedback(bool),
    CloseFeedback,
    ToggleResumePicker,
    RefreshResumePicker,
    LoadMoreThreads,
    CloseResumePicker,
    ConfirmDiscardDraft,
    CancelDiscardDraft,
    Interrupt,
    QueueMessage,
    InterruptAndSend,
    CancelQueuedMessage(u64),
    EditQueuedMessage(u64),
    RetryQueuedMessage(u64),
    RespondApproval(ServerRequestId, String, CodexApprovalChoice),
    InteractionAnswerChanged(String),
    SubmitInput(ServerRequestId, Vec<String>),
    DismissInput(ServerRequestId),
    ConversationScrolled(twinkle_ui::ScrollExtent),
    JumpToLatest,
    ToggleActivityDetails(String),
    ToggleProject(String),
    ToggleProjectCollapsed(String),
    ToggleFileMenu,
    SelectConnection(String),
    ManageRemoteHosts,
    CloseRemoteHosts,
    AddRemoteHost,
    EditRemoteHost(String),
    RemoveRemoteHost(String),
    RemoteHostIdChanged(String),
    RemoteHostNameChanged(String),
    RemoteHostEndpointChanged(String),
    RemoteHostTokenEnvChanged(String),
    RemoteHostCwdChanged(String),
    SaveRemoteHost,
    OpenMarkdownLink(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellRequest {
    CloseProjectMenu,
    OpenProject {
        cwd: PathBuf,
        project_id: String,
        name: String,
        initial_thread: Option<nickel_codex::ThreadId>,
    },
    ResumeThread(nickel_codex::ThreadId),
    ResumeSucceeded(nickel_codex::ThreadId),
    ResumeFailed(nickel_codex::ThreadId),
}

/// Read-only request snapshot for a shell notification; the Codex controller remains authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexApprovalNotification {
    pub connection_generation: u64,
    pub request_revision: u64,
    pub thread_id: Option<nickel_codex::ThreadId>,
    pub interaction: PendingInteraction,
    pub presentation: ApprovalPresentation,
    pub actionable: bool,
    pub submitting: bool,
    pub unconfirmed: bool,
}

impl CodexApprovalNotification {
    fn choices(&self) -> Vec<(String, CodexApprovalChoice)> {
        let PendingInteraction::Approval {
            approval_type,
            context,
            ..
        } = &self.interaction
        else {
            return Vec::new();
        };
        approval_choices(approval_type, context)
    }

    pub fn needs_review(&self) -> bool {
        let choices = self.choices();
        choices.len() > 3
            || choices.iter().any(|(_, choice)| {
                matches!(
                    choice,
                    CodexApprovalChoice::Command(
                        CommandDecision::AcceptForSession
                            | CommandDecision::AcceptWithExecpolicyAmendment { .. }
                            | CommandDecision::ApplyNetworkPolicyAmendment { .. }
                    )
                )
            })
    }

    pub fn notification_actions(&self) -> Vec<(String, String)> {
        let choices = self.choices();
        if !self.actionable || self.needs_review() {
            return Vec::new();
        }
        choices
            .into_iter()
            .enumerate()
            .map(|(index, (label, choice))| {
                let key = match choice {
                    CodexApprovalChoice::Approve
                    | CodexApprovalChoice::Command(CommandDecision::Accept) => "approve".into(),
                    CodexApprovalChoice::Decline
                    | CodexApprovalChoice::Command(CommandDecision::Decline) => "decline".into(),
                    CodexApprovalChoice::Cancel
                    | CodexApprovalChoice::Command(CommandDecision::Cancel) => "cancel".into(),
                    _ => format!("decision-{index}"),
                };
                (key, label)
            })
            .collect()
    }

    pub fn notification_choice(&self, key: &str) -> Option<CodexApprovalChoice> {
        if !self.actionable {
            return None;
        }
        let choices = self.choices();
        if self.needs_review() {
            return None;
        }
        self.notification_actions()
            .iter()
            .position(|(candidate, _)| candidate == key)
            .map(|index| choices[index].1.clone())
    }

    /// A saturated notification feed may refuse delivery only through a
    /// decision the source actually offered. Decline takes precedence over
    /// Cancel because Cancel can also terminate the owning turn.
    pub fn overflow_refusal_choice(&self) -> Option<CodexApprovalChoice> {
        if !self.actionable {
            return None;
        }
        let choices = self.choices();
        choices
            .iter()
            .find(|(_, choice)| {
                matches!(
                    choice,
                    CodexApprovalChoice::Decline
                        | CodexApprovalChoice::Command(CommandDecision::Decline)
                )
            })
            .or_else(|| {
                choices.iter().find(|(_, choice)| {
                    matches!(
                        choice,
                        CodexApprovalChoice::Cancel
                            | CodexApprovalChoice::Command(CommandDecision::Cancel)
                    )
                })
            })
            .map(|(_, choice)| choice.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodexApprovalChoice {
    Approve,
    Decline,
    Cancel,
    /// Preserve a source-supplied command decision exactly, including its scope.
    Command(CommandDecision),
}

#[derive(Clone, Debug)]
enum PendingNavigation {
    NewChat,
    NewChatIn(PathBuf, String),
    SelectThread(nickel_codex::ThreadId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClipboardWritePurpose {
    Other,
    DiagnosticSummary,
}

fn draft_changed(value: String) -> ChatMessage {
    ChatMessage::DraftChanged(value)
}

fn interaction_answer_changed(value: String) -> ChatMessage {
    ChatMessage::InteractionAnswerChanged(value)
}

fn conversation_scrolled(extent: twinkle_ui::ScrollExtent) -> ChatMessage {
    ChatMessage::ConversationScrolled(extent)
}

fn remote_host_id_changed(value: String) -> ChatMessage {
    ChatMessage::RemoteHostIdChanged(value)
}

fn remote_host_name_changed(value: String) -> ChatMessage {
    ChatMessage::RemoteHostNameChanged(value)
}

fn remote_host_endpoint_changed(value: String) -> ChatMessage {
    ChatMessage::RemoteHostEndpointChanged(value)
}

fn remote_host_token_env_changed(value: String) -> ChatMessage {
    ChatMessage::RemoteHostTokenEnvChanged(value)
}

fn remote_host_cwd_changed(value: String) -> ChatMessage {
    ChatMessage::RemoteHostCwdChanged(value)
}

fn project_window_title(path: &std::path::Path) -> String {
    format!("Codex — {}", project_window_name(path))
}

fn project_window_name(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Project")
        .to_owned()
}

fn approval_policy_label(policy: ApprovalPolicy) -> &'static str {
    match policy {
        ApprovalPolicy::Untrusted => "Ask for unknown commands",
        ApprovalPolicy::OnFailure => "Ask on failure",
        ApprovalPolicy::OnRequest => "Ask as needed",
        ApprovalPolicy::Never => "Never ask",
    }
}

fn approval_policy_description(policy: ApprovalPolicy) -> &'static str {
    match policy {
        ApprovalPolicy::Untrusted => "Only known-safe commands run without confirmation",
        ApprovalPolicy::OnFailure => "Sandboxed actions may request a retry outside the sandbox",
        ApprovalPolicy::OnRequest => "Codex decides when an action needs confirmation",
        ApprovalPolicy::Never => "Codex cannot pause to request approval",
    }
}

fn sandbox_policy_label(policy: Option<SandboxPolicy>) -> &'static str {
    match policy {
        None => "Backend-managed access (unverified)",
        Some(SandboxPolicy::ReadOnly) => "Read-only sandbox",
        Some(SandboxPolicy::WorkspaceWrite) => "Workspace-write sandbox",
        Some(SandboxPolicy::DangerFullAccess) => "YOLO — full access, unsandboxed",
    }
}

const APPROVAL_POLICIES: [ApprovalPolicy; 4] = [
    ApprovalPolicy::Untrusted,
    ApprovalPolicy::OnFailure,
    ApprovalPolicy::OnRequest,
    ApprovalPolicy::Never,
];

const CONTROLLER_POLL_MIN: std::time::Duration = std::time::Duration::from_millis(16);
const CONTROLLER_POLL_MAX: std::time::Duration = std::time::Duration::from_millis(128);

pub struct ChatApplication {
    pub state: ChatState,
    controller: ChatController,
    mode: BackendMode,
    settings: CodexSettings,
    settings_path: Option<PathBuf>,
    managing_hosts: bool,
    host_editor: Option<RemoteHostEditor>,
    settings_error: Option<String>,
    shell_host: bool,
    window_title: String,
    project_menu_mode: bool,
    shell_requests: Vec<ShellRequest>,
    pending_initial_resume: Option<nickel_codex::ThreadId>,
    shell_writer_thread: Option<nickel_codex::ThreadId>,
    pending_previous_writer: Option<nickel_codex::ThreadId>,
    recovery_thread: Option<nickel_codex::ThreadId>,
    recovery_project_configuration_pending: bool,
    shell_project: Option<(PathBuf, Option<String>)>,
    pub(crate) pending_shell_command: Option<String>,
    pub(crate) shell_warning_acknowledged: bool,
    pub(crate) model_picker_generation: u64,
    reasoning_picker_generation: u64,
    approval_picker_generation: u64,
    sandbox_picker_generation: u64,
    run_settings_open: bool,
    plan_mode: bool,
    slash_selection: usize,
    mention_selection: usize,
    feedback_open: bool,
    feedback_classification: Option<String>,
    feedback_reason: String,
    feedback_pending: bool,
    pub(crate) resume_picker_open: bool,
    pub(crate) resume_picker_loading: bool,
    pub(crate) resume_picker_pending: Option<nickel_codex::ThreadId>,
    resume_request_started: Option<std::time::Instant>,
    resume_first_view: std::cell::Cell<Option<(std::time::Instant, usize)>>,
    new_chat_pending: bool,
    pending_new_chat_title: Option<String>,
    pending_navigation: Option<PendingNavigation>,
    open_discard_dialog: bool,
    confirmed_new_chat_discard: bool,
    confirmed_project_discard: Option<(PathBuf, String)>,
    confirmed_thread_discard: Option<nickel_codex::ThreadId>,
    remote_control_open: bool,
    diagnostics_open: bool,
    diagnostics_details_open: bool,
    diagnostic_copy_result: Option<Result<(), String>>,
    controller_poll_interval: std::time::Duration,
    theme: SemanticTheme,
    clipboard_write: Option<String>,
    clipboard_write_purpose: Option<ClipboardWritePurpose>,
    queued_messages: std::collections::VecDeque<QueuedMessage>,
    editing_queued_message: Option<QueuedMessage>,
    next_queued_message_id: u64,
    queued_interrupt_deadline: Option<std::time::Instant>,
}

const QUEUED_MESSAGE_CAPACITY: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueuedMessagePhase {
    Waiting,
    InterruptRequested,
    BoundaryPending,
    InterruptTimedOut,
    Dispatching,
    Unconfirmed,
}

#[derive(Clone, Debug)]
struct QueuedMessage {
    id: u64,
    generation: u64,
    thread_id: nickel_codex::ThreadId,
    text: String,
    images: Vec<nickel_codex::TurnImage>,
    model: Option<String>,
    reasoning_effort: Option<String>,
    approval_policy: ApprovalPolicy,
    sandbox_policy: Option<SandboxPolicy>,
    plan_mode: bool,
    phase: QueuedMessagePhase,
}

#[derive(Clone, Debug)]
struct RemoteHostEditor {
    original_id: Option<String>,
    id: String,
    name: String,
    endpoint: String,
    token_env: String,
    default_cwd: String,
}

#[derive(Clone, Copy, Default)]
struct ChatOverlays<'a> {
    pending_shell_command: Option<&'a str>,
    model_picker_generation: u64,
    reasoning_picker_generation: u64,
    approval_picker_generation: u64,
    sandbox_picker_generation: u64,
    run_settings_open: bool,
    plan_mode: bool,
    slash_selection: usize,
    mention_selection: usize,
    feedback_open: bool,
    feedback_classification: Option<&'a str>,
    feedback_reason: &'a str,
    feedback_pending: bool,
    resume_picker_open: bool,
    shell_host: bool,
    resume_picker_loading: bool,
    resume_picker_pending: Option<&'a nickel_codex::ThreadId>,
    remote_control_open: bool,
    diagnostics_open: bool,
    diagnostics_details_open: bool,
    diagnostic_copy_result: Option<&'a Result<(), String>>,
    project_root: Option<&'a std::path::Path>,
    project_id: Option<&'a str>,
    queued_messages: Option<&'a std::collections::VecDeque<QueuedMessage>>,
    editing_queued_message: bool,
    editing_unconfirmed_message: bool,
}

impl RemoteHostEditor {
    fn empty() -> Self {
        Self {
            original_id: None,
            id: String::new(),
            name: String::new(),
            endpoint: "wss://".into(),
            token_env: String::new(),
            default_cwd: "/".into(),
        }
    }

    fn from_host(host: &RemoteHost) -> Self {
        Self {
            original_id: Some(host.id.clone()),
            id: host.id.clone(),
            name: host.name.clone(),
            endpoint: host.endpoint.clone(),
            token_env: host.token_env.clone().unwrap_or_default(),
            default_cwd: host.default_cwd.clone(),
        }
    }

    fn host(&self) -> RemoteHost {
        RemoteHost {
            id: self.id.clone(),
            name: self.name.clone(),
            endpoint: self.endpoint.clone(),
            token_env: (!self.token_env.trim().is_empty()).then(|| self.token_env.clone()),
            default_cwd: self.default_cwd.clone(),
        }
    }
}

impl ChatApplication {
    fn mark_queued_messages_for_review(&mut self, reason: &str) {
        if self.queued_messages.is_empty() {
            return;
        }
        self.queued_interrupt_deadline = None;
        for queued in &mut self.queued_messages {
            queued.phase = QueuedMessagePhase::Unconfirmed;
        }
        self.state.command_feedback = Some(format!(
            "Queued messages were not sent and require review after {reason}"
        ));
    }

    fn queue_current_draft(&mut self) {
        if (self.state.active_turn.is_none() && self.editing_queued_message.is_none())
            || self.state.status != ConnectionStatus::Ready
            || !self.state.account.authenticated
            || self.state.recovery_pending
            || self.state.unconfirmed_work
        {
            return;
        }
        if self.editing_queued_message.is_none()
            && self.queued_messages.len() >= QUEUED_MESSAGE_CAPACITY
        {
            self.state.command_feedback = Some(format!(
                "Message queue is full ({QUEUED_MESSAGE_CAPACITY}); cancel a queued message first"
            ));
            return;
        }
        let Some(thread_id) = self.state.selected_thread.clone() else {
            self.state.command_feedback =
                Some("Wait for the conversation to finish loading".into());
            return;
        };
        if self.state.draft.trim().is_empty() && self.state.attachments.is_empty() {
            return;
        }
        let replacement = self.editing_queued_message.take();
        self.next_queued_message_id = if replacement.is_some() {
            self.next_queued_message_id
        } else {
            self.next_queued_message_id.wrapping_add(1).max(1)
        };
        let replacement_phase = replacement
            .as_ref()
            .map(|queued| queued.phase)
            .unwrap_or(QueuedMessagePhase::Waiting);
        let replacement_images = replacement
            .as_ref()
            .map(|queued| queued.images.clone())
            .unwrap_or_default();
        let queued = QueuedMessage {
            id: replacement
                .as_ref()
                .map_or(self.next_queued_message_id, |queued| queued.id),
            generation: self.state.generation,
            thread_id,
            text: std::mem::take(&mut self.state.draft),
            images: if self.state.attachments.is_empty() {
                replacement_images
            } else {
                self.state
                    .attachments
                    .iter()
                    .map(crate::PendingAttachment::turn_image)
                    .collect()
            },
            model: self.state.selected_model.clone(),
            reasoning_effort: self.state.selected_reasoning_effort.clone(),
            approval_policy: self.state.selected_approval_policy,
            sandbox_policy: self.state.selected_sandbox_policy,
            plan_mode: self.plan_mode,
            phase: if replacement_phase == QueuedMessagePhase::Unconfirmed {
                QueuedMessagePhase::Unconfirmed
            } else {
                QueuedMessagePhase::Waiting
            },
        };
        if replacement.is_some() {
            self.queued_messages.push_front(queued);
        } else {
            self.queued_messages.push_back(queued);
        }
        self.state.attachments.clear();
        self.state.command_feedback =
            Some(if replacement_phase == QueuedMessagePhase::Unconfirmed {
                "Reviewed message saved without sending; retrying may duplicate a previous delivery"
                    .into()
            } else {
                format!(
                    "Queued message {} of {QUEUED_MESSAGE_CAPACITY}",
                    self.queued_messages.len()
                )
            });
        if self.state.active_turn.is_none() && replacement_phase != QueuedMessagePhase::Unconfirmed
        {
            self.dispatch_queued_after_boundary();
        }
    }

    fn request_queued_interrupt(&mut self) {
        let Some(queued) = self.queued_messages.front_mut() else {
            return;
        };
        if !matches!(
            queued.phase,
            QueuedMessagePhase::Waiting | QueuedMessagePhase::InterruptTimedOut
        ) || self.state.active_turn.is_none()
            || self.state.interrupt_requested
        {
            return;
        }
        queued.phase = QueuedMessagePhase::InterruptRequested;
        self.queued_interrupt_deadline =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(10));
        self.state.interrupt_requested = true;
        if !self.controller.send(ControllerCommand::Interrupt) {
            queued.phase = QueuedMessagePhase::Waiting;
            self.queued_interrupt_deadline = None;
            self.state.interrupt_requested = false;
            self.state.command_feedback = Some("Could not queue the interrupt request".into());
        }
    }

    fn dispatch_queued_after_boundary(&mut self) {
        let Some(queued) = self.queued_messages.front_mut() else {
            return;
        };
        if !matches!(
            queued.phase,
            QueuedMessagePhase::Waiting
                | QueuedMessagePhase::InterruptRequested
                | QueuedMessagePhase::BoundaryPending
        ) || self.state.active_turn.is_some()
        {
            return;
        }
        if queued.generation != self.state.generation
            || self.state.selected_thread.as_ref() != Some(&queued.thread_id)
            || !self.state.begin_queued_send(&queued.text)
        {
            queued.phase = QueuedMessagePhase::Unconfirmed;
            self.state.command_feedback =
                Some("Queued message requires review before it can be sent".into());
            return;
        }
        queued.phase = QueuedMessagePhase::Dispatching;
        if !self.controller.send(ControllerCommand::Send {
            text: queued.text.clone(),
            images: queued.images.clone(),
            model: queued.model.clone(),
            reasoning_effort: queued.reasoning_effort.clone(),
            approval_policy: queued.approval_policy,
            sandbox_policy: queued.sandbox_policy,
            plan_mode: queued.plan_mode,
        }) {
            queued.phase = QueuedMessagePhase::Unconfirmed;
            self.state.unconfirmed_work = true;
            self.state.command_feedback =
                Some("Queued message delivery is unconfirmed; reconnect before retrying".into());
        }
    }

    fn current_project_root(&self) -> PathBuf {
        self.shell_project
            .as_ref()
            .map(|(root, _)| root.clone())
            .unwrap_or_else(|| match &self.mode {
                BackendMode::Live { cwd, .. } | BackendMode::Replay { cwd, .. } => cwd.clone(),
                BackendMode::Remote { host } => PathBuf::from(&host.default_cwd),
            })
    }

    fn update_file_search(&mut self) {
        let Some(query) = mention_fragment(&self.state.draft).map(str::to_owned) else {
            self.state.file_search_query = None;
            self.state.file_search_matches.clear();
            self.state.file_search_pending = false;
            self.state.file_search_error = None;
            return;
        };
        if self.state.file_search_query.as_deref() == Some(query.as_str()) {
            return;
        }
        self.mention_selection = 0;
        self.state.file_search_query = Some(query.clone());
        self.state.file_search_matches.clear();
        self.state.file_search_pending = true;
        self.state.file_search_error = None;
        if !self.controller.send(ControllerCommand::SearchFiles {
            query,
            root: self.current_project_root(),
        }) {
            self.state.file_search_pending = false;
            self.state.file_search_error =
                Some("Codex is disconnected; file search is unavailable".into());
        }
    }

    #[cfg(feature = "workbench-fixtures")]
    pub fn fixture_shell_project_menu(state: &str) -> Self {
        let mode = BackendMode::Replay {
            backend: nickel_codex::ReplayBackend::from_json(
                r#"{"name":"shell-project-menu","projects":[],"events":[]}"#,
            )
            .expect("static project-menu replay fixture must parse"),
            cwd: PathBuf::from("/projects/nickel"),
        };
        let mut app = Self::with_settings(mode, CodexSettings::default(), None);
        app.shell_host = true;
        app.window_title = "Nickel Codex Projects".into();
        app.project_menu_mode = true;
        app.state.status = ConnectionStatus::Ready;
        app.state.provenance = "Deterministic workbench fixture".into();
        app.state.projects = match state {
            "open" => vec![nickel_codex::Project {
                id: "nickel".into(),
                name: "Nickel".into(),
                roots: vec![PathBuf::from("/projects/nickel")],
            }],
            "search" => vec![
                nickel_codex::Project {
                    id: "nickel".into(),
                    name: "Nickel".into(),
                    roots: vec![PathBuf::from("/projects/nickel")],
                },
                nickel_codex::Project {
                    id: "vesalius".into(),
                    name: "Vesalius".into(),
                    roots: vec![PathBuf::from("/projects/vesalius")],
                },
            ],
            "empty" => Vec::new(),
            other => panic!("unknown Codex project-menu fixture state `{other}`"),
        };
        app.state.draft = if state == "search" {
            "nickel".into()
        } else {
            String::new()
        };
        app.controller = ChatController::fixture_idle(app.state.generation);
        app
    }

    pub fn new(mode: BackendMode) -> Self {
        Self::with_settings(mode, CodexSettings::default(), None)
    }

    pub fn with_settings(
        mode: BackendMode,
        settings: CodexSettings,
        settings_path: Option<PathBuf>,
    ) -> Self {
        let mut state = ChatState::default();
        state.effective_approval_policy = settings.approval_policy;
        state.selected_approval_policy = settings.approval_policy;
        state.effective_sandbox_policy = settings.sandbox_policy;
        state.selected_sandbox_policy = settings.sandbox_policy;
        Self {
            state,
            controller: ChatController::spawn(mode.clone()),
            mode,
            settings,
            settings_path,
            managing_hosts: false,
            host_editor: None,
            settings_error: None,
            shell_host: false,
            window_title: "Nickel".into(),
            project_menu_mode: false,
            shell_requests: Vec::new(),
            pending_initial_resume: None,
            shell_writer_thread: None,
            pending_previous_writer: None,
            recovery_thread: None,
            recovery_project_configuration_pending: false,
            shell_project: None,
            pending_shell_command: None,
            shell_warning_acknowledged: false,
            model_picker_generation: 0,
            reasoning_picker_generation: 0,
            approval_picker_generation: 0,
            sandbox_picker_generation: 0,
            run_settings_open: false,
            plan_mode: false,
            slash_selection: 0,
            mention_selection: 0,
            feedback_open: false,
            feedback_classification: None,
            feedback_reason: String::new(),
            feedback_pending: false,
            resume_picker_open: false,
            resume_picker_loading: false,
            resume_picker_pending: None,
            resume_request_started: None,
            resume_first_view: std::cell::Cell::new(None),
            new_chat_pending: false,
            pending_new_chat_title: None,
            pending_navigation: None,
            open_discard_dialog: false,
            confirmed_new_chat_discard: false,
            confirmed_project_discard: None,
            confirmed_thread_discard: None,
            remote_control_open: false,
            diagnostics_open: false,
            diagnostics_details_open: false,
            diagnostic_copy_result: None,
            controller_poll_interval: CONTROLLER_POLL_MIN,
            theme: semantic_theme(),
            clipboard_write: None,
            clipboard_write_purpose: None,
            queued_messages: std::collections::VecDeque::new(),
            editing_queued_message: None,
            next_queued_message_id: 0,
            queued_interrupt_deadline: None,
        }
    }

    pub fn set_theme(&mut self, theme: SemanticTheme) -> bool {
        if self.theme == theme {
            return false;
        }
        self.theme = theme;
        true
    }

    pub fn as_shell_project_menu(mut self) -> Self {
        self.shell_host = true;
        self.window_title = "Nickel Codex Projects".into();
        self.project_menu_mode = true;
        self.state.generation = self.state.generation.saturating_add(1);
        self.controller =
            ChatController::spawn_project_menu(self.mode.clone(), self.state.generation);
        self
    }

    pub fn as_shell_chat(mut self, cwd: &std::path::Path) -> Self {
        self.shell_host = true;
        self.window_title = project_window_title(cwd);
        self.state.generation = self.state.generation.saturating_add(1);
        self.controller =
            ChatController::spawn_project_chat(self.mode.clone(), self.state.generation);
        self
    }

    pub fn resume_thread(&mut self, id: nickel_codex::ThreadId) -> Result<(), String> {
        self.controller_poll_interval = CONTROLLER_POLL_MIN;
        self.resume_request_started = Some(std::time::Instant::now());
        self.pending_initial_resume = Some(id.clone());
        self.pending_previous_writer = self.shell_writer_thread.replace(id.clone());
        if self.controller.send(ControllerCommand::SelectThread(id)) {
            Ok(())
        } else {
            self.pending_initial_resume = None;
            self.resume_request_started = None;
            self.shell_writer_thread = self.pending_previous_writer.take();
            Err("Codex controller stopped before thread resume".into())
        }
    }

    pub fn take_shell_requests(&mut self) -> Vec<ShellRequest> {
        std::mem::take(&mut self.shell_requests)
    }

    pub fn approval_notifications(&self) -> Vec<CodexApprovalNotification> {
        self.state
            .pending
            .iter()
            .filter_map(|interaction| {
                let PendingInteraction::Approval {
                    request_id,
                    approval_type,
                    summary,
                    context,
                } = interaction
                else {
                    return None;
                };
                let request_revision = self.state.interaction_revision(request_id)?;
                Some(CodexApprovalNotification {
                    connection_generation: self.state.generation,
                    request_revision,
                    thread_id: self.state.interaction_thread_id(request_id).cloned(),
                    interaction: interaction.clone(),
                    presentation: codex_approval_presentation(approval_type, summary, context),
                    actionable: self.state.interaction_is_actionable(request_id)
                        && !approval_choices(approval_type, context).is_empty(),
                    submitting: self.state.interaction_submission_pending(request_id),
                    unconfirmed: self.state.interaction_response_unconfirmed(request_id),
                })
            })
            .collect()
    }

    /// Apply shell delivery state only to the exact still-pending request.
    /// This does not decide or expire the source-owned approval.
    pub fn report_approval_notification_delivery(
        &mut self,
        expected: &CodexApprovalNotification,
        delivered: bool,
    ) -> bool {
        let PendingInteraction::Approval { request_id, .. } = &expected.interaction else {
            return false;
        };
        if expected.connection_generation != self.state.generation
            || self.state.interaction_revision(request_id) != Some(expected.request_revision)
            || self.state.interaction_thread_id(request_id) != expected.thread_id.as_ref()
            || !self.state.pending.contains(&expected.interaction)
        {
            return false;
        }
        if delivered {
            self.state
                .clear_approval_notification_unavailable(request_id);
        } else {
            self.state
                .mark_approval_notification_unavailable(request_id);
        }
        true
    }

    pub fn respond_approval_notification(
        &mut self,
        expected: &CodexApprovalNotification,
        choice: CodexApprovalChoice,
    ) -> bool {
        let PendingInteraction::Approval {
            request_id,
            approval_type,
            context,
            ..
        } = &expected.interaction
        else {
            return false;
        };
        // Exact snapshot comparison rejects stale actions after reconnect, scope revision,
        // resolution, or request-ID reuse. The normal reducer still performs final validation.
        if expected.connection_generation != self.state.generation
            || self.state.interaction_revision(request_id) != Some(expected.request_revision)
            || self.state.interaction_thread_id(request_id) != expected.thread_id.as_ref()
            || !self.state.interaction_is_actionable(request_id)
            || !approval_choice_allowed(approval_type, context, &choice)
            || !self.state.pending.contains(&expected.interaction)
        {
            return false;
        }
        self.update(ChatMessage::RespondApproval(
            request_id.clone(),
            approval_type.clone(),
            choice,
        ));
        self.state.interaction_submission_pending(request_id)
    }

    pub fn report_resume_rejection(&mut self, message: impl Into<String>) {
        self.pending_initial_resume = None;
        self.resume_picker_pending = None;
        self.confirmed_thread_discard = None;
        self.resume_picker_open = true;
        self.state.report_diagnostic(message);
    }

    /// Clear the picker when the shell has focused an existing local writer.
    /// No transcript, draft, or controller generation changes ownership here.
    pub fn report_resume_owner_activation(&mut self) {
        self.resume_picker_pending = None;
        self.resume_picker_open = false;
        self.confirmed_thread_discard = None;
    }

    /// Called by the shell only for an in-place replacement, after it has
    /// ruled out activation of another local owner.
    pub fn prepare_shell_resume(&mut self, id: &nickel_codex::ThreadId) -> bool {
        if let Some(reason) = self.state.replacement_block_reason() {
            self.report_resume_rejection(reason);
            return false;
        }
        if (!self.state.draft.is_empty() || !self.state.attachments.is_empty())
            && self.confirmed_thread_discard.as_ref() != Some(id)
        {
            self.resume_picker_pending = None;
            self.pending_navigation = Some(PendingNavigation::SelectThread(id.clone()));
            self.open_discard_dialog = true;
            return false;
        }
        true
    }

    pub fn use_project(&mut self, cwd: PathBuf, project_id: String) {
        self.controller_poll_interval = CONTROLLER_POLL_MIN;
        self.shell_project = Some((cwd.clone(), Some(project_id.clone())));
        self.controller
            .send(ControllerCommand::ConfigureProject(cwd, Some(project_id)));
    }

    pub fn use_project_root(&mut self, cwd: PathBuf) {
        self.controller_poll_interval = CONTROLLER_POLL_MIN;
        self.shell_project = Some((cwd.clone(), None));
        self.controller
            .send(ControllerCommand::ConfigureProject(cwd, None));
    }

    pub fn poll_controller(&mut self) -> bool {
        let mut changed = false;
        let started = std::time::Instant::now();
        for _ in 0..128 {
            if started.elapsed() >= std::time::Duration::from_millis(4) {
                break;
            }
            let Some((generation, event)) = self.controller.try_recv() else {
                break;
            };
            let turn_boundary = matches!(
                &event,
                ControllerEvent::Protocol(nickel_codex::CodexEvent {
                    kind: nickel_codex::EventKind::TurnCompleted { turn_id, .. },
                    ..
                }) if self.state.active_turn.as_ref() == Some(turn_id)
            );
            let mut recovery_attachments = None;
            if generation == self.state.generation {
                if let ControllerEvent::ThreadSelected(thread) = &event
                    && (self
                        .pending_initial_resume
                        .as_ref()
                        .is_some_and(|pending| pending != &thread.id)
                        || self
                            .recovery_thread
                            .as_ref()
                            .is_some_and(|pending| pending != &thread.id))
                {
                    // A superseded selection must not replace the visible transcript
                    // or complete the writer-lease handoff for the newer destination.
                    continue;
                }
                // Reloading the same thread is not a navigation replacement: keep
                // unsent images while the transcript is rebuilt from the backend.
                recovery_attachments = matches!(&event, ControllerEvent::ThreadSelected(thread)
                    if self.recovery_thread.as_ref() == Some(&thread.id))
                .then(|| std::mem::take(&mut self.state.attachments));
                match &event {
                    ControllerEvent::ProjectConfigured => {
                        self.recovery_project_configuration_pending = false;
                        if self.recovery_thread.is_none()
                            && self.state.status == ConnectionStatus::Ready
                            && !self.state.recovery_failed
                        {
                            self.state.recovery_pending = false;
                        }
                    }
                    ControllerEvent::NewChatPrepared => {
                        if let Some(title) = self.pending_new_chat_title.take() {
                            self.window_title = title;
                        }
                        self.new_chat_pending = false;
                        self.confirmed_new_chat_discard = false;
                        self.confirmed_project_discard = None;
                    }
                    ControllerEvent::NewChatFailed(_) => {
                        self.pending_new_chat_title = None;
                        self.new_chat_pending = false;
                        self.confirmed_new_chat_discard = false;
                        self.confirmed_project_discard = None;
                    }
                    ControllerEvent::ThreadSelected(thread) => {
                        if self.recovery_thread.as_ref() == Some(&thread.id) {
                            self.recovery_thread = None;
                            self.state.recovery_pending = false;
                            self.state.recovery_failed = false;
                        }
                        if self.shell_host
                            && self.pending_initial_resume.as_ref() == Some(&thread.id)
                        {
                            self.shell_requests
                                .push(ShellRequest::ResumeSucceeded(thread.id.clone()));
                        }
                        if self.confirmed_thread_discard.as_ref() == Some(&thread.id) {
                            // Confirmation is committed only after destination hydration succeeds.
                            self.state.draft.clear();
                            self.confirmed_thread_discard = None;
                        }
                        if self.resume_picker_pending.as_ref() == Some(&thread.id) {
                            self.resume_picker_pending = None;
                            self.resume_picker_open = false;
                        }
                        if self.pending_initial_resume.as_ref() == Some(&thread.id) {
                            self.pending_initial_resume = None;
                            self.pending_previous_writer = None;
                        }
                    }
                    ControllerEvent::InterruptAccepted => {
                        self.queued_interrupt_deadline = None;
                        if let Some(queued) = self.queued_messages.front_mut()
                            && queued.phase == QueuedMessagePhase::InterruptRequested
                        {
                            queued.phase = QueuedMessagePhase::BoundaryPending;
                        }
                    }
                    ControllerEvent::InterruptFailed(_) => {
                        self.queued_interrupt_deadline = None;
                        if let Some(queued) = self.queued_messages.front_mut()
                            && matches!(
                                queued.phase,
                                QueuedMessagePhase::InterruptRequested
                                    | QueuedMessagePhase::BoundaryPending
                            )
                        {
                            queued.phase = QueuedMessagePhase::Waiting;
                        }
                    }
                    ControllerEvent::TurnAccepted => {
                        if self
                            .queued_messages
                            .front()
                            .is_some_and(|queued| queued.phase == QueuedMessagePhase::Dispatching)
                        {
                            self.queued_messages.pop_front();
                        }
                    }
                    ControllerEvent::TurnStartFailed(_) => {
                        if let Some(queued) = self.queued_messages.front_mut()
                            && queued.phase == QueuedMessagePhase::Dispatching
                        {
                            queued.phase = QueuedMessagePhase::Unconfirmed;
                            self.state.unconfirmed_work = true;
                        }
                    }
                    ControllerEvent::OperationFailed(_) => {
                        self.resume_request_started = None;
                        if self.recovery_thread.take().is_some() {
                            self.state.recovery_failed = true;
                            self.state.report_diagnostic("The selected conversation could not be reloaded. Reconnect to retry; no prompt or approval was replayed.");
                        }
                        self.confirmed_thread_discard = None;
                        self.resume_picker_pending = None;
                        if self.pending_initial_resume.take().is_some()
                            && let Some(thread) = self.shell_writer_thread.take()
                        {
                            self.shell_requests.push(ShellRequest::ResumeFailed(thread));
                            self.shell_writer_thread = self.pending_previous_writer.take();
                        }
                    }
                    ControllerEvent::ApprovalPolicyAccepted(policy) => {
                        self.accept_approval_policy(*policy);
                    }
                    ControllerEvent::SandboxPolicyAccepted(policy) => {
                        self.accept_sandbox_policy(*policy);
                    }
                    ControllerEvent::ModelRejected { .. } => {}
                    ControllerEvent::FeedbackUploaded { .. } => {
                        self.feedback_pending = false;
                        self.feedback_open = false;
                        self.feedback_classification = None;
                        self.feedback_reason.clear();
                    }
                    ControllerEvent::FeedbackUploadFailed(_) => {
                        self.feedback_pending = false;
                    }
                    ControllerEvent::Failure(_)
                    | ControllerEvent::Incompatible(_)
                    | ControllerEvent::Unavailable(_) => {
                        self.mark_queued_messages_for_review("connection loss");
                        self.new_chat_pending = false;
                        self.pending_new_chat_title = None;
                        self.pending_initial_resume = None;
                        self.resume_picker_loading = false;
                        self.resume_picker_pending = None;
                        if let Some(thread) = self.shell_writer_thread.take() {
                            self.shell_requests.push(ShellRequest::ResumeFailed(thread));
                        }
                        if let Some(previous) = self.pending_previous_writer.take() {
                            self.shell_requests
                                .push(ShellRequest::ResumeFailed(previous));
                        }
                    }
                    _ => {}
                }
                if matches!(event, ControllerEvent::Ready { .. }) {
                    self.resume_picker_loading = false;
                    if self.recovery_thread.is_none()
                        && !self.recovery_project_configuration_pending
                        && !self.state.recovery_failed
                    {
                        self.state.recovery_pending = false;
                    }
                }
            }
            let resume_hydration =
                matches!(&event, ControllerEvent::ThreadSelected(_)).then(std::time::Instant::now);
            changed |= self.state.apply(generation, event);
            if let Some(started) = resume_hydration {
                if crate::controller::codex_resume_timing_enabled() {
                    eprintln!(
                        "nickel: Codex resumed transcript hydration projection completed in {} ms ({} visible items)",
                        started.elapsed().as_millis(),
                        self.state.items.len()
                    );
                    self.resume_first_view.set(Some((
                        self.resume_request_started.take().unwrap_or(started),
                        self.state.items.len(),
                    )));
                } else {
                    self.resume_request_started = None;
                }
            }
            if turn_boundary {
                self.queued_interrupt_deadline = None;
                self.dispatch_queued_after_boundary();
                changed = true;
            }
            if let Some(attachments) = recovery_attachments {
                self.state.attachments = attachments;
            }
            if self.project_menu_mode {
                self.state.thread_error = None;
            }
        }
        if self
            .queued_interrupt_deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            self.queued_interrupt_deadline = None;
            if let Some(queued) = self.queued_messages.front_mut()
                && matches!(
                    queued.phase,
                    QueuedMessagePhase::InterruptRequested | QueuedMessagePhase::BoundaryPending
                )
            {
                queued.phase = QueuedMessagePhase::InterruptTimedOut;
                self.state.interrupt_requested = false;
                self.state.command_feedback = Some(
                    "Interrupt outcome is still unknown; the queued message was not sent".into(),
                );
                changed = true;
            }
        }
        self.controller_poll_interval = if changed {
            CONTROLLER_POLL_MIN
        } else {
            self.controller_poll_interval
                .saturating_mul(2)
                .min(CONTROLLER_POLL_MAX)
        };
        changed
    }

    fn reconnect_controller(&mut self) {
        self.resume_request_started = None;
        self.resume_first_view.set(None);
        self.queued_interrupt_deadline = None;
        self.new_chat_pending = false;
        self.pending_new_chat_title = None;
        if let Some(pending) = self.pending_initial_resume.take() {
            self.shell_requests
                .push(ShellRequest::ResumeFailed(pending));
            self.shell_writer_thread = self.pending_previous_writer.take();
        }
        self.recovery_thread = self.state.selected_thread.clone();
        self.recovery_project_configuration_pending = self.shell_project.is_some();
        self.state.recovery_pending = true;
        self.state.recovery_failed = false;
        for queued in &mut self.queued_messages {
            queued.phase = QueuedMessagePhase::Unconfirmed;
        }
        self.state.generation = self.state.generation.saturating_add(1);
        self.state.status = ConnectionStatus::Loading;
        self.state.backend_source = None;
        self.state.fallback_reason = None;
        let had_active_turn = self.state.active_turn.is_some();
        self.state.mark_unconfirmed_on_connection_loss();
        let invalidated = self.state.invalidate_pending_for_reconnect();
        self.state.diagnostics.clear();
        if had_active_turn {
            self.state.report_diagnostic(
                "The previous turn outcome is unconfirmed until this conversation reloads.",
            );
        }
        if invalidated != 0 {
            self.state.report_diagnostic(format!(
                "{invalidated} request(s) from the previous connection are no longer actionable"
            ));
        }
        self.state.active_turn = None;
        self.state.interrupt_requested = false;
        self.controller = if self.project_menu_mode {
            ChatController::spawn_project_menu(self.mode.clone(), self.state.generation)
        } else if self.shell_host {
            ChatController::spawn_project_chat(self.mode.clone(), self.state.generation)
        } else {
            ChatController::spawn_generation(self.mode.clone(), self.state.generation)
        };
        if let Some((cwd, project_id)) = self.shell_project.clone()
            && !self
                .controller
                .send(ControllerCommand::ConfigureProject(cwd, project_id))
        {
            self.recovery_project_configuration_pending = false;
            self.state.recovery_failed = true;
            self.state
                .report_diagnostic("Could not queue project recovery; reconnect to retry.");
        }
        if let Some(thread) = self.recovery_thread.clone()
            && !self
                .controller
                .send(ControllerCommand::SelectThread(thread))
        {
            self.recovery_thread = None;
            self.state.recovery_failed = true;
            self.state
                .report_diagnostic("Could not queue the conversation reload; reconnect to retry.");
        }
    }

    fn dispatch_interaction_response(
        &mut self,
        request_id: ServerRequestId,
        command: ControllerCommand,
    ) {
        if !self.controller.send(command) {
            self.state.apply(
                self.state.generation,
                ControllerEvent::InteractionResponseFailed {
                    request_id,
                    message: "controller command was not queued".into(),
                },
            );
        }
    }

    fn save_settings(&mut self, settings: CodexSettings) -> bool {
        let Some(path) = &self.settings_path else {
            self.settings_error = Some("persistent host settings are unavailable".into());
            return false;
        };
        match settings.save(path) {
            Ok(()) => {
                self.settings = settings;
                self.settings_error = None;
                true
            }
            Err(error) => {
                self.settings_error = Some(error.to_string());
                false
            }
        }
    }

    pub(crate) fn accept_approval_policy(&mut self, policy: ApprovalPolicy) {
        self.state.effective_approval_policy = policy;
        let mut settings = self.settings.clone();
        settings.approval_policy = policy;
        if !self.save_settings(settings) {
            self.state.report_diagnostic(
                "Codex accepted the approval policy, but Nickel could not save it",
            );
        }
    }

    fn accept_sandbox_policy(&mut self, policy: Option<SandboxPolicy>) {
        self.state.effective_sandbox_policy = policy;
        let mut settings = self.settings.clone();
        settings.sandbox_policy = policy;
        if !self.save_settings(settings) {
            self.state.report_diagnostic(
                "Codex accepted the sandbox policy, but Nickel could not save it",
            );
        }
    }

    fn select_connection(&mut self, id: String) {
        let mode = if id == "local" {
            match create_managed_workspace() {
                Ok(cwd) => BackendMode::Live {
                    choice: BackendChoice::Automatic,
                    cwd,
                },
                Err(error) => {
                    self.settings_error = Some(error);
                    return;
                }
            }
        } else {
            let Some(host) = self.settings.hosts.iter().find(|host| host.id == id) else {
                self.settings_error = Some(format!("remote host {id} no longer exists"));
                return;
            };
            BackendMode::Remote { host: host.clone() }
        };
        let mut settings = self.settings.clone();
        settings.selected = id;
        if !self.save_settings(settings) {
            return;
        }
        let generation = self.state.generation.saturating_add(1);
        let mut state = ChatState::default();
        state.effective_approval_policy = self.settings.approval_policy;
        state.selected_approval_policy = self.settings.approval_policy;
        state.effective_sandbox_policy = self.settings.sandbox_policy;
        state.selected_sandbox_policy = self.settings.sandbox_policy;
        state.generation = generation;
        self.state = state;
        self.controller = if self.project_menu_mode {
            ChatController::spawn_project_menu(mode.clone(), generation)
        } else if self.shell_host {
            ChatController::spawn_project_chat(mode.clone(), generation)
        } else {
            ChatController::spawn_generation(mode.clone(), generation)
        };
        self.mode = mode;
    }
}

#[cfg(test)]
pub fn chat_view(state: &ChatState) -> impl View<ChatMessage> {
    configured_chat_view(
        state,
        &DEFAULT_CODEX_SETTINGS,
        ChatHostPanel::default(),
        ChatOverlays::default(),
        semantic_theme(),
        1120.0,
    )
}

#[cfg(test)]
pub fn shell_project_menu_view(state: &ChatState) -> impl View<ChatMessage> {
    project_menu_view(state, None, semantic_theme())
}

fn close_icon_pixels(color: twinkle_ui::Color) -> std::sync::Arc<image::RgbaImage> {
    let rgba = image::Rgba([
        ((color >> 16) & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        (color & 0xff) as u8,
        255,
    ]);
    let mut pixels = image::RgbaImage::new(24, 24);
    for coordinate in 4_i32..20 {
        for thickness in -1_i32..=1 {
            let forward = coordinate + thickness;
            let reverse = 23 - coordinate + thickness;
            if (0..24).contains(&forward) {
                pixels.put_pixel(forward as u32, coordinate as u32, rgba);
            }
            if (0..24).contains(&reverse) {
                pixels.put_pixel(reverse as u32, coordinate as u32, rgba);
            }
        }
    }
    std::sync::Arc::new(pixels)
}

// Keep the related remote-host presentation inputs together at the view boundary.
#[derive(Default)]
struct ChatHostPanel<'a> {
    managing_hosts: bool,
    editor: Option<&'a RemoteHostEditor>,
    settings_error: Option<&'a str>,
}

fn configured_chat_view(
    state: &ChatState,
    settings: &CodexSettings,
    host_panel: ChatHostPanel<'_>,
    overlays: ChatOverlays<'_>,
    theme: SemanticTheme,
    viewport_width: f32,
) -> impl View<ChatMessage> {
    let ChatHostPanel {
        managing_hosts,
        editor,
        settings_error,
    } = host_panel;
    let ChatOverlays {
        pending_shell_command,
        model_picker_generation,
        reasoning_picker_generation,
        approval_picker_generation,
        sandbox_picker_generation,
        run_settings_open,
        plan_mode,
        slash_selection,
        mention_selection,
        feedback_open,
        feedback_classification,
        feedback_reason,
        feedback_pending,
        resume_picker_open,
        shell_host,
        resume_picker_loading,
        resume_picker_pending,
        remote_control_open,
        diagnostics_open,
        diagnostics_details_open,
        diagnostic_copy_result,
        project_root,
        project_id,
        queued_messages,
        editing_queued_message,
        editing_unconfirmed_message,
    } = overlays;
    let queued_count = queued_messages.map_or(0, std::collections::VecDeque::len);
    let narrow = viewport_width < 720.0;
    let settings_stacked = viewport_width < 900.0;
    let available_slash_commands = available_slash_commands(&state.draft);
    let selected_slash_command = (!available_slash_commands.is_empty()).then(|| {
        available_slash_commands[slash_selection % available_slash_commands.len()].command
    });
    let selected_mention_path = (!state.file_search_matches.is_empty()).then(|| {
        state.file_search_matches[mention_selection % state.file_search_matches.len()]
            .path
            .as_str()
    });
    let transcript_window = transcript_window(state);
    let transcript_range = transcript_window.range.clone();
    let transcript_document = state.transcript_selection_document();
    ui! {
        <Column fill_width fill_height background={theme.surfaces.window}>
            <MenuBar id={id!(menu_bar)}>
                <Menu id={id!(file_menu)} on_toggle={ChatMessage::ToggleFileMenu} label={"File"}>
                    <MenuItem label={"New conversation"} on_press={ChatMessage::NewChat} />
                    <MenuItem label={"Refresh"} on_press={ChatMessage::Refresh} />
                </Menu>
                {thread_menu(state, project_root)}
                <Menu id={id!(codex_menu)} on_toggle={ChatMessage::ToggleFileMenu} label={"Codex"}>
                    <MenuItem label={"Run settings"} on_press={ChatMessage::ToggleRunSettings} />
                    <MenuItem label={"Phone access…"} on_press={ChatMessage::OpenRemoteControl} />
                    <MenuItem label={"Diagnostics…"} on_press={ChatMessage::ToggleDiagnostics} />
                </Menu>
                <Menu id={id!(view_menu)} on_toggle={ChatMessage::ToggleFileMenu} label={"View"}>
                    <MenuItem label={"Jump to latest"} on_press={ChatMessage::JumpToLatest} />
                </Menu>
                {connection_menu(settings)}
            </MenuBar>
            {[()].into_iter().filter(|_| run_settings_open).map(|_| ui! {
                <Column id={id!(compact_run_settings)} fill_width
                    padding={Insets::all(6.0)} gap={4.0}
                    background={theme.surfaces.raised} radius={6.0}>
                    <Row fill_width align_items={Align::Start} gap={6.0}>
                        <Container grow={1.0} min_width={0.0}>
                            {run_settings_controls(state, model_picker_generation,
                                reasoning_picker_generation, approval_picker_generation,
                                sandbox_picker_generation, settings_stacked, theme)}
                        </Container>
                        <Container id={id!(close_run_settings)} width={36.0} height={36.0}
                            padding={Insets::all(6.0)} radius={4.0}
                            background={theme.surfaces.hover}
                            on_press={ChatMessage::CloseRunSettings}
                            semantic_role={SemanticRole::Button}
                            accessibility_label={"Close run settings"}>
                            {Image::new(0xc0de, close_icon_pixels(theme.text.primary))
                                .width(24.0).height(24.0).decorative()}
                        </Container>
                    </Row>
                    <Text color={theme.text.secondary}>{"Never ask only changes prompts. YOLO grants unsandboxed full access and never asks."}</Text>
                </Column>
            })}
            {if managing_hosts {
                remote_hosts_panel(settings, editor, settings_error, theme)
            } else { AnyView::new(ui! {
            <Column grow={1.0} min_width={0.0} fill_height gap={8.0}>
                <Column id={id!(conversation)} grow={1.0} fill_width gap={10.0}
                    padding={Insets::all(if narrow { 8.0 } else { 18.0 })}
                    accessibility_label={"Conversation"} semantic_role={SemanticRole::Group}
                    overflow_y={Overflow::Auto} follow_scroll_end={state.conversation_pinned}
                    on_scroll_extent={conversation_scrolled}>
                    {if diagnostics_open {
                        AnyView::new(diagnostics_panel(state, diagnostics_details_open,
                            diagnostic_copy_result, theme))
                    } else if remote_control_open {
                        codex_phone_access_panel(state, theme)
                    } else if feedback_open {
                        AnyView::new(feedback_panel(
                            feedback_classification,
                            feedback_reason,
                            feedback_pending,
                            theme,
                        ))
                    } else if resume_picker_open {
                        resume_picker(
                            state,
                            project_root,
                            project_id,
                            resume_picker_loading,
                            resume_picker_pending,
                            shell_host,
                            theme,
                        )
                    } else if !state.account.authenticated && state.items.is_empty() {
                        login_panel(state, theme)
                    } else if state.items.is_empty() {
                        AnyView::new(ui! {
                            <Container grow={1.0} fill_width padding={Insets::all(28.0)}>
                                <Text scale={2.0} color={theme.text.primary}>{"What are we building?"}</Text>
                            <Text color={theme.text.secondary}>{"Start a conversation with Codex. Approvals follow the policy below."}</Text>
                            </Container>
                        })
                    } else {
                        AnyView::new(ui! {
                            <Container id={id!(transcript_column)} fill_width>
                            {SelectionRegion::new(transcript_document)
                                .id(id!(transcript_selection))
                                .fill_width()
                                .child(VirtualColumn::new()
                                    .window(transcript_window)
                                    .gap(TRANSCRIPT_GAP)
                                    .fill_width()
                                    .children(state.items.iter().enumerate()
                                        .skip(transcript_range.start)
                                        .take(transcript_range.len())
                                        .map(|(index, item)| {
                                            let expanded = state.expanded_items.contains(&item.id);
                                            let activity = is_collapsible_activity(&item.kind);
                                            let document = if !activity || expanded {
                                                state.markdown_document(index)
                                            } else {
                                                empty_activity_document()
                                            };
                                            ui! { <ItemCard key={item.id.clone()} item={item} document={document} expanded={expanded} outcome={state.activity_outcomes.get(&item.id).unwrap_or(&EMPTY_ACTIVITY_OUTCOME)} file_summary={state.file_change_summaries.get(&item.id).map(String::as_str).unwrap_or("")} theme={theme} /> }
                                        })))}
                            </Container>
                        })
                    }}
                </Column>
                {[()].into_iter().filter(|_| state.new_content_while_unpinned && !state.conversation_pinned).map(|_| ui! {
                    <Button id={id!(jump_to_latest)} on_press={ChatMessage::JumpToLatest}
                        background={theme.surfaces.raised} color={theme.text.primary}
                        align_self={Align::Center}>{"Jump to latest"}</Button>
                })}
                <Container id={id!(composer)} fill_width shrink={0.0}
                    navigation_scope={NavigationScope::group()}>
                <Column fill_width gap={8.0}>
                    // A backend may have several requests; keep the transcript
                    // reserve and editor visible while the requests remain scrollable.
                    <Column id={id!(pending_interactions)} fill_width max_height={if narrow { 146.0 } else { 164.0 }}
                        overflow_y={Overflow::Auto} gap={8.0}>
                    {state.pending.iter().map(|interaction| ui! {
                        <InteractionCard interaction={interaction} answer={&state.interaction_answer}
                            actionable={state.interaction_is_actionable(match interaction {
                                PendingInteraction::Approval { request_id, .. } | PendingInteraction::UserInput { request_id, .. } => request_id,
                            })}
                            submitting={state.interaction_submission_pending(match interaction {
                                PendingInteraction::Approval { request_id, .. } | PendingInteraction::UserInput { request_id, .. } => request_id,
                            })}
                            delivery={InteractionDeliveryStatus {
                                response_unconfirmed: state.interaction_response_unconfirmed(match interaction {
                                    PendingInteraction::Approval { request_id, .. } | PendingInteraction::UserInput { request_id, .. } => request_id,
                                }),
                                notification_unavailable: state.approval_notification_unavailable(match interaction {
                                    PendingInteraction::Approval { request_id, .. } | PendingInteraction::UserInput { request_id, .. } => request_id,
                                }),
                            }}
                            theme={theme} compact_approval_details={narrow} />
                    })}
                    </Column>
                    {pending_shell_command.map(|command| ui! {
                        <Column fill_width padding={Insets::all(10.0)} gap={8.0}
                            background={theme.surfaces.raised} radius={6.0}>
                            <Text color={theme.text.primary}>{"Run this command outside the Codex sandbox?"}</Text>
                            <Text color={theme.text.secondary}>{format!("!{command}")}</Text>
                            <Row gap={8.0}>
                                <Button on_press={ChatMessage::CancelShell} background={theme.surfaces.hover} color={theme.text.primary}>{"Cancel"}</Button>
                                <Button on_press={ChatMessage::ConfirmShell} background={theme.surfaces.hover} color={theme.text.warning}>{"Run unsandboxed"}</Button>
                            </Row>
                        </Column>
                    })}
                    {state.thread_error.as_ref().map(|diagnostic| ui! {
                        <Row fill_width padding={Insets::all(10.0)} gap={10.0}
                            background={theme.surfaces.raised} radius={6.0}>
                            <Text color={theme.text.primary} grow={1.0}>{format!("Conversations unavailable: {diagnostic}")}</Text>
                            <Button on_press={ChatMessage::Refresh} background={theme.surfaces.hover} color={theme.text.primary}>{"Retry"}</Button>
                        </Row>
                    })}
                    // Attachment count is independent of viewport height. Keep removal
                    // reachable without letting previews displace the editor or transcript.
                    <Column id={id!(attachment_list)} fill_width max_height={96.0}
                        overflow_y={Overflow::Auto} gap={8.0}>
                    {state.attachments.iter().map(|attachment| ui! {
                        <Row padding={Insets::all(8.0)} gap={8.0} background={theme.surfaces.sidebar} radius={6.0}>
                            <Image asset_id={attachment.id.0 as u16} image={attachment.preview.clone()} generation={attachment.id.0}
                                width={48.0} height={48.0} fit={ImageFit::Contain} decorative />
                            <Text color={theme.text.secondary}>{format!("{} × {} · {} KiB", attachment.width, attachment.height, attachment.encoded_size.div_ceil(1024))}</Text>
                            <Button on_press={ChatMessage::RemoveAttachment(attachment.id)}
                                background={theme.surfaces.hover} color={theme.text.danger}
                                accessibility_label={format!("Remove image attachment {}", attachment.id.0)}>{"Remove"}</Button>
                        </Row>
                    })}
                    </Column>
                    {slash_search_results(&state.draft).map(|options| ui! {
                        <Column id={id!(slash_search)} fill_width max_height={220.0}
                            overflow_y={Overflow::Auto} padding={Insets::all(8.0)} gap={4.0}
                            background={theme.surfaces.raised} radius={8.0}
                            accessibility_label={"Slash commands"} semantic_role={SemanticRole::Group}>
                            {options.into_iter().map(|option| {
                                if option.available {
                                    AnyView::new(Button::new(
                                        ChatMessage::SelectCommand(option.command.to_owned()),
                                        option.label,
                                    )
                                        .id(UiId::from(option.command))
                                        .background(if selected_slash_command == Some(option.command) {
                                            theme.accent.soft
                                        } else {
                                            theme.surfaces.hover
                                        })
                                        .color(theme.text.primary)
                                        .fill_width())
                                } else {
                                    AnyView::new(ui! {
                                        <Container key={option.command} fill_width padding={Insets::all(8.0)}>
                                            <Text color={theme.text.secondary} wrap={true}>{option.label}</Text>
                                        </Container>
                                    })
                                }
                            })}
                        </Column>
                    })}
                    {state.file_search_query.as_ref().map(|_| ui! {
                        <Column id={id!(file_mention_search)} fill_width max_height={220.0}
                            overflow_y={Overflow::Auto} padding={Insets::all(8.0)} gap={4.0}
                            background={theme.surfaces.raised} radius={8.0}
                            accessibility_label={"Project files"} semantic_role={SemanticRole::Group}>
                            {[()].into_iter().filter(|_| state.file_search_pending).map(|_| ui! {
                                <Text color={theme.text.secondary}>{"Searching project files…"}</Text>
                            })}
                            {state.file_search_matches.iter().take(20).map(|file| {
                                let label = if file.file_name == file.path {
                                    file.path.clone()
                                } else {
                                    format!("{} — {}", file.file_name, file.path)
                                };
                                AnyView::new(Button::new(
                                    ChatMessage::SelectMention(file.path.clone()),
                                    label,
                                )
                                    .id(UiId::from(format!("mention:{}", file.path)))
                                    .background(if selected_mention_path == Some(file.path.as_str()) {
                                        theme.accent.soft
                                    } else {
                                        theme.surfaces.hover
                                    })
                                    .color(theme.text.primary)
                                    .fill_width())
                            })}
                            {state.file_search_error.as_ref().map(|error| ui! {
                                <Text color={theme.text.danger} wrap={true}>{error}</Text>
                            })}
                            {[()].into_iter().filter(|_| !state.file_search_pending
                                && state.file_search_error.is_none()
                                && state.file_search_matches.is_empty()).map(|_| ui! {
                                <Text color={theme.text.secondary}>{"No matching project files"}</Text>
                            })}
                        </Column>
                    })}
                    <Text color={theme.text.secondary} scale={0.85}>
                        {run_status_label(state.run_presentation_status())}</Text>
                    {[()].into_iter().filter(|_| plan_mode).map(|_| ui! {
                        <Text color={theme.accent.ordinary} scale={0.85}>{"Plan mode"}</Text>
                    })}
                    {state.command_feedback.as_ref().map(|feedback| ui! {
                        <Text color={theme.text.secondary} wrap={true}>{feedback}</Text>
                    })}
                    {queued_messages.into_iter().flat_map(|messages| messages.iter()).enumerate().map(|(index, queued)| {
                        let phase = match queued.phase {
                            QueuedMessagePhase::Waiting => "Queued",
                            QueuedMessagePhase::InterruptRequested => "Requesting interrupt…",
                            QueuedMessagePhase::BoundaryPending => "Waiting for turn boundary…",
                            QueuedMessagePhase::InterruptTimedOut => {
                                "Interrupt unconfirmed — not sent"
                            }
                            QueuedMessagePhase::Dispatching => "Sending…",
                            QueuedMessagePhase::Unconfirmed => "Delivery unconfirmed — review before retry; a second send may duplicate it",
                        };
                        let target = if state.selected_thread.as_ref() == Some(&queued.thread_id) {
                            "Current conversation"
                        } else {
                            "Previous conversation — review before retry"
                        };
                        ui! {
                            <Row fill_width gap={8.0}
                                padding={Insets::all(8.0)} background={theme.surfaces.raised}
                                border={Border::new(theme.borders.ordinary, 1.0)} radius={8.0}>
                                <Column grow={1.0} min_width={0.0} gap={3.0}>
                                    <Text color={theme.text.primary} wrap={true}>{queued.text.clone()}</Text>
                                    <Text color={theme.text.secondary} scale={0.85}>{format!("{} of {} · {target} · {phase}", index + 1, queued_count)}</Text>
                                </Column>
                                {[()].into_iter().filter(|_| index == 0 && matches!(queued.phase, QueuedMessagePhase::Waiting | QueuedMessagePhase::InterruptTimedOut) && state.active_turn.is_some()).map(|_| ui! {
                                    <Button on_press={ChatMessage::InterruptAndSend}
                                        background={theme.accent.ordinary} color={theme.accent.on_accent}>
                                        {"Interrupt and send"}
                                    </Button>
                                })}
                                {[()].into_iter().filter(|_| index == 0 && queued.phase != QueuedMessagePhase::Dispatching).map(|_| ui! {
                                    <Button on_press={ChatMessage::CancelQueuedMessage(queued.id)}
                                        background={theme.surfaces.hover} color={theme.text.primary}>
                                        {"Cancel"}
                                    </Button>
                                })}
                                {[()].into_iter().filter(|_| index == 0 && matches!(queued.phase, QueuedMessagePhase::Waiting | QueuedMessagePhase::InterruptTimedOut | QueuedMessagePhase::Unconfirmed)).map(|_| ui! {
                                    <Button on_press={ChatMessage::EditQueuedMessage(queued.id)}
                                        background={theme.surfaces.hover} color={theme.text.primary}>
                                        {if queued.phase == QueuedMessagePhase::Unconfirmed { "Review" } else { "Edit" }}
                                    </Button>
                                })}
                                {[()].into_iter().filter(|_| index == 0 && queued.phase == QueuedMessagePhase::Unconfirmed).map(|_| ui! {
                                    <Button on_press={ChatMessage::RetryQueuedMessage(queued.id)}
                                        background={theme.surfaces.hover} color={theme.text.danger}>
                                        {"Retry"}
                                    </Button>
                                })}
                            </Row>
                        }
                    })}
                    <Row id={id!(composer_status)} fill_width shrink={0.0} gap={8.0} align_items={Align::End}>
                        <Container id={id!(composer_viewport)} accessibility_label={"Message composer"}
                            semantic_role={SemanticRole::Group}
                            grow={1.0} min_width={0.0} max_width={(viewport_width - 80.0).max(80.0)}
                            min_height={52.0} max_height={140.0} shrink={0.0}
                            padding={Insets::all(12.0)} background={theme.surfaces.card}
                            border={Border::new(theme.borders.ordinary, 1.0)} radius={10.0}
                            overflow_y={Overflow::Auto} follow_scroll_end={true}>
                            <TextField id={id!(chat_draft)} value={&state.draft} on_change={draft_changed}
                                color={theme.text.primary} wrap={true} />
                        </Container>
                        {if state.interrupt_requested {
                            ui! { <Text color={theme.text.secondary}>{"Interrupting…"}</Text> }
                        } else if editing_queued_message {
                            ui! { <Button on_press={ChatMessage::QueueMessage}
                                background={theme.accent.ordinary} color={theme.accent.on_accent}
                                enabled={!state.draft.trim().is_empty() || !state.attachments.is_empty()}>{if editing_unconfirmed_message { "Save for review" } else { "Save queued message" }}</Button> }
                        } else if state.active_turn.is_some() {
                            ui! {
                                <Row gap={8.0} align_items={Align::End}>
                                    <Button on_press={ChatMessage::QueueMessage}
                                        background={theme.accent.ordinary} color={theme.accent.on_accent}
                                        enabled={(!state.draft.trim().is_empty() || !state.attachments.is_empty()) && queued_count < QUEUED_MESSAGE_CAPACITY}>
                                        {"Queue"}
                                    </Button>
                                    <Button on_press={ChatMessage::Interrupt} background={theme.surfaces.hover} color={theme.text.danger}>{"Interrupt"}</Button>
                                </Row>
                            }
                        } else if state.account.authenticated {
                            ui! { <Button id={id!(send_button)} on_press={ChatMessage::Send}
                                background={theme.accent.ordinary} color={theme.accent.on_accent}
                                enabled={state.can_send()} shrink={0.0}>{"Send"}</Button> }
                        } else {
                            ui! { <Text color={theme.text.secondary}>{"Sign in to send"}</Text> }
                        }}
                    </Row>
                </Column>
                </Container>
            </Column>
            })} }
        </Column>
    }
}

#[cfg(test)]
mod tests;
