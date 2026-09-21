use std::path::PathBuf;

use nickel_codex::{
    ApprovalPolicy, BackendChoice, CodexSettings, CommandDecision, FileChangeDecision, RemoteHost,
    SandboxPolicy, ServerRequestId,
};
use nickel_markdown::{MarkdownDocument, MarkdownPalette, markdown_content_view};
use nickel_ui::SemanticRole;
use nickel_ui::ShortcutOutcome;
use nickel_ui::approval::{ApprovalPresentation, RequesterIdentity};
use nickel_ui::prelude::*;

#[cfg(test)]
use crate::model::item_markdown_document;
use crate::model::{ActivityOutcome, RunPresentationStatus, is_collapsible_activity, item_label};
use crate::{
    BackendMode, ChatController, ChatItem, ChatItemKind, ChatState, ConnectionStatus,
    ControllerCommand, ControllerEvent, PendingInteraction, create_managed_workspace,
};

const TRANSCRIPT_GAP: f32 = 10.0;
const TRANSCRIPT_OVERSCAN: f32 = 900.0;
// Height estimates are deliberately cheap, not layout-exact. For a modest
// transcript, drawing every row is inexpensive and avoids a false leading
// spacer when rich Markdown renders much shorter than its text estimate.
const TRANSCRIPT_VIRTUALIZATION_THRESHOLD: usize = 64;
static EMPTY_ACTIVITY_OUTCOME: std::sync::LazyLock<ActivityOutcome> =
    std::sync::LazyLock::new(ActivityOutcome::default);

