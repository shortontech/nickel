use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use nickel_codex::{
    AccountState, CodexEvent, EventKind, Model, Project, ServerRequestId, Thread, ThreadId, TurnId,
};
use nickel_markdown::{MarkdownDocument, markdown_selection_runs};
use nickel_ui::{SelectionDocument, SelectionRun};

use crate::ControllerEvent;

const MAX_ITEMS: usize = 2_000;
const MAX_DIAGNOSTICS: usize = 100;
const MAX_THREADS: usize = 200;
const MAX_PENDING: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    Loading,
    Ready,
    Disconnected,
    Incompatible,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatItemKind {
    User,
    Agent,
    Reasoning,
    Command,
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
    pub projects: Vec<Project>,
    pub threads: Vec<Thread>,
    pub thread_runtime: HashMap<ThreadId, nickel_codex::ThreadRuntime>,
    pub thread_error: Option<String>,
    pub selected_thread: Option<ThreadId>,
    pub active_turn: Option<TurnId>,
    pub interrupt_requested: bool,
    pub items: VecDeque<ChatItem>,
    pub draft: String,
    pub interaction_answer: String,
    pub pending: Vec<PendingInteraction>,
    pub diagnostics: VecDeque<String>,
    pub conversation_scroll: f32,
    pub conversation_pinned: bool,
    pub expanded_projects: HashSet<String>,
    pub collapsed_projects: HashSet<String>,
    local_sequence: u64,
    item_indexes: HashMap<String, usize>,
    item_height_estimates: VecDeque<f32>,
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
            projects: Vec::new(),
            threads: Vec::new(),
            thread_runtime: HashMap::new(),
            thread_error: None,
            selected_thread: None,
            active_turn: None,
            interrupt_requested: false,
            items: VecDeque::new(),
            draft: String::new(),
            interaction_answer: String::new(),
            pending: Vec::new(),
            diagnostics: VecDeque::new(),
            conversation_scroll: 0.0,
            conversation_pinned: true,
            expanded_projects: HashSet::new(),
            collapsed_projects: HashSet::new(),
            local_sequence: 0,
            item_indexes: HashMap::new(),
            item_height_estimates: VecDeque::new(),
            item_selection_runs: VecDeque::new(),
            selection_revision: 0,
            selection_document_cache: RefCell::new((0, 0, Arc::new(SelectionDocument::default()))),
        }
    }
}

impl ChatState {
    pub fn can_send(&self) -> bool {
        self.status == ConnectionStatus::Ready
            && self.active_turn.is_none()
            && !self.draft.trim().is_empty()
    }

