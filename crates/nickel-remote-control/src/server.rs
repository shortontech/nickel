use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::{Arc, Mutex, mpsc},
    thread,
};

use axum::{
    Json as AxumJson, Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::{Next, from_fn_with_state},
    response::Response,
    routing::post,
};
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::{
    Json, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    service::RequestContext,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::{Capability, ControlPlane};

const MCP_ADDRESS: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 42637);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WindowSummary {
    pub id: String,
    pub application_id: String,
    pub title: String,
    pub active: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub generation: u64,
}

pub trait DesktopAuthority: Send + Sync + 'static {
    fn list_windows(&self) -> Result<Vec<WindowSummary>, String>;
    fn focus_window(&self, id: &str, generation: u64) -> Result<WindowSummary, String>;
}

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("remote control must be enabled before starting its listener")]
    Disabled,
    #[error("cannot bind {MCP_ADDRESS}: {0}")]
    Bind(std::io::Error),
    #[error("remote-control listener failed to start")]
    Startup,
}

pub struct RemoteControlServer {
    cancellation: CancellationToken,
    worker: Option<thread::JoinHandle<()>>,
}

impl RemoteControlServer {
    pub fn start(
        control: Arc<Mutex<ControlPlane>>,
        desktop: Arc<dyn DesktopAuthority>,
    ) -> Result<Self, ServerError> {
        if !control.lock().unwrap().enabled() {
            return Err(ServerError::Disabled);
        }
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("nickel-remote-control".into())
            .spawn(move || {
                let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime construction");
                runtime.block_on(async move {
                    let listener = match tokio::net::TcpListener::bind(MCP_ADDRESS).await {
                        Ok(listener) => listener,
                        Err(error) => {
                            let _ = started_tx.send(Err(error));
                            return;
                        }
                    };
                    let service_control = control.clone();
                    let service_desktop = desktop.clone();
                    let config = StreamableHttpServerConfig::default()
                        .with_legacy_session_mode(false)
                        .with_json_response(true)
                        .with_sse_keep_alive(None)
                        .with_cancellation_token(worker_cancellation.clone());
                    let service: StreamableHttpService<McpHandler, LocalSessionManager> =
                        StreamableHttpService::new(
                            move || {
                                Ok(McpHandler::new(
                                    service_control.clone(),
                                    service_desktop.clone(),
                                ))
                            },
                            Default::default(),
                            config,
                        );
                    let mcp = Router::new()
                        .nest_service("/mcp", service)
                        .layer(from_fn_with_state(control.clone(), authorize_http));
                    let router = Router::new()
                        .route("/pair/exchange", post(pair_exchange))
                        .route("/pair/status", post(pair_status))
                        .with_state(control.clone())
                        .merge(mcp)
                        .layer(axum::extract::DefaultBodyLimit::max(4096));
                    let _ = started_tx.send(Ok(()));
                    let _ = axum::serve(listener, router)
                        .with_graceful_shutdown(worker_cancellation.cancelled_owned())
                        .await;
                });
            })
            .map_err(|_| ServerError::Startup)?;
        match started_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                cancellation,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(ServerError::Bind(error))
            }
            Err(_) => {
                let _ = worker.join();
                Err(ServerError::Startup)
            }
        }
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.cancellation.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairExchangeRequest {
    ceremony_id: String,
    qr_secret: Option<String>,
    short_code: Option<String>,
    label: String,
    requested: Vec<Capability>,
}

#[derive(Serialize)]
struct PairExchangeResponse {
    client_id: String,
    state: &'static str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairStatusRequest {
    client_id: String,
}

#[derive(Serialize)]
struct PairStatusResponse {
    state: &'static str,
    token: Option<String>,
    capabilities: Vec<Capability>,
}

#[derive(Serialize)]
struct PairApiError {
    error: String,
}

type PairApiResult<T> = Result<AxumJson<T>, (StatusCode, AxumJson<PairApiError>)>;

fn pairing_error(error: impl ToString) -> (StatusCode, AxumJson<PairApiError>) {
    (
        StatusCode::BAD_REQUEST,
        AxumJson(PairApiError {
            error: error.to_string().chars().take(160).collect(),
        }),
    )
}

