//! Live Linux acceptance for the Spec 0230 emergency-control boundary.
//!
//! This launches the production compositor and MCP server in an isolated nested
//! session. The private test-control protocol labels emergency input as either
//! synthetic or physical-fixture input. The latter exercises production source
//! classification but is not evidence from a physical keyboard.

use nickel_session_protocol::{
    ClientEnvelope, Command, InputState, Query, RemoteControlEffectiveState, RemoteLeaseTransition,
    Request, ServerEnvelope, ServerMessage, TestEmergencyControlSide, TestEmergencyControlSource,
    TestInput, decode, encode,
};
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::unix::{fs::PermissionsExt, net::UnixDatagram},
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, ExitCode, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const DEADLINE: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(100);
const SESSION_RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const MCP_VERSION: &str = "2025-06-18";
const EGL_VENDOR_FILENAMES: &str = "__EGL_VENDOR_LIBRARY_FILENAMES";
const MESA_EGL_VENDOR_MANIFEST: &str = "/usr/share/glvnd/egl_vendor.d/50_mesa.json";
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

fn main() -> ExitCode {
    match run() {
        Ok(Outcome::Passed) => ExitCode::SUCCESS,
        Ok(Outcome::Skipped(reason)) => {
            println!("SKIP: {reason}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("FAIL: {error}");
            ExitCode::FAILURE
        }
    }
}

enum Outcome {
    Passed,
    Skipped(String),
}

fn run() -> Result<Outcome, String> {
    let Some(display) = host_display() else {
        return Ok(Outcome::Skipped(
            "no reachable host Wayland or X11 display for a nested compositor".into(),
        ));
    };
    let executable = env::current_exe().map_err(|error| error.to_string())?;
    let directory = executable
        .parent()
        .ok_or("acceptance harness has no parent directory")?;
    let nickel = sibling(directory, "nickel")?;
    let runtime = RuntimeDirectory::create()?;
    let capability_file = runtime.path().join("shell-environment");
    let address = reserve_loopback_address()?;

    let mut command = ProcessCommand::new(nickel);
    command
        .args(["--backend", "winit", "--test-control"])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("XDG_CONFIG_HOME", runtime.path().join("config"))
        .env("XDG_STATE_HOME", runtime.path().join("state"))
        .env("XDG_DATA_HOME", runtime.path().join("data"))
        .env("NICKEL_TEST_CONTROL_ENV_FILE", &capability_file)
        .env("NICKEL_MCP_LISTEN_ADDR", address.to_string())
        .env("NICKEL_NESTED_SIZE", "960x640")
        .env("NICKEL_DISABLE_XWAYLAND", "1")
        .stdin(Stdio::null())
        .stderr(Stdio::piped());
    configure_software_renderer(&mut command);
    match display {
        HostDisplay::Wayland(path) => {
            command
                .env("WINIT_UNIX_BACKEND", "wayland")
                .env("WAYLAND_DISPLAY", path);
        }
        HostDisplay::X11(display) => {
            command
                .env("WINIT_UNIX_BACKEND", "x11")
                .env("DISPLAY", display);
        }
    }
    let compositor = command
        .spawn()
        .map_err(|error| format!("could not start nested compositor: {error}"))?;
    let mut session = SessionProcess::new(compositor, runtime.path().to_path_buf());

    let result = exercise(&mut session, &capability_file, address);
    let unavailable = if !capability_file.exists() {
        session.unavailable_environment_reason()
    } else {
        None
    };
    session.shutdown();
    if let (Err(_), Some(reason)) = (&result, unavailable) {
        return Ok(Outcome::Skipped(reason));
    }
    result?;
    println!(
        "PASS: a production full-session lease survived synthetic dual-Control input and was revoked by the explicitly attributed physical fixture; no physical keyboard was exercised"
    );
    Ok(Outcome::Passed)
}

fn configure_software_renderer(command: &mut ProcessCommand) {
    let mesa_manifest = Path::new(MESA_EGL_VENDOR_MANIFEST);
    if mesa_manifest.is_file() {
        // GLVND can select an installed hardware vendor that cannot initialize
        // against an isolated Xvfb or nested Wayland display. Pin the acceptance
        // child to Mesa so LIBGL_ALWAYS_SOFTWARE reliably selects llvmpipe.
        command
            .env(EGL_VENDOR_FILENAMES, mesa_manifest)
            .env("LIBGL_ALWAYS_SOFTWARE", "1");
    }
}

fn exercise(
    session: &mut SessionProcess,
    capability_file: &Path,
    address: SocketAddr,
) -> Result<(), String> {
    let environment = wait_for_environment(session, capability_file, Instant::now() + DEADLINE)?;
    wait_for_readiness(session, &environment, Instant::now() + DEADLINE)?;

    let initial = remote_snapshot(&environment)?;
    if initial.effective != RemoteControlEffectiveState::Enabled
        || initial.endpoint != format!("http://{address}/mcp")
    {
        return Err(format!(
            "isolated MCP listener is not ready at the requested endpoint: {initial:?}"
        ));
    }

    let identity = connect_identity(address, "Spec 0230 native acceptance")?;
    let watch = ConnectionWatch::start(address, &identity)?;
    let request_result = mcp_call(
        address,
        &identity,
        "request_control_lease",
        json!({
            "scope": {"kind": "full_session"},
            "duration_seconds": 1200,
            "allow_resumption": false,
            "full_debug": false
        }),
    )?;
    require_tool_success("request_control_lease", &request_result)?;
    if !request_result
        .to_string()
        .contains("pending_local_approval")
    {
        return Err(format!(
            "lease request did not reach local approval: {request_result}"
        ));
    }

    let deadline = Instant::now() + DEADLINE;
    let pending = loop {
        let snapshot = remote_snapshot(&environment)?;
        if let Some(pending) = snapshot.pending_leases.first().cloned() {
            break pending;
        }
        if Instant::now() >= deadline {
            return Err("lease request did not appear in trusted local state".into());
        }
        thread::sleep(POLL);
    };
    let approved = session_message(
        &environment,
        Request::Command(Command::DecideRemoteLease {
            pending_generation: pending.pending_generation,
            client_id: pending.client_id,
            request: pending.request,
            allow: true,
        }),
    )?;
    let ServerMessage::RemoteControl(approved) = approved else {
        return Err(format!("local lease approval returned {approved:?}"));
    };
    let lease = approved
        .active_leases
        .first()
        .ok_or("local approval did not create an active lease")?;
    let lease_id = lease.lease_id;

    let before = mcp_call(
        address,
        &identity,
        "list_surfaces",
        json!({"lease_id": lease_id}),
    )?;
    require_tool_success("list_surfaces before emergency input", &before)?;
    require_no_trusted_indicator_data(&before)?;

    emergency_chord(&environment, TestEmergencyControlSource::Synthetic)?;
    let after_synthetic = remote_snapshot(&environment)?;
    if after_synthetic.effective != RemoteControlEffectiveState::Enabled
        || after_synthetic.active_leases.len() != 1
        || after_synthetic.active_leases[0].lease_id != lease_id
    {
        return Err(format!(
            "synthetic dual-Control changed production authority: {after_synthetic:?}"
        ));
    }
    let still_authorized = mcp_call(
        address,
        &identity,
        "list_surfaces",
        json!({"lease_id": lease_id}),
    )?;
    require_tool_success(
        "list_surfaces after synthetic dual-Control",
        &still_authorized,
    )?;
    require_no_trusted_indicator_data(&still_authorized)?;

    emergency_chord(&environment, TestEmergencyControlSource::PhysicalFixture)?;
    let stopped = remote_snapshot(&environment)?;
    if stopped.effective != RemoteControlEffectiveState::Disabled
        || stopped.requested_enabled
        || !stopped.active_leases.is_empty()
        || !stopped.pending_leases.is_empty()
        || !stopped.lease_audit.iter().any(|event| {
            event.lease_id == lease_id && event.transition == RemoteLeaseTransition::Revoked
        })
    {
        return Err(format!(
            "physical-fixture dual-Control did not synchronously revoke authority: {stopped:?}"
        ));
    }
    if TcpStream::connect_timeout(&address, Duration::from_millis(500)).is_ok() {
        return Err("MCP listener remained reachable after emergency stop".into());
    }
    watch.finish()?;
    Ok(())
}

fn emergency_chord(
    environment: &SessionEnvironment,
    source: TestEmergencyControlSource,
) -> Result<(), String> {
    for (side, state) in [
        (TestEmergencyControlSide::Left, InputState::Pressed),
        (TestEmergencyControlSide::Right, InputState::Pressed),
        (TestEmergencyControlSide::Left, InputState::Released),
        (TestEmergencyControlSide::Right, InputState::Released),
    ] {
        let message = session_message(
            environment,
            Request::Command(Command::TestInput {
                input: TestInput::EmergencyControl {
                    source,
                    side,
                    state,
                },
            }),
        )?;
        if message != ServerMessage::Ack {
            return Err(format!("emergency fixture input returned {message:?}"));
        }
    }
    Ok(())
}

fn require_tool_success(operation: &str, response: &Value) -> Result<(), String> {
    if response.get("error").is_some()
        || response
            .pointer("/result/isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return Err(format!("{operation} failed: {response}"));
    }
    Ok(())
}

fn require_no_trusted_indicator_data(response: &Value) -> Result<(), String> {
    let serialized = response.to_string().to_ascii_lowercase();
    for forbidden in [
        "spec 0230 native acceptance",
        "remote-control-stop",
        "trustedcontrol",
        "nickel remote ai control",
    ] {
        if serialized.contains(forbidden) {
            return Err(format!(
                "remote surface inventory exposed trusted indicator data {forbidden:?}: {response}"
            ));
        }
    }
    Ok(())
}

#[derive(Debug)]
struct Identity {
    client_id: String,
    token: String,
}

fn connect_identity(address: SocketAddr, label: &str) -> Result<Identity, String> {
    let response = http_json(
        address,
        "/clients/connect",
        &json!({"label": label}).to_string(),
        &[],
    )?;
    Ok(Identity {
        client_id: response
            .get("client_id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("identity response omitted client_id: {response}"))?
            .to_owned(),
        token: response
            .get("token")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("identity response omitted token: {response}"))?
            .to_owned(),
    })
}

fn mcp_call(
    address: SocketAddr,
    identity: &Identity,
    name: &str,
    arguments: Value,
) -> Result<Value, String> {
    let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": arguments,
            "_meta": client_metadata(id)
        }
    })
    .to_string();
    let authorization = format!("Bearer {}", identity.token);
    http_json(
        address,
        "/mcp",
        &body,
        &[
            ("Accept", "application/json, text/event-stream"),
            ("MCP-Protocol-Version", MCP_VERSION),
            ("Mcp-Method", "tools/call"),
            ("Mcp-Name", name),
            ("X-Nickel-Client", &identity.client_id),
            ("Authorization", &authorization),
        ],
    )
}

