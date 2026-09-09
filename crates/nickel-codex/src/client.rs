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
        let mut child = command(executable);
        if let Some(codex_home) = codex_home {
            if !codex_home.is_absolute() {
                return Err(CodexError::Unavailable(
                    "isolated CODEX_HOME must be an absolute path".into(),
                ));
            }
            child.env("CODEX_HOME", codex_home);
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
        if let Err(error) = self.write(&json!({"id": id, "method": method, "params": params})) {
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
        let question_ids: Vec<String> = value["params"]
            .get("questions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|question| {
                question
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .collect();
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
                    approval_type: method.into(),
                    summary: params
                        .get("reason")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                });
            }
            "item/tool/requestUserInput" => {
                self.publish(EventKind::UserInputRequested {
                    request_id,
                    question_ids,
                });
            }
            _ => self.publish(EventKind::UnsupportedEvent {
                method: method.into(),
            }),
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
                    .filter(|item| item.get("source").and_then(Value::as_str) == Some("userShell"))
                    .and_then(|item| item.get("command"))
                    .and_then(Value::as_str)
                    .map(|command| format!("!{command}\n"))
                    .unwrap_or_default(),
            },
            "item/completed" => EventKind::ItemCompleted {
                item_id: nested("item", "id"),
            },
            "item/agentMessage/delta" => EventKind::AgentMessageDelta {
                item_id: string("itemId"),
                delta: string("delta"),
            },
            "item/commandExecution/outputDelta" => EventKind::CommandOutputDelta {
                item_id: string("itemId"),
                delta: string("delta"),
            },
            "item/fileChange/outputDelta" | "item/fileChange/patchUpdated" => {
                EventKind::FileChangeDelta {
                    item_id: string("itemId"),
                    delta: params
                        .get("delta")
                        .or_else(|| params.get("patch"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                }
            }
            "item/plan/delta" | "turn/plan/updated" => EventKind::PlanDelta {
                item_id: string("itemId"),
                delta: params
                    .get("delta")
                    .or_else(|| params.get("plan"))
                    .map(|value| {
                        value
                            .as_str()
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| value.to_string())
                    })
                    .unwrap_or_default(),
            },
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                EventKind::ReasoningDelta {
                    item_id: string("itemId"),
                    delta: string("delta"),
                }
            }
            "account/updated" | "account/rateLimits/updated" => EventKind::AccountUpdated,
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
            "error" => EventKind::Error {
                message: params
                    .get("error")
                    .and_then(|v| v.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("Codex error")
                    .into(),
            },
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
            EventKind::ItemCompleted { item_id } => {
                if let Some(item) = projection.items.get_mut(item_id) {
                    item.completed = true;
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
        let value = self.request("turn/start", json!({"threadId": request.thread_id.0, "input": input, "model": request.model, "effort": request.reasoning_effort, "approvalPolicy": request.approval_policy}))?;
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
        text: history_item_text(item_type, value),
        command_actions,
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
        "reasoning" => value
            .get("summary")
            .or_else(|| value.get("content"))
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
            .and_then(Value::as_array)
            .map(|changes| format!("{} file change(s)", changes.len()))
            .unwrap_or_default(),
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

    use super::*;

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
                    continue;
                };
                let result = match method {
                    "initialize" => json!({}),
                    "account/read" => json!({"account":{"type":"chatgpt"}}),
                    "model/list" => json!({"data":[{
                        "id":"gpt-fixture","displayName":"GPT Fixture",
                        "defaultReasoningEffort":"medium",
                        "supportedReasoningEfforts":[
                            {"reasoningEffort":"low","description":"Fast"},
                            {"reasoningEffort":"high","description":"Deep"}
                        ]
                    }],"nextCursor":null}),
                    "thread/list" => json!({"data":[],"nextCursor":null}),
                    "thread/start" => {
                        saw_remote_cwd = value["params"]["cwd"] == "/srv/code/nickel"
                            && value["params"]["projectId"] == "remote-project"
                            && value["params"]["config"]["model_reasoning_effort"] == "high";
                        saw_thread_policy = value["params"]["approvalPolicy"] == "on-request";
                        json!({"thread":{"id":"remote-thread","cwd":"/srv/code/nickel"}})
                    }
                    "turn/start" => {
                        assert_eq!(
                            value["params"]["input"][1]["url"].as_str().unwrap().len(),
                            9 * 1024 * 1024
                        );
                        saw_reasoning = value["params"]["effort"] == "high";
                        saw_turn_policy = value["params"]["approvalPolicy"] == "never";
                        json!({"turn":{"id":"remote-turn","status":"inProgress"}})
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
                                "params":{"reason":"fixture approval"}
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
                            "params":{"item":{"id":"shell-1"}}
                        }),
                    ] {
                        socket
                            .send(Message::Text(notification.to_string().into()))
                            .unwrap();
                    }
                }
            }
            (
                saw_remote_cwd,
                saw_approval,
                saw_shell,
                saw_interrupt,
                saw_reasoning,
                saw_thread_policy,
                saw_turn_policy,
            )
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
                approval_policy: ApprovalPolicy::OnRequest,
            })
            .unwrap();
        let turn = client
            .start_turn(StartTurn {
                thread_id: thread.id.clone(),
                text: "hello remotely".into(),
                images: vec![crate::TurnImage {
                    data_url: "a".repeat(9 * 1024 * 1024),
                }],
                model: None,
                reasoning_effort: Some("high".into()),
                approval_policy: ApprovalPolicy::Never,
            })
            .unwrap();
        assert_eq!(turn.id.0, "remote-turn");
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
            if let EventKind::ApprovalRequested { request_id, .. } = event.kind {
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
        client
            .interrupt_turn(ThreadId("remote-thread".into()), turn.id)
            .unwrap();
        client.shutdown();
        assert_eq!(
            server.join().unwrap(),
            (true, true, true, true, true, true, true)
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