fn semantic_theme() -> SemanticTheme {
    // Standalone fallback only. Embedded surfaces receive the shell's resolved
    // semantic theme through `ChatApplication::set_theme`.
    SemanticTheme::from_tokens(nickel_ui::SemanticTokenSet::standard(
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
    ConversationScrolled(nickel_ui::ScrollExtent),
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

fn conversation_scrolled(extent: nickel_ui::ScrollExtent) -> ChatMessage {
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

fn transcript_heights(state: &ChatState) -> Vec<f32> {
    state.estimated_item_heights()
}

fn transcript_window(state: &ChatState) -> VirtualWindow {
    if state.items.len() <= TRANSCRIPT_VIRTUALIZATION_THRESHOLD {
        return VirtualWindow {
            range: 0..state.items.len(),
            leading: 0.0,
            trailing: 0.0,
            total: 0.0,
        };
    }
    let heights = transcript_heights(state);
    let offset = if state.conversation_pinned {
        f32::MAX
    } else {
        state.conversation_scroll
    };
    let mut window = VirtualWindow::from_heights(
        &heights,
        TRANSCRIPT_GAP,
        offset,
        state.conversation_viewport_height,
        TRANSCRIPT_OVERSCAN,
    );
    if state.conversation_pinned {
        // A long Markdown source can estimate much taller than its rendered
        // card. Keep a bounded tail mounted so a pinned transcript cannot
        // collapse into an apparent blank viewport above its last message.
        let earliest_tail = heights
            .len()
            .saturating_sub(TRANSCRIPT_VIRTUALIZATION_THRESHOLD);
        if window.range.start > earliest_tail {
            window.range.start = earliest_tail;
            window.leading = heights[..earliest_tail].iter().sum::<f32>()
                + TRANSCRIPT_GAP * earliest_tail as f32;
        }
    }
    window
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

impl Application for ChatApplication {
    type Message = ChatMessage;

    fn remote_access_protected(&self) -> bool {
        self.state.login_pending
            || self.state.login_challenge.is_some()
            || self.remote_control_open
            || self.feedback_open
            || self.state.remote_pairing.is_some()
            || self.managing_hosts
            || self.host_editor.is_some()
            || (!self.state.account.authenticated && self.state.items.is_empty())
    }

    fn update(&mut self, message: Self::Message) {
        // User activity commonly sends work to the controller. Restore low-latency polling;
        // empty polls will back off again without rebuilding the UI.
        self.controller_poll_interval = CONTROLLER_POLL_MIN;
        match message {
            ChatMessage::DraftChanged(value) => {
                self.state.draft = value;
                self.slash_selection = 0;
                self.mention_selection = 0;
                self.state.command_feedback = None;
                self.update_file_search();
            }
            ChatMessage::PasteImage(bytes) => {
                if let Err(error) = self.state.attach_image(&bytes) {
                    self.state.report_diagnostic(error.to_string());
                }
            }
            ChatMessage::RemoveAttachment(id) => {
                self.state.remove_attachment(id);
            }
            ChatMessage::Send => {
                if self.new_chat_pending {
                    return;
                }
                let command = self.state.draft.trim().to_owned();
                if command == "/model" {
                    self.state.draft.clear();
                    self.run_settings_open = true;
                    self.update(ChatMessage::ToggleModelPicker);
                } else if command == "/permissions" {
                    self.state.draft.clear();
                    self.run_settings_open = true;
                    self.update(ChatMessage::ToggleApprovalPicker);
                } else if command == "/status" {
                    self.state.draft.clear();
                    self.diagnostics_open = true;
                    self.remote_control_open = false;
                    self.resume_picker_open = false;
                    self.state.rate_limits_pending = true;
                    self.state.rate_limits_error = None;
                    if !self.controller.send(ControllerCommand::ReadRateLimits) {
                        self.state.rate_limits_pending = false;
                        self.state.rate_limits_error =
                            Some("Codex is disconnected; reconnect to refresh usage".into());
                    }
                } else if command == "/compact" {
                    if self.controller.send(ControllerCommand::Compact) {
                        self.state.draft.clear();
                        self.state.command_feedback = Some("Starting compaction…".into());
                    } else {
                        self.state.command_feedback =
                            Some("Codex is disconnected; retry /compact after reconnecting".into());
                    }
                } else if command == "/review" {
                    if self.controller.send(ControllerCommand::Review {
                        model: self.state.selected_model.clone(),
                        reasoning_effort: self.state.selected_reasoning_effort.clone(),
                        approval_policy: self.state.selected_approval_policy,
                        sandbox_policy: self.state.selected_sandbox_policy,
                    }) {
                        self.state.draft.clear();
                        self.state.command_feedback = Some("Starting review…".into());
                    } else {
                        self.state.command_feedback =
                            Some("Codex is disconnected; retry /review after reconnecting".into());
                    }
                } else if command == "/plan" {
                    self.state.draft.clear();
                    self.plan_mode = !self.plan_mode;
                    self.state.command_feedback = Some(if self.plan_mode {
                        "Plan mode is on for subsequent turns".into()
                    } else {
                        "Plan mode is off".into()
                    });
                } else if command == "/mention" {
                    self.update(ChatMessage::DraftChanged("@".into()));
                } else if command == "/feedback" {
                    self.state.draft.clear();
                    self.feedback_open = true;
                    self.feedback_classification = None;
                    self.feedback_reason.clear();
                    self.feedback_pending = false;
                } else if command == "/diff" {
                    if self.controller.send(ControllerCommand::Diff) {
                        self.state.draft.clear();
                        self.state.command_feedback = Some("Loading working-tree diff…".into());
                    } else {
                        self.state.command_feedback =
                            Some("Codex is disconnected; retry /diff after reconnecting".into());
                    }
                } else if command == "/logout" {
                    if self.controller.send(ControllerCommand::Logout) {
                        self.state.draft.clear();
                        self.state.command_feedback = Some("Signing out…".into());
                    } else {
                        self.state.command_feedback =
                            Some("Codex is disconnected; retry /logout after reconnecting".into());
                    }
                } else if command == "/resume" {
                    self.state.draft.clear();
                    self.update(ChatMessage::ToggleResumePicker);
                } else if matches!(command.as_str(), "/new" | "/clear") {
                    self.state.draft.clear();
                    self.update(ChatMessage::NewChat);
                } else if command.starts_with('/') {
                    self.settings_error = Some(format!(
                        "{} is not supported in this build yet",
                        command.split_whitespace().next().unwrap_or(&command)
                    ));
                } else if self.state.can_send() && self.state.draft.trim_start().starts_with('!') {
                    let draft = std::mem::take(&mut self.state.draft);
                    let command = draft.trim_start()[1..].trim().to_owned();
                    if command.is_empty() {
                        self.settings_error = Some("Enter a command after !".into());
                    } else if self.shell_warning_acknowledged {
                        self.controller.send(ControllerCommand::Shell(command));
                    } else {
                        self.pending_shell_command = Some(command);
                    }
                } else if self.plan_mode && self.state.selected_model.is_none() {
                    self.state.command_feedback =
                        Some("Choose a model before sending in Plan mode".into());
                } else if let Some((text, images)) = self.state.begin_send() {
                    self.controller.send(ControllerCommand::Send {
                        text,
                        images,
                        model: self.state.selected_model.clone(),
                        reasoning_effort: self.state.selected_reasoning_effort.clone(),
                        approval_policy: self.state.selected_approval_policy,
                        sandbox_policy: self.state.selected_sandbox_policy,
                        plan_mode: self.plan_mode,
                    });
                }
            }
            ChatMessage::ConfirmShell => {
                if let Some(command) = self.pending_shell_command.take() {
                    self.shell_warning_acknowledged = true;
                    self.controller.send(ControllerCommand::Shell(command));
                }
            }
            ChatMessage::CancelShell => self.pending_shell_command = None,
            ChatMessage::StartLogin(method) => {
                self.state.login_pending = true;
                self.state.login_status = crate::model::LoginPresentationState::Starting;
                self.state.login_challenge = None;
                self.controller.send(ControllerCommand::StartLogin(method));
            }
            ChatMessage::CancelLogin(login_id) => {
                self.state.login_pending = true;
                self.state.login_status = crate::model::LoginPresentationState::Cancelling;
                self.controller
                    .send(ControllerCommand::CancelLogin(login_id));
            }
            ChatMessage::OpenLoginUrl(destination) => {
                if let Err(error) = nickel_platform::open_external_url(&destination) {
                    self.state.report_diagnostic(error);
                }
            }
            ChatMessage::CopyLoginText(text) => {
                self.clipboard_write = Some(text);
                self.clipboard_write_purpose = Some(ClipboardWritePurpose::Other);
            }
            ChatMessage::OpenRemoteControl => {
                self.remote_control_open = true;
                self.diagnostics_open = false;
                self.state.remote_control_pending = true;
                self.controller.send(ControllerCommand::ReadRemoteControl);
            }
            ChatMessage::CloseRemoteControl => {
                self.remote_control_open = false;
                if self.state.remote_pairing.is_some() {
                    // Leaving the QR view must not keep a pairing attempt active in
                    // Nickel's controller while its secret is no longer visible.
                    self.controller.send(ControllerCommand::CancelRemotePairing);
                }
            }
            ChatMessage::ToggleDiagnostics => {
                self.diagnostics_open = !self.diagnostics_open;
                self.diagnostic_copy_result = None;
                if self.diagnostics_open {
                    self.remote_control_open = false;
                    self.resume_picker_open = false;
                    self.diagnostics_details_open = false;
                }
            }
            ChatMessage::ToggleDiagnosticDetails => {
                self.diagnostics_details_open = !self.diagnostics_details_open;
            }
            ChatMessage::CopyDiagnosticSummary => {
                self.clipboard_write = Some(safe_diagnostic_summary(&self.state, &self.mode));
                self.clipboard_write_purpose = Some(ClipboardWritePurpose::DiagnosticSummary);
                self.diagnostic_copy_result = None;
            }
            ChatMessage::RefreshRemoteControl => {
                self.state.remote_control_pending = true;
                self.controller.send(ControllerCommand::ReadRemoteControl);
            }
            ChatMessage::EnableRemoteControl => {
                self.state.remote_control_pending = true;
                self.controller.send(ControllerCommand::EnableRemoteControl);
            }
            ChatMessage::DisableRemoteControl => {
                self.state.remote_control_pending = true;
                self.controller
                    .send(ControllerCommand::DisableRemoteControl);
            }
            ChatMessage::StartRemotePairing => {
                self.state.remote_control_pending = true;
                self.controller.send(ControllerCommand::StartRemotePairing);
            }
            ChatMessage::CancelRemotePairing => {
                self.state.remote_control_pending = true;
                self.controller.send(ControllerCommand::CancelRemotePairing);
            }
            ChatMessage::RevokeRemoteClient(environment_id, client_id) => {
                self.state.remote_control_pending = true;
                self.controller.send(ControllerCommand::RevokeRemoteClient {
                    environment_id,
                    client_id,
                });
            }
            ChatMessage::ToggleModelPicker => {
                self.model_picker_generation = self.model_picker_generation.saturating_add(1);
                self.resume_picker_open = false;
            }
            ChatMessage::ToggleReasoningPicker => {
                self.reasoning_picker_generation =
                    self.reasoning_picker_generation.saturating_add(1);
                self.resume_picker_open = false;
            }
            ChatMessage::ToggleApprovalPicker => {
                self.approval_picker_generation = self.approval_picker_generation.saturating_add(1);
                self.resume_picker_open = false;
            }
            ChatMessage::ToggleSandboxPicker => {
                self.sandbox_picker_generation = self.sandbox_picker_generation.saturating_add(1);
                self.resume_picker_open = false;
            }
            ChatMessage::ToggleRunSettings => {
                self.run_settings_open = !self.run_settings_open;
            }
            ChatMessage::CloseRunSettings => {
                self.run_settings_open = false;
            }
            ChatMessage::SelectModel(model) => {
                self.state.selected_model = Some(model);
                self.state.selected_reasoning_effort = self
                    .state
                    .models
                    .iter()
                    .find(|candidate| {
                        Some(candidate.id.as_str()) == self.state.selected_model.as_deref()
                    })
                    .and_then(|candidate| candidate.default_reasoning_effort.clone());
            }
            ChatMessage::SelectReasoningEffort(effort) => {
                self.state.selected_reasoning_effort = Some(effort);
            }
            ChatMessage::SelectApprovalPolicy(policy) => {
                self.state.selected_approval_policy = policy;
            }
            ChatMessage::SelectSandboxPolicy(policy) => {
                self.state.selected_sandbox_policy = policy;
                if policy == Some(SandboxPolicy::DangerFullAccess) {
                    self.state.selected_approval_policy = ApprovalPolicy::Never;
                }
            }
            ChatMessage::SelectCommand(command) => {
                self.state.draft = command;
                self.update(ChatMessage::Send);
            }
            ChatMessage::SelectMention(path) => {
                self.state.draft = replace_mention_fragment(&self.state.draft, &path);
                self.state.file_search_query = None;
                self.state.file_search_matches.clear();
                self.state.file_search_pending = false;
                self.state.file_search_error = None;
                self.mention_selection = 0;
            }
            ChatMessage::SelectFeedbackCategory(classification)
                if matches!(
                    classification.as_str(),
                    "bug" | "bad_result" | "good_result" | "safety_check" | "other"
                ) =>
            {
                self.feedback_classification = Some(classification);
            }
            ChatMessage::SelectFeedbackCategory(_) => {}
            ChatMessage::FeedbackReasonChanged(reason) => {
                self.feedback_reason = reason.chars().take(16 * 1024).collect();
            }
            ChatMessage::SubmitFeedback(include_logs) => {
                if self.feedback_pending {
                    return;
                }
                let Some(classification) = self.feedback_classification.clone() else {
                    self.state.command_feedback = Some("Choose a feedback category".into());
                    return;
                };
                let reason = (!self.feedback_reason.trim().is_empty())
                    .then(|| self.feedback_reason.trim().to_owned());
                self.feedback_pending = self.controller.send(ControllerCommand::UploadFeedback {
                    classification,
                    reason,
                    include_logs,
                });
                if !self.feedback_pending {
                    self.state.command_feedback =
                        Some("Codex is disconnected; feedback was not sent".into());
                }
            }
            ChatMessage::CloseFeedback => {
                if !self.feedback_pending {
                    self.feedback_open = false;
                    self.feedback_classification = None;
                    self.feedback_reason.clear();
                }
            }
            ChatMessage::ToggleResumePicker => {
                self.resume_picker_open = !self.resume_picker_open;
                if self.resume_picker_open {
                    self.resume_picker_loading = true;
                    self.controller.send(ControllerCommand::LoadThreads);
                } else {
                    self.resume_picker_pending = None;
                }
            }
            ChatMessage::RefreshResumePicker => {
                self.resume_picker_loading = true;
                self.resume_picker_pending = None;
                self.controller.send(ControllerCommand::LoadThreads);
            }
            ChatMessage::LoadMoreThreads => {
                if let Some((cursor, request)) = self.state.begin_thread_page()
                    && !self.controller.send(ControllerCommand::LoadMoreThreads {
                        cursor: cursor.clone(),
                        request,
                    })
                {
                    self.state.apply(
                        self.state.generation,
                        ControllerEvent::ThreadPageFailed {
                            request,
                            cursor,
                            message: "Codex controller stopped before older conversations could be loaded".into(),
                        },
                    );
                }
            }
            ChatMessage::CloseResumePicker => {
                self.resume_picker_open = false;
                self.resume_picker_loading = false;
                self.resume_picker_pending = None;
            }
            ChatMessage::CancelDiscardDraft => {
                self.pending_navigation = None;
                self.open_discard_dialog = false;
            }
            ChatMessage::ConfirmDiscardDraft => {
                let Some(target) = self.pending_navigation.take() else {
                    return;
                };
                self.open_discard_dialog = false;
                if let Some(reason) = self.state.replacement_block_reason() {
                    self.state.report_diagnostic(reason);
                    return;
                }
                match target {
                    PendingNavigation::NewChat => {
                        self.confirmed_new_chat_discard = true;
                        self.update(ChatMessage::NewChat);
                    }
                    PendingNavigation::NewChatIn(cwd, project_id) => {
                        self.confirmed_project_discard = Some((cwd.clone(), project_id.clone()));
                        self.update(ChatMessage::NewChatIn(cwd, project_id));
                    }
                    PendingNavigation::SelectThread(id) => {
                        // Retain the draft until hydration succeeds; a failed resume must not
                        // discard it merely because the user authorized a successful switch.
                        self.confirmed_thread_discard = Some(id.clone());
                        self.update(ChatMessage::SelectThread(id))
                    }
                }
            }
            ChatMessage::NewChat => {
                if self.new_chat_pending {
                    return;
                }
                if self.project_menu_mode {
                    self.settings_error =
                        Some("Choose + beside a project for a new conversation".into());
                    return;
                }
                self.mark_queued_messages_for_review("changing conversation");
                if let Some(reason) = self.state.replacement_block_reason() {
                    self.state.report_diagnostic(reason);
                    return;
                }
                if (!self.state.draft.is_empty() || !self.state.attachments.is_empty())
                    && !self.confirmed_new_chat_discard
                {
                    self.pending_navigation = Some(PendingNavigation::NewChat);
                    self.open_discard_dialog = true;
                    return;
                }
                self.resume_picker_open = false;
                self.resume_picker_pending = None;
                let accepted = if let Some((cwd, project_id)) = &self.shell_project {
                    self.controller.send(ControllerCommand::NewChatIn(
                        cwd.clone(),
                        project_id.clone(),
                    ))
                } else {
                    self.controller.send(ControllerCommand::NewChat)
                };
                if accepted {
                    // Retain the transcript until the controller acknowledges the transition.
                    self.new_chat_pending = true;
                } else {
                    self.state.report_diagnostic(
                        "Codex controller stopped before the new conversation could be created",
                    );
                }
                self.confirmed_new_chat_discard = false;
            }
            ChatMessage::NewChatIn(cwd, project_id) => {
                if self.new_chat_pending {
                    return;
                }
                if self.project_menu_mode {
                    let name = self
                        .state
                        .projects
                        .iter()
                        .find(|project| project.id == project_id)
                        .map(|project| project.name.clone())
                        .unwrap_or_else(|| project_window_name(&cwd));
                    self.shell_requests.push(ShellRequest::OpenProject {
                        cwd,
                        project_id,
                        name,
                        initial_thread: None,
                    });
                    return;
                }
                self.mark_queued_messages_for_review("changing conversation");
                if let Some(reason) = self.state.replacement_block_reason() {
                    self.state.report_diagnostic(reason);
                    return;
                }
                if (!self.state.draft.is_empty() || !self.state.attachments.is_empty())
                    && self.confirmed_project_discard.as_ref()
                        != Some(&(cwd.clone(), project_id.clone()))
                {
                    self.pending_navigation = Some(PendingNavigation::NewChatIn(cwd, project_id));
                    self.open_discard_dialog = true;
                    return;
                }
                if self
                    .controller
                    .send(ControllerCommand::NewChatIn(cwd.clone(), Some(project_id)))
                {
                    self.new_chat_pending = true;
                    self.pending_new_chat_title = Some(project_window_title(&cwd));
                } else {
                    self.state.report_diagnostic("Codex controller stopped before the new project conversation could be created");
                }
                self.confirmed_project_discard = None;
            }
            ChatMessage::Refresh => {
                if matches!(
                    self.state.status,
                    ConnectionStatus::Unavailable
                        | ConnectionStatus::Disconnected
                        | ConnectionStatus::Incompatible
                ) {
                    self.reconnect_controller();
                } else {
                    self.controller.send(ControllerCommand::Refresh);
                }
            }
            ChatMessage::Reconnect => {
                self.reconnect_controller();
            }
            ChatMessage::SelectThread(id) => {
                if self.new_chat_pending {
                    return;
                }
                if self.resume_picker_pending.is_some() {
                    return;
                }
                self.mark_queued_messages_for_review("changing conversation");
                if !self.shell_host {
                    if let Some(reason) = self.state.replacement_block_reason() {
                        self.state.report_diagnostic(reason);
                        return;
                    }
                    if (!self.state.draft.is_empty() || !self.state.attachments.is_empty())
                        && self.confirmed_thread_discard.as_ref() != Some(&id)
                    {
                        self.pending_navigation = Some(PendingNavigation::SelectThread(id));
                        self.open_discard_dialog = true;
                        return;
                    }
                }
                if self.project_menu_mode {
                    return;
                }
                if self.shell_host {
                    self.resume_picker_pending = Some(id.clone());
                    self.shell_requests.push(ShellRequest::ResumeThread(id));
                    return;
                }
                self.resume_picker_pending = Some(id.clone());
                self.resume_request_started = Some(std::time::Instant::now());
                if !self.controller.send(ControllerCommand::SelectThread(id)) {
                    self.resume_request_started = None;
                    self.resume_picker_pending = None;
                    self.confirmed_thread_discard = None;
                    self.state.report_diagnostic(
                        "Codex controller stopped before the conversation could be resumed",
                    );
                }
            }
            ChatMessage::Interrupt => {
                if self.state.active_turn.is_some() && !self.state.interrupt_requested {
                    self.state.interrupt_requested = true;
                    self.controller.send(ControllerCommand::Interrupt);
                }
            }
            ChatMessage::QueueMessage => self.queue_current_draft(),
            ChatMessage::InterruptAndSend => self.request_queued_interrupt(),
            ChatMessage::CancelQueuedMessage(id) => {
                if self.queued_messages.front().is_some_and(|queued| {
                    queued.id == id && queued.phase != QueuedMessagePhase::Dispatching
                }) {
                    self.queued_messages.pop_front();
                    self.queued_interrupt_deadline = None;
                    self.state.command_feedback = Some("Queued message cancelled".into());
                }
            }
            ChatMessage::EditQueuedMessage(id) => {
                if !self.state.draft.is_empty() || !self.state.attachments.is_empty() {
                    self.state.command_feedback =
                        Some("Clear or queue the current draft before editing this message".into());
                    return;
                }
                if self.queued_messages.front().is_some_and(|queued| {
                    queued.id == id
                        && matches!(
                            queued.phase,
                            QueuedMessagePhase::Waiting
                                | QueuedMessagePhase::InterruptTimedOut
                                | QueuedMessagePhase::Unconfirmed
                        )
                }) && let Some(queued) = self.queued_messages.pop_front()
                {
                    self.state.draft = queued.text.clone();
                    self.editing_queued_message = Some(queued);
                    self.state.command_feedback = Some(
                        "Queued message returned to the composer; its attachments remain preserved"
                            .into(),
                    );
                }
            }
            ChatMessage::RetryQueuedMessage(id) => {
                if self.editing_queued_message.is_some() {
                    return;
                }
                let Some(queued) = self.queued_messages.front_mut() else {
                    return;
                };
                if queued.id != id || queued.phase != QueuedMessagePhase::Unconfirmed {
                    return;
                }
                if self.state.selected_thread.as_ref() != Some(&queued.thread_id)
                    || self.state.status != ConnectionStatus::Ready
                    || self.state.unconfirmed_work
                    || self.state.recovery_pending
                    || self.state.active_turn.is_some()
                {
                    self.state.command_feedback = Some(
                        "Reconnect and review the original conversation before retrying; sending again may duplicate the message".into(),
                    );
                    return;
                }
                queued.generation = self.state.generation;
                queued.phase = QueuedMessagePhase::Waiting;
                self.dispatch_queued_after_boundary();
            }
            ChatMessage::RespondApproval(request_id, approval_type, choice) => {
                // Validate the exact live source decision before marking the
                // request submitting; a stale button cannot broaden authority.
                let allowed = self.state.pending.iter().any(|interaction| {
                    matches!(interaction,
                        PendingInteraction::Approval { request_id: pending, approval_type: kind, context, .. }
                        if pending == &request_id && kind == &approval_type
                            && approval_choice_allowed(kind, context, &choice))
                });
                if !allowed
                    || !self
                        .state
                        .begin_approval_response(&request_id, &approval_type)
                {
                    return;
                }
                let command = match (approval_type.as_str(), choice) {
                    ("item/fileChange/requestApproval", CodexApprovalChoice::Approve) => {
                        ControllerCommand::FileApproval {
                            request_id: request_id.clone(),
                            decision: FileChangeDecision::Accept,
                        }
                    }
                    ("item/fileChange/requestApproval", CodexApprovalChoice::Decline) => {
                        ControllerCommand::FileApproval {
                            request_id: request_id.clone(),
                            decision: FileChangeDecision::Decline,
                        }
                    }
                    ("item/fileChange/requestApproval", CodexApprovalChoice::Cancel) => {
                        ControllerCommand::FileApproval {
                            request_id: request_id.clone(),
                            decision: FileChangeDecision::Cancel,
                        }
                    }
                    ("item/commandExecution/requestApproval", choice) => {
                        let decision = match choice {
                            CodexApprovalChoice::Approve => CommandDecision::Accept,
                            CodexApprovalChoice::Decline => CommandDecision::Decline,
                            CodexApprovalChoice::Cancel => CommandDecision::Cancel,
                            CodexApprovalChoice::Command(decision) => decision,
                        };
                        ControllerCommand::CommandApproval {
                            request_id: request_id.clone(),
                            decision,
                        }
                    }
                    _ => return,
                };
                self.dispatch_interaction_response(request_id, command);
            }
            ChatMessage::InteractionAnswerChanged(value) => {
                self.state.interaction_answer = value;
            }
            ChatMessage::SubmitInput(request_id, question_ids) => {
                if !self.state.begin_input_response(&request_id, &question_ids) {
                    return;
                }
                let answer = std::mem::take(&mut self.state.interaction_answer);
                let lines: Vec<_> = answer.lines().map(str::to_owned).collect();
                self.dispatch_interaction_response(
                    request_id.clone(),
                    ControllerCommand::UserInput {
                        request_id,
                        answers: question_ids
                            .into_iter()
                            .enumerate()
                            .map(|(index, question_id)| nickel_codex::UserInputAnswer {
                                question_id,
                                answer: lines.get(index).cloned().unwrap_or_default(),
                            })
                            .collect(),
                    },
                );
            }
            ChatMessage::DismissInput(request_id) => {
                let Some(question_ids) =
                    self.state
                        .pending
                        .iter()
                        .find_map(|interaction| match interaction {
                            PendingInteraction::UserInput {
                                request_id: pending,
                                question_ids,
                                ..
                            } if pending == &request_id => Some(question_ids.clone()),
                            _ => None,
                        })
                else {
                    return;
                };
                if !self.state.begin_input_response(&request_id, &question_ids) {
                    return;
                }
                self.dispatch_interaction_response(
                    request_id.clone(),
                    ControllerCommand::UserInput {
                        request_id,
                        answers: Vec::new(),
                    },
                );
            }
            ChatMessage::ConversationScrolled(extent) => {
                let maximum = (extent.content.height - extent.viewport.height).max(0.0);
                self.state.conversation_viewport_height = extent.viewport.height.max(1.0);
                self.state.conversation_scroll = extent.offset;
                self.state.conversation_pinned = extent.offset >= maximum - 20.0;
                if self.state.conversation_pinned {
                    self.state.new_content_while_unpinned = false;
                }
            }
            ChatMessage::JumpToLatest => {
                self.state.conversation_pinned = true;
                self.state.new_content_while_unpinned = false;
                self.state.conversation_scroll = f32::MAX;
            }
            ChatMessage::ToggleActivityDetails(id) => {
                if self.state.items.iter().any(|item| item.id == id)
                    && !self.state.expanded_items.remove(&id)
                {
                    self.state.expanded_items.insert(id);
                }
            }
            ChatMessage::ToggleProject(project) => {
                if !self.state.expanded_projects.remove(&project) {
                    self.state.expanded_projects.insert(project);
                }
            }
            ChatMessage::ToggleProjectCollapsed(project) => {
                if !self.state.collapsed_projects.remove(&project) {
                    self.state.collapsed_projects.insert(project);
                }
            }
            ChatMessage::ToggleFileMenu => {}
            ChatMessage::SelectConnection(id) => self.select_connection(id),
            ChatMessage::ManageRemoteHosts => {
                self.managing_hosts = true;
                self.host_editor = None;
                self.settings_error = None;
            }
            ChatMessage::CloseRemoteHosts => {
                self.managing_hosts = false;
                self.host_editor = None;
                self.settings_error = None;
            }
            ChatMessage::AddRemoteHost => {
                self.host_editor = Some(RemoteHostEditor::empty());
                self.settings_error = None;
            }
            ChatMessage::EditRemoteHost(id) => {
                self.host_editor = self
                    .settings
                    .hosts
                    .iter()
                    .find(|host| host.id == id)
                    .map(RemoteHostEditor::from_host);
                self.settings_error = None;
            }
            ChatMessage::RemoveRemoteHost(id) => {
                let removed_selected = self.settings.selected == id;
                let mut settings = self.settings.clone();
                if settings.remove_host(&id) && self.save_settings(settings) {
                    self.host_editor = None;
                    if removed_selected {
                        self.select_connection("local".into());
                    }
                }
            }
            ChatMessage::RemoteHostIdChanged(value) => {
                if let Some(editor) = &mut self.host_editor {
                    editor.id = value;
                }
            }
            ChatMessage::RemoteHostNameChanged(value) => {
                if let Some(editor) = &mut self.host_editor {
                    editor.name = value;
                }
            }
            ChatMessage::RemoteHostEndpointChanged(value) => {
                if let Some(editor) = &mut self.host_editor {
                    editor.endpoint = value;
                }
            }
            ChatMessage::RemoteHostTokenEnvChanged(value) => {
                if let Some(editor) = &mut self.host_editor {
                    editor.token_env = value;
                }
            }
            ChatMessage::RemoteHostCwdChanged(value) => {
                if let Some(editor) = &mut self.host_editor {
                    editor.default_cwd = value;
                }
            }
            ChatMessage::SaveRemoteHost => {
                let Some(editor) = self.host_editor.clone() else {
                    return;
                };
                let host = editor.host();
                if let Err(error) = host.validate() {
                    self.settings_error = Some(error.to_string());
                    return;
                }
                let mut settings = self.settings.clone();
                if let Some(original) = &editor.original_id {
                    if let Some(existing) = settings
                        .hosts
                        .iter_mut()
                        .find(|existing| existing.id == *original)
                    {
                        *existing = host.clone();
                    }
                    if settings.selected == *original {
                        settings.selected = host.id.clone();
                    }
                } else {
                    settings.hosts.push(host);
                }
                let reconnect = editor
                    .original_id
                    .as_ref()
                    .is_some_and(|original| self.settings.selected == *original);
                if self.save_settings(settings) {
                    self.host_editor = None;
                    if reconnect {
                        self.select_connection(self.settings.selected.clone());
                    }
                }
            }
            ChatMessage::OpenMarkdownLink(destination) => {
                let result = if destination.starts_with("https://")
                    || destination.starts_with("http://")
                    || destination.starts_with("mailto:")
                {
                    nickel_platform::open_external_url(&destination)
                } else {
                    Err(format!("Cannot open relative chat link: {destination}"))
                };
                if let Err(error) = result {
                    self.state.report_diagnostic(error);
                }
            }
        }
    }

    fn paste_clipboard_image(&mut self, width: u32, height: u32, rgba: &[u8]) -> bool {
        match self.state.attach_rgba(width, height, rgba) {
            Ok(_) => true,
            Err(error) => {
                self.state.report_diagnostic(error.to_string());
                true
            }
        }
    }

    fn poll(&mut self) -> bool {
        self.poll_controller()
    }

    fn poll_interval(&self) -> Option<std::time::Duration> {
        Some(self.controller_poll_interval)
    }

    fn shortcut_outcome(&mut self, shortcut: Shortcut) -> ShortcutOutcome {
        let slash_commands = available_slash_commands(&self.state.draft);
        let mention_count = self.state.file_search_matches.len();
        ShortcutOutcome::from_changed(match shortcut {
            Shortcut::NavigateUp if mention_count > 0 => {
                self.mention_selection = self
                    .mention_selection
                    .checked_sub(1)
                    .unwrap_or(mention_count - 1);
                true
            }
            Shortcut::NavigateDown if mention_count > 0 => {
                self.mention_selection = (self.mention_selection + 1) % mention_count;
                true
            }
            Shortcut::Submit if self.state.file_search_query.is_some() => {
                if mention_count > 0 {
                    let path = self.state.file_search_matches
                        [self.mention_selection % mention_count]
                        .path
                        .clone();
                    self.update(ChatMessage::SelectMention(path));
                }
                true
            }
            Shortcut::NavigateUp if !slash_commands.is_empty() => {
                self.slash_selection = self
                    .slash_selection
                    .checked_sub(1)
                    .unwrap_or(slash_commands.len() - 1);
                true
            }
            Shortcut::NavigateDown if !slash_commands.is_empty() => {
                self.slash_selection = (self.slash_selection + 1) % slash_commands.len();
                true
            }
            Shortcut::Submit if !slash_commands.is_empty() => {
                let selected = slash_commands[self.slash_selection % slash_commands.len()];
                self.update(ChatMessage::SelectCommand(selected.command.to_owned()));
                true
            }
            Shortcut::Submit
                if self.state.active_turn.is_some()
                    && self.queued_messages.len() < QUEUED_MESSAGE_CAPACITY
                    && (!self.state.draft.trim().is_empty()
                        || !self.state.attachments.is_empty()) =>
            {
                self.update(ChatMessage::QueueMessage);
                true
            }
            Shortcut::Submit if self.state.can_send() => {
                self.update(ChatMessage::Send);
                true
            }
            Shortcut::Newline => false,
            Shortcut::Escape if self.host_editor.is_some() => {
                self.update(ChatMessage::ManageRemoteHosts);
                true
            }
            Shortcut::Escape if self.managing_hosts => {
                self.update(ChatMessage::CloseRemoteHosts);
                true
            }
            Shortcut::Escape if self.remote_control_open => {
                self.update(ChatMessage::CloseRemoteControl);
                true
            }
            Shortcut::Escape if self.diagnostics_open => {
                self.update(ChatMessage::ToggleDiagnostics);
                true
            }
            Shortcut::Escape if self.feedback_open && !self.feedback_pending => {
                self.update(ChatMessage::CloseFeedback);
                true
            }
            Shortcut::Escape if self.resume_picker_open => {
                self.update(ChatMessage::CloseResumePicker);
                true
            }
            Shortcut::Escape if self.run_settings_open => {
                self.update(ChatMessage::CloseRunSettings);
                true
            }
            Shortcut::Escape if self.state.active_turn.is_some() => {
                self.update(ChatMessage::Interrupt);
                true
            }
            _ => false,
        })
    }

    fn view(&self, context: nickel_ui::ViewContext) -> impl View<Self::Message> {
        let project_root = self
            .shell_project
            .as_ref()
            .map(|(root, _)| root.as_path())
            .or_else(|| {
                // Standalone hosts still have a workspace even without the shell's
                // explicit project lease; navigation should identify that source.
                match &self.mode {
                    BackendMode::Live { cwd, .. } | BackendMode::Replay { cwd, .. } => {
                        Some(cwd.as_path())
                    }
                    BackendMode::Remote { host } => Some(std::path::Path::new(&host.default_cwd)),
                }
            });
        let view = if self.project_menu_mode {
            AnyView::new(project_menu_view(
                &self.state,
                self.settings_error.as_deref(),
                self.theme,
            ))
        } else {
            AnyView::new(configured_chat_view(
                &self.state,
                &self.settings,
                ChatHostPanel {
                    managing_hosts: self.managing_hosts,
                    editor: self.host_editor.as_ref(),
                    settings_error: self.settings_error.as_deref(),
                },
                ChatOverlays {
                    pending_shell_command: self.pending_shell_command.as_deref(),
                    model_picker_generation: self.model_picker_generation,
                    reasoning_picker_generation: self.reasoning_picker_generation,
                    approval_picker_generation: self.approval_picker_generation,
                    sandbox_picker_generation: self.sandbox_picker_generation,
                    run_settings_open: self.run_settings_open,
                    plan_mode: self.plan_mode,
                    slash_selection: self.slash_selection,
                    mention_selection: self.mention_selection,
                    feedback_open: self.feedback_open,
                    feedback_classification: self.feedback_classification.as_deref(),
                    feedback_reason: &self.feedback_reason,
                    feedback_pending: self.feedback_pending,
                    resume_picker_open: self.resume_picker_open,
                    shell_host: self.shell_host,
                    resume_picker_loading: self.resume_picker_loading,
                    resume_picker_pending: self.resume_picker_pending.as_ref(),
                    remote_control_open: self.remote_control_open,
                    diagnostics_open: self.diagnostics_open,
                    diagnostics_details_open: self.diagnostics_details_open,
                    diagnostic_copy_result: self.diagnostic_copy_result.as_ref(),
                    project_root,
                    project_id: self
                        .shell_project
                        .as_ref()
                        .and_then(|(_, id)| id.as_deref()),
                    queued_messages: Some(&self.queued_messages),
                    editing_queued_message: self.editing_queued_message.is_some(),
                    editing_unconfirmed_message: self
                        .editing_queued_message
                        .as_ref()
                        .is_some_and(|queued| queued.phase == QueuedMessagePhase::Unconfirmed),
                },
                self.theme,
                context.viewport.size.width,
            ))
        };
        if crate::controller::codex_resume_timing_enabled()
            && let Some((started, visible_items)) = self.resume_first_view.take()
        {
            eprintln!(
                "nickel: Codex first resumed view constructed in {} ms ({} visible items)",
                started.elapsed().as_millis(),
                visible_items
            );
        }
        view
    }

    fn frame_overlays(
        &self,
        _context: nickel_ui::ViewContext,
    ) -> Vec<nickel_ui::FrameOverlay<Self::Message>> {
        if self.pending_navigation.is_none() {
            return Vec::new();
        }
        // The host owns dialog focus trapping, hit testing, and focus return.
        let dialog = TransientSurface::dialog(
            "codex-discard-draft",
            OverlayAnchor::InvocationTargetCenter(UiId::from("root/menu-bar/file-menu")),
            nickel_ui::Size::new(420.0, 164.0),
            OverlayStyle::from_theme(&self.theme),
        )
        .accessible_name("Discard unsent Codex input?")
        .dismiss(DismissPolicy {
            cancel: false,
            outside_pointer: false,
            action: true,
        });
        vec![nickel_ui::FrameOverlay::surface(
            dialog,
            ui! {
                <Column fill_width fill_height padding={Insets::all(16.0)} gap={12.0}
                    background={self.theme.surfaces.raised}>
                    <Text color={self.theme.text.primary}>{"Discard unsent input?"}</Text>
                    <Text color={self.theme.text.secondary}>{"The current draft and attachments will be lost only if the replacement succeeds."}</Text>
                    <Row gap={8.0}>
                        <Button on_press={ChatMessage::CancelDiscardDraft}
                            background={self.theme.surfaces.hover} color={self.theme.text.primary}>{"Keep working"}</Button>
                        <Button on_press={ChatMessage::ConfirmDiscardDraft}
                            background={self.theme.surfaces.hover} color={self.theme.text.danger}>{"Discard and continue"}</Button>
                    </Row>
                </Column>
            },
        )]
    }

    fn take_transient_request(&mut self) -> Option<(OverlayId, UiId)> {
        if std::mem::take(&mut self.open_discard_dialog) {
            Some((
                OverlayId::new("codex-discard-draft"),
                UiId::from("root/menu-bar/file-menu"),
            ))
        } else {
            None
        }
    }

    fn take_clipboard_write(&mut self) -> Option<String> {
        self.clipboard_write.take()
    }

    fn clipboard_write_completed(&mut self, result: Result<(), String>) -> bool {
        if self.clipboard_write_purpose.take() != Some(ClipboardWritePurpose::DiagnosticSummary) {
            return false;
        }
        let changed = self.diagnostic_copy_result.as_ref() != Some(&result);
        self.diagnostic_copy_result = Some(result);
        changed
    }

    fn title(&self) -> &str {
        &self.window_title
    }

    fn initial_size(&self) -> (u32, u32) {
        (1120, 760)
    }
}

#[component]
fn ItemCard(
    item: &ChatItem,
    document: std::sync::Arc<MarkdownDocument>,
    expanded: bool,
    outcome: Option<&ActivityOutcome>,
    file_summary: Option<&str>,
    theme: SemanticTheme,
) -> impl View<ChatMessage> {
    let (background, color) = match &item.kind {
        ChatItemKind::User => (theme.surfaces.selected, theme.text.primary),
        ChatItemKind::Agent => (theme.surfaces.window, theme.text.primary),
        ChatItemKind::Reasoning => (theme.surfaces.sidebar, theme.text.secondary),
        ChatItemKind::Command => (theme.surfaces.hover, theme.text.primary),
        ChatItemKind::Activity => (theme.surfaces.card, theme.text.secondary),
        ChatItemKind::FileChange => (theme.surfaces.raised, theme.text.primary),
        ChatItemKind::Tool | ChatItemKind::Search | ChatItemKind::Image => {
            (theme.surfaces.hover, theme.text.primary)
        }
        ChatItemKind::Delegation => (theme.surfaces.sidebar, theme.text.primary),
        ChatItemKind::ApprovalReference => (theme.surfaces.raised, theme.text.warning),
        ChatItemKind::QuestionReference => (theme.surfaces.selected, theme.text.primary),
        ChatItemKind::SessionNotice => (theme.surfaces.card, theme.text.secondary),
        ChatItemKind::Plan => (theme.surfaces.selected, theme.text.primary),
        ChatItemKind::Warning => (theme.surfaces.raised, theme.text.warning),
        ChatItemKind::Error => (theme.surfaces.raised, theme.text.danger),
        ChatItemKind::Unknown(_) => (theme.surfaces.card, theme.text.secondary),
    };
    let label = item_label(&item.kind);
    let label_run_id = format!("{}/label", item.id);
    let is_activity = is_collapsible_activity(&item.kind);
    let summary = activity_summary(item, outcome, file_summary);
    let outcome_detail = outcome.and_then(|outcome| {
        let mut details = Vec::new();
        if let Some(duration_ms) = outcome.duration_ms {
            details.push(format!("Duration: {duration_ms} ms"));
        }
        (!details.is_empty()).then(|| details.join(" · "))
    });
    let exit_status = outcome.and_then(|outcome| outcome.exit_code).map(|code| {
        if code == 0 {
            (theme.text.success, format!("● {code}"))
        } else {
            (theme.text.danger, format!("● {code}"))
        }
    });
    let is_prose = matches!(item.kind, ChatItemKind::User | ChatItemKind::Agent);
    let padding = if item.kind == ChatItemKind::Agent {
        Insets::all(4.0)
    } else {
        Insets::all(10.0)
    };
    ui! {
        <Container fill_width padding={padding} gap={7.0}
            background={background}
            border={Border::new(theme.borders.ordinary, if is_prose { 0.0 } else { 1.0 })}
            radius={if is_prose { 0.0 } else { 10.0 }}>
            {[()].into_iter().filter(|_| item.kind != ChatItemKind::Agent).map(|_| ui! {
                <Text color={color} scale={0.9} selection_run_id={label_run_id.clone()}
                    selection_boundary={TextBoundary::Block}>{label}</Text>
            })}
            {[()].into_iter().filter(|_| is_activity).map(|_| ui! {
                <Column fill_width gap={6.0}>
                    <Row fill_width gap={6.0}>
                        {exit_status.iter().map(|(status_color, status)| ui! {
                            <Text color={*status_color}>{status.clone()}</Text>
                        })}
                        <Text color={color} wrap={true}>{summary.clone()}</Text>
                        {[()].into_iter().filter(|_| !item.text.is_empty()).map(|_| ui! {
                            <Button on_press={ChatMessage::ToggleActivityDetails(item.id.clone())}
                                background={theme.surfaces.hover} color={theme.text.secondary}>
                                {if expanded { "▾" } else { "▸" }}
                            </Button>
                        })}
                    </Row>
                    {outcome_detail.iter().map(|detail| ui! {
                        <Text color={theme.text.secondary}>{detail}</Text>
                    })}
                </Column>
            })}
            {[()].into_iter().filter(|_| !is_activity || expanded).map(|_| markdown_content_view(
                &document,
                MarkdownPalette {
                    foreground: color,
                    muted: theme.text.secondary,
                    accent: theme.accent.ordinary,
                    surface: theme.surfaces.sidebar,
                    border: theme.borders.ordinary,
                    code: theme.text.primary,
                },
                &format!("{}/body", item.id),
                |destination| ChatMessage::OpenMarkdownLink(destination.to_owned()),
            ))}
        </Container>
    }
}

fn empty_activity_document() -> std::sync::Arc<MarkdownDocument> {
    static EMPTY: std::sync::OnceLock<std::sync::Arc<MarkdownDocument>> =
        std::sync::OnceLock::new();
    EMPTY
        .get_or_init(|| std::sync::Arc::new(MarkdownDocument::parse("")))
        .clone()
}

fn activity_summary(
    item: &ChatItem,
    outcome: Option<&ActivityOutcome>,
    file_summary: Option<&str>,
) -> String {
    // Summaries use fields projected by known item adapters, not arbitrary prose in other cards.
    let state = outcome
        .and_then(|outcome| outcome.status.as_deref())
        .unwrap_or(if item.complete { "Finished" } else { "Running" });
    match &item.kind {
        ChatItemKind::Command => {
            let command = item.text.lines().next().unwrap_or_default();
            if command.starts_with("$ ") || command.starts_with('!') {
                format!("{state}: {}", command.chars().take(180).collect::<String>())
            } else {
                format!("{state} command")
            }
        }
        ChatItemKind::FileChange => file_summary
            .filter(|source| !source.is_empty())
            .map_or_else(
                || format!("{state} file change"),
                |source| format!("{state}: {source}"),
            ),
        ChatItemKind::Unknown(kind) => format!("Unsupported Codex activity: {kind}"),
        ChatItemKind::Tool
        | ChatItemKind::Search
        | ChatItemKind::Image
        | ChatItemKind::Delegation => item
            .text
            .lines()
            .next()
            .filter(|line| !line.is_empty())
            .map(|line| line.chars().take(180).collect())
            .unwrap_or_else(|| format!("{state} {}", item_label(&item.kind).to_lowercase())),
        _ => String::new(),
    }
}

// Keep every approval surface aligned with the decisions the controller can dispatch.
fn supported_approval_type(approval_type: &str) -> bool {
    matches!(
        approval_type,
        "item/fileChange/requestApproval" | "item/commandExecution/requestApproval"
    )
}

fn approval_choices(
    approval_type: &str,
    context: &nickel_codex::ApprovalContext,
) -> Vec<(String, CodexApprovalChoice)> {
    if approval_type == "item/commandExecution/requestApproval"
        && let Some(decisions) = &context.available_decisions
    {
        return decisions
            .iter()
            .map(|decision| {
                let label = match decision {
                    CommandDecision::Accept => "Approve",
                    CommandDecision::AcceptForSession => "Approve for session",
                    CommandDecision::AcceptWithExecpolicyAmendment { .. } => {
                        "Approve and save command rule"
                    }
                    CommandDecision::ApplyNetworkPolicyAmendment { .. } => "Apply network rule",
                    CommandDecision::Decline => "Decline",
                    CommandDecision::Cancel => "Cancel turn",
                };
                (label.into(), CodexApprovalChoice::Command(decision.clone()))
            })
            .collect();
    }
    if !supported_approval_type(approval_type) {
        return Vec::new();
    }
    // Older servers and file-change approvals do not supply a decision list.
    [
        ("Cancel".into(), CodexApprovalChoice::Cancel),
        ("Decline".into(), CodexApprovalChoice::Decline),
        ("Approve".into(), CodexApprovalChoice::Approve),
    ]
    .into_iter()
    .collect()
}

fn approval_choice_allowed(
    approval_type: &str,
    context: &nickel_codex::ApprovalContext,
    choice: &CodexApprovalChoice,
) -> bool {
    approval_choices(approval_type, context)
        .iter()
        .any(|(_, available)| available == choice)
}

fn codex_approval_presentation(
    approval_type: &str,
    summary: &str,
    context: &nickel_codex::ApprovalContext,
) -> ApprovalPresentation {
    let mut detail = match &context.command {
        Some(command) => format!("Command: {command}\nReason: {summary}"),
        None => summary.into(),
    };
    // Session-wide and rule-changing choices require their exact scope in the
    // owning card before the user can choose them. Shell notifications route
    // these decisions here rather than compressing them into inline actions.
    for decision in context.available_decisions.iter().flatten() {
        match decision {
            CommandDecision::AcceptForSession => {
                detail.push_str("\nApprove for session: future matching prompts in this session may run without asking again.");
            }
            CommandDecision::AcceptWithExecpolicyAmendment {
                execpolicy_amendment,
            } => {
                detail.push_str("\nProposed command rule: ");
                detail.push_str(&execpolicy_amendment.join(", "));
            }
            CommandDecision::ApplyNetworkPolicyAmendment {
                network_policy_amendment,
            } => {
                detail.push_str(&format!(
                    "\nProposed network rule: {:?} {}",
                    network_policy_amendment.action, network_policy_amendment.host
                ));
            }
            _ => {}
        }
    }
    let warning = match (context.asks_network_access, context.proposes_session_rule) {
        (true, true) => Some("Requests network access and proposes a rule for future commands; review the available decision before approving.".into()),
        (true, false) => Some("Requests network access.".into()),
        (false, true) => Some("Proposes a rule for future commands; review the available decision before approving.".into()),
        (false, false) => None,
    };
    ApprovalPresentation {
        requester: "Codex".into(),
        identity: RequesterIdentity::BackendReported,
        action: match approval_type {
            "item/commandExecution/requestApproval"
                if context.kind.as_deref() == Some("writeStdin") =>
            {
                "Send input to a running command".into()
            }
            "item/commandExecution/requestApproval" => "Run a command".into(),
            "item/fileChange/requestApproval" => "Change files".into(),
            _ => "Perform an operation".into(),
        },
        scope: context
            .grant_root
            .as_ref()
            .map(|root| format!("Requested write root: {root}"))
            .or_else(|| {
                context
                    .cwd
                    .as_ref()
                    .map(|cwd| format!("Working directory: {cwd}"))
            }),
        duration: None,
        warning,
        detail: Some(detail),
    }
}

#[derive(Clone, Copy)]
struct InteractionDeliveryStatus {
    response_unconfirmed: bool,
    notification_unavailable: bool,
}

#[component]
fn InteractionCard(
    interaction: &PendingInteraction,
    answer: &str,
    actionable: bool,
    submitting: bool,
    delivery: InteractionDeliveryStatus,
    theme: SemanticTheme,
    compact_approval_details: bool,
) -> impl View<ChatMessage> {
    match interaction {
        PendingInteraction::Approval {
            request_id,
            approval_type,
            summary,
            context,
        } => {
            let presentation = codex_approval_presentation(approval_type, summary, context);
            let choices = approval_choices(approval_type, context);
            let actionable = actionable && !choices.is_empty();
            let is_command_approval = approval_type == "item/commandExecution/requestApproval";
            let command = context
                .command
                .as_deref()
                .filter(|command| !command.trim().is_empty());
            ui! {
                <Container fill_width padding={Insets::all(8.0)} gap={4.0}
                    background={theme.surfaces.raised} border={Border::new(theme.text.warning, 1.0)} radius={8.0}>
                    {[()].into_iter().filter(|_| !is_command_approval).map(|_| ui! {
                        <Text color={theme.text.primary}>{presentation.title()}</Text>
                    })}
                    {[()].into_iter().filter(|_| is_command_approval).map(|_| ui! {
                        <Container id={id!(approval_command)} fill_width
                            max_height={if compact_approval_details { 48.0 } else { 96.0 }}
                            overflow_y={Overflow::Auto} padding={Insets::all(8.0)}
                            background={theme.surfaces.card} radius={4.0}>
                            <Text color={if command.is_some() { theme.text.primary } else { theme.text.warning }} wrap={true}>
                                {command.unwrap_or("Codex did not provide the command to review.")}
                            </Text>
                        </Container>
                    })}
                    {[()].into_iter().filter(|_| !is_command_approval).map(|_| ui! {
                        <Text color={theme.text.primary}>{presentation.action.clone()}</Text>
                    })}
                    {[()].into_iter().filter(|_| delivery.notification_unavailable).map(|_| ui! {
                        <Text color={theme.text.warning} wrap={true}>{"Shell notification unavailable; review and respond here."}</Text>
                    })}
                    // Scope and source detail stay inspectable without moving the
                    // authority-owned decisions out of a compact viewport.
                    <Column id={id!(approval_details)} fill_width max_height={if compact_approval_details { 18.0 } else { 52.0 }}
                        overflow_y={Overflow::Auto} gap={4.0}>
                    <Text color={theme.text.secondary} wrap={true}>{presentation.scope_line()}</Text>
                    {presentation.warning.iter().map(|warning| ui! {
                        <Text color={theme.text.warning} wrap={true}>{warning}</Text>
                    })}
                    {presentation.detail.iter().map(|detail| ui! {
                        <Text color={theme.text.secondary} wrap={true}>{detail}</Text>
                    })}
                    </Column>
                    {[()].into_iter().filter(|_| !actionable).map(|_| ui! {
                        <Text color={theme.text.secondary}>{if submitting {
                            if delivery.response_unconfirmed { "Response unconfirmed; reconnect before acting again." }
                            else { "Response submitted; awaiting resolution." }
                        } else {
                            "Request unavailable on this connection."
                        }}</Text>
                    })}
                    {[()].into_iter().filter(|_| actionable && choices.len() <= 3).map(|_| ui! { <Row gap={8.0}>
                        {choices.iter().map(|(label, choice)| ui! {
                            <Button on_press={ChatMessage::RespondApproval(request_id.clone(), approval_type.clone(), choice.clone())}
                                background={theme.surfaces.hover} color={theme.text.primary}>{label.clone()}</Button>
                        })}
                    </Row> })}
                    {[()].into_iter().filter(|_| actionable && choices.len() > 3).map(|_| ui! { <Column gap={4.0}>
                        {choices.iter().map(|(label, choice)| ui! {
                            <Button on_press={ChatMessage::RespondApproval(request_id.clone(), approval_type.clone(), choice.clone())}
                                background={theme.surfaces.hover} color={theme.text.primary}>{label.clone()}</Button>
                        })}
                    </Column> })}
                </Container>
            }
        }
        PendingInteraction::UserInput {
            request_id,
            question_ids,
            questions,
        } => ui! {
            <Container fill_width padding={Insets::all(8.0)} gap={4.0}
                background={theme.surfaces.raised} border={Border::new(theme.borders.ordinary, 1.0)} radius={8.0}>
                <Text color={theme.text.primary}>{"Codex requested input"}</Text>
                // Questions and option descriptions may be long, but the answer
                // field and authority-owned actions must remain outside this scroll.
                <Column id={id!(user_input_questions)} fill_width max_height={32.0}
                    overflow_y={Overflow::Auto} gap={4.0}>
                {questions.iter().map(|question| ui! {
                    <Column fill_width gap={4.0}>
                        <Text color={theme.text.primary}>{format!("{} — {}", question.header, question.question)}</Text>
                        {question.options.iter().map(|option| ui! {
                            <Text color={theme.text.secondary}>{format!("{}: {}", option.label, option.description)}</Text>
                        })}
                    </Column>
                })}
                <Text color={theme.text.secondary}>{"Enter one answer per line, in question order. Use an option label or your own answer."}</Text>
                </Column>
                <Container fill_width padding={Insets::all(8.0)} background={theme.surfaces.card} radius={6.0}>
                    {[()].into_iter().filter(|_| questions.iter().any(|question| question.is_secret)).map(|_| ui! {
                        {TextField::on_change_masked(answer, '●', interaction_answer_changed).color(theme.text.primary)}
                    })}
                    {[()].into_iter().filter(|_| !questions.iter().any(|question| question.is_secret)).map(|_| ui! {
                        <TextField value={answer} on_change={interaction_answer_changed} color={theme.text.primary} />
                    })}
                </Container>
                {[()].into_iter().filter(|_| !actionable).map(|_| ui! {
                    <Text color={theme.text.secondary}>{if submitting {
                        if delivery.response_unconfirmed { "Answer unconfirmed; reconnect before acting again." }
                        else { "Answer submitted; awaiting resolution." }
                    } else {
                        "Request unavailable on this connection."
                    }}</Text>
                })}
                {[()].into_iter().filter(|_| actionable).map(|_| ui! { <Row gap={8.0}>
                    <Button on_press={ChatMessage::DismissInput(request_id.clone())}
                        background={theme.surfaces.hover} color={theme.text.danger}>{"Cancel"}</Button>
                    <Button on_press={ChatMessage::SubmitInput(request_id.clone(), question_ids.clone())}
                        background={theme.surfaces.hover} color={theme.text.success}>{"Submit"}</Button>
                </Row> })}
            </Container>
        },
    }
}

fn remote_hosts_panel(
    settings: &CodexSettings,
    editor: Option<&RemoteHostEditor>,
    settings_error: Option<&str>,
    theme: SemanticTheme,
) -> AnyView<ChatMessage> {
    if let Some(editor) = editor {
        return AnyView::new(ui! {
            <Column fill_width grow={1.0} min_height={0.0} padding={Insets::all(24.0)} gap={12.0}
                background={theme.surfaces.window} overflow_y={Overflow::Auto}>
                <Row fill_width gap={8.0} align={Align::Center}>
                    <Column grow={1.0}>
                        <Text scale={1.6} color={theme.text.primary}>{if editor.original_id.is_some() { "Edit remote host" } else { "Add remote host" }}</Text>
                    </Column>
                    <Button on_press={ChatMessage::ManageRemoteHosts} background={theme.surfaces.hover} color={theme.text.primary}>{"Back"}</Button>
                </Row>
                <Text color={theme.text.secondary}>{"Nickel stores only the environment-variable name, never its secret value."}</Text>
                <Text color={theme.text.primary}>{"Identifier"}</Text>
                <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.card} radius={6.0}>
                    <TextField value={&editor.id} on_change={remote_host_id_changed} color={theme.text.primary} />
                </Container>
                <Text color={theme.text.primary}>{"Display name"}</Text>
                <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.card} radius={6.0}>
                    <TextField value={&editor.name} on_change={remote_host_name_changed} color={theme.text.primary} />
                </Container>
                <Text color={theme.text.primary}>{"WebSocket endpoint"}</Text>
                <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.card} radius={6.0}>
                    <TextField value={&editor.endpoint} on_change={remote_host_endpoint_changed} color={theme.text.primary} />
                </Container>
                <Text color={theme.text.primary}>{"Bearer-token environment variable (optional)"}</Text>
                <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.card} radius={6.0}>
                    <TextField value={&editor.token_env} on_change={remote_host_token_env_changed} color={theme.text.primary} />
                </Container>
                <Text color={theme.text.primary}>{"Default working directory on the remote host"}</Text>
                <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.card} radius={6.0}>
                    <TextField value={&editor.default_cwd} on_change={remote_host_cwd_changed} color={theme.text.primary} />
                </Container>
                {settings_error.map(|error| ui! {
                    <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.raised} radius={6.0}>
                        <Text color={theme.text.danger}>{error}</Text>
                    </Container>
                })}
                <Row gap={8.0}>
                    <Button on_press={ChatMessage::ManageRemoteHosts} background={theme.surfaces.card} color={theme.text.primary}>{"Cancel"}</Button>
                    <Button on_press={ChatMessage::SaveRemoteHost} background={theme.accent.ordinary} color={theme.accent.on_accent}>{"Save host"}</Button>
                </Row>
            </Column>
        });
    }

    AnyView::new(ui! {
        <Column fill_width grow={1.0} min_height={0.0} padding={Insets::all(24.0)} gap={12.0}
            background={theme.surfaces.window} overflow_y={Overflow::Auto}>
            <Row fill_width gap={8.0} align={Align::Center}>
                <Column grow={1.0}>
                    <Text scale={1.6} color={theme.text.primary}>{"Remote Codex hosts"}</Text>
                </Column>
                <Button on_press={ChatMessage::CloseRemoteHosts} background={theme.surfaces.hover} color={theme.text.primary}>{"Back"}</Button>
            </Row>
            <Text color={theme.text.secondary}>{"These are Nickel settings. Nickel does not read or modify Codex Desktop configuration."}</Text>
            <Container fill_width padding={Insets::all(12.0)} background={theme.surfaces.card} radius={8.0}>
                <Text color={theme.text.primary}>{"Local"}</Text>
                <Text color={theme.text.secondary}>{if settings.selected == "local" { "Selected" } else { "Uses the installed or bundled Codex CLI" }}</Text>
            </Container>
            {settings.hosts.iter().map(|host| ui! {
                <Container key={host.id.clone()} fill_width padding={Insets::all(12.0)} gap={6.0}
                    background={theme.surfaces.card} radius={8.0}>
                    <Text color={theme.text.primary}>{if settings.selected == host.id { format!("{} · Selected", host.name) } else { host.name.clone() }}</Text>
                    <Text color={theme.text.secondary}>{&host.endpoint}</Text>
                    <Text color={theme.text.secondary}>{format!("Remote cwd: {}", host.default_cwd)}</Text>
                    <Row gap={8.0}>
                        <Button on_press={ChatMessage::EditRemoteHost(host.id.clone())} background={theme.surfaces.sidebar} color={theme.text.primary}>{"Edit"}</Button>
                        <Button on_press={ChatMessage::RemoveRemoteHost(host.id.clone())} background={theme.surfaces.hover} color={theme.text.danger}>{"Remove"}</Button>
                    </Row>
                </Container>
            })}
            {settings_error.map(|error| ui! {
                <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.raised} radius={6.0}>
                    <Text color={theme.text.danger}>{error}</Text>
                </Container>
            })}
            <Row gap={8.0}>
                <Button on_press={ChatMessage::CloseRemoteHosts} background={theme.surfaces.card} color={theme.text.primary}>{"Done"}</Button>
                <Button on_press={ChatMessage::AddRemoteHost} background={theme.accent.ordinary} color={theme.accent.on_accent}>{"Add remote host"}</Button>
            </Row>
        </Column>
    })
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

