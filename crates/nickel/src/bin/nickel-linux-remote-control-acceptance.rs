//! Live Linux acceptance for the Spec 0230/0231 remote-control boundary.
//!
//! This launches the production compositor and MCP server in an isolated nested
//! session. It exercises the pre-lease tool and diagnostics gates before locally
//! approving debug authority. The private test-control protocol labels emergency
//! input as either synthetic or physical-fixture input. The latter exercises
//! production source classification but is not evidence from a physical keyboard.

use nickel_session_protocol::{
    ClientEnvelope, Command, InputState, Query, RemoteControlEffectiveState, RemoteLeaseTransition,
    RemoteOperationOutcome, Request, ServerEnvelope, ServerMessage, TestEmergencyControlSide,
    TestEmergencyControlSource, TestInput, decode, encode,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
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
const READINESS_DEADLINE: Duration = Duration::from_secs(90);
const POLL: Duration = Duration::from_millis(100);
const MATRIX_PACING: Duration = Duration::from_millis(75);
const SESSION_RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const MCP_VERSION: &str = "2025-06-18";
const EGL_VENDOR_FILENAMES: &str = "__EGL_VENDOR_LIBRARY_FILENAMES";
const MESA_EGL_VENDOR_MANIFEST: &str = "/usr/share/glvnd/egl_vendor.d/50_mesa.json";
const MESA_VBLANK_MODE: &str = "vblank_mode";
const TYPED_CANARY: &str = "native-typed-password-DO-NOT-RETAIN";
const CREDENTIAL_CANARY: &str = "native-credential-DO-NOT-RETAIN";
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
    configure_software_renderer(&mut command, matches!(&display, HostDisplay::Wayland(_)));
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
        "PASS: production pre-lease metrics, complete desktop-tool denial, payload-free diagnostics, and emergency revocation passed; no physical keyboard was exercised"
    );
    Ok(Outcome::Passed)
}

fn configure_software_renderer(command: &mut ProcessCommand, wayland: bool) {
    let mesa_manifest = Path::new(MESA_EGL_VENDOR_MANIFEST);
    if mesa_manifest.is_file() {
        // GLVND can select an installed hardware vendor that cannot initialize
        // against an isolated Xvfb or nested Wayland display. Pin the acceptance
        // child to Mesa so LIBGL_ALWAYS_SOFTWARE reliably selects llvmpipe.
        configure_mesa_software_renderer(command, mesa_manifest, wayland);
    }
}

fn configure_mesa_software_renderer(command: &mut ProcessCommand, manifest: &Path, wayland: bool) {
    command
        .env(EGL_VENDOR_FILENAMES, manifest)
        .env("LIBGL_ALWAYS_SOFTWARE", "1");
    if wayland {
        // Mesa's Wayland software EGL path otherwise retains its default vblank
        // throttle despite Smithay requesting a non-vsynced config. On a host
        // exposing only wl_shm, its second swap can then wait inside Mesa's
        // private Wayland queue and prevent the compositor loop from servicing
        // test control. The X11 acceptance path already presents without this
        // override and deliberately keeps its existing environment.
        command.env(MESA_VBLANK_MODE, "0");
    }
}

