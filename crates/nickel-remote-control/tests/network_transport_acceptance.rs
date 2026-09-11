//! Host-network acceptance for Spec 0231's protected MCP transport.
//!
//! This is ignored because it requires a bindable non-loopback interface. It uses two OS
//! processes and the host's actual routing table, but it is not cross-machine evidence.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::Arc,
    time::Duration,
};

use nickel_remote_control::{
    ClientConnectionAction, ClientConnectionPermit, DesktopAuthority, DesktopPermit,
    EffectiveState, RemoteAiControlSettings, RemoteControlRuntime, WindowSummary,
    diagnostics::DiagnosticSnapshot,
    keyboard::KeyboardAction,
    pointer::PointerAction,
    window_actions::{WindowAction, WindowOutcome},
};
use sha2::{Digest, Sha256};

const ROLE: &str = "NICKEL_NETWORK_ACCEPTANCE_ROLE";
const ADDRESS: &str = "NICKEL_NETWORK_ACCEPTANCE_ADDRESS";
const TEST_NAME: &str = "non_loopback_tls_process_acceptance";
const EVENT: &str = "NICKEL_NETWORK_ACCEPTANCE ";
const PROTOCOL: &str = "2025-06-18";

struct AcceptanceDesktop;

impl DesktopAuthority for AcceptanceDesktop {
    fn client_connection(
        &self,
        permit: ClientConnectionPermit,
        action: ClientConnectionAction,
    ) -> Result<(), String> {
        permit.apply(action, false)
    }

    fn keyboard_action(
        &self,
        _permit: DesktopPermit,
        _id: &str,
        _generation: u64,
        _action: KeyboardAction,
    ) -> Result<(), String> {
        Err("acceptance fixture has no native keyboard".into())
    }

    fn pointer_action(
        &self,
        _permit: DesktopPermit,
        _target: nickel_remote_control::pointer::PointerTarget,
        _x: i32,
        _y: i32,
        _action: PointerAction,
    ) -> Result<(), String> {
        Err("acceptance fixture has no native pointer".into())
    }

    fn diagnostic_snapshot(&self, _permit: DesktopPermit) -> Result<DiagnosticSnapshot, String> {
        Err("acceptance fixture has no compositor".into())
    }

    fn list_windows(&self, _permit: DesktopPermit) -> Result<Vec<WindowSummary>, String> {
        Ok(Vec::new())
    }

    fn window_action(
        &self,
        _permit: DesktopPermit,
        _id: &str,
        _generation: u64,
        _action: WindowAction,
    ) -> Result<WindowOutcome, String> {
        Err("acceptance fixture has no windows".into())
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn child_command(role: &str, address: SocketAddr) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("acceptance test executable"));
    command
        .args(["--ignored", "--exact", TEST_NAME, "--nocapture"])
        .env(ROLE, role)
        .env(ADDRESS, address.to_string())
        .env("NICKEL_MCP_LISTEN_ADDR", address.to_string())
        .env("NICKEL_MCP_TLS_CERT", fixture("localhost-cert.pem"))
        .env("NICKEL_MCP_TLS_KEY", fixture("localhost-key.pem"));
    command
}

fn emit(value: serde_json::Value) {
    println!("{EVENT}{value}");
    std::io::stdout().flush().unwrap();
}

fn server_child(once: bool) {
    let requested = std::env::var(ADDRESS).unwrap();
    let mut runtime = RemoteControlRuntime::default();
    runtime.apply(
        &RemoteAiControlSettings::default(),
        Arc::new(AcceptanceDesktop),
    );
    let status = runtime.status();
    emit(serde_json::json!({
        "kind": "status",
        "effective": format!("{:?}", status.effective),
        "endpoint": status.endpoint,
        "fingerprint": status.host_fingerprint,
        "environment_override": status.environment_override,
        "diagnostic": status.diagnostic,
    }));
    if once || status.effective != EffectiveState::Enabled {
        return;
    }
    assert_eq!(status.endpoint, format!("https://{requested}/mcp"));

    let control = runtime.control();
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let mut words = line.split_whitespace();
        match words.next() {
            Some("approve") => {
                let pending = {
                    let state = control.lock().unwrap();
                    state
                        .lease_requests()
                        .pending()
                        .next()
                        .map(|(client, request)| (client.to_owned(), request.clone()))
                }
                .expect("remote lease request reached local owner");
                let lease = {
                    let mut state = control.lock().unwrap();
                    let generation = state
                        .lease_requests()
                        .pending_generation(&pending.0)
                        .unwrap();
                    state
                        .approve_lease_local(
                            &pending.0,
                            &pending.1,
                            generation,
                            std::time::Instant::now(),
                        )
                        .unwrap()
                };
                emit(serde_json::json!({"kind":"approved", "lease_id":lease}));
            }
            Some("origin") => {
                let client = words.next().unwrap();
                let state = control.lock().unwrap();
                let origin = state
                    .client_origin(client)
                    .expect("authenticated client origin");
                assert!(state.granted_clients().any(|grant| grant.id == client));
                emit(serde_json::json!({
                    "kind":"origin", "address":origin.address.to_string(), "tls":origin.tls,
                    "capabilities":0
                }));
            }
            Some("revoke") => {
                let client = words.next().unwrap();
                assert!(control.lock().unwrap().revoke(client));
                emit(serde_json::json!({"kind":"revoked"}));
            }
            Some("stop") => break,
            command => panic!("unknown parent command {command:?}"),
        }
    }
}

