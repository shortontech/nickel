use std::{
    collections::HashMap,
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
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

use crate::protocol::*;

const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
const EVENT_BACKLOG: usize = 1024;
const OUTBOUND_BACKLOG: usize = 256;

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
    subscribers: Mutex<Vec<mpsc::SyncSender<CodexEvent>>>,
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
    WebSocket(mpsc::SyncSender<RemoteWrite>),
}

enum RemoteWrite {
    Text(String),
    Close,
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

    pub fn spawn_with_timeout(
        executable: &Path,
        cwd: &Path,
        request_timeout: Duration,
    ) -> Result<Self, CodexError> {
        let mut child = Command::new(executable)
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
        let (socket, _) = connect(request).map_err(|error| {
            CodexError::Unavailable(format!("remote app-server connection failed: {error}"))
        })?;
        let (writer, outbound) = mpsc::sync_channel(OUTBOUND_BACKLOG);
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
        client.start_websocket(socket, outbound);
        client.initialize()?;
        Ok(client)
    }

    fn initialize(&self) -> Result<(), CodexError> {
        self.request("initialize", json!({
            "clientInfo": {"name": "nickel", "title": "Nickel", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": false}
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
                match reader.read_line(&mut line) {
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
        outbound: mpsc::Receiver<RemoteWrite>,
    ) {
        set_socket_timeout(socket.get_mut(), Duration::from_millis(50));
        let inner = Arc::downgrade(&self.inner);
        thread::spawn(move || {
            loop {
                while let Ok(write) = outbound.try_recv() {
                    let result = match write {
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
        self.inner.pending.lock().unwrap().insert(key.clone(), tx);
        if let Err(error) = self.write(&json!({"id": id, "method": method, "params": params})) {
            self.inner.pending.lock().unwrap().remove(&key);
            return Err(error);
        }
        rx.recv_timeout(self.inner.request_timeout)
            .map_err(|_| CodexError::Timeout(format!("{method} timed out")))?
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), CodexError> {
        self.write(&json!({"method": method, "params": params}))
    }

    fn write(&self, value: &Value) -> Result<(), CodexError> {
        let text = serde_json::to_string(value)?;
        if text.len() > MAX_FRAME_BYTES {
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
        self.inner.outstanding.lock().unwrap().insert(
            id.clone(),
            PendingInteraction {
                method: method.into(),
                raw_id: value["id"].clone(),
                question_ids: question_ids.clone(),
            },
        );
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
            "account/updated" | "account/login/completed" | "account/rateLimits/updated" => {
                EventKind::AccountUpdated
            }
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
        self.publish(event);
    }

    fn project(&self, event: &EventKind) {
        let mut projection = self.inner.projection.lock().unwrap();
        let mut inconsistency = None;
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
                projection.active_turn = None;
                let thread = projection.threads.entry(thread_id.clone()).or_default();
                thread.active_turn = None;
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
            EventKind::AgentMessageDelta { item_id, delta }
            | EventKind::CommandOutputDelta { item_id, delta }
            | EventKind::FileChangeDelta { item_id, delta }
            | EventKind::PlanDelta { item_id, delta }
            | EventKind::ReasoningDelta { item_id, delta } => {
                match projection.items.get_mut(item_id) {
                    Some(item) if !item.completed => item.text.push_str(delta),
                    Some(_) => {
                        inconsistency = Some(format!("delta for terminal item {item_id}"));
                    }
                    None => {
                        inconsistency = Some(format!("delta for unknown item {item_id}"));
                        projection
                            .items
                            .entry(item_id.clone())
                            .or_default()
                            .text
                            .push_str(delta);
                    }
                }
            }
            EventKind::Error { message } => projection.terminal_error = Some(message.clone()),
            _ => {}
        }
        drop(projection);
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
            match subscriber.try_send(event.clone()) {
                Ok(()) => true,
                Err(mpsc::TrySendError::Full(_)) => {
                    self.inner.dropped_events.fetch_add(1, Ordering::Relaxed);
                    true
                }
                Err(mpsc::TrySendError::Disconnected(_)) => false,
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
    fn list_threads(&self, page: ThreadPage) -> Result<ThreadPageResult, CodexError> {
        let value = self.request(
            "thread/list",
            json!({"cursor": page.cursor, "limit": page.limit}),
        )?;
        let threads = value
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(parse_thread)
            .collect();
        Ok(ThreadPageResult {
            threads,
            next_cursor: value
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(Into::into),
        })
    }
    fn start_thread(&self, request: StartThread) -> Result<Thread, CodexError> {
        let value = self.request(
            "thread/start",
            json!({"cwd": request.cwd, "model": request.model}),
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
        let value = self.request("turn/start", json!({"threadId": request.thread_id.0, "input": [{"type": "text", "text": request.text}]}))?;
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
    fn subscribe(&self) -> mpsc::Receiver<CodexEvent> {
        let (tx, rx) = mpsc::sync_channel(EVENT_BACKLOG);
        self.inner.subscribers.lock().unwrap().push(tx);
        rx
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
    Some(ThreadHistoryItem {
        id: value.get("id")?.as_str()?.into(),
        item_type: item_type.into(),
        text: history_item_text(item_type, value),
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

    use super::*;

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
            let mut saw_interrupt = false;
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
                    "model/list" => json!({"data":[],"nextCursor":null}),
                    "thread/list" => json!({"data":[],"nextCursor":null}),
                    "thread/start" => {
                        saw_remote_cwd = value["params"]["cwd"] == "/srv/code/nickel";
                        json!({"thread":{"id":"remote-thread","cwd":"/srv/code/nickel"}})
                    }
                    "turn/start" => json!({"turn":{"id":"remote-turn","status":"inProgress"}}),
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
                }
            }
            (saw_remote_cwd, saw_approval, saw_interrupt)
        });

        let client = CodexClient::connect_remote_with_timeout(
            &endpoint,
            Some("fixture-secret"),
            Duration::from_secs(2),
        )
        .unwrap();
        assert!(client.account().unwrap().authenticated);
        assert!(client.models().unwrap().is_empty());
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
            })
            .unwrap();
        let turn = client
            .start_turn(StartTurn {
                thread_id: thread.id,
                text: "hello remotely".into(),
            })
            .unwrap();
        assert_eq!(turn.id.0, "remote-turn");
        let request_id = loop {
            let event = events.recv_timeout(Duration::from_secs(2)).unwrap();
            if let EventKind::ApprovalRequested { request_id, .. } = event.kind {
                break request_id;
            }
        };
        assert_eq!(client.projection().items["agent-1"].text, "remote response");
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
        assert_eq!(server.join().unwrap(), (true, true, true));
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
                    socket.send(message).unwrap();
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

        assert!(rejected(Some(Message::Binary(vec![1, 2, 3].into()))).contains("stopped"));
        assert!(rejected(Some(Message::Text("not json".into()))).contains("stopped"));
        assert!(
            rejected(Some(Message::Text("x".repeat(MAX_FRAME_BYTES + 1).into())))
                .contains("stopped")
        );
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

    proptest! {
        #[test]
        fn arbitrary_bounded_frames_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..16384)) {
            if let Ok(text) = std::str::from_utf8(&bytes) {
                let _ = serde_json::from_str::<serde_json::Value>(text);
            }
        }

        #[test]
        fn valid_json_round_trips_across_every_fragment_boundary(text in ".{0,256}") {
            let encoded = serde_json::to_vec(&serde_json::json!({"method":"fixture/event","params":{"delta":text}})).unwrap();
            for split in 0..=encoded.len() {
                let mut joined = encoded[..split].to_vec();
                joined.extend_from_slice(&encoded[split..]);
                let decoded: serde_json::Value = serde_json::from_slice(&joined).unwrap();
                prop_assert_eq!(decoded["params"]["delta"].as_str(), Some(text.as_str()));
            }
        }
    }
}