async fn pair_exchange(
    State(control): State<Arc<Mutex<ControlPlane>>>,
    AxumJson(request): AxumJson<PairExchangeRequest>,
) -> PairApiResult<PairExchangeResponse> {
    if request.ceremony_id.len() != 32 || request.label.len() > 128 || request.requested.len() > 8 {
        return Err(pairing_error("pairing request exceeds bounds"));
    }
    if request.qr_secret.is_some() == request.short_code.is_some() {
        return Err(pairing_error(
            "provide exactly one QR secret or short pairing code",
        ));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut control = control.lock().unwrap();
    let pending = if let Some(secret) = request.qr_secret {
        control.exchange_qr_secret(
            &request.ceremony_id,
            &secret,
            &request.label,
            request.requested,
            now,
        )
    } else {
        control.exchange_short_code(
            &request.ceremony_id,
            request.short_code.as_deref().unwrap_or_default(),
            &request.label,
            request.requested,
            now,
        )
    }
    .map_err(pairing_error)?;
    Ok(AxumJson(PairExchangeResponse {
        client_id: pending.id,
        state: "pending_local_approval",
    }))
}

async fn pair_status(
    State(control): State<Arc<Mutex<ControlPlane>>>,
    AxumJson(request): AxumJson<PairStatusRequest>,
) -> PairApiResult<PairStatusResponse> {
    if request.client_id.len() != 32 {
        return Err(pairing_error("invalid client identity"));
    }
    let mut control = control.lock().unwrap();
    if control
        .pending_clients()
        .any(|client| client.id == request.client_id)
    {
        return Ok(AxumJson(PairStatusResponse {
            state: "pending_local_approval",
            token: None,
            capabilities: Vec::new(),
        }));
    }
    if let Some(issued) = control.claim_issued(&request.client_id) {
        return Ok(AxumJson(PairStatusResponse {
            state: "approved",
            token: Some(issued.token),
            capabilities: issued.capabilities,
        }));
    }
    if let Some(grant) = control
        .granted_clients()
        .find(|client| client.id == request.client_id)
    {
        return Ok(AxumJson(PairStatusResponse {
            state: "approved",
            token: None,
            capabilities: grant.capabilities,
        }));
    }
    Ok(AxumJson(PairStatusResponse {
        state: "denied_or_revoked",
        token: None,
        capabilities: Vec::new(),
    }))
}

impl Drop for RemoteControlServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Clone)]
struct AuthContext {
    client_id: String,
    token: String,
}

