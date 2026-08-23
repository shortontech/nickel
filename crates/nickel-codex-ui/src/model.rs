use std::collections::{HashMap, VecDeque};

use nickel_codex::{
    AccountState, CodexEvent, EventKind, Model, ServerRequestId, Thread, ThreadId, TurnId,
};

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
    pub threads: Vec<Thread>,
    pub selected_thread: Option<ThreadId>,
    pub active_turn: Option<TurnId>,
    pub interrupt_requested: bool,
    pub items: VecDeque<ChatItem>,
    pub draft: String,
    pub interaction_answer: String,
    pub pending: Vec<PendingInteraction>,
    pub diagnostics: VecDeque<String>,
    local_sequence: u64,
    item_indexes: HashMap<String, usize>,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            generation: 1,
            status: ConnectionStatus::Loading,
            provenance: "Selecting Codex…".into(),
            account: AccountState::default(),
            models: Vec::new(),
            threads: Vec::new(),
            selected_thread: None,
            active_turn: None,
            interrupt_requested: false,
            items: VecDeque::new(),
            draft: String::new(),
            interaction_answer: String::new(),
            pending: Vec::new(),
            diagnostics: VecDeque::new(),
            local_sequence: 0,
            item_indexes: HashMap::new(),
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
                threads,
            } => {
                self.status = ConnectionStatus::Ready;
                self.provenance = provenance;
                self.account = account;
                self.models = models.into_iter().take(100).collect();
                self.threads = threads.into_iter().take(MAX_THREADS).collect();
            }
            ControllerEvent::ThreadSelected(thread) => {
                self.hydrate_thread(&thread);
                self.selected_thread = Some(thread.id.clone());
                if !self.threads.iter().any(|known| known.id == thread.id) {
                    self.threads.insert(0, thread);
                    self.threads.truncate(MAX_THREADS);
                }
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
        self.active_turn = None;
        self.interrupt_requested = false;
        self.items.clear();
        self.item_indexes.clear();
        self.pending.clear();
    }

    fn hydrate_thread(&mut self, thread: &Thread) {
        self.items.clear();
        self.item_indexes.clear();
        self.pending.clear();
        self.active_turn = None;
        self.interrupt_requested = false;
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
                self.push_item(ChatItem {
                    id: item_id,
                    kind,
                    text: String::new(),
                    complete: false,
                });
            }
            EventKind::ItemCompleted { item_id } => {
                if let Some(item) = self.item_mut(&item_id) {
                    item.complete = true;
                } else {
                    self.push_item(ChatItem {
                        id: item_id,
                        kind: ChatItemKind::Unknown("completed item".into()),
                        text: String::new(),
                        complete: true,
                    });
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
        if self.items.len() == MAX_ITEMS {
            self.items.pop_front();
        }
        self.items.push_back(item);
        self.reindex();
    }

    fn push_pending(&mut self, interaction: PendingInteraction) {
        if self.pending.len() == MAX_PENDING {
            self.pending.remove(0);
            self.push_diagnostic("Pending interaction limit reached".into());
        }
        self.pending.push(interaction);
    }

    fn append_delta(&mut self, item_id: String, delta: String, inferred_kind: ChatItemKind) {
        if let Some(item) = self.item_mut(&item_id) {
            item.text.push_str(&delta);
        } else {
            self.push_item(ChatItem {
                id: item_id,
                kind: inferred_kind,
                text: delta,
                complete: false,
            });
        }
    }

    fn item_mut(&mut self, id: &str) -> Option<&mut ChatItem> {
        let index = *self.item_indexes.get(id)?;
        self.items.get_mut(index)
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