fn exercise(
    session: &mut SessionProcess,
    capability_file: &Path,
    address: SocketAddr,
) -> Result<(), String> {
    let environment = wait_for_environment(session, capability_file, Instant::now() + DEADLINE)?;
    wait_for_readiness(session, &environment, Instant::now() + READINESS_DEADLINE)?;

    let initial = remote_snapshot(&environment)?;
    if initial.effective != RemoteControlEffectiveState::Enabled
        || initial.endpoint != format!("http://{address}/mcp")
    {
        return Err(format!(
            "isolated MCP listener is not ready at the requested endpoint: {initial:?}"
        ));
    }

    let initial_metrics = metrics(address)?;
    require_metric(&initial_metrics, "nickel_mcp_active_leases 0")?;
    require_no_canaries(
        "pre-identity metrics",
        &initial_metrics,
        &[TYPED_CANARY, CREDENTIAL_CANARY],
    )?;

    let identity = connect_identity(address, CREDENTIAL_CANARY)?;
    let watch = ConnectionWatch::start(address, &identity)?;
    let audit_baseline = remote_snapshot(&environment)?
        .operation_audit
        .last()
        .map_or(0, |event| event.generation);
    let desktop_tools = desktop_tool_fixtures(address, &identity)?;
    if desktop_tools.len() < 52 {
        return Err("tools/list exposed fewer than the reviewed desktop-tool baseline".into());
    }
    for (name, arguments) in &desktop_tools {
        let response = mcp_call(address, &identity, name, arguments.clone())
            .map_err(|_| format!("pre-lease {name} did not receive an MCP response"))?;
        require_prelease_denial(name, &response)?;
        // The production authenticated admission bucket permits a 32-request
        // burst and refills at 16 requests/second. Pace the complete inventory
        // so this test reaches every lease gate instead of testing HTTP 429.
        thread::sleep(MATRIX_PACING);
    }
    let prelease_metrics = metrics(address)?;
    require_metric(&prelease_metrics, "nickel_mcp_active_leases 0")?;
    for name in desktop_tools.keys() {
        require_metric(
            &prelease_metrics,
            &format!("nickel_mcp_requests_total{{method=\"{name}\",outcome=\"error\"}} 1"),
        )?;
    }
    require_no_canaries(
        "pre-lease metrics",
        &prelease_metrics,
        &[
            TYPED_CANARY,
            CREDENTIAL_CANARY,
            &identity.client_id,
            &identity.token,
        ],
    )?;
    require_prelease_operation_audit(
        &remote_snapshot(&environment)?,
        audit_baseline,
        desktop_tools.keys().map(String::as_str),
    )?;

    let request_result = mcp_call(
        address,
        &identity,
        "request_control_lease",
        json!({
            "scope": {"kind": "full_session"},
            "duration_seconds": 1200,
            "allow_resumption": false,
            "full_debug": true
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

    let trace_started = mcp_call(
        address,
        &identity,
        "diagnostic_action",
        json!({
            "lease_id": lease_id,
            "action": {"start_frame_trace": {"duration_seconds": 5}}
        }),
    )?;
    require_tool_success("start_frame_trace", &trace_started)?;
    let repaint = mcp_call(
        address,
        &identity,
        "diagnostic_action",
        json!({"lease_id": lease_id, "action": "repaint"}),
    )?;
    require_tool_success("repaint during frame trace", &repaint)?;
    let typed = mcp_call(
        address,
        &identity,
        "keyboard_action",
        json!({
            "lease_id": lease_id,
            "window_id": "native-privacy-missing-window",
            "generation": 1,
            "action": {"kind": "text", "text": TYPED_CANARY}
        }),
    )?;
    require_tool_error("leased typed-text privacy probe", &typed)?;
    require_no_canaries(
        "typed-text error response",
        &typed.to_string(),
        &[TYPED_CANARY, &identity.token],
    )?;
    thread::sleep(Duration::from_millis(250));
    let trace_stopped = mcp_call(
        address,
        &identity,
        "diagnostic_action",
        json!({"lease_id": lease_id, "action": "stop_frame_trace"}),
    )?;
    require_tool_success("stop_frame_trace", &trace_stopped)?;
    let diagnostics = mcp_call(
        address,
        &identity,
        "diagnostic_snapshot",
        json!({"lease_id": lease_id}),
    )?;
    require_tool_success("diagnostic_snapshot privacy probe", &diagnostics)?;
    require_diagnostic_privacy(&diagnostics, &identity)?;
    require_typed_probe_audit(&remote_snapshot(&environment)?, &identity)?;

    let live_metrics = metrics(address)?;
    require_no_canaries(
        "post-probe metrics",
        &live_metrics,
        &[
            TYPED_CANARY,
            CREDENTIAL_CANARY,
            &identity.client_id,
            &identity.token,
        ],
    )?;

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

fn require_tool_error(operation: &str, response: &Value) -> Result<(), String> {
    if response.get("error").is_some()
        || response
            .pointer("/result/isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        Ok(())
    } else {
        Err(format!("{operation} unexpectedly succeeded"))
    }
}

fn require_prelease_denial(operation: &str, response: &Value) -> Result<(), String> {
    require_tool_error(operation, response)?;
    if response
        .to_string()
        .contains("lease is missing, expired, suspended, or outside the resource boundary")
    {
        Ok(())
    } else {
        Err(format!(
            "pre-lease {operation} did not reach the production lease gate"
        ))
    }
}

fn require_metric(metrics: &str, expected: &str) -> Result<(), String> {
    metrics
        .contains(expected)
        .then_some(())
        .ok_or_else(|| format!("metrics omitted expected fixed series {expected}"))
}

fn require_no_canaries(surface: &str, text: &str, canaries: &[&str]) -> Result<(), String> {
    if canaries.iter().any(|canary| text.contains(canary)) {
        Err(format!("{surface} retained a private payload canary"))
    } else {
        Ok(())
    }
}

fn require_prelease_operation_audit<'a>(
    snapshot: &nickel_session_protocol::RemoteControlSnapshot,
    baseline: u64,
    expected: impl Iterator<Item = &'a str>,
) -> Result<(), String> {
    let expected = expected.collect::<BTreeSet<_>>();
    let events = snapshot
        .operation_audit
        .iter()
        .filter(|event| event.generation > baseline)
        .collect::<Vec<_>>();
    let observed = events
        .iter()
        .map(|event| event.method.as_str())
        .collect::<BTreeSet<_>>();
    if observed != expected || events.len() != expected.len() {
        return Err(
            "trusted operation audit did not record every pre-lease desktop denial exactly once"
                .into(),
        );
    }
    if events.iter().any(|event| {
        event.matched_lease_id.is_some() || event.outcome != RemoteOperationOutcome::Error
    }) {
        return Err(
            "trusted operation audit attributed a pre-lease denial to desktop authority".into(),
        );
    }
    let visible = events
        .iter()
        .map(|event| event.method.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    require_no_canaries(
        "trusted pre-lease operation audit",
        &visible,
        &[TYPED_CANARY, CREDENTIAL_CANARY],
    )
}

fn require_typed_probe_audit(
    snapshot: &nickel_session_protocol::RemoteControlSnapshot,
    identity: &Identity,
) -> Result<(), String> {
    let Some(event) = snapshot
        .operation_audit
        .iter()
        .rev()
        .find(|event| event.method == "keyboard_action")
    else {
        return Err("trusted operation audit omitted the leased typed-text probe".into());
    };
    if event.outcome != RemoteOperationOutcome::Error || event.matched_lease_id.is_some() {
        return Err("trusted operation audit recorded the wrong typed-text probe outcome".into());
    }
    let visible = snapshot
        .operation_audit
        .iter()
        .map(|event| event.method.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    require_no_canaries(
        "trusted operation audit",
        &visible,
        &[TYPED_CANARY, CREDENTIAL_CANARY, &identity.token],
    )
}

fn require_diagnostic_privacy(response: &Value, identity: &Identity) -> Result<(), String> {
    let snapshot = response
        .pointer("/result/structuredContent")
        .and_then(Value::as_object)
        .ok_or("diagnostic_snapshot omitted structured content")?;
    for field in [
        "metrics",
        "recent_events",
        "diagnostic_logs",
        "frame_trace",
        "trace_lifecycle",
    ] {
        if !snapshot.get(field).is_some_and(Value::is_object) {
            return Err(format!(
                "diagnostic_snapshot omitted available production {field} data"
            ));
        }
    }
    if snapshot.contains_key("operation_audit") {
        return Err("MCP diagnostics exposed the trusted local operation audit".into());
    }
    require_no_canaries(
        "metrics, events, logs, and traces in diagnostic_snapshot",
        &response.to_string(),
        &[
            TYPED_CANARY,
            CREDENTIAL_CANARY,
            &identity.client_id,
            &identity.token,
        ],
    )
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

fn desktop_tool_fixtures(
    address: SocketAddr,
    identity: &Identity,
) -> Result<BTreeMap<String, Value>, String> {
    let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/list",
        "params": {},
    })
    .to_string();
    let authorization = format!("Bearer {}", identity.token);
    let response = http_json(
        address,
        "/mcp",
        &body,
        &[
            ("Accept", "application/json, text/event-stream"),
            ("MCP-Protocol-Version", MCP_VERSION),
            ("Mcp-Method", "tools/list"),
            ("X-Nickel-Client", &identity.client_id),
            ("Authorization", &authorization),
        ],
    )?;
    if response.get("error").is_some() {
        return Err("tools/list failed while discovering the pre-lease matrix".into());
    }
    let tools = response
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .ok_or("tools/list omitted its tool inventory")?;
    let capability_free = [
        "client_connection",
        "request_control_lease",
        "list_control_leases",
        "get_control_status",
    ];
    let names = tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    if capability_free.iter().any(|name| !names.contains(name)) {
        return Err("tools/list omitted a capability-free control-plane tool".into());
    }

    tools
        .iter()
        .filter_map(|tool| {
            let name = tool.get("name")?.as_str()?;
            (!capability_free.contains(&name)).then_some((name, tool))
        })
        .map(|(name, tool)| {
            let schema = tool
                .get("inputSchema")
                .ok_or_else(|| format!("published tool {name} omitted its input schema"))?;
            let mut fixture = schema_fixture(schema, schema)?;
            if name == "pointer_action" {
                fixture["target"] = json!({"kind": "desktop"});
            } else if name == "keyboard_action" {
                fixture = json!({
                    "lease_id": 1,
                    "window_id": "native-privacy-missing-window",
                    "generation": 1,
                    "action": {"kind": "text", "text": TYPED_CANARY}
                });
            }
            Ok((name.to_owned(), fixture))
        })
        .collect()
}

fn schema_fixture(schema: &Value, root: &Value) -> Result<Value, String> {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let pointer = reference
            .strip_prefix('#')
            .ok_or("published tool schema used a non-local reference")?;
        return schema_fixture(
            root.pointer(pointer)
                .ok_or("published tool schema reference did not resolve")?,
            root,
        );
    }
    if let Some(value) = schema.get("const") {
        return Ok(value.clone());
    }
    if let Some(value) = schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|values| values.iter().find(|value| !value.is_null()))
    {
        return Ok(value.clone());
    }
    for keyword in ["oneOf", "anyOf"] {
        if let Some(options) = schema.get(keyword).and_then(Value::as_array) {
            let option = options
                .iter()
                .find(|option| option.get("type").and_then(Value::as_str) != Some("null"))
                .ok_or("published tool schema union had no concrete fixture")?;
            return schema_fixture(option, root);
        }
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("object") | None if schema.get("properties").is_some() => {
            let properties = schema["properties"]
                .as_object()
                .ok_or("published tool object schema had invalid properties")?;
            let required = schema
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|name| {
                    name.as_str()
                        .ok_or("published tool required field was not a string")
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut fixture = serde_json::Map::new();
            for name in required {
                let property = properties
                    .get(name)
                    .ok_or("published tool required field had no schema")?;
                fixture.insert(name.to_owned(), schema_fixture(property, root)?);
            }
            Ok(Value::Object(fixture))
        }
        Some("array") => Ok(Value::Array(Vec::new())),
        Some("integer") => Ok(json!(
            schema
                .get("minimum")
                .and_then(Value::as_i64)
                .unwrap_or(1)
                .max(1)
        )),
        Some("number") => Ok(json!(
            schema
                .get("minimum")
                .and_then(Value::as_f64)
                .unwrap_or(1.0)
                .max(1.0)
        )),
        Some("boolean") => Ok(Value::Bool(false)),
        Some("string") => Ok(Value::String("fixture".into())),
        Some("null") => Ok(Value::Null),
        kind => Err(format!("unsupported published tool schema kind {kind:?}")),
    }
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

fn metrics(address: SocketAddr) -> Result<String, String> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .map_err(|error| format!("could not connect to metrics listener: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    write!(
        stream,
        "GET /metrics HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|error| error.to_string())?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| format!("could not read metrics response: {error}"))?;
    if !response.starts_with("HTTP/1.1 200") {
        return Err("pre-lease metrics endpoint was unavailable".into());
    }
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .ok_or("metrics response omitted a body".into())
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
            return Err("shell readiness did not become ready before its deadline".into());
        }
        match query(response_timeout.min(remaining)) {
            Ok(ServerMessage::ShellReadiness(readiness)) if readiness.ready => return Ok(()),
            Ok(ServerMessage::ShellReadiness(_)) => {}
            Ok(message) => return Err(format!("readiness query returned {message:?}")),
            Err(SessionMessageError::RetryableReceive(_)) if Instant::now() < deadline => {}
            Err(SessionMessageError::RetryableReceive(error)) => {
                return Err(format!(
                    "readiness did not respond before its deadline: {error}"
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
    use super::{
        EGL_VENDOR_FILENAMES, MESA_VBLANK_MODE, SessionMessageError, classify_receive_error,
        configure_mesa_software_renderer, poll_readiness,
    };
    use nickel_session_protocol::{ServerMessage, ShellReadinessSnapshot};
    use std::{
        ffi::OsStr,
        io,
        path::Path,
        process::Command,
        time::{Duration, Instant},
    };

    #[test]
    fn wayland_software_renderer_disables_mesa_vblank_without_changing_x11() {
        let manifest = Path::new("/test/mesa-egl.json");
        let mut wayland = Command::new("nickel");
        configure_mesa_software_renderer(&mut wayland, manifest, true);
        assert_eq!(
            command_environment(&wayland, MESA_VBLANK_MODE),
            Some(OsStr::new("0"))
        );
        assert_eq!(
            command_environment(&wayland, EGL_VENDOR_FILENAMES),
            Some(manifest.as_os_str())
        );

        let mut x11 = Command::new("nickel");
        configure_mesa_software_renderer(&mut x11, manifest, false);
        assert_eq!(command_environment(&x11, MESA_VBLANK_MODE), None);
        assert_eq!(
            command_environment(&x11, EGL_VENDOR_FILENAMES),
            Some(manifest.as_os_str())
        );
    }

    fn command_environment<'a>(command: &'a Command, name: &str) -> Option<&'a OsStr> {
        command
            .get_envs()
            .find_map(|(key, value)| (key == name).then_some(value).flatten())
    }

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
