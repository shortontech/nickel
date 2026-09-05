use std::path::PathBuf;

use nickel_codex::{
    ApprovalPolicy, BackendChoice, CodexSettings, CommandDecision, FileChangeDecision, RemoteHost,
    ServerRequestId,
};
use nickel_markdown::{MarkdownPalette, markdown_content_view};
use nickel_ui::SemanticRole;
use nickel_ui::prelude::*;

use crate::model::{item_label, item_markdown_document};
use crate::{
    BackendMode, ChatController, ChatItem, ChatItemKind, ChatState, ConnectionStatus,
    ControllerCommand, ControllerEvent, PendingInteraction, create_managed_workspace,
};

const TRANSCRIPT_GAP: f32 = 10.0;
const TRANSCRIPT_VIEWPORT_ESTIMATE: f32 = 600.0;
const TRANSCRIPT_OVERSCAN: f32 = 900.0;

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
    Reconnect,
    SelectThread(nickel_codex::ThreadId),
    ToggleModelPicker,
    ToggleReasoningPicker,
    ToggleApprovalPicker,
    SelectModel(String),
    SelectReasoningEffort(String),
    SelectApprovalPolicy(ApprovalPolicy),
    ToggleCommandPicker,
    SelectCommand(String),
    ToggleResumePicker,
    RefreshResumePicker,
    CloseResumePicker,
    Interrupt,
    Approve(ServerRequestId, String),
    Decline(ServerRequestId, String),
    InteractionAnswerChanged(String),
    SubmitInput(ServerRequestId, Vec<String>),
    DismissInput(ServerRequestId),
    ConversationScrolled(f32),
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
    ResumeFailed(nickel_codex::ThreadId),
}

fn draft_changed(value: String) -> ChatMessage {
    ChatMessage::DraftChanged(value)
}

fn interaction_answer_changed(value: String) -> ChatMessage {
    ChatMessage::InteractionAnswerChanged(value)
}

