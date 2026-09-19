use std::{
    collections::HashMap,
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Stdio},
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use serde_json::{Value, json};
use tungstenite::{
    Message,
    client::IntoClientRequest,
    connect,
    http::{HeaderValue, header::AUTHORIZATION},
    stream::MaybeTlsStream,
};
use url::Url;

use crate::process::command;
use crate::protocol::*;

const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
const OUTBOUND_BACKLOG: usize = 16;
const MAX_OUTBOUND_BYTES: usize = 160 * 1024 * 1024;
const MAX_HISTORY_ITEM_TEXT_BYTES: usize = 256 * 1024;
const HISTORY_OMISSION_MARKER: &str =
    "\n\n[Further history detail omitted from this local view; server history is unchanged.]";

fn bound_history_item_text(mut text: String) -> String {
    if text.len() > MAX_HISTORY_ITEM_TEXT_BYTES {
        let mut end = MAX_HISTORY_ITEM_TEXT_BYTES - HISTORY_OMISSION_MARKER.len();
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(HISTORY_OMISSION_MARKER);
    }
    if text.capacity() > MAX_HISTORY_ITEM_TEXT_BYTES {
        text.shrink_to_fit();
    }
    text
}

fn parse_command_action(value: &Value) -> CommandAction {
    let string = |name: &str| value.get(name).and_then(Value::as_str).map(str::to_owned);
    match value.get("type").and_then(Value::as_str) {
        Some("read") => CommandAction::Read {
            name: string("name").unwrap_or_else(|| "file".into()),
            path: string("path").unwrap_or_default(),
        },
        Some("listFiles") => CommandAction::ListFiles {
            path: string("path"),
        },
        Some("search") => CommandAction::Search {
            query: string("query"),
            path: string("path"),
        },
        _ => CommandAction::Unknown,
    }
}