fn client_metadata(progress: u64) -> Value {
    json!({
        "progressToken": progress,
        "io.modelcontextprotocol/protocolVersion": MCP_VERSION,
        "io.modelcontextprotocol/clientInfo": {
            "name": "nickel-linux-remote-control-acceptance",
            "version": env!("CARGO_PKG_VERSION")
        },
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

fn http_json(
    address: SocketAddr,
    path: &str,
    body: &str,
    headers: &[(&str, &str)],
) -> Result<Value, String> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .map_err(|error| format!("could not connect to MCP listener: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n").map_err(|error| error.to_string())?;
    }
    write!(stream, "\r\n{body}").map_err(|error| error.to_string())?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| format!("could not read MCP response: {error}"))?;
    if !response.starts_with("HTTP/1.1 200") {
        return Err(format!("HTTP request failed: {response}"));
    }
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .ok_or_else(|| format!("HTTP response omitted a body: {response}"))?;
    serde_json::from_str(body).map_err(|error| format!("invalid JSON response ({error}): {body}"))
}

struct ConnectionWatch {
    stop: mpsc::SyncSender<()>,
    worker: thread::JoinHandle<Result<(), String>>,
}

impl ConnectionWatch {
    fn start(address: SocketAddr, identity: &Identity) -> Result<Self, String> {
        let client_id = identity.client_id.clone();
        let token = identity.token.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (stop, stopped) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            open_connection_watch(address, &client_id, &token, ready_tx, stopped)
        });
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(Self { stop, worker }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                let _ = stop.send(());
                let _ = worker.join();
                Err("MCP connection watch did not become ready".into())
            }
        }
    }

    fn finish(self) -> Result<(), String> {
        let _ = self.stop.send(());
        self.worker
            .join()
            .map_err(|_| "connection watch thread panicked".to_owned())?
    }
}

