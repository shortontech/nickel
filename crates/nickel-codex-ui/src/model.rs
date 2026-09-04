use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use nickel_codex::{
    AccountState, ApprovalPolicy, CodexEvent, CommandAction, EventKind, Model, Project,
    ServerRequestId, Thread, ThreadId, TurnId,
};
use nickel_markdown::{MarkdownDocument, markdown_selection_runs};
use nickel_ui::{SelectionDocument, SelectionRun};

use crate::ControllerEvent;
use crate::{AttachmentError, AttachmentId, AttachmentLimits, PendingAttachment};

const MAX_ITEMS: usize = 2_000;
const MAX_ITEM_ALIASES: usize = 2_000;
const MAX_DIAGNOSTICS: usize = 100;
const MAX_THREADS: usize = 200;
const MAX_PENDING: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    Loading,
    Ready,
    Unavailable,
    Disconnected,
    Incompatible,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatItemKind {
    User,
    Agent,
    Reasoning,
    Command,
    Activity,
    FileChange,
    Plan,
    Error,
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatItem {
    pub id: String,
    pub kind: ChatItemKind,
    pub text: String,
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingInteraction {
    Approval {
        request_id: ServerRequestId,
        approval_type: String,
        summary: String,
    },
    UserInput {
        request_id: ServerRequestId,
        question_ids: Vec<String>,
    },
}

#[derive(Clone, Debug)]
pub struct ChatState {
    pub generation: u64,
    pub status: ConnectionStatus,
    pub provenance: String,
    pub account: AccountState,
    pub models: Vec<Model>,
    pub selected_model: Option<String>,
    pub selected_reasoning_effort: Option<String>,
    pub effective_approval_policy: ApprovalPolicy,
    pub selected_approval_policy: ApprovalPolicy,
    pub projects: Vec<Project>,
    pub threads: Vec<Thread>,
    pub thread_runtime: HashMap<ThreadId, nickel_codex::ThreadRuntime>,
    pub thread_error: Option<String>,
    pub thread_snapshot_available: bool,
    pub selected_thread: Option<ThreadId>,
    pub active_turn: Option<TurnId>,
    pub interrupt_requested: bool,
    pub items: VecDeque<ChatItem>,
    pub draft: String,
    pub attachments: Vec<PendingAttachment>,
    next_attachment_id: u64,
    send_pending: bool,
    pub interaction_answer: String,
    pub pending: Vec<PendingInteraction>,
    pub diagnostics: VecDeque<String>,
    pub conversation_scroll: f32,
    pub conversation_pinned: bool,
    pub expanded_projects: HashSet<String>,
    pub collapsed_projects: HashSet<String>,
    local_sequence: u64,
    /// Backend item IDs that intentionally route into a differently named merged transcript item.
    /// Ordinary item IDs are resolved directly from `items` instead of mirrored in an index.
    item_aliases: VecDeque<(String, String)>,
    turn_agent_index: Option<usize>,
    exploration_index: Option<usize>,
    exploration_item_ids: HashSet<String>,
    exploration_reads: HashSet<String>,
    exploration_lists: HashSet<String>,
    exploration_searches: HashSet<String>,
    item_selection_runs: VecDeque<Vec<SelectionRun>>,
    selection_revision: u64,
    selection_document_cache: RefCell<(u64, usize, Arc<SelectionDocument>)>,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            generation: 1,
            status: ConnectionStatus::Loading,
            provenance: "Locating OpenAI Codex CLI…".into(),
            account: AccountState::default(),
            models: Vec::new(),
            selected_model: None,
            selected_reasoning_effort: None,
            effective_approval_policy: ApprovalPolicy::default(),
            selected_approval_policy: ApprovalPolicy::default(),
            projects: Vec::new(),
            threads: Vec::new(),
            thread_runtime: HashMap::new(),
            thread_error: None,
            thread_snapshot_available: false,
            selected_thread: None,
            active_turn: None,
            interrupt_requested: false,
            items: VecDeque::new(),
            draft: String::new(),
            attachments: Vec::new(),
            next_attachment_id: 1,
            send_pending: false,
            interaction_answer: String::new(),
            pending: Vec::new(),
            diagnostics: VecDeque::new(),
            conversation_scroll: 0.0,
            conversation_pinned: true,
            expanded_projects: HashSet::new(),
            collapsed_projects: HashSet::new(),
            local_sequence: 0,
            item_aliases: VecDeque::new(),
            turn_agent_index: None,
            exploration_index: None,
            exploration_item_ids: HashSet::new(),
            exploration_reads: HashSet::new(),
            exploration_lists: HashSet::new(),
            exploration_searches: HashSet::new(),
            item_selection_runs: VecDeque::new(),
            selection_revision: 0,
            selection_document_cache: RefCell::new((0, 0, Arc::new(SelectionDocument::default()))),
        }
    }
}