fn connection_menu(settings: &CodexSettings) -> Menu<ChatMessage> {
    let mut items = vec![MenuItem::new(
        if settings.selected == "local" {
            "✓ Local"
        } else {
            "Local"
        },
        ChatMessage::SelectConnection("local".into()),
    )];
    items.extend(settings.hosts.iter().map(|host| {
        MenuItem::new(
            if settings.selected == host.id {
                format!("✓ {}", host.name)
            } else {
                host.name.clone()
            },
            ChatMessage::SelectConnection(host.id.clone()),
        )
    }));
    items.push(MenuItem::new(
        "Manage remote hosts…",
        ChatMessage::ManageRemoteHosts,
    ));
    Menu::new(ChatMessage::ToggleFileMenu, "Connection", items).id(id!(connection_menu))
}

fn bounded_menu_context(value: &str) -> String {
    // The complete title remains in the thread picker/window title; a source-
    // supplied title must not set the width of every app-chrome popover.
    let mut characters = value.chars();
    let prefix = characters.by_ref().take(48).collect::<String>();
    if characters.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn thread_menu(state: &ChatState, project_root: Option<&std::path::Path>) -> Menu<ChatMessage> {
    // The menu remains available before the backend's thread list arrives.
    // Identity labels are context, not authority or a fabricated history row.
    let project = project_root
        .and_then(std::path::Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("Current workspace");
    let thread = state.selected_thread.as_ref().map_or_else(
        || "New conversation".to_owned(),
        |id| {
            state
                .threads
                .iter()
                .find(|thread| thread.id == *id)
                .and_then(|thread| thread.title.clone())
                .unwrap_or_else(|| format!("Conversation {}", id.0))
        },
    );
    Menu::new(
        ChatMessage::ToggleFileMenu,
        "Thread",
        [
            MenuItem::disabled(format!("Project: {}", bounded_menu_context(project))),
            MenuItem::disabled(format!("Current: {}", bounded_menu_context(&thread))),
            MenuItem::new("New conversation", ChatMessage::NewChat),
            MenuItem::new("Browse conversations…", ChatMessage::ToggleResumePicker),
        ],
    )
    .id(id!(thread_menu))
}

#[derive(Clone, Copy)]
struct SlashCommandOption {
    command: &'static str,
    label: &'static str,
    available: bool,
}

const SLASH_COMMANDS: &[SlashCommandOption] = &[
    SlashCommandOption {
        command: "/model",
        label: "/model — choose model and reasoning",
        available: true,
    },
    SlashCommandOption {
        command: "/resume",
        label: "/resume — resume a project conversation",
        available: true,
    },
    SlashCommandOption {
        command: "/new",
        label: "/new — start a new conversation",
        available: true,
    },
    SlashCommandOption {
        command: "/clear",
        label: "/clear — clear into a new conversation",
        available: true,
    },
    SlashCommandOption {
        command: "/review",
        label: "/review — review uncommitted changes",
        available: true,
    },
    SlashCommandOption {
        command: "/compact",
        label: "/compact — compact the current conversation",
        available: true,
    },
    SlashCommandOption {
        command: "/plan",
        label: "/plan — toggle Plan mode",
        available: true,
    },
    SlashCommandOption {
        command: "/status",
        label: "/status — show Codex connection and run status",
        available: true,
    },
    SlashCommandOption {
        command: "/diff",
        label: "/diff — show working-tree changes",
        available: true,
    },
    SlashCommandOption {
        command: "/mention",
        label: "/mention — mention a project file",
        available: true,
    },
    SlashCommandOption {
        command: "/permissions",
        label: "/permissions — choose approval policy",
        available: true,
    },
    SlashCommandOption {
        command: "/feedback",
        label: "/feedback — send feedback to the Codex team",
        available: true,
    },
    SlashCommandOption {
        command: "/logout",
        label: "/logout — sign out of Codex",
        available: true,
    },
];

fn slash_search_results(draft: &str) -> Option<Vec<SlashCommandOption>> {
    let fragment = draft.trim_start().strip_prefix('/')?;
    if fragment.chars().any(char::is_whitespace) {
        return None;
    }
    let needle = fragment.to_ascii_lowercase();
    Some(
        SLASH_COMMANDS
            .iter()
            .copied()
            .filter(|option| option.command[1..].contains(&needle))
            .collect(),
    )
}

fn mention_fragment(draft: &str) -> Option<&str> {
    let token = draft
        .rsplit_once(char::is_whitespace)
        .map_or(draft, |(_, token)| token);
    token.strip_prefix('@')
}

fn replace_mention_fragment(draft: &str, path: &str) -> String {
    let token_start = draft
        .char_indices()
        .rev()
        .find_map(|(index, character)| {
            character
                .is_whitespace()
                .then_some(index + character.len_utf8())
        })
        .unwrap_or(0);
    let mut result = draft[..token_start].to_owned();
    result.push('@');
    result.push_str(path);
    result.push(' ');
    result
}

fn available_slash_commands(draft: &str) -> Vec<SlashCommandOption> {
    slash_search_results(draft)
        .unwrap_or_default()
        .into_iter()
        .filter(|option| option.available)
        .collect()
}

fn run_status_label(status: RunPresentationStatus) -> &'static str {
    match status {
        RunPresentationStatus::Connecting => "Connecting to Codex…",
        RunPresentationStatus::Recovering => "Reloading conversation…",
        RunPresentationStatus::RecoveryFailed => "Conversation reload failed — reconnect to retry",
        RunPresentationStatus::Unconfirmed => "Earlier work is unconfirmed — reconnect",
        RunPresentationStatus::AuthenticationRequired => "Sign in to Codex",
        RunPresentationStatus::Ready => "Ready",
        RunPresentationStatus::Starting => "Starting turn…",
        RunPresentationStatus::Working => "Codex is working…",
        RunPresentationStatus::Interrupting => "Interrupting…",
        RunPresentationStatus::AwaitingApproval => "Awaiting approval",
        RunPresentationStatus::AwaitingInput => "Awaiting your answer",
        RunPresentationStatus::Retrying => "Codex is retrying…",
        RunPresentationStatus::TurnFailed => "Turn failed — see diagnostics",
        RunPresentationStatus::Unavailable => "Codex is unavailable",
        RunPresentationStatus::Disconnected => "Codex is disconnected",
        RunPresentationStatus::Incompatible => "Codex is incompatible",
    }
}

fn diagnostic_guidance(state: &ChatState) -> (&'static str, &'static str) {
    match state.status {
        ConnectionStatus::Unavailable => (
            "Codex is not available.",
            "Check that the selected Codex backend is installed, then reconnect.",
        ),
        ConnectionStatus::Disconnected => (
            "The Codex connection was interrupted.",
            "Reconnect to reload the conversation; unsent work will not be sent automatically.",
        ),
        ConnectionStatus::Incompatible => (
            "The Codex backend is incompatible with this version of Nickel.",
            "Use a supported Codex version or backend, then reconnect.",
        ),
        ConnectionStatus::Loading => (
            "Nickel is connecting to Codex.",
            "Wait for the connection and conversation reload to finish.",
        ),
        ConnectionStatus::Ready if state.recovery_failed => (
            "The conversation could not be reloaded.",
            "Reconnect to retry. Nickel will not replay a prompt or approval.",
        ),
        ConnectionStatus::Ready if state.recovery_pending => (
            "The conversation is being reloaded.",
            "Wait before sending a prompt or responding to a request.",
        ),
        ConnectionStatus::Ready if state.unconfirmed_work => (
            "An earlier turn or request outcome is unconfirmed.",
            "Reconnect to reload the conversation before acting again.",
        ),
        ConnectionStatus::Ready if !state.account.authenticated => (
            "Codex needs sign-in.",
            "Sign in to Codex, then retry the action.",
        ),
        ConnectionStatus::Ready if state.last_turn_error.is_some() => (
            "The last turn failed, but the connection is healthy.",
            "Review the turn details before deciding whether to send a new prompt.",
        ),
        ConnectionStatus::Ready => ("Codex is connected.", "No recovery action is needed."),
    }
}

fn safe_diagnostic_summary(state: &ChatState, mode: &BackendMode) -> String {
    let backend = match mode {
        BackendMode::Live { .. } => match state.backend_source.as_ref() {
            Some(nickel_codex::CandidateSource::Installed) => "Installed Codex",
            Some(nickel_codex::CandidateSource::Bundled) => "Bundled Codex",
            Some(nickel_codex::CandidateSource::Explicit) => "Explicit local Codex",
            None => "Local Codex",
        },
        BackendMode::Remote { .. } => "Remote Codex",
        BackendMode::Replay { .. } => "Replay fixture",
    };
    let connection = match state.status {
        ConnectionStatus::Loading => "Connecting",
        ConnectionStatus::Ready => "Connected",
        ConnectionStatus::Unavailable => "Unavailable",
        ConnectionStatus::Disconnected => "Disconnected",
        ConnectionStatus::Incompatible => "Incompatible",
    };
    let (cause, next_action) = diagnostic_guidance(state);
    // Fixed labels only: backend diagnostics, IDs, paths, prompts, credentials,
    // approval payloads, and pairing secrets are deliberately never copied.
    format!(
        "Nickel Codex diagnostics\nBackend: {backend}\nConnection: {connection}\nStatus: {}\nCause: {cause}\nNext action: {next_action}",
        run_status_label(state.run_presentation_status())
    )
}

fn diagnostics_panel(
    state: &ChatState,
    details_open: bool,
    copy_result: Option<&Result<(), String>>,
    theme: SemanticTheme,
) -> impl View<ChatMessage> {
    let can_reconnect = state.recovery_failed
        || state.unconfirmed_work
        || matches!(
            state.status,
            ConnectionStatus::Unavailable
                | ConnectionStatus::Disconnected
                | ConnectionStatus::Incompatible
        );
    let (cause, next_action) = diagnostic_guidance(state);
    ui! {
        <Column id={id!(codex_diagnostics)} fill_width gap={10.0} padding={Insets::all(12.0)}
            background={theme.surfaces.card} radius={8.0}>
            <Row fill_width gap={8.0} align_items={Align::Center}>
                <Text scale={1.25} color={theme.text.primary} grow={1.0}>{"Codex diagnostics"}</Text>
                <Button on_press={ChatMessage::ToggleDiagnostics}
                    background={theme.surfaces.hover} color={theme.text.primary}>{"Back"}</Button>
            </Row>
            <Text color={theme.text.primary}>{run_status_label(state.run_presentation_status())}</Text>
            {state.run_secondary_status().map(|secondary| ui! {
                <Text color={theme.text.secondary} wrap={true}>{secondary}</Text>
            })}
            <Text color={theme.text.primary} wrap={true}>{cause}</Text>
            <Text color={theme.text.secondary} wrap={true}>{next_action}</Text>
            <Text color={theme.text.primary}>{"Account usage"}</Text>
            {[()].into_iter().filter(|_| state.rate_limits_pending).map(|_| ui! {
                <Text color={theme.text.secondary}>{"Loading rate limits…"}</Text>
            })}
            {state.rate_limits_error.as_ref().map(|error| ui! {
                <Text color={theme.text.warning} wrap={true}>{format!("Rate limits unavailable: {error}")}</Text>
            })}
            {state.rate_limits.as_ref().map(|limits| ui! {
                <Column fill_width gap={4.0}>
                    <Text color={theme.text.secondary}>{match limits.ordinary_usage_allowed {
                        Some(true) => "Ordinary usage allowed",
                        Some(false) => "Ordinary usage blocked",
                        None => "Ordinary usage availability unknown",
                    }}</Text>
                    {limits.buckets.iter().map(|bucket| ui! {
                        <Text color={theme.text.secondary} wrap={true}>{format!(
                            "{}: primary {} · secondary {}",
                            bucket.name,
                            bucket.primary_used_percent.map_or("unknown".into(), |used| format!("{used}% used")),
                            bucket.secondary_used_percent.map_or("unknown".into(), |used| format!("{used}% used")),
                        )}</Text>
                    })}
                </Column>
            })}
            <Row fill_width gap={8.0}>
                <Button on_press={ChatMessage::ToggleDiagnosticDetails}
                    background={theme.surfaces.hover} color={theme.text.primary}>{if details_open { "Hide technical details" } else { "Show technical details" }}</Button>
                <Button on_press={ChatMessage::CopyDiagnosticSummary}
                    background={theme.surfaces.hover} color={theme.text.primary}>{"Copy safe summary"}</Button>
                {[()].into_iter().filter(|_| matches!(copy_result, Some(Ok(())))).map(|_| ui! {
                    <Text id={id!(diagnostic_copy_confirmation)} color={theme.text.success}
                        accessibility_label={"Safe diagnostic summary copied to clipboard"}>{"Copied"}</Text>
                })}
                {[()].into_iter().filter(|_| matches!(copy_result, Some(Err(_)))).map(|_| ui! {
                    <Text id={id!(diagnostic_copy_failure)} color={theme.text.danger}
                        accessibility_label={"Safe diagnostic summary could not be copied"}>{"Copy failed"}</Text>
                })}
            </Row>
            {[()].into_iter().filter(|_| details_open).map(|_| ui! {
                <Column fill_width gap={6.0}>
                    <Text color={theme.text.secondary} wrap={true}>{format!("Backend: {}", state.provenance)}</Text>
                    {state.backend_source.as_ref().map(|source| ui! {
                        <Text color={theme.text.secondary}>{format!("Source: {source:?}")}</Text>
                    })}
                    {state.fallback_reason.as_ref().map(|reason| ui! {
                        <Text color={theme.text.secondary} wrap={true}>{format!("Installed Codex was rejected: {reason}")}</Text>
                    })}
                    {state.selected_thread.as_ref().map(|thread| ui! {
                        <Text color={theme.text.secondary} wrap={true}>{format!("Thread: {}", thread.0)}</Text>
                    })}
                    {state.active_turn.as_ref().map(|turn| ui! {
                        <Text color={theme.text.secondary} wrap={true}>{format!("Turn: {}", turn.0)}</Text>
                    })}
                    {state.diagnostics.iter().rev().take(5).map(|message| ui! {
                        <Text color={theme.text.secondary} wrap={true}>{message}</Text>
                    })}
                </Column>
            })}
            {[()].into_iter().filter(|_| can_reconnect).map(|_| ui! {
                <Button on_press={ChatMessage::Reconnect}
                    background={theme.accent.ordinary} color={theme.accent.on_accent}>{"Reconnect"}</Button>
            })}
        </Column>
    }
}

fn project_menu_view(
    state: &ChatState,
    settings_error: Option<&str>,
    theme: SemanticTheme,
) -> impl View<ChatMessage> {
    let controller_focus = theme.borders.controller_focus;
    let project_query = state.draft.trim().to_lowercase();
    let matching_projects = state
        .projects
        .iter()
        .filter(|project| {
            !project.roots.is_empty()
                && (project_query.is_empty()
                    || project.name.to_lowercase().contains(&project_query)
                    || project.id.to_lowercase().contains(&project_query)
                    || project.roots.iter().any(|root| {
                        root.to_string_lossy()
                            .to_lowercase()
                            .contains(&project_query)
                    }))
        })
        .collect::<Vec<_>>();
    let status = match state.status {
        ConnectionStatus::Loading => "Loading projects…",
        ConnectionStatus::Ready if state.projects.is_empty() => "No projects available",
        ConnectionStatus::Ready if matching_projects.is_empty() => "No matching projects",
        ConnectionStatus::Ready => "Choose a project",
        ConnectionStatus::Unavailable => "Codex is not installed",
        ConnectionStatus::Disconnected => "Codex is disconnected",
        ConnectionStatus::Incompatible => "Codex is incompatible",
    };
    ui! {
        <Column fill_width fill_height padding={Insets::all(18.0)} gap={12.0}
            background={theme.surfaces.window} border={Border::new(theme.borders.ordinary, 1.0)} radius={14.0}>
            <Container fill_width shrink={0.0} padding={Insets::all(14.0)} gap={6.0}
                background={theme.surfaces.sidebar} border={Border::new(theme.borders.ordinary, 1.0)} radius={10.0}>
                <Row fill_width shrink={0.0} gap={8.0}>
                    <Text scale={1.5} color={theme.text.primary} grow={1.0}>{"Codex projects"}</Text>
                    <Button on_press={ChatMessage::Refresh} background={theme.surfaces.raised} color={theme.text.primary}
                        controller_focus_background_tint={controller_focus} radius={7.0}>{"Refresh"}</Button>
                </Row>
                <Text color={theme.text.secondary} shrink={0.0}>{status}</Text>
            </Container>
            <Container id={id!(project_search_container)} accessibility_label={"Search projects"}
                semantic_role={SemanticRole::Group} fill_width shrink={0.0}
                padding={Insets::symmetric(12.0, 10.0)} background={theme.surfaces.card}
                border={Border::new(theme.borders.ordinary, 1.0)} radius={9.0}>
                <TextField id={id!(project_search)} value={&state.draft}
                    on_change={draft_changed} color={theme.text.primary} />
            </Container>
            {state.diagnostics.back().map(|diagnostic| ui! {
                <Container fill_width padding={Insets::all(8.0)} background={theme.surfaces.raised} radius={6.0}>
                    <Text color={theme.text.primary} max_lines={3}>{diagnostic}</Text>
                </Container>
            })}
            {settings_error.map(|error| ui! {
                <Container fill_width padding={Insets::all(8.0)} background={theme.surfaces.raised} radius={6.0}>
                    <Text color={theme.text.primary}>{error}</Text>
                </Container>
            })}
            <Column id={id!(project_menu_list)} grow={1.0} min_height={0.0}
                overflow_y={Overflow::Auto} gap={8.0}>
                {matching_projects.into_iter().map(|project| {
                    let root = project.roots[0].clone();
                    ui! {
                        <Button key={project.id.clone()} height={48.0}
                            on_press={ChatMessage::NewChatIn(root, project.id.clone())}
                            background={theme.surfaces.raised} color={theme.text.primary} label_align={TextAlign::Start}
                            controller_focus_background_tint={controller_focus}
                            focus_background_tint={theme.surfaces.hover}
                            border={Border::new(theme.borders.ordinary, 1.0)} radius={9.0}
                            padding={Insets::symmetric(14.0, 10.0)} fill_width>{&project.name}</Button>
                    }
                })}
            </Column>
        </Column>
    }
}

const RESUME_PREVIEW_LIMIT: usize = 180;

pub(crate) fn resume_preview(thread: &nickel_codex::Thread) -> Option<String> {
    let text = thread
        .turns
        .iter()
        .rev()
        .flat_map(|turn| turn.items.iter().rev())
        .find(|item| {
            matches!(
                item.item_type.as_str(),
                "userMessage" | "agentMessage" | "user_message" | "agent_message"
            ) && !item.text.trim().is_empty()
        })?
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = text.chars();
    let preview = chars
        .by_ref()
        .take(RESUME_PREVIEW_LIMIT)
        .collect::<String>();
    Some(if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    })
}

