// Host input and effect dispatch for the single chat application owner.
use super::{
    CONTROLLER_POLL_MIN, ChatApplication, ChatHostPanel, ChatMessage, ChatOverlays,
    ClipboardWritePurpose, CodexApprovalChoice, PendingNavigation, QUEUED_MESSAGE_CAPACITY,
    QueuedMessagePhase, RemoteHostEditor, ShellRequest, approval_choice_allowed,
    available_slash_commands, configured_chat_view, project_menu_view, project_window_name,
    project_window_title, replace_mention_fragment, safe_diagnostic_summary,
};
use crate::{
    BackendMode, ConnectionStatus, ControllerCommand, ControllerEvent, PendingInteraction,
};
use nickel_codex::{ApprovalPolicy, CommandDecision, FileChangeDecision, SandboxPolicy};
use twinkle_ui::ShortcutOutcome;
use twinkle_ui::prelude::*;

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
            Shortcut::Escape if self.project_menu_mode => {
                self.shell_requests.push(ShellRequest::CloseProjectMenu);
                true
            }
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

    fn view(&self, context: twinkle_ui::ViewContext) -> impl View<Self::Message> {
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
        _context: twinkle_ui::ViewContext,
    ) -> Vec<twinkle_ui::FrameOverlay<Self::Message>> {
        if self.pending_navigation.is_none() {
            return Vec::new();
        }
        // The host owns dialog focus trapping, hit testing, and focus return.
        let dialog = TransientSurface::dialog(
            "codex-discard-draft",
            OverlayAnchor::InvocationTargetCenter(UiId::from("root/menu-bar/file-menu")),
            twinkle_ui::Size::new(420.0, 164.0),
            OverlayStyle::from_theme(&self.theme),
        )
        .accessible_name("Discard unsent Codex input?")
        .dismiss(DismissPolicy {
            cancel: false,
            outside_pointer: false,
            action: true,
        });
        vec![twinkle_ui::FrameOverlay::surface(
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