impl ChatState {
    fn reconcile_selected_model(&mut self) {
        if self.models.is_empty() {
            return;
        }
        let unavailable = self
            .selected_model
            .as_ref()
            .is_some_and(|id| !self.models.iter().any(|candidate| candidate.id == *id));
        if self.selected_model.is_none() || unavailable {
            self.selected_model = self.models.first().map(|model| model.id.clone());
            self.selected_reasoning_effort = self
                .models
                .first()
                .and_then(|model| model.default_reasoning_effort.clone());
        }
        if unavailable {
            self.report_diagnostic(
                "The selected model is no longer available; using the first available model",
            );
        }
    }

    pub fn can_send(&self) -> bool {
        self.status == ConnectionStatus::Ready
            && self.active_turn.is_none()
            && !self.send_pending
            && (!self.draft.trim().is_empty() || !self.attachments.is_empty())
    }

    pub fn begin_send(&mut self) -> Option<(String, Vec<nickel_codex::TurnImage>)> {
        if !self.can_send() {
            return None;
        }
        let text = self.draft.clone();
        self.send_pending = true;
        self.local_sequence += 1;
        self.push_item(ChatItem {
            id: format!("local-user-{}", self.local_sequence),
            kind: ChatItemKind::User,
            text: text.clone(),
            complete: true,
        });
        let images = self
            .attachments
            .iter()
            .map(PendingAttachment::turn_image)
            .collect();
        Some((text, images))
    }

    pub fn attach_image(&mut self, bytes: &[u8]) -> Result<AttachmentId, AttachmentError> {
        self.attach_image_with_limits(bytes, AttachmentLimits::default())
    }

    fn attach_image_with_limits(
        &mut self,
        bytes: &[u8],
        limits: AttachmentLimits,
    ) -> Result<AttachmentId, AttachmentError> {
        if self.attachments.len() >= limits.count {
            return Err(AttachmentError::TooMany);
        }
        let retained = self
            .attachments
            .iter()
            .map(PendingAttachment::retained_bytes)
            .sum::<usize>();
        let id = AttachmentId(self.next_attachment_id);
        let attachment = PendingAttachment::decode(id, bytes, limits)?;
        if retained.saturating_add(attachment.retained_bytes()) > limits.aggregate_decoded_bytes {
            return Err(AttachmentError::AggregateLimit);
        }
        self.next_attachment_id = self.next_attachment_id.saturating_add(1);
        self.attachments.push(attachment);
        Ok(id)
    }