fn resume_recency(last_used_at: Option<i64>, now: i64) -> String {
    let Some(timestamp) = last_used_at else {
        return "Last used unknown".into();
    };
    let age = now.saturating_sub(timestamp).max(0);
    match age {
        0..=59 => "Used just now".into(),
        60..=3_599 => format!("Used {} min ago", age / 60),
        3_600..=86_399 => format!("Used {} hr ago", age / 3_600),
        _ => format!("Used {} days ago", age / 86_400),
    }
}

fn thread_belongs_to_project(
    thread: &nickel_codex::Thread,
    runtime: Option<&nickel_codex::ThreadRuntime>,
    project_root: Option<&std::path::Path>,
    project_id: Option<&str>,
) -> bool {
    runtime
        .and_then(|runtime| runtime.project_id.as_deref())
        .map_or_else(
            || {
                project_root.is_none_or(|root| {
                    thread
                        .cwd
                        .as_deref()
                        .is_some_and(|cwd| cwd.starts_with(root))
                })
            },
            |thread_project| project_id == Some(thread_project),
        )
}

fn resume_picker(
    state: &ChatState,
    project_root: Option<&std::path::Path>,
    project_id: Option<&str>,
    loading: bool,
    pending: Option<&nickel_codex::ThreadId>,
    shell_host: bool,
    theme: SemanticTheme,
) -> AnyView<ChatMessage> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64);
    let threads = state
        .threads
        .iter()
        .filter_map(|thread| {
            let runtime = state.thread_runtime.get(&thread.id);
            thread_belongs_to_project(thread, runtime, project_root, project_id)
                .then_some((thread, runtime))
        })
        .collect::<Vec<_>>();
    let has_threads = !threads.is_empty();
    AnyView::new(ui! {
        <Column id={id!(resume_picker)} fill_width grow={1.0} min_height={0.0}
            padding={Insets::all(8.0)} gap={6.0} background={theme.surfaces.card}
            border={Border::new(theme.borders.ordinary, 1.0)} radius={8.0}>
            <Row fill_width gap={8.0}>
                <Text color={theme.text.primary} grow={1.0}>{"Resume conversation"}</Text>
                <Button on_press={ChatMessage::NewChat} background={theme.surfaces.hover} color={theme.text.primary}>{"New"}</Button>
                <Button on_press={ChatMessage::CloseResumePicker} background={theme.surfaces.hover} color={theme.text.primary}>{"Back"}</Button>
            </Row>
            {[()].into_iter().filter(|_| loading).map(|_| ui! {
                <Text color={theme.text.secondary}>{"Loading recent conversations…"}</Text>
            })}
            {[()].into_iter().filter(|_| !loading && state.thread_error.is_some()).map(|_| ui! {
                <Row fill_width gap={8.0}>
                    <Text color={theme.text.primary} grow={1.0}>{format!("Could not load conversations: {}", state.thread_error.as_deref().unwrap_or("Unknown error"))}</Text>
                    <Button on_press={ChatMessage::RefreshResumePicker} background={theme.surfaces.hover} color={theme.text.primary}>{"Retry"}</Button>
                </Row>
            })}
            {[()].into_iter().filter(|_| !loading && state.thread_error.is_none() && !has_threads).map(|_| ui! {
                <Text color={theme.text.secondary}>{if state.thread_next_cursor.is_some() {
                    "No conversations for this project on the loaded page; older pages remain."
                } else {
                    "No conversations yet for this project."
                }}</Text>
            })}
            <Column id={id!(resume_conversation_list)} fill_width grow={1.0} min_height={0.0}
                overflow_y={Overflow::Auto} gap={4.0}>
                {threads.into_iter().filter(|_| !loading && state.thread_error.is_none()).map(|(thread, runtime)| {
                    let title = thread.title.clone().filter(|title| !title.trim().is_empty())
                        .unwrap_or_else(|| "Untitled conversation".into());
                    let identity = thread.id.0.clone();
                    let preview = resume_preview(thread).unwrap_or_else(|| "No message preview available".into());
                    let label = format!("{title} — {} — {identity}\n{preview}", resume_recency(thread.last_used_at, now));
                    let active = runtime.is_some_and(|runtime| runtime.status == nickel_codex::ThreadRuntimeStatus::Active);
                    let resumable = runtime.is_some_and(|runtime| matches!(runtime.status, nickel_codex::ThreadRuntimeStatus::Idle | nickel_codex::ThreadRuntimeStatus::NotLoaded));
                    let waiting = pending == Some(&thread.id);
                    if (resumable || active && shell_host) && pending.is_none() {
                        AnyView::new(ui! {
                            <Column key={thread.id.0.clone()} fill_width gap={2.0}>
                                <Button label_align={TextAlign::Start}
                                    on_press={ChatMessage::SelectThread(thread.id.clone())}
                                    background={theme.surfaces.hover} color={theme.text.primary}
                                    accessibility_label={title.clone()} fill_width>{label}</Button>
                                {[()].into_iter().filter(|_| active).map(|_| ui! {
                                    <Text color={theme.text.secondary}>{"Already active"}</Text>
                                })}
                            </Column>
                        })
                    } else {
                        let status = if waiting { "Resuming…" } else if active { "Already active" } else { "Unavailable" };
                        AnyView::new(ui! {
                            <Column key={thread.id.0.clone()} fill_width padding={Insets::all(10.0)} background={theme.surfaces.sidebar} radius={6.0}>
                                <Text color={theme.text.secondary}>{label}</Text>
                                <Text color={theme.text.secondary}>{status}</Text>
                            </Column>
                        })
                    }
                })}
            </Column>
            {[()].into_iter().filter(|_| state.thread_windowed).map(|_| ui! {
                <Text color={theme.text.secondary}>{"Showing a bounded older-conversation window. Refresh to return to the newest page."}</Text>
            })}
            {state.thread_page_error.as_ref().map(|error| ui! {
                <Text color={theme.text.danger}>{format!("Could not load older conversations: {error}")}</Text>
            })}
            {[()].into_iter().filter(|_| !loading && state.thread_next_cursor.is_some() && !state.thread_paging).map(|_| ui! {
                <Button on_press={ChatMessage::LoadMoreThreads} background={theme.surfaces.hover}
                    color={theme.text.primary}>{"Older conversations"}</Button>
            })}
            {[()].into_iter().filter(|_| state.thread_paging).map(|_| ui! {
                <Text color={theme.text.secondary}>{"Loading older conversations…"}</Text>
            })}
        </Column>
    })
}

fn run_settings_controls(
    state: &ChatState,
    model_picker_generation: u64,
    reasoning_picker_generation: u64,
    approval_picker_generation: u64,
    sandbox_picker_generation: u64,
    compact: bool,
    theme: SemanticTheme,
) -> AnyView<ChatMessage> {
    let mut controls: Vec<AnyView<ChatMessage>> = Vec::with_capacity(4);
    controls.push(AnyView::new(
        Dropdown::new(
            ChatMessage::ToggleModelPicker,
            state
                .models
                .iter()
                .find(|model| Some(model.id.as_str()) == state.selected_model.as_deref())
                .map(|model| model.display_name.clone())
                .unwrap_or_else(|| "Model".into()),
            state.models.iter().map(|model| {
                (
                    model.display_name.clone(),
                    ChatMessage::SelectModel(model.id.clone()),
                )
            }),
        )
        .id(id!(model_selector))
        .accessibility_label("Model selector")
        .semantic_role(SemanticRole::Button)
        .overlay(true)
        .open_generation(model_picker_generation)
        .colors(
            theme.surfaces.card,
            theme.surfaces.sidebar,
            theme.text.primary,
        ),
    ));
    if let Some(model) = state
        .models
        .iter()
        .find(|model| Some(model.id.as_str()) == state.selected_model.as_deref())
        .filter(|model| !model.supported_reasoning_efforts.is_empty())
    {
        controls.push(AnyView::new(
            Dropdown::new(
                ChatMessage::ToggleReasoningPicker,
                state
                    .selected_reasoning_effort
                    .clone()
                    .unwrap_or_else(|| "Effort".into()),
                model.supported_reasoning_efforts.iter().map(|option| {
                    (
                        format!("{} — {}", option.reasoning_effort, option.description),
                        ChatMessage::SelectReasoningEffort(option.reasoning_effort.clone()),
                    )
                }),
            )
            .id(id!(reasoning_effort_selector))
            .accessibility_label("Reasoning effort selector")
            .semantic_role(SemanticRole::Button)
            .overlay(true)
            .open_generation(reasoning_picker_generation)
            .colors(
                theme.surfaces.card,
                theme.surfaces.sidebar,
                theme.text.primary,
            ),
        ));
    }
    controls.push(AnyView::new(
        Dropdown::new(
            ChatMessage::ToggleApprovalPicker,
            approval_policy_label(state.selected_approval_policy),
            APPROVAL_POLICIES.into_iter().map(|policy| {
                (
                    format!(
                        "{} — {}",
                        approval_policy_label(policy),
                        approval_policy_description(policy)
                    ),
                    ChatMessage::SelectApprovalPolicy(policy),
                )
            }),
        )
        .id(id!(approval_policy_selector))
        .overlay(true)
        .open_generation(approval_picker_generation)
        .colors(
            theme.surfaces.card,
            theme.surfaces.sidebar,
            theme.text.primary,
        )
        .accessibility_label("Approval policy selector")
        .accessibility_description(format!(
            "Effective: {}. This does not change sandbox or filesystem access.",
            approval_policy_label(state.effective_approval_policy),
        ))
        .semantic_role(SemanticRole::Button),
    ));
    controls.push(AnyView::new(
        Dropdown::new(
            ChatMessage::ToggleSandboxPicker,
            sandbox_policy_label(state.selected_sandbox_policy),
            [
                SandboxPolicy::ReadOnly,
                SandboxPolicy::WorkspaceWrite,
                SandboxPolicy::DangerFullAccess,
            ]
            .into_iter()
            .map(|policy| {
                (
                    sandbox_policy_label(Some(policy)).to_owned(),
                    ChatMessage::SelectSandboxPolicy(Some(policy)),
                )
            }),
        )
        .id(id!(sandbox_policy_selector))
        .overlay(true)
        .open_generation(sandbox_picker_generation)
        .colors(
            theme.surfaces.card,
            theme.surfaces.sidebar,
            theme.text.primary,
        )
        .accessibility_label("Sandbox access selector")
        .accessibility_description(format!(
            "Effective: {}. YOLO is unsandboxed and sets approval policy to Never ask.",
            sandbox_policy_label(state.effective_sandbox_policy),
        ))
        .semantic_role(SemanticRole::Button),
    ));
    if compact {
        AnyView::new(Column::new().fill_width().gap(4.0).children(controls))
    } else {
        AnyView::new(Row::new().gap(8.0).children(controls))
    }
}