fn open_connection_watch(
    address: SocketAddr,
    client_id: &str,
    token: &str,
    ready: mpsc::SyncSender<Result<(), String>>,
    stop: mpsc::Receiver<()>,
) -> Result<(), String> {
    let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "client_connection",
            "arguments": {"action": "watch"},
            "_meta": client_metadata(id)
        }
    })
    .to_string();
    let stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .map_err(|error| format!("could not open MCP connection watch: {error}"));
    let mut stream = match stream {
        Ok(stream) => stream,
        Err(error) => {
            let _ = ready.send(Err(error.clone()));
            return Err(error);
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    write!(
        stream,
        "POST /mcp HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: {MCP_VERSION}\r\nMcp-Method: tools/call\r\nMcp-Name: client_connection\r\nX-Nickel-Client: {client_id}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    let mut received = Vec::new();
    loop {
        let mut chunk = [0; 4096];
        let count = stream
            .read(&mut chunk)
            .map_err(|error| format!("connection watch did not become ready: {error}"))?;
        if count == 0 {
            return Err(format!(
                "connection watch ended before readiness: {}",
                String::from_utf8_lossy(&received)
            ));
        }
        received.extend_from_slice(&chunk[..count]);
        if String::from_utf8_lossy(&received).contains("Connection watch active") {
            break;
        }
        if received.len() >= 16 * 1024 {
            return Err("connection watch readiness exceeded its response bound".into());
        }
    }
    let _ = ready.send(Ok(()));
    let _ = stop.recv_timeout(Duration::from_secs(60));
    Ok(())
}

#[derive(Clone)]
struct SessionEnvironment {
    control: PathBuf,
    token: String,
    runtime: PathBuf,
}

fn wait_for_environment(
    session: &mut SessionProcess,
    capability_file: &Path,
    deadline: Instant,
) -> Result<SessionEnvironment, String> {
    loop {
        if let Some(status) = session.try_wait()? {
            return Err(format!(
                "nested compositor exited before readiness: {status}"
            ));
        }
        if let Ok(environment) = read_environment(capability_file) {
            return Ok(environment);
        }
        if Instant::now() >= deadline {
            return Err("nested compositor did not publish test control within 30 seconds".into());
        }
        thread::sleep(POLL);
    }
}

fn wait_for_readiness(
    session: &mut SessionProcess,
    environment: &SessionEnvironment,
    deadline: Instant,
) -> Result<(), String> {
    poll_readiness(
        deadline,
        SESSION_RESPONSE_TIMEOUT,
        POLL,
        |read_timeout| {
            session_message_with_timeout(
                environment,
                Request::Query(Query::ShellReadiness),
                read_timeout,
            )
        },
        || {
            if let Some(status) = session.try_wait()? {
                Err(format!(
                    "nested compositor exited while awaiting readiness: {status}"
                ))
            } else {
                Ok(())
            }
        },
    )
}

fn poll_readiness(
    deadline: Instant,
    response_timeout: Duration,
    poll_interval: Duration,
    mut query: impl FnMut(Duration) -> Result<ServerMessage, SessionMessageError>,
    mut ensure_running: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("shell readiness did not become ready within 30 seconds".into());
        }
        match query(response_timeout.min(remaining)) {
            Ok(ServerMessage::ShellReadiness(readiness)) if readiness.ready => return Ok(()),
            Ok(ServerMessage::ShellReadiness(_)) => {}
            Ok(message) => return Err(format!("readiness query returned {message:?}")),
            Err(SessionMessageError::RetryableReceive(_)) if Instant::now() < deadline => {}
            Err(SessionMessageError::RetryableReceive(error)) => {
                return Err(format!(
                    "readiness did not respond within 30 seconds: {error}"
                ));
            }
            Err(SessionMessageError::Fatal(error)) => {
                return Err(format!("readiness query failed: {error}"));
            }
        }
        ensure_running()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let sleep_for = poll_interval.min(remaining);
        if !sleep_for.is_zero() {
            thread::sleep(sleep_for);
        }
    }
}

