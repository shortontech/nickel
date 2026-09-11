use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, mpsc},
    time::Duration,
};

use nickel_remote_control::{
    ClientConnectionAction, ClientConnectionPermit, DesktopAuthority, DesktopPermit,
    EffectiveState, RemoteAiControlSettings, RemoteControlRuntime, WindowSummary,
    diagnostics::DiagnosticSnapshot,
    keyboard::KeyboardAction,
    pointer::{PointerAction, PointerTarget},
    window_actions::{WindowAction, WindowOutcome},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

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
        _target: PointerTarget,
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
        .join("../nickel-remote-control/tests/fixtures")
        .join(name)
}

fn numeric_address() -> SocketAddr {
    let non_loopback = UdpSocket::bind("0.0.0.0:0")
        .and_then(|probe| {
            probe.connect("192.0.2.1:9")?;
            probe.local_addr().map(|address| address.ip())
        })
        .ok()
        .filter(|address| !address.is_loopback() && !address.is_unspecified());
    let ip = non_loopback.unwrap_or_else(|| "127.0.0.2".parse::<IpAddr>().unwrap());
    let reservation = TcpListener::bind((ip, 0)).unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    address
}

fn certificate() -> rustls::pki_types::CertificateDer<'static> {
    let bytes = std::fs::read(fixture("localhost-cert.pem")).unwrap();
    rustls_pemfile::certs(&mut bytes.as_slice())
        .next()
        .unwrap()
        .unwrap()
}

fn tls_post(address: SocketAddr, path: &str, body: &str) -> Value {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(certificate()).unwrap();
    let config = Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let connection =
        rustls::ClientConnection::new(config, "localhost".try_into().unwrap()).unwrap();
    let socket = TcpStream::connect_timeout(&address, Duration::from_secs(3)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut stream = rustls::StreamOwned::new(connection, socket);
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    let result = stream.read_to_string(&mut response);
    assert!(
        result.is_ok()
            || result
                .as_ref()
                .is_err_and(|error| error.kind() == std::io::ErrorKind::UnexpectedEof),
        "TLS response failed: {result:?}"
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap()
}

fn client_command(address: SocketAddr, client: &str, token: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nickel-mcp-client"));
    command
        .env("NICKEL_MCP_URL", format!("https://{address}/mcp"))
        .env("NICKEL_MCP_CLIENT", client)
        .env("NICKEL_MCP_TOKEN", token)
        .env_remove("NICKEL_MCP_CA_FILE")
        .env_remove("NICKEL_MCP_HOST_FINGERPRINT");
    command
}

#[test]
fn production_client_pins_numeric_production_server_identity() {
    let address = numeric_address();
    let cert_path = fixture("localhost-cert.pem");
    let key_path = fixture("localhost-key.pem");

    // SAFETY: this integration-test executable contains one test, so no other thread reads or
    // writes the process environment while the production server snapshots its configuration.
    unsafe {
        std::env::set_var("NICKEL_MCP_LISTEN_ADDR", address.to_string());
        std::env::set_var("NICKEL_MCP_TLS_CERT", &cert_path);
        std::env::set_var("NICKEL_MCP_TLS_KEY", &key_path);
    }
    let mut runtime = RemoteControlRuntime::default();
    runtime.apply(
        &RemoteAiControlSettings::default(),
        Arc::new(AcceptanceDesktop),
    );
    let status = runtime.status();
    assert_eq!(status.effective, EffectiveState::Enabled, "{status:?}");
    assert_eq!(status.endpoint, format!("https://{address}/mcp"));
    let advertised_fingerprint = status.host_fingerprint.clone().unwrap();
    assert_eq!(
        advertised_fingerprint,
        format!("{:x}", Sha256::digest(certificate().as_ref()))
    );

    let identity = tls_post(
        address,
        "/clients/connect",
        r#"{"label":"numeric-production-client"}"#,
    );
    let client = identity["client_id"].as_str().unwrap();
    let token = identity["token"].as_str().unwrap();

    let rejected = client_command(address, client, token)
        .env("NICKEL_MCP_CA_FILE", &cert_path)
        .output()
        .unwrap();
    assert!(
        !rejected.status.success(),
        "CA trust without an exact pin must retain numeric-IP SAN verification"
    );

    let mut child = client_command(address, client, token)
        .env("NICKEL_MCP_HOST_FINGERPRINT", &advertised_fingerprint)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        "{}",
        json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params":{
                "protocolVersion":"2025-06-18",
                "capabilities":{},
                "clientInfo":{"name":"numeric-production-acceptance","version":"1"}
            }
        })
    )
    .unwrap();
    stdin.flush().unwrap();

    let stdout = child.stdout.take().unwrap();
    let (line_tx, line_rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = line_tx.send(result);
    });
    let line = line_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("production client returned an initialize response")
        .unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], 1);
    assert!(response.get("result").is_some(), "{response}");

    drop(stdin);
    assert!(child.wait().unwrap().success());
}