fn conversation_scrolled(offset: f32) -> ChatMessage {
    ChatMessage::ConversationScrolled(offset)
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
        ApprovalPolicy::Untrusted => "Ask for untrusted commands",
        ApprovalPolicy::OnFailure => "Ask after sandbox failure",
        ApprovalPolicy::OnRequest => "Ask when Codex requests",
        ApprovalPolicy::Never => "Never ask automatically",
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

const APPROVAL_POLICIES: [ApprovalPolicy; 4] = [
    ApprovalPolicy::Untrusted,
    ApprovalPolicy::OnFailure,
    ApprovalPolicy::OnRequest,
    ApprovalPolicy::Never,
];

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
    shell_project: Option<(PathBuf, Option<String>)>,
    pub(crate) pending_shell_command: Option<String>,
    pub(crate) shell_warning_acknowledged: bool,
    pub(crate) model_picker_generation: u64,
    reasoning_picker_generation: u64,
    approval_picker_generation: u64,
    pub(crate) resume_picker_open: bool,
    pub(crate) resume_picker_loading: bool,
    pub(crate) resume_picker_pending: Option<nickel_codex::ThreadId>,
    pub(crate) command_picker_open: bool,
    theme: SemanticTheme,
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
    resume_picker_open: bool,
    resume_picker_loading: bool,
    resume_picker_pending: Option<&'a nickel_codex::ThreadId>,
    command_picker_open: bool,
    project_root: Option<&'a std::path::Path>,
    project_id: Option<&'a str>,
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
            shell_project: None,
            pending_shell_command: None,
            shell_warning_acknowledged: false,
            model_picker_generation: 0,
            reasoning_picker_generation: 0,
            approval_picker_generation: 0,
            resume_picker_open: false,
            resume_picker_loading: false,
            resume_picker_pending: None,
            command_picker_open: false,
            theme: semantic_theme(),
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
        self.pending_initial_resume = Some(id.clone());
        self.shell_writer_thread = Some(id.clone());
        if self.controller.send(ControllerCommand::SelectThread(id)) {
            Ok(())
        } else {
            self.pending_initial_resume = None;
            self.shell_writer_thread = None;
            Err("Codex controller stopped before thread resume".into())
        }
    }

    pub fn take_shell_requests(&mut self) -> Vec<ShellRequest> {
        std::mem::take(&mut self.shell_requests)
    }

    pub fn report_resume_rejection(&mut self, message: impl Into<String>) {
        self.pending_initial_resume = None;
        self.resume_picker_pending = None;
        self.resume_picker_open = true;
        self.state.report_diagnostic(message);
    }

    pub fn use_project(&mut self, cwd: PathBuf, project_id: String) {
        self.shell_project = Some((cwd.clone(), Some(project_id.clone())));
        self.controller
            .send(ControllerCommand::NewChatIn(cwd, Some(project_id)));
    }

    pub fn use_project_root(&mut self, cwd: PathBuf) {
        self.shell_project = Some((cwd.clone(), None));
        self.controller
            .send(ControllerCommand::NewChatIn(cwd, None));
    }

    pub fn poll_controller(&mut self) -> bool {
        let mut changed = false;
        while let Some((generation, event)) = self.controller.try_recv() {
            if generation == self.state.generation {
                match &event {
                    ControllerEvent::ThreadSelected(thread) => {
                        if self.resume_picker_pending.as_ref() == Some(&thread.id) {
                            self.resume_picker_pending = None;
                            self.resume_picker_open = false;
                        }
                        if self.pending_initial_resume.as_ref() == Some(&thread.id) {
                            self.pending_initial_resume = None;
                        }
                    }
                    ControllerEvent::OperationFailed(_) => {
                        self.resume_picker_pending = None;
                        if self.pending_initial_resume.take().is_some()
                            && let Some(thread) = self.shell_writer_thread.take()
                        {
                            self.shell_requests.push(ShellRequest::ResumeFailed(thread));
                        }
                    }
                    ControllerEvent::ApprovalPolicyAccepted(policy) => {
                        self.accept_approval_policy(*policy);
                    }
                    ControllerEvent::Failure(_)
                    | ControllerEvent::Incompatible(_)
                    | ControllerEvent::Unavailable(_) => {
                        self.pending_initial_resume = None;
                        self.resume_picker_loading = false;
                        self.resume_picker_pending = None;
                        if let Some(thread) = self.shell_writer_thread.take() {
                            self.shell_requests.push(ShellRequest::ResumeFailed(thread));
                        }
                    }
                    _ => {}
                }
                if matches!(event, ControllerEvent::Ready { .. }) {
                    self.resume_picker_loading = false;
                }
            }
            changed |= self.state.apply(generation, event);
            if self.project_menu_mode {
                self.state.thread_error = None;
            }
        }
        changed
    }

    fn reconnect_controller(&mut self) {
        self.state.generation = self.state.generation.saturating_add(1);
        self.state.status = ConnectionStatus::Loading;
        self.state.diagnostics.clear();
        self.state.active_turn = None;
        self.state.interrupt_requested = false;
        self.controller = if self.project_menu_mode {
            ChatController::spawn_project_menu(self.mode.clone(), self.state.generation)
        } else if self.shell_host {
            ChatController::spawn_project_chat(self.mode.clone(), self.state.generation)
        } else {
            ChatController::spawn_generation(self.mode.clone(), self.state.generation)
        };
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

    fn update(&mut self, message: Self::Message) {
        match message {
            ChatMessage::DraftChanged(value) => self.state.draft = value,
            ChatMessage::PasteImage(bytes) => {
                if let Err(error) = self.state.attach_image(&bytes) {
                    self.state.report_diagnostic(error.to_string());
                }
            }
            ChatMessage::RemoveAttachment(id) => {
                self.state.remove_attachment(id);
            }
            ChatMessage::Send => {
                let command = self.state.draft.trim().to_owned();
                if command == "/model" {
                    self.state.draft.clear();
                    self.update(ChatMessage::ToggleModelPicker);
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
                } else if let Some((text, images)) = self.state.begin_send() {
                    self.controller.send(ControllerCommand::Send {
                        text,
                        images,
                        model: self.state.selected_model.clone(),
                        reasoning_effort: self.state.selected_reasoning_effort.clone(),
                        approval_policy: self.state.selected_approval_policy,
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
            ChatMessage::ToggleModelPicker => {
                self.model_picker_generation = self.model_picker_generation.saturating_add(1);
                self.resume_picker_open = false;
                self.command_picker_open = false;
            }
            ChatMessage::ToggleReasoningPicker => {
                self.reasoning_picker_generation =
                    self.reasoning_picker_generation.saturating_add(1);
                self.resume_picker_open = false;
                self.command_picker_open = false;
            }
            ChatMessage::ToggleApprovalPicker => {
                self.approval_picker_generation = self.approval_picker_generation.saturating_add(1);
                self.resume_picker_open = false;
                self.command_picker_open = false;
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
            ChatMessage::ToggleCommandPicker => {
                self.command_picker_open = !self.command_picker_open;
                self.resume_picker_open = false;
            }
            ChatMessage::SelectCommand(command) => {
                self.command_picker_open = false;
                self.state.draft = command;
                self.update(ChatMessage::Send);
            }
            ChatMessage::ToggleResumePicker => {
                self.resume_picker_open = !self.resume_picker_open;
                self.command_picker_open = false;
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
            ChatMessage::CloseResumePicker => {
                self.resume_picker_open = false;
                self.resume_picker_loading = false;
                self.resume_picker_pending = None;
            }
            ChatMessage::NewChat => {
                self.state.attachments.clear();
                self.resume_picker_open = false;
                self.resume_picker_pending = None;
                if self.project_menu_mode {
                    self.settings_error =
                        Some("Choose + beside a project for a new conversation".into());
                    return;
                }
                self.state.new_chat();
                if let Some((cwd, project_id)) = &self.shell_project {
                    self.controller.send(ControllerCommand::NewChatIn(
                        cwd.clone(),
                        project_id.clone(),
                    ));
                } else {
                    self.controller.send(ControllerCommand::NewChat);
                }
            }
            ChatMessage::NewChatIn(cwd, project_id) => {
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
                self.state.new_chat();
                self.controller
                    .send(ControllerCommand::NewChatIn(cwd.clone(), Some(project_id)));
                self.window_title = project_window_title(&cwd);
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
                if self.resume_picker_pending.is_some() {
                    return;
                }
                if self.shell_host
                    && self.state.thread_runtime.get(&id).is_some_and(|runtime| {
                        runtime.status == nickel_codex::ThreadRuntimeStatus::Active
                    })
                {
                    return;
                }
                if self.project_menu_mode {
                    return;
                }
                if self.shell_host {
                    self.resume_picker_pending = Some(id.clone());
                    self.shell_requests.push(ShellRequest::ResumeThread(id));
                    return;
                }
                if self.shell_host
                    && let Some(cwd) = self
                        .state
                        .threads
                        .iter()
                        .find(|thread| thread.id == id)
                        .and_then(|thread| thread.cwd.as_deref())
                {
                    self.window_title = project_window_title(cwd);
                }
                self.resume_picker_pending = Some(id.clone());
                self.controller.send(ControllerCommand::SelectThread(id));
            }
            ChatMessage::Interrupt => {
                if self.state.active_turn.is_some() && !self.state.interrupt_requested {
                    self.state.interrupt_requested = true;
                    self.controller.send(ControllerCommand::Interrupt);
                }
            }
            ChatMessage::Approve(request_id, approval_type) => {
                self.state.pending.retain(|pending| match pending {
                    PendingInteraction::Approval {
                        request_id: pending,
                        ..
                    }
                    | PendingInteraction::UserInput {
                        request_id: pending,
                        ..
                    } => pending != &request_id,
                });
                if approval_type.contains("fileChange") {
                    self.controller.send(ControllerCommand::FileApproval {
                        request_id,
                        decision: FileChangeDecision::Accept,
                    });
                } else {
                    self.controller.send(ControllerCommand::CommandApproval {
                        request_id,
                        decision: CommandDecision::Accept,
                    });
                }
            }
            ChatMessage::Decline(request_id, approval_type) => {
                self.state.pending.retain(|pending| match pending {
                    PendingInteraction::Approval {
                        request_id: pending,
                        ..
                    }
                    | PendingInteraction::UserInput {
                        request_id: pending,
                        ..
                    } => pending != &request_id,
                });
                if approval_type.contains("fileChange") {
                    self.controller.send(ControllerCommand::FileApproval {
                        request_id,
                        decision: FileChangeDecision::Decline,
                    });
                } else {
                    self.controller.send(ControllerCommand::CommandApproval {
                        request_id,
                        decision: CommandDecision::Decline,
                    });
                }
            }
            ChatMessage::InteractionAnswerChanged(value) => {
                self.state.interaction_answer = value;
            }
            ChatMessage::SubmitInput(request_id, question_ids) => {
                let answer = std::mem::take(&mut self.state.interaction_answer);
                let lines: Vec<_> = answer.lines().map(str::to_owned).collect();
                self.state.pending.retain(|pending| match pending {
                    PendingInteraction::Approval {
                        request_id: pending,
                        ..
                    }
                    | PendingInteraction::UserInput {
                        request_id: pending,
                        ..
                    } => pending != &request_id,
                });
                self.controller.send(ControllerCommand::UserInput {
                    request_id,
                    answers: question_ids
                        .into_iter()
                        .enumerate()
                        .map(|(index, question_id)| nickel_codex::UserInputAnswer {
                            question_id,
                            answer: lines.get(index).cloned().unwrap_or_default(),
                        })
                        .collect(),
                });
            }
            ChatMessage::DismissInput(request_id) => {
                self.state.pending.retain(|pending| match pending {
                    PendingInteraction::Approval {
                        request_id: pending,
                        ..
                    }
                    | PendingInteraction::UserInput {
                        request_id: pending,
                        ..
                    } => pending != &request_id,
                });
                self.controller.send(ControllerCommand::UserInput {
                    request_id,
                    answers: Vec::new(),
                });
            }
            ChatMessage::ConversationScrolled(offset) => {
                let heights = transcript_heights(&self.state);
                let total = VirtualWindow::from_heights(
                    &heights,
                    TRANSCRIPT_GAP,
                    f32::MAX,
                    TRANSCRIPT_VIEWPORT_ESTIMATE,
                    0.0,
                )
                .total;
                let maximum = (total - TRANSCRIPT_VIEWPORT_ESTIMATE).max(0.0);
                self.state.conversation_scroll = offset;
                self.state.conversation_pinned = offset >= maximum - 2.0;
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
        Some(std::time::Duration::from_millis(16))
    }

    fn shortcut(&mut self, shortcut: Shortcut) -> bool {
        match shortcut {
            Shortcut::Submit if self.state.can_send() => {
                self.update(ChatMessage::Send);
                true
            }
            Shortcut::Newline => false,
            Shortcut::Escape if self.resume_picker_open => {
                self.update(ChatMessage::CloseResumePicker);
                true
            }
            Shortcut::Escape if self.state.active_turn.is_some() => {
                self.update(ChatMessage::Interrupt);
                true
            }
            _ => false,
        }
    }

    fn view(&self, _context: nickel_ui::ViewContext) -> impl View<Self::Message> {
        if self.project_menu_mode {
            AnyView::new(project_menu_view(
                &self.state,
                self.settings_error.as_deref(),
                self.theme,
            ))
        } else {
            AnyView::new(configured_chat_view(
                &self.state,
                &self.settings,
                self.managing_hosts,
                self.host_editor.as_ref(),
                self.settings_error.as_deref(),
                ChatOverlays {
                    pending_shell_command: self.pending_shell_command.as_deref(),
                    model_picker_generation: self.model_picker_generation,
                    reasoning_picker_generation: self.reasoning_picker_generation,
                    approval_picker_generation: self.approval_picker_generation,
                    resume_picker_open: self.resume_picker_open,
                    resume_picker_loading: self.resume_picker_loading,
                    resume_picker_pending: self.resume_picker_pending.as_ref(),
                    command_picker_open: self.command_picker_open,
                    project_root: self.shell_project.as_ref().map(|(root, _)| root.as_path()),
                    project_id: self
                        .shell_project
                        .as_ref()
                        .and_then(|(_, id)| id.as_deref()),
                },
                self.theme,
            ))
        }
    }

    fn title(&self) -> &str {
        &self.window_title
    }

    fn initial_size(&self) -> (u32, u32) {
        (1120, 760)
    }
}

#[component]
fn ItemCard(item: &ChatItem, theme: SemanticTheme) -> impl View<ChatMessage> {
    let (background, color) = match &item.kind {
        ChatItemKind::User => (theme.surfaces.selected, theme.text.primary),
        ChatItemKind::Agent => (theme.surfaces.card, theme.text.primary),
        ChatItemKind::Reasoning => (theme.surfaces.sidebar, theme.text.secondary),
        ChatItemKind::Command => (theme.surfaces.hover, theme.text.primary),
        ChatItemKind::Activity => (theme.surfaces.card, theme.text.secondary),
        ChatItemKind::FileChange => (theme.surfaces.raised, theme.text.primary),
        ChatItemKind::Plan => (theme.surfaces.selected, theme.text.primary),
        ChatItemKind::Error => (theme.surfaces.raised, theme.text.danger),
        ChatItemKind::Unknown(_) => (theme.surfaces.card, theme.text.secondary),
    };
    let label = item_label(&item.kind);
    let document = item_markdown_document(item);
    let label_run_id = format!("{}/label", item.id);
    let (maximum_width, alignment) = if item.kind == ChatItemKind::User {
        (760.0, Align::End)
    } else {
        (920.0, Align::Start)
    };
    ui! {
        <Container fill_width max_width={maximum_width} align_self={alignment}
            padding={Insets::all(14.0)} gap={7.0}
            background={background} border={Border::new(theme.borders.ordinary, 1.0)} radius={10.0}>
            <Text color={color} scale={0.9} selection_run_id={label_run_id}
                selection_boundary={TextBoundary::Block}>{label}</Text>
            {markdown_content_view(
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
            )}
        </Container>
    }
}

#[component]
fn InteractionCard(
    interaction: &PendingInteraction,
    answer: &str,
    theme: SemanticTheme,
) -> impl View<ChatMessage> {
    match interaction {
        PendingInteraction::Approval {
            request_id,
            approval_type,
            summary,
        } => ui! {
            <Container fill_width padding={Insets::all(12.0)} gap={8.0}
                background={theme.surfaces.raised} border={Border::new(theme.text.warning, 1.0)} radius={8.0}>
                <Text color={theme.text.primary}>{"Approval requested"}</Text>
                <Text color={theme.text.secondary}>{summary}</Text>
                <Row gap={8.0}>
                    <Button on_press={ChatMessage::Decline(request_id.clone(), approval_type.clone())}
                        background={theme.surfaces.hover} color={theme.text.danger}>{"Decline"}</Button>
                    <Button on_press={ChatMessage::Approve(request_id.clone(), approval_type.clone())}
                        background={theme.surfaces.hover} color={theme.text.success}>{"Approve"}</Button>
                </Row>
            </Container>
        },
        PendingInteraction::UserInput {
            request_id,
            question_ids,
        } => ui! {
            <Container fill_width padding={Insets::all(12.0)} gap={8.0}
                background={theme.surfaces.raised} border={Border::new(theme.borders.ordinary, 1.0)} radius={8.0}>
                <Text color={theme.text.primary}>{"Codex requested input"}</Text>
                <Text color={theme.text.secondary}>{format!("Questions: {}", question_ids.join(", "))}</Text>
                <Text color={theme.text.secondary}>{"Enter one answer per line"}</Text>
                <Container fill_width padding={Insets::all(8.0)} background={theme.surfaces.card} radius={6.0}>
                    <TextField value={answer} on_change={interaction_answer_changed} color={theme.text.primary} />
                </Container>
                <Row gap={8.0}>
                    <Button on_press={ChatMessage::DismissInput(request_id.clone())}
                        background={theme.surfaces.hover} color={theme.text.danger}>{"Cancel"}</Button>
                    <Button on_press={ChatMessage::SubmitInput(request_id.clone(), question_ids.clone())}
                        background={theme.surfaces.hover} color={theme.text.success}>{"Submit"}</Button>
                </Row>
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
                <Text scale={1.6} color={theme.text.primary}>{if editor.original_id.is_some() { "Edit remote host" } else { "Add remote host" }}</Text>
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
            <Text scale={1.6} color={theme.text.primary}>{"Remote Codex hosts"}</Text>
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
        false,
        None,
        None,
        ChatOverlays::default(),
        semantic_theme(),
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
        <Column fill_width fill_height padding={Insets::all(14.0)} gap={10.0}
            background={theme.surfaces.window} border={Border::new(theme.borders.ordinary, 1.0)}>
            <Row fill_width shrink={0.0} gap={8.0}>
                <Text scale={1.25} color={theme.text.primary} grow={1.0}>{"Codex projects"}</Text>
                <Button on_press={ChatMessage::Refresh} background={theme.surfaces.card} color={theme.text.primary}
                    controller_focus_background_tint={controller_focus}>{"Retry"}</Button>
            </Row>
            <Text color={theme.text.secondary} shrink={0.0}>{status}</Text>
            <Container id={id!(project_search_container)} accessibility_label={"Search projects"}
                semantic_role={SemanticRole::Group} fill_width shrink={0.0}
                padding={Insets::symmetric(10.0, 8.0)} background={theme.surfaces.card}
                border={Border::new(theme.borders.ordinary, 1.0)} radius={6.0}>
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
                overflow_y={Overflow::Auto} gap={6.0}>
                {matching_projects.into_iter().map(|project| {
                    let root = project.roots[0].clone();
                    ui! {
                        <Button key={project.id.clone()} height={42.0}
                            on_press={ChatMessage::NewChatIn(root, project.id.clone())}
                            background={theme.surfaces.card} color={theme.text.primary} label_align={TextAlign::Start}
                            controller_focus_background_tint={controller_focus}
                            padding={Insets::symmetric(12.0, 8.0)} fill_width>{&project.name}</Button>
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
                <Text color={theme.text.secondary}>{"No conversations yet for this project."}</Text>
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
                    if resumable && pending.is_none() {
                        AnyView::new(ui! {
                            <Button key={thread.id.0.clone()} label_align={TextAlign::Start}
                                on_press={ChatMessage::SelectThread(thread.id.clone())}
                                background={theme.surfaces.hover} color={theme.text.primary}
                                accessibility_label={title.clone()} fill_width>{label}</Button>
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
        </Column>
    })
}

fn configured_chat_view(
    state: &ChatState,
    settings: &CodexSettings,
    managing_hosts: bool,
    editor: Option<&RemoteHostEditor>,
    settings_error: Option<&str>,
    overlays: ChatOverlays<'_>,
    theme: SemanticTheme,
) -> impl View<ChatMessage> {
    let ChatOverlays {
        pending_shell_command,
        model_picker_generation,
        reasoning_picker_generation,
        approval_picker_generation,
        resume_picker_open,
        resume_picker_loading,
        resume_picker_pending,
        command_picker_open,
        project_root,
        project_id,
    } = overlays;
    let transcript_heights = transcript_heights(state);
    let transcript_offset = if state.conversation_pinned {
        f32::MAX
    } else {
        state.conversation_scroll
    };
    let transcript_window = VirtualWindow::from_heights(
        &transcript_heights,
        TRANSCRIPT_GAP,
        transcript_offset,
        TRANSCRIPT_VIEWPORT_ESTIMATE,
        TRANSCRIPT_OVERSCAN,
    );
    let transcript_range = transcript_window.range.clone();
    let transcript_document = state.transcript_selection_document();
    ui! {
        <Column fill_width fill_height background={theme.surfaces.window}>
            <MenuBar id={id!(menu_bar)}>
                <Menu id={id!(file_menu)} on_toggle={ChatMessage::ToggleFileMenu} label={"File"}>
                    <MenuItem label={"New conversation"} on_press={ChatMessage::NewChat} />
                    <MenuItem label={"Refresh"} on_press={ChatMessage::Refresh} />
                </Menu>
                {connection_menu(settings)}
            </MenuBar>
            {if managing_hosts {
                remote_hosts_panel(settings, editor, settings_error, theme)
            } else { AnyView::new(ui! {
            <Column grow={1.0} min_width={0.0} fill_height padding={Insets::all(18.0)} gap={12.0}>
                <Column id={id!(conversation)} grow={1.0} fill_width gap={10.0}
                    accessibility_label={"Conversation"} semantic_role={SemanticRole::Group}
                    overflow_y={Overflow::Auto} follow_scroll_end={state.conversation_pinned}
                    on_scroll={conversation_scrolled}>
                    {if resume_picker_open {
                        resume_picker(
                            state,
                            project_root,
                            project_id,
                            resume_picker_loading,
                            resume_picker_pending,
                            theme,
                        )
                    } else if state.items.is_empty() {
                        AnyView::new(ui! {
                            <Container grow={1.0} fill_width padding={Insets::all(28.0)}>
                                <Text scale={2.0} color={theme.text.primary}>{"What are we building?"}</Text>
                                <Text color={theme.text.secondary}>{"Start a conversation with Codex. Tool requests always require an explicit decision."}</Text>
                            </Container>
                        })
                    } else {
                        AnyView::new(ui! {
                            {SelectionRegion::new(transcript_document)
                                .id(id!(transcript_selection))
                                .fill_width()
                                .child(VirtualColumn::new()
                                    .window(transcript_window)
                                    .gap(TRANSCRIPT_GAP)
                                    .max_width(1000.0)
                                    .align_self(Align::Center)
                                    .children(state.items.iter().enumerate()
                                        .skip(transcript_range.start)
                                        .take(transcript_range.len())
                                        .map(|(_, item)| ui! { <ItemCard key={item.id.clone()} item={item} theme={theme} /> })))}
                        })
                    }}
                </Column>
                <Container id={id!(composer)} fill_width shrink={0.0}
                    navigation_scope={NavigationScope::group()}>
                <Column fill_width gap={8.0}>
                    {state.pending.iter().map(|interaction| ui! {
                        <InteractionCard interaction={interaction} answer={&state.interaction_answer} theme={theme} />
                    })}
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
                    {state.diagnostics.back().map(|diagnostic| ui! {
                        <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.raised} radius={6.0}>
                            <Text color={theme.text.primary}>{diagnostic}</Text>
                        </Container>
                    })}
                    {state.thread_error.as_ref().map(|diagnostic| ui! {
                        <Row fill_width padding={Insets::all(10.0)} gap={10.0}
                            background={theme.surfaces.raised} radius={6.0}>
                            <Text color={theme.text.primary} grow={1.0}>{format!("Conversations unavailable: {diagnostic}")}</Text>
                            <Button on_press={ChatMessage::Refresh} background={theme.surfaces.hover} color={theme.text.primary}>{"Retry"}</Button>
                        </Row>
                    })}
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
                    <Container id={id!(composer_viewport)} accessibility_label={"Message composer"}
                        semantic_role={SemanticRole::Group}
                        fill_width min_height={52.0} max_height={140.0} shrink={0.0}
                        padding={Insets::all(12.0)} background={theme.surfaces.card}
                        border={Border::new(theme.borders.ordinary, 1.0)} radius={10.0}
                        overflow_y={Overflow::Auto} follow_scroll_end={true}>
                        <TextField id={id!(chat_draft)} value={&state.draft} on_change={draft_changed}
                            color={theme.text.primary} wrap={true} />
                    </Container>
                    <Row shrink={0.0} gap={8.0}>
                        {Menu::new(
                            ChatMessage::ToggleCommandPicker,
                            "Commands",
                            [
                                MenuItem::new("/model — choose model and reasoning", ChatMessage::SelectCommand("/model".into())),
                                MenuItem::new("/resume — resume a project conversation", ChatMessage::SelectCommand("/resume".into())),
                                MenuItem::new("/new — start a new conversation", ChatMessage::SelectCommand("/new".into())),
                                MenuItem::new("/clear — clear into a new conversation", ChatMessage::SelectCommand("/clear".into())),
                                MenuItem::disabled("/review — unavailable: backend support not implemented"),
                                MenuItem::disabled("/compact — unavailable: backend support not implemented"),
                                MenuItem::disabled("/plan — unavailable: use a natural-language request"),
                                MenuItem::disabled("/status — unavailable: status is shown below the composer"),
                                MenuItem::disabled("/diff — unavailable: file changes appear inline"),
                                MenuItem::disabled("/mention — unavailable: file mention chooser not implemented"),
                                MenuItem::disabled("/permissions — unavailable: use approval prompts"),
                                MenuItem::disabled("/feedback — unavailable in this client"),
                                MenuItem::disabled("/logout — unavailable: manage the Codex CLI account externally"),
                            ],
                        ).id(id!(command_picker)).width(460.0).expanded(command_picker_open)
                            .accessibility_label("Commands")
                            .colors(theme.surfaces.card, theme.surfaces.sidebar, theme.text.primary)}
                        <Button on_press={ChatMessage::ToggleResumePicker} background={theme.surfaces.hover} color={theme.text.primary}>{"Resume"}</Button>
                        {Dropdown::new(
                            ChatMessage::ToggleModelPicker,
                            state.models.iter()
                                .find(|model| Some(model.id.as_str()) == state.selected_model.as_deref())
                                .map(|model| model.display_name.clone())
                                .unwrap_or_else(|| "Model".into()),
                            state.models.iter().map(|model| (
                                model.display_name.clone(),
                                ChatMessage::SelectModel(model.id.clone()),
                            )),
                        ).id(id!(model_selector))
                            .accessibility_label("Model selector")
                            .semantic_role(SemanticRole::Button)
                            .overlay(true)
                            .open_generation(model_picker_generation)
                            .colors(theme.surfaces.card, theme.surfaces.sidebar, theme.text.primary)}
                        {state.models.iter()
                            .find(|model| Some(model.id.as_str()) == state.selected_model.as_deref())
                            .filter(|model| !model.supported_reasoning_efforts.is_empty())
                            .map(|model| Dropdown::new(
                                ChatMessage::ToggleReasoningPicker,
                                state.selected_reasoning_effort.clone().unwrap_or_else(|| "Effort".into()),
                                model.supported_reasoning_efforts.iter().map(|option| (
                                    format!("{} — {}", option.reasoning_effort, option.description),
                                    ChatMessage::SelectReasoningEffort(option.reasoning_effort.clone()),
                                )),
                            ).id(id!(reasoning_effort_selector))
                                .accessibility_label("Reasoning effort selector")
                                .semantic_role(SemanticRole::Button)
                                .overlay(true)
                                .open_generation(reasoning_picker_generation)
                                .colors(theme.surfaces.card, theme.surfaces.sidebar, theme.text.primary))}
                        {Dropdown::new(
                            ChatMessage::ToggleApprovalPicker,
                            approval_policy_label(state.selected_approval_policy),
                            APPROVAL_POLICIES.into_iter().map(|policy| (
                                format!("{} — {}", approval_policy_label(policy), approval_policy_description(policy)),
                                ChatMessage::SelectApprovalPolicy(policy),
                            )),
                        ).id(id!(approval_policy_selector))
                            .overlay(true)
                            .open_generation(approval_picker_generation)
                            .colors(theme.surfaces.card, theme.surfaces.sidebar, theme.text.primary)
                            .accessibility_label("Approval policy selector")
                            .accessibility_description(format!(
                                "Effective: {}. This does not change sandbox or filesystem access.",
                                approval_policy_label(state.effective_approval_policy),
                            ))
                            .semantic_role(SemanticRole::Button)}
                        <Column gap={2.0} grow={1.0}>
                            <Text color={theme.text.secondary}>{if state.active_turn.is_some() {
                                "Codex is working…".to_owned()
                            } else if state.selected_approval_policy != state.effective_approval_policy {
                                format!("Applies to the next turn; effective now: {}", approval_policy_label(state.effective_approval_policy))
                            } else {
                                format!("Approval policy: {}", approval_policy_label(state.effective_approval_policy))
                            }}</Text>
                            <Text color={theme.text.secondary} scale={0.72}>{"Approval only; sandbox and filesystem access are unchanged"}</Text>
                            <Text color={theme.text.secondary} scale={0.72}>{&state.provenance}</Text>
                        </Column>
                        <Spacer fill />
                        {if state.interrupt_requested {
                            ui! { <Text color={theme.text.secondary}>{"Interrupting…"}</Text> }
                        } else if state.active_turn.is_some() {
                            ui! { <Button on_press={ChatMessage::Interrupt} background={theme.surfaces.hover} color={theme.text.danger}>{"Interrupt"}</Button> }
                        } else if state.can_send() {
                            ui! { <Button on_press={ChatMessage::Send} background={theme.accent.ordinary} color={theme.accent.on_accent}>{"Send"}</Button> }
                        } else {
                            ui! { <Text color={theme.text.secondary}>{"Enter a message"}</Text> }
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
mod tests {
    use nickel_codex::{ReplayBackend, Thread, ThreadId};
    use nickel_ui::{HostBatch, HostEvent, Rect, UiFrame};
    use nickel_ui_testkit::Scenario;

    use super::*;

    fn alternate_theme() -> SemanticTheme {
        SemanticTheme::from_tokens(nickel_ui::SemanticTokenSet::standard(
            0xf4f6f8, 0xe8edf4, 0xffffff, 0xd6dce5, 0xcbd2dc, 0x171a20, 0x4d5664, 0x075ca8,
            0xc9e5ff, 0x6c3fa0, 0xefe4ff,
        ))
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
            include_str!("../../nickel-shell/src/window_preview.rs")
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
            ui! { <ItemCard item={&item} theme={semantic_theme()} /> },
            Rect::new(0.0, 0.0, 600.0, 400.0),
        );
        assert!(has_accessible_text(&tree, "Heading"));
        assert!(has_accessible_text(&tree, "<b>plain</b>"));
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
            ui! { <ItemCard item={&item} theme={semantic_theme()} /> },
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
}