    pub fn begin_send(&mut self) -> Option<String> {
        if !self.can_send() {
            return None;
        }
        let text = std::mem::take(&mut self.draft);
        self.local_sequence += 1;
        self.push_item(ChatItem {
            id: format!("local-user-{}", self.local_sequence),
            kind: ChatItemKind::User,
            text: text.clone(),
            complete: true,
        });
        Some(text)
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
                self.projects = projects.into_iter().take(100).collect();
                self.threads = threads.into_iter().take(MAX_THREADS).collect();
                self.thread_runtime = runtime;
                self.thread_error = thread_error.map(|message| sanitize_diagnostic(&message));
            }
            ControllerEvent::ThreadCreated(thread) => {
                self.record_selected_thread(thread);
            }
            ControllerEvent::ThreadSelected(thread) => {
                self.hydrate_thread(&thread);
                self.record_selected_thread(thread);
            }
            ControllerEvent::Protocol(event) => self.apply_protocol(event),
            ControllerEvent::Incompatible(message) => {
                self.status = ConnectionStatus::Incompatible;
                self.push_diagnostic(message);
                self.active_turn = None;
                self.interrupt_requested = false;
            }
            ControllerEvent::OperationFailed(message) => self.push_diagnostic(message),
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
        self.active_turn = None;
        self.interrupt_requested = false;
        self.items.clear();
        self.item_height_estimates.clear();
        self.item_selection_runs.clear();
        self.selection_revision = self.selection_revision.wrapping_add(1);
        self.item_indexes.clear();
        self.pending.clear();
        self.diagnostics.clear();
        self.interaction_answer.clear();
        self.conversation_scroll = 0.0;
        self.conversation_pinned = true;
    }

    fn hydrate_thread(&mut self, thread: &Thread) {
        self.items.clear();
        self.item_height_estimates.clear();
        self.item_selection_runs.clear();
        self.selection_revision = self.selection_revision.wrapping_add(1);
        self.item_indexes.clear();
        self.pending.clear();
        self.active_turn = None;
        self.interrupt_requested = false;
        self.conversation_scroll = 0.0;
        self.conversation_pinned = true;
        for turn in &thread.turns {
            for item in &turn.items {
                self.push_item(ChatItem {
                    id: item.id.clone(),
                    kind: chat_item_kind(&item.item_type),
                    text: item.text.clone(),
                    complete: true,
                });
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
                self.active_turn = Some(turn_id);
                self.interrupt_requested = false;
            }
            EventKind::TurnCompleted { .. } => {
                self.active_turn = None;
                self.interrupt_requested = false;
            }
            EventKind::ItemStarted {
                item_id, item_type, ..
            } => {
                let kind = chat_item_kind(&item_type);
                if kind != ChatItemKind::User || !self.reconcile_optimistic_user(&item_id) {
                    self.push_item(ChatItem {
                        id: item_id,
                        kind,
                        text: String::new(),
                        complete: false,
                    });
                }
            }
            EventKind::ItemCompleted { item_id } => {
                if let Some(index) = self.item_indexes.get(&item_id).copied() {
                    if self.items[index].text.is_empty() {
                        self.reconcile_height_estimates();
                        self.items.remove(index);
                        self.item_height_estimates.remove(index);
                        self.item_selection_runs.remove(index);
                        self.selection_revision = self.selection_revision.wrapping_add(1);
                        self.reindex();
                    } else {
                        self.items[index].complete = true;
                    }
                }
            }
            EventKind::AgentMessageDelta { item_id, delta } => {
                self.append_delta(item_id, delta, ChatItemKind::Agent)
            }
            EventKind::CommandOutputDelta { item_id, delta } => {
                self.append_delta(item_id, delta, ChatItemKind::Command)
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

    fn push_item(&mut self, item: ChatItem) {
        self.reconcile_height_estimates();
        if self.items.len() == MAX_ITEMS {
            self.items.pop_front();
            self.item_height_estimates.pop_front();
            self.item_selection_runs.pop_front();
        }
        self.item_height_estimates
            .push_back(estimate_item_height(&item));
        self.item_selection_runs
            .push_back(selection_runs_for_item(&item));
        self.selection_revision = self.selection_revision.wrapping_add(1);
        self.items.push_back(item);
        self.reindex();
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
        self.reconcile_height_estimates();
        self.item_selection_runs[index] = selection_runs_for_item(&self.items[index]);
        self.selection_revision = self.selection_revision.wrapping_add(1);
        self.reindex();
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
        if let Some(index) = self.item_indexes.get(&item_id).copied() {
            self.reconcile_height_estimates();
            self.items[index].text.push_str(&delta);
            self.item_height_estimates[index] = estimate_item_height(&self.items[index]);
            self.item_selection_runs[index] = selection_runs_for_item(&self.items[index]);
            self.selection_revision = self.selection_revision.wrapping_add(1);
        } else {
            self.push_item(ChatItem {
                id: item_id,
                kind: inferred_kind,
                text: delta,
                complete: false,
            });
        }
    }

    fn reindex(&mut self) {
        self.item_indexes = self
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| (item.id.clone(), index))
            .collect();
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

    fn reconcile_height_estimates(&mut self) {
        if self.item_height_estimates.len() != self.items.len() {
            self.item_height_estimates = self.items.iter().map(estimate_item_height).collect();
        }
        if self.item_selection_runs.len() != self.items.len() {
            self.item_selection_runs = self.items.iter().map(selection_runs_for_item).collect();
            self.selection_revision = self.selection_revision.wrapping_add(1);
        }
    }

    pub fn estimated_item_heights(&self) -> Vec<f32> {
        if self.item_height_estimates.len() == self.items.len() {
            self.item_height_estimates.iter().copied().collect()
        } else {
            self.items.iter().map(estimate_item_height).collect()
        }
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