fn remote_snapshot(
    environment: &SessionEnvironment,
) -> Result<nickel_session_protocol::RemoteControlSnapshot, String> {
    match session_message(environment, Request::Query(Query::RemoteControl))? {
        ServerMessage::RemoteControl(snapshot) => Ok(snapshot),
        message => Err(format!("remote-control query returned {message:?}")),
    }
}

fn session_message(
    environment: &SessionEnvironment,
    request: Request,
) -> Result<ServerMessage, String> {
    session_message_with_timeout(environment, request, SESSION_RESPONSE_TIMEOUT)
        .map_err(SessionMessageError::into_string)
}

enum SessionMessageError {
    RetryableReceive(String),
    Fatal(String),
}

impl SessionMessageError {
    fn into_string(self) -> String {
        match self {
            Self::RetryableReceive(error) | Self::Fatal(error) => error,
        }
    }
}

fn session_message_with_timeout(
    environment: &SessionEnvironment,
    request: Request,
    read_timeout: Duration,
) -> Result<ServerMessage, SessionMessageError> {
    let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let reply = environment
        .runtime
        .join(format!("nickel-0230-{}-{id}.sock", std::process::id()));
    let socket = UnixDatagram::bind(&reply).map_err(|error| {
        SessionMessageError::Fatal(format!("could not bind private test reply socket: {error}"))
    })?;
    let _reply = ReplyPath(reply);
    socket
        .set_read_timeout(Some(read_timeout))
        .map_err(|error| SessionMessageError::Fatal(error.to_string()))?;
    socket
        .send_to(
            &encode(&ClientEnvelope {
                token: environment.token.clone(),
                request_id: id,
                request,
            })
            .map_err(|error| SessionMessageError::Fatal(error.to_string()))?,
            &environment.control,
        )
        .map_err(|error| {
            SessionMessageError::Fatal(format!("could not send private test request: {error}"))
        })?;
    let mut response = vec![0; nickel_session_protocol::MAX_FRAME_BYTES];
    let length = socket.recv(&mut response).map_err(classify_receive_error)?;
    let envelope = decode::<ServerEnvelope>(&response[..length])
        .map_err(|error| SessionMessageError::Fatal(error.to_string()))?;
    if envelope.request_id != id {
        return Err(SessionMessageError::Fatal(
            "private test response correlation mismatch".into(),
        ));
    }
    match envelope.message {
        ServerMessage::Error { message, .. } => Err(SessionMessageError::Fatal(message)),
        message => Ok(message),
    }
}

