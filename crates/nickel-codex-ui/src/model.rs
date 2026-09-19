use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use nickel_codex::{
    AccountState, ApprovalPolicy, CodexEvent, CommandAction, EventKind, FilePatchChange,
    FileSearchMatch, LoginChallenge, Model, Project, RateLimitsStatus, RemoteControlClient,
    RemoteControlStatus, RemotePairingChallenge, SandboxPolicy, ServerRequestId, Thread, ThreadId,
    TurnId,
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
pub const MAX_ITEM_TEXT_BYTES: usize = 256 * 1024;
pub const MAX_TRANSCRIPT_TEXT_BYTES: usize = 8 * 1024 * 1024;
const OMISSION_MARKER: &str = "\n\n[Further output omitted from this local view (256 KiB limit); server history is unchanged.]";

fn structured_file_change_summary(changes: &[FilePatchChange]) -> Option<String> {
    match changes {
        [] => None,
        [change] => {
            // Only typed patch fields may name a target in collapsed UI. A
            // free-form diff/delta line is not evidence of a file operation.
            let kind = change.kind.lines().next()?.trim();
            let path = change.path.lines().next()?.trim();
            if kind.is_empty() || path.is_empty() {
                return None;
            }
            Some(format!(
                "{}: {}",
                kind.chars().take(32).collect::<String>(),
                path.chars().take(180).collect::<String>()
            ))
        }
        many => Some(format!("{} files changed", many.len())),
    }
}

fn bound_text(text: &mut String) {
    if text.len() > MAX_ITEM_TEXT_BYTES {
        let mut end = MAX_ITEM_TEXT_BYTES - OMISSION_MARKER.len();
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(OMISSION_MARKER);
    }
    if text.capacity() > MAX_ITEM_TEXT_BYTES {
        text.shrink_to_fit();
    }
}

fn append_bounded(text: &mut String, delta: &str) {
    if text.len() >= MAX_ITEM_TEXT_BYTES - 3 && text.ends_with(OMISSION_MARKER) {
        return;
    }
    let mut end = delta
        .len()
        .min(MAX_ITEM_TEXT_BYTES.saturating_sub(text.len()) + 1);
    while !delta.is_char_boundary(end) {
        end -= 1;
    }
    text.push_str(&delta[..end]);
    if end < delta.len() && text.len() <= MAX_ITEM_TEXT_BYTES {
        text.push_str(OMISSION_MARKER);
    }
    bound_text(text);
}

#[derive(Clone, Debug, Default)]
struct ItemProjection {
    generation: u64,
    cached: RefCell<Option<Arc<ItemSnapshot>>>,
    builds: BuildCounter,
}

#[derive(Clone, Debug, Default)]
struct BuildCounter(Arc<AtomicU64>);

impl BuildCounter {
    #[cfg(test)]
    fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Debug)]
struct ItemSnapshot {
    generation: u64,
    item: ChatItem,
    derived: OnceLock<DerivedItem>,
    builds: BuildCounter,
}

#[derive(Clone, Debug)]
struct DerivedItem {
    document: Arc<MarkdownDocument>,
    runs: Vec<SelectionRun>,
}

impl ItemProjection {
    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        *self.cached.get_mut() = None;
    }

    fn snapshot(&self, item: &ChatItem) -> Arc<ItemSnapshot> {
        if self
            .cached
            .borrow()
            .as_ref()
            .is_some_and(|cached| cached.generation == self.generation)
        {
            return self.cached.borrow().as_ref().unwrap().clone();
        }
        let snapshot = Arc::new(ItemSnapshot {
            generation: self.generation,
            item: item.clone(),
            derived: OnceLock::new(),
            builds: self.builds.clone(),
        });
        *self.cached.borrow_mut() = Some(snapshot.clone());
        snapshot
    }
}

impl std::ops::Deref for ItemSnapshot {
    type Target = DerivedItem;

