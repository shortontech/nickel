use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use serde_json::{Value, json};

use crate::protocol::*;

const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
const EVENT_BACKLOG: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Starting,
    Ready,
    Stopping,
    Stopped,
    Failed,
}

struct Inner {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
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

struct PendingInteraction {
    method: String,
    raw_id: Value,
    question_ids: Vec<String>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut() {
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
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
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
        client.request("initialize", json!({
            "clientInfo": {"name": "nickel", "title": "Nickel", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": false}
        }))?;
        client.notify("initialized", json!({}))?;
        *client.inner.state.lock().unwrap() = ConnectionState::Ready;
        client.publish(EventKind::Connection {
            state: "ready".into(),
        });
        Ok(client)
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
        let mut stdin = self.inner.stdin.lock().unwrap();
        serde_json::to_writer(&mut *stdin, value)?;
        stdin.write_all(b"\n")?;
        stdin.flush()?;
        Ok(())
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
        let mut child = self.inner.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
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
    use proptest::prelude::*;

    use super::parse_thread;

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