fn close_icon_pixels(color: nickel_ui::Color) -> std::sync::Arc<image::RgbaImage> {
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

fn feedback_panel(
    classification: Option<&str>,
    reason: &str,
    pending: bool,
    theme: SemanticTheme,
) -> impl View<ChatMessage> {
    const CATEGORIES: [(&str, &str, &str); 5] = [
        ("bug", "Bug", "Crash, error, hang, or broken behavior"),
        (
            "bad_result",
            "Bad result",
            "Incorrect, incomplete, or unhelpful output",
        ),
        (
            "good_result",
            "Good result",
            "Helpful or especially high-quality output",
        ),
        (
            "safety_check",
            "Safety check",
            "Benign work blocked by a safety check",
        ),
        (
            "other",
            "Other",
            "Performance, feature, or general UX feedback",
        ),
    ];
    Column::new()
        .fill_width()
        .gap(10.0)
        .children([
            AnyView::new(ui! {
                <Row fill_width align_items={Align::Center} gap={8.0}>
                    <Text scale={1.2} color={theme.text.primary} grow={1.0}>{"Send Codex feedback"}</Text>
                    {Button::new(ChatMessage::CloseFeedback, "Back")
                        .background(theme.surfaces.hover)
                        .color(theme.text.primary)
                        .enabled(!pending)}
                </Row>
            }),
            AnyView::new(ui! {
                <Column fill_width gap={6.0}>
                    <Text color={theme.text.secondary}>{"Choose a category"}</Text>
                    {CATEGORIES.into_iter().map(|(value, label, description)| ui! {
                        <Button on_press={ChatMessage::SelectFeedbackCategory(value.into())}
                            enabled={!pending}
                            background={if classification == Some(value) { theme.accent.soft } else { theme.surfaces.hover }}
                            color={theme.text.primary} fill_width
                            accessibility_label={format!("{label}: {description}")}>
                            {format!("{label} — {description}")}
                        </Button>
                    })}
                </Column>
            }),
            AnyView::new(ui! {
                <Column fill_width gap={6.0}>
                    <Text color={theme.text.secondary}>{"Optional note"}</Text>
                    <Container fill_width min_height={80.0} padding={Insets::all(10.0)}
                        background={theme.surfaces.card}
                        border={Border::new(theme.borders.ordinary, 1.0)} radius={6.0}>
                        <TextField id={id!(feedback_reason)} value={reason}
                            on_change={ChatMessage::FeedbackReasonChanged}
                            wrap={true} color={theme.text.primary} />
                    </Container>
                </Column>
            }),
            AnyView::new(ui! {
                <Text color={theme.text.secondary} wrap={true}>
                    {"Sending diagnostics may include Codex logs for this session. Choose the no-diagnostics action if you only want to send the category and note."}
                </Text>
            }),
            AnyView::new(ui! {
                <Row fill_width gap={8.0}>
                    {Button::new(ChatMessage::SubmitFeedback(false), if pending { "Sending…" } else { "Send without diagnostics" })
                        .background(theme.surfaces.hover)
                        .color(theme.text.primary)
                        .enabled(classification.is_some() && !pending)}
                    {Button::new(ChatMessage::SubmitFeedback(true), "Send with diagnostics")
                        .background(theme.accent.ordinary)
                        .color(theme.accent.on_accent)
                        .enabled(classification.is_some() && !pending)}
                </Row>
            }),
        ])
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

fn login_panel(state: &ChatState, theme: SemanticTheme) -> AnyView<ChatMessage> {
    let challenge = state.login_challenge.as_ref();
    let (login_id, url, user_code) = match challenge {
        Some(nickel_codex::LoginChallenge::Browser { login_id, auth_url }) => {
            (Some(login_id), Some(auth_url), None)
        }
        Some(nickel_codex::LoginChallenge::DeviceCode {
            login_id,
            user_code,
            verification_url,
        }) => (Some(login_id), Some(verification_url), Some(user_code)),
        None => (None, None, None),
    };
    AnyView::new(ui! {
        <Column grow={1.0} fill_width align_self={Align::Center} max_width={560.0}
            padding={Insets::all(28.0)} gap={14.0}>
            <Text scale={1.8} color={theme.text.primary}>{"Sign in to Codex"}</Text>
            <Text color={theme.text.secondary}>{"Authenticate this Codex profile. QR codes are generated locally by Nickel."}</Text>
            {state.login_qr.as_ref().map(|qr| ui! {
                <Image asset_id={65001} image={qr.clone()} generation={1}
                    width={264.0} height={264.0} fit={ImageFit::Contain}
                    accessibility_label={"Login QR code"} />
            })}
            {user_code.map(|code| ui! {
                <Container fill_width padding={Insets::all(14.0)} background={theme.surfaces.card}
                    border={Border::new(theme.borders.ordinary, 1.0)} radius={8.0}>
                    <Text scale={1.4} color={theme.text.primary} selection_run_id={"login/user-code"}
                        selection_boundary={TextBoundary::Block}>{code}</Text>
                </Container>
            })}
            {url.map(|url| ui! {
                <Text color={theme.text.secondary} max_lines={3} selection_run_id={"login/url"}
                    selection_boundary={TextBoundary::Block}>{url}</Text>
            })}
            <Row gap={8.0}>
                {if let (Some(id), Some(url)) = (login_id, url) {
                    AnyView::new(ui! {
                        <Row gap={8.0}>
                            <Button on_press={ChatMessage::OpenLoginUrl(url.clone())}
                                background={theme.accent.ordinary} color={theme.accent.on_accent}>{"Open"}</Button>
                            <Button on_press={ChatMessage::CopyLoginText(user_code.unwrap_or(url).clone())}
                                background={theme.surfaces.card} color={theme.text.primary}>
                                {if user_code.is_some() { "Copy code" } else { "Copy link" }}
                            </Button>
                            <Button on_press={ChatMessage::CancelLogin(id.clone())}
                                background={theme.surfaces.hover} color={theme.text.danger}>{"Cancel"}</Button>
                        </Row>
                    })
                } else {
                    AnyView::new(ui! {
                        <Row gap={8.0}>
                            <Button on_press={ChatMessage::StartLogin(nickel_codex::LoginMethod::Browser)}
                                background={theme.accent.ordinary} color={theme.accent.on_accent}>{"Sign in with browser"}</Button>
                            <Button on_press={ChatMessage::StartLogin(nickel_codex::LoginMethod::DeviceCode)}
                                background={theme.surfaces.card} color={theme.text.primary}>{"Use phone or code"}</Button>
                        </Row>
                    })
                }}
            </Row>
            {if state.login_pending {
                AnyView::new(ui! { <Text color={theme.text.secondary}>{"Waiting for Codex…"}</Text> })
            } else if state.login_status == crate::model::LoginPresentationState::Waiting {
                AnyView::new(ui! { <Text color={theme.text.secondary}>{"Waiting for sign in…"}</Text> })
            } else if state.login_status == crate::model::LoginPresentationState::Cancelled {
                AnyView::new(ui! { <Text color={theme.text.secondary}>{"Sign in cancelled"}</Text> })
            } else if state.login_status == crate::model::LoginPresentationState::Failed {
                AnyView::new(ui! { <Text color={theme.text.danger}>{"Sign in failed — try again"}</Text> })
            } else {
                AnyView::new(Spacer::vertical(0.0))
            }}
        </Column>
    })
}

fn codex_phone_access_panel(state: &ChatState, theme: SemanticTheme) -> AnyView<ChatMessage> {
    let status = state.remote_control_status.as_ref();
    let status_label = match status.map(|status| status.status) {
        Some(nickel_codex::RemoteControlConnectionStatus::Disabled) => "Disabled",
        Some(nickel_codex::RemoteControlConnectionStatus::Connecting) => "Connecting…",
        Some(nickel_codex::RemoteControlConnectionStatus::Connected) => "Connected",
        Some(nickel_codex::RemoteControlConnectionStatus::Errored) => "Error",
        None => "Checking…",
    };
    let connected = status.is_some_and(|status| {
        status.status == nickel_codex::RemoteControlConnectionStatus::Connected
    });
    let environment_id = status.and_then(|status| status.environment_id.as_deref());
    AnyView::new(ui! {
        <Column grow={1.0} fill_width align_self={Align::Center} max_width={520.0}
            padding={Insets::all(20.0)} gap={10.0}>
            <Row fill_width gap={8.0} align={Align::Center}>
                <Column grow={1.0}>
                    <Text scale={1.5} color={theme.text.primary}>{"Phone access"}</Text>
                </Column>
                <Button on_press={ChatMessage::CloseRemoteControl}
                    background={theme.surfaces.hover} color={theme.text.primary}>{"Back"}</Button>
            </Row>
            <Text color={theme.text.secondary} wrap={true}>
                {"Paired phones can use Codex remotely. Nickel desktop permissions remain separate."}
            </Text>
            <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.card} radius={8.0}>
                <Row fill_width gap={10.0} align={Align::Center}>
                    <Column grow={1.0}>
                        <Text color={theme.text.primary}>{format!("Codex connection: {status_label}")}</Text>
                    </Column>
                    {if connected {
                        AnyView::new(ui! { <Button on_press={ChatMessage::DisableRemoteControl}
                            background={theme.surfaces.hover} color={theme.text.danger}>{"Disable"}</Button> })
                    } else {
                        AnyView::new(ui! { <Button on_press={ChatMessage::EnableRemoteControl}
                            background={theme.accent.ordinary} color={theme.accent.on_accent}>{"Enable"}</Button> })
                    }}
                </Row>
            </Container>
            {state.remote_pairing_qr.as_ref().map(|qr| ui! {
                <Image asset_id={65002} image={qr.clone()} generation={1}
                    width={264.0} height={264.0} fit={ImageFit::Contain}
                    accessibility_label={"Codex phone pairing QR code"} />
            })}
            {state.remote_pairing.as_ref().and_then(|pairing| pairing.manual_pairing_code.as_ref()).map(|code| ui! {
                <Column fill_width gap={6.0}>
                    <Text color={theme.text.secondary}>{"Manual pairing code"}</Text>
                    <Container fill_width padding={Insets::all(14.0)} background={theme.surfaces.card}
                        border={Border::new(theme.borders.ordinary, 1.0)} radius={8.0}>
                        <Text scale={1.4} color={theme.text.primary} selection_run_id={"phone/manual-code"}
                            selection_boundary={TextBoundary::Block}>{code}</Text>
                    </Container>
                    <Text color={theme.text.secondary}>{format!("Expires at {}", state.remote_pairing.as_ref().unwrap().expires_at)}</Text>
                </Column>
            })}
            <Row gap={8.0} align={Align::Center}>
                {if state.remote_pairing.is_some() {
                    AnyView::new(ui! {
                        <Row gap={8.0}>
                            {state.remote_pairing.as_ref().and_then(|pairing| pairing.manual_pairing_code.as_ref()).map(|code| ui! {
                                <Button on_press={ChatMessage::CopyLoginText(code.clone())}
                                    background={theme.surfaces.card} color={theme.text.primary}>{"Copy code"}</Button>
                            })}
                            <Button on_press={ChatMessage::CancelRemotePairing}
                                background={theme.surfaces.hover} color={theme.text.danger}>{"Stop waiting"}</Button>
                        </Row>
                    })
                } else {
                    if connected && !state.remote_control_pending {
                        AnyView::new(ui! { <Button on_press={ChatMessage::StartRemotePairing}
                            background={theme.accent.ordinary} color={theme.accent.on_accent}>{"Pair a phone"}</Button> })
                    } else {
                        AnyView::new(ui! { <Text color={theme.text.secondary}>{"Pair a phone"}</Text> })
                    }
                }}
            </Row>
            {if state.remote_control_pending {
                AnyView::new(ui! { <Text color={theme.text.secondary}>{"Waiting for Codex…"}</Text> })
            } else { AnyView::new(Spacer::vertical(0.0)) }}
            {state.remote_control_message.as_ref().map(|message| ui! {
                <Text color={theme.text.secondary} wrap={true}>{message}</Text>
            })}
            {if state.remote_clients.is_empty() {
                AnyView::new(ui! { <Text color={theme.text.secondary}>{"No paired phones"}</Text> })
            } else {
                AnyView::new(ui! {
                    <Column fill_width gap={6.0}>
                        <Text color={theme.text.secondary}>{"Paired phones"}</Text>
                        {state.remote_clients.iter().map(|client| ui! {
                            <Container key={client.client_id.clone()} fill_width padding={Insets::all(8.0)}
                                background={theme.surfaces.card} radius={8.0}>
                                <Row fill_width gap={8.0} align={Align::Center}>
                                    <Column grow={1.0}>
                                        <Text color={theme.text.primary}>{client.display_name.as_deref().unwrap_or("Phone")}</Text>
                                    </Column>
                                    {environment_id.map(|environment_id| ui! {
                                        <Button on_press={ChatMessage::RevokeRemoteClient(environment_id.to_owned(), client.client_id.clone())}
                                            background={theme.surfaces.hover} color={theme.text.danger}>{"Revoke"}</Button>
                                    })}
                                </Row>
                            </Container>
                        })}
                    </Column>
                })
            }}
        </Column>
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use nickel_codex::{ReplayBackend, Thread, ThreadId};
    use nickel_ui::{HostBatch, HostEvent, Rect, UiFrame, UiHost};
    use nickel_ui_testkit::Scenario;

    use super::*;

    #[test]
    fn resumed_medium_transcript_has_no_estimated_blank_spacer() {
        let mut state = ChatState::default();
        state.conversation_pinned = true;
        state.conversation_viewport_height = 650.0;
        for index in 0..22 {
            state.items.push_back(ChatItem {
                id: format!("resumed-{index}"),
                kind: if index % 2 == 0 {
                    ChatItemKind::User
                } else {
                    ChatItemKind::Agent
                },
                // The estimate treats this as tall, while rich Markdown can
                // lay it out much shorter. Every row must still be present.
                text: "A long resumed message ".repeat(100),
                complete: true,
            });
        }

        let window = transcript_window(&state);
        assert_eq!(window.range, 0..22);
        assert_eq!(window.leading, 0.0);
        assert_eq!(window.trailing, 0.0);

        for index in 22..=TRANSCRIPT_VIRTUALIZATION_THRESHOLD {
            state.items.push_back(ChatItem {
                id: format!("resumed-{index}"),
                kind: ChatItemKind::Agent,
                text: "Short".into(),
                complete: true,
            });
        }
        let large_window = transcript_window(&state);
        assert!(large_window.range.start > 0);
        assert_eq!(large_window.range.end, state.items.len());
    }

    #[test]
    fn pinned_large_transcript_mounts_bounded_tail_despite_height_overestimates() {
        let mut state = ChatState::default();
        state.conversation_pinned = true;
        state.conversation_viewport_height = 650.0;
        for index in 0..200 {
            state.items.push_back(ChatItem {
                id: format!("long-{index}"),
                kind: ChatItemKind::Agent,
                text: "Markdown source much taller than its rendered card ".repeat(300),
                complete: true,
            });
        }

        let window = transcript_window(&state);
        assert_eq!(window.range, 136..200);
        assert_eq!(window.trailing, 0.0);
        assert!(window.leading > 0.0);

        state.conversation_pinned = false;
        state.conversation_scroll = 0.0;
        let unpinned = transcript_window(&state);
        assert_eq!(unpinned.range.start, 0);
        assert!(unpinned.range.end < 200);
    }

    #[test]
    fn run_settings_close_icon_has_a_legible_raster_footprint() {
        let icon = close_icon_pixels(0x00ff_ffff);
        assert_eq!(icon.dimensions(), (24, 24));
        let painted = icon.pixels().filter(|pixel| pixel[3] != 0).count();
        assert!(
            painted >= 80,
            "close strokes should not collapse into a tiny glyph"
        );
    }

    #[test]
    fn shell_approval_snapshot_rejects_scope_revision_and_repeated_action() {
        let backend = ReplayBackend::from_json(r#"{"name":"approval","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        let request_id = ServerRequestId("request".into());
        let request = |sequence, root: &str| {
            ControllerEvent::Protocol(nickel_codex::CodexEvent {
                sequence,
                kind: nickel_codex::EventKind::ApprovalRequested {
                    request_id: request_id.clone(),
                    thread_id: Some(ThreadId("thread".into())),
                    approval_type: "item/fileChange/requestApproval".into(),
                    summary: Some("Write files".into()),
                    context: nickel_codex::ApprovalContext {
                        grant_root: Some(root.into()),
                        ..Default::default()
                    },
                },
            })
        };
        app.state.apply(app.state.generation, request(1, "/safe"));
        let old = app.approval_notifications().remove(0);
        // A duplicate authority fact must not create a fresh action identity or toast.
        app.state.apply(app.state.generation, request(2, "/safe"));
        assert_eq!(app.approval_notifications(), vec![old.clone()]);
        assert!(app.report_approval_notification_delivery(&old, false));
        assert!(app.state.approval_notification_unavailable(&request_id));
        app.state
            .apply(app.state.generation, request(3, "/broader"));
        assert!(!app.state.approval_notification_unavailable(&request_id));
        assert!(!app.report_approval_notification_delivery(&old, false));
        assert!(!app.respond_approval_notification(&old, CodexApprovalChoice::Approve));

        let broader = app.approval_notifications().remove(0);
        assert_eq!(
            broader.presentation.scope.as_deref(),
            Some("Requested write root: /broader")
        );
        app.state.apply(app.state.generation, request(4, "/safe"));
        assert!(!app.respond_approval_notification(&old, CodexApprovalChoice::Approve));
        assert!(!app.respond_approval_notification(&broader, CodexApprovalChoice::Approve));
        let current = app.approval_notifications().remove(0);
        assert!(current.request_revision > old.request_revision);
        assert!(app.report_approval_notification_delivery(&current, false));
        assert!(app.state.approval_notification_unavailable(&request_id));
        assert!(app.report_approval_notification_delivery(&current, true));
        assert!(!app.state.approval_notification_unavailable(&request_id));
        assert!(app.respond_approval_notification(&current, CodexApprovalChoice::Cancel));
        assert!(!app.respond_approval_notification(&current, CodexApprovalChoice::Approve));
        app.state.apply(
            app.state.generation,
            ControllerEvent::Protocol(nickel_codex::CodexEvent {
                sequence: 5,
                kind: nickel_codex::EventKind::ServerRequestResolved {
                    thread_id: ThreadId("thread".into()),
                    request_id: request_id.clone(),
                },
            }),
        );
        app.state.apply(app.state.generation, request(6, "/new"));
        let reused = app.approval_notifications().remove(0);
        assert!(reused.request_revision > current.request_revision);
        assert!(!app.respond_approval_notification(&current, CodexApprovalChoice::Cancel));
        assert!(app.respond_approval_notification(&reused, CodexApprovalChoice::Cancel));
    }

    #[test]
    fn unsupported_approval_cannot_claim_a_notification_response() {
        let backend = ReplayBackend::from_json(r#"{"name":"approval","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        let request_id = ServerRequestId("unsupported".into());
        app.state.apply(
            app.state.generation,
            ControllerEvent::Protocol(nickel_codex::CodexEvent {
                sequence: 1,
                kind: nickel_codex::EventKind::ApprovalRequested {
                    request_id: request_id.clone(),
                    thread_id: Some(ThreadId("thread".into())),
                    approval_type: "item/unknown/requestApproval".into(),
                    summary: Some("Unknown decision".into()),
                    context: nickel_codex::ApprovalContext::default(),
                },
            }),
        );
        let notification = app.approval_notifications().remove(0);
        assert!(!notification.actionable);
        assert!(!app.respond_approval_notification(&notification, CodexApprovalChoice::Approve));
        assert!(!app.state.interaction_submission_pending(&request_id));
    }

    #[test]
    fn offered_command_decisions_gate_both_notification_and_inline_actions() {
        let backend = ReplayBackend::from_json(r#"{"name":"choices","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
        app.controller = controller;
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        let request_id = ServerRequestId("decision-set".into());
        let decisions = vec![CommandDecision::AcceptForSession, CommandDecision::Decline];
        app.state.apply(
            app.state.generation,
            ControllerEvent::Protocol(nickel_codex::CodexEvent {
                sequence: 1,
                kind: nickel_codex::EventKind::ApprovalRequested {
                    request_id: request_id.clone(),
                    thread_id: Some(ThreadId("thread".into())),
                    approval_type: "item/commandExecution/requestApproval".into(),
                    summary: Some("Run command".into()),
                    context: nickel_codex::ApprovalContext {
                        available_decisions: Some(decisions),
                        ..Default::default()
                    },
                },
            }),
        );
        let notification = app.approval_notifications().remove(0);
        assert!(notification.needs_review());
        assert!(notification.notification_actions().is_empty());
        assert!(
            notification
                .presentation
                .detail
                .as_deref()
                .unwrap()
                .contains("future matching prompts in this session")
        );
        assert!(!app.respond_approval_notification(&notification, CodexApprovalChoice::Approve));
        app.update(ChatMessage::RespondApproval(
            request_id.clone(),
            "item/commandExecution/requestApproval".into(),
            CodexApprovalChoice::Cancel,
        ));
        assert!(!app.state.interaction_submission_pending(&request_id));
        assert!(app.respond_approval_notification(
            &notification,
            CodexApprovalChoice::Command(CommandDecision::AcceptForSession),
        ));
        assert!(app.state.interaction_submission_pending(&request_id));
        assert!(matches!(
            commands.recv_timeout(std::time::Duration::from_millis(10)),
            Ok(ControllerCommand::CommandApproval {
                request_id: delivered,
                decision: CommandDecision::AcceptForSession,
            }) if delivered == request_id
        ));
    }

    #[test]
    fn compact_four_decision_approval_scrolls_to_last_action() {
        let backend = ReplayBackend::from_json(r#"{"name":"choices","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.apply(
            app.state.generation,
            ControllerEvent::Protocol(nickel_codex::CodexEvent {
                sequence: 1,
                kind: nickel_codex::EventKind::ApprovalRequested {
                    request_id: ServerRequestId("four-choices".into()),
                    thread_id: Some(ThreadId("thread".into())),
                    approval_type: "item/commandExecution/requestApproval".into(),
                    summary: Some("Run command".into()),
                    context: nickel_codex::ApprovalContext {
                        available_decisions: Some(vec![
                            CommandDecision::Accept,
                            CommandDecision::AcceptForSession,
                            CommandDecision::Decline,
                            CommandDecision::Cancel,
                        ]),
                        ..Default::default()
                    },
                },
            }),
        );
        let mut scenario = Scenario::new(app, 640, 480);
        let pending = scenario
            .host()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Approve".into(),
            })
            .expect("first approval action");
        let point = nickel_ui::Point {
            x: pending.bounds.origin.x + pending.bounds.size.width / 2.0,
            y: pending.bounds.origin.y + pending.bounds.size.height / 2.0,
        };
        scenario
            .host_mut()
            .handle_event(nickel_ui::UiEvent::Scroll {
                point,
                delta_y: 400.0,
            });
        scenario
            .pointer_activate(&nickel_ui_testkit::Selector::role_name(
                SemanticRole::Button,
                "Cancel turn",
            ))
            .expect("last offered decision remains reachable after scrolling");
    }

    #[test]
    fn approval_copy_uses_structured_scope_without_exposing_protocol_ids() {
        let context = nickel_codex::ApprovalContext {
            item_id: Some("internal-item-id".into()),
            approval_id: Some("internal-callback-id".into()),
            command: Some("cargo test".into()),
            cwd: Some("/projects/nickel".into()),
            asks_network_access: true,
            proposes_session_rule: true,
            ..Default::default()
        };
        let copy = codex_approval_presentation(
            "item/commandExecution/requestApproval",
            "Run tests",
            &context,
        );
        assert!(copy.scope_line().contains("/projects/nickel"));
        assert!(copy.warning.as_deref().unwrap().contains("future commands"));
        assert!(copy.detail.as_deref().unwrap().contains("cargo test"));
        assert!(!copy.notification_body().contains("internal-item-id"));
        assert!(!copy.notification_body().contains("internal-callback-id"));
        let missing = codex_approval_presentation(
            "item/fileChange/requestApproval",
            "Edit files",
            &nickel_codex::ApprovalContext::default(),
        );
        assert!(missing.scope_line().contains("not supplied"));
    }

    #[test]
    fn command_approval_shows_command_outside_bounded_secondary_details() {
        let command = "cargo test -p nickel-codex-ui --lib";
        let interaction = PendingInteraction::Approval {
            request_id: ServerRequestId("approval".into()),
            approval_type: "item/commandExecution/requestApproval".into(),
            summary: "Run a command".into(),
            context: nickel_codex::ApprovalContext {
                command: Some(command.into()),
                cwd: Some("/projects/nickel".into()),
                ..Default::default()
            },
        };
        let frame = UiFrame::layout(
            ui! { <InteractionCard interaction={&interaction} answer={""} actionable={true}
            submitting={false} delivery={InteractionDeliveryStatus {
                response_unconfirmed: false,
                notification_unavailable: false,
            }} theme={semantic_theme()} compact_approval_details={true} /> },
            Rect::new(0.0, 0.0, 520.0, 240.0),
        );
        assert!(has_accessible_text(&frame, command));
        assert!(has_accessible_text(&frame, "Approve"));
        let nodes = frame.resolved_layout().nodes();
        let command_node = nodes
            .iter()
            .find(|node| node.id.as_str().ends_with("approval-command"))
            .expect("prominent command node");
        let detail_node = nodes
            .iter()
            .find(|node| node.id.as_str().ends_with("approval-details"))
            .expect("secondary detail node");
        assert!(command_node.allocated.origin.y < detail_node.allocated.origin.y);
    }

    fn alternate_theme() -> SemanticTheme {
        SemanticTheme::from_tokens(nickel_ui::SemanticTokenSet::standard(
            0xf4f6f8, 0xe8edf4, 0xffffff, 0xd6dce5, 0xcbd2dc, 0x171a20, 0x4d5664, 0x075ca8,
            0xc9e5ff, 0x6c3fa0, 0xefe4ff,
        ))
    }

    #[test]
    fn backend_diagnostics_are_reachable_but_not_permanent_composer_copy() {
        let backend = ReplayBackend::from_json(r#"{"name":"diagnostics","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.status = ConnectionStatus::Disconnected;
        app.state.account.authenticated = true;
        app.state.provenance = "Installed Codex 9.9.9".into();
        app.state.backend_source = Some(nickel_codex::CandidateSource::Bundled);
        app.state.fallback_reason = Some("installed schema incompatible".into());
        app.state.report_diagnostic("Transport interrupted");
        let normal = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 900.0, 640.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 900.0, 640.0),
        );
        assert!(has_accessible_text(&normal, "Codex is disconnected"));
        assert!(!has_accessible_text(&normal, "Installed Codex 9.9.9"));
        app.update(ChatMessage::ToggleDiagnostics);
        let summary = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 900.0, 640.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 900.0, 640.0),
        );
        assert!(has_accessible_text(
            &summary,
            "The Codex connection was interrupted."
        ));
        assert!(has_accessible_text(&summary, "Copy safe summary"));
        assert!(has_accessible_text(&summary, "Show technical details"));
        assert!(!has_accessible_text(&summary, "Installed Codex 9.9.9"));
        app.update(ChatMessage::ToggleDiagnosticDetails);
        let details = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 900.0, 640.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 900.0, 640.0),
        );
        assert!(has_accessible_text(&details, "Installed Codex 9.9.9"));
        assert!(has_accessible_text(&details, "Transport interrupted"));
        assert!(has_accessible_text(
            &details,
            "Installed Codex was rejected: installed schema incompatible"
        ));
        assert!(has_accessible_text(&details, "Reconnect"));
    }

    #[test]
    fn diagnostic_surface_distinguishes_backend_transport_auth_and_turn_failures() {
        for (status, authenticated, turn_failed, cause, reconnect) in [
            (
                ConnectionStatus::Unavailable,
                true,
                false,
                "Codex is not available.",
                true,
            ),
            (
                ConnectionStatus::Disconnected,
                true,
                false,
                "The Codex connection was interrupted.",
                true,
            ),
            (
                ConnectionStatus::Incompatible,
                true,
                false,
                "The Codex backend is incompatible with this version of Nickel.",
                true,
            ),
            (
                ConnectionStatus::Ready,
                false,
                false,
                "Codex needs sign-in.",
                false,
            ),
            (
                ConnectionStatus::Ready,
                true,
                true,
                "The last turn failed, but the connection is healthy.",
                false,
            ),
        ] {
            let backend = ReplayBackend::from_json(r#"{"name":"failure-state","events":[]}"#)
                .expect("offline backend");
            let mut app = ChatApplication::new(BackendMode::Replay {
                backend,
                cwd: "/projects/nickel".into(),
            });
            app.state.status = status;
            app.state.account.authenticated = authenticated;
            if turn_failed {
                app.state.last_turn_error = Some((
                    nickel_codex::TurnId("turn".into()),
                    "The command failed".into(),
                    false,
                ));
            }
            app.update(ChatMessage::ToggleDiagnostics);
            let area = Rect::new(0.0, 0.0, 640.0, 480.0);
            let frame = UiFrame::layout(
                app.view(nickel_ui::ViewContext::new(
                    area,
                    nickel_ui::InputModality::Keyboard,
                )),
                area,
            );
            assert!(has_accessible_text(&frame, cause), "missing {cause}");
            assert_eq!(
                has_accessible_text(&frame, "Reconnect"),
                reconnect,
                "wrong recovery action for {cause}"
            );
        }
    }

    #[test]
    fn unauthenticated_ready_transport_shows_sign_in_not_send() {
        let backend = ReplayBackend::from_json(r#"{"name":"sign-in","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.status = ConnectionStatus::Ready;
        app.state.draft = "Do not send".into();
        let frame = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 640.0, 480.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 640.0, 480.0),
        );
        assert!(has_accessible_text(&frame, "Sign in to Codex"));
        assert!(
            !frame
                .resolved_layout()
                .nodes()
                .iter()
                .any(|node| node.id.as_str().ends_with("send-button"))
        );
    }

    #[test]
    fn copied_diagnostic_summary_excludes_source_payloads_and_secrets() {
        let backend = ReplayBackend::from_json(r#"{"name":"safe-copy","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.status = ConnectionStatus::Disconnected;
        app.state.provenance = "token=super-secret".into();
        app.state.fallback_reason = Some("fallback secret".into());
        app.state
            .report_diagnostic("prompt content /private/file pairing-secret");
        app.state.draft = "private draft".into();
        app.state.selected_thread = Some(ThreadId("private-thread-id".into()));
        app.update(ChatMessage::ToggleDiagnostics);
        app.update(ChatMessage::CopyDiagnosticSummary);
        let copied = app.clipboard_write.take().unwrap();
        assert!(copied.contains("Disconnected"));
        for secret in [
            "super-secret",
            "fallback secret",
            "prompt content",
            "/private/file",
            "pairing-secret",
            "private draft",
            "private-thread-id",
        ] {
            assert!(!copied.contains(secret), "copied summary included {secret}");
        }
        assert!(app.diagnostic_copy_result.is_none());
        assert!(app.clipboard_write_completed(Ok(())));
        assert!(matches!(app.diagnostic_copy_result, Some(Ok(()))));
        let area = Rect::new(0.0, 0.0, 640.0, 480.0);
        let copied = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                area,
                nickel_ui::InputModality::Keyboard,
            )),
            area,
        );
        assert!(has_accessible_text(
            &copied,
            "Safe diagnostic summary copied to clipboard"
        ));

        app.update(ChatMessage::CopyDiagnosticSummary);
        let _ = app.clipboard_write.take().unwrap();
        assert!(app.clipboard_write_completed(Err("selection unavailable".into())));
        let failed = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                area,
                nickel_ui::InputModality::Keyboard,
            )),
            area,
        );
        assert!(has_accessible_text(
            &failed,
            "Safe diagnostic summary could not be copied"
        ));
    }

    #[test]
    fn authentication_and_phone_permission_views_protect_remote_access() {
        let backend = ReplayBackend::from_json(r#"{"name":"protection","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        assert!(
            app.remote_access_protected(),
            "initial sign-in view must be protected"
        );
        app.state.account.authenticated = true;
        assert!(!app.remote_access_protected());
        app.update(ChatMessage::OpenRemoteControl);
        assert!(app.remote_access_protected());
        app.update(ChatMessage::CloseRemoteControl);
        assert!(!app.remote_access_protected());
        app.update(ChatMessage::ManageRemoteHosts);
        assert!(app.remote_access_protected());
        app.update(ChatMessage::CloseRemoteHosts);
        assert!(!app.remote_access_protected());
        app.update(ChatMessage::AddRemoteHost);
        assert!(app.remote_access_protected());
        app.update(ChatMessage::CloseRemoteHosts);
        assert!(!app.remote_access_protected());
        app.state.apply(
            app.state.generation,
            ControllerEvent::LoginStarted(nickel_codex::LoginChallenge::DeviceCode {
                login_id: "fixture".into(),
                user_code: "PRIVATE-CODE".into(),
                verification_url: "https://example.test/device".into(),
            }),
        );
        assert!(
            app.remote_access_protected(),
            "device code is ordinary text but remains protected"
        );
        app.state.apply(
            app.state.generation,
            ControllerEvent::LoginCancelled("fixture".into()),
        );
        assert!(!app.remote_access_protected());
    }

    #[test]
    fn escape_backs_out_of_phone_and_host_panels_before_interrupting_chat() {
        let backend = ReplayBackend::from_json(r#"{"name":"panel-back","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.active_turn = Some(nickel_codex::TurnId("running".into()));
        app.update(ChatMessage::OpenRemoteControl);
        app.shortcut_outcome(Shortcut::Escape);
        assert!(!app.remote_control_open);
        assert_eq!(
            app.state.active_turn,
            Some(nickel_codex::TurnId("running".into()))
        );

        app.update(ChatMessage::ManageRemoteHosts);
        app.update(ChatMessage::AddRemoteHost);
        app.shortcut_outcome(Shortcut::Escape);
        assert!(app.managing_hosts);
        assert!(app.host_editor.is_none());
        app.shortcut_outcome(Shortcut::Escape);
        assert!(!app.managing_hosts);
    }

    #[test]
    fn leaving_phone_panel_stops_hidden_pairing_attempt() {
        let backend = ReplayBackend::from_json(r#"{"name":"pair-back","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
        app.controller = controller;
        app.remote_control_open = true;
        app.state.remote_pairing = Some(nickel_codex::RemotePairingChallenge {
            environment_id: "environment".into(),
            expires_at: 42,
            pairing_code: "opaque".into(),
            manual_pairing_code: Some("12345678".into()),
        });

        app.update(ChatMessage::CloseRemoteControl);

        assert!(!app.remote_control_open);
        assert!(matches!(
            commands.try_recv(),
            Ok(ControllerCommand::CancelRemotePairing)
        ));
    }

    #[test]
    fn empty_controller_polls_back_off_without_rebuilding() {
        let backend = ReplayBackend::from_json(r#"{"name":"idle","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        let started = Instant::now();
        let mut host = UiHost::new_at(app, 800, 600, started);

        let first_due = started + CONTROLLER_POLL_MIN;
        let first = host.step(HostBatch {
            now: Some(first_due),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        assert!(!first.changed);
        assert!(!first.telemetry.rebuilt);
        assert_eq!(
            first.next_deadline,
            Some(first_due + Duration::from_millis(32))
        );

        let second_due = first.next_deadline.unwrap();
        let second = host.step(HostBatch {
            now: Some(second_due),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        assert!(!second.changed);
        assert!(!second.telemetry.rebuilt);
        assert_eq!(
            second.next_deadline,
            Some(second_due + Duration::from_millis(64))
        );
    }

    #[test]
    fn user_activity_restores_low_latency_controller_polling() {
        let backend = ReplayBackend::from_json(r#"{"name":"idle","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        for _ in 0..4 {
            assert!(!app.poll_controller());
        }
        assert_eq!(Application::poll_interval(&app), Some(CONTROLLER_POLL_MAX));

        app.update(ChatMessage::DraftChanged("hello".into()));

        assert_eq!(Application::poll_interval(&app), Some(CONTROLLER_POLL_MIN));
    }

    #[test]
    fn embedded_theme_contract_is_live_and_idempotent() {
        let backend = ReplayBackend::from_json(r#"{"name":"theme","events":[]}"#).unwrap();
        let app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        let light = alternate_theme();

        let mut scenario = Scenario::new(app, 900, 640);
        let dark_commands = scenario.host().commands().to_vec();
        assert!(scenario.host_mut().application_mut().set_theme(light));
        scenario.host_mut().step(HostBatch {
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });

        assert_eq!(scenario.host().application().theme, light);
        assert_ne!(scenario.host().commands(), dark_commands);
        assert!(!scenario.host_mut().application_mut().set_theme(light));
    }

    #[test]
    fn production_codex_and_task_switcher_views_reject_literal_colors() {
        fn assert_no_literal_colors(source: &str, label: &str) {
            let offenders = source
                .lines()
                .enumerate()
                .filter(|(_, line)| {
                    let color_sink = [
                        "background=",
                        "color=",
                        "Border::new(",
                        "foreground:",
                        "muted:",
                        "accent:",
                        "surface:",
                        "border:",
                        "code:",
                    ]
                    .iter()
                    .any(|sink| line.contains(sink));
                    color_sink
                        && line.as_bytes().windows(2).any(|prefix| prefix == b"0x")
                        && !line.trim_start().starts_with("//")
                })
                .map(|(line, text)| format!("{}: {}", line + 1, text.trim()))
                .collect::<Vec<_>>();
            assert!(offenders.is_empty(), "{label}: {offenders:?}");
        }

        let codex = include_str!("view.rs").replace("\r\n", "\n");
        let fallback_start = codex.find("fn semantic_theme()").unwrap();
        let fallback_end = codex[fallback_start..]
            .find("\n}\n\n")
            .map(|offset| fallback_start + offset + 3)
            .unwrap();
        let mut production = codex[..fallback_start].to_owned();
        production.push_str(&codex[fallback_end..codex.find("#[cfg(test)]\nmod tests").unwrap()]);
        assert_no_literal_colors(&production, "Codex UI");
        assert_no_literal_colors(
            include_str!("../../nickel/src/window_preview.rs")
                .split("#[cfg(test)]")
                .next()
                .unwrap(),
            "task switcher",
        );
    }

    fn has_accessible_text<Message: Clone>(frame: &UiFrame<Message>, needle: &str) -> bool {
        frame.accessibility_nodes().iter().any(|node| {
            node.label
                .as_deref()
                .is_some_and(|label| label.contains(needle))
                || node
                    .description
                    .as_deref()
                    .is_some_and(|description| description.contains(needle))
        })
    }

    #[test]
    fn interaction_fixture_has_a_deterministic_activity_projection() {
        let fixture: nickel_codex::ReplayScenario = serde_json::from_str(include_str!(
            "../../nickel-codex-fixture/fixtures/interactions.json"
        ))
        .expect("offline interaction fixture");
        let project = || {
            let mut state = ChatState::default();
            for event in fixture.events.iter().cloned() {
                state.apply(state.generation, ControllerEvent::Protocol(event));
            }
            assert_eq!(state.pending.len(), 3);
            state
                .items
                .iter()
                .map(|item| {
                    (
                        item.id.clone(),
                        item_label(&item.kind).to_owned(),
                        if is_collapsible_activity(&item.kind) {
                            activity_summary(
                                item,
                                state.activity_outcomes.get(&item.id),
                                state
                                    .file_change_summaries
                                    .get(&item.id)
                                    .map(String::as_str),
                            )
                        } else {
                            item.text.clone()
                        },
                    )
                })
                .collect::<Vec<_>>()
        };
        let first = project();
        assert_eq!(
            first,
            project(),
            "replaying the same fixture must not reorder cards"
        );
        assert_eq!(
            first,
            vec![
                (
                    "local:request:1:1".into(),
                    "Question from Codex".into(),
                    "Which implementation should Codex use?\nAnswer in the request panel below."
                        .into(),
                ),
                (
                    "command-1".into(),
                    "Command".into(),
                    "Running command".into()
                ),
                (
                    "patch-1".into(),
                    "File change".into(),
                    "Running file change".into(),
                ),
                ("plan-1".into(), "Plan".into(), "fixture plan".into()),
                (
                    "reasoning-1".into(),
                    "Reasoning summary".into(),
                    "fixture summary".into(),
                ),
            ]
        );
    }

    #[test]
    fn file_change_card_never_promotes_diff_prose_to_a_target() {
        let item = ChatItem {
            id: "file".into(),
            kind: ChatItemKind::FileChange,
            text: "+/not/an/authoritative/path\n-older line".into(),
            complete: false,
        };
        assert_eq!(activity_summary(&item, None, None), "Running file change");
        assert_eq!(
            activity_summary(&item, None, Some("update: /project/real.rs")),
            "Running: update: /project/real.rs"
        );
    }

    #[test]
    fn primary_chat_regions_remain_reachable_across_required_viewports() {
        let mut state = ChatState::default();
        state.account.authenticated = true;
        state.status = ConnectionStatus::Ready;
        // These are logical client areas, not physical output dimensions. The
        // scaled-output case is represented by its usable 960 × 540 area; the
        // 1120-unit case matches the nested project's default chat window.
        for draft in ["", "A message ready to send"] {
            state.draft = draft.into();
            for (width, height) in [
                (1920.0, 1080.0),
                (1366.0, 768.0),
                (1280.0, 720.0),
                (1120.0, 760.0),
                (960.0, 540.0),
                (640.0, 480.0),
                (640.0, 960.0),
            ] {
                let frame = UiFrame::layout(
                    configured_chat_view(
                        &state,
                        &DEFAULT_CODEX_SETTINGS,
                        ChatHostPanel::default(),
                        ChatOverlays::default(),
                        semantic_theme(),
                        width,
                    ),
                    Rect::new(0.0, 0.0, width, height),
                );
                let nodes = frame.resolved_layout().nodes();
                for suffix in [
                    "conversation",
                    "composer-viewport",
                    "composer-status",
                    "send-button",
                ] {
                    let bounds = nodes
                        .iter()
                        .find(|node| node.id.as_str().ends_with(suffix))
                        .unwrap_or_else(|| panic!("{suffix} missing at {width}×{height}"))
                        .allocated;
                    assert!(
                        bounds.size.width > 0.0 && bounds.size.height > 0.0,
                        "{suffix} collapsed at {width}×{height}: {bounds:?}"
                    );
                    assert!(
                        bounds.origin.x >= 0.0 && bounds.origin.y >= 0.0,
                        "{suffix} starts outside {width}×{height}: {bounds:?}"
                    );
                    assert!(
                        bounds.origin.x + bounds.size.width <= width + 0.5
                            && bounds.origin.y + bounds.size.height <= height + 0.5,
                        "{suffix} extends outside {width}×{height}: {bounds:?}"
                    );
                }
                assert!(
                    nodes
                        .iter()
                        .any(|node| node.id.as_str().ends_with("codex-menu"))
                );
            }
        }
    }

    #[test]
    fn long_content_keeps_composer_and_transcript_within_compact_client_bounds() {
        let cases = [
            (ChatItemKind::User, "A long user request. ".repeat(180)),
            (
                ChatItemKind::Agent,
                "A long assistant explanation. ".repeat(220),
            ),
            (
                ChatItemKind::FileChange,
                format!("/project/{}", "deep/nested/".repeat(180)),
            ),
            (
                ChatItemKind::Command,
                format!("$ cargo test {}", "--all-targets ".repeat(240)),
            ),
            (
                ChatItemKind::FileChange,
                format!("@@ -1 +1 @@\n{}", "+changed line\n".repeat(240)),
            ),
            (
                ChatItemKind::Error,
                "Backend failure with context. ".repeat(180),
            ),
            (
                ChatItemKind::Unknown("future/event".into()),
                "Unrecognized event. ".repeat(180),
            ),
        ];
        for (index, (kind, text)) in cases.into_iter().enumerate() {
            let mut state = ChatState::default();
            state.account.authenticated = true;
            state.status = ConnectionStatus::Ready;
            state.draft = "A long pending prompt. ".repeat(120);
            state.items.push_back(ChatItem {
                id: format!("stress-{index}"),
                kind,
                text,
                complete: true,
            });
            if matches!(
                state.items.back().map(|item| &item.kind),
                Some(ChatItemKind::Command | ChatItemKind::FileChange)
            ) {
                state.expanded_items.insert(format!("stress-{index}"));
            }
            for (width, height) in [(640.0, 480.0), (960.0, 540.0)] {
                let frame = UiFrame::layout(
                    configured_chat_view(
                        &state,
                        &DEFAULT_CODEX_SETTINGS,
                        ChatHostPanel::default(),
                        ChatOverlays::default(),
                        semantic_theme(),
                        width,
                    ),
                    Rect::new(0.0, 0.0, width, height),
                );
                let nodes = frame.resolved_layout().nodes();
                let region = |suffix: &str| {
                    nodes
                        .iter()
                        .find(|node| node.id.as_str().ends_with(suffix))
                        .unwrap_or_else(|| panic!("{suffix} missing in case {index}"))
                        .allocated
                };
                let conversation = region("conversation");
                let editor = region("composer-viewport");
                let submit = region("send-button");
                let transcript = region("transcript-column");
                assert!(
                    conversation.size.height >= 120.0,
                    "case {index} at {width}×{height}"
                );
                assert!((52.0..=140.0).contains(&editor.size.height));
                let horizontal_padding = if width < 720.0 { 16.0 } else { 36.0 };
                assert!(
                    transcript.size.width >= conversation.size.width - horizontal_padding - 0.5,
                    "case {index} transcript does not use available width at {width}×{height}: {transcript:?}"
                );
                // Transcript content intentionally extends inside its clipped,
                // independently scrollable conversation viewport.
                assert!(transcript.origin.y >= conversation.origin.y);
                for (name, bounds) in [
                    ("conversation", conversation),
                    ("editor", editor),
                    ("submit", submit),
                ] {
                    assert!(
                        bounds.origin.x >= 0.0 && bounds.origin.y >= 0.0,
                        "{name}: {bounds:?}"
                    );
                    assert!(
                        bounds.origin.x + bounds.size.width <= width + 0.5,
                        "{name}: {bounds:?}"
                    );
                    assert!(
                        bounds.origin.y + bounds.size.height <= height + 0.5,
                        "{name}: {bounds:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn compact_composer_bounds_attachment_previews_without_hiding_editor() {
        let mut state = ChatState::default();
        state.account.authenticated = true;
        for index in 0..8 {
            state.attachments.push(
                crate::PendingAttachment::from_rgba(
                    crate::AttachmentId(index),
                    1,
                    1,
                    &[255, 0, 0, 255],
                    crate::AttachmentLimits::default(),
                )
                .expect("fixture image"),
            );
        }
        let frame = UiFrame::layout(
            configured_chat_view(
                &state,
                &DEFAULT_CODEX_SETTINGS,
                ChatHostPanel::default(),
                ChatOverlays::default(),
                semantic_theme(),
                640.0,
            ),
            Rect::new(0.0, 0.0, 640.0, 480.0),
        );
        let nodes = frame.resolved_layout().nodes();
        let attachments = nodes
            .iter()
            .find(|node| node.id.as_str().ends_with("attachment-list"))
            .expect("bounded attachments");
        let editor = nodes
            .iter()
            .find(|node| node.id.as_str().ends_with("composer-viewport"))
            .expect("composer editor");
        assert!(attachments.allocated.size.height <= 96.0);
        assert!(editor.allocated.origin.y + editor.allocated.size.height <= 480.0);
        assert!(has_accessible_text(&frame, "Remove image attachment 7"));
    }

    #[test]
    fn compact_pending_approval_preserves_transcript_and_actions() {
        let mut state = ChatState::default();
        state.account.authenticated = true;
        state.status = ConnectionStatus::Ready;
        state.apply(
            state.generation,
            ControllerEvent::Protocol(nickel_codex::CodexEvent {
                sequence: 1,
                kind: nickel_codex::EventKind::ApprovalRequested {
                    request_id: ServerRequestId("compact-approval".into()),
                    thread_id: Some(nickel_codex::ThreadId("thread".into())),
                    approval_type: "item/commandExecution/requestApproval".into(),
                    summary: Some("Run tests".into()),
                    context: nickel_codex::ApprovalContext {
                        command: Some("cargo test ".to_owned() + &"--all ".repeat(100)),
                        cwd: Some("/projects/nickel".into()),
                        ..Default::default()
                    },
                },
            }),
        );
        // A saturated shell feed must leave the inline request actionable.
        state.mark_approval_notification_unavailable(&ServerRequestId("compact-approval".into()));
        let frame = UiFrame::layout(
            configured_chat_view(
                &state,
                &DEFAULT_CODEX_SETTINGS,
                ChatHostPanel::default(),
                ChatOverlays::default(),
                semantic_theme(),
                640.0,
            ),
            Rect::new(0.0, 0.0, 640.0, 480.0),
        );
        let nodes = frame.resolved_layout().nodes();
        let bounds = |suffix: &str| {
            nodes
                .iter()
                .find(|node| node.id.as_str().ends_with(suffix))
                .expect("Codex layout region")
                .allocated
        };
        assert!(bounds("conversation").size.height >= 120.0);
        assert!(bounds("pending-interactions").size.height <= 146.0);
        assert!(bounds("approval-details").size.height <= 18.0);
        let editor = bounds("composer-viewport");
        assert!(editor.origin.y + editor.size.height <= 480.0);
        assert!(has_accessible_text(&frame, "Approve"));
        assert!(has_accessible_text(
            &frame,
            "Shell notification unavailable"
        ));
        let backend = ReplayBackend::from_json(r#"{"name":"approval","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state = state;
        let mut scenario = Scenario::new(app, 640, 480);
        scenario
            .pointer_activate(&nickel_ui_testkit::Selector::role_name(
                SemanticRole::Button,
                "Approve",
            ))
            .expect("approval action remains clickable at minimum size");
    }

    #[test]
    fn pending_approval_survives_live_run_settings_collapse_and_expansion() {
        let backend = ReplayBackend::from_json(r#"{"name":"resize","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.apply(
            app.state.generation,
            ControllerEvent::Protocol(nickel_codex::CodexEvent {
                sequence: 1,
                kind: nickel_codex::EventKind::ApprovalRequested {
                    request_id: ServerRequestId("resize-approval".into()),
                    thread_id: Some(ThreadId("thread".into())),
                    approval_type: "item/commandExecution/requestApproval".into(),
                    summary: Some("Run tests".into()),
                    context: nickel_codex::ApprovalContext::default(),
                },
            }),
        );
        let mut scenario = Scenario::new(app, 1280, 720);
        // Run settings replace the three inline selectors below 1280 logical
        // units; the source-owned approval must not disappear in either mode.
        for (width, height) in [(1279, 720), (960, 540), (640, 480), (1280, 720)] {
            scenario.resize(width, height, 1.0).expect("live resize");
            let approve = scenario
                .host()
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Approve".into(),
                })
                .expect("approval decision after resize");
            assert!(approve.bounds.origin.x + approve.bounds.size.width <= width as f32);
            assert!(approve.bounds.origin.y + approve.bounds.size.height <= height as f32);
            if width < 1280 {
                scenario
                    .pointer_activate(&nickel_ui_testkit::Selector::id("root/menu-bar/codex-menu"))
                    .expect("Codex menu opens");
                scenario
                    .pointer_activate(&nickel_ui_testkit::Selector::role_name(
                        SemanticRole::MenuItem,
                        "Run settings",
                    ))
                    .expect("menu-owned run settings remains operable");
                assert!(
                    scenario
                        .host()
                        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                            role: SemanticRole::Button,
                            name: "Approval policy selector".into(),
                        })
                        .is_ok()
                );
                scenario
                    .pointer_activate(&nickel_ui_testkit::Selector::id("root/menu-bar/codex-menu"))
                    .expect("Codex menu reopens");
                scenario
                    .pointer_activate(&nickel_ui_testkit::Selector::role_name(
                        SemanticRole::MenuItem,
                        "Run settings",
                    ))
                    .expect("menu-owned run settings closes");
            }
        }
        scenario
            .resize(960, 540, 2.0)
            .expect("high-scale logical client");
        scenario.host_mut().set_scale_factor(2.0);
        scenario
            .pointer_activate(&nickel_ui_testkit::Selector::id("root/menu-bar/codex-menu"))
            .expect("scaled Codex menu hit region");
        scenario
            .pointer_activate(&nickel_ui_testkit::Selector::role_name(
                SemanticRole::MenuItem,
                "Run settings",
            ))
            .expect("scaled menu-owned settings hit region");
        assert!(
            scenario
                .host()
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Approval policy selector".into(),
                })
                .is_ok()
        );
    }

    #[test]
    fn streamed_content_does_not_repin_a_reader_scrolled_away() {
        let backend = ReplayBackend::from_json(r#"{"name":"scroll","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.update(ChatMessage::ConversationScrolled(nickel_ui::ScrollExtent {
            viewport: nickel_ui::Size {
                width: 640.0,
                height: 240.0,
            },
            content: nickel_ui::Size {
                width: 640.0,
                height: 1200.0,
            },
            offset_x: 0.0,
            offset: 120.0,
        }));
        assert!(!app.state.conversation_pinned);
        let mut scenario = Scenario::new(app, 1280, 720);
        scenario
            .resize(1279, 720, 1.0)
            .expect("collapse before streaming");
        let generation = scenario.host().application().state.generation;
        scenario.host_mut().application_mut().state.apply(
            generation,
            ControllerEvent::Protocol(nickel_codex::CodexEvent {
                sequence: 1,
                kind: nickel_codex::EventKind::AgentMessageDelta {
                    item_id: "stream".into(),
                    delta: "New text arrived while reading history".into(),
                },
            }),
        );
        scenario.host_mut().step(HostBatch {
            application_changed: true,
            ..HostBatch::default()
        });
        scenario
            .resize(640, 480, 1.0)
            .expect("compact while streaming");
        assert!(!scenario.host().application().state.conversation_pinned);
        assert!(
            scenario
                .host()
                .application()
                .state
                .new_content_while_unpinned
        );
        scenario
            .pointer_activate(&nickel_ui_testkit::Selector::role_name(
                SemanticRole::Button,
                "Jump to latest",
            ))
            .expect("reader can choose to resume following the transcript");
        assert!(scenario.host().application().state.conversation_pinned);
        assert!(
            !scenario
                .host()
                .application()
                .state
                .new_content_while_unpinned
        );
    }

    #[test]
    fn composer_ime_preedit_survives_compact_resize_without_committing_early() {
        let backend = ReplayBackend::from_json(r#"{"name":"ime","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        let mut scenario = Scenario::new(app, 1280, 720);
        let editor = scenario
            .host()
            .query_unique(&nickel_ui::SemanticSelector::Role(SemanticRole::TextField))
            .expect("composer field");
        let point = nickel_ui::Point {
            x: editor.bounds.origin.x + editor.bounds.size.width / 2.0,
            y: editor.bounds.origin.y + editor.bounds.size.height / 2.0,
        };
        scenario
            .host_mut()
            .handle_event(nickel_ui::UiEvent::PointerPressed(point));
        scenario
            .host_mut()
            .handle_event(nickel_ui::UiEvent::PointerReleased(point));
        scenario.ime_preedit("かな").expect("preedit starts");
        assert!(scenario.host().application().state.draft.is_empty());
        scenario.resize(640, 480, 1.0).expect("compact resize");
        assert!(scenario.host().application().state.draft.is_empty());
        scenario.text_input("仮名").expect("commit after resize");
        assert_eq!(scenario.host().application().state.draft, "仮名");
    }

    #[test]
    fn transcript_selection_survives_compact_resize() {
        let backend = ReplayBackend::from_json(r#"{"name":"selection","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.apply(
            app.state.generation,
            ControllerEvent::Protocol(nickel_codex::CodexEvent {
                sequence: 1,
                kind: nickel_codex::EventKind::AgentMessageDelta {
                    item_id: "selection".into(),
                    delta: "A distinct passage for selection across live layout changes.".into(),
                },
            }),
        );
        let mut scenario = Scenario::new(app, 1280, 720);
        let frame = UiFrame::layout(
            configured_chat_view(
                &scenario.host().application().state,
                &DEFAULT_CODEX_SETTINGS,
                ChatHostPanel::default(),
                ChatOverlays::default(),
                semantic_theme(),
                1280.0,
            ),
            Rect::new(0.0, 0.0, 1280.0, 720.0),
        );
        let nodes = frame.resolved_layout().nodes();
        // Markdown body text is paint-only, so select its leaf through the
        // production transcript layout instead of inventing screen coordinates.
        let passage = nodes
            .iter()
            .rfind(|node| {
                node.id.as_str().contains("transcript-selection")
                    && node.allocated.size.height > 0.0
            })
            .unwrap_or_else(|| {
                panic!(
                    "rendered transcript passage: {:?}",
                    nodes
                        .iter()
                        .map(|node| node.id.as_str())
                        .collect::<Vec<_>>()
                )
            });
        let start = nickel_ui::Point {
            x: passage.allocated.origin.x + 8.0,
            y: passage.allocated.origin.y + passage.allocated.size.height / 2.0,
        };
        let end = nickel_ui::Point {
            x: start.x + 100.0,
            y: start.y,
        };
        scenario
            .host_mut()
            .handle_event(nickel_ui::UiEvent::PointerPressed(start));
        scenario
            .host_mut()
            .handle_event(nickel_ui::UiEvent::PointerMoved(end));
        scenario
            .host_mut()
            .handle_event(nickel_ui::UiEvent::PointerReleased(end));
        let selected = scenario
            .host()
            .selected_text()
            .expect("selected transcript text");
        assert!(!selected.is_empty());
        scenario.resize(640, 480, 1.0).expect("compact resize");
        assert_eq!(
            scenario.host().selected_text().as_deref(),
            Some(selected.as_str())
        );
    }

    #[test]
    fn open_command_popup_stays_inside_client_during_compact_resize() {
        let backend = ReplayBackend::from_json(r#"{"name":"popup","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.draft = "/".into();
        let mut scenario = Scenario::new(app, 1280, 720);
        for (width, height) in [(1279, 720), (960, 540), (640, 480), (1280, 720)] {
            scenario
                .resize(width, height, 1.0)
                .expect("live popup resize");
            assert_eq!(scenario.host().application().state.draft, "/");
            let popovers = scenario
                .host()
                .query(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Group,
                    name: "Slash commands".into(),
                });
            assert!(
                !popovers.is_empty(),
                "command popup missing at {width}×{height}"
            );
            for popover in popovers {
                assert!(popover.bounds.origin.x >= 0.0 && popover.bounds.origin.y >= 0.0);
                assert!(popover.bounds.origin.x + popover.bounds.size.width <= width as f32 + 0.5);
                assert!(
                    popover.bounds.origin.y + popover.bounds.size.height <= height as f32 + 0.5
                );
            }
        }
    }

    #[test]
    fn slash_search_keyboard_selection_opens_matching_command() {
        let backend = ReplayBackend::from_json(r#"{"name":"slash-keyboard","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.draft = "/sta".into();
        assert!(app.shortcut_outcome(Shortcut::NavigateUp).changed);
        assert!(app.shortcut_outcome(Shortcut::Submit).changed);
        assert!(app.diagnostics_open);
        assert!(app.state.draft.is_empty());
    }

    #[test]
    fn mention_command_searches_and_keyboard_selection_inserts_a_project_path() {
        let backend = ReplayBackend::from_json(r#"{"name":"mention-picker","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;

        app.update(ChatMessage::SelectCommand("/mention".into()));
        assert_eq!(app.state.draft, "@");
        assert_eq!(app.state.file_search_query.as_deref(), Some(""));
        app.state.apply(
            app.state.generation,
            ControllerEvent::FileSearchLoaded {
                query: String::new(),
                matches: vec![
                    nickel_codex::FileSearchMatch {
                        root: "/projects/nickel".into(),
                        path: "Cargo.toml".into(),
                        file_name: "Cargo.toml".into(),
                        is_directory: false,
                        score: 100,
                    },
                    nickel_codex::FileSearchMatch {
                        root: "/projects/nickel".into(),
                        path: "crates/nickel/src/main.rs".into(),
                        file_name: "main.rs".into(),
                        is_directory: false,
                        score: 90,
                    },
                ],
            },
        );
        assert!(app.shortcut_outcome(Shortcut::NavigateDown).changed);
        assert!(app.shortcut_outcome(Shortcut::Submit).changed);
        assert_eq!(app.state.draft, "@crates/nickel/src/main.rs ");
        assert!(app.state.file_search_query.is_none());
        assert!(app.state.active_turn.is_none());
    }

    #[test]
    fn mention_replacement_preserves_the_draft_prefix() {
        assert_eq!(mention_fragment("Please inspect @src/ma"), Some("src/ma"));
        assert_eq!(
            replace_mention_fragment("Please inspect @src/ma", "src/main.rs"),
            "Please inspect @src/main.rs "
        );
        assert_eq!(mention_fragment("email@example.test"), None);
    }

    #[test]
    fn feedback_requires_category_and_preserves_explicit_log_consent() {
        let backend = ReplayBackend::from_json(r#"{"name":"feedback","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
        app.controller = controller;
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;

        app.update(ChatMessage::SelectCommand("/feedback".into()));
        assert!(app.feedback_open);
        assert!(app.remote_access_protected());
        app.update(ChatMessage::SubmitFeedback(true));
        assert_eq!(
            app.state.command_feedback.as_deref(),
            Some("Choose a feedback category")
        );
        assert!(commands.try_recv().is_err());

        app.update(ChatMessage::SelectFeedbackCategory("bug".into()));
        app.update(ChatMessage::FeedbackReasonChanged(
            "The picker broke".into(),
        ));
        app.update(ChatMessage::SubmitFeedback(false));
        assert!(app.feedback_pending);
        assert!(matches!(
            commands.try_recv(),
            Ok(ControllerCommand::UploadFeedback {
                classification,
                reason: Some(reason),
                include_logs: false,
            }) if classification == "bug" && reason == "The picker broke"
        ));
        app.update(ChatMessage::CloseFeedback);
        assert!(app.feedback_open, "an in-flight report cannot be discarded");
    }

    #[test]
    fn feedback_panel_keeps_both_consent_actions_reachable_at_compact_size() {
        let backend =
            ReplayBackend::from_json(r#"{"name":"feedback-layout","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.update(ChatMessage::SelectCommand("/feedback".into()));
        app.update(ChatMessage::SelectFeedbackCategory("other".into()));

        let mut host = UiHost::new(app, 640, 480);
        host.step(HostBatch::default());
        for name in ["Send without diagnostics", "Send with diagnostics"] {
            let target = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: name.into(),
                })
                .unwrap_or_else(|error| panic!("{name}: {error:?}"));
            assert!(target.bounds.origin.x >= 0.0 && target.bounds.origin.y >= 0.0);
            assert!(target.bounds.origin.x + target.bounds.size.width <= 640.5);
            assert!(target.bounds.origin.y + target.bounds.size.height <= 480.5);
        }
    }

    #[test]
    fn diff_command_requests_workspace_diff_without_starting_a_turn() {
        let backend = ReplayBackend::from_json(r#"{"name":"diff","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
        app.controller = controller;
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.draft = "/diff".into();

        app.update(ChatMessage::Send);
        assert!(app.state.draft.is_empty());
        assert!(matches!(commands.try_recv(), Ok(ControllerCommand::Diff)));
        assert!(app.state.active_turn.is_none());

        app.state.apply(
            app.state.generation,
            ControllerEvent::DiffLoaded("```diff\n-old\n+new\n```".into()),
        );
        let item = app.state.items.back().expect("local diff transcript item");
        assert_eq!(item.kind, ChatItemKind::SessionNotice);
        assert!(item.text.contains("+new"));
    }

    #[test]
    fn review_command_carries_every_staged_run_setting() {
        let backend =
            ReplayBackend::from_json(r#"{"name":"review-settings","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
        app.controller = controller;
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.selected_model = Some("gpt-review".into());
        app.state.selected_reasoning_effort = Some("high".into());
        app.state.selected_approval_policy = ApprovalPolicy::Never;
        app.state.selected_sandbox_policy = Some(SandboxPolicy::DangerFullAccess);

        app.update(ChatMessage::SelectCommand("/review".into()));
        assert!(matches!(
            commands.try_recv(),
            Ok(ControllerCommand::Review {
                model: Some(model),
                reasoning_effort: Some(effort),
                approval_policy: ApprovalPolicy::Never,
                sandbox_policy: Some(SandboxPolicy::DangerFullAccess),
            }) if model == "gpt-review" && effort == "high"
        ));
    }

    #[test]
    fn plan_command_toggles_run_mode_without_sending_a_prompt() {
        let backend = ReplayBackend::from_json(r#"{"name":"plan-toggle","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;

        app.state.draft = "/plan".into();
        app.update(ChatMessage::Send);
        assert!(app.plan_mode);
        assert!(app.state.draft.is_empty());
        assert!(app.state.active_turn.is_none());

        app.state.draft = "/plan".into();
        app.update(ChatMessage::Send);
        assert!(!app.plan_mode);
        assert!(app.state.active_turn.is_none());
    }

    #[test]
    fn compact_user_question_keeps_answer_and_submit_reachable() {
        let mut state = ChatState::default();
        state.account.authenticated = true;
        state.status = ConnectionStatus::Ready;
        let request_id = ServerRequestId("compact-question".into());
        let question = nickel_codex::UserInputQuestion {
            id: "choice".into(),
            header: "Choose an approach".into(),
            question: "Which implementation should Codex use? ".repeat(30),
            options: (0..3)
                .map(|index| nickel_codex::UserInputOption {
                    label: format!("Option {index}"),
                    description: "Detailed consequences ".repeat(18),
                })
                .collect(),
            is_other: true,
            is_secret: false,
        };
        state.pending.push(PendingInteraction::UserInput {
            request_id,
            question_ids: vec!["choice".into()],
            questions: vec![question],
        });
        let frame = UiFrame::layout(
            configured_chat_view(
                &state,
                &DEFAULT_CODEX_SETTINGS,
                ChatHostPanel::default(),
                ChatOverlays::default(),
                semantic_theme(),
                640.0,
            ),
            Rect::new(0.0, 0.0, 640.0, 480.0),
        );
        let nodes = frame.resolved_layout().nodes();
        let conversation = nodes
            .iter()
            .find(|node| node.id.as_str().ends_with("conversation"))
            .expect("transcript viewport");
        assert!(conversation.allocated.size.height >= 119.5);
        let backend = ReplayBackend::from_json(r#"{"name":"question","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state = state;
        let mut scenario = Scenario::new(app, 640, 480);
        scenario
            .pointer_activate(&nickel_ui_testkit::Selector::role_name(
                SemanticRole::Button,
                "Submit",
            ))
            .expect("question submit remains clickable at minimum size");
    }

    #[test]
    fn thread_navigation_is_available_before_history_loads() {
        let backend = ReplayBackend::from_json(r#"{"name":"threads","events":[]}"#).unwrap();
        let app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        let mut scenario = Scenario::new(app, 640, 480);
        for id in [
            "file-menu",
            "thread-menu",
            "codex-menu",
            "view-menu",
            "connection-menu",
        ] {
            let menu = scenario
                .host()
                .query_unique(&nickel_ui::SemanticSelector::Id(
                    format!("root/menu-bar/{id}").into(),
                ))
                .expect("compact app-chrome menu");
            assert!(menu.bounds.origin.x >= 0.0);
            assert!(menu.bounds.origin.x + menu.bounds.size.width <= 640.0);
        }
        scenario
            .pointer_activate(&nickel_ui_testkit::Selector::id(
                "root/menu-bar/thread-menu",
            ))
            .expect("thread menu opens before the backend list loads");
        scenario
            .pointer_activate(&nickel_ui_testkit::Selector::role_name(
                SemanticRole::MenuItem,
                "Browse conversations…",
            ))
            .expect("conversation history remains discoverable");
        assert!(scenario.host_mut().application_mut().resume_picker_open);
    }

    #[test]
    fn source_supplied_thread_title_cannot_expand_menu_without_bound() {
        let title = "A very long project thread title ".repeat(20);
        let compact = bounded_menu_context(&title);
        assert_eq!(compact.chars().count(), 49);
        assert!(compact.ends_with('…'));
        assert_eq!(bounded_menu_context("Current work"), "Current work");
    }

    #[test]
    fn connection_menu_manage_hosts_action_wins_over_loaded_content() {
        let mut state = ChatState::default();
        state.threads.extend((0..40).map(|index| Thread {
            id: ThreadId(format!("thread-{index}")),
            title: Some(format!("Thread {index}")),
            cwd: Some("/projects/nickel".into()),
            last_used_at: Some(index),
            turns: Vec::new(),
            model: None,
            reasoning_effort: None,
        }));
        let backend = ReplayBackend::from_json(r#"{"name":"hosts","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state = state;
        let mut scenario = Scenario::new(app, 900, 640);
        assert!(
            scenario
                .host()
                .query_unique(&nickel_ui::SemanticSelector::Id(
                    "root/menu-bar/connection-menu".into(),
                ))
                .is_ok()
        );

        scenario
            .host_mut()
            .application_mut()
            .update(ChatMessage::ManageRemoteHosts);
        assert!(scenario.host_mut().application_mut().managing_hosts);
    }

    #[test]
    fn chat_uses_shared_markdown_and_keeps_unsupported_html_inert() {
        let item = ChatItem {
            id: "markdown".into(),
            kind: ChatItemKind::Agent,
            text: "# Heading\n- item with `code`\n```rust\nfn main() {}\n```\n<b>plain</b>".into(),
            complete: true,
        };
        let document = item_markdown_document(&item);
        assert_eq!(
            document.logical_text(),
            "Heading\n• item with code\nfn main() {}\n\n<b>plain</b>"
        );
        assert!(!document.diagnostics.is_empty());

        let tree = UiFrame::layout(
            ui! { <ItemCard item={&item} document={std::sync::Arc::new(item_markdown_document(&item))} expanded={true} theme={semantic_theme()} /> },
            Rect::new(0.0, 0.0, 600.0, 400.0),
        );
        assert!(has_accessible_text(&tree, "Heading"));
        assert!(has_accessible_text(&tree, "<b>plain</b>"));
    }

    #[test]
    fn command_activity_is_compact_until_details_are_requested() {
        let item = ChatItem {
            id: "command".into(),
            kind: ChatItemKind::Command,
            text: "$ cargo test\nSENTINEL_OUTPUT".into(),
            complete: true,
        };
        let document = std::sync::Arc::new(item_markdown_document(&item));
        let collapsed = UiFrame::layout(
            ui! { <ItemCard item={&item} document={empty_activity_document()} expanded={false} theme={semantic_theme()} /> },
            Rect::new(0.0, 0.0, 640.0, 480.0),
        );
        assert!(has_accessible_text(&collapsed, "▸"));
        assert!(!has_accessible_text(&collapsed, "SENTINEL_OUTPUT"));
        let expanded = UiFrame::layout(
            ui! { <ItemCard item={&item} document={document} expanded={true} theme={semantic_theme()} /> },
            Rect::new(0.0, 0.0, 640.0, 480.0),
        );
        assert!(has_accessible_text(&expanded, "SENTINEL_OUTPUT"));
    }

    #[test]
    fn completed_command_uses_backend_outcome_not_generic_finished_copy() {
        let item = ChatItem {
            id: "command".into(),
            kind: ChatItemKind::Command,
            text: "$ cargo test\nfailed output".into(),
            complete: true,
        };
        let outcome = ActivityOutcome {
            status: Some("failed".into()),
            exit_code: Some(2),
            duration_ms: Some(17),
        };
        let frame = UiFrame::layout(
            ui! { <ItemCard item={&item} document={empty_activity_document()} expanded={false} outcome={&outcome} theme={semantic_theme()} /> },
            Rect::new(0.0, 0.0, 640.0, 480.0),
        );
        assert!(has_accessible_text(&frame, "failed: $ cargo test"));
        assert!(has_accessible_text(&frame, "● 2"));
        assert!(has_accessible_text(&frame, "Duration: 17 ms"));
        assert!(!has_accessible_text(&frame, "failed output"));
    }

    #[test]
    fn compact_thread_picker_exposes_older_pages_and_partial_window() {
        let mut state = ChatState::default();
        state.thread_snapshot_available = true;
        state.thread_next_cursor = Some("100".into());
        state.thread_windowed = true;
        let frame = UiFrame::layout(
            resume_picker(&state, None, None, false, None, false, semantic_theme()),
            Rect::new(0.0, 0.0, 640.0, 480.0),
        );
        assert!(has_accessible_text(&frame, "Older conversations"));
        assert!(has_accessible_text(
            &frame,
            "bounded older-conversation window"
        ));
    }

    #[test]
    fn multiline_code_block_reserves_padding_beyond_both_text_lines() {
        let item = ChatItem {
            id: "code".into(),
            kind: ChatItemKind::Agent,
            text: "```text\nfirst line\nsecond line\n```".into(),
            complete: true,
        };
        let tree = UiFrame::layout(
            ui! { <ItemCard item={&item} document={std::sync::Arc::new(item_markdown_document(&item))} expanded={true} theme={semantic_theme()} /> },
            Rect::new(0.0, 0.0, 600.0, 200.0),
        );
        let nodes = tree.resolved_layout().nodes();
        let (code_index, code) = nodes
            .iter()
            .enumerate()
            .find(|(_, node)| node.id.as_str().contains("markdown-code-"))
            .expect("code row");
        let text_bounds = nodes[*code.children.first().expect("code text child")].allocated;
        let code_bounds = nodes
            .iter()
            .find(|node| node.children.contains(&code_index))
            .expect("code container")
            .allocated;
        assert!(text_bounds.size.height >= 31.0);
        assert!(code_bounds.size.height >= text_bounds.size.height + 18.0);
        assert!(text_bounds.origin.y >= code_bounds.origin.y + 9.0);
    }

    #[test]
    fn retry_replaces_a_stopped_controller_generation() {
        let backend = ReplayBackend::from_json(r#"{"name":"retry","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: PathBuf::from("/projects/nickel"),
        });
        app.state.status = ConnectionStatus::Disconnected;
        let generation = app.state.generation;

        app.update(ChatMessage::Refresh);

        assert_eq!(app.state.status, ConnectionStatus::Loading);
        assert_eq!(app.state.generation, generation + 1);
        assert_eq!(app.controller.generation(), generation + 1);
    }

    #[test]
    fn standalone_project_root_remains_fixed_for_new_conversations() {
        let backend = ReplayBackend::from_json(r#"{"name":"fixed-root","events":[]}"#).unwrap();
        let root = PathBuf::from("/projects/nickel");
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: root.clone(),
        });
        app.use_project_root(root.clone());
        app.update(ChatMessage::NewChat);

        assert_eq!(app.shell_project, Some((root, None)));
    }

    #[test]
    fn draft_replacement_uses_host_dialog_and_cancel_preserves_input() {
        let backend = ReplayBackend::from_json(r#"{"name":"discard","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        app.state.status = ConnectionStatus::Ready;
        app.state.draft = "Unsent work".into();
        app.update(ChatMessage::NewChat);
        assert!(matches!(
            app.pending_navigation,
            Some(PendingNavigation::NewChat)
        ));
        assert_eq!(app.state.draft, "Unsent work");
        let context = nickel_ui::ViewContext::new(
            Rect::new(0.0, 0.0, 640.0, 480.0),
            nickel_ui::InputModality::Keyboard,
        );
        assert_eq!(app.frame_overlays(context).len(), 1);
        let mut host = UiHost::new(app, 640, 480);
        let opened = host.step(HostBatch::default());
        assert_eq!(
            host.inspect().open_overlay,
            Some(OverlayId::new("codex-discard-draft")),
            "{opened:?}"
        );
        let keep = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Keep working".into(),
            })
            .unwrap();
        host.perform_accessibility_action(
            keep.id,
            nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
        );
        assert!(host.application().pending_navigation.is_none());
        assert_eq!(host.application().state.draft, "Unsent work");
        assert_eq!(host.inspect().open_overlay, None);
    }

    #[test]
    fn failed_thread_resume_after_discard_confirmation_keeps_the_draft() {
        let backend = ReplayBackend::from_json(r#"{"name":"resume","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        app.state.status = ConnectionStatus::Ready;
        app.state.draft = "Keep on failure".into();
        let destination = ThreadId("other".into());
        app.update(ChatMessage::SelectThread(destination.clone()));
        assert!(matches!(
            app.pending_navigation,
            Some(PendingNavigation::SelectThread(_))
        ));
        app.update(ChatMessage::ConfirmDiscardDraft);
        assert_eq!(app.state.draft, "Keep on failure");
        // A closed controller rejects the command synchronously, so no identity commits.
        assert_eq!(app.resume_picker_pending, None);
        assert_eq!(app.confirmed_thread_discard, None);
        assert_eq!(app.state.draft, "Keep on failure");
    }

    #[test]
    fn shell_resume_requires_discard_confirmation_without_losing_draft() {
        let backend = ReplayBackend::from_json(r#"{"name":"shell-resume","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.shell_host = true;
        app.state.draft = "Unsent work".into();
        let destination = ThreadId("another".into());
        app.update(ChatMessage::SelectThread(destination.clone()));
        assert!(
            matches!(app.take_shell_requests().as_slice(), [ShellRequest::ResumeThread(id)] if *id == destination)
        );
        assert!(!app.prepare_shell_resume(&destination));
        assert!(matches!(
            app.pending_navigation,
            Some(PendingNavigation::SelectThread(_))
        ));
        assert_eq!(app.state.draft, "Unsent work");
        app.update(ChatMessage::CancelDiscardDraft);
        assert_eq!(app.state.draft, "Unsent work");
    }

    #[test]
    fn focusing_existing_writer_clears_picker_without_replacing_draft() {
        let backend = ReplayBackend::from_json(r#"{"name":"owner","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.shell_host = true;
        app.resume_picker_open = true;
        app.resume_picker_pending = Some(ThreadId("open".into()));
        app.state.draft = "Still here".into();
        app.report_resume_owner_activation();
        assert!(!app.resume_picker_open);
        assert!(app.resume_picker_pending.is_none());
        assert_eq!(app.state.draft, "Still here");
    }

    #[test]
    fn shell_resume_commits_only_matching_hydrated_destination() {
        let backend = ReplayBackend::from_json(r#"{"name":"handoff","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        let (controller, events) = ChatController::fixture_with_events(app.state.generation);
        app.controller = controller;
        app.shell_host = true;
        let old = ThreadId("old".into());
        let destination = ThreadId("destination".into());
        app.state.selected_thread = Some(old.clone());
        app.state.draft = "Discard after success".into();
        app.pending_initial_resume = Some(destination.clone());
        app.shell_writer_thread = Some(destination.clone());
        app.confirmed_thread_discard = Some(destination.clone());
        let thread = |id: ThreadId| nickel_codex::Thread {
            id,
            title: None,
            cwd: None,
            last_used_at: None,
            turns: Vec::new(),
            model: None,
            reasoning_effort: None,
        };
        events
            .send((
                app.state.generation,
                ControllerEvent::ThreadSelected(thread(old.clone())),
            ))
            .unwrap();
        app.poll_controller();
        assert_eq!(app.state.selected_thread, Some(old));
        assert_eq!(app.state.draft, "Discard after success");
        assert!(app.take_shell_requests().is_empty());

        events
            .send((
                app.state.generation,
                ControllerEvent::ThreadSelected(thread(destination.clone())),
            ))
            .unwrap();
        app.poll_controller();
        assert_eq!(app.state.selected_thread, Some(destination.clone()));
        assert!(app.state.draft.is_empty());
        assert_eq!(
            app.take_shell_requests(),
            vec![ShellRequest::ResumeSucceeded(destination)]
        );
    }

    #[test]
    fn failed_shell_resume_restores_previous_writer_tracking() {
        let backend = ReplayBackend::from_json(r#"{"name":"failed-handoff","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        let (controller, events) = ChatController::fixture_with_events(app.state.generation);
        app.controller = controller;
        app.shell_host = true;
        let old = ThreadId("old".into());
        let destination = ThreadId("destination".into());
        app.shell_writer_thread = Some(destination.clone());
        app.pending_previous_writer = Some(old.clone());
        app.pending_initial_resume = Some(destination.clone());
        app.state.selected_thread = Some(old.clone());
        app.state.draft = "Keep this".into();
        events
            .send((
                app.state.generation,
                ControllerEvent::OperationFailed("no writer".into()),
            ))
            .unwrap();
        app.poll_controller();
        assert_eq!(app.shell_writer_thread, Some(old.clone()));
        assert!(app.pending_previous_writer.is_none());
        assert_eq!(app.state.selected_thread, Some(old));
        assert_eq!(app.state.draft, "Keep this");
        assert_eq!(
            app.take_shell_requests(),
            vec![ShellRequest::ResumeFailed(destination)]
        );
    }

    #[test]
    fn failed_recovery_stays_gated_and_offers_reconnect() {
        let backend = ReplayBackend::from_json(r#"{"name":"recovery","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        let (controller, events) = ChatController::fixture_with_events(app.state.generation);
        app.controller = controller;
        app.state.status = ConnectionStatus::Ready;
        app.state.draft = "Do not replay".into();
        app.state.recovery_pending = true;
        app.recovery_thread = Some(ThreadId("old".into()));
        events
            .send((
                app.state.generation,
                ControllerEvent::OperationFailed("reload rejected".into()),
            ))
            .unwrap();
        app.poll_controller();
        assert!(app.state.recovery_pending);
        assert!(app.state.recovery_failed);
        assert!(!app.state.can_send());
        assert_eq!(app.state.draft, "Do not replay");
        let frame = UiFrame::layout(
            diagnostics_panel(&app.state, false, None, semantic_theme()),
            Rect::new(0.0, 0.0, 640.0, 480.0),
        );
        assert!(has_accessible_text(&frame, "Conversation reload failed"));
        assert!(has_accessible_text(&frame, "Reconnect"));
    }

    #[test]
    fn reconnect_preserves_draft_and_fences_previous_turn() {
        let backend = ReplayBackend::from_json(r#"{"name":"reconnect","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.status = ConnectionStatus::Disconnected;
        app.state.selected_thread = Some(ThreadId("prior".into()));
        app.state.active_turn = Some(nickel_codex::TurnId("unfinished".into()));
        app.state.pending.push(PendingInteraction::Approval {
            request_id: ServerRequestId("old-approval".into()),
            approval_type: "command".into(),
            summary: "Old request".into(),
            context: nickel_codex::ApprovalContext::default(),
        });
        app.state.draft = "Unsent prompt".into();
        app.state.attach_rgba(1, 1, &[255, 0, 0, 255]).unwrap();
        let generation = app.state.generation;
        app.reconnect_controller();
        assert_eq!(app.state.generation, generation + 1);
        assert_eq!(app.state.status, ConnectionStatus::Loading);
        assert_eq!(app.recovery_thread, Some(ThreadId("prior".into())));
        assert!(app.state.recovery_pending);
        assert!(!app.state.can_send());
        assert_eq!(app.state.draft, "Unsent prompt");
        assert_eq!(app.state.attachments.len(), 1);
        assert!(app.state.active_turn.is_none());
        assert!(app.state.pending.is_empty());
        assert!(
            app.state
                .diagnostics
                .iter()
                .any(|message| message.contains("unconfirmed"))
        );
    }

    #[test]
    fn matching_recovery_hydration_reopens_send_without_replaying_draft() {
        let backend = ReplayBackend::from_json(r#"{"name":"hydration","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        let (controller, events) = ChatController::fixture_with_events(app.state.generation);
        app.controller = controller;
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.draft = "Still unsent".into();
        app.state.attach_rgba(1, 1, &[0, 255, 0, 255]).unwrap();
        app.state.recovery_pending = true;
        app.state.unconfirmed_work = true;
        let id = ThreadId("reloaded".into());
        app.recovery_thread = Some(id.clone());
        events
            .send((
                app.state.generation,
                ControllerEvent::ThreadSelected(nickel_codex::Thread {
                    id: id.clone(),
                    title: None,
                    cwd: None,
                    last_used_at: None,
                    turns: Vec::new(),
                    model: None,
                    reasoning_effort: None,
                }),
            ))
            .unwrap();
        app.poll_controller();
        assert_eq!(app.state.selected_thread, Some(id));
        assert!(!app.state.recovery_pending);
        assert!(!app.state.recovery_failed);
        assert!(!app.state.unconfirmed_work);
        assert!(app.state.can_send());
        assert_eq!(app.state.draft, "Still unsent");
        assert_eq!(app.state.attachments.len(), 1);
        assert!(app.take_shell_requests().is_empty());
    }

    #[test]
    fn empty_project_recovery_waits_for_configuration_acknowledgment() {
        let backend = ReplayBackend::from_json(r#"{"name":"project-ack","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        let (controller, events) = ChatController::fixture_with_events(app.state.generation);
        app.controller = controller;
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.draft = "Unsent".into();
        app.state.recovery_pending = true;
        app.recovery_project_configuration_pending = true;
        assert!(!app.state.can_send());
        events
            .send((app.state.generation, ControllerEvent::ProjectConfigured))
            .unwrap();
        app.poll_controller();
        assert!(!app.state.recovery_pending);
        assert!(app.state.can_send());
        assert_eq!(app.state.draft, "Unsent");
    }

    #[test]
    fn selecting_a_persisted_remote_host_replaces_local_project_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("codex-hosts.toml");
        let host = RemoteHost {
            id: "arm_host".into(),
            name: "ARM host".into(),
            endpoint: "ws://127.0.0.1:9/app-server".into(),
            token_env: None,
            default_cwd: "/srv/nickel".into(),
        };
        let mut settings = CodexSettings::default();
        settings.hosts.push(host.clone());
        settings.save(&path).unwrap();
        let backend = ReplayBackend::from_json(r#"{"name":"switch","events":[]}"#).unwrap();
        let mut app = ChatApplication::with_settings(
            BackendMode::Replay {
                backend,
                cwd: directory.path().into(),
            },
            settings,
            Some(path.clone()),
        );
        app.state.threads.push(Thread {
            id: ThreadId("local-thread".into()),
            title: Some("Local thread".into()),
            cwd: Some("/projects/local".into()),
            last_used_at: Some(1),
            turns: Vec::new(),
            model: None,
            reasoning_effort: None,
        });

        app.update(ChatMessage::SelectConnection("arm_host".into()));

        assert!(app.state.threads.is_empty());
        assert!(matches!(
            &app.mode,
            BackendMode::Remote { host } if host.id == "arm_host"
        ));
        assert_eq!(CodexSettings::load(&path).unwrap().selected, "arm_host");
    }

    #[test]
    fn invalid_host_edits_do_not_replace_last_known_good_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("codex-hosts.toml");
        let settings = CodexSettings::default();
        settings.save(&path).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        let backend = ReplayBackend::from_json(r#"{"name":"invalid","events":[]}"#).unwrap();
        let mut app = ChatApplication::with_settings(
            BackendMode::Replay {
                backend,
                cwd: directory.path().into(),
            },
            settings,
            Some(path.clone()),
        );

        app.update(ChatMessage::ManageRemoteHosts);
        app.update(ChatMessage::AddRemoteHost);
        app.update(ChatMessage::RemoteHostIdChanged("bad host".into()));
        app.update(ChatMessage::SaveRemoteHost);

        assert!(app.settings_error.is_some());
        assert!(app.settings.hosts.is_empty());
        assert_eq!(std::fs::read_to_string(path).unwrap(), before);
    }

    fn active_turn_app() -> ChatApplication {
        let backend = ReplayBackend::from_json(r#"{"name":"queue","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.controller = ChatController::fixture_idle(app.state.generation);
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.selected_thread = Some(ThreadId("thread".into()));
        app.state.apply(
            app.state.generation,
            ControllerEvent::Protocol(nickel_codex::CodexEvent {
                sequence: 1,
                kind: nickel_codex::EventKind::TurnStarted {
                    thread_id: ThreadId("thread".into()),
                    turn_id: nickel_codex::TurnId("turn".into()),
                },
            }),
        );
        app
    }

    #[test]
    fn queued_message_is_bounded_and_rejection_preserves_the_draft() {
        let mut app = active_turn_app();
        for index in 0..QUEUED_MESSAGE_CAPACITY {
            app.state.draft = format!("queued {index}");
            app.update(ChatMessage::QueueMessage);
        }
        assert_eq!(app.queued_messages.len(), QUEUED_MESSAGE_CAPACITY);
        app.state.draft = "keep this draft".into();
        app.update(ChatMessage::QueueMessage);
        assert_eq!(app.state.draft, "keep this draft");
        assert_eq!(app.queued_messages.len(), QUEUED_MESSAGE_CAPACITY);
    }

    #[test]
    fn queued_message_dispatch_uses_captured_image_and_run_settings() {
        let mut app = active_turn_app();
        let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
        app.controller = controller;
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([20, 40, 60, 255]),
        ))
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
        app.state.attach_image(encoded.get_ref()).unwrap();
        app.state.draft = "captured payload".into();
        app.state.selected_model = Some("original-model".into());
        app.state.selected_reasoning_effort = Some("high".into());
        app.state.selected_approval_policy = ApprovalPolicy::Never;
        app.state.selected_sandbox_policy = Some(SandboxPolicy::DangerFullAccess);
        app.plan_mode = true;

        app.update(ChatMessage::QueueMessage);
        assert!(commands.try_recv().is_err(), "queueing is not sending");
        assert!(app.state.attachments.is_empty());
        assert!(app.state.draft.is_empty());
        app.state.draft = "next draft".into();
        app.state.selected_model = Some("later-model".into());
        app.state.selected_reasoning_effort = Some("low".into());
        app.state.selected_approval_policy = ApprovalPolicy::OnRequest;
        app.state.selected_sandbox_policy = Some(SandboxPolicy::ReadOnly);
        app.plan_mode = false;
        app.state.active_turn = None;

        app.dispatch_queued_after_boundary();
        let Ok(ControllerCommand::Send {
            text,
            images,
            model,
            reasoning_effort,
            approval_policy,
            sandbox_policy,
            plan_mode,
        }) = commands.try_recv()
        else {
            panic!("expected one queued send");
        };
        assert_eq!(text, "captured payload");
        assert_eq!(images.len(), 1);
        assert!(images[0].data_url.starts_with("data:image/png;base64,"));
        assert_eq!(model.as_deref(), Some("original-model"));
        assert_eq!(reasoning_effort.as_deref(), Some("high"));
        assert_eq!(approval_policy, ApprovalPolicy::Never);
        assert_eq!(sandbox_policy, Some(SandboxPolicy::DangerFullAccess));
        assert!(plan_mode);
        assert_eq!(app.state.draft, "next draft");
    }

    #[test]
    fn queued_message_dispatches_once_only_after_matching_boundary() {
        let mut app = active_turn_app();
        let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
        app.controller = controller;
        app.state.draft = "follow up".into();
        app.update(ChatMessage::QueueMessage);
        assert!(
            commands.try_recv().is_err(),
            "queueing must not start a turn"
        );

        app.update(ChatMessage::InterruptAndSend);
        assert!(app.controller.fixture_interrupt_requested());
        app.state.apply(
            app.state.generation,
            ControllerEvent::Protocol(nickel_codex::CodexEvent {
                sequence: 2,
                kind: nickel_codex::EventKind::TurnCompleted {
                    thread_id: ThreadId("thread".into()),
                    turn_id: nickel_codex::TurnId("turn".into()),
                    status: "interrupted".into(),
                },
            }),
        );
        app.dispatch_queued_after_boundary();
        assert!(matches!(
            commands.try_recv(),
            Ok(ControllerCommand::Send { ref text, .. }) if text == "follow up"
        ));
        app.dispatch_queued_after_boundary();
        assert!(
            commands.try_recv().is_err(),
            "a boundary cannot double-send"
        );
    }

    #[test]
    fn timed_out_interrupt_never_sends_the_queued_message() {
        let mut app = active_turn_app();
        let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
        app.controller = controller;
        app.state.draft = "do not send without a boundary".into();
        app.update(ChatMessage::QueueMessage);
        app.update(ChatMessage::InterruptAndSend);
        app.queued_interrupt_deadline = Some(std::time::Instant::now());

        assert!(app.poll_controller());
        assert_eq!(
            app.queued_messages.front().map(|queued| queued.phase),
            Some(QueuedMessagePhase::InterruptTimedOut)
        );
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn conversation_change_preserves_queue_for_explicit_review() {
        let mut app = active_turn_app();
        app.state.draft = "keep this for the old conversation".into();
        app.update(ChatMessage::QueueMessage);

        app.mark_queued_messages_for_review("changing conversation");

        let queued = app.queued_messages.front().unwrap();
        assert_eq!(queued.phase, QueuedMessagePhase::Unconfirmed);
        assert_eq!(queued.thread_id, ThreadId("thread".into()));
        assert_eq!(queued.text, "keep this for the old conversation");
    }

    #[test]
    fn conversation_change_keeps_uncertain_dispatched_message_for_review() {
        let mut app = active_turn_app();
        app.state.draft = "delivery may already be in flight".into();
        app.update(ChatMessage::QueueMessage);
        app.queued_messages.front_mut().unwrap().phase = QueuedMessagePhase::Dispatching;

        app.mark_queued_messages_for_review("changing conversation");

        assert_eq!(
            app.queued_messages.front().map(|queued| queued.phase),
            Some(QueuedMessagePhase::Unconfirmed)
        );
        assert_eq!(
            app.queued_messages
                .front()
                .map(|queued| queued.text.as_str()),
            Some("delivery may already be in flight")
        );
    }

    #[test]
    fn saving_an_unconfirmed_message_does_not_retry_until_explicit_action() {
        let mut app = active_turn_app();
        let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
        app.controller = controller;
        app.state.draft = "possibly delivered".into();
        app.update(ChatMessage::QueueMessage);
        let id = app.queued_messages.front().unwrap().id;
        app.mark_queued_messages_for_review("connection loss");
        app.state.active_turn = None;

        app.update(ChatMessage::EditQueuedMessage(id));
        app.state.draft = "reviewed text".into();
        app.update(ChatMessage::QueueMessage);
        assert_eq!(
            app.queued_messages.front().map(|queued| queued.phase),
            Some(QueuedMessagePhase::Unconfirmed)
        );
        assert!(commands.try_recv().is_err(), "saving is not a retry");

        app.update(ChatMessage::RetryQueuedMessage(id));
        assert!(matches!(
            commands.try_recv(),
            Ok(ControllerCommand::Send { ref text, .. }) if text == "reviewed text"
        ));
        app.update(ChatMessage::RetryQueuedMessage(id));
        assert!(commands.try_recv().is_err(), "retry cannot double-send");
    }

    #[test]
    fn retry_rejects_a_different_conversation_without_sending() {
        let mut app = active_turn_app();
        let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
        app.controller = controller;
        app.state.draft = "old conversation".into();
        app.update(ChatMessage::QueueMessage);
        let id = app.queued_messages.front().unwrap().id;
        app.mark_queued_messages_for_review("changing conversation");
        app.state.active_turn = None;
        app.state.selected_thread = Some(ThreadId("other".into()));

        app.update(ChatMessage::RetryQueuedMessage(id));
        assert_eq!(
            app.queued_messages.front().map(|queued| queued.phase),
            Some(QueuedMessagePhase::Unconfirmed)
        );
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn stale_generation_never_dispatches_a_queued_message() {
        let mut app = active_turn_app();
        let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
        app.controller = controller;
        app.state.draft = "generation-bound".into();
        app.update(ChatMessage::QueueMessage);
        app.state.active_turn = None;
        app.state.generation = app.state.generation.saturating_add(1);

        app.dispatch_queued_after_boundary();

        assert_eq!(
            app.queued_messages.front().map(|queued| queued.phase),
            Some(QueuedMessagePhase::Unconfirmed)
        );
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn repeated_interrupt_and_send_emits_only_one_interrupt() {
        let mut app = active_turn_app();
        let (controller, _commands) = ChatController::fixture_with_commands(app.state.generation);
        app.controller = controller;
        app.state.draft = "one interrupt".into();
        app.update(ChatMessage::QueueMessage);

        app.update(ChatMessage::InterruptAndSend);
        let first_deadline = app.queued_interrupt_deadline;
        app.update(ChatMessage::InterruptAndSend);

        assert!(app.controller.fixture_interrupt_requested());
        assert_eq!(app.queued_interrupt_deadline, first_deadline);
        assert_eq!(
            app.queued_messages.front().map(|queued| queued.phase),
            Some(QueuedMessagePhase::InterruptRequested)
        );
    }

    #[test]
    fn natural_completion_racing_interrupt_acknowledgement_sends_once() {
        let mut app = active_turn_app();
        let (controller, commands, events) =
            ChatController::fixture_with_commands_and_events(app.state.generation);
        app.controller = controller;
        app.state.draft = "send after the natural boundary".into();
        app.update(ChatMessage::QueueMessage);
        app.update(ChatMessage::InterruptAndSend);

        events
            .send((
                app.state.generation,
                ControllerEvent::Protocol(nickel_codex::CodexEvent {
                    sequence: 2,
                    kind: nickel_codex::EventKind::TurnCompleted {
                        thread_id: ThreadId("thread".into()),
                        turn_id: nickel_codex::TurnId("turn".into()),
                        status: "completed".into(),
                    },
                }),
            ))
            .unwrap();
        assert!(app.poll_controller());
        assert!(matches!(
            commands.try_recv(),
            Ok(ControllerCommand::Send { ref text, .. })
                if text == "send after the natural boundary"
        ));

        events
            .send((app.state.generation, ControllerEvent::InterruptAccepted))
            .unwrap();
        app.poll_controller();
        assert!(commands.try_recv().is_err());
        assert_eq!(
            app.queued_messages.front().map(|queued| queued.phase),
            Some(QueuedMessagePhase::Dispatching)
        );
    }

    #[test]
    fn rejected_interrupt_keeps_the_queued_message_without_sending() {
        let mut app = active_turn_app();
        let (controller, commands, events) =
            ChatController::fixture_with_commands_and_events(app.state.generation);
        app.controller = controller;
        app.state.draft = "wait for review".into();
        app.update(ChatMessage::QueueMessage);
        app.update(ChatMessage::InterruptAndSend);

        events
            .send((
                app.state.generation,
                ControllerEvent::InterruptFailed("backend rejected interrupt".into()),
            ))
            .unwrap();
        assert!(app.poll_controller());
        assert_eq!(
            app.queued_messages.front().map(|queued| queued.phase),
            Some(QueuedMessagePhase::Waiting)
        );
        assert_eq!(app.queued_interrupt_deadline, None);
        assert!(!app.state.interrupt_requested);
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn disconnect_during_interrupt_requires_review_and_ignores_late_boundary() {
        let mut app = active_turn_app();
        let (controller, commands, events) =
            ChatController::fixture_with_commands_and_events(app.state.generation);
        app.controller = controller;
        app.state.draft = "preserve on disconnect".into();
        app.update(ChatMessage::QueueMessage);
        app.update(ChatMessage::InterruptAndSend);
        let generation = app.state.generation;

        events
            .send((
                generation,
                ControllerEvent::Failure("connection closed".into()),
            ))
            .unwrap();
        assert!(app.poll_controller());
        assert_eq!(app.state.status, ConnectionStatus::Disconnected);
        assert_eq!(app.queued_interrupt_deadline, None);
        assert_eq!(
            app.queued_messages.front().map(|queued| queued.phase),
            Some(QueuedMessagePhase::Unconfirmed)
        );
        assert_eq!(
            app.queued_messages
                .front()
                .map(|queued| queued.text.as_str()),
            Some("preserve on disconnect")
        );

        events
            .send((
                generation,
                ControllerEvent::Protocol(nickel_codex::CodexEvent {
                    sequence: 2,
                    kind: nickel_codex::EventKind::TurnCompleted {
                        thread_id: ThreadId("thread".into()),
                        turn_id: nickel_codex::TurnId("turn".into()),
                        status: "interrupted".into(),
                    },
                }),
            ))
            .unwrap();
        app.poll_controller();
        assert!(commands.try_recv().is_err());
    }
}