async fn authorize_http(
    State(control): State<Arc<Mutex<ControlPlane>>>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let client_id = request
        .headers()
        .get("x-nickel-client")
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 64)
        .ok_or(StatusCode::UNAUTHORIZED)?
        .to_owned();
    let token = request
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| value.len() == 64)
        .ok_or(StatusCode::UNAUTHORIZED)?
        .to_owned();
    if !control.lock().unwrap().authenticate(&client_id, &token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    request
        .extensions_mut()
        .insert(AuthContext { client_id, token });
    Ok(next.run(request).await)
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FocusWindowRequest {
    window_id: String,
    generation: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ControlStatus {
    enabled: bool,
    client_id: String,
    emergency_stop: &'static str,
}

#[derive(Clone)]
struct McpHandler {
    control: Arc<Mutex<ControlPlane>>,
    desktop: Arc<dyn DesktopAuthority>,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl McpHandler {
    fn new(control: Arc<Mutex<ControlPlane>>, desktop: Arc<dyn DesktopAuthority>) -> Self {
        Self {
            control,
            desktop,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Report the authorized Nickel remote-control session")]
    async fn get_control_status(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ControlStatus>, String> {
        let auth = self.authorize(&context, Capability::Observe)?;
        Ok(Json(ControlStatus {
            enabled: self.control.lock().unwrap().enabled(),
            client_id: auth.client_id,
            emergency_stop: "Press physical Left Control and Right Control together",
        }))
    }

    #[tool(description = "List ordinary windows visible to remote observation")]
    async fn list_windows(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<Vec<WindowSummary>>, String> {
        self.authorize(&context, Capability::Observe)?;
        self.desktop.list_windows().map(Json)
    }

    #[tool(description = "Focus a generation-bearing ordinary window")]
    async fn focus_window(
        &self,
        Parameters(request): Parameters<FocusWindowRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<WindowSummary>, String> {
        self.authorize(&context, Capability::WindowManagement)?;
        if request.window_id.len() > 128 {
            return Err("window identity exceeds limit".into());
        }
        self.desktop
            .focus_window(&request.window_id, request.generation)
            .map(Json)
    }

    fn authorize(
        &self,
        context: &RequestContext<RoleServer>,
        capability: Capability,
    ) -> Result<AuthContext, String> {
        let parts = context
            .extensions
            .get::<http::request::Parts>()
            .ok_or("HTTP authorization context is unavailable")?;
        let auth = parts
            .extensions
            .get::<AuthContext>()
            .ok_or("client is unauthorized")?;
        if !self
            .control
            .lock()
            .unwrap()
            .authorize(&auth.client_id, &auth.token, capability)
        {
            return Err("client lacks the required capability".into());
        }
        Ok(auth.clone())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("nickel-remote-control", env!("CARGO_PKG_VERSION")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    static PORT_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EmptyDesktop;

    impl DesktopAuthority for EmptyDesktop {
        fn list_windows(&self) -> Result<Vec<WindowSummary>, String> {
            Ok(Vec::new())
        }

        fn focus_window(&self, _id: &str, _generation: u64) -> Result<WindowSummary, String> {
            Err("not found".into())
        }
    }

    #[test]
    fn listener_is_absent_until_enabled_and_releases_the_fixed_port_on_stop() {
        let _port = PORT_TEST.lock().unwrap_or_else(|error| error.into_inner());
        let control = Arc::new(Mutex::new(ControlPlane::default()));
        assert!(matches!(
            RemoteControlServer::start(control.clone(), Arc::new(EmptyDesktop)),
            Err(ServerError::Disabled)
        ));
        control.lock().unwrap().set_enabled(true);
        let server = RemoteControlServer::start(control.clone(), Arc::new(EmptyDesktop)).unwrap();
        assert!(std::net::TcpStream::connect(MCP_ADDRESS).is_ok());
        assert!(matches!(
            RemoteControlServer::start(control, Arc::new(EmptyDesktop)),
            Err(ServerError::Bind(_))
        ));
        server.stop();
        assert!(std::net::TcpListener::bind(MCP_ADDRESS).is_ok());
    }

    fn http(request: &str) -> String {
        let mut stream = std::net::TcpStream::connect(MCP_ADDRESS).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    fn post_json(path: &str, body: &str) -> String {
        http(&format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ))
    }

    fn response_json(response: &str) -> serde_json::Value {
        serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap()
    }

    fn observe_grant(control: &mut ControlPlane) -> crate::IssuedCapability {
        let display = control.start_pairing(1).unwrap();
        let pending = control
            .exchange_short_code(
                &display.ceremony_id,
                &display.short_code,
                "MCP test",
                vec![Capability::Observe],
                2,
            )
            .unwrap();
        control
            .approve(
                &pending.id,
                crate::Approval::AllowOnce,
                vec![Capability::Observe],
            )
            .unwrap()
            .unwrap()
    }

    #[test]
    fn http_requires_a_grant_before_mcp_initialization() {
        let _port = PORT_TEST.lock().unwrap_or_else(|error| error.into_inner());
        let control = Arc::new(Mutex::new(ControlPlane::default()));
        control.lock().unwrap().set_enabled(true);
        let grant = observe_grant(&mut control.lock().unwrap());
        let server = RemoteControlServer::start(control, Arc::new(EmptyDesktop)).unwrap();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#;
        let unauthenticated = http(&format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ));
        assert!(
            unauthenticated.starts_with("HTTP/1.1 401"),
            "{unauthenticated:?}"
        );
        let authenticated = http(&format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nX-Nickel-Client: {}\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            grant.client_id,
            grant.token,
            body.len()
        ));
        assert!(authenticated.starts_with("HTTP/1.1 200"), "{authenticated}");
        assert!(authenticated.contains("nickel-remote-control"));
        server.stop();
    }

    #[test]
    fn pairing_exchange_remains_pending_until_local_approval_and_claims_once() {
        let _port = PORT_TEST.lock().unwrap_or_else(|error| error.into_inner());
        let control = Arc::new(Mutex::new(ControlPlane::default()));
        control.lock().unwrap().set_enabled(true);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let display = control.lock().unwrap().start_pairing(now).unwrap();
        let server = RemoteControlServer::start(control.clone(), Arc::new(EmptyDesktop)).unwrap();
        let exchange = post_json(
            "/pair/exchange",
            &serde_json::json!({
                "ceremony_id": display.ceremony_id,
                "short_code": display.short_code,
                "label": "Local phone test",
                "requested": ["observe", "window_management"]
            })
            .to_string(),
        );
        assert!(exchange.starts_with("HTTP/1.1 200"), "{exchange}");
        let exchange = response_json(&exchange);
        assert_eq!(exchange["state"], "pending_local_approval");
        let client_id = exchange["client_id"].as_str().unwrap();
        assert!(!control.lock().unwrap().authenticate(client_id, ""));

        control
            .lock()
            .unwrap()
            .approve(
                client_id,
                crate::Approval::AllowOnce,
                vec![Capability::Observe],
            )
            .unwrap();
        let status_body = serde_json::json!({ "client_id": client_id }).to_string();
        let approved = response_json(&post_json("/pair/status", &status_body));
        assert_eq!(approved["state"], "approved");
        let token = approved["token"].as_str().unwrap();
        assert_eq!(token.len(), 64);
        assert!(
            control
                .lock()
                .unwrap()
                .authorize(client_id, token, Capability::Observe)
        );

        let claimed_again = response_json(&post_json("/pair/status", &status_body));
        assert_eq!(claimed_again["state"], "approved");
        assert!(claimed_again["token"].is_null());
        server.stop();
    }
}