fn classify_receive_error(error: std::io::Error) -> SessionMessageError {
    let message = format!("could not receive private test response: {error}");
    if matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ) {
        SessionMessageError::RetryableReceive(message)
    } else {
        SessionMessageError::Fatal(message)
    }
}

fn read_environment(path: &Path) -> Result<SessionEnvironment, String> {
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let value = |name: &str| {
        contents
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .map(str::to_owned)
            .ok_or_else(|| format!("shell capability environment omitted {name}"))
    };
    Ok(SessionEnvironment {
        control: PathBuf::from(value("NICKEL_SESSION_CONTROL")?),
        token: value("NICKEL_SESSION_TOKEN")?,
        runtime: PathBuf::from(value("XDG_RUNTIME_DIR")?),
    })
}

struct ReplyPath(PathBuf);

impl Drop for ReplyPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct SessionProcess {
    child: Child,
    runtime: PathBuf,
}

impl SessionProcess {
    fn new(child: Child, runtime: PathBuf) -> Self {
        Self { child, runtime }
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, String> {
        self.child.try_wait().map_err(|error| error.to_string())
    }

    fn unavailable_environment_reason(&mut self) -> Option<String> {
        self.child.try_wait().ok().flatten()?;
        let mut stderr = String::new();
        self.child.stderr.take()?.read_to_string(&mut stderr).ok()?;
        [
            "Failed to connect to Wayland display",
            "Failed to connect to X11 display",
        ]
        .into_iter()
        .find(|message| stderr.contains(message))
        .map(|message| format!("nested compositor prerequisite unavailable: {message}"))
    }

    fn shutdown(&mut self) {
        if self.child.try_wait().ok().flatten().is_none()
            && let Ok(environment) = read_environment(&self.runtime.join("shell-environment"))
        {
            let _ = session_message(&environment, Request::Command(Command::LogOut));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(POLL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for SessionProcess {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct RuntimeDirectory(PathBuf);

impl RuntimeDirectory {
    fn create() -> Result<Self, String> {
        let path = env::temp_dir().join(format!(
            "nickel-linux-control-acceptance-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir(&path).map_err(|error| error.to_string())?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for RuntimeDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

enum HostDisplay {
    Wayland(PathBuf),
    X11(String),
}

fn host_display() -> Option<HostDisplay> {
    if let Some(display) = env::var_os("WAYLAND_DISPLAY") {
        let path = PathBuf::from(display);
        let path = if path.is_absolute() {
            path
        } else {
            PathBuf::from(env::var_os("XDG_RUNTIME_DIR")?).join(path)
        };
        if path.exists() {
            return Some(HostDisplay::Wayland(path));
        }
    }
    let display = env::var("DISPLAY").ok()?;
    let local = display.strip_prefix(':')?;
    let number = local.split_once('.').map_or(local, |(number, _)| number);
    Path::new(&format!("/tmp/.X11-unix/X{number}"))
        .exists()
        .then_some(HostDisplay::X11(display))
}

fn reserve_loopback_address() -> Result<SocketAddr, String> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| format!("could not reserve an isolated MCP port: {error}"))?;
    listener.local_addr().map_err(|error| error.to_string())
}

fn sibling(directory: &Path, name: &str) -> Result<PathBuf, String> {
    let path = directory.join(name);
    path.is_file()
        .then_some(path)
        .ok_or_else(|| format!("missing {}; build nickel and this harness together", name))
}

#[cfg(test)]
mod tests {
    use super::{SessionMessageError, classify_receive_error, poll_readiness};
    use nickel_session_protocol::{ServerMessage, ShellReadinessSnapshot};
    use std::{
        io,
        time::{Duration, Instant},
    };

    #[test]
    fn readiness_retries_would_block_before_deadline() {
        let mut attempts = 0;
        poll_readiness(
            Instant::now() + Duration::from_secs(1),
            Duration::from_millis(10),
            Duration::ZERO,
            |_| {
                attempts += 1;
                if attempts == 1 {
                    Err(classify_receive_error(io::Error::from(
                        io::ErrorKind::WouldBlock,
                    )))
                } else {
                    Ok(ready_message())
                }
            },
            || Ok(()),
        )
        .expect("a transient EAGAIN should be retried");
        assert_eq!(attempts, 2);
    }

    #[test]
    fn readiness_does_not_retry_fatal_protocol_errors() {
        let mut attempts = 0;
        let error = poll_readiness(
            Instant::now() + Duration::from_secs(1),
            Duration::from_millis(10),
            Duration::ZERO,
            |_| {
                attempts += 1;
                Err(SessionMessageError::Fatal("malformed reply".into()))
            },
            || Ok(()),
        )
        .expect_err("a fatal protocol error should fail immediately");
        assert_eq!(attempts, 1);
        assert_eq!(error, "readiness query failed: malformed reply");
    }

    fn ready_message() -> ServerMessage {
        ServerMessage::ShellReadiness(ShellReadinessSnapshot {
            expected_shell_pid: None,
            authenticated_shell_pid: None,
            outputs: 1,
            desktops: 1,
            panels: 1,
            locks: 1,
            launchers: 1,
            required_singletons_ready: true,
            output_roles_ready: true,
            reserved_ordinary_windows: 0,
            ready: true,
        })
    }
}