    pub fn attach_rgba(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<AttachmentId, AttachmentError> {
        self.attach_rgba_with_limits(width, height, rgba, AttachmentLimits::default())
    }

    fn attach_rgba_with_limits(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
        limits: AttachmentLimits,
    ) -> Result<AttachmentId, AttachmentError> {
        if self.attachments.len() >= limits.count {
            return Err(AttachmentError::TooMany);
        }
        let used = self
            .attachments
            .iter()
            .map(PendingAttachment::retained_bytes)
            .sum::<usize>();
        let id = AttachmentId(self.next_attachment_id);
        let attachment = PendingAttachment::from_rgba(id, width, height, rgba, limits)?;
        if used.saturating_add(attachment.retained_bytes()) > limits.aggregate_decoded_bytes {
            return Err(AttachmentError::AggregateLimit);
        }
        self.next_attachment_id = self.next_attachment_id.saturating_add(1);
        self.attachments.push(attachment);
        Ok(id)
    }

    pub fn remove_attachment(&mut self, id: AttachmentId) -> bool {
        let before = self.attachments.len();
        self.attachments.retain(|attachment| attachment.id != id);
        before != self.attachments.len()
    }

    pub fn apply(&mut self, generation: u64, event: ControllerEvent) -> bool {
        if generation != self.generation {
            return false;
        }
        match event {
            ControllerEvent::Ready {
                provenance,
                account,
                models,
                projects,
                threads,
                runtime,
                thread_error,
            } => {
                self.status = ConnectionStatus::Ready;
                self.provenance = provenance;
                self.account = account;
                self.models = models.into_iter().take(100).collect();
                self.reconcile_selected_model();
                if self.selected_reasoning_effort.is_none() {
                    self.selected_reasoning_effort = self
                        .models
                        .iter()
                        .find(|model| Some(model.id.as_str()) == self.selected_model.as_deref())
                        .and_then(|model| model.default_reasoning_effort.clone());
                }
                self.projects = projects.into_iter().take(100).collect();
                let mut seen = HashSet::new();
                self.threads = threads
                    .into_iter()
                    .filter(|thread| seen.insert(thread.id.clone()))
                    .collect();
                self.threads.sort_by(|left, right| {
                    right
                        .last_used_at
                        .cmp(&left.last_used_at)
                        .then_with(|| left.id.0.cmp(&right.id.0))
                });
                self.threads.truncate(MAX_THREADS);
                let retained = self
                    .threads
                    .iter()
                    .map(|thread| thread.id.clone())
                    .collect::<HashSet<_>>();
                self.thread_runtime = runtime
                    .into_iter()
                    .filter(|(id, _)| retained.contains(id))
                    .collect();
                self.thread_snapshot_available = thread_error.is_none();
                self.thread_error = thread_error.map(|message| sanitize_diagnostic(&message));
            }
            ControllerEvent::ThreadCreated(thread) => {
                self.record_selected_thread(thread);
            }
            ControllerEvent::ThreadSelected(thread) => {
                self.attachments.clear();
                self.send_pending = false;
                self.hydrate_thread(&thread);
                self.reconcile_selected_model();
                self.record_selected_thread(thread);
            }
            ControllerEvent::TurnAccepted => {
                if self.send_pending {
                    self.draft.clear();
                    self.attachments.clear();
                    self.send_pending = false;
                }
            }
            ControllerEvent::ApprovalPolicyAccepted(policy) => {
                self.effective_approval_policy = policy;
            }
            ControllerEvent::Protocol(event) => self.apply_protocol(event),
            ControllerEvent::Incompatible(message) => {
                self.status = ConnectionStatus::Incompatible;
                self.push_diagnostic(message);
                self.active_turn = None;
                self.interrupt_requested = false;
            }
            ControllerEvent::Unavailable(message) => {
                self.status = ConnectionStatus::Unavailable;
                self.projects.clear();
                self.threads.clear();
                self.thread_runtime.clear();
                self.push_diagnostic(message);
                self.active_turn = None;
                self.interrupt_requested = false;
            }
            ControllerEvent::OperationFailed(message) => {
                self.send_pending = false;
                self.push_diagnostic(message);
            }
            ControllerEvent::Failure(message) => {
                self.status = ConnectionStatus::Disconnected;
                self.push_diagnostic(message);
                self.active_turn = None;
                self.interrupt_requested = false;
            }
        }
        true
    }

    pub fn new_chat(&mut self) {
        self.selected_thread = None;
        self.clear_conversation();
    }

    pub fn begin_thread_selection(&mut self, id: ThreadId) {
        self.selected_thread = Some(id);
        self.clear_conversation();
    }

    fn clear_conversation(&mut self) {
        self.attachments.clear();
        self.send_pending = false;
        self.active_turn = None;
        self.interrupt_requested = false;
        self.items.clear();
        self.item_selection_runs.clear();
        self.invalidate_selection_projection();
        self.item_aliases.clear();
        self.turn_agent_index = None;
        self.clear_exploration();
        self.pending.clear();
        self.diagnostics.clear();
        self.interaction_answer.clear();
        self.conversation_scroll = 0.0;
        self.conversation_pinned = true;
    }

    fn hydrate_thread(&mut self, thread: &Thread) {
        if thread.model.is_some() {
            self.selected_model.clone_from(&thread.model);
        }
        if thread.reasoning_effort.is_some() {
            self.selected_reasoning_effort
                .clone_from(&thread.reasoning_effort);
        }
        self.items.clear();
        self.item_selection_runs.clear();
        self.invalidate_selection_projection();
        self.item_aliases.clear();
        self.pending.clear();
        self.active_turn = None;
        self.interrupt_requested = false;
        self.conversation_scroll = 0.0;
        self.conversation_pinned = true;
        for turn in &thread.turns {
            self.clear_exploration();
            let mut turn_agent_index = None;
            for item in &turn.items {
                let kind = chat_item_kind(&item.item_type);
                if kind == ChatItemKind::Command
                    && !item.command_actions.is_empty()
                    && item
                        .command_actions
                        .iter()
                        .all(|action| !matches!(action, CommandAction::Unknown))
                {
                    self.upsert_exploration(&item.id, &item.command_actions);
                } else if kind == ChatItemKind::Agent && turn_agent_index.is_some() {
                    let index = turn_agent_index.expect("checked above");
                    if !item.text.is_empty() {
                        if !self.items[index].text.is_empty() {
                            self.items[index].text.push_str("\n\n");
                        }
                        self.items[index].text.push_str(&item.text);
                    }
                    self.register_item_alias(item.id.clone(), index);
                    self.refresh_item_projection(index);
                } else {
                    self.push_item(ChatItem {
                        id: item.id.clone(),
                        kind: kind.clone(),
                        text: item.text.clone(),
                        complete: true,
                    });
                    if kind == ChatItemKind::Agent {
                        turn_agent_index = Some(self.items.len() - 1);
                    }
                }
            }
            if let Some(index) = self.exploration_index {
                self.items[index].complete = true;
                self.refresh_exploration_text();
            }
            if turn.status == "inProgress" {
                self.active_turn = Some(turn.id.clone());
            }
        }
    }

    fn apply_protocol(&mut self, event: CodexEvent) {
        match event.kind {
            EventKind::Connection { state } if state == "failed" => {
                self.status = ConnectionStatus::Disconnected;
                self.active_turn = None;
                self.interrupt_requested = false;
            }
            EventKind::TurnStarted { turn_id, .. } => {
                if self.send_pending {
                    self.draft.clear();
                    self.attachments.clear();
                    self.send_pending = false;
                }
                self.clear_exploration();
                self.turn_agent_index = None;
                self.active_turn = Some(turn_id);
                self.interrupt_requested = false;
            }
            EventKind::TurnCompleted { .. } => {
                if let Some(index) = self.exploration_index {
                    self.items[index].complete = true;
                    self.refresh_exploration_text();
                }
                self.active_turn = None;
                self.turn_agent_index = None;
                self.interrupt_requested = false;
            }
            EventKind::ItemStarted {
                item_id,
                item_type,
                turn_id,
                command_actions,
                initial_text,
                ..
            } => {
                let kind = chat_item_kind(&item_type);
                if kind == ChatItemKind::Command
                    && !command_actions.is_empty()
                    && command_actions
                        .iter()
                        .all(|action| !matches!(action, CommandAction::Unknown))
                {
                    self.upsert_exploration(&item_id, &command_actions);
                    return;
                }
                if kind != ChatItemKind::User || !self.reconcile_optimistic_user(&item_id) {
                    let merge_agent_update = kind == ChatItemKind::Agent
                        && turn_id.is_some()
                        && turn_id.as_ref() == self.active_turn.as_ref()
                        && self
                            .turn_agent_index
                            .is_some_and(|index| self.items[index].complete);
                    if merge_agent_update {
                        let index = self.turn_agent_index.expect("checked above");
                        self.items[index].text.push_str("\n\n");
                        self.items[index].complete = false;
                        self.register_item_alias(item_id, index);
                        self.refresh_item_projection(index);
                    } else {
                        self.push_item(ChatItem {
                            id: item_id,
                            kind: kind.clone(),
                            text: initial_text,
                            complete: false,
                        });
                        if kind == ChatItemKind::Agent {
                            self.turn_agent_index = Some(self.items.len() - 1);
                        }
                    }
                }
            }
            EventKind::ItemCompleted { item_id } => {
                if self.exploration_item_ids.contains(&item_id) {
                    return;
                }
                if let Some(index) = self.resolve_item_index(&item_id) {
                    if self.items[index].text.is_empty() {
                        self.reconcile_selection_runs();
                        self.items.remove(index);
                        self.item_selection_runs.remove(index);
                        self.invalidate_selection_projection();
                        self.reconcile_item_aliases();
                    } else {
                        self.items[index].complete = true;
                    }
                }
            }
            EventKind::AgentMessageDelta { item_id, delta } => {
                self.append_delta(item_id, delta, ChatItemKind::Agent)
            }
            EventKind::CommandOutputDelta { item_id, delta } => {
                if !self.exploration_item_ids.contains(&item_id) {
                    self.append_delta(item_id, delta, ChatItemKind::Command)
                }
            }
            EventKind::FileChangeDelta { item_id, delta } => {
                self.append_delta(item_id, delta, ChatItemKind::FileChange)
            }
            EventKind::PlanDelta { item_id, delta } => {
                self.append_delta(item_id, delta, ChatItemKind::Plan)
            }
            EventKind::ReasoningDelta { item_id, delta } => {
                self.append_delta(item_id, delta, ChatItemKind::Reasoning)
            }
            EventKind::ApprovalRequested {
                request_id,
                approval_type,
                summary,
            } => {
                self.push_pending(PendingInteraction::Approval {
                    request_id,
                    approval_type,
                    summary: summary.unwrap_or_else(|| "Codex requests approval".into()),
                });
            }
            EventKind::UserInputRequested {
                request_id,
                question_ids,
            } => self.push_pending(PendingInteraction::UserInput {
                request_id,
                question_ids,
            }),
            EventKind::Error { message } => self.push_diagnostic(message),
            EventKind::Inconsistency { message }
                if message.starts_with("delta for unknown item ")
                    || message.starts_with("completion for unknown item ") => {}
            EventKind::Inconsistency { message } => self.push_diagnostic(message),
            EventKind::UnsupportedEvent { .. } => {}
            _ => {}
        }
    }

    fn clear_exploration(&mut self) {
        self.exploration_index = None;
        self.exploration_item_ids.clear();
        self.exploration_reads.clear();
        self.exploration_lists.clear();
        self.exploration_searches.clear();
    }

    fn upsert_exploration(&mut self, item_id: &str, actions: &[CommandAction]) {
        for action in actions {
            match action {
                CommandAction::Read { name, path } => {
                    self.exploration_reads.insert(if path.is_empty() {
                        name.clone()
                    } else {
                        path.clone()
                    });
                }
                CommandAction::ListFiles { path } => {
                    self.exploration_lists
                        .insert(path.clone().unwrap_or_else(|| ".".into()));
                }
                CommandAction::Search { query, path } => {
                    self.exploration_searches.insert(format!(
                        "{}\u{0}{}",
                        query.as_deref().unwrap_or_default(),
                        path.as_deref().unwrap_or_default()
                    ));
                }
                CommandAction::Unknown => {}
            }
        }
        self.exploration_item_ids.insert(item_id.to_owned());
        let index = if let Some(index) = self.exploration_index {
            index
        } else {
            self.push_item(ChatItem {
                id: item_id.to_owned(),
                kind: ChatItemKind::Activity,
                text: String::new(),
                complete: false,
            });
            let index = self.items.len() - 1;
            self.exploration_index = Some(index);
            index
        };
        self.register_item_alias(item_id.to_owned(), index);
        self.refresh_exploration_text();
    }

    fn refresh_exploration_text(&mut self) {
        let Some(index) = self.exploration_index else {
            return;
        };
        let mut lines = vec![if self.items[index].complete {
            "Explored".to_owned()
        } else {
            "Exploring".to_owned()
        }];
        let count_line = |verb: &str, count: usize, noun: &str| {
            format!("{verb} {count} {noun}{}", if count == 1 { "" } else { "s" })
        };
        if !self.exploration_reads.is_empty() {
            lines.push(count_line("Read", self.exploration_reads.len(), "file"));
        }
        if !self.exploration_lists.is_empty() {
            lines.push(count_line(
                "Listed",
                self.exploration_lists.len(),
                "location",
            ));
        }
        if !self.exploration_searches.is_empty() {
            lines.push(count_line(
                "Searched",
                self.exploration_searches.len(),
                "query",
            ));
        }
        self.items[index].text = lines.join("\n");
        self.refresh_item_projection(index);
    }

    fn push_item(&mut self, item: ChatItem) {
        self.reconcile_selection_runs();
        if self.items.len() == MAX_ITEMS {
            let removed = self
                .items
                .pop_front()
                .expect("bounded transcript is non-empty");
            self.item_selection_runs.pop_front();
            self.item_aliases
                .retain(|(_, canonical_id)| canonical_id != &removed.id);
        }
        self.item_selection_runs
            .push_back(selection_runs_for_item(&item));
        self.invalidate_selection_projection();
        self.items.push_back(item);
    }

    fn refresh_item_projection(&mut self, index: usize) {
        self.item_selection_runs[index] = selection_runs_for_item(&self.items[index]);
        self.invalidate_selection_projection();
    }

    fn record_selected_thread(&mut self, thread: Thread) {
        self.selected_thread = Some(thread.id.clone());
        if !self.threads.iter().any(|known| known.id == thread.id) {
            self.threads.insert(0, thread);
            self.threads.truncate(MAX_THREADS);
        }
    }

    fn reconcile_optimistic_user(&mut self, item_id: &str) -> bool {
        let Some(index) = self.items.iter().rposition(|item| {
            item.kind == ChatItemKind::User && item.id.starts_with("local-user-")
        }) else {
            return false;
        };
        self.items[index].id = item_id.to_owned();
        self.items[index].complete = false;
        self.reconcile_selection_runs();
        self.item_selection_runs[index] = selection_runs_for_item(&self.items[index]);
        self.invalidate_selection_projection();
        self.reconcile_item_aliases();
        true
    }

    fn push_pending(&mut self, interaction: PendingInteraction) {
        if self.pending.len() == MAX_PENDING {
            self.pending.remove(0);
            self.push_diagnostic("Pending interaction limit reached".into());
        }
        self.pending.push(interaction);
    }

    fn append_delta(&mut self, item_id: String, delta: String, inferred_kind: ChatItemKind) {
        if let Some(index) = self.resolve_item_index(&item_id) {
            self.reconcile_selection_runs();
            self.items[index].text.push_str(&delta);
            self.item_selection_runs[index] = selection_runs_for_item(&self.items[index]);
            self.invalidate_selection_projection();
        } else {
            self.push_item(ChatItem {
                id: item_id,
                kind: inferred_kind,
                text: delta,
                complete: false,
            });
        }
    }

    fn resolve_item_index(&self, item_id: &str) -> Option<usize> {
        self.items
            .iter()
            .position(|item| item.id == item_id)
            .or_else(|| {
                let canonical_id = self
                    .item_aliases
                    .iter()
                    .rev()
                    .find_map(|(alias, canonical)| (alias == item_id).then_some(canonical))?;
                self.items.iter().position(|item| &item.id == canonical_id)
            })
    }

    fn register_item_alias(&mut self, item_id: String, index: usize) {
        let canonical_id = &self.items[index].id;
        if &item_id != canonical_id {
            if let Some((_, target)) = self
                .item_aliases
                .iter_mut()
                .find(|(alias, _)| alias == &item_id)
            {
                target.clone_from(canonical_id);
                return;
            }
            if self.item_aliases.len() == MAX_ITEM_ALIASES {
                self.item_aliases.pop_front();
            }
            self.item_aliases.push_back((item_id, canonical_id.clone()));
        }
    }

    fn reconcile_item_aliases(&mut self) {
        self.item_aliases
            .retain(|(_, canonical_id)| self.items.iter().any(|item| &item.id == canonical_id));
    }

    fn push_diagnostic(&mut self, message: String) {
        if self.diagnostics.len() == MAX_DIAGNOSTICS {
            self.diagnostics.pop_front();
        }
        self.diagnostics.push_back(sanitize_diagnostic(&message));
    }

    pub(crate) fn report_diagnostic(&mut self, message: impl Into<String>) {
        self.push_diagnostic(message.into());
    }

    fn reconcile_selection_runs(&mut self) {
        if self.item_selection_runs.len() != self.items.len() {
            self.item_selection_runs = self.items.iter().map(selection_runs_for_item).collect();
            self.invalidate_selection_projection();
        }
    }

    /// Drops the state's reference to the previous transcript-sized projection. A frame that is
    /// still being presented may keep its `Arc`, but mutations do not retain a second stale copy.
    fn invalidate_selection_projection(&mut self) {
        self.selection_revision = self.selection_revision.wrapping_add(1);
        *self.selection_document_cache.get_mut() = (0, 0, Arc::new(SelectionDocument::default()));
    }

    pub fn estimated_item_heights(&self) -> Vec<f32> {
        self.items.iter().map(estimate_item_height).collect()
    }

    pub fn transcript_selection_document(&self) -> Arc<SelectionDocument> {
        {
            let cache = self.selection_document_cache.borrow();
            if cache.0 == self.selection_revision && cache.1 == self.items.len() {
                return cache.2.clone();
            }
        }
        let runs = if self.item_selection_runs.len() == self.items.len() {
            self.item_selection_runs
                .iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>()
        } else {
            self.items
                .iter()
                .flat_map(selection_runs_for_item)
                .collect()
        };
        let document = Arc::new(SelectionDocument::new(runs));
        *self.selection_document_cache.borrow_mut() =
            (self.selection_revision, self.items.len(), document.clone());
        document
    }
}

pub(crate) fn item_markdown_source(item: &ChatItem) -> &str {
    if item.text.is_empty() {
        if item.complete { "—" } else { "…" }
    } else {
        item.text.as_str()
    }
}

pub(crate) fn item_markdown_document(item: &ChatItem) -> MarkdownDocument {
    MarkdownDocument::parse(item_markdown_source(item))
}

fn selection_runs_for_item(item: &ChatItem) -> Vec<SelectionRun> {
    let mut runs = vec![SelectionRun::block(
        format!("{}/label", item.id),
        item_label(&item.kind),
    )];
    let document = item_markdown_document(item);
    runs.extend(markdown_selection_runs(
        &document,
        &format!("{}/body", item.id),
    ));
    runs
}

pub(crate) fn item_label(kind: &ChatItemKind) -> &'static str {
    match kind {
        ChatItemKind::User => "You",
        ChatItemKind::Agent => "Codex",
        ChatItemKind::Reasoning => "Reasoning summary",
        ChatItemKind::Command => "Command",
        ChatItemKind::Activity => "Codex",
        ChatItemKind::FileChange => "File change",
        ChatItemKind::Plan => "Plan",
        ChatItemKind::Error => "Error",
        ChatItemKind::Unknown(_) => "Additional event",
    }
}