fn parse_user_input_questions(params: &Value) -> Option<Vec<UserInputQuestion>> {
    let raw = params.get("questions")?.as_array()?;
    if raw.is_empty() || raw.len() > 32 {
        return None;
    }
    let mut retained_bytes = 0usize;
    let mut questions = Vec::with_capacity(raw.len());
    for value in raw {
        let id = value.get("id")?.as_str()?;
        let header = value.get("header")?.as_str()?;
        let question = value.get("question")?.as_str()?;
        let raw_options = value.get("options").and_then(Value::as_array);
        if raw_options.is_some_and(|options| options.len() > 16) {
            return None;
        }
        let mut options = Vec::new();
        for option in raw_options.into_iter().flatten() {
            let label = option.get("label")?.as_str()?;
            let description = option.get("description")?.as_str()?;
            retained_bytes = retained_bytes.checked_add(label.len() + description.len())?;
            options.push(UserInputOption {
                label: label.into(),
                description: description.into(),
            });
        }
        retained_bytes = retained_bytes.checked_add(id.len() + header.len() + question.len())?;
        if id.len() > 256 || retained_bytes > 32 * 1024 {
            return None;
        }
        questions.push(UserInputQuestion {
            id: id.into(),
            header: header.into(),
            question: question.into(),
            options,
            is_other: value
                .get("isOther")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            is_secret: value
                .get("isSecret")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    Some(questions)
}

fn parse_available_decisions(
    params: &Value,
) -> Result<Option<Vec<crate::CommandDecision>>, &'static str> {
    let Some(raw) = params
        .get("availableDecisions")
        .filter(|raw| !raw.is_null())
    else {
        return Ok(None);
    };
    if raw.to_string().len() > 4096 {
        return Err("Approval decision set exceeds local limit");
    }
    let decisions = serde_json::from_value::<Vec<crate::CommandDecision>>(raw.clone())
        .map_err(|_| "Unsupported approval decision set")?;
    if decisions.len() > 8 {
        return Err("Approval decision set exceeds local limit");
    }
    Ok(Some(decisions))
}

fn parse_file_changes(value: &Value) -> Option<Vec<FilePatchChange>> {
    let raw = value.as_array()?;
    if raw.len() > 64 {
        return None;
    }
    let mut retained_bytes = 0usize;
    let mut changes = Vec::with_capacity(raw.len());
    for change in raw {
        let path = change.get("path")?.as_str()?;
        let kind_value = change.get("kind")?;
        let kind = kind_value.get("type")?.as_str()?;
        let move_path = kind_value.get("move_path").and_then(Value::as_str);
        let diff = change.get("diff")?.as_str()?;
        retained_bytes = retained_bytes
            .checked_add(path.len() + kind.len() + diff.len() + move_path.map_or(0, str::len))?;
        if path.len() > 4096
            || move_path.is_some_and(|path| path.len() > 4096)
            || kind.len() > 256
            || retained_bytes > 256 * 1024
        {
            return None;
        }
        changes.push(FilePatchChange {
            path: path.into(),
            kind: kind.into(),
            move_path: move_path.map(str::to_owned),
            diff: diff.into(),
        });
    }
    Some(changes)
}

fn format_file_changes(changes: &[FilePatchChange]) -> String {
    changes
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
        .join("\n\n")
}

fn parse_file_patch_updated(params: &Value) -> EventKind {
    let parsed = (|| {
        let thread_id = params.get("threadId")?.as_str()?;
        let turn_id = params.get("turnId")?.as_str()?;
        let item_id = params.get("itemId")?.as_str()?;
        if [thread_id, turn_id, item_id]
            .iter()
            .any(|id| id.len() > 4096)
        {
            return None;
        }
        let changes = parse_file_changes(params.get("changes")?)?;
        Some(EventKind::FilePatchUpdated {
            thread_id: ThreadId(thread_id.into()),
            turn_id: TurnId(turn_id.into()),
            item_id: item_id.into(),
            changes,
        })
    })();
    parsed.unwrap_or_else(|| EventKind::Inconsistency {
        message: "File patch update was malformed or exceeded the local detail budget".into(),
    })
}

fn parse_turn_plan_updated(params: &Value) -> EventKind {
    let parsed = (|| {
        let thread_id = params.get("threadId")?.as_str()?;
        let turn_id = params.get("turnId")?.as_str()?;
        let raw = params.get("plan")?.as_array()?;
        let explanation = params.get("explanation").and_then(Value::as_str);
        if raw.len() > 128
            || thread_id.len() > 4096
            || turn_id.len() > 4096
            || explanation.is_some_and(|text| text.len() > 4096)
        {
            return None;
        }
        let mut retained_bytes = explanation.map_or(0, str::len);
        let mut steps = Vec::with_capacity(raw.len());
        for entry in raw {
            let step = entry.get("step")?.as_str()?;
            let status = entry.get("status")?.as_str()?;
            retained_bytes = retained_bytes.checked_add(step.len() + status.len())?;
            if step.len() > 4096
                || retained_bytes > 128 * 4096
                || !matches!(status, "pending" | "inProgress" | "completed")
            {
                return None;
            }
            steps.push(TurnPlanStep {
                step: step.into(),
                status: status.into(),
            });
        }
        Some(EventKind::TurnPlanUpdated {
            thread_id: ThreadId(thread_id.into()),
            turn_id: TurnId(turn_id.into()),
            explanation: explanation.map(str::to_owned),
            steps,
        })
    })();
    parsed.unwrap_or_else(|| EventKind::Inconsistency {
        message: "Turn plan update was malformed or exceeded the local detail budget".into(),
    })
}

fn parse_item_completed(params: &Value) -> EventKind {
    let parsed = (|| {
        let thread_id = params.get("threadId")?.as_str()?;
        let turn_id = params.get("turnId")?.as_str()?;
        let completed_at_ms = params.get("completedAtMs")?.as_i64()?;
        let item = params.get("item")?;
        let item_id = item.get("id")?.as_str()?;
        let item_type = item.get("type")?.as_str()?;
        if [thread_id, turn_id, item_id]
            .iter()
            .any(|id| id.len() > 4096)
            || item_type.len() > 256
        {
            return None;
        }
        let summary_parts = if item_type == "reasoning" {
            let raw = item.get("summary")?.as_array()?;
            if raw.len() > 64 {
                return None;
            }
            raw.iter()
                .map(|part| part.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?
        } else {
            Vec::new()
        };
        if summary_parts.iter().map(String::len).sum::<usize>() > 256 * 1024 {
            return None;
        }
        let changes = if item_type == "fileChange" {
            parse_file_changes(item.get("changes")?)?
        } else {
            Vec::new()
        };
        // A completion is an authoritative snapshot, but it must still fit the local view budget.
        let command_actions = match item.get("commandActions").and_then(Value::as_array) {
            Some(raw) if raw.len() <= 32 && serde_json::to_string(raw).ok()?.len() <= 32 * 1024 => {
                raw.iter().map(parse_command_action).collect()
            }
            Some(_) => return None,
            None => Vec::new(),
        };
        let text = match item_type {
            "reasoning" => summary_parts.join("\n"),
            "fileChange" => format_file_changes(&changes),
            _ => history_item_text(item_type, item),
        };
        if text.len() > 256 * 1024 {
            return None;
        }
        let exit_code = item
            .get("exitCode")
            .and_then(Value::as_i64)
            .map(i32::try_from)
            .transpose()
            .ok()?;
        let status = item.get("status").and_then(Value::as_str);
        if status.is_some_and(|status| status.len() > 256) {
            return None;
        }
        Some(EventKind::ItemCompleted {
            item_id: item_id.into(),
            completion: Some(CompletedItem {
                thread_id: ThreadId(thread_id.into()),
                turn_id: TurnId(turn_id.into()),
                completed_at_ms: Some(completed_at_ms),
                item_type: item_type.into(),
                text,
                status: status.map(str::to_owned),
                exit_code,
                duration_ms: item.get("durationMs").and_then(Value::as_i64),
                changes,
                summary_parts,
                command_actions,
            }),
        })
    })();
    parsed.unwrap_or_else(|| EventKind::Inconsistency {
        message: "Completed item was malformed or exceeded the local detail budget".into(),
    })
}

fn parse_turn_error(params: &Value) -> EventKind {
    let parsed = (|| {
        let thread_id = params.get("threadId")?.as_str()?;
        let turn_id = params.get("turnId")?.as_str()?;
        let message = params.get("error")?.get("message")?.as_str()?;
        let will_retry = params.get("willRetry")?.as_bool()?;
        if thread_id.len() > 4096 || turn_id.len() > 4096 || message.len() > 4096 {
            return None;
        }
        Some(EventKind::TurnError {
            thread_id: ThreadId(thread_id.into()),
            turn_id: TurnId(turn_id.into()),
            message: message.into(),
            will_retry,
        })
    })();
    parsed.unwrap_or_else(|| EventKind::Inconsistency {
        message: "Turn error notification was malformed or exceeded the local detail budget".into(),
    })
}

fn parse_warning(method: &str, params: &Value) -> EventKind {
    let message = if method == "configWarning" {
        params.get("summary")
    } else {
        params.get("message")
    }
    .and_then(Value::as_str);
    let thread_id = params.get("threadId").and_then(Value::as_str);
    match (message, thread_id) {
        (Some(message), thread_id)
            if message.len() <= 4096
                && thread_id.is_none_or(|id| id.len() <= 4096)
                && (method != "guardianWarning" || thread_id.is_some()) =>
        {
            EventKind::Warning {
                thread_id: thread_id.map(|id| ThreadId(id.into())),
                message: message.into(),
                guardian: method == "guardianWarning",
            }
        }
        _ => EventKind::Inconsistency {
            message: "Warning notification was malformed or exceeded the local detail budget"
                .into(),
        },
    }
}

fn initial_item_text(item: &Value) -> String {
    let Some(item_type) = item.get("type").and_then(Value::as_str) else {
        return String::new();
    };
    let text = match item_type {
        "commandExecution" => {
            let Some(command) = item.get("command").and_then(Value::as_str) else {
                return String::new();
            };
            if command.len() > 4096 {
                return String::new();
            }
            if item.get("source").and_then(Value::as_str) == Some("userShell") {
                format!("!{command}\n")
            } else {
                format!("$ {command}\n")
            }
        }
        // These public item families supply useful identity before terminal detail arrives.
        // Final item snapshots replace this provisional text rather than appending to it.
        "mcpToolCall"
        | "dynamicToolCall"
        | "webSearch"
        | "imageView"
        | "imageGeneration"
        | "collabAgentToolCall"
        | "subAgentActivity"
        | "enteredReviewMode"
        | "exitedReviewMode"
        | "contextCompaction" => history_item_text(item_type, item),
        _ => String::new(),
    };
    if text.len() <= 4096 {
        text
    } else {
        String::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Starting,
    Ready,
    Stopping,
    Stopped,
    Failed,
}

struct Inner {
    child: Mutex<Option<Child>>,
    writer: RpcWriter,
    next_id: Mutex<u64>,
    pending: Mutex<HashMap<String, mpsc::Sender<Result<Value, CodexError>>>>,
    subscribers: Mutex<Vec<crate::delivery::DeliverySender<CodexEvent>>>,
    outstanding: Mutex<HashMap<String, PendingInteraction>>,
    projection: Mutex<Projection>,
    sequence: Mutex<u64>,
    state: Mutex<ConnectionState>,
    request_timeout: Duration,
    dropped_events: AtomicU64,
    stderr: Mutex<Vec<u8>>,
}

enum RpcWriter {
    Stdio(Mutex<ChildStdin>),
    WebSocket(RemoteWriter),
}

enum RemoteWrite {
    Text(String),
    Close,
}

struct RemoteWriter {
    sender: mpsc::SyncSender<QueuedRemoteWrite>,
    retained: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
}
struct QueuedRemoteWrite {
    message: RemoteWrite,
    retained: Arc<AtomicUsize>,
    bytes: usize,
}
impl Drop for QueuedRemoteWrite {
    fn drop(&mut self) {
        self.retained.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
impl RemoteWriter {
    fn channel() -> (Self, mpsc::Receiver<QueuedRemoteWrite>) {
        let (sender, receiver) = mpsc::sync_channel(OUTBOUND_BACKLOG);
        (
            Self {
                sender,
                retained: Arc::new(AtomicUsize::new(0)),
                closed: Arc::new(AtomicBool::new(false)),
            },
            receiver,
        )
    }
    fn try_send(&self, message: RemoteWrite) -> Result<(), mpsc::TrySendError<RemoteWrite>> {
        if matches!(message, RemoteWrite::Close) {
            self.closed.store(true, Ordering::Release);
            return Ok(());
        }
        if self.closed.load(Ordering::Acquire) {
            return Err(mpsc::TrySendError::Disconnected(message));
        }
        let bytes = match &message {
            RemoteWrite::Text(text) => text.capacity(),
            RemoteWrite::Close => 0,
        };
        if self
            .retained
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |retained| {
                retained
                    .checked_add(bytes)
                    .filter(|total| *total <= MAX_OUTBOUND_BYTES)
            })
            .is_err()
        {
            return Err(mpsc::TrySendError::Full(message));
        }
        let queued = QueuedRemoteWrite {
            message,
            retained: self.retained.clone(),
            bytes,
        };
        self.sender.try_send(queued).map_err(|error| match error {
            mpsc::TrySendError::Full(mut queued) => {
                mpsc::TrySendError::Full(std::mem::replace(&mut queued.message, RemoteWrite::Close))
            }
            mpsc::TrySendError::Disconnected(mut queued) => mpsc::TrySendError::Disconnected(
                std::mem::replace(&mut queued.message, RemoteWrite::Close),
            ),
        })
    }
}

struct PendingInteraction {
    method: String,
    raw_id: Value,
    question_ids: Vec<String>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let RpcWriter::WebSocket(writer) = &self.writer {
            let _ = writer.try_send(RemoteWrite::Close);
        }
        if let Ok(Some(child)) = self.child.get_mut().map(Option::as_mut) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Clone)]
pub struct CodexClient {
    inner: Arc<Inner>,
}

impl CodexClient {
    pub fn spawn(executable: &Path, cwd: &Path) -> Result<Self, CodexError> {
        Self::spawn_with_timeout(executable, cwd, Duration::from_secs(15))
    }

    pub fn spawn_with_home(
        executable: &Path,
        cwd: &Path,
        codex_home: &Path,
    ) -> Result<Self, CodexError> {
        Self::spawn_with_timeout_and_home(
            executable,
            cwd,
            Duration::from_secs(15),
            Some(codex_home),
        )
    }

    pub fn spawn_with_timeout(
        executable: &Path,
        cwd: &Path,
        request_timeout: Duration,
    ) -> Result<Self, CodexError> {
        Self::spawn_with_timeout_and_home(executable, cwd, request_timeout, None)
    }

    /// Starts Codex with an optional isolated profile. The override applies only to the
    /// app-server child; Nickel and compatibility probes retain their ordinary environment.
    pub fn spawn_with_timeout_and_home(
        executable: &Path,
        cwd: &Path,
        request_timeout: Duration,
        codex_home: Option<&Path>,
    ) -> Result<Self, CodexError> {
        Self::spawn_with_options(executable, cwd, request_timeout, codex_home, false)
    }

    /// Starts a secondary local client without competing for the profile's single
    /// Codex phone connection. Remote-control RPCs must use the primary client.
    pub fn spawn_without_remote_control(
        executable: &Path,
        cwd: &Path,
        codex_home: Option<&Path>,
    ) -> Result<Self, CodexError> {
        Self::spawn_with_options(executable, cwd, Duration::from_secs(15), codex_home, true)
    }

    fn spawn_with_options(
        executable: &Path,
        cwd: &Path,
        request_timeout: Duration,
        codex_home: Option<&Path>,
        remote_control_disabled: bool,
    ) -> Result<Self, CodexError> {
        let mut child = command(executable);
        if let Some(codex_home) = codex_home {
            if !codex_home.is_absolute() {
                return Err(CodexError::Unavailable(
                    "isolated CODEX_HOME must be an absolute path".into(),
                ));
            }
            child.env("CODEX_HOME", codex_home);
        }
        if remote_control_disabled {
            child.env("CODEX_INTERNAL_APP_SERVER_REMOTE_CONTROL_DISABLED", "1");
        }
        let mut child = child
            .args(["app-server", "--listen", "stdio://"])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| CodexError::Protocol("missing child stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CodexError::Protocol("missing child stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CodexError::Protocol("missing child stderr".into()))?;
        let inner = Arc::new(Inner {
            child: Mutex::new(Some(child)),
            writer: RpcWriter::Stdio(Mutex::new(stdin)),
            next_id: Mutex::new(1),
            pending: Mutex::new(HashMap::new()),
            subscribers: Mutex::new(Vec::new()),
            outstanding: Mutex::new(HashMap::new()),
            projection: Mutex::new(Projection::default()),
            sequence: Mutex::new(0),
            state: Mutex::new(ConnectionState::Starting),
            request_timeout,
            dropped_events: AtomicU64::new(0),
            stderr: Mutex::new(Vec::new()),
        });
        let client = Self { inner };
        client.start_reader(stdout);
        client.start_stderr_drain(stderr);
        client.initialize()?;
        Ok(client)
    }

    pub fn connect_remote(endpoint: &str, bearer_token: Option<&str>) -> Result<Self, CodexError> {
        Self::connect_remote_with_timeout(endpoint, bearer_token, Duration::from_secs(15))
    }

    pub fn connect_remote_with_timeout(
        endpoint: &str,
        bearer_token: Option<&str>,
        request_timeout: Duration,
    ) -> Result<Self, CodexError> {
        validate_remote_endpoint(endpoint)?;
        let mut request = endpoint.into_client_request().map_err(|error| {
            CodexError::Unavailable(format!("invalid remote endpoint: {error}"))
        })?;
        if let Some(token) = bearer_token {
            if token.is_empty() {
                return Err(CodexError::Unavailable(
                    "configured remote bearer token is empty".into(),
                ));
            }
            let value = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| CodexError::Unavailable("remote bearer token is invalid".into()))?;
            request.headers_mut().insert(AUTHORIZATION, value);
        }
        let (mut socket, _) = connect(request).map_err(|error| {
            CodexError::Unavailable(format!("remote app-server connection failed: {error}"))
        })?;
        socket.set_config(|config| {
            config.max_message_size = Some(MAX_FRAME_BYTES);
            config.max_frame_size = Some(MAX_FRAME_BYTES);
        });
        let (writer, outbound) = RemoteWriter::channel();
        let outbound_closed = writer.closed.clone();
        let inner = Arc::new(Inner {
            child: Mutex::new(None),
            writer: RpcWriter::WebSocket(writer),
            next_id: Mutex::new(1),
            pending: Mutex::new(HashMap::new()),
            subscribers: Mutex::new(Vec::new()),
            outstanding: Mutex::new(HashMap::new()),
            projection: Mutex::new(Projection::default()),
            sequence: Mutex::new(0),
            state: Mutex::new(ConnectionState::Starting),
            request_timeout,
            dropped_events: AtomicU64::new(0),
            stderr: Mutex::new(Vec::new()),
        });
        let client = Self { inner };
        client.start_websocket(socket, outbound, outbound_closed);
        client.initialize()?;
        Ok(client)
    }

    fn initialize(&self) -> Result<(), CodexError> {
        self.request("initialize", json!({
            "clientInfo": {"name": "nickel", "title": "Nickel", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": true}
        }))?;
        self.notify("initialized", json!({}))?;
        *self.inner.state.lock().unwrap() = ConnectionState::Ready;
        self.publish(EventKind::Connection {
            state: "ready".into(),
        });
        Ok(())
    }

    pub fn state(&self) -> ConnectionState {
        *self.inner.state.lock().unwrap()
    }

    pub fn projection(&self) -> Projection {
        self.inner.projection.lock().unwrap().clone()
    }

    pub fn dropped_event_count(&self) -> u64 {
        self.inner.dropped_events.load(Ordering::Relaxed)
    }

    pub fn stderr_snapshot(&self) -> String {
        String::from_utf8_lossy(&self.inner.stderr.lock().unwrap()).into_owned()
    }

    fn start_reader(&self, stdout: impl std::io::Read + Send + 'static) {
        let inner = Arc::downgrade(&self.inner);
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match std::io::Read::by_ref(&mut reader)
                    .take((MAX_FRAME_BYTES + 1) as u64)
                    .read_line(&mut line)
                {
                    Ok(0) => {
                        if let Some(inner) = inner.upgrade() {
                            Self { inner }.fail("app-server stdout closed");
                        }
                        break;
                    }
                    Ok(_) if line.len() > MAX_FRAME_BYTES => {
                        if let Some(inner) = inner.upgrade() {
                            Self { inner }.fail("app-server frame exceeded limit");
                        }
                        break;
                    }
                    Ok(_) => match serde_json::from_str::<Value>(line.trim_end()) {
                        Ok(value) => {
                            let Some(inner) = inner.upgrade() else { break };
                            Self { inner }.handle(value)
                        }
                        Err(error) => {
                            if let Some(inner) = inner.upgrade() {
                                Self { inner }.fail(&format!("malformed app-server JSON: {error}"));
                            }
                            break;
                        }
                    },
                    Err(error) => {
                        if let Some(inner) = inner.upgrade() {
                            Self { inner }.fail(&format!("app-server read failed: {error}"));
                        }
                        break;
                    }
                }
            }
        });
    }

    fn start_stderr_drain(&self, stderr: impl std::io::Read + Send + 'static) {
        let inner = Arc::downgrade(&self.inner);
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut chunk = [0u8; 4096];
            while let Ok(count) = reader.read(&mut chunk) {
                if count == 0 {
                    break;
                }
                let Some(inner) = inner.upgrade() else { break };
                let mut retained = inner.stderr.lock().unwrap();
                let available = (64usize * 1024).saturating_sub(retained.len());
                retained.extend_from_slice(&chunk[..count.min(available)]);
            }
        });
    }

    fn start_websocket(
        &self,
        mut socket: tungstenite::WebSocket<MaybeTlsStream<std::net::TcpStream>>,
        outbound: mpsc::Receiver<QueuedRemoteWrite>,
        closed: Arc<AtomicBool>,
    ) {
        set_socket_timeout(socket.get_mut(), Duration::from_millis(50));
        let inner = Arc::downgrade(&self.inner);
        thread::spawn(move || {
            loop {
                if closed.load(Ordering::Acquire) || inner.strong_count() == 0 {
                    return;
                }
                while let Ok(mut write) = outbound.try_recv() {
                    if closed.load(Ordering::Acquire) {
                        return;
                    }
                    let result = match std::mem::replace(&mut write.message, RemoteWrite::Close) {
                        RemoteWrite::Text(text) => socket.send(Message::Text(text.into())),
                        RemoteWrite::Close => {
                            let _ = socket.close(None);
                            return;
                        }
                    };
                    if let Err(error) = result {
                        if let Some(inner) = inner.upgrade() {
                            Self { inner }
                                .fail(&format!("remote app-server write failed: {error}"));
                        }
                        return;
                    }
                }
                match socket.read() {
                    Ok(Message::Text(text)) if text.len() > MAX_FRAME_BYTES => {
                        if let Some(inner) = inner.upgrade() {
                            Self { inner }.fail("remote app-server frame exceeded limit");
                        }
                        return;
                    }
                    Ok(Message::Text(text)) => match serde_json::from_str::<Value>(text.as_ref()) {
                        Ok(value) => {
                            let Some(inner) = inner.upgrade() else {
                                return;
                            };
                            Self { inner }.handle(value);
                        }
                        Err(error) => {
                            if let Some(inner) = inner.upgrade() {
                                Self { inner }
                                    .fail(&format!("malformed remote app-server JSON: {error}"));
                            }
                            return;
                        }
                    },
                    Ok(Message::Binary(_)) => {
                        if let Some(inner) = inner.upgrade() {
                            Self { inner }.fail("remote app-server sent a binary protocol message");
                        }
                        return;
                    }
                    Ok(Message::Close(_)) => {
                        if let Some(inner) = inner.upgrade() {
                            Self { inner }.fail("remote app-server closed the connection");
                        }
                        return;
                    }
                    Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
                    Err(tungstenite::Error::Capacity(_)) => {
                        if let Some(inner) = inner.upgrade() {
                            Self { inner }.fail("remote app-server frame exceeded limit");
                        }
                        return;
                    }
                    Err(tungstenite::Error::Io(error))
                        if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                    Err(error) => {
                        if let Some(inner) = inner.upgrade() {
                            Self { inner }.fail(&format!("remote app-server read failed: {error}"));
                        }
                        return;
                    }
                }
            }
        });
    }

    fn request(&self, method: &str, params: Value) -> Result<Value, CodexError> {
        let started = std::time::Instant::now();
        if matches!(
            self.state(),
            ConnectionState::Failed | ConnectionState::Stopped
        ) {
            return Err(CodexError::Stopped("connection is not active".into()));
        }
        let id = {
            let mut next = self.inner.next_id.lock().unwrap();
            let id = *next;
            *next += 1;
            id
        };
        let key = id.to_string();
        let (tx, rx) = mpsc::channel();
        {
            let mut pending = self.inner.pending.lock().unwrap();
            if pending.len() >= 128 {
                return Err(CodexError::Unavailable(
                    "request correlation limit reached".into(),
                ));
            }
            pending.insert(key.clone(), tx);
        }
        let message = if params.is_null() {
            json!({"id": id, "method": method})
        } else {
            json!({"id": id, "method": method, "params": params})
        };
        if let Err(error) = self.write(&message) {
            self.inner.pending.lock().unwrap().remove(&key);
            return Err(error);
        }
        let received = rx.recv_timeout(self.inner.request_timeout);
        self.inner.pending.lock().unwrap().remove(&key);
        let result = received.map_err(|_| CodexError::Timeout(format!("{method} timed out")))?;
        if std::env::var_os("NICKEL_CODEX_TIMING").is_some() {
            eprintln!(
                "nickel-codex timing: method={method} elapsed_ms={:.3} success={}",
                started.elapsed().as_secs_f64() * 1_000.0,
                result.is_ok()
            );
        }
        result
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), CodexError> {
        self.write(&json!({"method": method, "params": params}))
    }

    fn write(&self, value: &Value) -> Result<(), CodexError> {
        let text = String::from_utf8(crate::delivery::encode_json(value, MAX_OUTBOUND_BYTES)?)
            .expect("JSON encoder emits UTF-8");
        if text.len() > MAX_OUTBOUND_BYTES {
            return Err(CodexError::Protocol(
                "outbound app-server frame exceeded limit".into(),
            ));
        }
        match &self.inner.writer {
            RpcWriter::Stdio(stdin) => {
                let mut stdin = stdin.lock().unwrap();
                stdin.write_all(text.as_bytes())?;
                stdin.write_all(b"\n")?;
                stdin.flush()?;
                Ok(())
            }
            RpcWriter::WebSocket(writer) => {
                writer
                    .try_send(RemoteWrite::Text(text))
                    .map_err(|error| match error {
                        mpsc::TrySendError::Full(_) => {
                            CodexError::Unavailable("remote app-server write queue is full".into())
                        }
                        mpsc::TrySendError::Disconnected(_) => {
                            CodexError::Stopped("remote app-server connection is closed".into())
                        }
                    })
            }
        }
    }

    fn handle(&self, value: Value) {
        if matches!(
            self.state(),
            ConnectionState::Failed | ConnectionState::Stopped
        ) {
            return;
        }
        if let Some(id) = value.get("id").and_then(request_id) {
            if value.get("method").is_some() {
                self.handle_server_request(id, &value);
            } else if let Some(sender) = self.inner.pending.lock().unwrap().remove(&id) {
                let result = value.get("error").map_or_else(
                    || Ok(value.get("result").cloned().unwrap_or(Value::Null)),
                    |error| Err(CodexError::Protocol(error.to_string())),
                );
                let _ = sender.send(result);
            } else {
                self.publish(EventKind::Inconsistency {
                    message: format!("response for unknown id {id}"),
                });
            }
        } else if let Some(method) = value.get("method").and_then(Value::as_str) {
            self.handle_notification(method, value.get("params").unwrap_or(&Value::Null));
        } else {
            self.publish(EventKind::Inconsistency {
                message: "message has neither id nor method".into(),
            });
        }
    }

    fn handle_server_request(&self, id: String, value: &Value) {
        let method = value["method"].as_str().unwrap_or_default();
        // A server request is a callback, not a notification. Unknown methods must receive a
        // terminal protocol error immediately so Codex cannot wait forever on a UI card.
        if !matches!(
            method,
            "item/commandExecution/requestApproval"
                | "item/fileChange/requestApproval"
                | "item/tool/requestUserInput"
        ) {
            let _ = self.write(&json!({"id": value["id"], "error": {
                "code": -32601, "message": "Unsupported client request"
            }}));
            self.publish(EventKind::UnsupportedEvent {
                method: method.into(),
            });
            return;
        }
        // Reject malformed questions before recording a pending callback; IDs alone are not
        // enough to let a person answer safely.
        let questions = if method == "item/tool/requestUserInput" {
            match parse_user_input_questions(&value["params"]) {
                Some(questions) => questions,
                None => {
                    let _ = self.write(&json!({"id": value["id"], "error": {
                        "code": -32602, "message": "Unsupported or malformed user-input questions"
                    }}));
                    return;
                }
            }
        } else {
            Vec::new()
        };
        let question_ids: Vec<String> = questions
            .iter()
            .map(|question| question.id.clone())
            .collect();
        // The ordered decision set is consent authority, not presentation copy.
        // Reject malformed or oversized sets before registering a pending callback.
        let available_decisions = if method == "item/commandExecution/requestApproval" {
            match parse_available_decisions(&value["params"]) {
                Ok(decisions) => decisions,
                Err(message) => {
                    let _ = self.write(&json!({"id": value["id"], "error": {
                        "code": -32602, "message": message
                    }}));
                    return;
                }
            }
        } else {
            None
        };
        let mut outstanding = self.inner.outstanding.lock().unwrap();
        if outstanding.len() >= 32
            || id.len() > 4096
            || method.len() > 4096
            || question_ids.len() > 32
            || question_ids.iter().map(String::capacity).sum::<usize>() > 32 * 1024
        {
            drop(outstanding);
            let _ = self.write(&json!({"id": value["id"], "error": {"code": -32000, "message": "Local interaction capacity reached; retry after resolving pending requests"}}));
            self.publish(EventKind::Error { message: "An additional interaction was rejected because local interaction capacity is full".into() });
            return;
        }
        outstanding.insert(
            id.clone(),
            PendingInteraction {
                method: method.into(),
                raw_id: value["id"].clone(),
                question_ids: question_ids.clone(),
            },
        );
        drop(outstanding);
        let request_id = ServerRequestId(id);
        let params = &value["params"];
        match method {
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                self.publish(EventKind::ApprovalRequested {
                    request_id,
                    thread_id: params
                        .get("threadId")
                        .and_then(Value::as_str)
                        .map(|id| ThreadId(id.to_owned())),
                    approval_type: method.into(),
                    summary: params
                        .get("reason")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    context: crate::ApprovalContext {
                        item_id: params
                            .get("itemId")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        approval_id: params
                            .get("approvalId")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        command: params
                            .get("command")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        cwd: params.get("cwd").and_then(Value::as_str).map(str::to_owned),
                        grant_root: params
                            .get("grantRoot")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        kind: params
                            .get("kind")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        asks_network_access: params
                            .get("networkApprovalContext")
                            .is_some_and(|value| !value.is_null()),
                        proposes_session_rule: [
                            "proposedExecpolicyAmendment",
                            "proposedNetworkPolicyAmendments",
                        ]
                        .iter()
                        .any(|key| params.get(*key).is_some_and(|value| !value.is_null())),
                        available_decisions,
                    },
                });
            }
            "item/tool/requestUserInput" => {
                self.publish(EventKind::UserInputRequested {
                    request_id,
                    question_ids,
                    questions,
                });
            }
            _ => unreachable!("request method validated above"),
        }
    }

    fn handle_notification(&self, method: &str, params: &Value) {
        let string = |name: &str| {
            params
                .get(name)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let nested = |container: &str, name: &str| {
            params
                .get(container)
                .and_then(|v| v.get(name))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let event = match method {
            "thread/started" => EventKind::ThreadStarted {
                thread_id: ThreadId(nested("thread", "id")),
            },
            "turn/started" => EventKind::TurnStarted {
                thread_id: ThreadId(string("threadId")),
                turn_id: TurnId(nested("turn", "id")),
            },
            "turn/completed" => EventKind::TurnCompleted {
                thread_id: ThreadId(string("threadId")),
                turn_id: TurnId(nested("turn", "id")),
                status: nested("turn", "status"),
            },
            "item/started" => EventKind::ItemStarted {
                thread_id: params
                    .get("threadId")
                    .and_then(Value::as_str)
                    .map(|v| ThreadId(v.into())),
                turn_id: params
                    .get("turnId")
                    .and_then(Value::as_str)
                    .map(|v| TurnId(v.into())),
                item_id: nested("item", "id"),
                item_type: nested("item", "type"),
                command_actions: params
                    .get("item")
                    .filter(|item| item.get("source").and_then(Value::as_str) != Some("userShell"))
                    .and_then(|item| item.get("commandActions"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(parse_command_action)
                    .collect(),
                initial_text: params
                    .get("item")
                    .map(initial_item_text)
                    .unwrap_or_default(),
            },
            "item/completed" => parse_item_completed(params),
            "item/agentMessage/delta" => EventKind::AgentMessageDelta {
                item_id: string("itemId"),
                delta: string("delta"),
            },
            "item/commandExecution/outputDelta" => EventKind::CommandOutputDelta {
                item_id: string("itemId"),
                delta: string("delta"),
            },
            "item/fileChange/outputDelta" => EventKind::FileChangeDelta {
                item_id: string("itemId"),
                delta: string("delta"),
            },
            "item/fileChange/patchUpdated" => parse_file_patch_updated(params),
            "item/plan/delta" => EventKind::PlanDelta {
                item_id: string("itemId"),
                delta: string("delta"),
            },
            "turn/plan/updated" => parse_turn_plan_updated(params),
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                EventKind::ReasoningDelta {
                    item_id: string("itemId"),
                    delta: string("delta"),
                }
            }
            "account/updated" | "account/rateLimits/updated" => EventKind::AccountUpdated,
            "serverRequest/resolved" => match (
                params.get("threadId").and_then(Value::as_str),
                params.get("requestId").and_then(request_id),
            ) {
                (Some(thread_id), Some(request_id)) => EventKind::ServerRequestResolved {
                    thread_id: ThreadId(thread_id.to_owned()),
                    request_id: ServerRequestId(request_id),
                },
                _ => EventKind::Inconsistency {
                    message: "serverRequest/resolved lacked thread or request identity".into(),
                },
            },
            "account/login/completed" => EventKind::AccountLoginCompleted {
                completion: crate::LoginCompletion {
                    login_id: params
                        .get("loginId")
                        .and_then(Value::as_str)
                        .map(Into::into),
                    success: params
                        .get("success")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    error: params.get("error").and_then(Value::as_str).map(Into::into),
                },
            },
            "remoteControl/status/changed" => match parse_remote_control_status(params.clone()) {
                Ok(status) => EventKind::RemoteControlStatusChanged { status },
                Err(error) => EventKind::Inconsistency {
                    message: error.to_string(),
                },
            },
            "error" => parse_turn_error(params),
            "warning" | "guardianWarning" | "configWarning" => parse_warning(method, params),
            _ => EventKind::UnsupportedEvent {
                method: method.into(),
            },
        };
        self.project(&event);
        if self.state() != ConnectionState::Failed {
            self.publish(event);
        }
    }

    fn project(&self, event: &EventKind) {
        let metadata_valid = match event {
            EventKind::ThreadStarted { thread_id } => thread_id.0.len() <= 4096,
            EventKind::TurnStarted { thread_id, turn_id }
            | EventKind::TurnCompleted {
                thread_id, turn_id, ..
            } => thread_id.0.len() <= 4096 && turn_id.0.len() <= 4096,
            EventKind::ItemStarted {
                item_id, item_type, ..
            } => item_id.len() <= 4096 && item_type.len() <= 4096,
            EventKind::ServerRequestResolved {
                thread_id,
                request_id,
            } => thread_id.0.len() <= 4096 && request_id.0.len() <= 4096,
            EventKind::AgentMessageDelta { item_id, .. }
            | EventKind::CommandOutputDelta { item_id, .. }
            | EventKind::FileChangeDelta { item_id, .. }
            | EventKind::PlanDelta { item_id, .. }
            | EventKind::ReasoningDelta { item_id, .. } => item_id.len() <= 4096,
            _ => true,
        };
        if !metadata_valid {
            self.publish(EventKind::Error {
                message: "Lifecycle metadata exceeds projection limit; reload server history"
                    .into(),
            });
            self.fail("Lifecycle metadata exceeds projection limit");
            return;
        }
        let mut projection = self.inner.projection.lock().unwrap();
        let mut inconsistency = None;
        let mut metadata_changed = true;
        match event {
            EventKind::ThreadStarted { thread_id } => {
                projection.threads.entry(thread_id.clone()).or_default();
            }
            EventKind::TurnStarted { thread_id, turn_id } => {
                if projection.active_turn.is_some()
                    && projection.active_turn.as_ref() != Some(turn_id)
                {
                    drop(projection);
                    self.publish(EventKind::Inconsistency {
                        message: "second active turn observed".into(),
                    });
                    return;
                }
                projection.active_turn = Some(turn_id.clone());
                projection
                    .threads
                    .entry(thread_id.clone())
                    .or_default()
                    .active_turn = Some(turn_id.clone());
            }
            EventKind::TurnCompleted {
                thread_id, turn_id, ..
            } => {
                let duplicate = projection
                    .threads
                    .get(thread_id)
                    .is_some_and(|thread| thread.terminal_turns.contains(turn_id));
                if !duplicate
                    && projection
                        .threads
                        .get(thread_id)
                        .and_then(|thread| thread.active_turn.as_ref())
                        != Some(turn_id)
                {
                    inconsistency = Some("terminal turn observed before matching start".into());
                }
                if projection.active_turn.as_ref() == Some(turn_id) {
                    projection.active_turn = None;
                }
                let thread = projection.threads.entry(thread_id.clone()).or_default();
                if thread.active_turn.as_ref() == Some(turn_id) {
                    thread.active_turn = None;
                }
                if !thread.terminal_turns.contains(turn_id) {
                    thread.terminal_turns.push(turn_id.clone());
                }
            }
            EventKind::ItemStarted {
                item_id, item_type, ..
            } => {
                projection
                    .items
                    .entry(item_id.clone())
                    .or_insert_with(|| ProjectedItem {
                        item_type: item_type.clone(),
                        ..ProjectedItem::default()
                    });
            }
            EventKind::ItemCompleted {
                item_id,
                completion,
            } => {
                if let Some(item) = projection.items.get_mut(item_id) {
                    item.completed = true;
                } else if let Some(completion) = completion {
                    projection.items.insert(
                        item_id.clone(),
                        ProjectedItem {
                            item_type: completion.item_type.clone(),
                            // Delivery owns the bounded payload; projection retains metadata.
                            text: String::new(),
                            completed: true,
                        },
                    );
                } else {
                    inconsistency = Some(format!("completion for unknown item {item_id}"));
                }
            }
            EventKind::AgentMessageDelta { item_id, .. }
            | EventKind::CommandOutputDelta { item_id, .. }
            | EventKind::FileChangeDelta { item_id, .. }
            | EventKind::PlanDelta { item_id, .. }
            | EventKind::ReasoningDelta { item_id, .. } => {
                match projection.items.get_mut(item_id) {
                    Some(item) if !item.completed => {
                        metadata_changed = false;
                    }
                    Some(_) => {
                        inconsistency = Some(format!("delta for terminal item {item_id}"));
                    }
                    None => {
                        inconsistency = Some(format!("delta for unknown item {item_id}"));
                        projection.items.entry(item_id.clone()).or_default();
                    }
                }
            }
            EventKind::Error { message } => {
                let mut end = message.len().min(4096);
                while !message.is_char_boundary(end) {
                    end -= 1;
                }
                projection.terminal_error = Some(message[..end].to_owned());
            }
            EventKind::TurnError {
                message,
                will_retry,
                ..
            } if !will_retry => {
                projection.terminal_error = Some(message.clone());
            }
            _ => {}
        }
        let within_budget = !metadata_changed || projection.enforce_limits();
        if !within_budget {
            // A capacity failure is terminal, rather than silently forgetting live items.
            // Keep the independently owned active turn available for cancellation diagnostics.
            projection.items.clear();
            projection.items.shrink_to_fit();
            projection.threads.clear();
            projection.threads.shrink_to_fit();
            projection.terminal_error =
                Some("Lifecycle capacity exceeded; reconnect and reload server history".into());
        }
        drop(projection);
        if !within_budget {
            self.publish(EventKind::Error {
                message: "Lifecycle capacity exceeded; reconnect and reload server history".into(),
            });
            self.fail("Lifecycle capacity exceeded; reconnect and reload server history");
            return;
        }
        if let Some(message) = inconsistency {
            self.publish(EventKind::Inconsistency { message });
        }
    }

    fn publish(&self, kind: EventKind) {
        let sequence = {
            let mut sequence = self.inner.sequence.lock().unwrap();
            *sequence += 1;
            *sequence
        };
        let event = CodexEvent { sequence, kind };
        self.inner.subscribers.lock().unwrap().retain(|subscriber| {
            match subscriber.send(event.clone()) {
                Ok(()) => true,
                Err(_) => {
                    self.inner.dropped_events.fetch_add(1, Ordering::Relaxed);
                    false
                }
            }
        });
    }

    fn fail(&self, message: &str) {
        let mut state = self.inner.state.lock().unwrap();
        if matches!(*state, ConnectionState::Failed | ConnectionState::Stopped) {
            return;
        }
        *state = ConnectionState::Failed;
        drop(state);
        for (_, sender) in self.inner.pending.lock().unwrap().drain() {
            let _ = sender.send(Err(CodexError::Stopped(message.into())));
        }
        self.inner.outstanding.lock().unwrap().clear();
        self.publish(EventKind::Connection {
            state: "failed".into(),
        });
    }

    pub fn shutdown(&self) {
        let mut state = self.inner.state.lock().unwrap();
        if matches!(*state, ConnectionState::Stopped | ConnectionState::Stopping) {
            return;
        }
        *state = ConnectionState::Stopping;
        drop(state);
        if let RpcWriter::WebSocket(writer) = &self.inner.writer {
            let _ = writer.try_send(RemoteWrite::Close);
        }
        if let Some(child) = self.inner.child.lock().unwrap().as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        *self.inner.state.lock().unwrap() = ConnectionState::Stopped;
        self.publish(EventKind::Connection {
            state: "stopped".into(),
        });
    }
}

impl Drop for CodexClient {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1 {
            self.shutdown();
        }
    }
}

fn validate_remote_endpoint(endpoint: &str) -> Result<(), CodexError> {
    let endpoint = Url::parse(endpoint)
        .map_err(|error| CodexError::Unavailable(format!("invalid remote endpoint: {error}")))?;
    if !matches!(endpoint.scheme(), "ws" | "wss") {
        return Err(CodexError::Unavailable(
            "remote endpoint must use ws or wss".into(),
        ));
    }
    if endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(CodexError::Unavailable(
            "remote endpoint must include a host and must not contain credentials or a fragment"
                .into(),
        ));
    }
    Ok(())
}