struct ServerChild {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl ServerChild {
    fn spawn(address: SocketAddr) -> Self {
        let mut child = child_command("server", address)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            input,
            output,
        }
    }

    fn event(&mut self) -> serde_json::Value {
        loop {
            let mut line = String::new();
            assert_ne!(
                self.output.read_line(&mut line).unwrap(),
                0,
                "server child exited"
            );
            if let Some((_, value)) = line.split_once(EVENT) {
                return serde_json::from_str(value.trim()).unwrap();
            }
        }
    }

    fn command(&mut self, command: &str) -> serde_json::Value {
        writeln!(self.input, "{command}").unwrap();
        self.input.flush().unwrap();
        self.event()
    }

    fn stop(mut self) {
        writeln!(self.input, "stop").unwrap();
        self.input.flush().unwrap();
        let status = self.child.wait().unwrap();
        assert!(status.success(), "server child failed: {status}");
    }
}

impl Drop for ServerChild {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn expected_certificate() -> rustls::pki_types::CertificateDer<'static> {
    rustls_pemfile::certs(&mut include_bytes!("fixtures/localhost-cert.pem").as_slice())
        .next()
        .unwrap()
        .unwrap()
}

fn fingerprint(certificate: &[u8]) -> String {
    Sha256::digest(certificate)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn tls_stream(
    address: SocketAddr,
    server_name: &str,
    expected_fingerprint: &str,
) -> Result<rustls::StreamOwned<rustls::ClientConnection, TcpStream>, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certificate = expected_certificate();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(certificate).map_err(|error| error.to_string())?;
    let config = Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let name = server_name
        .to_owned()
        .try_into()
        .map_err(|_| "invalid test server name".to_owned())?;
    let connection = rustls::ClientConnection::new(config, name).map_err(|e| e.to_string())?;
    let socket =
        TcpStream::connect_timeout(&address, Duration::from_secs(3)).map_err(|e| e.to_string())?;
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    socket
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    let mut stream = rustls::StreamOwned::new(connection, socket);
    while stream.conn.is_handshaking() {
        stream
            .conn
            .complete_io(&mut stream.sock)
            .map_err(|e| e.to_string())?;
    }
    let presented = stream
        .conn
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .ok_or("server presented no certificate")?;
    let presented = fingerprint(presented.as_ref());
    if presented != expected_fingerprint {
        return Err(format!(
            "host fingerprint mismatch: expected {expected_fingerprint}, received {presented}"
        ));
    }
    Ok(stream)
}

fn tls_request(address: SocketAddr, fingerprint: &str, request: &str) -> String {
    let mut stream = tls_stream(address, "localhost", fingerprint).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    let result = stream.read_to_string(&mut response);
    assert!(
        result.is_ok()
            || result
                .as_ref()
                .is_err_and(|error| error.kind() == std::io::ErrorKind::UnexpectedEof),
        "TLS response failed: {result:?}"
    );
    response
}

fn response_json(response: &str) -> serde_json::Value {
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap()
}

fn mcp_error(response: &str) -> Option<bool> {
    response.starts_with("HTTP/1.1 200").then(|| {
        response_json(response)["result"]["isError"]
            .as_bool()
            .expect("MCP tool result has an error disposition")
    })
}