fn estimate_item_height(item: &ChatItem) -> f32 {
    let characters_per_line = if item.kind == ChatItemKind::User {
        78
    } else {
        98
    };
    let lines = item
        .text
        .lines()
        .map(|line| line.chars().count().max(1).div_ceil(characters_per_line))
        .sum::<usize>()
        .max(1);
    58.0 + lines as f32 * 21.0
}

fn chat_item_kind(item_type: &str) -> ChatItemKind {
    match item_type {
        "userMessage" => ChatItemKind::User,
        "agentMessage" => ChatItemKind::Agent,
        "reasoning" => ChatItemKind::Reasoning,
        "commandExecution" => ChatItemKind::Command,
        "fileChange" => ChatItemKind::FileChange,
        "plan" => ChatItemKind::Plan,
        "error" => ChatItemKind::Error,
        other => ChatItemKind::Unknown(other.into()),
    }
}

fn sanitize_diagnostic(message: &str) -> String {
    let lower = message.to_ascii_lowercase();
    if [
        "authorization",
        "bearer ",
        "access_token",
        "refresh_token",
        "cookie",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || message.contains("/home/")
        || message.contains("\\Users\\")
    {
        return "Sensitive backend diagnostic redacted".into();
    }
    message.chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use std::{hint::black_box, mem::size_of, time::Instant};

    use super::*;

    const TINY_DERIVED_OPERATION_P95_ADDITION: std::time::Duration =
        std::time::Duration::from_micros(100);

    #[test]
    fn failed_send_retains_unicode_draft_and_images_until_turn_is_accepted() {
        let mut state = ChatState {
            status: ConnectionStatus::Ready,
            draft: "hello 世界".into(),
            ..ChatState::default()
        };
        state.attach_rgba(1, 1, &[1, 2, 3, 255]).unwrap();
        let (text, images) = state.begin_send().unwrap();
        assert_eq!(text, "hello 世界");
        assert_eq!(images.len(), 1);
        assert!(!state.can_send());
        state.apply(1, ControllerEvent::OperationFailed("offline".into()));
        assert_eq!(state.draft, "hello 世界");
        assert_eq!(state.attachments.len(), 1);
        assert!(state.can_send());

        state.begin_send().unwrap();
        state.apply(
            1,
            ControllerEvent::Protocol(CodexEvent {
                sequence: 1,
                kind: EventKind::TurnStarted {
                    thread_id: ThreadId("t".into()),
                    turn_id: TurnId("turn".into()),
                },
            }),
        );
        assert!(state.draft.is_empty());
        assert!(state.attachments.is_empty());
    }

    #[test]
    fn attachment_admission_is_count_and_total_resident_memory_bounded() {
        let mut state = ChatState::default();
        let tiny_count = AttachmentLimits {
            count: 1,
            ..AttachmentLimits::default()
        };
        state
            .attach_rgba_with_limits(1, 1, &[1, 2, 3, 255], tiny_count)
            .unwrap();
        assert_eq!(
            state
                .attach_rgba_with_limits(1, 1, &[1, 2, 3, 255], tiny_count)
                .unwrap_err(),
            AttachmentError::TooMany
        );

        let first = state.attachments[0].id;
        assert!(state.remove_attachment(first));
        assert!(state.attachments.is_empty());
        let tiny_memory = AttachmentLimits {
            aggregate_decoded_bytes: 4,
            ..AttachmentLimits::default()
        };
        assert_eq!(
            state
                .attach_rgba_with_limits(1, 1, &[1, 2, 3, 255], tiny_memory)
                .unwrap_err(),
            AttachmentError::AggregateLimit
        );
        assert!(state.attachments.is_empty());
    }

    fn representative_long_transcript() -> ChatState {
        let mut state = ChatState::default();
        for index in 0..MAX_ITEMS {
            state.push_item(ChatItem {
                id: format!("item-{index}"),
                kind: if index % 4 == 0 {
                    ChatItemKind::User
                } else {
                    ChatItemKind::Agent
                },
                text: format!(
                    "## Transcript item {index}\n\nThis is representative prose with **formatting**, \
                     a [link](https://example.invalid/{index}), and enough content to exercise \
                     Markdown selection projection.\n\n- first result\n- second result\n\n```text\n\
                     deterministic output {index}\n```"
                ),
                complete: true,
            });
        }
        state
    }

    fn p95(samples: &mut [std::time::Duration]) -> std::time::Duration {
        samples.sort_unstable();
        samples[samples.len() * 95 / 100]
    }

    fn selection_run_retained_bytes(state: &ChatState) -> usize {
        state
            .item_selection_runs
            .iter()
            .flatten()
            .map(|run| size_of::<SelectionRun>() + run.id.capacity() + run.text.len())
            .sum()
    }

    fn item_index_retained_bytes(index: &HashMap<String, usize>) -> usize {
        index.capacity() * (size_of::<String>() + size_of::<usize>() + size_of::<usize>())
            + index.keys().map(|key| key.capacity()).sum::<usize>()
    }

    #[test]
    #[ignore = "release-profile cache admission measurement; run explicitly"]
    fn recomputing_2k_item_heights_is_within_tiny_operation_budget() {
        let items = (0..MAX_ITEMS)
            .map(|index| ChatItem {
                id: format!("item-{index}"),
                kind: if index % 3 == 0 {
                    ChatItemKind::User
                } else {
                    ChatItemKind::Agent
                },
                text: format!(
                    "Measured transcript item {index}: deterministic text spanning a representative chat line."
                ),
                complete: true,
            })
            .collect::<Vec<_>>();
        let cached = items.iter().map(estimate_item_height).collect::<Vec<_>>();
        let recomputed = items.iter().map(estimate_item_height).collect::<Vec<_>>();
        assert_eq!(cached, recomputed, "cached and recomputed heights differ");

        let mut cached_samples = Vec::with_capacity(200);
        let mut recomputed_samples = Vec::with_capacity(200);
        for _ in 0..200 {
            let start = Instant::now();
            let cached_result = black_box(&cached).to_vec();
            black_box(cached_result);
            cached_samples.push(start.elapsed());

            let start = Instant::now();
            let recomputed_result = black_box(&items)
                .iter()
                .map(estimate_item_height)
                .collect::<Vec<_>>();
            black_box(recomputed_result);
            recomputed_samples.push(start.elapsed());
        }
        cached_samples.sort_unstable();
        recomputed_samples.sort_unstable();
        let p95_index = cached_samples.len() * 95 / 100;
        let cached_p95 = cached_samples[p95_index];
        let recomputed_p95 = recomputed_samples[p95_index];
        let addition = recomputed_p95.saturating_sub(cached_p95);
        eprintln!(
            "2k item heights: cached_p95={cached_p95:?} recomputed_p95={recomputed_p95:?} addition={addition:?}"
        );
        assert!(
            addition <= TINY_DERIVED_OPERATION_P95_ADDITION,
            "recomputation added {addition:?}, exceeding the predeclared {:?} p95 budget",
            TINY_DERIVED_OPERATION_P95_ADDITION
        );
    }

    #[test]
    fn selection_projections_are_bounded_released_and_equivalent() {
        let mut state = representative_long_transcript();
        for index in MAX_ITEMS..MAX_ITEMS + 20 {
            state.push_item(ChatItem {
                id: format!("item-{index}"),
                kind: ChatItemKind::Agent,
                text: format!("replacement {index}"),
                complete: true,
            });
        }

        assert_eq!(state.items.len(), MAX_ITEMS);
        assert!(state.item_aliases.is_empty());
        assert_eq!(state.item_selection_runs.len(), MAX_ITEMS);
        assert_eq!(state.resolve_item_index("item-0"), None);
        assert_eq!(state.resolve_item_index("item-2019"), Some(MAX_ITEMS - 1));

        let cached = state.transcript_selection_document();
        let recomputed =
            SelectionDocument::new(state.items.iter().flat_map(selection_runs_for_item));
        assert_eq!(&*cached, &recomputed);

        state.clear_conversation();
        assert!(state.items.is_empty());
        assert!(state.item_aliases.is_empty());
        assert!(state.item_selection_runs.is_empty());
        assert!(state.selection_document_cache.borrow().2.runs().is_empty());
    }

    #[test]
    fn transcript_mutation_drops_stale_cached_document() {
        let mut state = representative_long_transcript();
        let stale = state.transcript_selection_document();
        assert!(!stale.runs().is_empty());
        assert_eq!(Arc::strong_count(&stale), 2);

        state.append_delta("item-1999".into(), " tail".into(), ChatItemKind::Agent);

        assert!(state.selection_document_cache.borrow().2.runs().is_empty());
        assert_eq!(Arc::strong_count(&stale), 1);
        let current = state.transcript_selection_document();
        assert_ne!(&*current, &*stale);
    }

    #[test]
    fn merged_item_alias_routing_is_bounded_and_reconciled() {
        let mut state = ChatState::default();
        state.push_item(ChatItem {
            id: "canonical".into(),
            kind: ChatItemKind::Agent,
            text: "first".into(),
            complete: true,
        });
        for index in 0..MAX_ITEM_ALIASES + 20 {
            state.register_item_alias(format!("alias-{index}"), 0);
        }
        assert_eq!(state.item_aliases.len(), MAX_ITEM_ALIASES);
        assert_eq!(state.resolve_item_index("alias-0"), None);
        assert_eq!(
            state.resolve_item_index(&format!("alias-{}", MAX_ITEM_ALIASES + 19)),
            Some(0)
        );

        state.clear_conversation();
        assert!(state.item_aliases.is_empty());
    }

    #[test]
    #[ignore = "release-profile cache admission measurement; run explicitly"]
    fn long_transcript_cache_admission_measurement() {
        let state = representative_long_transcript();
        let cached_document = state.transcript_selection_document();
        let expected_runs = cached_document.runs().len();
        let cached_index = state
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| (item.id.clone(), index))
            .collect::<HashMap<_, _>>();
        let index_bytes = item_index_retained_bytes(&cached_index);
        let selection_run_bytes = selection_run_retained_bytes(&state);
        let document_bytes = expected_runs
            * (size_of::<SelectionRun>() + size_of::<String>() + size_of::<usize>() * 2)
            + cached_document
                .runs()
                .iter()
                .map(|run| run.id.capacity() * 2)
                .sum::<usize>();

        let mut indexed = Vec::with_capacity(400);
        let mut linear = Vec::with_capacity(400);
        for sample in 0..400 {
            let id = format!("item-{}", (sample * 1543) % MAX_ITEMS);
            let start = Instant::now();
            black_box(cached_index.get(black_box(&id)).copied());
            indexed.push(start.elapsed());
            let start = Instant::now();
            black_box(state.items.iter().position(|item| item.id == id));
            linear.push(start.elapsed());
        }

        let mut cached_runs = Vec::with_capacity(40);
        let mut rebuilt_runs = Vec::with_capacity(40);
        for _ in 0..40 {
            let start = Instant::now();
            let runs = state
                .item_selection_runs
                .iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(runs.len(), expected_runs);
            black_box(runs);
            cached_runs.push(start.elapsed());

            let start = Instant::now();
            let runs = state
                .items
                .iter()
                .flat_map(selection_runs_for_item)
                .collect::<Vec<_>>();
            assert_eq!(runs.len(), expected_runs);
            black_box(runs);
            rebuilt_runs.push(start.elapsed());
        }

        let mut cached_documents = Vec::with_capacity(100);
        let mut rebuilt_documents = Vec::with_capacity(100);
        for _ in 0..100 {
            let start = Instant::now();
            black_box(state.transcript_selection_document());
            cached_documents.push(start.elapsed());

            let start = Instant::now();
            let document =
                SelectionDocument::new(state.item_selection_runs.iter().flatten().cloned());
            assert_eq!(document.runs().len(), expected_runs);
            black_box(document);
            rebuilt_documents.push(start.elapsed());
        }

        let indexed_p95 = p95(&mut indexed);
        let linear_p95 = p95(&mut linear);
        let cached_runs_p95 = p95(&mut cached_runs);
        let rebuilt_runs_p95 = p95(&mut rebuilt_runs);
        let cached_document_p95 = p95(&mut cached_documents);
        let rebuilt_document_p95 = p95(&mut rebuilt_documents);
        eprintln!(
            "long transcript ({MAX_ITEMS} items, {expected_runs} runs): \
             item_index cached_p95={indexed_p95:?} linear_p95={linear_p95:?} retained={index_bytes}B; \
             selection_runs cached_p95={cached_runs_p95:?} rebuilt_p95={rebuilt_runs_p95:?} retained={selection_run_bytes}B; \
             selection_document cached_p95={cached_document_p95:?} rebuilt_p95={rebuilt_document_p95:?} retained={document_bytes}B"
        );

        assert!(linear_p95.saturating_sub(indexed_p95) <= TINY_DERIVED_OPERATION_P95_ADDITION);
        assert!(
            rebuilt_runs_p95.saturating_sub(cached_runs_p95) > TINY_DERIVED_OPERATION_P95_ADDITION
        );
        assert!(
            rebuilt_document_p95.saturating_sub(cached_document_p95)
                > TINY_DERIVED_OPERATION_P95_ADDITION
        );
    }
}