fn set_socket_timeout(stream: &mut MaybeTlsStream<std::net::TcpStream>, timeout: Duration) {
    let result = match stream {
        MaybeTlsStream::Plain(stream) => stream.set_read_timeout(Some(timeout)),
        MaybeTlsStream::Rustls(stream) => stream.sock.set_read_timeout(Some(timeout)),
        _ => return,
    };
    let _ = result;
}

fn turn_input(text: String, images: Vec<crate::TurnImage>) -> Vec<Value> {
    let mut input = vec![json!({"type": "text", "text": text})];
    input.extend(
        images
            .into_iter()
            .map(|image| json!({"type": "image", "url": image.data_url})),
    );
    input
}

fn parse_login_challenge(
    method: crate::LoginMethod,
    value: Value,
) -> Result<crate::LoginChallenge, CodexError> {
    fn field(value: &Value, name: &str, limit: usize) -> Result<String, CodexError> {
        let text = value
            .get(name)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty() && text.len() <= limit)
            .ok_or_else(|| CodexError::Protocol(format!("invalid login response field {name}")))?;
        Ok(text.into())
    }

    let response_type = field(&value, "type", 64)?;
    let login_id = field(&value, "loginId", 1024)?;
    match (method, response_type.as_str()) {
        (crate::LoginMethod::Browser, "chatgpt") => Ok(crate::LoginChallenge::Browser {
            login_id,
            auth_url: field(&value, "authUrl", 8192)?,
        }),
        (crate::LoginMethod::DeviceCode, "chatgptDeviceCode") => {
            Ok(crate::LoginChallenge::DeviceCode {
                login_id,
                user_code: field(&value, "userCode", 128)?,
                verification_url: field(&value, "verificationUrl", 8192)?,
            })
        }
        _ => Err(CodexError::Protocol(format!(
            "login response type {response_type:?} did not match request"
        ))),
    }
}