fn post_json(address: SocketAddr, fingerprint: &str, path: &str, body: &str) -> String {
    tls_request(
        address,
        fingerprint,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
}

fn mcp_call(
    address: SocketAddr,
    fingerprint: &str,
    client: &str,
    token: &str,
    id: u64,
    name: &str,
    arguments: serde_json::Value,
) -> String {
    let body = serde_json::json!({
        "jsonrpc":"2.0", "id":id, "method":"tools/call",
        "params":{"name":name, "arguments":arguments}
    })
    .to_string();
    tls_request(
        address,
        fingerprint,
        &format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: {PROTOCOL}\r\nMcp-Method: tools/call\r\nMcp-Name: {name}\r\nX-Nickel-Client: {client}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
}

fn open_watch(
    address: SocketAddr,
    fingerprint: String,
    client: String,
    token: String,
    id: u64,
) -> (
    std::thread::JoinHandle<String>,
    std::sync::mpsc::Receiver<()>,
) {
    let (ready, observed) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let body = serde_json::json!({
            "jsonrpc":"2.0", "id":id, "method":"tools/call", "params": {
                "name":"client_connection", "arguments":{"action":"watch"},
                "_meta": {
                    "progressToken":id,
                    "io.modelcontextprotocol/protocolVersion":PROTOCOL,
                    "io.modelcontextprotocol/clientInfo":{"name":"network acceptance","version":"1"},
                    "io.modelcontextprotocol/clientCapabilities":{}
                }
            }
        })
        .to_string();
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: {PROTOCOL}\r\nMcp-Method: tools/call\r\nMcp-Name: client_connection\r\nX-Nickel-Client: {client}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let mut stream = tls_stream(address, "localhost", &fingerprint).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut bytes = Vec::new();
        let mut announced = false;
        loop {
            let mut chunk = [0; 4096];
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => {
                    bytes.extend_from_slice(&chunk[..count]);
                    if !announced
                        && String::from_utf8_lossy(&bytes).contains("Connection watch active")
                    {
                        ready.send(()).unwrap();
                        announced = true;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(error)
                    if announced
                        && matches!(
                            error.kind(),
                            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                        ) =>
                {
                    break;
                }
                Err(error) => panic!("watch transport failed: {error}"),
            }
        }
        assert!(announced, "watch never became ready");
        String::from_utf8(bytes).unwrap()
    });
    (worker, observed)
}

fn host_address() -> IpAddr {
    let probe = UdpSocket::bind("0.0.0.0:0").unwrap();
    probe.connect("192.0.2.1:9").unwrap();
    let address = probe.local_addr().unwrap().ip();
    assert!(
        !address.is_loopback() && !address.is_unspecified(),
        "acceptance requires a routable non-loopback IPv4 interface"
    );
    address
}

#[test]
#[ignore = "requires a bindable host-network address; this is local multi-process evidence only"]
fn non_loopback_tls_process_acceptance() {
    match std::env::var(ROLE).as_deref() {
        Ok("server") => return server_child(false),
        Ok("server-once") => return server_child(true),
        _ => {}
    }

    let reservation = TcpListener::bind(SocketAddr::new(host_address(), 0)).unwrap();
    let address = reservation.local_addr().unwrap();
    let unavailable = child_command("server-once", address).output().unwrap();
    assert!(unavailable.status.success());
    let unavailable = String::from_utf8(unavailable.stdout).unwrap();
    let rejected: serde_json::Value = serde_json::from_str(
        unavailable
            .split(EVENT)
            .nth(1)
            .expect("rejected child status")
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(rejected["effective"], "Rejected");
    assert_eq!(rejected["endpoint"], format!("https://{address}/mcp"));
    assert_eq!(rejected["environment_override"], true);
    assert!(
        rejected["diagnostic"]
            .as_str()
            .unwrap()
            .contains(&address.to_string())
    );
    drop(reservation);

    let mut server = ServerChild::spawn(address);
    let status = server.event();
    assert_eq!(status["effective"], "Enabled");
    assert_eq!(status["endpoint"], format!("https://{address}/mcp"));
    assert_eq!(status["environment_override"], true);
    let advertised_fingerprint = status["fingerprint"].as_str().unwrap().to_owned();
    assert_eq!(
        advertised_fingerprint,
        fingerprint(expected_certificate().as_ref())
    );

    assert!(
        tls_stream(address, "wrong-host.invalid", &advertised_fingerprint).is_err(),
        "certificate must reject the wrong DNS identity"
    );
    assert!(
        tls_stream(address, "localhost", &"0".repeat(64)).is_err(),
        "client must reject an unconfirmed host fingerprint"
    );

    let metrics = tls_request(
        address,
        &advertised_fingerprint,
        "GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(metrics.starts_with("HTTP/1.1 200"), "{metrics}");
    assert!(metrics.contains("nickel_mcp_active_leases 0"));
    let mut plaintext = TcpStream::connect(address).unwrap();
    plaintext
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    plaintext
        .write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut exposed = String::new();
    let _ = plaintext.read_to_string(&mut exposed);
    assert!(!exposed.contains("nickel_mcp_"));
    assert!(!exposed.starts_with("HTTP/1.1 200"));

    let identity = response_json(&post_json(
        address,
        &advertised_fingerprint,
        "/clients/connect",
        r#"{"label":"private-network-acceptance-client"}"#,
    ));
    let client = identity["client_id"].as_str().unwrap().to_owned();
    let token = identity["token"].as_str().unwrap().to_owned();
    let origin = server.command(&format!("origin {client}"));
    let origin_address: IpAddr = origin["address"].as_str().unwrap().parse().unwrap();
    assert!(!origin_address.is_loopback());
    assert_eq!(origin["tls"], true);
    assert_eq!(origin["capabilities"], 0);

    let (first_watch, first_ready) = open_watch(
        address,
        advertised_fingerprint.clone(),
        client.clone(),
        token.clone(),
        10,
    );
    first_ready.recv_timeout(Duration::from_secs(5)).unwrap();
    let denied = mcp_call(
        address,
        &advertised_fingerprint,
        &client,
        &token,
        11,
        "list_windows",
        serde_json::json!({"lease_id":1}),
    );
    assert_eq!(mcp_error(&denied), Some(true), "{denied}");
    assert!(denied.contains("lease is missing, expired, suspended"));

    let requested = mcp_call(
        address,
        &advertised_fingerprint,
        &client,
        &token,
        12,
        "request_control_lease",
        serde_json::json!({
            "scope":{"kind":"full_session"},
            "duration_seconds":120,
            "allow_resumption":true,
            "full_debug":true
        }),
    );
    assert!(requested.contains("pending_local_approval"), "{requested}");
    let approved = server.command("approve");
    let lease = approved["lease_id"].as_u64().unwrap();
    let allowed = mcp_call(
        address,
        &advertised_fingerprint,
        &client,
        &token,
        13,
        "list_windows",
        serde_json::json!({"lease_id":lease}),
    );
    assert_eq!(mcp_error(&allowed), Some(false), "{allowed}");
    assert_eq!(
        response_json(&allowed)["result"]["structuredContent"],
        serde_json::json!([])
    );

    let disconnected = mcp_call(
        address,
        &advertised_fingerprint,
        &client,
        &token,
        14,
        "client_connection",
        serde_json::json!({"action":"disconnect"}),
    );
    assert_eq!(mcp_error(&disconnected), Some(false), "{disconnected}");
    let first_watch_response = first_watch.join().unwrap();
    assert!(first_watch_response.contains("Connection watch active"));
    let suspended = mcp_call(
        address,
        &advertised_fingerprint,
        &client,
        &token,
        15,
        "list_windows",
        serde_json::json!({"lease_id":lease}),
    );
    assert_eq!(mcp_error(&suspended), Some(true), "{suspended}");

    let (replacement, replacement_ready) = open_watch(
        address,
        advertised_fingerprint.clone(),
        client.clone(),
        token.clone(),
        16,
    );
    replacement_ready
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    let resumed = mcp_call(
        address,
        &advertised_fingerprint,
        &client,
        &token,
        17,
        "list_windows",
        serde_json::json!({"lease_id":lease}),
    );
    assert_eq!(mcp_error(&resumed), Some(false), "{resumed}");

    assert_eq!(
        server.command(&format!("revoke {client}"))["kind"],
        "revoked"
    );
    let replacement_response = replacement.join().unwrap();
    assert!(replacement_response.contains("Connection watch active"));
    let revoked = mcp_call(
        address,
        &advertised_fingerprint,
        &client,
        &token,
        18,
        "list_windows",
        serde_json::json!({"lease_id":lease}),
    );
    assert!(mcp_error(&revoked) == Some(true) || revoked.starts_with("HTTP/1.1 401"));
    let reconnect_revoked = mcp_call(
        address,
        &advertised_fingerprint,
        &client,
        &token,
        19,
        "client_connection",
        serde_json::json!({"action":"reconnect"}),
    );
    assert!(
        mcp_error(&reconnect_revoked) == Some(true)
            || reconnect_revoked.starts_with("HTTP/1.1 401")
    );

    let final_metrics = tls_request(
        address,
        &advertised_fingerprint,
        "GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(final_metrics.contains("nickel_mcp_active_connections 0"));
    assert!(final_metrics.contains("nickel_mcp_active_leases 0"));
    for secret in [
        &client,
        &token,
        &"private-network-acceptance-client".to_owned(),
    ] {
        assert!(!final_metrics.contains(secret));
    }

    server.stop();
    assert!(
        TcpListener::bind(address).is_ok(),
        "listener port was not released"
    );
}