    fn deref(&self) -> &DerivedItem {
        self.derived.get_or_init(|| {
            let item = &self.item;
            let mut document = Arc::new(item_markdown_document(item));
            let mut runs = selection_runs_from_document(item, &document);
            // Tie derived expansion to the aggregate source budget, including run identifiers.
            let budget = 2048 + 16 * item.text.capacity() + 4 * item.id.capacity();
            let selection_budget = 2048 + 8 * item.text.capacity() + 4 * item.id.capacity();
            if crate::projection_memory::derived_capacity(&document, &runs) > budget
                || crate::projection_memory::selection_capacity(&runs) > selection_budget
            {
                let source = format!(
                    "{}\n\n[Formatting omitted from this local view to limit memory use.]",
                    item_markdown_source(item)
                );
                document = Arc::new(MarkdownDocument {
                    source: source.clone(),
                    blocks: vec![nickel_markdown::Block::Paragraph {
                        inlines: vec![nickel_markdown::Inline::Text { text: source }],
                    }],
                    diagnostics: Vec::new(),
                });
                runs = selection_runs_from_document(item, &document);
            }
            self.builds.0.fetch_add(1, Ordering::Relaxed);
            DerivedItem { document, runs }
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    Loading,
    Ready,
    Unavailable,
    Disconnected,
    Incompatible,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunPresentationStatus {
    Connecting,
    Recovering,
    RecoveryFailed,
    Unconfirmed,
    AuthenticationRequired,
    Ready,
    Starting,
    Working,
    Interrupting,
    AwaitingApproval,
    AwaitingInput,
    Retrying,
    TurnFailed,
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
    Tool,
    Search,
    Image,
    Delegation,
    ApprovalReference,
    QuestionReference,
    SessionNotice,
    Plan,
    Warning,
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

/// Bounded, source-owned terminal facts kept separately from transcript prose.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ActivityOutcome {
    pub status: Option<String>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
}

impl ActivityOutcome {
    fn from_source(
        status: Option<&str>,
        exit_code: Option<i32>,
        duration_ms: Option<i64>,
    ) -> Option<Self> {
        let outcome = Self {
            status: status
                .filter(|value| !value.is_empty())
                .map(|value| value.chars().take(64).collect()),
            exit_code,
            duration_ms: duration_ms.filter(|value| *value >= 0),
        };
        (outcome.status.is_some() || outcome.exit_code.is_some() || outcome.duration_ms.is_some())
            .then_some(outcome)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingInteraction {
    Approval {
        request_id: ServerRequestId,
        approval_type: String,
        summary: String,
        context: nickel_codex::ApprovalContext,
    },
    UserInput {
        request_id: ServerRequestId,
        question_ids: Vec<String>,
        questions: Vec<nickel_codex::UserInputQuestion>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoginPresentationState {
    #[default]
    Idle,
    Starting,
    Waiting,
    Cancelling,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug)]
pub struct ChatState {
    pub generation: u64,
    pub status: ConnectionStatus,
    pub provenance: String,
    pub backend_source: Option<nickel_codex::CandidateSource>,
    pub fallback_reason: Option<String>,
    pub account: AccountState,
    pub rate_limits: Option<RateLimitsStatus>,
    pub rate_limits_error: Option<String>,
    pub rate_limits_pending: bool,
    pub login_challenge: Option<LoginChallenge>,
    pub login_qr: Option<Arc<image::RgbaImage>>,
    pub login_pending: bool,
    pub login_status: LoginPresentationState,
    pub remote_control_status: Option<RemoteControlStatus>,
    pub remote_pairing: Option<RemotePairingChallenge>,
    pub remote_pairing_qr: Option<Arc<image::RgbaImage>>,
    pub remote_clients: Vec<RemoteControlClient>,
    pub remote_control_pending: bool,
    pub remote_control_message: Option<String>,
    pub models: Vec<Model>,
    pub selected_model: Option<String>,
    pub selected_reasoning_effort: Option<String>,
    pub effective_approval_policy: ApprovalPolicy,
    pub selected_approval_policy: ApprovalPolicy,
    pub effective_sandbox_policy: Option<SandboxPolicy>,
    pub selected_sandbox_policy: Option<SandboxPolicy>,
    pub projects: Vec<Project>,
    pub threads: Vec<Thread>,
    pub thread_runtime: HashMap<ThreadId, nickel_codex::ThreadRuntime>,
    pub thread_error: Option<String>,
    pub thread_snapshot_available: bool,
    pub thread_next_cursor: Option<String>,
    pub thread_paging: bool,
    thread_page_request: u64,
    pending_thread_page_request: Option<u64>,
    pub thread_page_error: Option<String>,
    pub thread_windowed: bool,
    pub selected_thread: Option<ThreadId>,
    /// Blocks new actions until reconnect has reloaded the selected thread.
    pub recovery_pending: bool,
    pub recovery_failed: bool,
    /// The old controller could not confirm an in-flight turn or request.
    pub unconfirmed_work: bool,
    pub active_turn: Option<TurnId>,
    pub last_turn_error: Option<(TurnId, String, bool)>,
    /// One transcript error block per backend turn, updated as retry/failure changes.
    turn_error_item_ids: HashMap<TurnId, String>,
    pub interrupt_requested: bool,
    pub items: VecDeque<ChatItem>,
    pub expanded_items: HashSet<String>,
    pub draft: String,
    pub command_feedback: Option<String>,
    pub file_search_query: Option<String>,
    pub file_search_matches: Vec<FileSearchMatch>,
    pub file_search_pending: bool,
    pub file_search_error: Option<String>,
    pub attachments: Vec<PendingAttachment>,
    next_attachment_id: u64,
    send_pending: bool,
    pub interaction_answer: String,
    pub pending: Vec<PendingInteraction>,
    pending_thread_ids: HashMap<ServerRequestId, ThreadId>,
    pending_revisions: HashMap<ServerRequestId, u64>,
    /// Shell delivery failure is scoped to one authoritative request revision.
    notification_unavailable: HashMap<ServerRequestId, u64>,
    /// Transcript references share request identity with the sole pending decision owner.
    interaction_item_ids: HashMap<ServerRequestId, String>,
    next_pending_revision: u64,
    submitting_requests: HashSet<ServerRequestId>,
    unconfirmed_responses: HashSet<ServerRequestId>,
    pub diagnostics: VecDeque<String>,
    unsupported_method_counts: HashMap<String, u32>,
    pub conversation_scroll: f32,
    pub conversation_viewport_height: f32,
    pub conversation_pinned: bool,
    pub new_content_while_unpinned: bool,
    pub expanded_projects: HashSet<String>,
    pub collapsed_projects: HashSet<String>,
    local_sequence: u64,
    /// Backend item IDs that intentionally route into a differently named merged transcript item.
    /// Ordinary item IDs are resolved directly from `items` instead of mirrored in an index.
    item_aliases: VecDeque<(String, String)>,
    merged_agent_starts: HashMap<String, usize>,
    completed_item_ids: VecDeque<String>,
    retired_item_ids: VecDeque<String>,
    pub(crate) activity_outcomes: HashMap<String, ActivityOutcome>,
    /// Source-derived labels are retained only for live transcript items.
    pub(crate) file_change_summaries: HashMap<String, String>,
    turn_agent_index: Option<usize>,
    exploration_index: Option<usize>,
    exploration_item_ids: HashSet<String>,
    exploration_reads: HashSet<String>,
    exploration_lists: HashSet<String>,
    exploration_searches: HashSet<String>,
    item_selection_runs: VecDeque<ItemProjection>,
    selection_revision: u64,
    selection_document_cache: RefCell<(u64, usize, Arc<SelectionDocument>)>,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            generation: 1,
            status: ConnectionStatus::Loading,
            provenance: "Locating OpenAI Codex CLI…".into(),
            backend_source: None,
            fallback_reason: None,
            account: AccountState::default(),
            rate_limits: None,
            rate_limits_error: None,
            rate_limits_pending: false,
            login_challenge: None,
            login_qr: None,
            login_pending: false,
            login_status: LoginPresentationState::Idle,
            remote_control_status: None,
            remote_pairing: None,
            remote_pairing_qr: None,
            remote_clients: Vec::new(),
            remote_control_pending: false,
            remote_control_message: None,
            models: Vec::new(),
            selected_model: None,
            selected_reasoning_effort: None,
            effective_approval_policy: ApprovalPolicy::default(),
            selected_approval_policy: ApprovalPolicy::default(),
            effective_sandbox_policy: None,
            selected_sandbox_policy: None,
            projects: Vec::new(),
            threads: Vec::new(),
            thread_runtime: HashMap::new(),
            thread_error: None,
            thread_snapshot_available: false,
            thread_next_cursor: None,
            thread_paging: false,
            thread_page_request: 0,
            pending_thread_page_request: None,
            thread_page_error: None,
            thread_windowed: false,
            selected_thread: None,
            recovery_pending: false,
            recovery_failed: false,
            unconfirmed_work: false,
            active_turn: None,
            last_turn_error: None,
            turn_error_item_ids: HashMap::new(),
            interrupt_requested: false,
            items: VecDeque::new(),
            expanded_items: HashSet::new(),
            draft: String::new(),
            command_feedback: None,
            file_search_query: None,
            file_search_matches: Vec::new(),
            file_search_pending: false,
            file_search_error: None,
            attachments: Vec::new(),
            next_attachment_id: 1,
            send_pending: false,
            interaction_answer: String::new(),
            pending: Vec::new(),
            pending_thread_ids: HashMap::new(),
            pending_revisions: HashMap::new(),
            notification_unavailable: HashMap::new(),
            interaction_item_ids: HashMap::new(),
            next_pending_revision: 0,
            submitting_requests: HashSet::new(),
            unconfirmed_responses: HashSet::new(),
            diagnostics: VecDeque::new(),
            unsupported_method_counts: HashMap::new(),
            conversation_scroll: 0.0,
            conversation_viewport_height: 600.0,
            conversation_pinned: true,
            new_content_while_unpinned: false,
            expanded_projects: HashSet::new(),
            collapsed_projects: HashSet::new(),
            local_sequence: 0,
            item_aliases: VecDeque::new(),
            merged_agent_starts: HashMap::new(),
            completed_item_ids: VecDeque::new(),
            retired_item_ids: VecDeque::new(),
            activity_outcomes: HashMap::new(),
            file_change_summaries: HashMap::new(),
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
            && self.account.authenticated
            && !self.recovery_pending
            && !self.unconfirmed_work
            && self.active_turn.is_none()
            && !self.send_pending
            && (!self.draft.trim().is_empty() || !self.attachments.is_empty())
    }

    pub fn replacement_block_reason(&self) -> Option<&'static str> {
        // Replacing a conversation must not implicitly interrupt a run or abandon authority.
        if self.recovery_pending {
            Some("Wait for conversation recovery, or reconnect if reloading failed.")
        } else if self.send_pending {
            Some("Wait for the new turn to start before replacing this conversation.")
        } else if self.active_turn.is_some() {
            Some("Interrupt or finish the active turn before replacing this conversation.")
        } else if !self.pending.is_empty() {
            Some("Resolve the outstanding Codex request before replacing this conversation.")
        } else {
            None
        }
    }

    pub fn begin_thread_page(&mut self) -> Option<(String, u64)> {
        if self.thread_paging || !self.thread_snapshot_available {
            return None;
        }
        let cursor = self.thread_next_cursor.clone()?;
        self.thread_page_request = self.thread_page_request.wrapping_add(1);
        let request = self.thread_page_request;
        self.pending_thread_page_request = Some(request);
        self.thread_paging = true;
        self.thread_page_error = None;
        Some((cursor, request))
    }

    pub fn run_presentation_status(&self) -> RunPresentationStatus {
        match self.status {
            ConnectionStatus::Loading => return RunPresentationStatus::Connecting,
            ConnectionStatus::Unavailable => return RunPresentationStatus::Unavailable,
            ConnectionStatus::Disconnected => return RunPresentationStatus::Disconnected,
            ConnectionStatus::Incompatible => return RunPresentationStatus::Incompatible,
            ConnectionStatus::Ready => {}
        }
        if self.recovery_failed {
            return RunPresentationStatus::RecoveryFailed;
        }
        if self.recovery_pending {
            return RunPresentationStatus::Recovering;
        }
        if self.unconfirmed_work {
            return RunPresentationStatus::Unconfirmed;
        }
        if !self.account.authenticated {
            return RunPresentationStatus::AuthenticationRequired;
        }
        if self
            .pending
            .iter()
            .any(|interaction| matches!(interaction, PendingInteraction::Approval { .. }))
        {
            return RunPresentationStatus::AwaitingApproval;
        }
        if self
            .pending
            .iter()
            .any(|interaction| matches!(interaction, PendingInteraction::UserInput { .. }))
        {
            return RunPresentationStatus::AwaitingInput;
        }
        if self.interrupt_requested {
            RunPresentationStatus::Interrupting
        } else if self
            .last_turn_error
            .as_ref()
            .is_some_and(|(_, _, retry)| *retry)
        {
            RunPresentationStatus::Retrying
        } else if self.active_turn.is_some() {
            RunPresentationStatus::Working
        } else if self.send_pending {
            RunPresentationStatus::Starting
        } else if self.last_turn_error.is_some() {
            RunPresentationStatus::TurnFailed
        } else {
            RunPresentationStatus::Ready
        }
    }

    pub fn run_secondary_status(&self) -> Option<&'static str> {
        (self.unconfirmed_work && (self.status != ConnectionStatus::Ready || self.recovery_pending))
            .then_some("Earlier turn or request outcome is unconfirmed until recovery completes.")
    }

    pub(crate) fn mark_unconfirmed_on_connection_loss(&mut self) {
        self.unconfirmed_work |= self.active_turn.is_some()
            || self.send_pending
            || !self.pending.is_empty()
            || !self.unconfirmed_responses.is_empty();
    }

    pub fn interaction_is_actionable(&self, request_id: &ServerRequestId) -> bool {
        self.status == ConnectionStatus::Ready
            && self.account.authenticated
            && !self.recovery_pending
            && !self.unconfirmed_work
            && !self.submitting_requests.contains(request_id)
            && self.pending.iter().any(|interaction| match interaction {
                PendingInteraction::Approval {
                    request_id: pending,
                    ..
                }
                | PendingInteraction::UserInput {
                    request_id: pending,
                    ..
                } => pending == request_id,
            })
    }

    pub fn interaction_submission_pending(&self, request_id: &ServerRequestId) -> bool {
        self.submitting_requests.contains(request_id)
    }

    pub fn interaction_response_unconfirmed(&self, request_id: &ServerRequestId) -> bool {
        self.unconfirmed_responses.contains(request_id)
    }

    pub fn interaction_thread_id(&self, request_id: &ServerRequestId) -> Option<&ThreadId> {
        self.pending_thread_ids.get(request_id)
    }

    pub fn interaction_revision(&self, request_id: &ServerRequestId) -> Option<u64> {
        self.pending_revisions.get(request_id).copied()
    }

    pub fn approval_notification_unavailable(&self, request_id: &ServerRequestId) -> bool {
        self.notification_unavailable.get(request_id) == self.pending_revisions.get(request_id)
            && self.pending_revisions.contains_key(request_id)
    }

    pub fn mark_approval_notification_unavailable(&mut self, request_id: &ServerRequestId) {
        if let Some(revision) = self.interaction_revision(request_id) {
            self.notification_unavailable
                .insert(request_id.clone(), revision);
        }
    }

    pub fn clear_approval_notification_unavailable(&mut self, request_id: &ServerRequestId) {
        self.notification_unavailable.remove(request_id);
    }

    pub fn begin_approval_response(
        &mut self,
        request_id: &ServerRequestId,
        approval_type: &str,
    ) -> bool {
        if !matches!(
            approval_type,
            "item/fileChange/requestApproval" | "item/commandExecution/requestApproval"
        ) || !self.interaction_is_actionable(request_id)
            || !self.pending.iter().any(|interaction| {
                matches!(interaction,
                    PendingInteraction::Approval { request_id: pending, approval_type: kind, .. }
                    if pending == request_id && kind == approval_type
                )
            })
        {
            return false;
        }
        self.submitting_requests.insert(request_id.clone())
    }

    pub fn begin_input_response(
        &mut self,
        request_id: &ServerRequestId,
        question_ids: &[String],
    ) -> bool {
        if !self.interaction_is_actionable(request_id) || !self.pending.iter().any(|interaction| {
            matches!(interaction,
                PendingInteraction::UserInput { request_id: pending, question_ids: expected, .. }
                if pending == request_id && expected == question_ids
            )
        }) {
            return false;
        }
        self.submitting_requests.insert(request_id.clone())
    }

    pub fn invalidate_pending_for_reconnect(&mut self) -> usize {
        let count = self.pending.len();
        // The old connection no longer owns these decisions. Keep their transcript
        // chronology, but do not direct the user to a request panel that was retired.
        let references = self
            .interaction_item_ids
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for item_id in references {
            if let Some(index) = self.resolve_item_index(&item_id) {
                self.items[index].complete = true;
                append_bounded(
                    &mut self.items[index].text,
                    "\nRequest state unconfirmed after reconnect; wait for recovery.",
                );
                self.refresh_item_projection(index);
            }
        }
        self.pending.clear();
        self.pending_thread_ids.clear();
        self.pending_revisions.clear();
        self.notification_unavailable.clear();
        self.interaction_item_ids.clear();
        self.submitting_requests.clear();
        self.unconfirmed_responses.clear();
        count
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
        let valid_metadata = match &event {
            ControllerEvent::ThreadSelected(thread) => thread
                .turns
                .iter()
                .flat_map(|turn| &turn.items)
                .all(|item| item.id.len() <= 4096 && item.item_type.len() <= 4096),
            ControllerEvent::Protocol(event) => match &event.kind {
                EventKind::ItemStarted {
                    item_id, item_type, ..
                } => item_id.len() <= 4096 && item_type.len() <= 4096,
                EventKind::AgentMessageDelta { item_id, .. }
                | EventKind::CommandOutputDelta { item_id, .. }
                | EventKind::FileChangeDelta { item_id, .. }
                | EventKind::PlanDelta { item_id, .. }
                | EventKind::ReasoningDelta { item_id, .. } => item_id.len() <= 4096,
                EventKind::FilePatchUpdated {
                    thread_id,
                    turn_id,
                    item_id,
                    ..
                } => thread_id.0.len() <= 4096 && turn_id.0.len() <= 4096 && item_id.len() <= 4096,
                EventKind::TurnPlanUpdated {
                    thread_id, turn_id, ..
                } => thread_id.0.len() <= 4096 && turn_id.0.len() <= 4096,
                EventKind::ApprovalRequested {
                    request_id,
                    thread_id,
                    approval_type,
                    summary,
                    context,
                } => {
                    request_id.0.len() <= 4096
                        && thread_id.as_ref().is_none_or(|id| id.0.len() <= 4096)
                        && approval_type.len() <= 256
                        && summary.as_ref().is_none_or(|value| value.len() <= 4096)
                        && [
                            &context.item_id,
                            &context.approval_id,
                            &context.command,
                            &context.cwd,
                            &context.grant_root,
                            &context.kind,
                        ]
                        .iter()
                        .all(|value| value.as_ref().is_none_or(|value| value.len() <= 4096))
                }
                EventKind::UserInputRequested {
                    request_id,
                    question_ids,
                    questions,
                } => {
                    request_id.0.len() <= 4096
                        && question_ids.len() <= 32
                        && question_ids.iter().all(|id| id.len() <= 256)
                        && questions.len() == question_ids.len()
                        && questions
                            .iter()
                            .map(|question| {
                                question.id.len()
                                    + question.header.len()
                                    + question.question.len()
                                    + question
                                        .options
                                        .iter()
                                        .map(|option| option.label.len() + option.description.len())
                                        .sum::<usize>()
                            })
                            .sum::<usize>()
                            <= 32 * 1024
                }
                EventKind::ServerRequestResolved {
                    thread_id,
                    request_id,
                } => thread_id.0.len() <= 4096 && request_id.0.len() <= 4096,
                EventKind::AccountLoginCompleted { completion } => {
                    completion
                        .login_id
                        .as_ref()
                        .is_none_or(|value| value.len() <= 1024)
                        && completion
                            .error
                            .as_ref()
                            .is_none_or(|value| value.len() <= 4096)
                }
                _ => true,
            },
            _ => true,
        };
        if !valid_metadata {
            self.status = ConnectionStatus::Disconnected;
            self.push_diagnostic(
                "Transcript metadata exceeds local capacity; reconnect and reload server history"
                    .into(),
            );
            return true;
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
                thread_next_cursor,
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
                    .map(|mut thread| {
                        thread.turns = Vec::new();
                        thread
                    })
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
                self.thread_next_cursor = thread_next_cursor.filter(|cursor| cursor.len() <= 4096);
                self.thread_paging = false;
                self.pending_thread_page_request = None;
                self.thread_page_error = None;
                self.thread_windowed = false;
            }
            ControllerEvent::ThreadPageLoaded {
                request,
                cursor,
                threads,
                runtime,
                next_cursor,
            } => {
                if !self.thread_paging
                    || self.pending_thread_page_request != Some(request)
                    || self.thread_next_cursor.as_deref() != Some(&cursor)
                {
                    return false;
                }
                let mut known = self
                    .threads
                    .iter()
                    .map(|thread| thread.id.clone())
                    .collect::<HashSet<_>>();
                self.threads.extend(
                    threads
                        .into_iter()
                        .filter(|thread| known.insert(thread.id.clone()))
                        .map(|mut thread| {
                            thread.turns.clear();
                            thread
                        }),
                );
                self.threads.sort_by(|left, right| {
                    right
                        .last_used_at
                        .cmp(&left.last_used_at)
                        .then_with(|| left.id.0.cmp(&right.id.0))
                });
                if self.threads.len() > MAX_THREADS {
                    let excess = self.threads.len() - MAX_THREADS;
                    self.threads.drain(..excess);
                    self.thread_windowed = true;
                }
                let retained = self
                    .threads
                    .iter()
                    .map(|thread| thread.id.clone())
                    .collect::<HashSet<_>>();
                self.thread_runtime.extend(runtime);
                self.thread_runtime.retain(|id, _| retained.contains(id));
                self.thread_next_cursor =
                    next_cursor.filter(|next| next != &cursor && next.len() <= 4096);
                self.thread_paging = false;
                self.pending_thread_page_request = None;
                self.thread_page_error = None;
            }
            ControllerEvent::ThreadPageFailed {
                request,
                cursor,
                message,
            } => {
                if !self.thread_paging
                    || self.pending_thread_page_request != Some(request)
                    || self.thread_next_cursor.as_deref() != Some(&cursor)
                {
                    return false;
                }
                self.thread_paging = false;
                self.pending_thread_page_request = None;
                self.thread_page_error = Some(sanitize_diagnostic(&message));
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
                self.unconfirmed_work = false;
            }
            ControllerEvent::BackendSelected {
                source,
                fallback_reason,
            } => {
                self.backend_source = Some(source);
                self.fallback_reason = fallback_reason.map(|reason| sanitize_diagnostic(&reason));
            }
            ControllerEvent::ProjectConfigured => {}
            ControllerEvent::NewChatPrepared => self.new_chat(),
            ControllerEvent::NewChatFailed(message) => self.push_diagnostic(message),
            ControllerEvent::TurnAccepted => {
                if self.send_pending {
                    self.draft.clear();
                    self.attachments.clear();
                    self.send_pending = false;
                }
            }
            ControllerEvent::CompactionAccepted => {
                self.command_feedback = Some("Compaction started".into());
            }
            ControllerEvent::CompactionFailed(message) => {
                self.command_feedback = Some(sanitize_diagnostic(&message));
                self.push_diagnostic(message);
            }
            ControllerEvent::ReviewAccepted => {
                self.command_feedback = Some("Review started".into());
            }
            ControllerEvent::ReviewFailed(message) => {
                self.command_feedback = Some(sanitize_diagnostic(&message));
                self.push_diagnostic(message);
            }
            ControllerEvent::AccountLoggedOut(account) => {
                self.account = account;
                self.rate_limits = None;
                self.rate_limits_error = None;
                self.rate_limits_pending = false;
                self.command_feedback = Some("Signed out of Codex".into());
            }
            ControllerEvent::LogoutFailed(message) => {
                self.command_feedback = Some(sanitize_diagnostic(&message));
                self.push_diagnostic(message);
            }
            ControllerEvent::RateLimitsLoaded(status) => {
                self.rate_limits = Some(status);
                self.rate_limits_error = None;
                self.rate_limits_pending = false;
            }
            ControllerEvent::RateLimitsFailed(message) => {
                self.rate_limits_error = Some(sanitize_diagnostic(&message));
                self.rate_limits_pending = false;
                self.push_diagnostic(message);
            }
            ControllerEvent::FileSearchLoaded { query, matches }
                if self.file_search_query.as_deref() == Some(query.as_str()) =>
            {
                self.file_search_matches = matches;
                self.file_search_pending = false;
                self.file_search_error = None;
            }
            ControllerEvent::FileSearchFailed { query, message }
                if self.file_search_query.as_deref() == Some(query.as_str()) =>
            {
                self.file_search_matches.clear();
                self.file_search_pending = false;
                self.file_search_error = Some(sanitize_diagnostic(&message));
            }
            ControllerEvent::FileSearchLoaded { .. } | ControllerEvent::FileSearchFailed { .. } => {
            }
            ControllerEvent::FeedbackUploaded {
                report_id,
                include_logs,
            } => {
                self.command_feedback = Some(if include_logs {
                    format!("Feedback and diagnostics sent · report {report_id}")
                } else {
                    format!("Feedback sent without diagnostics · report {report_id}")
                });
            }
            ControllerEvent::FeedbackUploadFailed(message) => {
                self.command_feedback = Some(sanitize_diagnostic(&message));
                self.push_diagnostic(message);
            }
            ControllerEvent::DiffLoaded(mut diff) => {
                bound_text(&mut diff);
                self.local_sequence = self.local_sequence.wrapping_add(1);
                self.push_item(ChatItem {
                    id: format!("local:diff:{}:{}", self.generation, self.local_sequence),
                    kind: ChatItemKind::SessionNotice,
                    text: diff,
                    complete: true,
                });
                self.command_feedback = Some("Working-tree diff loaded".into());
            }
            ControllerEvent::DiffFailed(message) => {
                self.command_feedback = Some(sanitize_diagnostic(&message));
                self.push_diagnostic(message);
            }
            ControllerEvent::LoginStarted(challenge) => {
                self.login_qr = login_challenge_url(&challenge)
                    .and_then(|url| render_qr_code(url).ok())
                    .map(Arc::new);
                self.login_challenge = Some(challenge);
                self.login_pending = false;
                self.login_status = LoginPresentationState::Waiting;
            }
            ControllerEvent::LoginCancelled(login_id) => {
                let matches =
                    self.login_challenge
                        .as_ref()
                        .is_some_and(|challenge| match challenge {
                            LoginChallenge::Browser {
                                login_id: active, ..
                            }
                            | LoginChallenge::DeviceCode {
                                login_id: active, ..
                            } => active == &login_id,
                        });
                if matches {
                    self.login_challenge = None;
                    self.login_qr = None;
                }
                self.login_pending = false;
                self.login_status = LoginPresentationState::Cancelled;
            }
            ControllerEvent::RemoteControlStatus(status) => {
                if status.status != nickel_codex::RemoteControlConnectionStatus::Connected {
                    self.remote_pairing = None;
                    self.remote_pairing_qr = None;
                }
                self.remote_control_status = Some(status);
                self.remote_control_pending = false;
                self.remote_control_message = None;
            }
            ControllerEvent::RemotePairingStarted(challenge) => {
                self.remote_pairing_qr = phone_pairing_url(&challenge.pairing_code)
                    .and_then(|url| render_qr_code(&url).ok())
                    .map(Arc::new);
                self.remote_pairing = Some(challenge);
                self.remote_control_pending = false;
                self.remote_control_message = Some("Waiting for phone…".into());
            }
            ControllerEvent::RemotePairingClaimed => {
                self.remote_pairing = None;
                self.remote_pairing_qr = None;
                self.remote_control_pending = false;
                self.remote_control_message = Some("Phone paired".into());
            }
            ControllerEvent::RemotePairingCancelled => {
                self.remote_pairing = None;
                self.remote_pairing_qr = None;
                self.remote_control_pending = false;
                self.remote_control_message =
                    Some("Pairing stopped; the Codex code expires automatically".into());
            }
            ControllerEvent::RemotePairingFailed(message) => {
                self.remote_pairing = None;
                self.remote_pairing_qr = None;
                self.remote_control_pending = false;
                self.remote_control_message = Some(sanitize_diagnostic(&message));
                self.push_diagnostic(message);
            }
            ControllerEvent::RemoteClients(page) => {
                self.remote_clients = page.data.into_iter().take(100).collect();
                self.remote_control_pending = false;
                if !self.remote_clients.is_empty()
                    && self.remote_control_message.as_deref() == Some("Phone paired")
                {
                    self.remote_control_message = None;
                }
            }
            ControllerEvent::ModelRejected { model, message } => {
                self.send_pending = false;
                if self.selected_model.as_deref() == Some(model.as_str()) {
                    self.selected_model = self
                        .models
                        .iter()
                        .find(|candidate| candidate.id != model)
                        .map(|candidate| candidate.id.clone());
                    self.selected_reasoning_effort = self
                        .selected_model
                        .as_deref()
                        .and_then(|selected| {
                            self.models
                                .iter()
                                .find(|candidate| candidate.id == selected)
                        })
                        .and_then(|candidate| candidate.default_reasoning_effort.clone());
                }
                self.push_diagnostic(format!(
                    "The selected model was rejected; choose another model before retrying: {message}"
                ));
            }
            ControllerEvent::ApprovalPolicyAccepted(policy) => {
                self.effective_approval_policy = policy;
            }
            ControllerEvent::SandboxPolicyAccepted(policy) => {
                self.effective_sandbox_policy = policy;
            }
            ControllerEvent::InteractionResponseFailed {
                request_id,
                message,
            } => {
                if self.submitting_requests.contains(&request_id) {
                    self.unconfirmed_responses.insert(request_id);
                    self.push_diagnostic(format!(
                        "Codex response was not confirmed; reconnect before acting again: {message}"
                    ));
                }
            }
            ControllerEvent::Protocol(event) => self.apply_protocol(event),
            ControllerEvent::Incompatible(message) => {
                self.mark_unconfirmed_on_connection_loss();
                self.status = ConnectionStatus::Incompatible;
                self.push_diagnostic(message);
                self.active_turn = None;
                self.interrupt_requested = false;
            }
            ControllerEvent::Unavailable(message) => {
                self.mark_unconfirmed_on_connection_loss();
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
                self.remote_control_pending = false;
                if self.login_pending {
                    self.login_pending = false;
                    self.login_status = LoginPresentationState::Failed;
                }
                self.push_diagnostic(message);
            }
            ControllerEvent::Failure(message) => {
                self.mark_unconfirmed_on_connection_loss();
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
        self.unconfirmed_work = false;
        // A confirmed replacement discards the old unsent draft along with its transcript.
        self.draft.clear();
    }

    pub fn begin_thread_selection(&mut self, id: ThreadId) {
        self.selected_thread = Some(id);
        self.clear_conversation();
    }

    fn clear_conversation(&mut self) {
        self.attachments.clear();
        self.send_pending = false;
        self.active_turn = None;
        self.last_turn_error = None;
        self.turn_error_item_ids.clear();
        self.interrupt_requested = false;
        self.items.clear();
        self.expanded_items.clear();
        self.item_selection_runs.clear();
        self.invalidate_selection_projection();
        self.item_aliases.clear();
        self.merged_agent_starts.clear();
        self.completed_item_ids.clear();
        self.retired_item_ids.clear();
        self.activity_outcomes.clear();
        self.file_change_summaries.clear();
        self.turn_agent_index = None;
        self.clear_exploration();
        self.pending.clear();
        self.pending_thread_ids.clear();
        self.pending_revisions.clear();
        self.notification_unavailable.clear();
        self.interaction_item_ids.clear();
        self.submitting_requests.clear();
        self.unconfirmed_responses.clear();
        self.diagnostics.clear();
        self.unsupported_method_counts.clear();
        self.interaction_answer.clear();
        self.conversation_scroll = 0.0;
        self.conversation_pinned = true;
        self.new_content_while_unpinned = false;
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
        self.expanded_items.clear();
        self.item_selection_runs.clear();
        self.invalidate_selection_projection();
        self.item_aliases.clear();
        self.merged_agent_starts.clear();
        self.completed_item_ids.clear();
        self.retired_item_ids.clear();
        self.activity_outcomes.clear();
        self.file_change_summaries.clear();
        self.pending.clear();
        self.pending_thread_ids.clear();
        self.pending_revisions.clear();
        self.notification_unavailable.clear();
        self.interaction_item_ids.clear();
        self.submitting_requests.clear();
        self.unconfirmed_responses.clear();
        self.active_turn = None;
        self.last_turn_error = None;
        self.turn_error_item_ids.clear();
        self.interrupt_requested = false;
        self.conversation_scroll = 0.0;
        self.conversation_pinned = true;
        self.new_content_while_unpinned = false;
        for turn in &thread.turns {
            self.clear_exploration();
            let mut turn_agent_id: Option<String> = None;
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
                } else if kind == ChatItemKind::Agent
                    && turn_agent_id
                        .as_deref()
                        .and_then(|id| self.resolve_item_index(id))
                        .is_some_and(|index| index + 1 == self.items.len())
                {
                    let index = self
                        .resolve_item_index(turn_agent_id.as_deref().unwrap())
                        .unwrap();
                    if !item.text.is_empty() {
                        if !self.items[index].text.is_empty() {
                            append_bounded(&mut self.items[index].text, "\n\n");
                        }
                        append_bounded(&mut self.items[index].text, &item.text);
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
                        turn_agent_id = self.items.back().map(|item| item.id.clone());
                    }
                }
                if let Some(outcome) = ActivityOutcome::from_source(
                    item.status.as_deref(),
                    item.exit_code,
                    item.duration_ms,
                ) && let Some(index) = self.resolve_item_index(&item.id)
                {
                    self.activity_outcomes
                        .insert(self.items[index].id.clone(), outcome);
                }
                self.mark_item_completed(&item.id);
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
        if !self.conversation_pinned
            && matches!(
                &event.kind,
                EventKind::ItemStarted { .. }
                    | EventKind::AgentMessageDelta { .. }
                    | EventKind::CommandOutputDelta { .. }
                    | EventKind::FileChangeDelta { .. }
                    | EventKind::FilePatchUpdated { .. }
                    | EventKind::PlanDelta { .. }
                    | EventKind::TurnPlanUpdated { .. }
                    | EventKind::ReasoningDelta { .. }
            )
        {
            self.new_content_while_unpinned = true;
        }
        match event.kind {
            EventKind::Connection { state } if state == "failed" => {
                self.status = ConnectionStatus::Disconnected;
                self.active_turn = None;
                self.interrupt_requested = false;
            }
            EventKind::TurnStarted { turn_id, .. } => {
                self.last_turn_error = None;
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
            EventKind::TurnCompleted {
                turn_id, status, ..
            } => {
                if self
                    .active_turn
                    .as_ref()
                    .is_some_and(|active| active != &turn_id)
                {
                    return;
                }
                if let Some(index) = self.exploration_index {
                    self.items[index].complete = true;
                    self.refresh_exploration_text();
                }
                self.active_turn = None;
                if status == "completed" {
                    self.last_turn_error = None;
                    if let Some(item_id) = self.turn_error_item_ids.get(&turn_id)
                        && let Some(index) = self.resolve_item_index(item_id)
                        && !self.items[index].complete
                    {
                        self.items[index].complete = true;
                        append_bounded(&mut self.items[index].text, "\nRetry succeeded.");
                        self.refresh_item_projection(index);
                    }
                } else if status == "failed" {
                    if let Some((_, _, retry)) = self
                        .last_turn_error
                        .as_mut()
                        .filter(|(failed_turn, _, _)| failed_turn == &turn_id)
                    {
                        *retry = false;
                    } else {
                        self.last_turn_error = Some((
                            turn_id.clone(),
                            "Codex reported a failed turn without further detail".into(),
                            false,
                        ));
                    }
                    let message = self
                        .last_turn_error
                        .as_ref()
                        .filter(|(failed_turn, _, _)| failed_turn == &turn_id)
                        .map(|(_, message, _)| message.clone())
                        .unwrap_or_else(|| "Codex reported a failed turn".into());
                    self.upsert_turn_error(&turn_id, &message, false);
                }
                self.turn_agent_index = None;
                self.interrupt_requested = false;
            }
            EventKind::TurnError {
                thread_id,
                turn_id,
                message,
                will_retry,
            } => {
                // A retrying turn error is not a transport disconnect or a terminal run.
                if self.selected_thread.as_ref() == Some(&thread_id) {
                    self.last_turn_error = Some((turn_id.clone(), message.clone(), will_retry));
                    self.upsert_turn_error(&turn_id, &message, will_retry);
                    self.push_diagnostic(format!(
                        "{}: {message}",
                        if will_retry {
                            "Codex is retrying"
                        } else {
                            "Turn failed"
                        }
                    ));
                }
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
                if kind == ChatItemKind::Reasoning && initial_text.trim().is_empty() {
                    // The server often announces reasoning while exposing no
                    // user-visible summary. A later nonempty delta can create it.
                    return;
                }
                if let Some(index) = self.resolve_item_index(&item_id) {
                    // Detail can precede the start notification. Keep the existing stable
                    // item and its streamed/snapshot content instead of duplicating it.
                    if self.items[index].text.is_empty() && !initial_text.is_empty() {
                        self.items[index].text = initial_text;
                        self.refresh_item_projection(index);
                    }
                    return;
                }
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
                        && self.turn_agent_index.is_some_and(|index| {
                            index + 1 == self.items.len() && self.items[index].complete
                        });
                    if merge_agent_update {
                        let index = self.turn_agent_index.expect("checked above");
                        append_bounded(&mut self.items[index].text, "\n\n");
                        let segment_start = self.items[index].text.len();
                        append_bounded(&mut self.items[index].text, &initial_text);
                        self.items[index].complete = false;
                        self.register_item_alias(item_id.clone(), index);
                        self.merged_agent_starts.insert(item_id, segment_start);
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
            EventKind::ItemCompleted {
                item_id,
                completion,
            } => {
                if self.exploration_item_ids.contains(&item_id) {
                    return;
                }
                // Evicted identities stay retired even if a late terminal snapshot arrives.
                if self.retired_item_ids.contains(&item_id) {
                    return;
                }
                if completion.as_ref().is_some_and(|completed| {
                    self.selected_thread
                        .as_ref()
                        .is_some_and(|selected| selected != &completed.thread_id)
                }) {
                    return;
                }
                if !self.mark_item_completed(&item_id) {
                    return;
                }
                let outcome = completion.as_ref().and_then(|completed| {
                    ActivityOutcome::from_source(
                        completed.status.as_deref(),
                        completed.exit_code,
                        completed.duration_ms,
                    )
                });
                let file_snapshot = completion
                    .as_ref()
                    .filter(|completed| {
                        chat_item_kind(&completed.item_type) == ChatItemKind::FileChange
                    })
                    .map(|completed| structured_file_change_summary(&completed.changes));
                if let Some(index) = self.resolve_item_index(&item_id) {
                    // The terminal snapshot supersedes streamed deltas, including lost or
                    // duplicated chunks. Keep an existing item when the server supplies text.
                    if let Some(completed) = &completion {
                        if self.items[index].kind == ChatItemKind::Reasoning
                            && completed.text.trim().is_empty()
                        {
                            self.reconcile_selection_runs();
                            self.remove_item(index);
                            return;
                        }
                        if let Some(start) = self.merged_agent_starts.get(&item_id).copied() {
                            if start <= self.items[index].text.len()
                                && self.items[index].text.is_char_boundary(start)
                            {
                                self.items[index].text.truncate(start);
                                append_bounded(&mut self.items[index].text, &completed.text);
                            }
                        } else {
                            self.items[index].text = completed.text.clone();
                        }
                        self.items[index].complete = true;
                        self.refresh_item_projection(index);
                    } else if self.items[index].text.is_empty() {
                        self.reconcile_selection_runs();
                        self.remove_item(index);
                    } else {
                        self.items[index].complete = true;
                    }
                    if let Some(outcome) = outcome
                        && let Some(retained) = self.resolve_item_index(&item_id)
                    {
                        self.activity_outcomes
                            .insert(self.items[retained].id.clone(), outcome);
                    }
                } else if let Some(completed) = completion {
                    if chat_item_kind(&completed.item_type) == ChatItemKind::Reasoning
                        && completed.text.trim().is_empty()
                    {
                        return;
                    }
                    // A final-only item is valid when the app-server omitted its start/deltas.
                    self.push_item(ChatItem {
                        id: item_id.clone(),
                        kind: chat_item_kind(&completed.item_type),
                        text: completed.text,
                        complete: true,
                    });
                    if let Some(outcome) = outcome
                        && self.resolve_item_index(&item_id).is_some()
                    {
                        self.activity_outcomes.insert(item_id.clone(), outcome);
                    }
                }
                if let Some(summary) = file_snapshot
                    && self.resolve_item_index(&item_id).is_some()
                {
                    if let Some(summary) = summary {
                        self.file_change_summaries.insert(item_id, summary);
                    } else {
                        self.file_change_summaries.remove(&item_id);
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
            EventKind::FilePatchUpdated {
                thread_id,
                item_id,
                changes,
                ..
            } => {
                if self
                    .selected_thread
                    .as_ref()
                    .is_some_and(|selected| selected != &thread_id)
                {
                    return;
                }
                let mut text = changes
                    .iter()
                    .map(|change| {
                        format!(
                            "{}: {}{}\n{}",
                            change.kind,
                            change.path,
                            change
                                .move_path
                                .as_ref()
                                .map_or(String::new(), |target| format!(" → {target}")),
                            change.diff
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                bound_text(&mut text);
                if let Some(index) = self.resolve_item_index(&item_id) {
                    self.items[index].text = text;
                    self.refresh_item_projection(index);
                } else {
                    self.push_item(ChatItem {
                        id: item_id.clone(),
                        kind: ChatItemKind::FileChange,
                        text,
                        complete: false,
                    });
                }
                if self.resolve_item_index(&item_id).is_some() {
                    if let Some(summary) = structured_file_change_summary(&changes) {
                        self.file_change_summaries.insert(item_id, summary);
                    } else {
                        self.file_change_summaries.remove(&item_id);
                    }
                }
            }
            EventKind::PlanDelta { item_id, delta } => {
                self.append_delta(item_id, delta, ChatItemKind::Plan)
            }
            EventKind::TurnPlanUpdated {
                thread_id,
                turn_id,
                explanation,
                steps,
            } => {
                if self
                    .selected_thread
                    .as_ref()
                    .is_some_and(|selected| selected != &thread_id)
                    || (self.selected_thread.is_none()
                        && self.active_turn.as_ref() != Some(&turn_id))
                {
                    return;
                }
                let id = format!("local:turn-plan:{}:{}", thread_id.0, turn_id.0);
                let mut lines = explanation.into_iter().collect::<Vec<_>>();
                lines.extend(
                    steps
                        .into_iter()
                        .map(|step| format!("[{}] {}", step.status, step.step)),
                );
                let mut text = lines.join("\n");
                bound_text(&mut text);
                if let Some(index) = self.resolve_item_index(&id) {
                    self.items[index].text = text;
                    self.refresh_item_projection(index);
                } else {
                    self.push_item(ChatItem {
                        id,
                        kind: ChatItemKind::Plan,
                        text,
                        complete: false,
                    });
                }
            }
            EventKind::ReasoningDelta { item_id, delta } => {
                if !delta.trim().is_empty() {
                    self.append_delta(item_id, delta, ChatItemKind::Reasoning)
                }
            }
            EventKind::ApprovalRequested {
                request_id,
                thread_id,
                approval_type,
                summary,
                context,
            } => {
                if let Some(thread_id) = thread_id.or_else(|| self.selected_thread.clone()) {
                    if self
                        .pending_thread_ids
                        .get(&request_id)
                        .is_some_and(|existing| existing != &thread_id)
                    {
                        self.status = ConnectionStatus::Disconnected;
                        self.push_diagnostic(
                            "Request identity changed across threads; reconnect before acting"
                                .into(),
                        );
                        return;
                    }
                    self.pending_thread_ids
                        .insert(request_id.clone(), thread_id);
                }
                self.push_pending(PendingInteraction::Approval {
                    request_id,
                    approval_type,
                    summary: summary.unwrap_or_else(|| "Codex requests approval".into()),
                    context,
                });
            }
            EventKind::UserInputRequested {
                request_id,
                question_ids,
                questions,
            } => self.push_pending(PendingInteraction::UserInput {
                request_id,
                question_ids,
                questions,
            }),
            EventKind::ServerRequestResolved {
                thread_id,
                request_id,
            } => {
                if self.pending_thread_ids.get(&request_id) == Some(&thread_id) {
                    self.pending.retain(|interaction| match interaction {
                        PendingInteraction::Approval {
                            request_id: pending,
                            ..
                        }
                        | PendingInteraction::UserInput {
                            request_id: pending,
                            ..
                        } => pending != &request_id,
                    });
                    self.submitting_requests.remove(&request_id);
                    self.unconfirmed_responses.remove(&request_id);
                    self.pending_thread_ids.remove(&request_id);
                    self.pending_revisions.remove(&request_id);
                    self.notification_unavailable.remove(&request_id);
                    if let Some(item_id) = self.interaction_item_ids.remove(&request_id)
                        && let Some(index) = self.resolve_item_index(&item_id)
                    {
                        self.items[index].complete = true;
                        append_bounded(&mut self.items[index].text, "\nRequest resolved.");
                        self.refresh_item_projection(index);
                    }
                }
            }
            EventKind::AccountLoginCompleted { completion } => {
                let matches = self.login_challenge.as_ref().is_some_and(|challenge| {
                    let active = match challenge {
                        LoginChallenge::Browser { login_id, .. }
                        | LoginChallenge::DeviceCode { login_id, .. } => login_id,
                    };
                    completion.login_id.as_deref() == Some(active.as_str())
                });
                if matches {
                    self.login_challenge = None;
                    self.login_qr = None;
                    self.login_pending = false;
                    if completion.success {
                        self.login_status = LoginPresentationState::Idle;
                        if !self.account.authenticated {
                            self.push_diagnostic(
                                "Codex reported login completion, but account/read has not confirmed the account".into(),
                            );
                        }
                    } else {
                        self.login_status = LoginPresentationState::Failed;
                        self.push_diagnostic(
                            completion
                                .error
                                .unwrap_or_else(|| "Codex login failed".into()),
                        );
                    }
                }
            }
            EventKind::Error { message } => self.push_diagnostic(message),
            EventKind::Warning {
                thread_id,
                message,
                guardian,
            } => {
                let title = if guardian {
                    "Security warning"
                } else {
                    "Warning"
                };
                if thread_id.as_ref() == self.selected_thread.as_ref() && thread_id.is_some() {
                    self.local_sequence = self.local_sequence.wrapping_add(1);
                    self.push_item(ChatItem {
                        id: format!("local:warning:{}:{}", self.generation, self.local_sequence),
                        kind: ChatItemKind::Warning,
                        text: format!("{title}: {message}"),
                        complete: true,
                    });
                } else {
                    // Global or unrelated-thread warnings belong in bounded diagnostics.
                    self.push_diagnostic(format!("{title}: {message}"));
                }
            }
            EventKind::Inconsistency { message }
                if message.starts_with("delta for unknown item ")
                    || message.starts_with("completion for unknown item ") => {}
            EventKind::Inconsistency { message } => self.push_diagnostic(message),
            EventKind::UnsupportedEvent { method } => {
                // Housekeeping/future notifications belong in bounded diagnostics, not a new
                // transcript card for every occurrence.
                let key = if method.len() <= 128
                    && (self.unsupported_method_counts.contains_key(&method)
                        || self.unsupported_method_counts.len() < 32)
                {
                    method
                } else {
                    "other unsupported method".into()
                };
                let count = self
                    .unsupported_method_counts
                    .entry(key.clone())
                    .or_default();
                *count = count.saturating_add(1);
                let count = *count;
                if count.is_power_of_two() {
                    self.push_diagnostic(format!(
                        "Unsupported Codex protocol method {key} (seen {count} time(s))"
                    ));
                }
            }
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
        let mut limited = false;
        for set in [
            &mut self.exploration_reads,
            &mut self.exploration_lists,
            &mut self.exploration_searches,
            &mut self.exploration_item_ids,
        ] {
            set.retain(|value| {
                let keep = value.len() <= 4096;
                limited |= !keep;
                keep
            });
            while set.len() > MAX_ITEMS {
                if let Some(key) = set.iter().min().cloned() {
                    set.remove(&key);
                    limited = true;
                }
            }
            // HashSet growth is geometric even when the live entry count is bounded.
            if set.capacity() > MAX_ITEMS * 2 {
                set.shrink_to_fit();
            }
        }
        if limited {
            self.push_diagnostic(
                "Exploration detail capacity reached; displayed counts are lower bounds".into(),
            );
        }
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

    fn push_item(&mut self, mut item: ChatItem) {
        bound_text(&mut item.text);
        item.id.shrink_to_fit();
        self.reconcile_selection_runs();
        if self.items.len() == MAX_ITEMS {
            let index = self
                .items
                .iter()
                .position(|item| item.complete)
                .unwrap_or(0);
            self.remove_item(index);
        }
        self.item_selection_runs
            .push_back(ItemProjection::default());
        self.invalidate_selection_projection();
        self.items.push_back(item);
        self.enforce_transcript_budget();
    }

    fn refresh_item_projection(&mut self, index: usize) {
        bound_text(&mut self.items[index].text);
        self.item_selection_runs[index].invalidate();
        self.invalidate_selection_projection();
        self.enforce_transcript_budget();
    }

    fn remove_item(&mut self, index: usize) {
        if let Some(removed) = self.items.remove(index) {
            self.expanded_items.remove(&removed.id);
            self.activity_outcomes.remove(&removed.id);
            self.file_change_summaries.remove(&removed.id);
            self.item_selection_runs.remove(index);
            self.retired_item_ids.extend(
                self.item_aliases
                    .iter()
                    .filter(|(_, canonical)| canonical == &removed.id)
                    .map(|(alias, _)| alias.clone()),
            );
            self.retired_item_ids.push_back(removed.id.clone());
            while self.retired_item_ids.len() > MAX_ITEMS
                || self
                    .retired_item_ids
                    .iter()
                    .map(String::capacity)
                    .sum::<usize>()
                    > 2 * 1024 * 1024
            {
                self.retired_item_ids.pop_front();
            }
            self.item_aliases
                .retain(|(_, canonical)| canonical != &removed.id);
            self.merged_agent_starts.retain(|alias, _| {
                self.item_aliases
                    .iter()
                    .any(|(existing, _)| existing == alias)
            });
            for slot in [&mut self.turn_agent_index, &mut self.exploration_index] {
                *slot = slot.and_then(|old| {
                    if old == index {
                        None
                    } else {
                        Some(old - usize::from(old > index))
                    }
                });
            }
            self.invalidate_selection_projection();
        }
    }

    fn enforce_transcript_budget(&mut self) {
        let mut evicted = false;
        while self
            .items
            .iter()
            .map(|item| item.text.capacity())
            .sum::<usize>()
            > MAX_TRANSCRIPT_TEXT_BYTES
        {
            let index = self
                .items
                .iter()
                .position(|item| item.complete)
                .unwrap_or(0);
            self.remove_item(index);
            evicted = true;
        }
        if evicted {
            self.push_diagnostic("Older output omitted from this local view to limit memory; server history is unchanged".into());
        }
    }

    fn record_selected_thread(&mut self, mut thread: Thread) {
        thread.turns = Vec::new();
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
        self.item_selection_runs[index].invalidate();
        self.invalidate_selection_projection();
        self.reconcile_item_aliases();
        true
    }

    fn push_pending(&mut self, interaction: PendingInteraction) {
        let request_id = match &interaction {
            PendingInteraction::Approval { request_id, .. }
            | PendingInteraction::UserInput { request_id, .. } => request_id.clone(),
        };
        if let Some(thread_id) = &self.selected_thread {
            self.pending_thread_ids
                .entry(request_id.clone())
                .or_insert_with(|| thread_id.clone());
        }
        if let Some(index) = self.pending.iter().position(|pending| match pending {
            PendingInteraction::Approval {
                request_id: existing,
                ..
            }
            | PendingInteraction::UserInput {
                request_id: existing,
                ..
            } => existing == &request_id,
        }) {
            if self.pending[index] != interaction {
                if self.submitting_requests.contains(&request_id) {
                    self.status = ConnectionStatus::Disconnected;
                    self.push_diagnostic(
                        "Request scope changed while a response was in flight; reconnect before acting"
                            .into(),
                    );
                } else {
                    self.pending[index] = interaction;
                    self.advance_pending_revision(request_id);
                    self.sync_interaction_reference(self.pending[index].clone());
                }
            }
            return;
        }
        if self.pending.len() == MAX_PENDING {
            self.status = ConnectionStatus::Disconnected;
            self.push_diagnostic(
                "Pending interaction capacity exceeded; reconnect to recover authoritative state"
                    .into(),
            );
            return;
        }
        self.pending.push(interaction);
        self.advance_pending_revision(request_id);
        self.sync_interaction_reference(self.pending.last().expect("just pushed").clone());
    }

    fn sync_interaction_reference(&mut self, interaction: PendingInteraction) {
        // Approvals are already rendered in the actionable request panel. A second
        // transcript card repeats the same request without adding decision authority.
        if matches!(interaction, PendingInteraction::Approval { .. }) {
            return;
        }
        let (request_id, kind, text) = match interaction {
            PendingInteraction::UserInput {
                request_id,
                questions,
                ..
            } => (
                request_id,
                ChatItemKind::QuestionReference,
                format!(
                    "{}\nAnswer in the request panel below.",
                    questions
                        .iter()
                        .map(|question| question.question.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            ),
            PendingInteraction::Approval { .. } => unreachable!("handled above"),
        };
        if let Some(item_id) = self.interaction_item_ids.get(&request_id)
            && let Some(index) = self.resolve_item_index(item_id)
        {
            self.items[index].text = text;
            self.refresh_item_projection(index);
            return;
        }
        // A reference is chronology only: it never stores a decision or mutates request
        // authority, which stays in pending and is shared with shell notifications.
        self.local_sequence = self.local_sequence.wrapping_add(1);
        let item_id = format!("local:request:{}:{}", self.generation, self.local_sequence);
        self.push_item(ChatItem {
            id: item_id.clone(),
            kind,
            text,
            complete: false,
        });
        self.interaction_item_ids.insert(request_id, item_id);
    }

    fn upsert_turn_error(&mut self, turn_id: &TurnId, message: &str, will_retry: bool) {
        let title = if will_retry {
            "Codex is retrying"
        } else {
            "Turn failed"
        };
        let text = format!("{title}: {message}");
        if let Some(item_id) = self.turn_error_item_ids.get(turn_id)
            && let Some(index) = self.resolve_item_index(item_id)
        {
            self.items[index].text = text;
            self.items[index].complete = !will_retry;
            self.refresh_item_projection(index);
            return;
        }
        // Correlation is only for retained display items; the active turn and retry
        // authority remain in last_turn_error and the backend controller.
        if self.turn_error_item_ids.len() >= 64 {
            let oldest = self.items.iter().find_map(|item| {
                self.turn_error_item_ids
                    .iter()
                    .find(|(_, id)| *id == &item.id)
                    .map(|(turn, _)| turn.clone())
            });
            if let Some(oldest) = oldest {
                self.turn_error_item_ids.remove(&oldest);
            } else {
                self.turn_error_item_ids.clear();
            }
        }
        self.local_sequence = self.local_sequence.wrapping_add(1);
        let id = format!(
            "local:turn-error:{}:{}",
            self.generation, self.local_sequence
        );
        self.push_item(ChatItem {
            id: id.clone(),
            kind: ChatItemKind::Error,
            text,
            complete: !will_retry,
        });
        self.turn_error_item_ids.insert(turn_id.clone(), id);
    }

    fn advance_pending_revision(&mut self, request_id: ServerRequestId) {
        self.notification_unavailable.remove(&request_id);
        let Some(next) = self.next_pending_revision.checked_add(1) else {
            self.status = ConnectionStatus::Disconnected;
            self.push_diagnostic(
                "Request revision counter exhausted; reconnect before acting".into(),
            );
            return;
        };
        self.next_pending_revision = next;
        self.pending_revisions.insert(request_id, next);
    }

    fn append_delta(&mut self, item_id: String, delta: String, inferred_kind: ChatItemKind) {
        if self.retired_item_ids.contains(&item_id) || self.completed_item_ids.contains(&item_id) {
            return;
        }
        if let Some(index) = self.resolve_item_index(&item_id) {
            self.reconcile_selection_runs();
            append_bounded(&mut self.items[index].text, &delta);
            self.refresh_item_projection(index);
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
            while self
                .item_aliases
                .iter()
                .map(|(alias, id)| alias.capacity() + id.capacity())
                .sum::<usize>()
                > 2 * 1024 * 1024
            {
                self.item_aliases.pop_front();
            }
            self.merged_agent_starts.retain(|alias, _| {
                self.item_aliases
                    .iter()
                    .any(|(existing, _)| existing == alias)
            });
        }
    }

    fn mark_item_completed(&mut self, item_id: &str) -> bool {
        if self
            .completed_item_ids
            .iter()
            .any(|completed| completed == item_id)
        {
            return false;
        }
        self.completed_item_ids.push_back(item_id.into());
        while self.completed_item_ids.len() > MAX_ITEM_ALIASES
            || self
                .completed_item_ids
                .iter()
                .map(String::capacity)
                .sum::<usize>()
                > 2 * 1024 * 1024
        {
            self.completed_item_ids.pop_front();
        }
        true
    }

    fn reconcile_item_aliases(&mut self) {
        self.item_aliases
            .retain(|(_, canonical_id)| self.items.iter().any(|item| &item.id == canonical_id));
        self.merged_agent_starts.retain(|alias, _| {
            self.item_aliases
                .iter()
                .any(|(existing, _)| existing == alias)
        });
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
            self.item_selection_runs = self
                .items
                .iter()
                .map(|_| ItemProjection::default())
                .collect();
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
        self.items
            .iter()
            .map(|item| {
                if is_collapsible_activity(&item.kind) && !self.expanded_items.contains(&item.id) {
                    // Virtualization must reserve the collapsed card, not its hidden output.
                    104.0
                } else {
                    estimate_item_height(item)
                }
            })
            .collect()
    }

    pub fn transcript_selection_document(&self) -> Arc<SelectionDocument> {
        {
            let cache = self.selection_document_cache.borrow();
            if cache.0 == self.selection_revision && cache.1 == self.items.len() {
                return cache.2.clone();
            }
        }
        let snapshots = if self.item_selection_runs.len() == self.items.len() {
            self.item_selection_runs
                .iter()
                .zip(&self.items)
                .map(|(projection, item)| projection.snapshot(item))
                .collect::<Vec<_>>()
        } else {
            self.items
                .iter()
                .map(|item| ItemProjection::default().snapshot(item))
                .collect()
        };
        let document = Arc::new(SelectionDocument::lazy(
            self.selection_revision,
            move || {
                snapshots
                    .iter()
                    .flat_map(|snapshot| snapshot.runs.clone())
                    .collect()
            },
        ));
        *self.selection_document_cache.borrow_mut() =
            (self.selection_revision, self.items.len(), document.clone());
        document
    }

    pub(crate) fn markdown_document(&self, index: usize) -> Arc<MarkdownDocument> {
        if self.item_selection_runs.len() == self.items.len() {
            let projection = &self.item_selection_runs[index];
            projection.snapshot(&self.items[index]).document.clone()
        } else {
            Arc::new(item_markdown_document(&self.items[index]))
        }
    }
}

fn login_challenge_url(challenge: &LoginChallenge) -> Option<&str> {
    let url = match challenge {
        LoginChallenge::Browser { auth_url, .. } => auth_url,
        LoginChallenge::DeviceCode {
            verification_url, ..
        } => verification_url,
    };
    (url.starts_with("https://") || url.starts_with("http://")).then_some(url)
}

fn phone_pairing_url(pairing_code: &str) -> Option<String> {
    if pairing_code.is_empty() {
        return None;
    }
    // Codex returns the opaque code; the phone scanner expects a link containing it.
    let mut url = url::Url::parse("https://chatgpt.com/codex/pair").ok()?;
    url.query_pairs_mut()
        .append_pair("pairing_code", pairing_code);
    Some(url.into())
}

fn render_qr_code(text: &str) -> Result<image::RgbaImage, qrcode::types::QrError> {
    const MODULE: u32 = 6;
    const QUIET: u32 = 4;
    let code = qrcode::QrCode::new(text.as_bytes())?;
    let modules = code.width() as u32;
    let size = (modules + QUIET * 2) * MODULE;
    let mut image = image::RgbaImage::from_pixel(size, size, image::Rgba([255, 255, 255, 255]));
    for y in 0..modules {
        for x in 0..modules {
            if code[(x as usize, y as usize)] == qrcode::Color::Dark {
                for pixel_y in 0..MODULE {
                    for pixel_x in 0..MODULE {
                        image.put_pixel(
                            (x + QUIET) * MODULE + pixel_x,
                            (y + QUIET) * MODULE + pixel_y,
                            image::Rgba([0, 0, 0, 255]),
                        );
                    }
                }
            }
        }
    }
    Ok(image)
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

#[cfg(test)]
fn selection_runs_for_item(item: &ChatItem) -> Vec<SelectionRun> {
    selection_runs_from_document(item, &item_markdown_document(item))
}

fn selection_runs_from_document(item: &ChatItem, document: &MarkdownDocument) -> Vec<SelectionRun> {
    let mut runs = vec![SelectionRun::block(
        format!("{}/label", item.id),
        item_label(&item.kind),
    )];
    runs.extend(markdown_selection_runs(
        document,
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
        ChatItemKind::Tool => "Tool",
        ChatItemKind::Search => "Web search",
        ChatItemKind::Image => "Image",
        ChatItemKind::Delegation => "Delegation",
        ChatItemKind::ApprovalReference => "Approval requested",
        ChatItemKind::QuestionReference => "Question from Codex",
        ChatItemKind::SessionNotice => "Session",
        ChatItemKind::Plan => "Plan",
        ChatItemKind::Warning => "Warning",
        ChatItemKind::Error => "Error",
        ChatItemKind::Unknown(_) => "Additional event",
    }
}

pub(crate) fn is_collapsible_activity(kind: &ChatItemKind) -> bool {
    matches!(
        kind,
        ChatItemKind::Command
            | ChatItemKind::FileChange
            | ChatItemKind::Tool
            | ChatItemKind::Search
            | ChatItemKind::Image
            | ChatItemKind::Delegation
            | ChatItemKind::Unknown(_)
    )
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
        "mcpToolCall" | "dynamicToolCall" => ChatItemKind::Tool,
        "webSearch" => ChatItemKind::Search,
        "imageView" | "imageGeneration" => ChatItemKind::Image,
        "collabAgentToolCall" | "subAgentActivity" => ChatItemKind::Delegation,
        "enteredReviewMode" | "exitedReviewMode" | "contextCompaction" => {
            ChatItemKind::SessionNotice
        }
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

    #[test]
    fn new_chat_commits_only_after_controller_acknowledgement() {
        let mut state = ChatState {
            selected_thread: Some(ThreadId("current".into())),
            draft: "unsent work".into(),
            ..Default::default()
        };
        state.items.push_back(ChatItem {
            id: "current-answer".into(),
            kind: ChatItemKind::Agent,
            text: "Existing transcript".into(),
            complete: true,
        });

        state.apply(
            1,
            ControllerEvent::NewChatFailed("Workspace unavailable".into()),
        );
        assert_eq!(state.selected_thread, Some(ThreadId("current".into())));
        assert_eq!(state.draft, "unsent work");
        assert_eq!(state.items[0].text, "Existing transcript");

        state.apply(1, ControllerEvent::NewChatPrepared);
        assert_eq!(state.selected_thread, None);
        assert!(state.draft.is_empty());
        assert!(state.items.is_empty());
    }

    #[test]
    fn presentation_status_keeps_transport_failure_ahead_of_pending_work() {
        let mut state = ChatState {
            status: ConnectionStatus::Ready,
            ..Default::default()
        };
        state.account.authenticated = true;
        assert_eq!(
            state.run_presentation_status(),
            RunPresentationStatus::Ready
        );
        state.send_pending = true;
        assert_eq!(
            state.run_presentation_status(),
            RunPresentationStatus::Starting
        );
        state.send_pending = false;
        state.active_turn = Some(TurnId("turn".into()));
        assert_eq!(
            state.run_presentation_status(),
            RunPresentationStatus::Working
        );
        state.pending.push(PendingInteraction::Approval {
            request_id: ServerRequestId("approval".into()),
            approval_type: "command".into(),
            summary: "Run a command".into(),
            context: nickel_codex::ApprovalContext::default(),
        });
        assert_eq!(
            state.run_presentation_status(),
            RunPresentationStatus::AwaitingApproval
        );
        state.apply(1, ControllerEvent::Failure("transport closed".into()));
        assert_eq!(
            state.run_presentation_status(),
            RunPresentationStatus::Disconnected
        );
        assert_eq!(
            state.pending.len(),
            1,
            "transport loss must not erase authority facts"
        );
        assert!(state.run_secondary_status().is_some());
        state.status = ConnectionStatus::Ready;
        assert_eq!(
            state.run_presentation_status(),
            RunPresentationStatus::Unconfirmed
        );
        assert!(!state.can_send());
        state.new_chat();
        assert_eq!(
            state.run_presentation_status(),
            RunPresentationStatus::Ready
        );
    }

    #[test]
    fn healthy_unauthenticated_transport_requires_sign_in_before_actions() {
        let mut state = ChatState {
            status: ConnectionStatus::Ready,
            draft: "not yet sent".into(),
            ..Default::default()
        };
        let request_id = ServerRequestId("approval".into());
        state.pending.push(PendingInteraction::Approval {
            request_id: request_id.clone(),
            approval_type: "command".into(),
            summary: "Do something".into(),
            context: nickel_codex::ApprovalContext::default(),
        });
        assert_eq!(
            state.run_presentation_status(),
            RunPresentationStatus::AuthenticationRequired
        );
        assert!(!state.can_send());
        assert!(!state.interaction_is_actionable(&request_id));
        state.account.authenticated = true;
        assert_eq!(
            state.run_presentation_status(),
            RunPresentationStatus::AwaitingApproval
        );
        assert!(state.interaction_is_actionable(&request_id));
    }

    #[test]
    fn pending_response_is_fenced_until_matching_resolution_or_reconnect() {
        let mut state = ChatState {
            status: ConnectionStatus::Ready,
            ..Default::default()
        };
        state.account.authenticated = true;
        state.selected_thread = Some(ThreadId("thread-a".into()));
        let request_id = ServerRequestId("request-1".into());
        let kind = "item/commandExecution/requestApproval";
        state.push_pending(PendingInteraction::Approval {
            request_id: request_id.clone(),
            approval_type: kind.into(),
            summary: "Run tests".into(),
            context: nickel_codex::ApprovalContext::default(),
        });
        assert!(!state.begin_approval_response(&request_id, "wrong-kind"));
        assert!(state.begin_approval_response(&request_id, kind));
        assert!(!state.begin_approval_response(&request_id, kind));
        assert_eq!(state.pending.len(), 1, "dispatch is not resolution");
        state.apply_protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::ServerRequestResolved {
                thread_id: ThreadId("other-thread".into()),
                request_id: request_id.clone(),
            },
        });
        assert_eq!(
            state.pending.len(),
            1,
            "another thread cannot retire the request"
        );
        state.apply_protocol(CodexEvent {
            sequence: 2,
            kind: EventKind::ServerRequestResolved {
                thread_id: ThreadId("thread-a".into()),
                request_id: request_id.clone(),
            },
        });
        assert!(state.pending.is_empty());
        assert!(!state.interaction_submission_pending(&request_id));
        state.push_pending(PendingInteraction::Approval {
            request_id: request_id.clone(),
            approval_type: kind.into(),
            summary: "Different request after resolution".into(),
            context: nickel_codex::ApprovalContext::default(),
        });
        assert!(state.interaction_is_actionable(&request_id));
        assert_eq!(state.invalidate_pending_for_reconnect(), 1);
        assert!(!state.interaction_is_actionable(&request_id));
        assert!(
            state.items.is_empty(),
            "approvals have no transcript duplicate"
        );
    }

    #[test]
    fn interaction_references_preserve_chronology_without_owning_decisions() {
        let mut state = ChatState {
            selected_thread: Some(ThreadId("thread-a".into())),
            ..Default::default()
        };
        let approval_id = ServerRequestId("approval-1".into());
        state.apply_protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::ApprovalRequested {
                request_id: approval_id.clone(),
                thread_id: Some(ThreadId("thread-a".into())),
                approval_type: "item/commandExecution/requestApproval".into(),
                summary: Some("Run cargo test".into()),
                context: nickel_codex::ApprovalContext::default(),
            },
        });
        state.apply_protocol(CodexEvent {
            sequence: 2,
            kind: EventKind::ItemStarted {
                thread_id: Some(ThreadId("thread-a".into())),
                turn_id: Some(TurnId("turn-a".into())),
                item_id: "agent-1".into(),
                item_type: "agentMessage".into(),
                command_actions: Vec::new(),
                initial_text: "Continuing".into(),
            },
        });
        let question_id = ServerRequestId("question-1".into());
        state.apply_protocol(CodexEvent {
            sequence: 3,
            kind: EventKind::UserInputRequested {
                request_id: question_id.clone(),
                question_ids: vec!["choice".into()],
                questions: vec![nickel_codex::UserInputQuestion {
                    id: "choice".into(),
                    header: "Choice".into(),
                    question: "Which option?".into(),
                    options: Vec::new(),
                    is_other: false,
                    is_secret: false,
                }],
            },
        });
        assert_eq!(state.items.len(), 2);
        assert_eq!(state.items[0].id, "agent-1");
        assert_eq!(state.items[1].kind, ChatItemKind::QuestionReference);
        assert!(state.items[1].text.contains("Which option?"));
        assert!(!state.items[1].text.contains("choice"));

        state.apply_protocol(CodexEvent {
            sequence: 4,
            kind: EventKind::ApprovalRequested {
                request_id: approval_id.clone(),
                thread_id: Some(ThreadId("thread-a".into())),
                approval_type: "item/commandExecution/requestApproval".into(),
                summary: Some("Run cargo test with network".into()),
                context: nickel_codex::ApprovalContext::default(),
            },
        });
        assert_eq!(state.items.len(), 2);
        assert!(
            matches!(&state.pending[0], PendingInteraction::Approval { summary, .. } if summary.contains("with network"))
        );
        state.apply_protocol(CodexEvent {
            sequence: 5,
            kind: EventKind::ServerRequestResolved {
                thread_id: ThreadId("thread-a".into()),
                request_id: approval_id,
            },
        });
        assert_eq!(state.items.len(), 2);
        assert_eq!(state.pending.len(), 1);
        assert!(matches!(
            &state.pending[0],
            PendingInteraction::UserInput { request_id, .. } if request_id == &question_id
        ));
    }

    #[test]
    fn scoped_warnings_are_distinct_cards_and_global_warnings_stay_diagnostic() {
        let mut state = ChatState {
            selected_thread: Some(ThreadId("thread-a".into())),
            ..Default::default()
        };
        state.apply_protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::Warning {
                thread_id: Some(ThreadId("thread-a".into())),
                message: "Approval policy changed".into(),
                guardian: true,
            },
        });
        state.apply_protocol(CodexEvent {
            sequence: 2,
            kind: EventKind::Warning {
                thread_id: None,
                message: "Deprecated setting".into(),
                guardian: false,
            },
        });
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].kind, ChatItemKind::Warning);
        assert!(state.items[0].text.starts_with("Security warning"));
        assert!(
            state
                .diagnostics
                .back()
                .unwrap()
                .contains("Deprecated setting")
        );
    }

    #[test]
    fn resolution_uses_request_thread_not_current_selection() {
        let mut state = ChatState {
            status: ConnectionStatus::Ready,
            ..Default::default()
        };
        let request_id = ServerRequestId("request-1".into());
        state.apply_protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::ApprovalRequested {
                request_id: request_id.clone(),
                thread_id: Some(ThreadId("owning-thread".into())),
                approval_type: "item/commandExecution/requestApproval".into(),
                summary: Some("Run tests".into()),
                context: nickel_codex::ApprovalContext::default(),
            },
        });
        state.apply_protocol(CodexEvent {
            sequence: 2,
            kind: EventKind::ServerRequestResolved {
                thread_id: ThreadId("different-thread".into()),
                request_id: request_id.clone(),
            },
        });
        assert_eq!(state.pending.len(), 1);
        state.apply_protocol(CodexEvent {
            sequence: 3,
            kind: EventKind::ApprovalRequested {
                request_id: request_id.clone(),
                thread_id: Some(ThreadId("different-thread".into())),
                approval_type: "item/commandExecution/requestApproval".into(),
                summary: Some("Run tests".into()),
                context: nickel_codex::ApprovalContext::default(),
            },
        });
        assert_eq!(state.status, ConnectionStatus::Disconnected);
        assert_eq!(state.pending.len(), 1);
        state.apply_protocol(CodexEvent {
            sequence: 4,
            kind: EventKind::ServerRequestResolved {
                thread_id: ThreadId("owning-thread".into()),
                request_id,
            },
        });
        assert!(state.pending.is_empty());
    }

    #[test]
    fn changed_scope_during_submission_requires_recovery() {
        let mut state = ChatState {
            status: ConnectionStatus::Ready,
            ..Default::default()
        };
        state.account.authenticated = true;
        let request_id = ServerRequestId("request-1".into());
        let kind = "item/fileChange/requestApproval";
        state.push_pending(PendingInteraction::Approval {
            request_id: request_id.clone(),
            approval_type: kind.into(),
            summary: "Write A".into(),
            context: nickel_codex::ApprovalContext::default(),
        });
        assert!(state.begin_approval_response(&request_id, kind));
        state.push_pending(PendingInteraction::Approval {
            request_id: request_id.clone(),
            approval_type: kind.into(),
            summary: "Write B".into(),
            context: nickel_codex::ApprovalContext::default(),
        });
        assert_eq!(state.status, ConnectionStatus::Disconnected);
        assert!(!state.interaction_is_actionable(&request_id));
    }

    #[test]
    fn failed_response_stays_visible_and_cannot_be_replayed() {
        let mut state = ChatState {
            status: ConnectionStatus::Ready,
            ..Default::default()
        };
        state.account.authenticated = true;
        let request_id = ServerRequestId("request-1".into());
        let kind = "item/commandExecution/requestApproval";
        state.push_pending(PendingInteraction::Approval {
            request_id: request_id.clone(),
            approval_type: kind.into(),
            summary: "Run command".into(),
            context: nickel_codex::ApprovalContext::default(),
        });
        assert!(state.begin_approval_response(&request_id, kind));
        state.apply(
            1,
            ControllerEvent::InteractionResponseFailed {
                request_id: request_id.clone(),
                message: "socket closed".into(),
            },
        );
        assert_eq!(state.pending.len(), 1);
        assert!(state.interaction_response_unconfirmed(&request_id));
        assert!(!state.begin_approval_response(&request_id, kind));
        assert_eq!(state.invalidate_pending_for_reconnect(), 1);
        assert!(!state.interaction_response_unconfirmed(&request_id));
    }

    #[test]
    fn oversized_interaction_metadata_cannot_enter_pending_authority() {
        let mut state = ChatState {
            status: ConnectionStatus::Ready,
            ..Default::default()
        };
        state.apply(
            1,
            ControllerEvent::Protocol(CodexEvent {
                sequence: 1,
                kind: EventKind::ApprovalRequested {
                    request_id: ServerRequestId("request".into()),
                    thread_id: Some(ThreadId("thread".into())),
                    approval_type: "item/commandExecution/requestApproval".into(),
                    summary: Some("x".repeat(4097)),
                    context: nickel_codex::ApprovalContext::default(),
                },
            }),
        );
        assert!(state.pending.is_empty());
        assert_eq!(state.status, ConnectionStatus::Disconnected);
    }

    #[test]
    fn login_completion_only_consumes_the_matching_challenge() {
        let mut state = ChatState::default();
        state.apply(
            1,
            ControllerEvent::LoginStarted(LoginChallenge::DeviceCode {
                login_id: "current".into(),
                user_code: "ABCD-EFGH".into(),
                verification_url: "https://example.test/device".into(),
            }),
        );
        assert!(state.login_qr.is_some());
        state.apply(
            1,
            ControllerEvent::Protocol(CodexEvent {
                sequence: 1,
                kind: EventKind::AccountLoginCompleted {
                    completion: nickel_codex::LoginCompletion {
                        login_id: Some("stale".into()),
                        success: true,
                        error: None,
                    },
                },
            }),
        );
        assert!(!state.account.authenticated);
        assert!(state.login_challenge.is_some());
        state.apply(
            1,
            ControllerEvent::Protocol(CodexEvent {
                sequence: 2,
                kind: EventKind::AccountLoginCompleted {
                    completion: nickel_codex::LoginCompletion {
                        login_id: Some("current".into()),
                        success: true,
                        error: None,
                    },
                },
            }),
        );
        assert!(!state.account.authenticated);
        assert!(state.login_challenge.is_none());
        assert!(state.login_qr.is_none());
    }

    #[test]
    fn codex_phone_pairing_secret_is_cleared_on_cancel_failure_and_disconnect() {
        let challenge = || nickel_codex::RemotePairingChallenge {
            environment_id: "environment-1".into(),
            expires_at: 42,
            pairing_code: "opaque-secret-payload".into(),
            manual_pairing_code: Some("1234".into()),
        };
        let mut state = ChatState::default();
        state.apply(1, ControllerEvent::RemotePairingStarted(challenge()));
        assert!(state.remote_pairing.is_some());
        assert!(state.remote_pairing_qr.is_some());

        state.apply(1, ControllerEvent::RemotePairingCancelled);
        assert!(state.remote_pairing.is_none());
        assert!(state.remote_pairing_qr.is_none());

        state.apply(1, ControllerEvent::RemotePairingStarted(challenge()));
        state.apply(1, ControllerEvent::RemotePairingFailed("expired".into()));
        assert!(state.remote_pairing.is_none());
        assert!(state.remote_pairing_qr.is_none());

        state.apply(1, ControllerEvent::RemotePairingStarted(challenge()));
        state.apply(
            1,
            ControllerEvent::RemoteControlStatus(nickel_codex::RemoteControlStatus {
                status: nickel_codex::RemoteControlConnectionStatus::Disabled,
                server_name: "workstation".into(),
                installation_id: "installation-1".into(),
                environment_id: None,
            }),
        );
        assert!(state.remote_pairing.is_none());
        assert!(state.remote_pairing_qr.is_none());
    }

    #[test]
    fn phone_pairing_qr_uses_url_with_opaque_code() {
        assert_eq!(
            phone_pairing_url("abc123").as_deref(),
            Some("https://chatgpt.com/codex/pair?pairing_code=abc123")
        );
        assert_eq!(
            phone_pairing_url("a+b/c").as_deref(),
            Some("https://chatgpt.com/codex/pair?pairing_code=a%2Bb%2Fc")
        );
        assert!(phone_pairing_url("").is_none());
    }

    #[test]
    fn paired_client_list_replaces_transient_pairing_success_message() {
        let mut state = ChatState {
            remote_control_message: Some("Phone paired".into()),
            ..ChatState::default()
        };
        state.apply(
            1,
            ControllerEvent::RemoteClients(nickel_codex::RemoteControlClientPage {
                data: vec![nickel_codex::RemoteControlClient {
                    client_id: "phone-1".into(),
                    display_name: Some("iPhone".into()),
                    device_model: None,
                    device_type: None,
                    platform: None,
                    os_version: None,
                    app_version: None,
                    last_seen_at: None,
                }],
                next_cursor: None,
            }),
        );
        assert!(state.remote_control_message.is_none());
        assert_eq!(state.remote_clients.len(), 1);
    }

    #[test]
    fn confirmed_account_snapshot_not_completion_establishes_authentication() {
        let mut state = ChatState::default();
        state.apply(
            1,
            ControllerEvent::LoginStarted(LoginChallenge::Browser {
                login_id: "current".into(),
                auth_url: "https://example.test/login".into(),
            }),
        );
        state.apply(
            1,
            ControllerEvent::Ready {
                provenance: "test".into(),
                account: AccountState {
                    authenticated: true,
                    ..Default::default()
                },
                models: Vec::new(),
                projects: Vec::new(),
                threads: Vec::new(),
                runtime: HashMap::new(),
                thread_error: None,
                thread_next_cursor: None,
            },
        );
        state.apply(
            1,
            ControllerEvent::Protocol(CodexEvent {
                sequence: 1,
                kind: EventKind::AccountLoginCompleted {
                    completion: nickel_codex::LoginCompletion {
                        login_id: Some("current".into()),
                        success: true,
                        error: None,
                    },
                },
            }),
        );
        assert!(state.account.authenticated);
        assert!(state.login_challenge.is_none());
    }

    fn delta(state: &mut ChatState, text: &str) {
        state.apply(
            1,
            ControllerEvent::Protocol(CodexEvent {
                sequence: 1,
                kind: EventKind::AgentMessageDelta {
                    item_id: "stream".into(),
                    delta: text.into(),
                },
            }),
        );
    }

    #[test]
    fn unicode_stream_truncation_is_visible_bounded_and_terminal() {
        let mut state = ChatState::default();
        delta(&mut state, &"界".repeat(MAX_ITEM_TEXT_BYTES));
        assert!(state.items[0].text.ends_with(OMISSION_MARKER));
        assert!(state.items[0].text.capacity() <= MAX_ITEM_TEXT_BYTES);
        let retained = state.items[0].text.clone();
        delta(&mut state, "late text");
        assert_eq!(state.items[0].text, retained);
        state.apply(
            1,
            ControllerEvent::Protocol(CodexEvent {
                sequence: 2,
                kind: EventKind::ItemCompleted {
                    item_id: "stream".into(),
                    completion: None,
                },
            }),
        );
        assert!(state.items[0].complete);
        assert!(
            state
                .transcript_selection_document()
                .runs()
                .iter()
                .any(|run| run.text.contains("Further output omitted"))
        );
    }

    #[test]
    fn ordinary_output_can_quote_the_local_omission_marker() {
        let mut text = OMISSION_MARKER.to_owned();
        append_bounded(&mut text, " followed by more output");
        assert!(text.ends_with(" followed by more output"));
    }

    #[test]
    fn streaming_rebuilds_only_consumed_items_once_per_batch() {
        let text = "## Heading\n\n**Unicode 世界** and `incomplete";
        let mut small = ChatState::default();
        small.push_item(ChatItem {
            id: "offscreen".into(),
            kind: ChatItemKind::Agent,
            text: "untouched".into(),
            complete: true,
        });
        for character in text.chars() {
            delta(&mut small, &character.to_string());
        }
        assert!(
            small
                .item_selection_runs
                .iter()
                .all(|projection| projection.builds.get() == 0)
        );
        let rendered = small.markdown_document(1);
        assert!(Arc::ptr_eq(&rendered, &small.markdown_document(1)));
        assert_eq!(small.item_selection_runs[1].builds.get(), 1);
        assert_eq!(small.item_selection_runs[0].builds.get(), 0);
        // The full selection document is an explicit consumer of offscreen logical text.
        small.transcript_selection_document().runs();
        assert_eq!(small.item_selection_runs[1].builds.get(), 1);
        assert_eq!(small.item_selection_runs[0].builds.get(), 1);
        let mut big = ChatState::default();
        delta(&mut big, text);
        assert_eq!(*rendered, *big.markdown_document(0));
        assert_eq!(
            small.item_selection_runs[1]
                .cached
                .borrow()
                .as_ref()
                .unwrap()
                .runs,
            big.item_selection_runs[0]
                .cached
                .borrow()
                .as_ref()
                .unwrap()
                .runs
        );
        delta(&mut small, "` end");
        assert_eq!(small.item_selection_runs[1].builds.get(), 1);
        assert!(!Arc::ptr_eq(&rendered, &small.markdown_document(1)));
        assert_eq!(small.item_selection_runs[1].builds.get(), 2);
    }

    #[test]
    fn adjacent_completed_agent_items_preserve_started_text_and_aliases() {
        let mut state = ChatState::default();
        let turn_id = TurnId("turn".into());
        state.apply(
            1,
            ControllerEvent::Protocol(CodexEvent {
                sequence: 1,
                kind: EventKind::TurnStarted {
                    thread_id: ThreadId("thread".into()),
                    turn_id: turn_id.clone(),
                },
            }),
        );
        for (item_id, initial_text) in [("first", "first text"), ("second", "世界 second")] {
            state.apply(
                1,
                ControllerEvent::Protocol(CodexEvent {
                    sequence: 2,
                    kind: EventKind::ItemStarted {
                        thread_id: None,
                        turn_id: Some(turn_id.clone()),
                        item_id: item_id.into(),
                        item_type: "agentMessage".into(),
                        command_actions: Vec::new(),
                        initial_text: initial_text.into(),
                    },
                }),
            );
            state.apply(
                1,
                ControllerEvent::Protocol(CodexEvent {
                    sequence: 3,
                    kind: EventKind::ItemCompleted {
                        item_id: item_id.into(),
                        completion: None,
                    },
                }),
            );
        }
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].text, "first text\n\n世界 second");
        assert!(state.items[0].complete);
        assert_eq!(state.resolve_item_index("second"), Some(0));
    }

    #[test]
    fn merged_agent_final_snapshot_replaces_only_its_own_segment() {
        let mut state = ChatState::default();
        let turn_id = TurnId("turn".into());
        state.apply_protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::TurnStarted {
                thread_id: ThreadId("thread".into()),
                turn_id: turn_id.clone(),
            },
        });
        let complete = |id: &str, text: &str| EventKind::ItemCompleted {
            item_id: id.into(),
            completion: Some(nickel_codex::CompletedItem {
                thread_id: ThreadId("thread".into()),
                turn_id: turn_id.clone(),
                completed_at_ms: Some(42),
                item_type: "agentMessage".into(),
                text: text.into(),
                status: None,
                exit_code: None,
                duration_ms: None,
                changes: Vec::new(),
                summary_parts: Vec::new(),
                command_actions: Vec::new(),
            }),
        };
        for (index, id) in ["first", "second"].into_iter().enumerate() {
            state.apply_protocol(CodexEvent {
                sequence: (index * 3 + 2) as u64,
                kind: EventKind::ItemStarted {
                    thread_id: Some(ThreadId("thread".into())),
                    turn_id: Some(turn_id.clone()),
                    item_id: id.into(),
                    item_type: "agentMessage".into(),
                    command_actions: Vec::new(),
                    initial_text: String::new(),
                },
            });
            state.apply_protocol(CodexEvent {
                sequence: (index * 3 + 3) as u64,
                kind: EventKind::AgentMessageDelta {
                    item_id: id.into(),
                    delta: "partial".into(),
                },
            });
            state.apply_protocol(CodexEvent {
                sequence: (index * 3 + 4) as u64,
                kind: complete(id, &format!("{id} final")),
            });
        }
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].text, "first final\n\nsecond final");
        state.apply_protocol(CodexEvent {
            sequence: 9,
            kind: complete("first", "stale overwrite"),
        });
        state.apply_protocol(CodexEvent {
            sequence: 10,
            kind: EventKind::AgentMessageDelta {
                item_id: "first".into(),
                delta: "stale delta".into(),
            },
        });
        assert_eq!(state.items[0].text, "first final\n\nsecond final");
    }

    #[test]
    fn assistant_items_do_not_merge_across_an_operation_live_or_resumed() {
        let mut live = ChatState::default();
        let turn_id = TurnId("turn".into());
        live.apply_protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::TurnStarted {
                thread_id: ThreadId("thread".into()),
                turn_id: turn_id.clone(),
            },
        });
        for (sequence, item_id, item_type, text) in [
            (2, "before", "agentMessage", "Before"),
            (4, "command", "commandExecution", "cargo test"),
            (6, "after", "agentMessage", "After"),
        ] {
            live.apply_protocol(CodexEvent {
                sequence,
                kind: EventKind::ItemStarted {
                    thread_id: Some(ThreadId("thread".into())),
                    turn_id: Some(turn_id.clone()),
                    item_id: item_id.into(),
                    item_type: item_type.into(),
                    command_actions: Vec::new(),
                    initial_text: text.into(),
                },
            });
            live.apply_protocol(CodexEvent {
                sequence: sequence + 1,
                kind: EventKind::ItemCompleted {
                    item_id: item_id.into(),
                    completion: None,
                },
            });
        }
        let expected = ["Before", "cargo test", "After"];
        assert_eq!(
            live.items
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            expected
        );

        let mut resumed = ChatState::default();
        resumed.hydrate_thread(&Thread {
            id: ThreadId("thread".into()),
            title: None,
            cwd: None,
            last_used_at: None,
            turns: vec![nickel_codex::ThreadHistoryTurn {
                id: turn_id,
                status: "completed".into(),
                items: [
                    ("before", "agentMessage", "Before"),
                    ("command", "commandExecution", "cargo test"),
                    ("after", "agentMessage", "After"),
                ]
                .into_iter()
                .map(|(id, item_type, text)| nickel_codex::ThreadHistoryItem {
                    id: id.into(),
                    item_type: item_type.into(),
                    text: text.into(),
                    command_actions: Vec::new(),
                    status: (id == "command").then(|| "failed".into()),
                    exit_code: (id == "command").then_some(2),
                    duration_ms: (id == "command").then_some(17),
                })
                .collect(),
            }],
            model: None,
            reasoning_effort: None,
        });
        assert_eq!(
            resumed
                .items
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            resumed.activity_outcomes.get("command"),
            Some(&ActivityOutcome {
                status: Some("failed".into()),
                exit_code: Some(2),
                duration_ms: Some(17),
            })
        );
    }

    #[test]
    fn structured_patch_and_turn_plan_snapshots_replace_in_place() {
        let mut state = ChatState {
            selected_thread: Some(ThreadId("thread".into())),
            ..Default::default()
        };
        let turn_id = TurnId("turn".into());
        state.apply_protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::TurnStarted {
                thread_id: ThreadId("thread".into()),
                turn_id: turn_id.clone(),
            },
        });
        let patch = |diff: &str| EventKind::FilePatchUpdated {
            thread_id: ThreadId("thread".into()),
            turn_id: turn_id.clone(),
            item_id: "file".into(),
            changes: vec![nickel_codex::FilePatchChange {
                path: "/project/file".into(),
                kind: "update".into(),
                move_path: None,
                diff: diff.into(),
            }],
        };
        state.apply_protocol(CodexEvent {
            sequence: 2,
            kind: patch("+first"),
        });
        state.apply_protocol(CodexEvent {
            sequence: 3,
            kind: EventKind::ItemStarted {
                thread_id: Some(ThreadId("thread".into())),
                turn_id: Some(turn_id.clone()),
                item_id: "file".into(),
                item_type: "fileChange".into(),
                command_actions: Vec::new(),
                initial_text: String::new(),
            },
        });
        state.apply_protocol(CodexEvent {
            sequence: 4,
            kind: patch("+second"),
        });
        assert_eq!(state.items.len(), 1);
        assert!(state.items[0].text.contains("+second"));
        assert!(!state.items[0].text.contains("+first"));
        assert_eq!(
            state.file_change_summaries.get("file").map(String::as_str),
            Some("update: /project/file")
        );
        state.apply_protocol(CodexEvent {
            sequence: 5,
            kind: EventKind::FilePatchUpdated {
                thread_id: ThreadId("thread".into()),
                turn_id: turn_id.clone(),
                item_id: "file".into(),
                changes: Vec::new(),
            },
        });
        assert!(!state.file_change_summaries.contains_key("file"));
        let plan = |status: &str| EventKind::TurnPlanUpdated {
            thread_id: ThreadId("thread".into()),
            turn_id: turn_id.clone(),
            explanation: None,
            steps: vec![nickel_codex::TurnPlanStep {
                step: "Test".into(),
                status: status.into(),
            }],
        };
        state.apply_protocol(CodexEvent {
            sequence: 6,
            kind: plan("pending"),
        });
        state.apply_protocol(CodexEvent {
            sequence: 7,
            kind: plan("completed"),
        });
        assert_eq!(state.items.len(), 2);
        assert_eq!(state.items[1].kind, ChatItemKind::Plan);
        assert_eq!(state.items[1].text, "[completed] Test");
        state.remove_item(0);
        assert!(!state.file_change_summaries.contains_key("file"));
    }