fn bounded_remote_text(value: &Value, name: &str, limit: usize) -> Result<String, CodexError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && text.len() <= limit)
        .map(Into::into)
        .ok_or_else(|| CodexError::Protocol(format!("invalid remote-control field {name}")))
}

fn optional_bounded_remote_text(
    value: &Value,
    name: &str,
    limit: usize,
) -> Result<Option<String>, CodexError> {
    match value.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if !text.is_empty() && text.len() <= limit => {
            Ok(Some(text.clone()))
        }
        _ => Err(CodexError::Protocol(format!(
            "invalid remote-control field {name}"
        ))),
    }
}

fn parse_remote_control_status(value: Value) -> Result<crate::RemoteControlStatus, CodexError> {
    let status = match bounded_remote_text(&value, "status", 32)?.as_str() {
        "disabled" => crate::RemoteControlConnectionStatus::Disabled,
        "connecting" => crate::RemoteControlConnectionStatus::Connecting,
        "connected" => crate::RemoteControlConnectionStatus::Connected,
        "errored" => crate::RemoteControlConnectionStatus::Errored,
        other => {
            return Err(CodexError::Protocol(format!(
                "unknown remote-control status {other:?}"
            )));
        }
    };
    Ok(crate::RemoteControlStatus {
        status,
        server_name: bounded_remote_text(&value, "serverName", 512)?,
        installation_id: bounded_remote_text(&value, "installationId", 1024)?,
        environment_id: optional_bounded_remote_text(&value, "environmentId", 1024)?,
    })
}

fn parse_remote_pairing(value: Value) -> Result<crate::RemotePairingChallenge, CodexError> {
    Ok(crate::RemotePairingChallenge {
        environment_id: bounded_remote_text(&value, "environmentId", 1024)?,
        expires_at: value
            .get("expiresAt")
            .and_then(Value::as_i64)
            .ok_or_else(|| CodexError::Protocol("invalid remote-control field expiresAt".into()))?,
        pairing_code: bounded_remote_text(&value, "pairingCode", 8192)?,
        manual_pairing_code: optional_bounded_remote_text(&value, "manualPairingCode", 128)?,
    })
}

fn parse_remote_client(value: &Value) -> Result<crate::RemoteControlClient, CodexError> {
    Ok(crate::RemoteControlClient {
        client_id: bounded_remote_text(value, "clientId", 1024)?,
        display_name: optional_bounded_remote_text(value, "displayName", 512)?,
        device_model: optional_bounded_remote_text(value, "deviceModel", 512)?,
        device_type: optional_bounded_remote_text(value, "deviceType", 128)?,
        platform: optional_bounded_remote_text(value, "platform", 128)?,
        os_version: optional_bounded_remote_text(value, "osVersion", 128)?,
        app_version: optional_bounded_remote_text(value, "appVersion", 128)?,
        last_seen_at: match value.get("lastSeenAt") {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_i64().ok_or_else(|| {
                CodexError::Protocol("invalid remote-control field lastSeenAt".into())
            })?),
        },
    })
}

fn parse_rate_limits_status(value: &Value) -> Result<crate::RateLimitsStatus, CodexError> {
    let mut entries = value
        .get("rateLimitsByLimitId")
        .and_then(Value::as_object)
        .map(|buckets| {
            let mut entries = buckets
                .iter()
                .map(|(id, bucket)| (id.clone(), bucket))
                .collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            entries
        })
        .unwrap_or_default();
    if entries.is_empty() {
        let bucket = value
            .get("rateLimits")
            .ok_or_else(|| CodexError::Protocol("rate-limit response omitted rateLimits".into()))?;
        entries.push(("codex".to_owned(), bucket));
    }
    let mut buckets = Vec::new();
    for (id, bucket) in entries.into_iter().take(16) {
        let name = bucket
            .get("limitName")
            .and_then(Value::as_str)
            .filter(|name| name.len() <= 80 && !name.chars().any(char::is_control))
            .unwrap_or(&id)
            .to_owned();
        let used = |key: &str| -> Result<Option<i32>, CodexError> {
            let Some(window) = bucket.get(key).filter(|window| !window.is_null()) else {
                return Ok(None);
            };
            let percent = window
                .get("usedPercent")
                .and_then(Value::as_i64)
                .ok_or_else(|| CodexError::Protocol("invalid rate-limit percentage".into()))?;
            i32::try_from(percent)
                .map(Some)
                .map_err(|_| CodexError::Protocol("rate-limit percentage is out of range".into()))
        };
        buckets.push(crate::RateLimitBucket {
            name,
            primary_used_percent: used("primary")?,
            secondary_used_percent: used("secondary")?,
        });
    }
    Ok(crate::RateLimitsStatus {
        ordinary_usage_allowed: value.get("ordinaryUsageAllowed").and_then(Value::as_bool),
        buckets,
    })
}

fn parse_file_search_matches(value: &Value) -> Result<Vec<crate::FileSearchMatch>, CodexError> {
    const MAX_MATCHES: usize = 200;
    const MAX_FIELD_BYTES: usize = 4096;
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| CodexError::Protocol("invalid fuzzy-file-search response".into()))?;
    if files.len() > MAX_MATCHES {
        return Err(CodexError::Protocol(
            "fuzzy-file-search response exceeds local capacity".into(),
        ));
    }
    files
        .iter()
        .map(|file| {
            let field = |name: &str, legacy_name: &str| -> Result<String, CodexError> {
                file.get(name)
                    .or_else(|| file.get(legacy_name))
                    .and_then(Value::as_str)
                    .filter(|value| value.len() <= MAX_FIELD_BYTES)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        CodexError::Protocol(format!(
                            "invalid fuzzy-file-search response field {name}"
                        ))
                    })
            };
            let match_type = field("matchType", "match_type")?;
            let is_directory = match match_type.as_str() {
                "file" => false,
                "directory" => true,
                _ => {
                    return Err(CodexError::Protocol(
                        "invalid fuzzy-file-search match type".into(),
                    ));
                }
            };
            let score = file
                .get("score")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| CodexError::Protocol("invalid fuzzy-file-search score".into()))?;
            Ok(crate::FileSearchMatch {
                root: field("root", "root")?,
                path: field("path", "path")?,
                file_name: field("fileName", "file_name")?,
                is_directory,
                score,
            })
        })
        .collect()
}

const WORKSPACE_DIFF_BYTES: usize = 256 * 1024;
const WORKSPACE_DIFF_FILE_LIMIT: usize = 128;
const WORKSPACE_DIFF_OMISSION: &str = "\n[Further diff output omitted by Nickel.]\n";

struct CommandExecOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

impl CodexClient {
    fn bounded_command_exec(
        &self,
        cwd: &std::path::Path,
        command: Vec<String>,
    ) -> Result<CommandExecOutput, CodexError> {
        let value = self.request(
            "command/exec",
            json!({
                "command": command,
                "processId": null,
                "tty": false,
                "streamStdin": false,
                "streamStdoutStderr": false,
                "outputBytesCap": WORKSPACE_DIFF_BYTES,
                "disableOutputCap": false,
                "disableTimeout": false,
                "timeoutMs": 30_000,
                "cwd": cwd,
                "env": null,
                "size": null,
                "sandboxPolicy": {"type":"readOnly"},
                "permissionProfile": null
            }),
        )?;
        let exit_code = value
            .get("exitCode")
            .and_then(Value::as_i64)
            .and_then(|code| i32::try_from(code).ok())
            .ok_or_else(|| CodexError::Protocol("invalid command/exec exit code".into()))?;
        let output = |field: &str| -> Result<String, CodexError> {
            value
                .get(field)
                .and_then(Value::as_str)
                .filter(|text| text.len() <= WORKSPACE_DIFF_BYTES)
                .map(str::to_owned)
                .ok_or_else(|| CodexError::Protocol(format!("invalid command/exec {field} output")))
        };
        Ok(CommandExecOutput {
            exit_code,
            stdout: output("stdout")?,
            stderr: output("stderr")?,
        })
    }
}

fn append_workspace_diff(destination: &mut String, source: &str) -> bool {
    let remaining = WORKSPACE_DIFF_BYTES.saturating_sub(destination.len());
    if source.len() <= remaining {
        destination.push_str(source);
        return true;
    }
    let content_limit = WORKSPACE_DIFF_BYTES.saturating_sub(WORKSPACE_DIFF_OMISSION.len());
    if destination.len() > content_limit {
        let mut end = content_limit;
        while !destination.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        destination.truncate(end);
    }
    let marker_space = content_limit.saturating_sub(destination.len());
    let mut end = marker_space.min(source.len());
    while !source.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    destination.push_str(&source[..end]);
    destination.push_str(WORKSPACE_DIFF_OMISSION);
    false
}

fn safe_git_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && path.as_bytes().get(1) != Some(&b':')
        && !path
            .split(['/', '\\'])
            .any(|component| component.is_empty() || component == "..")
        && !path.as_bytes().contains(&0)
}

