//! Live Linux acceptance for the Spec 0230/0231 remote-control boundary.
//!
//! This launches the production compositor and MCP server in an isolated nested
//! session. It exercises the pre-lease tool and diagnostics gates before locally
//! approving debug authority. The private test-control protocol labels emergency
//! input as either synthetic or physical-fixture input. The latter exercises
//! production source classification but is not evidence from a physical keyboard.

use nickel_session_protocol::{
    ClientEnvelope, Command, InputState, Query, RemoteControlEffectiveState, RemoteControlSnapshot,
    RemoteLeaseAction, RemoteLeaseTransition, RemoteOperationOutcome, RemoteResourceScope, Request,
    ServerEnvelope, ServerMessage, TestEmergencyControlSide, TestEmergencyControlSource, TestInput,
    decode, encode,
};
use serde_json::{Value, json};
#[path = "linux_remote_control_acceptance/ordinary_scopes.rs"]
mod ordinary_scopes;
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
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
const SUBSCRIPTION_MCP_VERSION: &str = "2026-07-28";
const STRESS_ROUNDS: usize = 4;
const MAX_STRESS_RESPONSE_LATENCY: Duration = Duration::from_secs(10);
const MAX_STRESS_WALL_TIME: Duration = Duration::from_secs(45);
const MAX_STRESS_RSS_KIB: u64 = 2 * 1024 * 1024;
const MAX_STRESS_RSS_GROWTH_KIB: u64 = 512 * 1024;
const EGL_VENDOR_FILENAMES: &str = "__EGL_VENDOR_LIBRARY_FILENAMES";
const MESA_EGL_VENDOR_MANIFEST: &str = "/usr/share/glvnd/egl_vendor.d/50_mesa.json";
const MESA_VBLANK_MODE: &str = "vblank_mode";
const TYPED_CANARY: &str = "native-typed-password-DO-NOT-RETAIN";
const CREDENTIAL_CANARY: &str = "native-credential-DO-NOT-RETAIN";
const STRESS_INPUT_CANARY: &str = "native-stress-input-DO-NOT-RETAIN";
const DENIAL_CANARY: &str = "native-denial-client-DO-NOT-RETAIN";
const EXPIRY_CANARY: &str = "native-expiry-client-DO-NOT-RETAIN";
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
    if ordinary_scopes::movement_enabled() {
        ordinary_scopes::prepare_movement_catalog(runtime.path(), directory)?;
    }
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
        "PASS: production pre-lease metrics, complete desktop-tool denial, bounded concurrent diagnostic/capture/input stress, denial/expiry/revocation, repeated shell-surface/output/full-session actions without new prompts, payload-free diagnostics, and emergency revocation passed; no physical keyboard was exercised"
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
    let denied_identity = exercise_permission_denial(&environment, address)?;

    let bootstrap = approve_scope(
        &environment,
        address,
        &identity,
        RemoteResourceScope::FullSession,
        true,
    )?;
    let mut stress = exercise_scope_matrix(&environment, address, &identity, bootstrap)?;
    let ordinary_scopes = ordinary_scopes::movement_enabled()
        || env::args().any(|argument| argument == "--ordinary-scopes");
    let lease_id = if ordinary_scopes {
        ordinary_scopes::exercise(&environment, address, &identity, stress.lease_id)?
    } else {
        stress.lease_id
    };
    stress = if ordinary_scopes {
        current_stress_resources(&environment, address, &identity, lease_id)?
    } else {
        StressResources { lease_id, ..stress }
    };

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
    require_native_renderer_diagnostics(&diagnostics)?;
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

    let known_methods = published_metric_methods(desktop_tools.keys().map(String::as_str));
    exercise_native_stress(
        session,
        address,
        &identity,
        &stress,
        &known_methods,
        &[&denied_identity.client_id, &denied_identity.token],
    )?;
    let expiry_identity = exercise_expiry_and_revocation(&environment, address, lease_id)?;
    let final_metrics = metrics(address)?;
    require_metric(&final_metrics, "nickel_mcp_active_leases 1")?;
    validate_fixed_metrics(&final_metrics, &known_methods)?;
    require_no_canaries(
        "post-expiry and revocation metrics",
        &final_metrics,
        &[
            TYPED_CANARY,
            CREDENTIAL_CANARY,
            STRESS_INPUT_CANARY,
            DENIAL_CANARY,
            EXPIRY_CANARY,
            &identity.client_id,
            &identity.token,
            &denied_identity.client_id,
            &denied_identity.token,
            &expiry_identity.client_id,
            &expiry_identity.token,
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

/// The nested shell is a real production resource, not an ordinary application
/// window. Do not fabricate window or executable identity to fill those scopes.
struct StressResources {
    lease_id: u64,
    surface: nickel_session_protocol::RemoteResourceId,
    output: nickel_session_protocol::RemoteResourceId,
}

fn current_stress_resources(
    environment: &SessionEnvironment,
    address: SocketAddr,
    identity: &Identity,
    lease_id: u64,
) -> Result<StressResources, String> {
    let response = session_message(
        environment,
        Request::Command(Command::SetLauncherVisible { visible: true }),
    )?;
    if response != ServerMessage::Ack {
        return Err(format!(
            "local launcher restoration failed before stress: {response:?}"
        ));
    }
    let deadline = Instant::now() + DEADLINE;
    loop {
        let surfaces = scope_call(
            address,
            identity,
            "list_surfaces",
            json!({"lease_id": lease_id}),
        )?;
        let launcher = surfaces.as_array().and_then(|surfaces| {
            surfaces
                .iter()
                .find(|surface| surface["role"] == "launcher")
        });
        if let Some(launcher) = launcher {
            let surface = native_resource(launcher, "id")?;
            let outputs = scope_call(
                address,
                identity,
                "list_outputs",
                json!({"lease_id": lease_id}),
            )?;
            let output = outputs["outputs"]
                .as_array()
                .and_then(|outputs| {
                    outputs
                        .iter()
                        .find(|output| output["name"] == launcher["output"])
                })
                .ok_or("restored launcher has no native output identity")?;
            return Ok(StressResources {
                lease_id,
                surface,
                output: native_resource(output, "name")?,
            });
        }
        if Instant::now() >= deadline {
            return Err("launcher did not regain a live generation before stress".into());
        }
        thread::sleep(POLL);
    }
}

fn exercise_scope_matrix(
    environment: &SessionEnvironment,
    address: SocketAddr,
    identity: &Identity,
    bootstrap: u64,
) -> Result<StressResources, String> {
    let response = session_message(
        environment,
        Request::Command(Command::SetLauncherVisible { visible: true }),
    )?;
    if response != ServerMessage::Ack {
        return Err(format!("local launcher setup failed: {response:?}"));
    }
    let deadline = Instant::now() + DEADLINE;
    let (launcher, panel) = loop {
        let response = scope_call(
            address,
            identity,
            "list_surfaces",
            json!({"lease_id": bootstrap}),
        )?;
        let surfaces = response
            .as_array()
            .ok_or("surface inventory is not an array")?;
        let launcher = surfaces
            .iter()
            .find(|surface| surface["role"] == "launcher");
        let panel = surfaces.iter().find(|surface| surface["role"] == "panel");
        if let (Some(launcher), Some(panel)) = (launcher, panel) {
            break (launcher.clone(), panel.clone());
        }
        if Instant::now() >= deadline {
            return Err("native launcher and panel did not become visible".into());
        }
        thread::sleep(POLL);
    };
    let surface = native_resource(&launcher, "id")?;
    let outputs = scope_call(
        address,
        identity,
        "list_outputs",
        json!({"lease_id": bootstrap}),
    )?;
    let output = outputs["outputs"]
        .as_array()
        .and_then(|outputs| {
            outputs
                .iter()
                .find(|output| output["name"] == launcher["output"])
        })
        .ok_or("launcher has no native output identity")?;
    let output = native_resource(output, "name")?;
    revoke_scope(environment, bootstrap)?;

    let scopes = [
        ("surface", RemoteResourceScope::Surface(surface.clone())),
        ("output", RemoteResourceScope::Output(output.clone())),
        ("full_session", RemoteResourceScope::FullSession),
    ];
    let mut last_lease = None;
    for (label, scope) in scopes {
        let lease_id = approve_scope(
            environment,
            address,
            identity,
            scope.clone(),
            label == "full_session",
        )?;
        let approval = remote_snapshot(environment)?;
        require_unchanged_scope_approval(&approval, &approval, lease_id, &scope)?;
        for iteration in 0..3 {
            let inventory = scope_call(
                address,
                identity,
                "list_surfaces",
                json!({"lease_id": lease_id}),
            )?;
            let surfaces = inventory
                .as_array()
                .ok_or("scope inventory is not an array")?;
            if !surfaces
                .iter()
                .any(|entry| entry["id"] == surface.id && entry["generation"] == surface.generation)
            {
                return Err(format!("{label} lost its authorized launcher"));
            }
            if label == "surface" && (surfaces.len() != 1 || surfaces[0]["id"] != surface.id) {
                return Err("exact surface lease exposed another shell surface".into());
            }
            let text = format!("native scope {label} query {iteration}");
            let inspect = json!({"lease_id": lease_id, "surface_id": surface.id, "generation": surface.generation});
            let semantic_deadline = Instant::now() + DEADLINE;
            let outcome = loop {
                let before = scope_call(address, identity, "inspect_surface", inspect.clone())?;
                if before["surface"] != surface.id
                    || before["surface_generation"] != surface.generation
                    || before["tree_generation"].as_u64().is_none()
                {
                    return Err("semantic observation returned a different native surface".into());
                }
                let field = search_field(&before)?;
                let action = json!({
                    "lease_id": lease_id, "surface_id": surface.id,
                    "surface_generation": surface.generation,
                    "tree_generation": before["tree_generation"], "node": field["id"],
                    "action": {"kind": "set_text", "value": text.clone()}
                });
                thread::sleep(MATRIX_PACING);
                let response = mcp_call(address, identity, "surface_semantic_action", action)?;
                if response
                    .pointer("/result/isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    && response.to_string().contains("stale semantic tree")
                    && Instant::now() < semantic_deadline
                {
                    continue;
                }
                require_tool_success("surface_semantic_action", &response)?;
                break structured_content("surface_semantic_action", &response)?;
            };
            if outcome["changed"] != true
                || outcome["partial"] != false
                || !matches!(
                    outcome["completion"].as_str(),
                    Some("ui_updated" | "confirmed")
                )
            {
                return Err(format!(
                    "{label} semantic input was not a completed UI update: {outcome}"
                ));
            }
            // Fresh production observation proves application of the input;
            // the setter's response alone is only delivery evidence.
            let after = scope_call(address, identity, "inspect_surface", inspect)?;
            if search_field(&after)?["value"] != json!({"kind": "text", "value": text}) {
                return Err(format!("{label} search did not retain the delivered text"));
            }
            require_unchanged_scope_approval(
                &approval,
                &remote_snapshot(environment)?,
                lease_id,
                &scope,
            )?;
        }
        if label == "surface" {
            let denial = mcp_call(
                address,
                identity,
                "inspect_surface",
                json!({
                    "lease_id": lease_id, "surface_id": panel["id"], "generation": panel["generation"]
                }),
            )?;
            require_tool_error("surface lease observing unrelated panel", &denial)?;
            require_unchanged_scope_approval(
                &approval,
                &remote_snapshot(environment)?,
                lease_id,
                &scope,
            )?;
        }
        println!(
            "PASS: {label} scope, one local approval, three observed search edits, no new permission request or lease"
        );
        if label == "full_session" {
            last_lease = Some(lease_id);
        } else {
            revoke_scope(environment, lease_id)?;
        }
    }
    println!(
        "SHELL MATRIX LIMIT: ordinary window/application scopes require --ordinary-scopes; cross-output movement, physical input and assistive workflow remain separate acceptance"
    );
    Ok(StressResources {
        lease_id: last_lease.ok_or("scope matrix did not retain its final debug lease")?,
        surface,
        output,
    })
}

fn exercise_permission_denial(
    environment: &SessionEnvironment,
    address: SocketAddr,
) -> Result<Identity, String> {
    let identity = connect_identity(address, DENIAL_CANARY)?;
    let watch = ConnectionWatch::start(address, &identity)?;
    let response = mcp_call(
        address,
        &identity,
        "request_control_lease",
        json!({
            "scope": {"kind": "full_session"},
            "duration_seconds": 30,
            "allow_resumption": false,
            "full_debug": true
        }),
    )?;
    require_tool_success("request_control_lease for denial", &response)?;
    let pending = remote_snapshot(environment)?
        .pending_leases
        .into_iter()
        .find(|request| request.client_id == identity.client_id)
        .ok_or("denial fixture did not reach trusted local approval")?;
    let denied = session_message(
        environment,
        Request::Command(Command::DecideRemoteLease {
            pending_generation: pending.pending_generation,
            client_id: pending.client_id,
            request: pending.request,
            allow: false,
        }),
    )?;
    let ServerMessage::RemoteControl(denied) = denied else {
        return Err("local denial omitted its authoritative snapshot".into());
    };
    if !denied.pending_leases.is_empty() || !denied.active_leases.is_empty() {
        return Err("local denial left desktop authority or a pending request".into());
    }
    let response = mcp_call(
        address,
        &identity,
        "diagnostic_snapshot",
        json!({"lease_id": 9_000_001_u64}),
    )?;
    require_tool_error("diagnostic_snapshot after local denial", &response)?;
    let exposed = metrics(address)?;
    require_metric(
        &exposed,
        "nickel_mcp_permission_requests_total{outcome=\"denied\"} 1",
    )?;
    require_no_canaries(
        "metrics after local denial",
        &exposed,
        &[DENIAL_CANARY, &identity.client_id, &identity.token],
    )?;
    watch.finish()?;
    Ok(identity)
}

fn exercise_native_stress(
    session: &SessionProcess,
    address: SocketAddr,
    identity: &Identity,
    resources: &StressResources,
    known_methods: &BTreeSet<String>,
    extra_canaries: &[&str],
) -> Result<(), String> {
    let started = Instant::now();
    let rss_before = session.rss_kib()?;
    if rss_before > MAX_STRESS_RSS_KIB {
        return Err(format!(
            "nested compositor RSS exceeded the stress correctness ceiling before load: {rss_before} KiB"
        ));
    }
    let baseline = metrics(address)?;
    validate_fixed_metrics(&baseline, known_methods)?;

    let mut control_timing = StressTiming::new("trace and final snapshot");
    let (trace_started, latency) = timed_tool_call(
        address,
        identity,
        "diagnostic_action",
        json!({
            "lease_id": resources.lease_id,
            "action": {"start_frame_trace": {"duration_seconds": 30}}
        }),
    )?;
    require_tool_success("stress frame trace start", &trace_started)?;
    control_timing.observe(latency)?;
    let subscription = EventSubscription::start(address, identity, resources.lease_id)?;

    // The production admission limit permits four concurrent requests per client.
    // The mandatory watch and event subscription occupy two slots, so these two
    // lanes exercise concurrent owner work without turning the fixture into a
    // rate-limit benchmark.
    let lease_id = resources.lease_id;
    let diagnostic_identity = identity.clone();
    let diagnostic = thread::spawn(move || {
        let mut timing = StressTiming::new("diagnostic snapshot and event polling");
        for _ in 0..STRESS_ROUNDS {
            let (response, elapsed) = timed_tool_call(
                address,
                &diagnostic_identity,
                "diagnostic_snapshot",
                json!({"lease_id": lease_id}),
            )?;
            require_tool_success("diagnostic_snapshot under stress", &response)?;
            require_diagnostic_privacy(&response, &diagnostic_identity)?;
            require_no_canaries(
                "diagnostic_snapshot under native stress",
                &response.to_string(),
                &[STRESS_INPUT_CANARY],
            )?;
            timing.observe(elapsed)?;

            let (response, elapsed) = timed_tool_call(
                address,
                &diagnostic_identity,
                "read_desktop_events",
                json!({"lease_id": lease_id, "after_generation": 0}),
            )?;
            require_tool_success("read_desktop_events under stress", &response)?;
            require_no_canaries(
                "desktop events under native stress",
                &response.to_string(),
                &[STRESS_INPUT_CANARY, TYPED_CANARY, CREDENTIAL_CANARY],
            )?;
            timing.observe(elapsed)?;
        }
        Ok::<_, String>(timing)
    });

    let action_identity = identity.clone();
    let surface = resources.surface.clone();
    let output = resources.output.clone();
    let action = thread::spawn(move || {
        let mut timing = StressTiming::new("capture, semantic input, and repaint");
        for round in 0..STRESS_ROUNDS {
            let (capture, elapsed) = if round % 2 == 0 {
                timed_tool_call(
                    address,
                    &action_identity,
                    "capture_surface",
                    json!({
                        "lease_id": lease_id,
                        "surface_id": surface.id,
                        "generation": surface.generation
                    }),
                )?
            } else {
                timed_tool_call(
                    address,
                    &action_identity,
                    "capture_output",
                    json!({
                        "lease_id": lease_id,
                        "output_id": output.id,
                        "generation": output.generation
                    }),
                )?
            };
            require_tool_success("capture under stress", &capture)?;
            timing.observe(elapsed)?;

            let (tree, elapsed) = timed_tool_call(
                address,
                &action_identity,
                "inspect_surface",
                json!({
                    "lease_id": lease_id,
                    "surface_id": surface.id,
                    "generation": surface.generation
                }),
            )?;
            require_tool_success("inspect_surface for stress input", &tree)?;
            timing.observe(elapsed)?;
            let tree = structured_content("inspect_surface for stress input", &tree)?;
            let field = search_field(&tree)?;
            let (input, elapsed) = timed_tool_call(
                address,
                &action_identity,
                "surface_semantic_action",
                json!({
                    "lease_id": lease_id,
                    "surface_id": surface.id,
                    "surface_generation": surface.generation,
                    "tree_generation": tree["tree_generation"],
                    "node": field["id"],
                    "action": {
                        "kind": "set_text",
                        "value": format!("{STRESS_INPUT_CANARY}-{round}")
                    }
                }),
            )?;
            require_tool_success("surface_semantic_action under stress", &input)?;
            require_no_canaries(
                "semantic input result under native stress",
                &input.to_string(),
                &[STRESS_INPUT_CANARY],
            )?;
            timing.observe(elapsed)?;

            let (repaint, elapsed) = timed_tool_call(
                address,
                &action_identity,
                "diagnostic_action",
                json!({"lease_id": lease_id, "action": "repaint"}),
            )?;
            require_tool_success("repaint under stress", &repaint)?;
            timing.observe(elapsed)?;
        }
        Ok::<_, String>(timing)
    });

    let diagnostic = diagnostic
        .join()
        .map_err(|_| "diagnostic stress lane panicked".to_owned())??;
    let action = action
        .join()
        .map_err(|_| "capture/input stress lane panicked".to_owned())??;
    let subscription_transcript = subscription.finish()?;
    require_no_canaries(
        "event subscription transport",
        &subscription_transcript,
        &[STRESS_INPUT_CANARY, TYPED_CANARY, CREDENTIAL_CANARY],
    )?;

    let (trace_stopped, latency) = timed_tool_call(
        address,
        identity,
        "diagnostic_action",
        json!({"lease_id": resources.lease_id, "action": "stop_frame_trace"}),
    )?;
    require_tool_success("stress frame trace stop", &trace_stopped)?;
    control_timing.observe(latency)?;
    let (final_snapshot, latency) = timed_tool_call(
        address,
        identity,
        "diagnostic_snapshot",
        json!({"lease_id": resources.lease_id}),
    )?;
    require_tool_success("post-stress diagnostic_snapshot", &final_snapshot)?;
    require_diagnostic_privacy(&final_snapshot, identity)?;
    require_no_canaries(
        "post-stress diagnostics",
        &final_snapshot.to_string(),
        &[STRESS_INPUT_CANARY],
    )?;
    control_timing.observe(latency)?;

    let expected_increments = [
        ("diagnostic_snapshot", STRESS_ROUNDS as u64 + 1),
        ("read_desktop_events", STRESS_ROUNDS as u64),
        ("capture_surface", STRESS_ROUNDS.div_ceil(2) as u64),
        ("capture_output", (STRESS_ROUNDS / 2) as u64),
        ("inspect_surface", STRESS_ROUNDS as u64),
        ("surface_semantic_action", STRESS_ROUNDS as u64),
        ("diagnostic_action", STRESS_ROUNDS as u64 + 2),
        ("event_subscription", 1),
    ];
    let deadline = Instant::now() + Duration::from_secs(5);
    let exposed = loop {
        let exposed = metrics(address)?;
        if expected_increments.iter().all(|(method, expected)| {
            request_total(&exposed, method)
                >= request_total(&baseline, method).saturating_add(*expected)
        }) {
            break exposed;
        }
        if Instant::now() >= deadline {
            return Err("native stress metrics did not observe every completed operation".into());
        }
        thread::sleep(POLL);
    };
    validate_fixed_metrics(&exposed, known_methods)?;
    let mut canaries = vec![
        TYPED_CANARY,
        CREDENTIAL_CANARY,
        STRESS_INPUT_CANARY,
        &identity.client_id,
        &identity.token,
    ];
    canaries.extend_from_slice(extra_canaries);
    require_no_canaries("metrics after native stress", &exposed, &canaries)?;

    thread::sleep(Duration::from_millis(250));
    let rss_after = session.rss_kib()?;
    let rss_growth = rss_after.saturating_sub(rss_before);
    if rss_after > MAX_STRESS_RSS_KIB || rss_growth > MAX_STRESS_RSS_GROWTH_KIB {
        return Err(format!(
            "nested compositor RSS exceeded the stress correctness ceiling: before={rss_before} KiB after={rss_after} KiB growth={rss_growth} KiB"
        ));
    }
    if started.elapsed() > MAX_STRESS_WALL_TIME {
        return Err(format!(
            "native stress exceeded its correctness deadline of {MAX_STRESS_WALL_TIME:?}"
        ));
    }
    let max_latency = [
        diagnostic.max_latency,
        action.max_latency,
        control_timing.max_latency,
    ]
    .into_iter()
    .max()
    .unwrap_or_default();
    println!(
        "PASS: bounded native MCP stress: {} requests in {:?}, max response {:?}, RSS {} -> {} KiB (+{} KiB)",
        diagnostic.calls + action.calls + control_timing.calls,
        started.elapsed(),
        max_latency,
        rss_before,
        rss_after,
        rss_growth
    );
    Ok(())
}

fn exercise_expiry_and_revocation(
    environment: &SessionEnvironment,
    address: SocketAddr,
    retained_lease: u64,
) -> Result<Identity, String> {
    let identity = connect_identity(address, EXPIRY_CANARY)?;
    let watch = ConnectionWatch::start(address, &identity)?;
    let expired_lease = approve_additional_lease(environment, address, &identity, 1)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let expired = loop {
        let _ = metrics(address)?;
        let snapshot = remote_snapshot(environment)?;
        let retired = !snapshot
            .active_leases
            .iter()
            .any(|lease| lease.lease_id == expired_lease);
        let audited = snapshot.lease_audit.iter().any(|event| {
            event.lease_id == expired_lease && event.transition == RemoteLeaseTransition::Expired
        });
        if retired && audited {
            break snapshot;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "one-second native stress lease did not fully project expiry: retired={retired} audited={audited}"
            ));
        }
        thread::sleep(POLL);
    };
    let retained_authority = expired
        .active_leases
        .iter()
        .any(|lease| lease.lease_id == retained_lease);
    if !retained_authority {
        return Err("lease expiry changed unrelated authority".into());
    }
    let response = mcp_call(
        address,
        &identity,
        "diagnostic_snapshot",
        json!({"lease_id": expired_lease}),
    )?;
    require_tool_error("diagnostic_snapshot after lease expiry", &response)?;

    let revoked_lease = approve_additional_lease(environment, address, &identity, 30)?;
    session_message(
        environment,
        Request::Command(Command::ManageRemoteLease {
            lease_id: revoked_lease,
            action: RemoteLeaseAction::Revoke,
        }),
    )?;
    let revoked = remote_snapshot(environment)?;
    if revoked
        .active_leases
        .iter()
        .any(|lease| lease.lease_id == revoked_lease)
        || !revoked
            .active_leases
            .iter()
            .any(|lease| lease.lease_id == retained_lease)
        || !revoked.lease_audit.iter().any(|event| {
            event.lease_id == revoked_lease && event.transition == RemoteLeaseTransition::Revoked
        })
    {
        return Err(
            "explicit revocation changed unrelated authority or omitted its audit event".into(),
        );
    }
    let response = mcp_call(
        address,
        &identity,
        "diagnostic_snapshot",
        json!({"lease_id": revoked_lease}),
    )?;
    require_tool_error("diagnostic_snapshot after explicit revocation", &response)?;
    watch.finish()?;
    Ok(identity)
}

fn approve_additional_lease(
    environment: &SessionEnvironment,
    address: SocketAddr,
    identity: &Identity,
    duration_seconds: u64,
) -> Result<u64, String> {
    let before = remote_snapshot(environment)?;
    let prior = before
        .active_leases
        .iter()
        .map(|lease| lease.lease_id)
        .collect::<BTreeSet<_>>();
    let response = mcp_call(
        address,
        identity,
        "request_control_lease",
        json!({
            "scope": {"kind": "full_session"},
            "duration_seconds": duration_seconds,
            "allow_resumption": false,
            "full_debug": true
        }),
    )?;
    require_tool_success("request_control_lease for lifecycle stress", &response)?;
    let pending = remote_snapshot(environment)?
        .pending_leases
        .into_iter()
        .find(|request| request.client_id == identity.client_id)
        .ok_or("lifecycle stress request did not reach local approval")?;
    let approved = session_message(
        environment,
        Request::Command(Command::DecideRemoteLease {
            pending_generation: pending.pending_generation,
            client_id: pending.client_id,
            request: pending.request,
            allow: true,
        }),
    )?;
    let ServerMessage::RemoteControl(approved) = approved else {
        return Err("lifecycle stress approval omitted its authoritative snapshot".into());
    };
    approved
        .active_leases
        .iter()
        .find(|lease| !prior.contains(&lease.lease_id))
        .map(|lease| lease.lease_id)
        .ok_or("lifecycle stress approval did not create a distinct lease".into())
}

#[derive(Debug)]
struct StressTiming {
    name: &'static str,
    calls: usize,
    max_latency: Duration,
}

impl StressTiming {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            calls: 0,
            max_latency: Duration::ZERO,
        }
    }

    fn observe(&mut self, latency: Duration) -> Result<(), String> {
        self.calls += 1;
        self.max_latency = self.max_latency.max(latency);
        if latency > MAX_STRESS_RESPONSE_LATENCY {
            return Err(format!(
                "{} exceeded the {:?} response correctness ceiling: {latency:?}",
                self.name, MAX_STRESS_RESPONSE_LATENCY
            ));
        }
        Ok(())
    }
}

