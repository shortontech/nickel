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
use twinkle_ui::{SelectionDocument, SelectionRun};

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
    /// Changes whenever the accepted backend project snapshot changes.
    pub project_revision: u64,
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
    preserve_draft_on_turn_accept: bool,
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
            project_revision: 0,
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
            preserve_draft_on_turn_accept: false,
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
        self.preserve_draft_on_turn_accept = false;
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

    pub fn begin_queued_send(&mut self, text: &str) -> bool {
        if self.status != ConnectionStatus::Ready
            || !self.account.authenticated
            || self.recovery_pending
            || self.unconfirmed_work
            || self.active_turn.is_some()
            || self.send_pending
        {
            return false;
        }
        self.send_pending = true;
        self.preserve_draft_on_turn_accept = true;
        self.local_sequence = self.local_sequence.wrapping_add(1);
        self.push_item(ChatItem {
            id: format!("local-user-{}", self.local_sequence),
            kind: ChatItemKind::User,
            text: text.to_owned(),
            complete: true,
        });
        true
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
                let projects = projects.into_iter().take(100).collect::<Vec<_>>();
                if self.projects != projects {
                    self.project_revision = self.project_revision.wrapping_add(1);
                    self.projects = projects;
                }
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
                    if !self.preserve_draft_on_turn_accept {
                        self.draft.clear();
                        self.attachments.clear();
                    }
                    self.send_pending = false;
                    self.preserve_draft_on_turn_accept = false;
                }
            }
            ControllerEvent::TurnStartFailed(message) => {
                self.send_pending = false;
                self.preserve_draft_on_turn_accept = false;
                self.push_diagnostic(format!("Message could not be sent: {message}"));
            }
            ControllerEvent::InterruptAccepted => {}
            ControllerEvent::InterruptFailed(message) => {
                self.interrupt_requested = false;
                self.push_diagnostic(format!("Interrupt failed: {message}"));
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
                if !self.projects.is_empty() {
                    self.project_revision = self.project_revision.wrapping_add(1);
                    self.projects.clear();
                }
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
        // Resume responses can contain the entire server transcript. Project only the
        // newest locally retainable suffix instead of constructing and then evicting
        // every historical item on the UI thread. The authoritative server history is
        // unchanged and remains available through a later resume.
        let total_items = thread
            .turns
            .iter()
            .map(|turn| turn.items.len())
            .sum::<usize>();
        let mut items_to_skip = total_items.saturating_sub(MAX_ITEMS);
        for turn in &thread.turns {
            self.clear_exploration();
            let mut turn_agent_id: Option<String> = None;
            let skip = items_to_skip.min(turn.items.len());
            items_to_skip -= skip;
            for item in turn.items.iter().skip(skip) {
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
mod tests;