impl CodexBackend for CodexClient {
    fn account(&self) -> Result<AccountState, CodexError> {
        let value = self.request("account/read", json!({"refreshToken": false}))?;
        let account = value.get("account").unwrap_or(&value);
        Ok(AccountState {
            authenticated: !account.is_null(),
            account_type: account.get("type").and_then(Value::as_str).map(Into::into),
            email: account.get("email").and_then(Value::as_str).map(Into::into),
        })
    }
    fn rate_limits(&self) -> Result<crate::RateLimitsStatus, CodexError> {
        let value = self.request("account/rateLimits/read", Value::Null)?;
        parse_rate_limits_status(&value)
    }
    fn search_files(
        &self,
        query: String,
        roots: Vec<String>,
    ) -> Result<Vec<crate::FileSearchMatch>, CodexError> {
        if query.len() > 4096
            || roots.is_empty()
            || roots.len() > 16
            || roots
                .iter()
                .any(|root| root.is_empty() || root.len() > 4096)
        {
            return Err(CodexError::Protocol(
                "invalid fuzzy-file-search request".into(),
            ));
        }
        parse_file_search_matches(&self.request(
            "fuzzyFileSearch",
            json!({
                "query": query,
                "roots": roots,
                "cancellationToken": "nickel-file-mention"
            }),
        )?)
    }
    fn upload_feedback(
        &self,
        classification: String,
        reason: Option<String>,
        thread_id: Option<ThreadId>,
        include_logs: bool,
    ) -> Result<String, CodexError> {
        if !matches!(
            classification.as_str(),
            "bug" | "bad_result" | "good_result" | "safety_check" | "other"
        ) || reason
            .as_ref()
            .is_some_and(|reason| reason.len() > 16 * 1024)
            || thread_id
                .as_ref()
                .is_some_and(|thread| thread.0.len() > 4096)
        {
            return Err(CodexError::Protocol("invalid feedback request".into()));
        }
        let value = self.request(
            "feedback/upload",
            json!({
                "classification": classification,
                "reason": reason,
                "threadId": thread_id.map(|thread| thread.0),
                "includeLogs": include_logs,
                "extraLogFiles": null,
                "tags": null
            }),
        )?;
        value
            .get("threadId")
            .and_then(Value::as_str)
            .filter(|thread_id| !thread_id.is_empty() && thread_id.len() <= 4096)
            .map(str::to_owned)
            .ok_or_else(|| CodexError::Protocol("invalid feedback response".into()))
    }
    fn workspace_diff(&self, cwd: std::path::PathBuf) -> Result<String, CodexError> {
        let git = |args: &[&str]| {
            self.bounded_command_exec(
                &cwd,
                std::iter::once("git".to_owned())
                    // These read-only commands never invoke hooks. Disable the
                    // filesystem monitor without injecting a client-OS-specific
                    // null path into an app-server that may run another OS.
                    .chain(
                        ["-c", "core.fsmonitor=false"]
                            .into_iter()
                            .map(str::to_owned),
                    )
                    .chain(args.iter().map(|argument| (*argument).to_owned()))
                    .collect(),
            )
        };
        let inside = git(&["rev-parse", "--is-inside-work-tree"])?;
        if inside.exit_code != 0 || inside.stdout.trim() != "true" {
            return Ok("`/diff` — _not inside a Git repository_".into());
        }

        let tracked = git(&[
            "diff",
            "--no-textconv",
            "--no-ext-diff",
            "--submodule=short",
            "--ignore-submodules=dirty",
            "--no-color",
        ])?;
        if !matches!(tracked.exit_code, 0 | 1) {
            return Err(CodexError::Protocol(format!(
                "git diff failed: {}",
                tracked.stderr.lines().next().unwrap_or("unknown error")
            )));
        }
        let mut diff = String::new();
        if !append_workspace_diff(&mut diff, &tracked.stdout) {
            return Ok(diff);
        }

        let untracked = git(&["ls-files", "--others", "--exclude-standard", "-z"])?;
        if untracked.exit_code != 0 {
            return Err(CodexError::Protocol(format!(
                "git ls-files failed: {}",
                untracked.stderr.lines().next().unwrap_or("unknown error")
            )));
        }
        let paths = untracked
            .stdout
            .split('\0')
            .filter(|path| safe_git_relative_path(path))
            .collect::<Vec<_>>();
        for path in paths.iter().take(WORKSPACE_DIFF_FILE_LIMIT) {
            let run_untracked = |null_path: &str| {
                git(&[
                    "diff",
                    "--no-textconv",
                    "--no-ext-diff",
                    "--submodule=short",
                    "--ignore-submodules=dirty",
                    "--no-color",
                    "--no-index",
                    "--",
                    null_path,
                    path,
                ])
            };
            // A remote app-server may run a different OS than this Nickel client.
            let preferred_null = if cfg!(windows) { "NUL" } else { "/dev/null" };
            let alternate_null = if cfg!(windows) { "/dev/null" } else { "NUL" };
            let mut output = run_untracked(preferred_null)?;
            if !matches!(output.exit_code, 0 | 1) {
                output = run_untracked(alternate_null)?;
            }
            if !matches!(output.exit_code, 0 | 1) {
                return Err(CodexError::Protocol(format!(
                    "git diff for an untracked file failed: {}",
                    output.stderr.lines().next().unwrap_or("unknown error")
                )));
            }
            if !append_workspace_diff(&mut diff, &output.stdout) {
                return Ok(diff);
            }
        }
        if paths.len() > WORKSPACE_DIFF_FILE_LIMIT {
            append_workspace_diff(&mut diff, WORKSPACE_DIFF_OMISSION);
        }
        if diff.is_empty() {
            Ok("`/diff` — _no working-tree changes_".into())
        } else {
            Ok(format!("```diff\n{diff}\n```"))
        }
    }
    fn logout(&self) -> Result<(), CodexError> {
        self.request("account/logout", Value::Null)?;
        Ok(())
    }
    fn start_login(&self, method: crate::LoginMethod) -> Result<crate::LoginChallenge, CodexError> {
        let params = match method {
            crate::LoginMethod::Browser => json!({"type": "chatgpt"}),
            crate::LoginMethod::DeviceCode => json!({"type": "chatgptDeviceCode"}),
        };
        parse_login_challenge(method, self.request("account/login/start", params)?)
    }
    fn cancel_login(&self, login_id: &str) -> Result<(), CodexError> {
        if login_id.is_empty() || login_id.len() > 1024 {
            return Err(CodexError::Protocol("invalid login id".into()));
        }
        self.request("account/login/cancel", json!({"loginId": login_id}))?;
        Ok(())
    }
    fn remote_control_status(&self) -> Result<crate::RemoteControlStatus, CodexError> {
        parse_remote_control_status(self.request("remoteControl/status/read", json!({}))?)
    }
    fn enable_remote_control(
        &self,
        ephemeral: bool,
    ) -> Result<crate::RemoteControlStatus, CodexError> {
        parse_remote_control_status(
            self.request("remoteControl/enable", json!({"ephemeral": ephemeral}))?,
        )
    }
    fn disable_remote_control(
        &self,
        ephemeral: bool,
    ) -> Result<crate::RemoteControlStatus, CodexError> {
        parse_remote_control_status(
            self.request("remoteControl/disable", json!({"ephemeral": ephemeral}))?,
        )
    }
    fn start_remote_pairing(
        &self,
        manual_code: bool,
    ) -> Result<crate::RemotePairingChallenge, CodexError> {
        parse_remote_pairing(self.request(
            "remoteControl/pairing/start",
            json!({"manualCode": manual_code}),
        )?)
    }
    fn remote_pairing_claimed(
        &self,
        pairing_code: Option<&str>,
        manual_pairing_code: Option<&str>,
    ) -> Result<bool, CodexError> {
        let params = match (pairing_code, manual_pairing_code) {
            (Some(code), None) if !code.is_empty() && code.len() <= 8192 => {
                json!({"pairingCode": code})
            }
            (None, Some(code)) if !code.is_empty() && code.len() <= 128 => {
                json!({"manualPairingCode": code})
            }
            _ => {
                return Err(CodexError::Protocol(
                    "provide exactly one valid remote pairing code".into(),
                ));
            }
        };
        let value = self.request("remoteControl/pairing/status", params)?;
        value
            .get("claimed")
            .and_then(Value::as_bool)
            .ok_or_else(|| CodexError::Protocol("invalid remote-control field claimed".into()))
    }
    fn remote_control_clients(
        &self,
        environment_id: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<crate::RemoteControlClientPage, CodexError> {
        if environment_id.is_empty()
            || environment_id.len() > 1024
            || cursor.is_some_and(|cursor| cursor.is_empty() || cursor.len() > 1024)
            || !(1..=100).contains(&limit)
        {
            return Err(CodexError::Protocol(
                "invalid remote-control client-list parameters".into(),
            ));
        }
        let value = self.request(
            "remoteControl/client/list",
            json!({
                "environmentId": environment_id,
                "cursor": cursor,
                "limit": limit,
            }),
        )?;
        let data = value
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| CodexError::Protocol("invalid remote-control field data".into()))?;
        if data.len() > 100 {
            return Err(CodexError::Protocol(
                "remote-control client page exceeded requested bound".into(),
            ));
        }
        Ok(crate::RemoteControlClientPage {
            data: data
                .iter()
                .map(parse_remote_client)
                .collect::<Result<_, _>>()?,
            next_cursor: optional_bounded_remote_text(&value, "nextCursor", 1024)?,
        })
    }
    fn revoke_remote_control_client(
        &self,
        environment_id: &str,
        client_id: &str,
    ) -> Result<(), CodexError> {
        if environment_id.is_empty()
            || environment_id.len() > 1024
            || client_id.is_empty()
            || client_id.len() > 1024
        {
            return Err(CodexError::Protocol(
                "invalid remote-control client identity".into(),
            ));
        }
        self.request(
            "remoteControl/client/revoke",
            json!({"environmentId": environment_id, "clientId": client_id}),
        )?;
        Ok(())
    }
    fn models(&self) -> Result<Vec<Model>, CodexError> {
        let mut models = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let value = self.request("model/list", json!({"limit": 100, "cursor": cursor}))?;
            models.extend(
                value
                    .get("data")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|model| {
                        Some(Model {
                            id: model.get("id")?.as_str()?.into(),
                            display_name: model
                                .get("displayName")
                                .and_then(Value::as_str)
                                .unwrap_or_else(|| model["id"].as_str().unwrap_or_default())
                                .into(),
                            default_reasoning_effort: model
                                .get("defaultReasoningEffort")
                                .and_then(Value::as_str)
                                .map(Into::into),
                            supported_reasoning_efforts: model
                                .get("supportedReasoningEfforts")
                                .and_then(Value::as_array)
                                .into_iter()
                                .flatten()
                                .filter_map(|option| {
                                    Some(crate::ReasoningEffortOption {
                                        reasoning_effort: option
                                            .get("reasoningEffort")?
                                            .as_str()?
                                            .into(),
                                        description: option
                                            .get("description")
                                            .and_then(Value::as_str)
                                            .unwrap_or_default()
                                            .into(),
                                    })
                                })
                                .collect(),
                        })
                    }),
            );
            cursor = value
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(Into::into);
            if cursor.is_none() {
                break;
            }
        }
        Ok(models)
    }
    fn list_projects(&self, page: ProjectPage) -> Result<ProjectPageResult, CodexError> {
        let value = self.request(
            "project/list",
            json!({"cursor": page.cursor, "limit": page.limit}),
        )?;
        Ok(ProjectPageResult {
            projects: value
                .get("data")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(parse_project)
                .collect(),
            next_cursor: value
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(Into::into),
        })
    }
    fn import_project(&self, project: ImportProject) -> Result<Project, CodexError> {
        let value = self.request(
            "project/import",
            json!({
                "idempotencyKey": project.idempotency_key,
                "name": project.name,
                "roots": project.roots.into_iter().map(|path| json!({"path": path})).collect::<Vec<_>>(),
                "threads": project.threads.into_iter().map(|thread| thread.0).collect::<Vec<_>>(),
                "metadata": null
            }),
        )?;
        parse_project(value.get("project").unwrap_or(&value))
            .ok_or_else(|| CodexError::Protocol("project/import omitted project".into()))
    }
    fn list_threads(&self, page: ThreadPage) -> Result<ThreadPageResult, CodexError> {
        let value = self.request(
            "thread/list",
            json!({"cursor": page.cursor, "limit": page.limit}),
        )?;
        let mut threads = Vec::new();
        let mut runtime = HashMap::new();
        for value in value
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(thread) = parse_thread(value) {
                runtime.insert(thread.id.clone(), parse_thread_runtime(value));
                threads.push(thread);
            }
        }
        Ok(ThreadPageResult {
            threads,
            next_cursor: value
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(Into::into),
            runtime,
        })
    }
    fn start_thread(&self, request: StartThread) -> Result<Thread, CodexError> {
        let value = self.request(
            "thread/start",
            json!({"cwd": request.cwd, "model": request.model, "projectId": request.project_id,
                "approvalPolicy": request.approval_policy,
                "sandboxPolicy": request.sandbox_policy,
                "config": {"model_reasoning_effort": request.reasoning_effort}}),
        )?;
        parse_thread(value.get("thread").unwrap_or(&value))
            .ok_or_else(|| CodexError::Protocol("thread/start omitted thread".into()))
    }
    fn resume_thread(&self, id: ThreadId) -> Result<Thread, CodexError> {
        let value = self.request("thread/resume", json!({"threadId": id.0}))?;
        parse_thread(value.get("thread").unwrap_or(&value))
            .ok_or_else(|| CodexError::Protocol("thread/resume omitted thread".into()))
    }
    fn start_turn(&self, request: StartTurn) -> Result<Turn, CodexError> {
        let thread_id = request.thread_id.clone();
        let input = turn_input(request.text, request.images);
        let mut params = json!({"threadId": request.thread_id.0, "input": input, "model": request.model.clone(), "effort": request.reasoning_effort.clone(), "approvalPolicy": request.approval_policy, "sandboxPolicy": request.sandbox_policy});
        if request.plan_mode {
            let model = request.model.as_deref().ok_or_else(|| {
                CodexError::Protocol("Plan mode requires a selected model".into())
            })?;
            params["collaborationMode"] = json!({
                "mode": "plan",
                "settings": {
                    "model": model,
                    "reasoning_effort": request.reasoning_effort,
                    "developer_instructions": null
                }
            });
        }
        let value = self.request("turn/start", params)?;
        let turn = value.get("turn").unwrap_or(&value);
        Ok(Turn {
            id: TurnId(
                turn.get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| CodexError::Protocol("turn/start omitted id".into()))?
                    .into(),
            ),
            thread_id,
            status: turn
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("inProgress")
                .into(),
        })
    }
    fn compact_thread(&self, thread: ThreadId) -> Result<(), CodexError> {
        self.request("thread/compact/start", json!({"threadId": thread.0}))?;
        Ok(())
    }
    fn review_uncommitted(
        &self,
        thread: ThreadId,
        settings: crate::ReviewSettings,
    ) -> Result<Turn, CodexError> {
        self.request(
            "thread/settings/update",
            json!({
                "threadId": thread.0.clone(),
                "disabledPluginIds": null,
                "cwd": null,
                "approvalPolicy": settings.approval_policy,
                "approvalsReviewer": null,
                "sandboxPolicy": settings.sandbox_policy,
                "permissions": null,
                "model": settings.model,
                "serviceTier": null,
                "effort": settings.reasoning_effort,
                "summary": null,
                "collaborationMode": null,
                "multiAgentMode": null,
                "personality": null
            }),
        )?;
        let value = self.request(
            "review/start",
            json!({
                "threadId": thread.0,
                "target": {"type": "uncommittedChanges"},
                "delivery": "inline",
            }),
        )?;
        let turn = value
            .get("turn")
            .ok_or_else(|| CodexError::Protocol("review/start omitted turn".into()))?;
        Ok(Turn {
            id: TurnId(
                turn.get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| CodexError::Protocol("review/start omitted turn id".into()))?
                    .into(),
            ),
            thread_id: thread,
            status: turn
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("inProgress")
                .into(),
        })
    }
    fn shell_command(&self, thread: ThreadId, command: String) -> Result<(), CodexError> {
        self.request(
            "thread/shellCommand",
            json!({"threadId": thread.0, "command": command}),
        )?;
        Ok(())
    }
    fn interrupt_turn(&self, thread: ThreadId, turn: TurnId) -> Result<(), CodexError> {
        self.request(
            "turn/interrupt",
            json!({"threadId": thread.0, "turnId": turn.0}),
        )
        .map(|_| ())
    }
    fn respond(
        &self,
        request: ServerRequestId,
        response: InteractionResponse,
    ) -> Result<(), CodexError> {
        let mut outstanding = self.inner.outstanding.lock().unwrap();
        let pending = outstanding
            .get(&request.0)
            .ok_or_else(|| CodexError::InvalidInteraction("request is not pending".into()))?;
        let result = match (&*pending.method, response) {
            (
                "item/commandExecution/requestApproval",
                InteractionResponse::CommandApproval { decision },
            ) => json!({"decision": serde_json::to_value(decision)?}),
            (
                "item/fileChange/requestApproval",
                InteractionResponse::FileChangeApproval { decision },
            ) => json!({"decision": serde_json::to_value(decision)?}),
            ("item/tool/requestUserInput", InteractionResponse::UserInput { answers }) => {
                if answers
                    .iter()
                    .any(|answer| !pending.question_ids.contains(&answer.question_id))
                {
                    return Err(CodexError::InvalidInteraction(
                        "user-input answer does not match a requested question".into(),
                    ));
                }
                json!({"answers": answers.into_iter().map(|answer| (answer.question_id, json!({"answers": [answer.answer]}))).collect::<serde_json::Map<_,_>>() })
            }
            _ => {
                return Err(CodexError::InvalidInteraction(
                    "response kind does not match request".into(),
                ));
            }
        };
        let raw_id = pending.raw_id.clone();
        outstanding.remove(&request.0);
        drop(outstanding);
        self.write(&json!({"id": raw_id, "result": result}))
    }
    fn subscribe(&self) -> crate::delivery::DeliveryReceiver<CodexEvent> {
        let (tx, rx) = crate::delivery::channel();
        self.inner.subscribers.lock().unwrap().push(tx);
        rx
    }
}

fn parse_project(value: &Value) -> Option<Project> {
    Some(Project {
        id: value.get("id")?.as_str()?.into(),
        name: value.get("name")?.as_str()?.into(),
        roots: value
            .get("roots")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|root| root.get("path").and_then(Value::as_str).map(Into::into))
            .collect(),
    })
}