fn timed_tool_call(
    address: SocketAddr,
    identity: &Identity,
    name: &str,
    arguments: Value,
) -> Result<(Value, Duration), String> {
    let started = Instant::now();
    let response = mcp_call(address, identity, name, arguments)?;
    Ok((response, started.elapsed()))
}

fn structured_content(operation: &str, response: &Value) -> Result<Value, String> {
    let content = response
        .pointer("/result/structuredContent")
        .ok_or_else(|| format!("{operation} omitted structured content"))?;
    Ok(content.get("result").unwrap_or(content).clone())
}

fn published_metric_methods<'a>(desktop: impl Iterator<Item = &'a str>) -> BTreeSet<String> {
    desktop
        .chain([
            "client_connection",
            "request_control_lease",
            "list_control_leases",
            "get_control_status",
            "event_subscription",
        ])
        .map(str::to_owned)
        .collect()
}

fn request_total(metrics: &str, method: &str) -> u64 {
    let needle = format!("method=\"{method}\"");
    metrics
        .lines()
        .filter(|line| line.starts_with("nickel_mcp_requests_total{") && line.contains(&needle))
        .filter_map(|line| line.rsplit_once(' ')?.1.parse::<u64>().ok())
        .fold(0, u64::saturating_add)
}

fn validate_fixed_metrics(metrics: &str, known_methods: &BTreeSet<String>) -> Result<(), String> {
    if metrics.len() > 128 * 1024 {
        return Err("public metrics exceeded the native acceptance bound".into());
    }
    let mut observed_methods = BTreeSet::new();
    let scopes = ["surface", "window", "application", "output", "full_session"];
    let outcomes = ["success", "error", "cancelled"];
    let permission_outcomes = [
        "submitted",
        "coalesced",
        "approved",
        "denied",
        "cancelled",
        "blocked",
        "invalid",
        "capacity",
        "cooldown",
        "blocked_request",
        "unauthorized",
    ];
    let duration_bounds = ["0.001", "0.01", "0.1", "1", "5", "+Inf"];
    for line in metrics.lines().filter(|line| !line.starts_with('#')) {
        let Some((metric, labelled)) = line.split_once('{') else {
            continue;
        };
        let labels = labelled
            .split_once('}')
            .map(|(labels, _)| labels)
            .ok_or("public metrics contained malformed labels")?;
        let values = labels
            .split(',')
            .map(|label| {
                let (key, value) = label
                    .split_once('=')
                    .ok_or("public metrics contained a malformed label")?;
                Ok((key, value.trim_matches('"')))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        match metric {
            "nickel_mcp_requests_total" => {
                if values.len() != 2
                    || !outcomes.contains(&values.get("outcome").copied().unwrap_or_default())
                {
                    return Err("request metrics exposed non-fixed outcome labels".into());
                }
                let method = values
                    .get("method")
                    .ok_or("request metrics omitted the method label")?;
                if !known_methods.contains(*method) {
                    return Err("request metrics exposed a non-published method label".into());
                }
                observed_methods.insert((*method).to_owned());
            }
            "nickel_mcp_request_duration_seconds_bucket" => {
                if values.len() != 2
                    || !known_methods.contains(*values.get("method").unwrap_or(&""))
                    || !duration_bounds.contains(&values.get("le").copied().unwrap_or_default())
                {
                    return Err("duration metrics exposed non-fixed labels".into());
                }
            }
            "nickel_mcp_request_duration_seconds_count"
            | "nickel_mcp_request_duration_seconds_sum" => {
                if values.len() != 1
                    || !known_methods.contains(*values.get("method").unwrap_or(&""))
                {
                    return Err("duration metrics exposed a non-published method label".into());
                }
            }
            "nickel_mcp_active_leases_by_scope" => {
                if values.len() != 1
                    || !scopes.contains(&values.get("scope").copied().unwrap_or_default())
                {
                    return Err("lease metrics exposed a non-fixed scope label".into());
                }
            }
            "nickel_mcp_permission_requests_total" => {
                if values.len() != 1
                    || !permission_outcomes
                        .contains(&values.get("outcome").copied().unwrap_or_default())
                {
                    return Err("permission metrics exposed a non-fixed outcome label".into());
                }
            }
            _ => {
                return Err(format!(
                    "public metrics exposed unexpected labelled family {metric}"
                ));
            }
        }
    }
    if observed_methods != *known_methods {
        return Err("public metrics omitted a published fixed method label".into());
    }
    Ok(())
}

fn native_resource(
    value: &Value,
    id: &str,
) -> Result<nickel_session_protocol::RemoteResourceId, String> {
    let id = value[id]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("native resource identity missing")?;
    let generation = value["generation"]
        .as_u64()
        .filter(|generation| *generation != 0)
        .ok_or("native resource generation missing")?;
    Ok(nickel_session_protocol::RemoteResourceId {
        id: id.into(),
        generation,
    })
}

fn search_field(tree: &Value) -> Result<&Value, String> {
    let mut fields = tree["nodes"]
        .as_array()
        .ok_or("semantic tree omitted nodes")?
        .iter()
        .filter(|node| node["role"] == "TextField" && node["enabled"] == true);
    let field = fields
        .next()
        .ok_or("launcher omitted its editable search field")?;
    if fields.next().is_some() || field["id"].as_u64().is_none() {
        return Err("launcher search field is ambiguous".into());
    }
    Ok(field)
}

fn scope_call(
    address: SocketAddr,
    identity: &Identity,
    operation: &str,
    arguments: Value,
) -> Result<Value, String> {
    thread::sleep(MATRIX_PACING);
    let response = mcp_call(address, identity, operation, arguments)?;
    require_tool_success(operation, &response)?;
    let content = response
        .pointer("/result/structuredContent")
        .ok_or_else(|| format!("{operation} omitted structured content"))?;
    // rmcp wraps non-object JSON results in a result field.
    Ok(content.get("result").unwrap_or(content).clone())
}

fn approve_scope(
    environment: &SessionEnvironment,
    address: SocketAddr,
    identity: &Identity,
    scope: RemoteResourceScope,
    full_debug: bool,
) -> Result<u64, String> {
    let before = remote_snapshot(environment)?;
    if !before.active_leases.is_empty() || !before.pending_leases.is_empty() {
        return Err("scope approval started with existing desktop authority or request".into());
    }
    let response = mcp_call(
        address,
        identity,
        "request_control_lease",
        json!({
            "scope": scope, "duration_seconds": 1200, "allow_resumption": false, "full_debug": full_debug
        }),
    )?;
    require_tool_success("request_control_lease", &response)?;
    let pending = remote_snapshot(environment)?;
    if pending.pending_leases.len() != 1 {
        return Err("scope request did not create exactly one local approval".into());
    }
    let pending = &pending.pending_leases[0];
    if pending.client_id != identity.client_id || pending.request.scope != scope {
        return Err("local approval identifies a different client or scope".into());
    }
    let approved = session_message(
        environment,
        Request::Command(Command::DecideRemoteLease {
            pending_generation: pending.pending_generation,
            client_id: pending.client_id.clone(),
            request: pending.request.clone(),
            allow: true,
        }),
    )?;
    let ServerMessage::RemoteControl(approved) = approved else {
        return Err("scope approval omitted its authoritative snapshot".into());
    };
    let lease = approved
        .active_leases
        .first()
        .ok_or("approval did not create a lease")?;
    require_unchanged_scope_approval(&approved, &approved, lease.lease_id, &scope)?;
    Ok(lease.lease_id)
}

fn revoke_scope(environment: &SessionEnvironment, lease_id: u64) -> Result<(), String> {
    session_message(
        environment,
        Request::Command(Command::ManageRemoteLease {
            lease_id,
            action: RemoteLeaseAction::Revoke,
        }),
    )?;
    let snapshot = remote_snapshot(environment)?;
    if !snapshot.active_leases.is_empty() || !snapshot.pending_leases.is_empty() {
        return Err("scope revocation left desktop authority or a pending request".into());
    }
    Ok(())
}

fn require_unchanged_scope_approval(
    approved: &RemoteControlSnapshot,
    current: &RemoteControlSnapshot,
    lease_id: u64,
    scope: &RemoteResourceScope,
) -> Result<(), String> {
    if !current.pending_leases.is_empty()
        || current.effective != RemoteControlEffectiveState::Enabled
        || current.active_leases.len() != 1
        || current.active_leases[0].lease_id != lease_id
        || &current.active_leases[0].scope != scope
        || current.active_leases[0].suspended
        || current.active_leases[0].full_debug != approved.active_leases[0].full_debug
        || current.permission_audit != approved.permission_audit
        || current.permission_audit_evicted != approved.permission_audit_evicted
        || current.lease_audit != approved.lease_audit
        || current.lease_audit_evicted != approved.lease_audit_evicted
    {
        return Err(
            "ordinary scoped action changed authority or requested another approval".into(),
        );
    }
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

fn require_native_renderer_diagnostics(response: &Value) -> Result<(), String> {
    let snapshot = response
        .pointer("/result/structuredContent")
        .and_then(Value::as_object)
        .ok_or("diagnostic_snapshot omitted structured content")?;
    let observed_at = snapshot
        .get("observed_at_us")
        .and_then(Value::as_u64)
        .ok_or("diagnostic_snapshot omitted its observation timestamp")?;

    let cache = snapshot
        .get("shared_presenter_cache")
        .and_then(Value::as_object)
        .ok_or("native diagnostic_snapshot omitted shared presenter cache accounting")?;
    if cache.get("observed_at_us").and_then(Value::as_u64) != Some(observed_at) {
        return Err("shared presenter cache was not correlated to the snapshot observation".into());
    }
    for field in [
        "observation_generation",
        "cache_generation",
        "cache_owners",
        "live_entries",
        "live_bytes",
        "peak_cache_bytes",
        "hits",
        "misses",
        "insertions",
        "evictions",
        "invalidations",
        "recomputation_nanos",
    ] {
        if !cache.get(field).is_some_and(Value::is_u64) {
            return Err(format!(
                "shared presenter cache omitted bounded counter {field}"
            ));
        }
    }

    // The nested acceptance child has no DRM/native presentation owner. If a
    // backend supplies this optional collector, require live, correlated data;
    // physical DRM acceptance remains a separate gate when it is absent.
    if let Some(dispatch) = snapshot
        .get("native_presentation_dispatch")
        .and_then(Value::as_object)
    {
        if dispatch.get("observed_at_us").and_then(Value::as_u64) != Some(observed_at) {
            return Err(
                "native presentation dispatch was not correlated to the snapshot observation"
                    .into(),
            );
        }
        if dispatch.get("dispatches").and_then(Value::as_u64) == Some(0)
            || dispatch.get("dispatch_generation").and_then(Value::as_u64) == Some(0)
            || !dispatch.get("dispatch_cpu_us").is_some_and(Value::is_u64)
            || !dispatch
                .get("max_dispatch_cpu_us")
                .is_some_and(Value::is_u64)
            || !dispatch
                .get("last_dispatch_at_us")
                .is_some_and(Value::is_u64)
        {
            return Err(
                "native presentation dispatch did not record completed render activity".into(),
            );
        }
    }

    let unavailable = snapshot
        .get("unavailable_domains")
        .and_then(Value::as_array)
        .ok_or("diagnostic_snapshot omitted unavailable domains")?;
    for domain in [
        "native_gpu_completion_timing",
        "gpu_driver_and_external_renderer_resources",
        "shared_renderer_cache_per_surface_attribution",
    ] {
        if !unavailable
            .iter()
            .any(|value| value.as_str() == Some(domain))
        {
            return Err(format!(
                "diagnostic_snapshot did not report {domain} unavailable"
            ));
        }
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

#[derive(Clone, Debug)]
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
    client_metadata_for(progress, MCP_VERSION)
}

fn client_metadata_for(progress: u64, protocol: &str) -> Value {
    json!({
        "progressToken": progress,
        "io.modelcontextprotocol/protocolVersion": protocol,
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

struct EventSubscription {
    stop: mpsc::SyncSender<()>,
    worker: thread::JoinHandle<Result<String, String>>,
}

impl EventSubscription {
    fn start(address: SocketAddr, identity: &Identity, lease_id: u64) -> Result<Self, String> {
        let client_id = identity.client_id.clone();
        let token = identity.token.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (stop, stopped) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            open_event_subscription(address, &client_id, &token, lease_id, ready_tx, stopped)
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
                Err("desktop event subscription did not become ready".into())
            }
        }
    }

    fn finish(self) -> Result<String, String> {
        let _ = self.stop.send(());
        self.worker
            .join()
            .map_err(|_| "desktop event subscription thread panicked".to_owned())?
    }
}

fn open_event_subscription(
    address: SocketAddr,
    client_id: &str,
    token: &str,
    lease_id: u64,
    ready: mpsc::SyncSender<Result<(), String>>,
    stop: mpsc::Receiver<()>,
) -> Result<String, String> {
    let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let uri = format!("nickel://desktop-events/{lease_id}");
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "subscriptions/listen",
        "params": {
            "notifications": {"resourceSubscriptions": [&uri]},
            "_meta": client_metadata_for(id, SUBSCRIPTION_MCP_VERSION)
        }
    })
    .to_string();
    let result = (|| {
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
            .map_err(|error| format!("could not open desktop event subscription: {error}"))?;
        stream
            .set_read_timeout(Some(Duration::from_millis(250)))
            .map_err(|error| error.to_string())?;
        write!(
            stream,
            "POST /mcp HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: {SUBSCRIPTION_MCP_VERSION}\r\nMcp-Method: subscriptions/listen\r\nX-Nickel-Client: {client_id}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .map_err(|error| error.to_string())?;
        let mut transcript = String::new();
        let mut acknowledged = false;
        loop {
            if stop.try_recv().is_ok() {
                let _ = stream.shutdown(Shutdown::Both);
                return Ok(transcript);
            }
            let mut chunk = [0; 4096];
            match stream.read(&mut chunk) {
                Ok(0) => {
                    return Err(
                        "desktop event subscription ended before the stress load completed".into(),
                    );
                }
                Ok(count) => {
                    transcript.push_str(&String::from_utf8_lossy(&chunk[..count]));
                    if transcript.len() > 128 * 1024 {
                        return Err(
                            "desktop event subscription transcript exceeded its bound".into()
                        );
                    }
                    if transcript.contains("\r\n\r\n") && !transcript.starts_with("HTTP/1.1 200") {
                        return Err("desktop event subscription HTTP request failed".into());
                    }
                    if !acknowledged
                        && transcript.contains("notifications/subscriptions/acknowledged")
                        && transcript.contains("notifications/resources/updated")
                        && transcript.contains(&uri)
                    {
                        acknowledged = true;
                        let _ = ready.send(Ok(()));
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(error) => return Err(format!("desktop event subscription failed: {error}")),
            }
        }
    })();
    if result.is_err() {
        let message = result
            .as_ref()
            .err()
            .cloned()
            .unwrap_or_else(|| "desktop event subscription failed".into());
        let _ = ready.send(Err(message));
    }
    result
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
    wayland: String,
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
        wayland: value("WAYLAND_DISPLAY")?,
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

    fn rss_kib(&self) -> Result<u64, String> {
        let status = fs::read_to_string(format!("/proc/{}/status", self.child.id()))
            .map_err(|error| format!("could not read nested compositor RSS: {error}"))?;
        status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse().ok())
            .ok_or("nested compositor status omitted VmRSS".into())
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
    fn scope_proof_detects_transient_permission_requests_and_extra_authority() {
        use nickel_session_protocol::{RemoteControlSnapshot, RemoteResourceScope};
        use serde_json::json;
        let approved: RemoteControlSnapshot = serde_json::from_value(json!({
            "requested_enabled": true, "effective": "enabled", "generation": 1,
            "acknowledged_generation": 1, "endpoint": "http://127.0.0.1:1/mcp",
            "diagnostic": null, "pending_clients": [], "granted_clients": [],
            "active_leases": [{"lease_id": 7, "client_label": "fixture",
                "scope": {"kind": "full_session"}, "remaining_seconds": 1200,
                "suspended": false, "full_debug": true}]
        }))
        .unwrap();
        let verify = |current: &RemoteControlSnapshot| {
            super::require_unchanged_scope_approval(
                &approved,
                current,
                7,
                &RemoteResourceScope::FullSession,
            )
        };
        let mut countdown = approved.clone();
        countdown.active_leases[0].remaining_seconds = Some(1199);
        assert!(
            verify(&countdown).is_ok(),
            "ordinary elapsed time is not a new approval"
        );
        let mut transient = approved.clone();
        transient.permission_audit.push(
            serde_json::from_value(json!({
                "generation": 1, "observed_at_us": 2, "client_id": 3, "outcome": "denied"
            }))
            .unwrap(),
        );
        assert!(
            verify(&transient).is_err(),
            "a request that disappeared must still fail proof"
        );
        let mut widened = approved.clone();
        widened
            .active_leases
            .push(approved.active_leases[0].clone());
        assert!(verify(&widened).is_err());
        let mut replaced = approved.clone();
        replaced.active_leases[0].lease_id = 8;
        assert!(verify(&replaced).is_err());
        let mut overflow = approved.clone();
        overflow.permission_audit_evicted = 1;
        assert!(
            verify(&overflow).is_err(),
            "audit eviction cannot conceal an extra prompt"
        );
    }

    #[test]
    fn scope_fixture_refuses_invented_identity_or_ambiguous_search() {
        use serde_json::json;
        assert!(super::native_resource(&json!({"id": "internal:4"}), "id").is_err());
        assert!(
            super::native_resource(&json!({"id": "internal:4", "generation": 0}), "id").is_err()
        );
        let field = json!({"id": 0, "role": "TextField", "enabled": true});
        assert!(super::search_field(&json!({"nodes": [field.clone()]})).is_ok());
        assert!(super::search_field(&json!({"nodes": [field.clone(), field]})).is_err());
        assert!(super::search_field(&json!({"nodes": []})).is_err());
    }

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