    #[test]
    fn completed_snapshot_replaces_deltas_and_restores_final_only_items() {
        let mut state = ChatState {
            selected_thread: Some(ThreadId("thread".into())),
            ..Default::default()
        };
        let complete = |id: &str, text: &str| EventKind::ItemCompleted {
            item_id: id.into(),
            completion: Some(nickel_codex::CompletedItem {
                thread_id: ThreadId("thread".into()),
                turn_id: TurnId("turn".into()),
                completed_at_ms: Some(42),
                item_type: "commandExecution".into(),
                text: text.into(),
                status: Some("completed".into()),
                exit_code: Some(0),
                duration_ms: Some(12),
                changes: Vec::new(),
                summary_parts: Vec::new(),
                command_actions: Vec::new(),
            }),
        };
        state.apply_protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::CommandOutputDelta {
                item_id: "streamed".into(),
                delta: "partial".into(),
            },
        });
        state.apply_protocol(CodexEvent {
            sequence: 2,
            kind: complete("streamed", "$ pwd\n/project"),
        });
        state.apply_protocol(CodexEvent {
            sequence: 3,
            kind: complete("final-only", "$ date\nToday"),
        });
        assert_eq!(state.items.len(), 2);
        assert_eq!(state.items[0].text, "$ pwd\n/project");
        assert_eq!(state.items[1].text, "$ date\nToday");
        assert!(state.items.iter().all(|item| item.complete));
        assert_eq!(state.activity_outcomes["streamed"].exit_code, Some(0));
        assert_eq!(state.activity_outcomes["final-only"].duration_ms, Some(12));
        state.remove_item(0);
        assert!(!state.activity_outcomes.contains_key("streamed"));
        assert!(state.activity_outcomes.contains_key("final-only"));
        state.clear_conversation();
        assert!(state.activity_outcomes.is_empty());
    }

    #[test]
    fn final_only_file_change_uses_typed_targets_not_diff_lines() {
        let mut state = ChatState {
            selected_thread: Some(ThreadId("thread".into())),
            ..Default::default()
        };
        state.apply_protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::ItemCompleted {
                item_id: "final-file".into(),
                completion: Some(nickel_codex::CompletedItem {
                    thread_id: ThreadId("thread".into()),
                    turn_id: TurnId("turn".into()),
                    completed_at_ms: None,
                    item_type: "fileChange".into(),
                    text: "+not-a-path\n-previous line".into(),
                    status: Some("completed".into()),
                    exit_code: None,
                    duration_ms: None,
                    changes: vec![FilePatchChange {
                        kind: "update".into(),
                        path: "/project/real.rs".into(),
                        move_path: None,
                        diff: "+not-a-path".into(),
                    }],
                    summary_parts: Vec::new(),
                    command_actions: Vec::new(),
                }),
            },
        });
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].text, "+not-a-path\n-previous line");
        assert_eq!(
            state
                .file_change_summaries
                .get("final-file")
                .map(String::as_str),
            Some("update: /project/real.rs")
        );
        state.clear_conversation();
        assert!(state.file_change_summaries.is_empty());
    }

    #[test]
    fn retrying_turn_error_keeps_connection_healthy_and_final_failure_is_distinct() {
        let mut state = ChatState {
            status: ConnectionStatus::Ready,
            ..Default::default()
        };
        state.account.authenticated = true;
        state.selected_thread = Some(ThreadId("thread".into()));
        state.active_turn = Some(TurnId("turn".into()));
        state.apply_protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::TurnError {
                thread_id: ThreadId("thread".into()),
                turn_id: TurnId("turn".into()),
                message: "Temporary outage".into(),
                will_retry: true,
            },
        });
        assert_eq!(state.status, ConnectionStatus::Ready);
        assert_eq!(
            state.run_presentation_status(),
            RunPresentationStatus::Retrying
        );
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].kind, ChatItemKind::Error);
        assert!(state.items[0].text.starts_with("Codex is retrying"));
        let error_item_id = state.items[0].id.clone();
        state.apply_protocol(CodexEvent {
            sequence: 2,
            kind: EventKind::TurnError {
                thread_id: ThreadId("thread".into()),
                turn_id: TurnId("turn".into()),
                message: "Final failure".into(),
                will_retry: false,
            },
        });
        state.apply_protocol(CodexEvent {
            sequence: 3,
            kind: EventKind::TurnCompleted {
                thread_id: ThreadId("thread".into()),
                turn_id: TurnId("turn".into()),
                status: "failed".into(),
            },
        });
        assert_eq!(state.status, ConnectionStatus::Ready);
        assert_eq!(
            state.run_presentation_status(),
            RunPresentationStatus::TurnFailed
        );
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].id, error_item_id);
        assert_eq!(state.items[0].text, "Turn failed: Final failure");
        assert!(state.items[0].complete);
    }

    #[test]
    fn successful_retry_closes_error_card_once() {
        let mut state = ChatState {
            selected_thread: Some(ThreadId("thread".into())),
            active_turn: Some(TurnId("turn".into())),
            ..Default::default()
        };
        state.apply_protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::TurnError {
                thread_id: ThreadId("thread".into()),
                turn_id: TurnId("turn".into()),
                message: "Temporary outage".into(),
                will_retry: true,
            },
        });
        for sequence in [2, 3] {
            state.apply_protocol(CodexEvent {
                sequence,
                kind: EventKind::TurnCompleted {
                    thread_id: ThreadId("thread".into()),
                    turn_id: TurnId("turn".into()),
                    status: "completed".into(),
                },
            });
        }
        assert_eq!(state.items.len(), 1);
        assert!(state.items[0].complete);
        assert_eq!(state.items[0].text.matches("Retry succeeded").count(), 1);
    }

    #[test]
    fn replacement_is_blocked_by_starting_active_and_pending_interactions() {
        let mut state = ChatState::default();
        assert_eq!(state.replacement_block_reason(), None);
        state.send_pending = true;
        assert!(state.replacement_block_reason().unwrap().contains("start"));
        state.send_pending = false;
        state.active_turn = Some(TurnId("turn".into()));
        assert!(
            state
                .replacement_block_reason()
                .unwrap()
                .contains("Interrupt")
        );
        state.active_turn = None;
        state.pending.push(PendingInteraction::Approval {
            request_id: ServerRequestId("request".into()),
            approval_type: "item/fileChange/requestApproval".into(),
            summary: "Change files".into(),
            context: nickel_codex::ApprovalContext::default(),
        });
        assert!(
            state
                .replacement_block_reason()
                .unwrap()
                .contains("Resolve")
        );
    }

    #[test]
    fn collapsed_command_uses_bounded_virtual_height_until_expanded() {
        let mut state = ChatState::default();
        state.push_item(ChatItem {
            id: "command".into(),
            kind: ChatItemKind::Command,
            text: format!("$ cargo test\n{}", "output\n".repeat(10_000)),
            complete: true,
        });
        assert_eq!(state.estimated_item_heights(), vec![104.0]);
        state.expanded_items.insert("command".into());
        assert!(state.estimated_item_heights()[0] > 104.0);
    }

    #[test]
    fn unknown_methods_coalesce_in_diagnostics_without_flooding_transcript() {
        let mut state = ChatState::default();
        for sequence in 1..=1000 {
            state.apply_protocol(CodexEvent {
                sequence,
                kind: EventKind::UnsupportedEvent {
                    method: "future/status".into(),
                },
            });
        }
        assert!(state.items.is_empty());
        assert_eq!(state.unsupported_method_counts["future/status"], 1000);
        assert!(state.diagnostics.len() <= 10);
        assert!(state.diagnostics.back().unwrap().contains("512"));
        for index in 0..40 {
            state.apply_protocol(CodexEvent {
                sequence: 1001 + index,
                kind: EventKind::UnsupportedEvent {
                    method: format!("future/{index}"),
                },
            });
        }
        state.apply_protocol(CodexEvent {
            sequence: 1041,
            kind: EventKind::UnsupportedEvent {
                method: "future/status".into(),
            },
        });
        assert_eq!(state.unsupported_method_counts["future/status"], 1001);
        assert!(state.unsupported_method_counts.len() <= 33);
    }

    #[test]
    fn app_server_activity_families_are_not_mislabeled_as_unknown() {
        for (wire, kind) in [
            ("mcpToolCall", ChatItemKind::Tool),
            ("dynamicToolCall", ChatItemKind::Tool),
            ("webSearch", ChatItemKind::Search),
            ("imageGeneration", ChatItemKind::Image),
            ("collabAgentToolCall", ChatItemKind::Delegation),
            ("subAgentActivity", ChatItemKind::Delegation),
            ("contextCompaction", ChatItemKind::SessionNotice),
        ] {
            assert_eq!(chat_item_kind(wire), kind);
        }
    }

    #[test]
    fn thread_pages_advance_without_silently_discarding_unreachable_history() {
        let thread = |index: usize| Thread {
            id: ThreadId(format!("thread-{index}")),
            title: Some(format!("Conversation {index}")),
            cwd: None,
            last_used_at: Some(1000 - index as i64),
            turns: Vec::new(),
            model: None,
            reasoning_effort: None,
        };
        let mut state = ChatState::default();
        state.apply(
            1,
            ControllerEvent::Ready {
                provenance: "fixture".into(),
                account: AccountState {
                    authenticated: true,
                    ..Default::default()
                },
                models: Vec::new(),
                projects: Vec::new(),
                threads: (0..100).map(thread).collect(),
                runtime: HashMap::new(),
                thread_error: None,
                thread_next_cursor: Some("100".into()),
            },
        );
        assert_eq!(state.begin_thread_page(), Some(("100".into(), 1)));
        state.apply(
            1,
            ControllerEvent::ThreadPageLoaded {
                request: 1,
                cursor: "stale".into(),
                threads: vec![thread(900)],
                runtime: HashMap::new(),
                next_cursor: None,
            },
        );
        assert_eq!(state.threads.len(), 100);
        state.apply(
            1,
            ControllerEvent::ThreadPageLoaded {
                request: 1,
                cursor: "100".into(),
                threads: (100..200).map(thread).collect(),
                runtime: HashMap::new(),
                next_cursor: Some("200".into()),
            },
        );
        assert_eq!(state.threads.len(), 200);
        assert_eq!(state.begin_thread_page(), Some(("200".into(), 2)));
        state.apply(
            1,
            ControllerEvent::ThreadPageLoaded {
                request: 2,
                cursor: "200".into(),
                threads: (200..250).map(thread).collect(),
                runtime: HashMap::new(),
                next_cursor: None,
            },
        );
        assert_eq!(state.threads.len(), 200);
        assert!(state.thread_windowed);
        assert_eq!(state.threads.first().unwrap().id.0, "thread-50");
        assert_eq!(state.threads.last().unwrap().id.0, "thread-249");
        assert!(state.thread_next_cursor.is_none());
    }

    #[test]
    fn refreshed_thread_page_rejects_old_response_even_when_cursor_matches() {
        let ready = || ControllerEvent::Ready {
            provenance: "fixture".into(),
            account: AccountState {
                authenticated: true,
                ..Default::default()
            },
            models: Vec::new(),
            projects: Vec::new(),
            threads: Vec::new(),
            runtime: HashMap::new(),
            thread_error: None,
            thread_next_cursor: Some("100".into()),
        };
        let mut state = ChatState::default();
        state.apply(1, ready());
        assert_eq!(state.begin_thread_page(), Some(("100".into(), 1)));
        state.apply(1, ready());
        assert_eq!(state.begin_thread_page(), Some(("100".into(), 2)));
        state.apply(
            1,
            ControllerEvent::ThreadPageFailed {
                request: 1,
                cursor: "100".into(),
                message: "stale failure".into(),
            },
        );
        assert!(state.thread_paging);
        assert!(state.thread_page_error.is_none());
        state.apply(
            1,
            ControllerEvent::ThreadPageFailed {
                request: 2,
                cursor: "100".into(),
                message: "current failure".into(),
            },
        );
        assert!(!state.thread_paging);
        assert_eq!(state.thread_page_error.as_deref(), Some("current failure"));
    }

    #[test]
    fn transcript_capacity_plateaus_and_evicted_late_deltas_stay_retired() {
        let mut state = ChatState::default();
        for index in 0..100 {
            state.push_item(ChatItem {
                id: format!("item-{index}"),
                kind: ChatItemKind::Agent,
                text: "x".repeat(MAX_ITEM_TEXT_BYTES),
                complete: true,
            });
            assert!(
                state
                    .items
                    .iter()
                    .map(|item| item.text.capacity())
                    .sum::<usize>()
                    <= MAX_TRANSCRIPT_TEXT_BYTES
            );
        }
        let retained = state.items.len();
        state.append_delta("item-0".into(), "stale".into(), ChatItemKind::Agent);
        assert_eq!(state.items.len(), retained);
        let cached = state.transcript_selection_document();
        let weak = Arc::downgrade(&cached);
        drop(cached);
        state.new_chat();
        assert!(weak.upgrade().is_none());
        assert!(state.retired_item_ids.is_empty());
    }

    #[test]
    fn pending_approval_outlives_evicted_transcript_reference() {
        let mut state = ChatState {
            status: ConnectionStatus::Ready,
            ..Default::default()
        };
        state.account.authenticated = true;
        state.selected_thread = Some(ThreadId("thread".into()));
        let request_id = ServerRequestId("pending-after-eviction".into());
        state.push_pending(PendingInteraction::Approval {
            request_id: request_id.clone(),
            approval_type: "item/commandExecution/requestApproval".into(),
            summary: "Run a command".into(),
            context: nickel_codex::ApprovalContext::default(),
        });
        assert!(!state.interaction_item_ids.contains_key(&request_id));
        // A pending approval does not need a transcript reference. It remains
        // authoritative even when the transcript fills and evicts older items.
        for index in 0..MAX_ITEMS {
            state.push_item(ChatItem {
                id: format!("running-{index}"),
                kind: ChatItemKind::Agent,
                text: "streaming".into(),
                complete: false,
            });
        }
        assert!(state.interaction_is_actionable(&request_id));
        assert_eq!(state.pending.len(), 1);
        state.apply_protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::ServerRequestResolved {
                thread_id: ThreadId("thread".into()),
                request_id: request_id.clone(),
            },
        });
        assert!(state.pending.is_empty());
        assert!(!state.interaction_is_actionable(&request_id));
    }

    #[test]
    fn pathological_markdown_expansion_uses_visible_bounded_plain_text() {
        let mut state = ChatState::default();
        state.push_item(ChatItem {
            id: "i".repeat(4096),
            kind: ChatItemKind::Agent,
            text: "**x** ".repeat(1000),
            complete: true,
        });
        let document = state.markdown_document(0);
        assert!(document.source.contains("Formatting omitted"));
        let cache = state.item_selection_runs[0].cached.borrow();
        let cache = cache.as_ref().unwrap();
        assert!(
            crate::projection_memory::derived_capacity(&document, &cache.runs)
                <= 2048 + 16 * state.items[0].text.capacity() + 4 * state.items[0].id.capacity()
        );
    }

    #[test]
    #[ignore = "release streaming comparison; run with --release --ignored --nocapture"]
    fn streaming_projection_measurement() {
        for bytes in [4096, 32768, 131072] {
            let chunk = "Some **text** and Unicode 世界. ".repeat(2);
            let count = bytes / chunk.len();
            let mut legacy = ChatItem {
                id: "stream".into(),
                kind: ChatItemKind::Agent,
                text: String::new(),
                complete: false,
            };
            let started = Instant::now();
            for _ in 0..count {
                legacy.text.push_str(&chunk);
                black_box(selection_runs_for_item(&legacy));
            }
            let legacy_elapsed = started.elapsed();
            let mut state = ChatState::default();
            let started = Instant::now();
            for index in 0..count {
                delta(&mut state, &chunk);
                if (index + 1) % 128 == 0 {
                    black_box(state.transcript_selection_document().runs());
                }
            }
            let selected = state.transcript_selection_document();
            black_box(selected.runs());
            let elapsed = started.elapsed();
            let builds = state.item_selection_runs[0].builds.get();
            assert_eq!(
                selected.runs(),
                SelectionDocument::new(selection_runs_for_item(&legacy)).runs()
            );
            assert_eq!(builds as usize, count.div_ceil(128));
            let cache = state.item_selection_runs[0].cached.borrow();
            let cache = cache.as_ref().unwrap();
            let retained = state.items[0].text.capacity()
                + crate::projection_memory::derived_capacity(&cache.document, &cache.runs);
            eprintln!(
                "streaming bytes={} deltas={count} legacy_rebuilds={count} batched_rebuilds={builds} legacy_us={} batched_us={} retained_capacity={retained}",
                legacy.text.len(),
                legacy_elapsed.as_micros(),
                elapsed.as_micros()
            );
            assert!(
                elapsed < legacy_elapsed / 4,
                "batching should reduce this repeated-parse workload by at least 75%"
            );
        }
    }

    const TINY_DERIVED_OPERATION_P95_ADDITION: std::time::Duration =
        std::time::Duration::from_micros(100);

    #[test]
    fn failed_send_retains_unicode_draft_and_images_until_turn_is_accepted() {
        let mut state = ChatState {
            status: ConnectionStatus::Ready,
            draft: "hello 世界".into(),
            ..ChatState::default()
        };
        state.account.authenticated = true;
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
            .flat_map(|projection| {
                projection
                    .cached
                    .borrow()
                    .as_ref()
                    .map(|cached| cached.runs.clone())
                    .unwrap_or_default()
            })
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
        assert_eq!(cached.runs(), recomputed.runs());

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
    fn rejected_selected_model_falls_back_visibly_without_losing_the_draft() {
        let mut state = ChatState {
            status: ConnectionStatus::Ready,
            models: vec![
                Model {
                    id: "rejected".into(),
                    display_name: "Rejected".into(),
                    default_reasoning_effort: Some("high".into()),
                    supported_reasoning_efforts: Vec::new(),
                },
                Model {
                    id: "fallback".into(),
                    display_name: "Fallback".into(),
                    default_reasoning_effort: Some("medium".into()),
                    supported_reasoning_efforts: Vec::new(),
                },
            ],
            selected_model: Some("rejected".into()),
            selected_reasoning_effort: Some("high".into()),
            draft: "keep this retry".into(),
            ..Default::default()
        };
        state.account.authenticated = true;
        assert!(state.begin_send().is_some());

        state.apply(
            state.generation,
            ControllerEvent::ModelRejected {
                model: "rejected".into(),
                message: "unknown model rejected".into(),
            },
        );

        assert_eq!(state.selected_model.as_deref(), Some("fallback"));
        assert_eq!(state.selected_reasoning_effort.as_deref(), Some("medium"));
        assert_eq!(state.draft, "keep this retry");
        assert!(state.can_send());
        assert!(state.diagnostics.back().is_some_and(|message| {
            message.contains("selected model was rejected")
                && message.contains("unknown model rejected")
        }));
    }

    #[test]
    fn late_rejection_cannot_replace_a_newer_explicit_model_choice() {
        let mut state = ChatState {
            models: vec![Model {
                id: "new-choice".into(),
                display_name: "New choice".into(),
                default_reasoning_effort: Some("medium".into()),
                supported_reasoning_efforts: Vec::new(),
            }],
            selected_model: Some("new-choice".into()),
            selected_reasoning_effort: Some("medium".into()),
            ..Default::default()
        };

        state.apply(
            state.generation,
            ControllerEvent::ModelRejected {
                model: "old-choice".into(),
                message: "unknown model old-choice".into(),
            },
        );

        assert_eq!(state.selected_model.as_deref(), Some("new-choice"));
        assert_eq!(state.selected_reasoning_effort.as_deref(), Some("medium"));
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
                .flat_map(|projection| projection.cached.borrow().as_ref().unwrap().runs.clone())
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
                SelectionDocument::new(state.item_selection_runs.iter().flat_map(|projection| {
                    projection.cached.borrow().as_ref().unwrap().runs.clone()
                }));
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