fn parse_thread_runtime(value: &Value) -> ThreadRuntime {
    let status = value.get("status");
    let status_type = status
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str);
    ThreadRuntime {
        project_id: value
            .get("projectId")
            .and_then(Value::as_str)
            .map(Into::into),
        status: match status_type {
            Some("notLoaded") => ThreadRuntimeStatus::NotLoaded,
            Some("idle") => ThreadRuntimeStatus::Idle,
            Some("active") => ThreadRuntimeStatus::Active,
            Some("systemError") => ThreadRuntimeStatus::SystemError,
            _ => ThreadRuntimeStatus::Unknown,
        },
        active_flags: status
            .and_then(|value| value.get("activeFlags"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(Into::into)
            .collect(),
        can_accept_direct_input: value.get("canAcceptDirectInput").and_then(Value::as_bool),
    }
}

fn parse_thread(value: &Value) -> Option<Thread> {
    Some(Thread {
        id: ThreadId(value.get("id")?.as_str()?.into()),
        title: value
            .get("name")
            .or_else(|| value.get("title"))
            .and_then(Value::as_str)
            .map(Into::into),
        cwd: value.get("cwd").and_then(Value::as_str).map(Into::into),
        last_used_at: value
            .get("recencyAt")
            .and_then(Value::as_i64)
            .or_else(|| value.get("updatedAt").and_then(Value::as_i64)),
        turns: value
            .get("turns")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(parse_history_turn)
            .collect(),
        model: value.get("model").and_then(Value::as_str).map(Into::into),
        reasoning_effort: value
            .get("reasoningEffort")
            .and_then(Value::as_str)
            .map(Into::into),
    })
}

fn parse_history_turn(value: &Value) -> Option<ThreadHistoryTurn> {
    Some(ThreadHistoryTurn {
        id: TurnId(value.get("id")?.as_str()?.into()),
        status: value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .into(),
        items: value
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(parse_history_item)
            .collect(),
    })
}

fn parse_history_item(value: &Value) -> Option<ThreadHistoryItem> {
    let item_type = value.get("type")?.as_str()?;
    let command_actions = if value.get("source").and_then(Value::as_str) == Some("userShell") {
        Vec::new()
    } else {
        value
            .get("commandActions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(parse_command_action)
            .collect()
    };
    Some(ThreadHistoryItem {
        id: value.get("id")?.as_str()?.into(),
        item_type: item_type.into(),
        // Hydration is transient backend data, but it must not hand unbounded item detail
        // to the UI before the transcript's own admission budget runs.
        text: bound_history_item_text(history_item_text(item_type, value)),
        command_actions,
        status: value
            .get("status")
            .and_then(Value::as_str)
            .filter(|status| status.len() <= 256)
            .map(str::to_owned),
        exit_code: value
            .get("exitCode")
            .and_then(Value::as_i64)
            .and_then(|code| i32::try_from(code).ok()),
        duration_ms: value
            .get("durationMs")
            .and_then(Value::as_i64)
            .filter(|duration| *duration >= 0),
    })
}

fn history_item_text(item_type: &str, value: &Value) -> String {
    match item_type {
        "userMessage" => value
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|input| match input.get("type").and_then(Value::as_str) {
                Some("text") => input.get("text").and_then(Value::as_str).map(str::to_owned),
                Some("image") | Some("localImage") => Some("[Image]".into()),
                Some("audio") | Some("localAudio") => Some("[Audio]".into()),
                Some("skill") => input
                    .get("name")
                    .and_then(Value::as_str)
                    .map(|name| format!("[Skill: {name}]")),
                Some("mention") => input
                    .get("name")
                    .and_then(Value::as_str)
                    .map(|name| format!("[@{name}]")),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        "agentMessage" | "plan" => value
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        // Raw reasoning content is private; only the supplied summary belongs in history.
        "reasoning" => value
            .get("summary")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
        "commandExecution" => {
            let command = value
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let output = value
                .get("aggregatedOutput")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match (command.is_empty(), output.is_empty()) {
                (false, false) => format!("$ {command}\n{output}"),
                (false, true) => format!("$ {command}"),
                (true, false) => output.into(),
                (true, true) => String::new(),
            }
        }
        "fileChange" => value
            .get("changes")
            .and_then(parse_file_changes)
            .map(|changes| {
                // History and live completion must project the same authoritative change
                // snapshot. The parser enforces the retained diff budget for both paths.
                format_file_changes(&changes)
            })
            .or_else(|| {
                value
                    .get("changes")
                    .and_then(Value::as_array)
                    .map(|changes| format!("{} file change(s); details unavailable", changes.len()))
            })
            .unwrap_or_else(|| "File changes unavailable".into()),
        "mcpToolCall" => {
            let server = value.get("server").and_then(Value::as_str).unwrap_or("MCP");
            let tool = value.get("tool").and_then(Value::as_str).unwrap_or("tool");
            let status = value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let mut lines = vec![format!("{server} / {tool} — {status}")];
            if let Some(error) = value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
            {
                lines.push(format!("Error: {error}"));
            }
            if let Some(content) = value
                .get("result")
                .and_then(|result| result.get("content"))
                .and_then(Value::as_array)
            {
                for part in content.iter().take(16) {
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        lines.push(text.into());
                    } else if let Some(kind) = part.get("type").and_then(Value::as_str) {
                        lines.push(format!("[{kind} result]"));
                    }
                }
            }
            lines.join("\n")
        }
        "dynamicToolCall" => {
            let namespace = value.get("namespace").and_then(Value::as_str);
            let tool = value.get("tool").and_then(Value::as_str).unwrap_or("tool");
            let status = value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let mut lines = vec![format!(
                "{}{} — {status}",
                namespace.map_or(String::new(), |namespace| format!("{namespace} / ")),
                tool
            )];
            if let Some(content) = value.get("contentItems").and_then(Value::as_array) {
                for part in content.iter().take(16) {
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        lines.push(text.into());
                    } else if let Some(kind) = part.get("type").and_then(Value::as_str) {
                        lines.push(format!("[{kind} result]"));
                    }
                }
            }
            lines.join("\n")
        }
        "webSearch" => value
            .get("query")
            .and_then(Value::as_str)
            .map(|query| format!("Search: {query}"))
            .unwrap_or_else(|| "Web search".into()),
        "imageView" => value
            .get("path")
            .and_then(Value::as_str)
            .map(|path| format!("Viewed image: {path}"))
            .unwrap_or_else(|| "Viewed image".into()),
        "imageGeneration" => {
            let status = value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let path = value
                .get("savedPath")
                .and_then(Value::as_str)
                .unwrap_or("No saved path reported");
            format!("Image generation — {status}\n{path}")
        }
        "collabAgentToolCall" => {
            let tool = value
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or("collaboration");
            let status = value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            format!("Delegated {tool} — {status}")
        }
        "subAgentActivity" => value
            .get("agentPath")
            .and_then(Value::as_str)
            .map(|path| format!("Subagent: {path}"))
            .unwrap_or_else(|| "Subagent activity".into()),
        "enteredReviewMode" => "Entered review mode".into(),
        "exitedReviewMode" => "Exited review mode".into(),
        "contextCompaction" => "Context compacted".into(),
        _ => value
            .get("text")
            .or_else(|| value.get("result"))
            .or_else(|| value.get("query"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
    }
}

#[cfg(test)]
mod tests {
    use std::{net::TcpListener, sync::Arc};

    use proptest::prelude::*;
    use tungstenite::{Message, accept_hdr};

    #[test]
    fn turn_input_associates_text_and_images_exactly_once() {
        let input = turn_input(
            "hello 世界".into(),
            vec![crate::TurnImage {
                data_url: "data:image/png;base64,c2VjcmV0".into(),
            }],
        );
        assert_eq!(
            input,
            vec![
                json!({"type":"text", "text":"hello 世界"}),
                json!({"type":"image", "url":"data:image/png;base64,c2VjcmV0"}),
            ]
        );
    }

    #[test]
    fn fuzzy_file_search_parser_preserves_typed_bounded_matches() {
        let matches = parse_file_search_matches(&json!({
            "files": [{
                "root": "/projects/nickel",
                "path": "crates/nickel/src/main.rs",
                "matchType": "file",
                "fileName": "main.rs",
                "score": 91,
                "indices": [0, 1]
            }]
        }))
        .unwrap();
        assert_eq!(matches[0].path, "crates/nickel/src/main.rs");
        assert!(!matches[0].is_directory);
        assert_eq!(matches[0].score, 91);
        assert!(
            parse_file_search_matches(&json!({
                "files": [{
                    "root": "/projects/nickel",
                    "path": "src",
                    "matchType": "unknown",
                    "fileName": "src",
                    "score": 1
                }]
            }))
            .is_err()
        );
    }

    #[test]
    fn workspace_diff_helpers_reject_escaping_paths_and_bound_unicode_output() {
        assert!(safe_git_relative_path("crates/nickel/src/main.rs"));
        assert!(!safe_git_relative_path("../secret"));
        assert!(!safe_git_relative_path("src/../../secret"));
        assert!(!safe_git_relative_path("/etc/passwd"));
        assert!(!safe_git_relative_path("C:\\Windows\\system.ini"));

        let mut output = "prefix\n".to_owned();
        assert!(!append_workspace_diff(
            &mut output,
            &"é".repeat(WORKSPACE_DIFF_BYTES)
        ));
        assert!(output.len() <= WORKSPACE_DIFF_BYTES);
        assert!(output.ends_with(WORKSPACE_DIFF_OMISSION));
        assert!(std::str::from_utf8(output.as_bytes()).is_ok());

        let mut nearly_full = "x".repeat(WORKSPACE_DIFF_BYTES - 1);
        assert!(!append_workspace_diff(&mut nearly_full, "overflow"));
        assert_eq!(nearly_full.len(), WORKSPACE_DIFF_BYTES);
        assert!(nearly_full.ends_with(WORKSPACE_DIFF_OMISSION));
    }

    use super::*;

    #[test]
    fn approval_decision_set_distinguishes_missing_empty_and_malformed() {
        assert_eq!(parse_available_decisions(&json!({})), Ok(None));
        assert_eq!(
            parse_available_decisions(&json!({"availableDecisions":[]})),
            Ok(Some(vec![]))
        );
        assert_eq!(
            parse_available_decisions(
                &json!({"availableDecisions":["acceptForSession", "decline"]})
            ),
            Ok(Some(vec![
                crate::CommandDecision::AcceptForSession,
                crate::CommandDecision::Decline,
            ]))
        );
        assert!(parse_available_decisions(&json!({"availableDecisions":["unknown"]})).is_err());
        assert!(parse_available_decisions(&json!({"availableDecisions":"accept"})).is_err());
    }

    #[test]
    fn upstream_patch_and_turn_plan_wire_shapes_preserve_structured_snapshots() {
        let patch = parse_file_patch_updated(&json!({
            "threadId":"thread", "turnId":"turn", "itemId":"file-1",
            "changes":[{"path":"/project/src/main.rs", "kind":{"type":"update","move_path":"/project/src/lib.rs"},
                "diff":"@@ -1 +1 @@\n-old\n+new"}]
        }));
        assert!(
            matches!(patch, EventKind::FilePatchUpdated { thread_id, turn_id, item_id, changes }
            if thread_id.0 == "thread" && turn_id.0 == "turn" && item_id == "file-1"
                && changes[0].move_path.as_deref() == Some("/project/src/lib.rs")
                && changes[0].diff.contains("+new"))
        );
        let plan = parse_turn_plan_updated(&json!({
            "threadId":"thread", "turnId":"turn", "explanation":"Check the implementation",
            "plan":[{"step":"Inspect","status":"completed"},{"step":"Test","status":"inProgress"}]
        }));
        assert!(
            matches!(plan, EventKind::TurnPlanUpdated { thread_id, turn_id, explanation, steps }
            if thread_id.0 == "thread" && turn_id.0 == "turn"
                && explanation.as_deref() == Some("Check the implementation")
                && steps[1].status == "inProgress")
        );
        assert!(matches!(
            parse_turn_plan_updated(&json!({"plan":"not an array"})),
            EventKind::Inconsistency { .. }
        ));
    }

    #[test]
    fn completed_item_uses_final_wire_payload_and_never_exposes_raw_reasoning() {
        let completed = parse_item_completed(&json!({
            "threadId":"thread", "turnId":"turn", "completedAtMs":42,
            "item":{"id":"command-1","type":"commandExecution","command":"pwd",
                "commandActions":[],"cwd":"/project","status":"completed",
                "aggregatedOutput":"/project\n","exitCode":0,"durationMs":12}
        }));
        assert!(
            matches!(completed, EventKind::ItemCompleted { item_id, completion: Some(item) }
            if item_id == "command-1" && item.thread_id.0 == "thread"
                && item.text == "$ pwd\n/project\n" && item.exit_code == Some(0))
        );

        let reasoning = parse_item_completed(&json!({
            "threadId":"thread", "turnId":"turn", "completedAtMs":43,
            "item":{"id":"reasoning-1","type":"reasoning",
                "summary":["Public summary"], "content":["Private chain"]}
        }));
        assert!(
            matches!(reasoning, EventKind::ItemCompleted { completion: Some(item), .. }
            if item.text == "Public summary" && !item.text.contains("Private chain"))
        );
    }

    #[test]
    fn file_change_history_matches_live_completion_and_bounds_bad_detail() {
        let item = json!({
            "id":"file-1", "type":"fileChange", "status":"completed",
            "changes":[{"path":"/project/src/main.rs", "kind":{"type":"update"},
                "diff":"@@ -1 +1 @@\n-old\n+new"}]
        });
        let live = parse_item_completed(&json!({
            "threadId":"thread", "turnId":"turn", "completedAtMs":42, "item":item
        }));
        let EventKind::ItemCompleted {
            completion: Some(live),
            ..
        } = live
        else {
            panic!("expected live file-change completion");
        };
        let resumed = parse_history_item(&item).expect("resumed file change");
        assert_eq!(resumed.text, live.text);
        assert!(resumed.text.contains("+new"));

        let oversized = json!({
            "id":"file-2", "type":"fileChange",
            "changes":[{"path":"/project/src/main.rs", "kind":{"type":"update"},
                "diff":"x".repeat(256 * 1024 + 1)}]
        });
        let resumed = parse_history_item(&oversized).expect("bounded history fallback");
        assert_eq!(resumed.text, "1 file change(s); details unavailable");
    }

    #[test]
    fn resumed_activity_detail_is_utf8_safe_and_visibly_bounded() {
        let oversized = json!({
            "id":"command-1", "type":"commandExecution", "command":"echo hello",
            "aggregatedOutput":"界".repeat(100_000)
        });
        let resumed = parse_history_item(&oversized).expect("bounded history item");
        assert!(resumed.text.len() <= MAX_HISTORY_ITEM_TEXT_BYTES);
        assert!(resumed.text.starts_with("$ echo hello\n"));
        assert!(resumed.text.ends_with(HISTORY_OMISSION_MARKER));
    }

    #[test]
    fn resumed_command_retains_bounded_terminal_wire_facts() {
        let item = parse_history_item(&json!({
            "id":"command-1", "type":"commandExecution", "command":"cargo test",
            "status":"failed", "exitCode":2, "durationMs":17,
            "aggregatedOutput":"failure"
        }))
        .expect("history item");
        assert_eq!(item.status.as_deref(), Some("failed"));
        assert_eq!(item.exit_code, Some(2));
        assert_eq!(item.duration_ms, Some(17));
        let malformed_metadata = parse_history_item(&json!({
            "id":"command-2", "type":"commandExecution", "status":"x".repeat(300),
            "durationMs":-1
        }))
        .expect("history item without unusable metadata");
        assert!(malformed_metadata.status.is_none());
        assert!(malformed_metadata.duration_ms.is_none());
    }

    #[test]
    fn user_input_questions_keep_prompt_options_and_secret_flag() {
        let questions = parse_user_input_questions(&json!({
            "questions":[{"id":"choice","header":"Proceed","question":"Apply the patch?",
                "options":[{"label":"Yes","description":"Apply it"},
                    {"label":"No","description":"Leave files unchanged"}],
                "isOther":true,"isSecret":false},
                {"id":"token","header":"Credential","question":"Enter the token",
                    "options":null,"isSecret":true}]
        }))
        .unwrap();
        assert_eq!(questions[0].question, "Apply the patch?");
        assert_eq!(questions[0].options[1].description, "Leave files unchanged");
        assert!(questions[0].is_other);
        assert!(questions[1].is_secret);
        assert!(parse_user_input_questions(&json!({"questions":[{"id":"q"}]})).is_none());
    }

    #[test]
    fn activity_starts_have_bounded_typed_identity_before_completion() {
        let cases = [
            (
                json!({"type":"mcpToolCall", "server":"files", "tool":"read"}),
                "files / read",
            ),
            (
                json!({"type":"dynamicToolCall", "namespace":"browser", "tool":"open"}),
                "browser / open",
            ),
            (
                json!({"type":"webSearch", "query":"Nickel"}),
                "Search: Nickel",
            ),
            (
                json!({"type":"imageView", "path":"/tmp/photo.png"}),
                "Viewed image: /tmp/photo.png",
            ),
            (
                json!({"type":"collabAgentToolCall", "tool":"spawnAgent"}),
                "Delegated spawnAgent",
            ),
        ];
        for (item, expected) in cases {
            assert!(initial_item_text(&item).contains(expected));
        }
        assert_eq!(
            initial_item_text(&json!({"type":"agentMessage", "text":"later"})),
            ""
        );
        assert_eq!(
            initial_item_text(&json!({"type":"webSearch", "query":"x".repeat(4097)})),
            ""
        );
    }

    #[test]
    fn upstream_retrying_and_final_turn_errors_keep_scope_and_retryability() {
        for will_retry in [true, false] {
            let event = parse_turn_error(&json!({
                "threadId":"thread", "turnId":"turn", "willRetry":will_retry,
                "error":{"message":"Service unavailable","additionalDetails":null,
                    "codexErrorInfo":null}
            }));
            assert!(
                matches!(event, EventKind::TurnError { thread_id, turn_id, message, will_retry: retry }
                if thread_id.0 == "thread" && turn_id.0 == "turn"
                    && message == "Service unavailable" && retry == will_retry)
            );
        }
        assert!(matches!(
            parse_turn_error(&json!({"error":{"message":"oops"}})),
            EventKind::Inconsistency { .. }
        ));
    }

    #[test]
    fn public_warning_wire_shapes_keep_scope_and_reject_malformed_guardian_warning() {
        assert!(matches!(
            parse_warning("warning", &json!({"threadId":"thread", "message":"Low quota"})),
            EventKind::Warning { thread_id: Some(thread), message, guardian: false }
                if thread.0 == "thread" && message == "Low quota"
        ));
        assert!(matches!(
            parse_warning("configWarning", &json!({"summary":"Deprecated setting"})),
            EventKind::Warning { thread_id: None, message, guardian: false }
                if message == "Deprecated setting"
        ));
        assert!(matches!(
            parse_warning("guardianWarning", &json!({"message":"Review required"})),
            EventKind::Inconsistency { .. }
        ));
    }

    #[test]
    fn live_tool_completion_matches_resumed_history_text() {
        let items = [
            json!({"id":"mcp","type":"mcpToolCall","server":"files","tool":"read",
                "arguments":{"path":"/project/readme"},"status":"completed",
                "result":{"content":[{"type":"text","text":"Read complete"}]}}),
            json!({"id":"search","type":"webSearch","query":"Nickel docs"}),
            json!({"id":"agent","type":"subAgentActivity","agentPath":"/root/review",
                "agentThreadId":"agent-thread","kind":{"type":"started"}}),
        ];
        for item in items {
            let item_type = item["type"].as_str().unwrap();
            let resumed = history_item_text(item_type, &item);
            let live = parse_item_completed(&json!({
                "threadId":"thread","turnId":"turn","completedAtMs":42,"item":item
            }));
            assert!(
                matches!(live, EventKind::ItemCompleted { completion: Some(completion), .. }
                if completion.text == resumed)
            );
        }
    }

    #[test]
    fn login_challenges_require_the_requested_flow_and_bounded_fields() {
        assert_eq!(
            parse_login_challenge(
                crate::LoginMethod::Browser,
                json!({"type":"chatgpt","loginId":"login-1","authUrl":"https://example.test/auth"}),
            )
            .unwrap(),
            crate::LoginChallenge::Browser {
                login_id: "login-1".into(),
                auth_url: "https://example.test/auth".into(),
            }
        );
        assert_eq!(
            parse_login_challenge(
                crate::LoginMethod::DeviceCode,
                json!({"type":"chatgptDeviceCode","loginId":"login-2","userCode":"ABCD-EFGH","verificationUrl":"https://example.test/device"}),
            )
            .unwrap(),
            crate::LoginChallenge::DeviceCode {
                login_id: "login-2".into(),
                user_code: "ABCD-EFGH".into(),
                verification_url: "https://example.test/device".into(),
            }
        );
        assert!(parse_login_challenge(
            crate::LoginMethod::Browser,
            json!({"type":"chatgptDeviceCode","loginId":"login-3","userCode":"1234","verificationUrl":"https://example.test"}),
        ).is_err());
        assert!(
            parse_login_challenge(
                crate::LoginMethod::Browser,
                json!({"type":"chatgpt","loginId":"login-4","authUrl":"x".repeat(8193)}),
            )
            .is_err()
        );
    }

    #[test]
    fn isolated_profile_override_must_be_absolute_before_process_spawn() {
        let error = match CodexClient::spawn_with_home(
            Path::new("/definitely/not/executed"),
            Path::new("/tmp"),
            Path::new("relative-profile"),
        ) {
            Ok(_) => panic!("relative profile unexpectedly started"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("must be an absolute path"));
    }

    #[test]
    fn remote_outbound_budget_includes_in_flight_and_close_bypasses_saturation() {
        let (writer, receiver) = RemoteWriter::channel();
        // Simulate an in-flight payload; exercise remaining-byte admission.
        writer
            .retained
            .store(MAX_OUTBOUND_BYTES - 4, Ordering::Release);
        assert!(matches!(
            writer.try_send(RemoteWrite::Text("12345".into())),
            Err(mpsc::TrySendError::Full(_))
        ));
        assert_eq!(
            writer.retained.load(Ordering::Acquire),
            MAX_OUTBOUND_BYTES - 4
        );
        writer.retained.store(0, Ordering::Release);
        assert!(writer.try_send(RemoteWrite::Text("1234".into())).is_ok());
        let pending = receiver.try_recv().unwrap();
        assert_eq!(writer.retained.load(Ordering::Acquire), 4);
        drop(pending);
        assert_eq!(writer.retained.load(Ordering::Acquire), 0);
        for _ in 0..OUTBOUND_BACKLOG {
            assert!(writer.try_send(RemoteWrite::Text("x".into())).is_ok());
        }
        assert!(matches!(
            writer.try_send(RemoteWrite::Text("y".into())),
            Err(mpsc::TrySendError::Full(_))
        ));
        assert!(writer.try_send(RemoteWrite::Close).is_ok());
        assert!(writer.closed.load(Ordering::Acquire));
        drop(receiver);
        assert_eq!(writer.retained.load(Ordering::Acquire), 0);
    }

    #[test]
    fn project_and_thread_runtime_use_v2_app_server_fields() {
        let project = parse_project(&json!({
            "id": "project-1",
            "name": "Nickel",
            "roots": [{"path": "/projects/nickel"}, {"path": "/projects/nickel-alt"}]
        }))
        .unwrap();
        assert_eq!(
            project.roots,
            vec![
                std::path::PathBuf::from("/projects/nickel"),
                std::path::PathBuf::from("/projects/nickel-alt"),
            ]
        );

        let runtime = parse_thread_runtime(&json!({
            "projectId": "project-1",
            "status": {"type": "active", "activeFlags": ["waitingOnUserInput"]},
            "canAcceptDirectInput": false
        }));
        assert_eq!(runtime.project_id.as_deref(), Some("project-1"));
        assert_eq!(runtime.status, ThreadRuntimeStatus::Active);
        assert_eq!(runtime.active_flags, ["waitingOnUserInput"]);
        assert_eq!(runtime.can_accept_direct_input, Some(false));

        for (wire, expected) in [
            (Some("notLoaded"), ThreadRuntimeStatus::NotLoaded),
            (Some("idle"), ThreadRuntimeStatus::Idle),
            (Some("active"), ThreadRuntimeStatus::Active),
            (Some("systemError"), ThreadRuntimeStatus::SystemError),
            (Some("futureStatus"), ThreadRuntimeStatus::Unknown),
            (None, ThreadRuntimeStatus::Unknown),
        ] {
            let value =
                wire.map_or_else(|| json!({}), |status| json!({"status": {"type": status}}));
            assert_eq!(parse_thread_runtime(&value).status, expected);
        }
    }

    #[test]
    #[allow(clippy::result_large_err)]
    fn remote_websocket_runs_typed_requests_with_bearer_auth_and_remote_cwd() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("ws://{}/app-server", listener.local_addr().unwrap());
        let authenticated = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed_auth = authenticated.clone();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = accept_hdr(
                stream,
                move |request: &tungstenite::handshake::server::Request,
                      response: tungstenite::handshake::server::Response| {
                    observed_auth.store(
                        request
                            .headers()
                            .get(AUTHORIZATION)
                            .is_some_and(|value| value == "Bearer fixture-secret"),
                        Ordering::Relaxed,
                    );
                    Ok(response)
                },
            )
            .unwrap();
            let mut saw_remote_cwd = false;
            let mut saw_approval = false;
            let mut saw_shell = false;
            let mut saw_interrupt = false;
            let mut saw_reasoning = false;
            let mut saw_thread_policy = false;
            let mut saw_turn_policy = false;
            let mut saw_compact = false;
            let mut saw_review = false;
            let mut saw_review_settings = false;
            let mut saw_logout = false;
            let mut saw_rate_limits = false;
            let mut saw_plan_mode = false;
            let mut saw_file_search = false;
            let mut saw_feedback = false;
            let mut workspace_diff_requests = 0usize;
            while !saw_interrupt {
                let Message::Text(text) = socket.read().unwrap() else {
                    continue;
                };
                let value: Value = serde_json::from_str(text.as_ref()).unwrap();
                let Some(id) = value.get("id").cloned() else {
                    continue;
                };
                let Some(method) = value.get("method").and_then(Value::as_str) else {
                    saw_approval |= id == "approval-1" && value.get("result").is_some();
                    if saw_approval {
                        socket
                            .send(Message::Text(
                                json!({
                                    "method":"serverRequest/resolved",
                                    "params":{"threadId":"remote-thread","requestId":"approval-1"}
                                })
                                .to_string()
                                .into(),
                            ))
                            .unwrap();
                    }
                    continue;
                };
                let result = match method {
                    "initialize" => json!({}),
                    "account/read" => {
                        if saw_logout {
                            json!({"account":null})
                        } else {
                            json!({"account":{"type":"chatgpt"}})
                        }
                    }
                    "account/logout" => {
                        saw_logout = value.get("params").is_none();
                        json!({})
                    }
                    "account/rateLimits/read" => {
                        saw_rate_limits = value.get("params").is_none();
                        json!({
                            "ordinaryUsageAllowed": true,
                            "rateLimits": {"limitId":"codex","limitName":"Codex","primary":{"usedPercent":25},"secondary":{"usedPercent":40}},
                            "rateLimitsByLimitId": null,
                        })
                    }
                    "model/list" => json!({"data":[{
                        "id":"gpt-fixture","displayName":"GPT Fixture",
                        "defaultReasoningEffort":"medium",
                        "supportedReasoningEfforts":[
                            {"reasoningEffort":"low","description":"Fast"},
                            {"reasoningEffort":"high","description":"Deep"}
                        ]
                    }],"nextCursor":null}),
                    "fuzzyFileSearch" => {
                        saw_file_search = value["params"]["query"] == "main"
                            && value["params"]["roots"] == json!(["/srv/code/nickel"]);
                        json!({"files":[{
                            "root":"/srv/code/nickel",
                            "path":"src/main.rs",
                            "matchType":"file",
                            "fileName":"main.rs",
                            "score":99,
                            "indices":[4,5,6,7]
                        }]})
                    }
                    "feedback/upload" => {
                        saw_feedback = value["params"]["classification"] == "bug"
                            && value["params"]["reason"] == "The picker broke"
                            && value["params"]["threadId"] == "remote-thread"
                            && value["params"]["includeLogs"] == false
                            && value["params"]["extraLogFiles"].is_null()
                            && value["params"]["tags"].is_null();
                        json!({"threadId":"feedback-report-1","promptHash":null})
                    }
                    "command/exec" => {
                        workspace_diff_requests += 1;
                        assert_eq!(value["params"]["cwd"], "/srv/code/nickel");
                        assert_eq!(value["params"]["outputBytesCap"], WORKSPACE_DIFF_BYTES);
                        assert_eq!(value["params"]["timeoutMs"], 30_000);
                        assert_eq!(value["params"]["sandboxPolicy"]["type"], "readOnly");
                        let command = value["params"]["command"].as_array().unwrap();
                        let has = |needle: &str| command.iter().any(|part| part == needle);
                        if has("rev-parse") {
                            json!({"exitCode":0,"stdout":"true\n","stderr":""})
                        } else if has("ls-files") {
                            json!({"exitCode":0,"stdout":"new.txt\u{0}","stderr":""})
                        } else if has("--no-index") {
                            json!({"exitCode":1,"stdout":"diff --git a/new.txt b/new.txt\n+untracked\n","stderr":""})
                        } else if has("diff") {
                            json!({"exitCode":0,"stdout":"diff --git a/src/main.rs b/src/main.rs\n-old\n+new\n","stderr":""})
                        } else {
                            panic!("unexpected command/exec argv: {command:?}")
                        }
                    }
                    "thread/list" => json!({"data":[],"nextCursor":null}),
                    "thread/start" => {
                        saw_remote_cwd = value["params"]["cwd"] == "/srv/code/nickel"
                            && value["params"]["projectId"] == "remote-project"
                            && value["params"]["config"]["model_reasoning_effort"] == "high";
                        saw_thread_policy = value["params"]["approvalPolicy"] == "never"
                            && value["params"]["sandboxPolicy"]["type"] == "dangerFullAccess";
                        json!({"thread":{"id":"remote-thread","cwd":"/srv/code/nickel"}})
                    }
                    "turn/start" => {
                        assert_eq!(
                            value["params"]["input"][1]["url"].as_str().unwrap().len(),
                            9 * 1024 * 1024
                        );
                        saw_reasoning = value["params"]["effort"] == "high";
                        saw_turn_policy = value["params"]["approvalPolicy"] == "never"
                            && value["params"]["sandboxPolicy"]["type"] == "dangerFullAccess";
                        saw_plan_mode = value["params"]["collaborationMode"]["mode"] == "plan"
                            && value["params"]["collaborationMode"]["settings"]["model"]
                                == "gpt-fixture"
                            && value["params"]["collaborationMode"]["settings"]
                                ["reasoning_effort"]
                                == "high"
                            && value["params"]["collaborationMode"]["settings"]
                                ["developer_instructions"]
                                .is_null();
                        json!({"turn":{"id":"remote-turn","status":"inProgress"}})
                    }
                    "thread/compact/start" => {
                        saw_compact = value["params"]["threadId"] == "remote-thread";
                        json!({})
                    }
                    "review/start" => {
                        saw_review = value["params"]["threadId"] == "remote-thread"
                            && value["params"]["target"]["type"] == "uncommittedChanges"
                            && value["params"]["delivery"] == "inline";
                        json!({"turn":{"id":"review-turn","status":"inProgress"},"reviewThreadId":"remote-thread"})
                    }
                    "thread/settings/update" => {
                        saw_review_settings = value["params"]["threadId"] == "remote-thread"
                            && value["params"]["model"] == "gpt-fixture"
                            && value["params"]["effort"] == "high"
                            && value["params"]["approvalPolicy"] == "never"
                            && value["params"]["sandboxPolicy"]["type"] == "dangerFullAccess";
                        json!({})
                    }
                    "thread/shellCommand" => {
                        saw_shell = value["params"]["threadId"] == "remote-thread"
                            && value["params"]["command"] == "printf hello | wc -c";
                        json!({})
                    }
                    "turn/interrupt" => {
                        saw_interrupt = value["params"]["threadId"] == "remote-thread"
                            && value["params"]["turnId"] == "remote-turn";
                        json!({})
                    }
                    method => panic!("unexpected method {method}"),
                };
                socket
                    .send(Message::Text(
                        json!({"id":id,"result":result}).to_string().into(),
                    ))
                    .unwrap();
                if method == "turn/start" {
                    socket
                        .send(Message::Text(
                            json!({
                                "method":"item/started",
                                "params":{"threadId":"remote-thread","turnId":"remote-turn","item":{"id":"agent-1","type":"agentMessage"}}
                            })
                            .to_string()
                            .into(),
                        ))
                        .unwrap();
                    socket
                        .send(Message::Text(
                            json!({
                                "method":"item/agentMessage/delta",
                                "params":{"itemId":"agent-1","delta":"remote response"}
                            })
                            .to_string()
                            .into(),
                        ))
                        .unwrap();
                    socket
                        .send(Message::Text(
                            json!({
                                "id":"approval-1",
                                "method":"item/commandExecution/requestApproval",
                                "params":{
                                    "threadId":"remote-thread",
                                    "turnId":"remote-turn",
                                    "itemId":"shell-1",
                                    "reason":"fixture approval",
                                    "command":"cargo test",
                                    "cwd":"/srv/code/nickel",
                                    "kind":"command",
                                    "availableDecisions":["accept", "acceptForSession", "decline", "cancel"],
                                    "networkApprovalContext":{"host":"example.test"},
                                    "proposedExecpolicyAmendment":["cargo test"]
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .unwrap();
                } else if method == "thread/shellCommand" {
                    for notification in [
                        json!({
                            "method":"item/started",
                            "params":{"threadId":"remote-thread","item":{
                                "id":"shell-1","type":"commandExecution",
                                "source":"userShell","command":"printf hello | wc -c"
                            }}
                        }),
                        json!({
                            "method":"item/commandExecution/outputDelta",
                            "params":{"itemId":"shell-1","delta":"5\n"}
                        }),
                        json!({
                            "method":"item/completed",
                            "params":{"threadId":"remote-thread","turnId":"remote-turn","completedAtMs":42,
                                "item":{"id":"shell-1","type":"commandExecution",
                                    "command":"printf hello | wc -c","commandActions":[],
                                    "cwd":"/srv/code/nickel","status":"completed",
                                    "aggregatedOutput":"5\n","exitCode":0}}
                        }),
                    ] {
                        socket
                            .send(Message::Text(notification.to_string().into()))
                            .unwrap();
                    }
                }
            }
            [
                saw_remote_cwd,
                saw_approval,
                saw_shell,
                saw_interrupt,
                saw_reasoning,
                saw_thread_policy,
                saw_turn_policy,
                saw_compact,
                saw_review,
                saw_review_settings,
                saw_logout,
                saw_rate_limits,
                saw_plan_mode,
                saw_file_search,
                saw_feedback,
                workspace_diff_requests == 4,
            ]
        });

        let client = CodexClient::connect_remote_with_timeout(
            &endpoint,
            Some("fixture-secret"),
            Duration::from_secs(2),
        )
        .unwrap();
        assert!(client.account().unwrap().authenticated);
        let models = client.models().unwrap();
        assert_eq!(
            models[0].default_reasoning_effort.as_deref(),
            Some("medium")
        );
        assert_eq!(
            models[0].supported_reasoning_efforts[1].reasoning_effort,
            "high"
        );
        assert!(
            client
                .list_threads(ThreadPage::default())
                .unwrap()
                .threads
                .is_empty()
        );
        let events = client.subscribe();
        let thread = client
            .start_thread(StartThread {
                cwd: "/srv/code/nickel".into(),
                model: None,
                project_id: Some("remote-project".into()),
                reasoning_effort: Some("high".into()),
                approval_policy: ApprovalPolicy::Never,
                sandbox_policy: Some(crate::SandboxPolicy::DangerFullAccess),
            })
            .unwrap();
        let turn = client
            .start_turn(StartTurn {
                thread_id: thread.id.clone(),
                text: "hello remotely".into(),
                images: vec![crate::TurnImage {
                    data_url: "a".repeat(9 * 1024 * 1024),
                }],
                model: Some("gpt-fixture".into()),
                reasoning_effort: Some("high".into()),
                approval_policy: ApprovalPolicy::Never,
                sandbox_policy: Some(crate::SandboxPolicy::DangerFullAccess),
                plan_mode: true,
            })
            .unwrap();
        assert_eq!(turn.id.0, "remote-turn");
        client.compact_thread(thread.id.clone()).unwrap();
        assert_eq!(
            client
                .review_uncommitted(
                    thread.id.clone(),
                    crate::ReviewSettings {
                        model: Some("gpt-fixture".into()),
                        reasoning_effort: Some("high".into()),
                        approval_policy: ApprovalPolicy::Never,
                        sandbox_policy: Some(crate::SandboxPolicy::DangerFullAccess),
                    },
                )
                .unwrap()
                .id
                .0,
            "review-turn"
        );
        client
            .shell_command(thread.id, "printf hello | wc -c".into())
            .unwrap();
        for _ in 0..100 {
            if client
                .projection()
                .items
                .get("shell-1")
                .is_some_and(|item| item.completed)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let request_id = loop {
            let event = events.recv_timeout(Duration::from_secs(2)).unwrap();
            if let EventKind::ApprovalRequested {
                request_id,
                thread_id,
                context,
                ..
            } = event.kind
            {
                assert_eq!(thread_id, Some(ThreadId("remote-thread".into())));
                assert_eq!(context.command.as_deref(), Some("cargo test"));
                assert_eq!(context.cwd.as_deref(), Some("/srv/code/nickel"));
                assert!(context.asks_network_access);
                assert!(context.proposes_session_rule);
                assert_eq!(
                    context.available_decisions,
                    Some(vec![
                        crate::CommandDecision::Accept,
                        crate::CommandDecision::AcceptForSession,
                        crate::CommandDecision::Decline,
                        crate::CommandDecision::Cancel,
                    ])
                );
                break request_id;
            }
        };
        assert!(client.projection().items["agent-1"].text.is_empty());
        assert!(client.projection().items["shell-1"].text.is_empty());
        client
            .respond(
                request_id,
                InteractionResponse::CommandApproval {
                    decision: CommandDecision::Accept,
                },
            )
            .unwrap();
        let resolved = loop {
            let event = events.recv_timeout(Duration::from_secs(2)).unwrap();
            if let EventKind::ServerRequestResolved {
                thread_id,
                request_id,
            } = event.kind
            {
                break (thread_id, request_id);
            }
        };
        assert_eq!(resolved.0, ThreadId("remote-thread".into()));
        assert_eq!(resolved.1, ServerRequestId("approval-1".into()));
        let limits = client.rate_limits().unwrap();
        assert_eq!(limits.ordinary_usage_allowed, Some(true));
        assert_eq!(limits.buckets[0].primary_used_percent, Some(25));
        let matches = client
            .search_files("main".into(), vec!["/srv/code/nickel".into()])
            .unwrap();
        assert_eq!(matches[0].path, "src/main.rs");
        assert_eq!(
            client
                .upload_feedback(
                    "bug".into(),
                    Some("The picker broke".into()),
                    Some(ThreadId("remote-thread".into())),
                    false,
                )
                .unwrap(),
            "feedback-report-1"
        );
        let diff = client
            .workspace_diff(std::path::PathBuf::from("/srv/code/nickel"))
            .unwrap();
        assert!(diff.contains("+new"));
        assert!(diff.contains("+untracked"));
        client.logout().unwrap();
        assert!(!client.account().unwrap().authenticated);
        client
            .interrupt_turn(ThreadId("remote-thread".into()), turn.id)
            .unwrap();
        client.shutdown();
        assert_eq!(
            server.join().unwrap(),
            [
                true, true, true, true, true, true, true, true, true, true, true, true, true, true,
                true, true
            ]
        );
        assert!(authenticated.load(Ordering::Relaxed));
    }

    #[test]
    fn remote_websocket_rejects_binary_malformed_oversized_and_abrupt_sessions() {
        fn rejected(message: Option<Message>) -> String {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("ws://{}/app-server", listener.local_addr().unwrap());
            let server = thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                let mut socket = tungstenite::accept(stream).unwrap();
                let _ = socket.read().unwrap();
                if let Some(message) = message {
                    // An oversized frame can be rejected from its header before
                    // the server finishes writing the payload.
                    let _ = socket.send(message);
                }
            });
            let error = CodexClient::connect_remote_with_timeout(
                &endpoint,
                None,
                Duration::from_millis(500),
            )
            .err()
            .expect("remote connection must fail")
            .to_string();
            server.join().unwrap();
            error
        }

        assert!(
            rejected(Some(Message::Binary(vec![1, 2, 3].into())))
                .contains("binary protocol message")
        );
        assert!(rejected(Some(Message::Text("not json".into()))).contains("malformed"));
        let oversized = serde_json::json!({
            "id": 1,
            "result": {"padding": "x".repeat(8_388_609)},
        })
        .to_string();
        assert!(rejected(Some(Message::Text(oversized.into()))).contains("frame exceeded limit"));
        assert!(rejected(None).contains("stopped"));
    }

    #[test]
    fn resumed_thread_projects_ordered_history_items() {
        let thread = parse_thread(&serde_json::json!({
            "id": "thread-1",
            "cwd": "/workspace",
            "turns": [{
                "id": "turn-1",
                "status": "completed",
                "items": [
                    {"id":"user-1","type":"userMessage","content":[{"type":"text","text":"hello"}]},
                    {"id":"agent-1","type":"agentMessage","text":"hi"},
                    {"id":"command-1","type":"commandExecution","command":"cargo test","aggregatedOutput":"ok"}
                ]
            }]
        }))
        .expect("thread");
        assert_eq!(thread.turns.len(), 1);
        assert_eq!(thread.turns[0].items[0].text, "hello");
        assert_eq!(thread.turns[0].items[1].text, "hi");
        assert_eq!(thread.turns[0].items[2].text, "$ cargo test\nok");
    }

    #[test]
    fn thread_recency_prefers_recency_at_and_falls_back_to_updated_at() {
        let preferred = parse_thread(&serde_json::json!({
            "id": "preferred", "recencyAt": 30, "updatedAt": 20
        }))
        .expect("preferred thread");
        let fallback = parse_thread(&serde_json::json!({
            "id": "fallback", "updatedAt": 10
        }))
        .expect("fallback thread");
        let missing =
            parse_thread(&serde_json::json!({"id": "missing"})).expect("thread without recency");

        assert_eq!(preferred.last_used_at, Some(30));
        assert_eq!(fallback.last_used_at, Some(10));
        assert_eq!(missing.last_used_at, None);
    }

    #[test]
    fn command_action_parser_preserves_supported_actions_and_unknown_input() {
        assert_eq!(
            parse_command_action(&json!({
                "type": "read",
                "name": "README.md",
                "path": "/workspace/README.md"
            })),
            CommandAction::Read {
                name: "README.md".into(),
                path: "/workspace/README.md".into(),
            }
        );
        assert_eq!(
            parse_command_action(&json!({"type":"listFiles","path":"/workspace"})),
            CommandAction::ListFiles {
                path: Some("/workspace".into()),
            }
        );
        assert_eq!(
            parse_command_action(&json!({"type":"search","query":"TODO"})),
            CommandAction::Search {
                query: Some("TODO".into()),
                path: None,
            }
        );
        assert_eq!(
            parse_command_action(&json!({"type":"futureAction"})),
            CommandAction::Unknown
        );
    }

    #[test]
    fn remote_control_status_parser_accepts_the_schema_and_rejects_unknown_states() {
        let parsed = parse_remote_control_status(json!({
            "status": "connected",
            "serverName": "workstation",
            "installationId": "install-1",
            "environmentId": "environment-1"
        }))
        .expect("valid status");
        assert_eq!(
            parsed.status,
            crate::RemoteControlConnectionStatus::Connected
        );
        assert_eq!(parsed.environment_id.as_deref(), Some("environment-1"));

        assert!(
            parse_remote_control_status(json!({
                "status": "surprising",
                "serverName": "workstation",
                "installationId": "install-1"
            }))
            .is_err()
        );
    }

    #[test]
    fn remote_pairing_parser_bounds_secret_bearing_fields() {
        let challenge = parse_remote_pairing(json!({
            "environmentId": "environment-1",
            "expiresAt": 12345,
            "pairingCode": "opaque-qr-payload",
            "manualPairingCode": "1234"
        }))
        .expect("valid challenge");
        assert_eq!(challenge.manual_pairing_code.as_deref(), Some("1234"));

        assert!(
            parse_remote_pairing(json!({
                "environmentId": "environment-1",
                "expiresAt": 12345,
                "pairingCode": "x".repeat(8193)
            }))
            .is_err()
        );
    }

    #[test]
    fn remote_client_parser_preserves_device_metadata_and_bounds_strings() {
        let client = parse_remote_client(&json!({
            "clientId": "phone-1",
            "displayName": "Phone",
            "platform": "ios",
            "lastSeenAt": 123
        }))
        .expect("valid client");
        assert_eq!(client.client_id, "phone-1");
        assert_eq!(client.platform.as_deref(), Some("ios"));
        assert_eq!(client.last_seen_at, Some(123));

        assert!(
            parse_remote_client(&json!({
                "clientId": "phone-1",
                "displayName": "x".repeat(513)
            }))
            .is_err()
        );
    }

    proptest! {
        #[test]
        fn arbitrary_json_command_actions_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..16384)) {
            if let Ok(text) = std::str::from_utf8(&bytes)
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(text)
            {
                let _ = parse_command_action(&value);
            }
        }
    }
}
