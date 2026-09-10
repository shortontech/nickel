#[cfg(test)]
use std::net::{Ipv4Addr, SocketAddrV4};
use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
};

use axum::{
    Json as AxumJson, Router,
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::{Next, from_fn_with_state},
    response::Response,
    routing::{get, post},
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
use http_body_util::BodyExt;

#[cfg(test)]
const MCP_ADDRESS: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 42637);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WindowSummary {
    pub id: String,
    /// Platform-reported application label for presentation.
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
    pub workspace: u64,
    /// Verified identity accepted by an application-scoped lease. Absent when
    /// process evidence cannot distinguish this application from a shared runtime.
    pub verified_application: Option<String>,
}

pub trait DesktopAuthority: Send + Sync + 'static {
    fn connection_cleanup_wake(&self) -> Option<crate::ConnectionCleanupWake> {
        None
    }

    fn client_connection(
        &self,
        _permit: crate::ClientConnectionPermit,
        _action: crate::ClientConnectionAction,
    ) -> Result<(), String> {
        Err("client connection transitions are unavailable on this backend".into())
    }

    fn read_desktop_events(
        &self,
        _permit: crate::DesktopPermit,
        _after: u64,
    ) -> Result<crate::desktop_events::DesktopEventObservation, String> {
        Err("desktop event history is unavailable on this backend".into())
    }

    fn launch_installed_application(
        &self,
        _permit: crate::DesktopPermit,
        _request: crate::diagnostics::LaunchApplicationRequest,
    ) -> Result<crate::diagnostics::LaunchApplicationOutcome, String> {
        Err("installed application launch is unavailable on this backend".into())
    }

    fn list_installed_applications(
        &self,
        _permit: crate::DesktopPermit,
    ) -> Result<crate::diagnostics::ApplicationInventory, String> {
        Err("installed application enumeration is unavailable on this backend".into())
    }

    fn list_outputs(
        &self,
        _permit: crate::DesktopPermit,
    ) -> Result<crate::diagnostics::OutputInventory, String> {
        Err("output enumeration is unavailable on this backend".into())
    }
    fn workspace_action(
        &self,
        _permit: crate::DesktopPermit,
        _action: crate::diagnostics::WorkspaceAction,
    ) -> Result<crate::diagnostics::WorkspaceOutcome, String> {
        Err("workspace operations are unavailable on this backend".into())
    }
    fn read_application_scale(
        &self,
        _permit: crate::DesktopPermit,
    ) -> Result<crate::application_scale::Snapshot, String> {
        Err("application scaling is unavailable on this backend".into())
    }
    fn application_scale_transaction(
        &self,
        _permit: crate::DesktopPermit,
        _transaction: crate::application_scale::Transaction,
    ) -> Result<crate::application_scale::TransactionOutcome, String> {
        Err("application scaling is unavailable on this backend".into())
    }
    fn read_appearance(
        &self,
        _permit: crate::DesktopPermit,
    ) -> Result<crate::appearance::Snapshot, String> {
        Err("appearance observation is unavailable on this backend".into())
    }
    fn appearance_transaction(
        &self,
        _permit: crate::DesktopPermit,
        _transaction: crate::appearance::Transaction,
    ) -> Result<crate::appearance::Snapshot, String> {
        Err("appearance transactions are unavailable on this backend".into())
    }
    fn read_launcher_favorites(
        &self,
        _permit: crate::DesktopPermit,
    ) -> Result<crate::launcher_favorites::Snapshot, String> {
        Err("launcher_favorites observation is unavailable on this backend".into())
    }
    fn launcher_favorites_transaction(
        &self,
        _permit: crate::DesktopPermit,
        _transaction: crate::launcher_favorites::Transaction,
    ) -> Result<crate::launcher_favorites::Snapshot, String> {
        Err("launcher_favorites transactions are unavailable on this backend".into())
    }
    fn read_wallpaper(
        &self,
        _permit: crate::DesktopPermit,
    ) -> Result<crate::wallpaper::Snapshot, String> {
        Err("wallpaper observation is unavailable on this backend".into())
    }
    fn wallpaper_transaction(
        &self,
        _permit: crate::DesktopPermit,
        _transaction: crate::wallpaper::Transaction,
    ) -> Result<crate::wallpaper::Snapshot, String> {
        Err("wallpaper transactions are unavailable on this backend".into())
    }
    fn read_terminal_presentation(
        &self,
        _permit: crate::DesktopPermit,
    ) -> Result<crate::terminal_presentation::Snapshot, String> {
        Err("terminal presentation observation is unavailable on this backend".into())
    }
    fn terminal_presentation_transaction(
        &self,
        _permit: crate::DesktopPermit,
        _transaction: crate::terminal_presentation::Transaction,
    ) -> Result<crate::terminal_presentation::Snapshot, String> {
        Err("terminal presentation transactions are unavailable on this backend".into())
    }
    fn shell_behavior_transaction(
        &self,
        _permit: crate::DesktopPermit,
        _transaction: nickel_session_protocol::ShellBehaviorTransaction,
    ) -> Result<crate::diagnostics::ShellBehaviorDiagnostic, String> {
        Err("shell behavior transactions are unavailable on this backend".into())
    }

    fn semantic_action(
        &self,
        _permit: crate::DesktopPermit,
        _request: crate::semantics::SemanticActionRequest,
    ) -> Result<bool, String> {
        Err("semantic actions are unavailable on this backend".into())
    }

    fn surface_semantic_action(
        &self,
        _permit: crate::DesktopPermit,
        _request: crate::semantics::SurfaceSemanticActionRequest,
    ) -> Result<crate::semantics::SurfaceSemanticActionOutcome, String> {
        Err("shell semantic actions are unavailable on this backend".into())
    }

    fn inspect_window(
        &self,
        _permit: crate::DesktopPermit,
        _id: &str,
        _generation: u64,
    ) -> Result<crate::semantics::SemanticSnapshot, String> {
        Err("window semantics are unavailable on this backend".into())
    }

    fn inspect_native_window(
        &self,
        _permit: crate::DesktopPermit,
        _id: &str,
        _generation: u64,
    ) -> Result<crate::native_semantics::NativeSemanticSnapshot, String> {
        Err("native accessibility association is unavailable on this backend".into())
    }

    fn inspect_native_application(
        &self,
        _permit: crate::DesktopPermit,
        _id: &str,
        _generation: u64,
    ) -> Result<crate::native_semantics::NativeSemanticSnapshot, String> {
        Err("native accessibility association is unavailable on this backend".into())
    }

    fn inspect_surface(
        &self,
        _permit: crate::DesktopPermit,
        _id: &str,
        _generation: u64,
    ) -> Result<crate::semantics::SurfaceSemanticSnapshot, String> {
        Err("surface semantics are unavailable on this backend".into())
    }

    fn diagnostic_action(
        &self,
        _permit: crate::DesktopPermit,
        _action: crate::diagnostics::DiagnosticAction,
    ) -> Result<crate::diagnostics::DiagnosticActionOutcome, String> {
        Err("diagnostic actions are unavailable on this backend".into())
    }
    fn list_surfaces(
        &self,
        _permit: crate::DesktopPermit,
    ) -> Result<Vec<crate::diagnostics::ShellSurfaceDiagnostic>, String> {
        Err("shell surface inventory is unavailable on this backend".into())
    }
    fn validate_surface_capture(
        &self,
        _permit: crate::DesktopPermit,
        _id: &str,
        _generation: u64,
    ) -> Result<(), String> {
        Err("surface capture validation is unavailable on this backend".into())
    }
    fn capture_surface(
        &self,
        _permit: crate::DesktopPermit,
        _id: &str,
        _generation: u64,
    ) -> Result<crate::capture::CapturedWindow, String> {
        Err("shell surface capture is unavailable on this backend".into())
    }
    fn validate_window_capture(
        &self,
        _permit: crate::DesktopPermit,
        _id: &str,
        _generation: u64,
    ) -> Result<(), String> {
        Err("capture validation is unavailable on this backend".into())
    }

    fn capture_window(
        &self,
        _permit: crate::DesktopPermit,
        _id: &str,
        _generation: u64,
    ) -> Result<crate::capture::CapturedWindow, String> {
        Err("window capture is unavailable on this backend".into())
    }

    fn keyboard_action(
        &self,
        permit: crate::DesktopPermit,
        id: &str,
        generation: u64,
        action: crate::keyboard::KeyboardAction,
    ) -> Result<(), String>;
    fn pointer_action(
        &self,
        permit: crate::DesktopPermit,
        id: &str,
        generation: u64,
        x: i32,
        y: i32,
        action: crate::pointer::PointerAction,
    ) -> Result<(), String>;
    fn diagnostic_snapshot(
        &self,
        permit: crate::DesktopPermit,
    ) -> Result<crate::diagnostics::DiagnosticSnapshot, String>;
    fn list_windows(&self, permit: crate::DesktopPermit) -> Result<Vec<WindowSummary>, String>;
    fn window_action(
        &self,
        permit: crate::DesktopPermit,
        id: &str,
        generation: u64,
        action: crate::window_actions::WindowAction,
    ) -> Result<crate::window_actions::WindowOutcome, String>;
}

async fn desktop_call<T: Send + 'static>(
    desktop: Arc<dyn DesktopAuthority>,
    effect: impl FnOnce(&dyn DesktopAuthority) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(move || effect(desktop.as_ref()))
        .await
        .map_err(|_| "desktop worker stopped".to_owned())?
}

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("remote control must be enabled before starting its listener")]
    Disabled,
    #[error("cannot bind MCP listener: {0}")]
    Bind(std::io::Error),
    #[error(transparent)]
    Configuration(#[from] crate::listener::ListenerError),
    #[error("cannot configure MCP HTTPS: {0}")]
    Tls(String),
    #[error("remote-control listener failed to start")]
    Startup,
}

pub struct RemoteControlServer {
    endpoint: String,
    pub(crate) host_fingerprint: Option<String>,
    pub(crate) environment_override: bool,
    cancellation: CancellationToken,
    worker: Option<thread::JoinHandle<()>>,
}

impl RemoteControlServer {
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub fn start(
        control: Arc<Mutex<ControlPlane>>,
        desktop: Arc<dyn DesktopAuthority>,
    ) -> Result<Self, ServerError> {
        Self::start_with_config(
            control,
            desktop,
            crate::listener::ListenerConfig::from_environment()?,
        )
    }

    pub fn start_with_config(
        control: Arc<Mutex<ControlPlane>>,
        desktop: Arc<dyn DesktopAuthority>,
        listener_config: crate::listener::ListenerConfig,
    ) -> Result<Self, ServerError> {
        if !control.lock().unwrap().enabled() {
            return Err(ServerError::Disabled);
        }
        let endpoint = listener_config.endpoint();
        let environment_override = listener_config.environment_override;
        let tls_material = listener_config
            .tls_material()
            .map_err(|error| ServerError::Tls(error.to_string()))?;
        let host_fingerprint = tls_material
            .as_ref()
            .map(|material| material.fingerprint.clone());
        let listener = std::net::TcpListener::bind(listener_config.address).map_err(|error| {
            ServerError::Bind(std::io::Error::new(
                error.kind(),
                format!("{}: {error}", listener_config.address),
            ))
        })?;
        listener.set_nonblocking(true).map_err(ServerError::Bind)?;
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("nickel-remote-control".into())
            .spawn(move || {
                let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime construction");
                runtime.block_on(async move {
                    let tls = if let Some(material) = tls_material {
                        let _ = rustls::crypto::ring::default_provider().install_default();
                        match axum_server::tls_rustls::RustlsConfig::from_pem(
                            material.certificate,
                            material.private_key,
                        )
                        .await
                        {
                            Ok(config) => Some(config),
                            Err(error) => {
                                let _ = started_tx.send(Err(ServerError::Tls(error.to_string())));
                                return;
                            }
                        }
                    } else {
                        None
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
                    let limits = crate::admission::AdmissionLimits::new();
                    let transport = PeerTransport { tls: tls.is_some() };
                    let router = Router::new()
                        .route("/metrics", get(operational_metrics))
                        .route("/clients/connect", post(connect_identity))
                        .route("/pair/exchange", post(pair_exchange))
                        .route("/pair/status", post(pair_status))
                        .with_state(control.clone())
                        .merge(mcp)
                        .layer(axum::extract::DefaultBodyLimit::max(
                            crate::admission::MAX_REQUEST_BYTES,
                        ))
                        .layer(from_fn_with_state(limits, bounded_http))
                        .layer(axum::Extension(transport));
                    if let Some(tls) = tls {
                        let server = match axum_server::from_tcp_rustls(listener, tls) {
                            Ok(server) => server,
                            Err(error) => {
                                let _ = started_tx.send(Err(ServerError::Tls(error.to_string())));
                                return;
                            }
                        };
                        let handle = axum_server::Handle::new();
                        let server = server.handle(handle.clone());
                        let _ = started_tx.send(Ok(()));
                        tokio::select! {
                            _ = server.serve(router.into_make_service_with_connect_info::<std::net::SocketAddr>()) => {},
                            _ = worker_cancellation.cancelled() => { handle.shutdown(); },
                        }
                    } else {
                        let listener = match tokio::net::TcpListener::from_std(listener) {
                            Ok(listener) => listener,
                            Err(error) => {
                                let _ = started_tx.send(Err(ServerError::Bind(error)));
                                return;
                            }
                        };
                        let _ = started_tx.send(Ok(()));
                        tokio::select! {
                            _ = axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()) => {},
                            _ = worker_cancellation.cancelled() => {},
                        }
                    }
                });
                // Revocation must not wait for filesystem/platform work inside
                // spawn_blocking. Those workers retain bounded admission and
                // must revalidate authority before any later desktop commit.
                runtime.shutdown_background();
            })
            .map_err(|_| ServerError::Startup)?;
        match started_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                endpoint,
                host_fingerprint,
                environment_override,
                cancellation,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
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

async fn bounded_http(
    State(limits): State<Arc<crate::admission::AdmissionLimits>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let admission = limits
        .acquire(None, std::time::Instant::now())
        .ok_or(StatusCode::TOO_MANY_REQUESTS)?;
    let response = tokio::time::timeout(crate::admission::REQUEST_TIMEOUT, async move {
        let (mut parts, body) = request.into_parts();
        // DefaultBodyLimit applies to extractors, not the nested MCP service.
        // Enforce the actual streamed bytes before either route parses JSON.
        let body = axum::body::to_bytes(body, crate::admission::MAX_REQUEST_BYTES)
            .await
            .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
        let capture = (parts.uri.path() == "/mcp" || parts.uri.path().starts_with("/mcp/"))
            .then(|| serde_json::from_slice::<serde_json::Value>(&body).ok())
            .flatten()
            .is_some_and(|request| {
                request["method"] == "tools/call"
                    && matches!(
                        request["params"]["name"].as_str(),
                        Some("capture_window" | "capture_surface")
                    )
            });
        let reservation = if capture {
            Some(CaptureReservation {
                _permit: Arc::new(
                    CAPTURE_GATE
                        .clone()
                        .try_acquire_owned()
                        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?,
                ),
            })
        } else {
            None
        };
        if let Some(reservation) = &reservation {
            parts.extensions.insert(reservation.clone());
        }
        parts.extensions.insert(limits);
        let response = next
            .run(Request::from_parts(parts, axum::body::Body::from(body)))
            .await;
        Ok::<_, StatusCode>(if let Some(reservation) = reservation {
            hold_response_budget(response, reservation)
        } else {
            response
        })
    })
    .await
    .map_err(|_| StatusCode::REQUEST_TIMEOUT)??;
    Ok(hold_response_budget(response, admission))
}

async fn authorize_http(
    State(control): State<Arc<Mutex<ControlPlane>>>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Protocol initialization and tool discovery do not convey desktop authority. Tool methods
    // requiring a client identity reject absent credentials themselves.
    if !request.headers().contains_key("x-nickel-client")
        && !request.headers().contains_key(http::header::AUTHORIZATION)
    {
        return Ok(next.run(request).await);
    }
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
    {
        let mut control = control.lock().unwrap();
        if !control.authenticate(&client_id, &token) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let peer = request
            .extensions()
            .get::<ConnectInfo<std::net::SocketAddr>>()
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let transport = request
            .extensions()
            .get::<PeerTransport>()
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        control.record_client_origin(
            &client_id,
            crate::ClientOrigin {
                address: peer.0.ip(),
                tls: transport.tls,
            },
        );
    }
    let limits = request
        .extensions()
        .get::<Arc<crate::admission::AdmissionLimits>>()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let admission = limits
        .acquire(Some(&client_id), std::time::Instant::now())
        .ok_or(StatusCode::TOO_MANY_REQUESTS)?;
    request
        .extensions_mut()
        .insert(AuthContext { client_id, token });
    Ok(hold_response_budget(next.run(request).await, admission))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectIdentityRequest {
    label: String,
}

#[derive(Serialize)]
struct ConnectIdentityResponse {
    client_id: String,
    token: String,
}

async fn operational_metrics(
    State(control): State<Arc<Mutex<ControlPlane>>>,
    axum::Extension(limits): axum::Extension<Arc<crate::admission::AdmissionLimits>>,
) -> ([(http::HeaderName, &'static str); 1], String) {
    let control = control.lock().unwrap();
    let now = std::time::Instant::now();
    let counts = control.lease_metrics(now, 0);
    let leases = counts.active_total;
    let pending = counts.pending_requests;
    let mut body = format!(
        "# TYPE nickel_mcp_active_leases gauge\nnickel_mcp_active_leases {leases}\n# TYPE nickel_mcp_permission_requests_pending gauge\nnickel_mcp_permission_requests_pending {pending}\n"
    );
    body.push_str("# TYPE nickel_mcp_active_leases_by_scope gauge\n");
    for (scope, count) in ["surface", "window", "application", "output", "full_session"]
        .iter()
        .zip(counts.active_by_scope)
    {
        use std::fmt::Write;
        let _ = writeln!(
            body,
            "nickel_mcp_active_leases_by_scope{{scope=\"{scope}\"}} {count}"
        );
    }
    body.push_str(&limits.metrics());
    body.push_str(&control.operation_metrics.exposition());
    body.push_str(&control.lease_requests().metrics());
    (
        [(
            http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}

#[derive(Clone, Copy)]
struct PeerTransport {
    tls: bool,
}

async fn connect_identity(
    State(control): State<Arc<Mutex<ControlPlane>>>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    axum::Extension(transport): axum::Extension<PeerTransport>,
    AxumJson(request): AxumJson<ConnectIdentityRequest>,
) -> PairApiResult<ConnectIdentityResponse> {
    let mut control = control.lock().unwrap();
    let identity = control
        .connect_identity(&request.label)
        .map_err(pairing_error)?;
    control.record_client_origin(
        &identity.client_id,
        crate::ClientOrigin {
            address: peer.ip(),
            tls: transport.tls,
        },
    );
    Ok(AxumJson(ConnectIdentityResponse {
        client_id: identity.client_id,
        token: identity.token,
    }))
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FocusWindowRequest {
    lease_id: u64,
    window_id: String,
    generation: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DiagnosticActionRequest {
    lease_id: u64,
    action: crate::diagnostics::DiagnosticAction,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WorkspaceActionRequest {
    lease_id: u64,
    action: crate::diagnostics::WorkspaceAction,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ShellBehaviorRequest {
    lease_id: u64,
    transaction: nickel_session_protocol::ShellBehaviorTransaction,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ApplicationScaleRequest {
    lease_id: u64,
    transaction: crate::application_scale::Transaction,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AppearanceRequest {
    lease_id: u64,
    transaction: crate::appearance::Transaction,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LauncherFavoritesRequest {
    lease_id: u64,
    transaction: crate::launcher_favorites::Transaction,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WallpaperRequest {
    lease_id: u64,
    transaction: crate::wallpaper::Transaction,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TerminalPresentationRequest {
    lease_id: u64,
    transaction: crate::terminal_presentation::Transaction,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WindowActionRequest {
    lease_id: u64,
    window_id: String,
    generation: u64,
    action: crate::window_actions::WindowAction,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PointerRequest {
    lease_id: u64,
    window_id: String,
    generation: u64,
    x: i32,
    y: i32,
    action: crate::pointer::PointerAction,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct KeyboardRequest {
    lease_id: u64,
    window_id: String,
    generation: u64,
    action: crate::keyboard::KeyboardAction,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LeaseOperation {
    lease_id: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
struct LeaseRequestOutcome {
    state: &'static str,
    changed: bool,
    retry_after_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ClientLeaseStatus {
    lease_id: u64,
    renewal_generation: u64,
    scope: crate::leases::ResourceScope,
    remaining_seconds: Option<u64>,
    suspended: bool,
    full_debug: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RequestLease {
    /// Renewal preserves scope and policy and still requires local approval.
    #[serde(default)]
    renewal: Option<nickel_session_protocol::RemoteLeaseRenewal>,
    scope: crate::leases::ResourceScope,
    /// Omit for until logout.
    duration_seconds: Option<u64>,
    #[serde(default)]
    allow_resumption: bool,
    #[serde(default)]
    full_debug: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ControlStatus {
    enabled: bool,
    client_id: String,
    emergency_stop: &'static str,
    connection_ready: bool,
    connection_watch_required: bool,
    connection_watch_seconds: u64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InspectWindowRequest {
    lease_id: u64,
    window_id: String,
    generation: u64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InspectSurfaceRequest {
    lease_id: u64,
    surface_id: String,
    generation: u64,
}

#[derive(Deserialize, JsonSchema)]
struct CaptureRequest {
    lease_id: u64,
    window_id: String,
    generation: u64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CaptureSurfaceRequest {
    lease_id: u64,
    surface_id: String,
    generation: u64,
}

static CAPTURE_GATE: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(1)));

#[derive(Clone)]
struct CaptureReservation {
    _permit: Arc<tokio::sync::OwnedSemaphorePermit>,
}

fn queue_capture_encoding(
    frame: crate::capture::CapturedWindow,
    permit: crate::DesktopPermit,
    reservation: CaptureReservation,
) -> tokio::task::JoinHandle<Result<crate::capture::WindowImage, String>> {
    tokio::task::spawn_blocking(move || {
        let _reservation = reservation;
        // A blocking task can wait in the runtime queue after its request was
        // abandoned. Check before PNG work, and discard cancelled results.
        permit.check_live()?;
        let image = frame.encode()?;
        permit.check_live()?;
        Ok(image)
    })
}

struct BudgetedBytes<T> {
    bytes: bytes::Bytes,
    _budget: Arc<T>,
}
impl<T> AsRef<[u8]> for BudgetedBytes<T> {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

fn hold_response_budget<T: Send + Sync + 'static>(response: Response, budget: T) -> Response {
    let budget = Arc::new(budget);
    let (mut parts, body) = response.into_parts();
    // Mapping changes ownership, never byte lengths. Preserve a known length
    // rather than losing it through MapFrame's deliberately unknown size hint.
    if let Some(length) = axum::body::HttpBody::size_hint(&body).exact()
        && length > 0
        && !parts.headers.contains_key(http::header::CONTENT_LENGTH)
        && !parts.headers.contains_key(http::header::TRANSFER_ENCODING)
    {
        parts.headers.insert(
            http::header::CONTENT_LENGTH,
            length.to_string().parse().expect("numeric content length"),
        );
    }
    let body = body.map_frame(move |frame| {
        frame.map_data(|bytes| {
            bytes::Bytes::from_owner(BudgetedBytes {
                bytes,
                _budget: budget.clone(),
            })
        })
    });
    Response::from_parts(parts, axum::body::Body::new(body))
}

#[derive(Clone)]
struct McpHandler {
    metrics: Arc<crate::operation_metrics::OperationMetrics>,
    control: Arc<Mutex<ControlPlane>>,
    desktop: Arc<dyn DesktopAuthority>,
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ClientConnectionRequest {
    action: crate::ClientConnectionAction,
}

#[tool_router(router = tool_router)]
impl McpHandler {
    fn new(control: Arc<Mutex<ControlPlane>>, desktop: Arc<dyn DesktopAuthority>) -> Self {
        let metrics = control.lock().unwrap().operation_metrics.clone();
        Self {
            metrics,
            control,
            desktop,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Disconnect, reconnect, or watch your authenticated identity. Watch requires a standard progress token and keeps this MCP request open for up to 60 seconds, sending progress readiness and keepalives. Open a replacement watch before the old one finishes; up to two may overlap per identity. Loss or completion of the last watch disconnects the identity. Watch never extends lease expiry. A ready watch is required before requesting or using desktop authority. The client transport maintains overlapping watches automatically during idle user time. Initialization, identity and status do not require a watch. Disconnect cancels pending approvals and all current operations, revokes non-resumable leases, and suspends resumable leases; the desktop owner releases held input before replying. Reconnect only restores eligible unexpired resumable leases, never local pauses or cancelled input. This affects every connection using your identity; it does not close the HTTP transport. Ordinary HTTP response completion is not a disconnect."
    )]
    async fn client_connection(
        &self,
        Parameters(request): Parameters<ClientConnectionRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<bool>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::ClientConnection, async {
                let auth = self.authenticated(&context)?;
                let mut permit = crate::ClientConnectionPermit::new(
                    self.control.clone(),
                    auth.client_id,
                    auth.token,
                    &self.metrics,
                    context.ct.clone(),
                )?;
                let progress = if matches!(request.action, crate::ClientConnectionAction::Watch) {
                    Some(
                        context
                            .meta
                            .get_progress_token()
                            .ok_or("watch requires a progress token")?,
                    )
                } else {
                    None
                };
                let watch = if progress.is_some() {
                    Some(
                        crate::connection_watch::ConnectionWatch::reserve(&mut permit)?
                            .with_owner_wake(self.desktop.connection_cleanup_wake()),
                    )
                } else {
                    None
                };
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.client_connection(permit, request.action)
                })
                .await?;
                if let (Some(mut watch), Some(progress)) = (watch, progress) {
                    let started = std::time::Instant::now();
                    let stream = async {
                        loop {
                            if !watch.is_live() || context.ct.is_cancelled() {
                                return Ok::<_, String>(());
                            }
                            tokio::time::timeout(
                                std::time::Duration::from_millis(500),
                                context.peer.notify_progress(
                                    rmcp::model::ProgressNotificationParam::new(
                                        progress.clone(),
                                        started.elapsed().as_secs_f64(),
                                    )
                                    .with_total(crate::connection_watch::WATCH_SECONDS as f64)
                                    .with_message("Connection watch active"),
                                ),
                            )
                            .await
                            .map_err(|_| "connection progress timed out")?
                            .map_err(|_| "connection progress stream closed")?;
                            tokio::select! {
                                _ = context.ct.cancelled() => return Ok(()),
                                _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                            }
                        }
                    };
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(crate::connection_watch::WATCH_SECONDS),
                        stream,
                    )
                    .await;
                    let reconcile = watch.close();
                    desktop_call(self.desktop.clone(), move |desktop| {
                        desktop
                            .client_connection(reconcile, crate::ClientConnectionAction::Disconnect)
                    })
                    .await?;
                    if let Ok(result) = result {
                        result?;
                    }
                }
                Ok(Json(true))
            })
            .await
    }

    #[tool(
        description = "Request local approval for a time-bounded resource control lease. Requires a ready client_connection watch maintained by the client transport."
    )]
    async fn request_control_lease(
        &self,
        Parameters(request): Parameters<RequestLease>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<LeaseRequestOutcome>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::RequestLease, async {
                let auth = self.authenticated(&context)?;
                let result = self.control.lock().unwrap().request_lease(
                    &auth.client_id,
                    &auth.token,
                    crate::lease_requests::LeaseRequest {
                        renewal: request.renewal,
                        scope: request.scope,
                        duration: request.duration_seconds.map(std::time::Duration::from_secs),
                        allow_resumption: request.allow_resumption,
                        full_debug: request.full_debug,
                    },
                    std::time::Instant::now(),
                );
                use crate::lease_requests::RequestError;
                let (state, changed, retry_after_ms) = match result {
                    Ok(changed) => ("pending_local_approval", changed, None),
                    Err(RequestError::Cooldown { retry_after }) => (
                        "cooldown",
                        false,
                        Some(retry_after.as_millis().min(u64::MAX as u128) as u64),
                    ),
                    Err(RequestError::Blocked) => ("blocked", false, None),
                    Err(RequestError::Capacity) => ("capacity", false, None),
                    Err(error) => return Err(error.to_string()),
                };
                Ok(Json(LeaseRequestOutcome {
                    state,
                    changed,
                    retry_after_ms,
                }))
            })
            .await
    }

    #[tool(description = "List this authenticated client's approved resource leases")]
    async fn list_control_leases(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<Vec<ClientLeaseStatus>>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::ListLeases, async {
                let auth = self.authenticated(&context)?;
                let control = self.control.lock().unwrap();
                let now = std::time::Instant::now();
                Ok(Json(
                    control
                        .leases()
                        .iter()
                        .filter(|lease| {
                            lease.client_identity == auth.client_id
                                && lease.expires_at.is_none_or(|deadline| now < deadline)
                        })
                        .map(|lease| ClientLeaseStatus {
                            lease_id: lease.id,
                            renewal_generation: lease.renewal_generation,
                            scope: lease.scope.clone(),
                            remaining_seconds: lease
                                .expires_at
                                .map(|deadline| deadline.saturating_duration_since(now).as_secs()),
                            suspended: lease.suspended,
                            full_debug: lease.full_debug,
                        })
                        .collect(),
                ))
            })
            .await
    }

    #[tool(
        description = "Read a bounded compositor snapshot using a Full Control & Debug Nickel lease"
    )]
    async fn diagnostic_snapshot(
        &self,
        Parameters(request): Parameters<LeaseOperation>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::diagnostics::DiagnosticSnapshot>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::Snapshot, async {
                let permit = self.permit(&context, request.lease_id)?;
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.diagnostic_snapshot(permit)
                })
                .await
                .map(Json)
            })
            .await
    }

    #[tool(
        description = "Run a typed production diagnostic action with a Full Control & Debug Nickel lease. repaint requests a complete redraw; refresh_scene also reconciles scene/output membership and removes dead scene entries, within the diagnostic window/output budget. Neither acknowledgement confirms presentation. identify_output shows the existing output number on one exact live output for up to 3 seconds, with cancellation/revocation cleanup; it never supplies custom text or confirms presentation. start_frame_trace enables a bounded frame CPU dispatch trace for 1..=60 seconds (256 records, one active trace globally); stop_frame_trace stops your lease's trace. Read records in diagnostic_snapshot.frame_trace. The trace category identifies the nested or DRM backend; timings do not confirm GPU completion or presentation."
    )]
    async fn diagnostic_action(
        &self,
        Parameters(request): Parameters<DiagnosticActionRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::diagnostics::DiagnosticActionOutcome>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::DiagnosticAction, async {
                request.action.validate()?;
                let permit = self.permit(&context, request.lease_id)?;
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.diagnostic_action(permit, request.action)
                })
                .await
                .map(Json)
            })
            .await
    }

    #[tool(
        description = "Read application-scale compatibility policy, generation, and bounded GTK/Qt availability, actual values, ownership and pending intent. Requires Full Control & Debug Nickel. Pending intents are uncertain accepted setters, not confirmed changes. No raw toolkit files or commands are exposed."
    )]
    async fn read_application_scale(
        &self,
        Parameters(request): Parameters<LeaseOperation>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::application_scale::Snapshot>, String> {
        self.metrics
            .measure(
                crate::operation_metrics::Method::ReadApplicationScale,
                async {
                    let permit = self.permit(&context, request.lease_id)?;
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        desktop_call(self.desktop.clone(), move |desktop| {
                            desktop.read_application_scale(permit)
                        }),
                    )
                    .await
                    .map_err(|_| "application scale observation timed out")?
                    .map(Json)
                },
            )
            .await
    }
    #[tool(
        description = "Change application scaling with fresh generation/prior policy. Requires Full Control & Debug Nickel and idle shared input at each commit. Includes ownership-aware GTK/Qt settings and launch policy, preserving original baseline across repeated writes. Durable intents precede external setters; uncertain/partial outcomes and restart requirements are explicit. Revocation prevents later commands but an already accepted native setter may finish. Read current state before retrying; never overwrite an external change during reset."
    )]
    async fn application_scale_transaction(
        &self,
        Parameters(request): Parameters<ApplicationScaleRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::application_scale::TransactionOutcome>, String> {
        self.metrics.measure(crate::operation_metrics::Method::ApplicationScaleTransaction,async{let permit=self.permit(&context,request.lease_id)?;tokio::time::timeout(std::time::Duration::from_secs(2),desktop_call(self.desktop.clone(),move|desktop|desktop.application_scale_transaction(permit,request.transaction))).await.map_err(|_|"application scale result uncertain; read current state before retrying")?.map(Json)}).await
    }

    #[tool(
        description = "Read typed Nickel shell appearance preferences and fresh configuration generation. Requires Full Control & Debug Nickel. Returns configured values, not presented pixels or OS-wide GNOME color-scheme state. Missing config uses production defaults; unreadable or oversized config fails closed."
    )]
    async fn read_appearance(
        &self,
        Parameters(request): Parameters<LeaseOperation>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::appearance::Snapshot>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::ReadAppearance, async {
                let permit = self.permit(&context, request.lease_id)?;
                tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    desktop_call(self.desktop.clone(), move |desktop| {
                        desktop.read_appearance(permit)
                    }),
                )
                .await
                .map_err(|_| "appearance observation timed out")?
                .map(Json)
            })
            .await
    }
    #[tool(
        description = "Change Nickel shell theme, accent hue/intensity, transparency preference and animation preference through a typed compare-and-set transaction. Requires Full Control & Debug Nickel and idle shared input. Supply generation and complete prior values from read_appearance. Uses the production shell settings writer and applies its committed shell theme; excludes OS-wide color-scheme publication and protected remote-control settings. Returned configuration is not presentation confirmation. Inspect current state before retrying an uncertain result."
    )]
    async fn appearance_transaction(
        &self,
        Parameters(request): Parameters<AppearanceRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::appearance::Snapshot>, String> {
        self.metrics
            .measure(
                crate::operation_metrics::Method::AppearanceTransaction,
                async {
                    let permit = self.permit(&context, request.lease_id)?;
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        desktop_call(self.desktop.clone(), move |desktop| {
                            desktop.appearance_transaction(permit, request.transaction)
                        }),
                    )
                    .await
                    .map_err(
                        |_| "appearance result uncertain; read current appearance before retrying",
                    )?
                    .map(Json)
                },
            )
            .await
    }

    #[tool(
        description = "Read bounded installed-application launcher favorites and configuration/catalog generations. Requires Full Control & Debug Nickel. Excludes recent history and unavailable stored IDs. runtime_applied refers to production model state, not presented pixels."
    )]
    async fn read_launcher_favorites(
        &self,
        Parameters(request): Parameters<LeaseOperation>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::launcher_favorites::Snapshot>, String> {
        self.metrics
            .measure(
                crate::operation_metrics::Method::ReadLauncherFavorites,
                async {
                    let permit = self.permit(&context, request.lease_id)?;
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        desktop_call(self.desktop.clone(), move |desktop| {
                            desktop.read_launcher_favorites(permit)
                        }),
                    )
                    .await
                    .map_err(|_| "launcher_favorites observation timed out")?
                    .map(Json)
                },
            )
            .await
    }
    #[tool(
        description = "Add, remove, or reorder installed-application launcher favorites through the production checked preferences writer. Requires Full Control & Debug Nickel, fresh generations and prior favorites, and idle shared input/local preference writes. Preserves recents and unavailable favorites without exposing them. No arbitrary paths, commands or history writes. Inspect current state before retrying an uncertain result."
    )]
    async fn launcher_favorites_transaction(
        &self,
        Parameters(request): Parameters<LauncherFavoritesRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::launcher_favorites::Snapshot>, String> {
        self.metrics
            .measure(
                crate::operation_metrics::Method::LauncherFavoritesTransaction,
                async {
                    let permit = self.permit(&context, request.lease_id)?;
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        desktop_call(self.desktop.clone(), move |desktop| {
                            desktop.launcher_favorites_transaction(permit, request.transaction)
                        }),
                    )
                    .await
                    .map_err(
                        |_| "launcher_favorites result uncertain; read current launcher_favorites before retrying",
                    )?
                    .map(Json)
                },
            )
            .await
    }

    #[tool(
        description = "Read typed Nickel wallpaper layout state. Requires Full Control & Debug Nickel. Reports only whether a custom image is configured; image paths and contents are excluded. The result is configuration state, not presented pixels."
    )]
    async fn read_wallpaper(
        &self,
        Parameters(request): Parameters<LeaseOperation>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::wallpaper::Snapshot>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::ReadWallpaper, async {
                let permit = self.permit(&context, request.lease_id)?;
                tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    desktop_call(self.desktop.clone(), move |desktop| {
                        desktop.read_wallpaper(permit)
                    }),
                )
                .await
                .map_err(|_| "wallpaper observation timed out")?
                .map(Json)
            })
            .await
    }

    #[tool(
        description = "Change the typed wallpaper position or reset the custom image through the checked production writer. Requires Full Control & Debug Nickel, a fresh generation/prior snapshot, and idle shared input. Paths and image contents are never accepted. A committed result requests a live shell reload but does not confirm presented pixels."
    )]
    async fn wallpaper_transaction(
        &self,
        Parameters(request): Parameters<WallpaperRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::wallpaper::Snapshot>, String> {
        self.metrics
            .measure(
                crate::operation_metrics::Method::WallpaperTransaction,
                async {
                    let permit = self.permit(&context, request.lease_id)?;
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        desktop_call(self.desktop.clone(), move |desktop| {
                            desktop.wallpaper_transaction(permit, request.transaction)
                        }),
                    )
                    .await
                    .map_err(
                        |_| "wallpaper result uncertain; read current wallpaper before retrying",
                    )?
                    .map(Json)
                },
            )
            .await
    }

    #[tool(
        description = "Read typed terminal presentation preferences. Requires Full Control & Debug Nickel. Executable and working-directory values are excluded except for presence booleans. Values apply when production creates a new terminal; existing terminals are unchanged."
    )]
    async fn read_terminal_presentation(
        &self,
        Parameters(request): Parameters<LeaseOperation>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::terminal_presentation::Snapshot>, String> {
        self.metrics
            .measure(
                crate::operation_metrics::Method::ReadTerminalPresentation,
                async {
                    let permit = self.permit(&context, request.lease_id)?;
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        desktop_call(self.desktop.clone(), move |desktop| {
                            desktop.read_terminal_presentation(permit)
                        }),
                    )
                    .await
                    .map_err(|_| "terminal presentation observation timed out")?
                    .map(Json)
                },
            )
            .await
    }

    #[tool(
        description = "Change terminal font, size, scrollback, cursor, colors and close-on-success through the checked production writer. Requires Full Control & Debug Nickel, fresh generation/prior values and idle shared input. Executable and working-directory values cannot be read or changed and are preserved. Changes apply to new terminals only."
    )]
    async fn terminal_presentation_transaction(
        &self,
        Parameters(request): Parameters<TerminalPresentationRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::terminal_presentation::Snapshot>, String> {
        self.metrics
            .measure(
                crate::operation_metrics::Method::TerminalPresentationTransaction,
                async {
                    let permit = self.permit(&context, request.lease_id)?;
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        desktop_call(self.desktop.clone(), move |desktop| {
                            desktop.terminal_presentation_transaction(permit, request.transaction)
                        }),
                    )
                    .await
                    .map_err(|_| {
                        "terminal presentation result uncertain; read current state before retrying"
                    })?
                    .map(Json)
                },
            )
            .await
    }

    #[tool(
        description = "Change shell bar display scope, bar window scope or desktop count through the production compare-and-set transaction. Requires Full Control & Debug Nickel and idle shared input. Read current values and topology_generation from diagnostic_snapshot.shell_behavior. The returned state confirms the committed configuration, not presentation. If the result is uncertain, inspect current state before making another transaction; do not retry blindly."
    )]
    async fn shell_behavior_transaction(
        &self,
        Parameters(request): Parameters<ShellBehaviorRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::diagnostics::ShellBehaviorDiagnostic>, String> {
        self.metrics
            .measure(
                crate::operation_metrics::Method::SettingsTransaction,
                async {
                    let permit = self.permit(&context, request.lease_id)?;
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        desktop_call(self.desktop.clone(), move |desktop| {
                            desktop.shell_behavior_transaction(permit, request.transaction)
                        }),
                    )
                    .await
                    .map_err(|_| "settings transaction result uncertain: preparation or desktop authority timed out; inspect current state before retrying".to_owned())?
                    .map(Json)
                },
            )
            .await
    }

    #[tool(
        description = "List, create, switch or remove session workspaces through production policy. Requires a full-session control lease; full-debug is not required. Mutations reject conflicting input and changes to protected windows. These change live session workspaces, not the configured startup count. Outcomes describe committed state, not presented pixels; inspect state before retrying an uncertain result."
    )]
    async fn workspace_action(
        &self,
        Parameters(request): Parameters<WorkspaceActionRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::diagnostics::WorkspaceOutcome>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::WorkspaceAction, async {
                let permit = self.permit(&context, request.lease_id)?;
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.workspace_action(permit, request.action)
                })
                .await
                .map(Json)
            })
            .await
    }

    #[tool(
        description = "List current outputs covered by this lease, including exact names and generations, logical geometry, work area and scale. Full-session leases see all ordinary outputs; an output lease sees only its matching generation. Window, surface and application leases do not authorize whole-output observation. No full-debug approval is required."
    )]
    async fn list_outputs(
        &self,
        Parameters(request): Parameters<LeaseOperation>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::diagnostics::OutputInventory>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::ListOutputs, async {
                let permit = self.permit(&context, request.lease_id)?;
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.list_outputs(permit)
                })
                .await
                .map(Json)
            })
            .await
    }

    #[tool(
        description = "List the bounded installed-application catalog covered by an application or full-session lease. Application leases see only targets matching verified native executable identity. Returns catalog IDs, display names and optional verified_application identities, without launch arguments. A verified_application may be used to request an application lease before a window exists; catalog IDs cannot. Identity is revalidated at launch and for each resulting window. Enumeration grants no control authority. Full-debug approval is not required."
    )]
    async fn list_installed_applications(
        &self,
        Parameters(request): Parameters<LeaseOperation>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::diagnostics::ApplicationInventory>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::ListApplications, async {
                let permit = self.permit(&context, request.lease_id)?;
                tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    desktop_call(self.desktop.clone(), move |desktop| {
                        desktop.list_installed_applications(permit)
                    }),
                )
                .await
                .map_err(|_| {
                    "application inventory timed out; preparation may still be busy".to_owned()
                })?
                .map(Json)
            })
            .await
    }

    #[tool(
        description = "Launch an installed application by exact catalog ID and generation using a full-session or verified application lease. No arbitrary command or arguments are accepted. Acknowledges process spawn only; resulting windows require independent observation and lease checks. An uncertain result must be inspected before retrying. Application-scoped launch pins a matching native executable; shared runtimes and interpreter scripts require stronger identity evidence and are unavailable. Output-scoped launch is not yet available."
    )]
    async fn launch_installed_application(
        &self,
        Parameters(request): Parameters<crate::diagnostics::LaunchApplicationRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::diagnostics::LaunchApplicationOutcome>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::LaunchApplication, async {
                if request.application_id.is_empty()
                    || request.application_id.len() > 512
                    || request.catalog_generation == 0
                {
                    return Err("invalid application launch target".to_owned());
                }
                let permit = self.permit(&context, request.lease_id)?;
                tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    desktop_call(self.desktop.clone(), move |desktop| {
                        desktop.launch_installed_application(permit, request)
                    }),
                ).await.map_err(|_| "launch result uncertain: preparation or desktop authority timed out; inspect windows before retrying".to_owned())?
                .map(Json)
            })
            .await
    }

    #[tool(
        description = "Read bounded payload-free compositor events after an event generation. Requires full-debug authority. history_gap reports evicted unread events; generation is the next cursor. A future cursor is rejected. Currently records ordinary native-window identity verification, retirement, focus assignments and output membership only. This tool polls; MCP 2026 subscriptions/listen can watch nickel://desktop-events/{lease_id} for up to 60 seconds, with one stream per client and four globally. Resource reads require the same full-debug lease."
    )]
    async fn read_desktop_events(
        &self,
        Parameters(request): Parameters<crate::desktop_events::ReadDesktopEvents>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::desktop_events::DesktopEventObservation>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::ReadDesktopEvents, async {
                let permit = self.permit(&context, request.lease_id)?;
                permit.with_debug(false, || Ok(()))?;
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.read_desktop_events(permit, request.after_generation)
                })
                .await
                .map(Json)
            })
            .await
    }

    #[tool(description = "Report the authorized Nickel remote-control session")]
    async fn get_control_status(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ControlStatus>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::Status, async {
                let auth = self.authenticated(&context)?;
                let control = self.control.lock().unwrap();
                Ok(Json(ControlStatus {
                    enabled: control.enabled(),
                    connection_ready: control
                        .has_ready_connection(&auth.client_id, std::time::Instant::now()),
                    connection_watch_required: true,
                    connection_watch_seconds: crate::connection_watch::WATCH_SECONDS,
                    client_id: auth.client_id,
                    emergency_stop: "Press physical Left Control and Right Control together",
                }))
            })
            .await
    }

    #[tool(description = "List ordinary windows visible to remote observation")]
    async fn list_windows(
        &self,
        Parameters(request): Parameters<LeaseOperation>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<Vec<WindowSummary>>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::ListWindows, async {
                let permit = self.permit(&context, request.lease_id)?;
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.list_windows(permit)
                })
                .await
                .map(Json)
            })
            .await
    }

    #[tool(
        description = "Inspect bounded current semantic nodes of an authorized hosted application window. Protected views are denied. External application metadata uses the separate native inspection tools. Node ordinals are valid only for the returned surface and tree generation."
    )]
    async fn inspect_window(
        &self,
        Parameters(request): Parameters<InspectWindowRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::semantics::SemanticSnapshot>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::Semantics, async {
                let permit = self.permit(&context, request.lease_id)?;
                if request.window_id.len() > 128 {
                    return Err("window identity exceeds limit".into());
                }
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.inspect_window(permit, &request.window_id, request.generation)
                })
                .await
                .map(Json)
            })
            .await
    }

    #[tool(
        description = "Inspect bounded read-only AT-SPI role/state/geometry of an authorized native GTK Wayland window with an exact surface association. Names, descriptions, text, values and actions are unavailable. XWayland and clients without an authenticated association are unavailable. Results are non-atomic and cannot be used as action handles."
    )]
    async fn inspect_native_window(
        &self,
        Parameters(request): Parameters<InspectWindowRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::native_semantics::NativeSemanticSnapshot>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::NativeSemantics, async {
                let permit = self.permit(&context, request.lease_id)?;
                if request.window_id.len() > 128 {
                    return Err("window identity exceeds limit".into());
                }
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.inspect_native_window(permit, &request.window_id, request.generation)
                })
                .await
                .map(Json)
            })
            .await
    }

    #[tool(
        description = "Inspect bounded AT-SPI role/state metadata from one authenticated accessibility connection of an authorized native window's owning process. Requires application or full-session scope; window/output/surface leases are denied. The window is an OS identity anchor, not a claimed accessibility-window mapping. Names, descriptions, text, values, geometry and actions are unavailable; results are non-atomic and partial."
    )]
    async fn inspect_native_application(
        &self,
        Parameters(request): Parameters<InspectWindowRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::native_semantics::NativeSemanticSnapshot>, String> {
        self.metrics
            .measure(
                crate::operation_metrics::Method::NativeApplicationSemantics,
                async {
                    let permit = self.permit(&context, request.lease_id)?;
                    if request.window_id.len() > 128 {
                        return Err("window identity exceeds limit".into());
                    }
                    desktop_call(self.desktop.clone(), move |desktop| {
                        desktop.inspect_native_application(
                            permit,
                            &request.window_id,
                            request.generation,
                        )
                    })
                    .await
                    .map(Json)
                },
            )
            .await
    }

    #[tool(
        description = "Inspect bounded current semantic nodes of an authorized ordinary Nickel shell surface. Protected views are denied. External application metadata uses the separate native inspection tools. Node ordinals are valid only for the returned surface and tree generation."
    )]
    async fn inspect_surface(
        &self,
        Parameters(request): Parameters<InspectSurfaceRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::semantics::SurfaceSemanticSnapshot>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::SurfaceSemantics, async {
                let permit = self.permit(&context, request.lease_id)?;
                if request.surface_id.len() > 128 {
                    return Err("surface identity exceeds limit".into());
                }
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.inspect_surface(permit, &request.surface_id, request.generation)
                })
                .await
                .map(Json)
            })
            .await
    }

    #[tool(
        description = "Apply an advertised semantic action to an authorized hosted application node from inspect_window. Requires the exact window, surface and tree generations. Returns whether application state changed, not presentation confirmation. Reinspect before another action; never automatically retry an action whose result is uncertain. External native accessibility actions are unavailable."
    )]
    async fn semantic_action(
        &self,
        Parameters(request): Parameters<crate::semantics::SemanticActionRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<bool>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::SemanticAction, async {
                let permit = self.permit(&context, request.lease_id)?;
                request.validate().map_err(str::to_owned)?;
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.semantic_action(permit, request)
                })
                .await
                .map(Json)
            })
            .await
    }

    #[tool(
        description = "Apply an advertised semantic action to an authorized ordinary Nickel shell node from inspect_surface. Requires exact surface and tree generations and idle shared input. Returns changed plus typed completion and partial-effect status; UI changes are not presentation confirmation. Reinspect before another action; never automatically retry an uncertain action. Protected surfaces and external native accessibility are unavailable."
    )]
    async fn surface_semantic_action(
        &self,
        Parameters(request): Parameters<crate::semantics::SurfaceSemanticActionRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::semantics::SurfaceSemanticActionOutcome>, String> {
        self.metrics
            .measure(
                crate::operation_metrics::Method::SurfaceSemanticAction,
                async {
                    let permit = self.permit(&context, request.lease_id)?;
                    request.validate().map_err(str::to_owned)?;
                    desktop_call(self.desktop.clone(), move |desktop| {
                        desktop.surface_semantic_action(permit, request)
                    })
                    .await
                    .map(Json)
                },
            )
            .await
    }

    #[tool(description = "Focus a generation-bearing ordinary window")]
    async fn focus_window(
        &self,
        Parameters(request): Parameters<FocusWindowRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<WindowSummary>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::FocusWindow, async {
                let permit = self.permit(&context, request.lease_id)?;
                if request.window_id.len() > 128 {
                    return Err("window identity exceeds limit".into());
                }
                let outcome = desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.window_action(
                        permit,
                        &request.window_id,
                        request.generation,
                        crate::window_actions::WindowAction::Activate,
                    )
                })
                .await?;
                if !outcome.confirmed {
                    return Err("window did not confirm focus".into());
                }
                outcome
                    .window
                    .map(Json)
                    .ok_or_else(|| "window disappeared".into())
            })
            .await
    }

    #[tool(
        description = "Capture a leased native window's client area as PNG, with monotonic frame timestamps; excludes shell overlays and returns no cached thumbnail"
    )]
    async fn capture_window(
        &self,
        Parameters(request): Parameters<CaptureRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CallToolResult, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::Capture, async {
                let reservation = context
                    .extensions
                    .get::<http::request::Parts>()
                    .and_then(|parts| parts.extensions.get::<CaptureReservation>())
                    .cloned()
                    .ok_or("capture admission is unavailable")?;
                if request.window_id.len() > 128 {
                    return Err("window identity exceeds limit".into());
                }
                let permit = self.permit(&context, request.lease_id)?;
                let final_permit = permit.clone();
                let id = request.window_id.clone();
                let expected_generation = request.generation;
                let frame = desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.capture_window(permit, &request.window_id, request.generation)
                })
                .await?;
                let image = queue_capture_encoding(frame, final_permit.clone(), reservation)
                    .await
                    .map_err(|_| "capture encoder stopped")??;
                // Encoding is off the compositor thread. Resolve current protected/resource
                // membership again before releasing the image, preserving the original
                // permit's cancellation generation and request deadline.
                if image.window_id != id || image.generation != expected_generation {
                    return Err("capture returned a different resource identity".into());
                }
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.validate_window_capture(final_permit, &id, expected_generation)
                })
                .await?;
                Ok(image.into_mcp())
            })
            .await
    }

    #[tool(
        description = "Capture one visible ordinary shell surface under a current surface, output, or full-session lease; protected chrome and transients are excluded"
    )]
    async fn capture_surface(
        &self,
        Parameters(request): Parameters<CaptureSurfaceRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CallToolResult, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::CaptureSurface, async {
                let reservation = context
                    .extensions
                    .get::<http::request::Parts>()
                    .and_then(|parts| parts.extensions.get::<CaptureReservation>())
                    .cloned()
                    .ok_or("capture admission is unavailable")?;
                if request.surface_id.len() > 128 {
                    return Err("window identity exceeds limit".into());
                }
                let permit = self.permit(&context, request.lease_id)?;
                let final_permit = permit.clone();
                let id = request.surface_id.clone();
                let expected_generation = request.generation;
                let frame = desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.capture_surface(permit, &request.surface_id, request.generation)
                })
                .await?;
                let image = queue_capture_encoding(frame, final_permit.clone(), reservation)
                    .await
                    .map_err(|_| "capture encoder stopped")??;
                // Encoding is off the compositor thread. Resolve current protected/resource
                // membership again before releasing the image, preserving the original
                // permit's cancellation generation and request deadline.
                if image.window_id != id || image.generation != expected_generation {
                    return Err("capture returned a different resource identity".into());
                }
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.validate_surface_capture(final_permit, &id, expected_generation)
                })
                .await?;
                Ok(image.into_surface_mcp())
            })
            .await
    }

    #[tool(
        description = "List visible ordinary shell surfaces authorized by this lease, with generation-correlated identities"
    )]
    async fn list_surfaces(
        &self,
        Parameters(request): Parameters<LeaseOperation>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<Vec<crate::diagnostics::ShellSurfaceDiagnostic>>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::ListSurfaces, async {
                let permit = self.permit(&context, request.lease_id)?;
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.list_surfaces(permit)
                })
                .await
                .map(Json)
            })
            .await
    }

    #[tool(description = "Manage a leased window and report requested versus confirmed state")]
    async fn window_action(
        &self,
        Parameters(request): Parameters<WindowActionRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::window_actions::WindowOutcome>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::WindowAction, async {
                request.action.validate()?;
                if request.window_id.len() > 128 {
                    return Err("window identity exceeds limit".into());
                }
                let permit = self.permit(&context, request.lease_id)?;
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.window_action(
                        permit,
                        &request.window_id,
                        request.generation,
                        request.action,
                    )
                })
                .await
                .map(Json)
            })
            .await
    }

    #[tool(
        description = "Move, click, scroll, or drag at a leased window's client-relative coordinates. DragStart holds one button; DragMove continues and DragEnd releases. DragCancel releases without moving. Continue within 30 seconds; local input or lost authority cancels the gesture."
    )]
    async fn pointer_action(
        &self,
        Parameters(request): Parameters<PointerRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<bool>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::Pointer, async {
                request.action.validate()?;
                if request.window_id.len() > 128 {
                    return Err("window identity exceeds limit".into());
                }
                let permit = self.permit(&context, request.lease_id)?;
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.pointer_action(
                        permit,
                        &request.window_id,
                        request.generation,
                        request.x,
                        request.y,
                        request.action,
                    )
                })
                .await?;
                Ok(Json(true))
            })
            .await
    }

    #[tool(
        description = "Deliver text, a complete chord, or owned held keys to a leased focused window. hold_start presses a chord; hold_keep_alive refreshes its 30-second idle deadline; hold_end or hold_cancel releases it. Focus loss, local input, or lost authority cancels held keys. Success reports dispatch, not application processing. A failed request may have delivered partial input; inspect the application before retrying."
    )]
    async fn keyboard_action(
        &self,
        Parameters(request): Parameters<KeyboardRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<bool>, String> {
        self.metrics
            .measure(crate::operation_metrics::Method::Keyboard, async {
                request.action.validate()?;
                if request.window_id.len() > 128 {
                    return Err("window identity exceeds limit".into());
                }
                let permit = self.permit(&context, request.lease_id)?;
                desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.keyboard_action(
                        permit,
                        &request.window_id,
                        request.generation,
                        request.action,
                    )
                })
                .await?;
                Ok(Json(true))
            })
            .await
    }

    fn authenticated(&self, context: &RequestContext<RoleServer>) -> Result<AuthContext, String> {
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
            .authenticate(&auth.client_id, &auth.token)
        {
            return Err("client is unauthorized".into());
        }
        Ok(auth.clone())
    }

    fn permit(
        &self,
        context: &RequestContext<RoleServer>,
        lease_id: u64,
    ) -> Result<crate::DesktopPermit, String> {
        let auth = self.authenticated(context)?;
        let mut permit =
            crate::DesktopPermit::new(self.control.clone(), auth.client_id, auth.token, lease_id);
        permit.admission = context
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| {
                parts
                    .extensions
                    .get::<Arc<crate::admission::AdmissionLimits>>()
            })
            .cloned();
        permit.attach_request_cancellation(context.ct.clone())?;
        permit.check_live()?;
        Ok(permit)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpHandler {
    async fn list_resource_templates(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourceTemplatesResult, rmcp::ErrorData> {
        Ok(rmcp::model::ListResourceTemplatesResult::with_all_items(vec![
            rmcp::model::ResourceTemplate::new(
                "nickel://desktop-events/{lease_id}",
                "desktop_events",
            )
            .with_description("Bounded payload-free desktop event history. Reading and subscribing require the client's active full-debug lease. Subscriptions last at most 60 seconds; reconnect and read the event cursor to recover gaps.")
            .with_mime_type("text/plain"),
        ]))
    }

    async fn read_resource(
        &self,
        request: rmcp::model::ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ReadResourceResponse, rmcp::ErrorData> {
        self.metrics
            .measure(crate::operation_metrics::Method::ReadDesktopEvents, async {
                let failure = |message: String| rmcp::ErrorData::invalid_params(message, None);
                let lease =
                    crate::event_subscriptions::resource_lease(&request.uri).map_err(failure)?;
                let permit = self.permit(&context, lease).map_err(failure)?;
                permit.with_debug(false, || Ok(())).map_err(failure)?;
                let observation = desktop_call(self.desktop.clone(), move |desktop| {
                    desktop.read_desktop_events(permit, 0)
                })
                .await
                .map_err(failure)?;
                let contents = serde_json::to_string(&observation)
                    .map_err(|_| failure("event serialization unavailable".into()))?;
                Ok(
                    rmcp::model::ReadResourceResult::new(vec![
                        rmcp::model::ResourceContents::text(contents, request.uri),
                    ])
                    .with_ttl_ms(0)
                    .with_cache_scope(rmcp::model::CacheScope::Private)
                    .into(),
                )
            })
            .await
    }

    fn accepted_subscription_filter(
        &self,
        requested: &rmcp::model::SubscriptionFilter,
    ) -> Option<rmcp::model::SubscriptionFilter> {
        let uris = requested.resource_subscriptions.as_ref()?;
        if uris.len() != 1 || crate::event_subscriptions::resource_lease(&uris[0]).is_err() {
            return None;
        }
        let mut accepted = rmcp::model::SubscriptionFilter::default();
        accepted.resource_subscriptions = Some(uris.clone());
        Some(accepted)
    }

    async fn listen(
        &self,
        context: rmcp::service::SubscriptionContext,
    ) -> Result<(), rmcp::ErrorData> {
        self.metrics
            .measure(crate::operation_metrics::Method::EventSubscription, async {
                let failure = |message: String| rmcp::ErrorData::invalid_params(message, None);
                let uri = context
                    .accepted()
                    .resource_subscriptions
                    .as_ref()
                    .and_then(|uris| uris.first())
                    .ok_or_else(|| failure("event resource subscription required".into()))?
                    .clone();
                let lease = crate::event_subscriptions::resource_lease(&uri).map_err(failure)?;
                let auth = self
                    .authenticated(context.request_context())
                    .map_err(failure)?;
                let permit = self
                    .permit(context.request_context(), lease)
                    .map_err(failure)?;
                permit.with_debug(false, || Ok(())).map_err(failure)?;
                let _admission =
                    crate::event_subscriptions::SubscriptionAdmission::acquire(&auth.client_id)
                        .map_err(failure)?;
                let stream = async {
                    let mut cursor = None;
                    loop {
                        let next = permit.continued_observation().map_err(failure)?;
                        let observation = desktop_call(self.desktop.clone(), move |desktop| {
                            desktop.read_desktop_events(next, 0)
                        })
                        .await
                        .map_err(failure)?;
                        if cursor != Some(observation.history.generation) {
                            permit.continued_observation().map_err(failure)?;
                            tokio::time::timeout(
                                std::time::Duration::from_millis(500),
                                context.sink().notify_resource_updated(uri.clone()),
                            )
                            .await
                            .map_err(|_| failure("event notification delivery timed out".into()))?
                            .map_err(|_| failure("event notification stream closed".into()))?;
                            cursor = Some(observation.history.generation);
                        }
                        tokio::select! {
                            _ = context.cancelled() => return Ok(()),
                            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
                        }
                    }
                };
                match tokio::time::timeout(
                    std::time::Duration::from_secs(
                        crate::event_subscriptions::MAX_SUBSCRIPTION_SECONDS,
                    ),
                    stream,
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Ok(()),
                }
            })
            .await
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_resources_subscribe()
                .build(),
        )
        .with_server_info(Implementation::new(
            "nickel-remote-control",
            env!("CARGO_PKG_VERSION"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    static PORT_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn queued_capture_rechecks_revocation_before_encoding_and_releases_budget() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        let control = Arc::new(Mutex::new(ControlPlane::default()));
        let now = std::time::Instant::now();
        let (identity, lease) = {
            let mut plane = control.lock().unwrap();
            plane.set_enabled(true);
            let identity = plane.connect_identity("Queued capture fixture").unwrap();
            crate::ready_connection(&mut plane, &identity, std::time::Instant::now());
            let request = crate::lease_requests::LeaseRequest {
                renewal: None,
                scope: crate::leases::ResourceScope::FullSession,
                duration: Some(std::time::Duration::from_secs(1200)),
                allow_resumption: false,
                full_debug: false,
            };
            plane
                .request_lease(&identity.client_id, &identity.token, request.clone(), now)
                .unwrap();
            let lease = {
                let generation = plane
                    .lease_requests()
                    .pending_generation(&identity.client_id)
                    .unwrap_or(0);
                plane.approve_lease_local(&identity.client_id, &request, generation, now)
            }
            .unwrap();
            (identity, lease)
        };
        let permit =
            crate::DesktopPermit::new(control.clone(), identity.client_id, identity.token, lease);
        permit.check_live().unwrap();
        runtime.block_on(async {
            let (started, ready) = std::sync::mpsc::channel();
            let (release, blocked) = std::sync::mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                started.send(()).unwrap();
                // The release sender drops on unwinding too, so cleanup cannot
                // strand this worker if a later assertion fails.
                let _ = blocked.recv();
            });
            ready
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap();
            let gate = Arc::new(tokio::sync::Semaphore::new(1));
            let reservation = CaptureReservation {
                _permit: Arc::new(gate.clone().try_acquire_owned().unwrap()),
            };
            // An invalid buffer detects whether encoding was reached: its error
            // must never supersede the revoked authority error.
            let frame = crate::capture::CapturedWindow {
                window_id: "1".into(),
                generation: 1,
                capture_generation: 1,
                submitted_at_us: 1,
                completed_at_us: 2,
                width: 1,
                height: 1,
                rgba: Vec::new(),
            };
            let worker = queue_capture_encoding(frame, permit.clone(), reservation);
            assert!(!worker.is_finished());
            assert_eq!(gate.available_permits(), 0);
            control.lock().unwrap().leases_mut().revoke(lease);
            let expected = permit.check_live().unwrap_err();
            release.send(()).unwrap();
            blocker.await.unwrap();
            let result = worker.await.unwrap();
            assert_eq!(result.err(), Some(expected));
            assert_eq!(gate.available_permits(), 1);
        });
    }

    #[test]
    fn capture_budget_remains_owned_by_queued_response_bytes_after_body_drop() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let gate = Arc::new(tokio::sync::Semaphore::new(1));
            let reservation = CaptureReservation {
                _permit: Arc::new(gate.clone().try_acquire_owned().unwrap()),
            };
            let response = hold_response_budget(
                Response::new(axum::body::Body::from("encoded image")),
                reservation,
            );
            let mut body = response.into_body();
            let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
            drop(body);
            assert_eq!(gate.available_permits(), 0);
            let queued_copy = bytes.clone();
            drop(bytes);
            assert_eq!(
                gate.available_permits(),
                0,
                "queued transport bytes still own the image budget"
            );
            drop(queued_copy);
            assert_eq!(gate.available_permits(), 1);
        });
    }

    struct EmptyDesktop;

    impl DesktopAuthority for EmptyDesktop {
        fn client_connection(
            &self,
            permit: crate::ClientConnectionPermit,
            action: crate::ClientConnectionAction,
        ) -> Result<(), String> {
            permit.apply(action, false)
        }

        fn keyboard_action(
            &self,
            _permit: crate::DesktopPermit,
            _id: &str,
            _generation: u64,
            _action: crate::keyboard::KeyboardAction,
        ) -> Result<(), String> {
            Err("no compositor attached".into())
        }
        fn pointer_action(
            &self,
            _permit: crate::DesktopPermit,
            _id: &str,
            _generation: u64,
            _x: i32,
            _y: i32,
            _action: crate::pointer::PointerAction,
        ) -> Result<(), String> {
            Err("no compositor attached".into())
        }
        fn diagnostic_snapshot(
            &self,
            _permit: crate::DesktopPermit,
        ) -> Result<crate::diagnostics::DiagnosticSnapshot, String> {
            Err("no compositor attached".into())
        }
        fn list_windows(
            &self,
            _permit: crate::DesktopPermit,
        ) -> Result<Vec<WindowSummary>, String> {
            Ok(Vec::new())
        }

        fn window_action(
            &self,
            _permit: crate::DesktopPermit,
            _id: &str,
            _generation: u64,
            _action: crate::window_actions::WindowAction,
        ) -> Result<crate::window_actions::WindowOutcome, String> {
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
    fn http_initialization_is_capability_free() {
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
            unauthenticated.starts_with("HTTP/1.1 200"),
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
    fn connection_watch_stream_loss_preserves_overlaps_and_resumes_only_approved_leases() {
        let _port = PORT_TEST.lock().unwrap_or_else(|error| error.into_inner());
        for version in ["2025-06-18", "2026-07-28"] {
            let control = Arc::new(Mutex::new(ControlPlane::default()));
            let (client, temporary, resumable) = {
                let mut plane = control.lock().unwrap();
                plane.set_enabled(true);
                let client = plane.connect_identity("Watch HTTP fixture").unwrap();
                crate::ready_connection(&mut plane, &client, std::time::Instant::now());
                let now = std::time::Instant::now();
                let mut ids = Vec::new();
                for allow_resumption in [false, true] {
                    let request = crate::lease_requests::LeaseRequest {
                        renewal: None,
                        scope: crate::leases::ResourceScope::FullSession,
                        duration: Some(std::time::Duration::from_secs(30)),
                        allow_resumption,
                        full_debug: false,
                    };
                    plane
                        .request_lease(&client.client_id, &client.token, request.clone(), now)
                        .unwrap();
                    ids.push(
                        {
                            let generation = plane
                                .lease_requests()
                                .pending_generation(&client.client_id)
                                .unwrap_or(0);
                            plane.approve_lease_local(&client.client_id, &request, generation, now)
                        }
                        .unwrap(),
                    );
                }
                (client, ids[0], ids[1])
            };
            let server =
                RemoteControlServer::start(control.clone(), Arc::new(EmptyDesktop)).unwrap();
            let open_watch = |id: u64| {
                let body = serde_json::json!({
                    "jsonrpc":"2.0", "id":id, "method":"tools/call", "params": {
                        "name":"client_connection", "arguments":{"action":"watch"},
                        "_meta": {
                            "progressToken":id,
                            "io.modelcontextprotocol/protocolVersion":version,
                            "io.modelcontextprotocol/clientInfo":{"name":"watch fixture","version":"1"},
                            "io.modelcontextprotocol/clientCapabilities":{}
                        }
                    }
                }).to_string();
                let mut stream = std::net::TcpStream::connect(MCP_ADDRESS).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                write!(stream, "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: {version}\r\nMcp-Method: tools/call\r\nMcp-Name: client_connection\r\nX-Nickel-Client: {}\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\n\r\n{body}", client.client_id, client.token, body.len()).unwrap();
                let mut received = Vec::new();
                while !String::from_utf8_lossy(&received).contains("Connection watch active") {
                    let mut chunk = [0; 4096];
                    let count = stream.read(&mut chunk).unwrap_or_else(|error| {
                        panic!(
                            "watch {version}/{id} did not become ready: {error}; {}",
                            String::from_utf8_lossy(&received)
                        )
                    });
                    assert_ne!(
                        count,
                        0,
                        "watch ended before readiness: {}",
                        String::from_utf8_lossy(&received)
                    );
                    received.extend_from_slice(&chunk[..count]);
                    assert!(received.len() < 16384);
                }
                stream
            };
            let bootstrap = *control
                .lock()
                .unwrap()
                .connection_watches
                .keys()
                .next()
                .unwrap();
            let first = open_watch(1);
            control
                .lock()
                .unwrap()
                .close_connection_watch(bootstrap, std::time::Instant::now());
            let second = open_watch(2);
            first.shutdown(std::net::Shutdown::Both).unwrap();
            drop(first);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while control.lock().unwrap().connection_watches.len() != 1 {
                assert!(
                    std::time::Instant::now() < deadline,
                    "closed first watch was retained"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(
                control
                    .lock()
                    .unwrap()
                    .leases
                    .active_lease(temporary, &client.client_id, std::time::Instant::now())
                    .is_ok()
            );
            second.shutdown(std::net::Shutdown::Both).unwrap();
            drop(second);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !control.lock().unwrap().connection_watches.is_empty() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "last watch loss did not disconnect"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(
                control
                    .lock()
                    .unwrap()
                    .leases
                    .active_lease(resumable, &client.client_id, std::time::Instant::now())
                    .is_err()
            );
            let replacement = open_watch(3);
            assert!(
                control
                    .lock()
                    .unwrap()
                    .leases
                    .active_lease(resumable, &client.client_id, std::time::Instant::now())
                    .is_ok()
            );
            assert!(
                !control
                    .lock()
                    .unwrap()
                    .leases
                    .iter()
                    .any(|lease| lease.id == temporary)
            );
            drop(replacement);
            server.stop();
        }
    }

    #[test]
    fn streamed_mcp_bodies_are_bounded_before_the_nested_service_parses_them() {
        let _port = PORT_TEST.lock().unwrap_or_else(|error| error.into_inner());
        let control = Arc::new(Mutex::new(ControlPlane::default()));
        control.lock().unwrap().set_enabled(true);
        let server = RemoteControlServer::start(control, Arc::new(EmptyDesktop)).unwrap();
        let payload = "x".repeat(crate::admission::MAX_REQUEST_BYTES + 1);
        let response = http(&format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{payload}\r\n0\r\n\r\n",
            payload.len()
        ));
        assert!(response.starts_with("HTTP/1.1 413"), "{response}");
        let metrics = http("GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        assert!(metrics.starts_with("HTTP/1.1 200"));
        assert!(metrics.contains("nickel_mcp_requests_admitted_total 2"));
        server.stop();
    }

    #[test]
    fn incomplete_http_body_times_out_and_releases_admission() {
        let _port = PORT_TEST.lock().unwrap_or_else(|error| error.into_inner());
        let control = Arc::new(Mutex::new(ControlPlane::default()));
        control.lock().unwrap().set_enabled(true);
        let server = RemoteControlServer::start(control, Arc::new(EmptyDesktop)).unwrap();
        let mut stream = std::net::TcpStream::connect(MCP_ADDRESS).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        stream.write_all(b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: 1\r\nConnection: close\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 408"), "{response}");
        let metrics = http("GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        // The scrape itself is the only remaining admitted request.
        assert!(metrics.contains("nickel_mcp_requests_active 1"));
        server.stop();
    }

    #[test]
    fn waiting_for_desktop_reply_does_not_block_the_network_runtime() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (release, wait) = mpsc::sync_channel(1);
        runtime.block_on(async {
            let (result, ()) = tokio::join!(
                desktop_call(Arc::new(EmptyDesktop), move |_| {
                    wait.recv_timeout(std::time::Duration::from_secs(1))
                        .map_err(|_| "network runtime was blocked".to_owned())
                }),
                async move {
                    tokio::task::yield_now().await;
                    let _ = release.send(());
                },
            );
            result.unwrap();
        });
    }

    #[test]
    fn connected_identity_and_metrics_do_not_grant_desktop_authority() {
        let _port = PORT_TEST.lock().unwrap_or_else(|error| error.into_inner());
        let control = Arc::new(Mutex::new(ControlPlane::default()));
        control.lock().unwrap().set_enabled(true);
        let server = RemoteControlServer::start(control.clone(), Arc::new(EmptyDesktop)).unwrap();
        let identity = response_json(&post_json(
            "/clients/connect",
            r#"{"label":"private-agent-name"}"#,
        ));
        let client = identity["client_id"].as_str().unwrap();
        let token = identity["token"].as_str().unwrap();
        assert!(control.lock().unwrap().authenticate(client, token));
        assert_eq!(
            control.lock().unwrap().client_origin(client),
            Some(crate::ClientOrigin {
                address: std::net::Ipv4Addr::LOCALHOST.into(),
                tls: false,
            })
        );
        assert!(
            !control
                .lock()
                .unwrap()
                .authorize(client, token, Capability::Observe)
        );
        let permit = crate::DesktopPermit::new(control.clone(), client.into(), token.into(), 1);
        assert!(permit.check_live().is_err());
        let call = |name: &str, arguments: serde_json::Value| {
            let body = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}}).to_string();
            http(&format!(
                "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nX-Forwarded-For: 203.0.113.9\r\nX-Forwarded-Proto: https\r\nX-Nickel-Client: {client}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ))
        };
        assert!(call("get_control_status", serde_json::json!({})).contains("emergency_stop"));
        assert_eq!(
            control.lock().unwrap().client_origin(client),
            Some(crate::ClientOrigin {
                address: std::net::Ipv4Addr::LOCALHOST.into(),
                tls: false,
            })
        );
        assert!(
            call(
                "keyboard_action",
                serde_json::json!({
                    "lease_id":1,"window_id":"private-window-id","generation":1,
                    "action":{"kind":"text","text":"private-typed-canary"}
                })
            )
            .contains("isError")
        );
        let lease = {
            let mut state = control.lock().unwrap();
            state
                .leases_mut()
                .approve_local(
                    client.into(),
                    crate::leases::ResourceScope::FullSession,
                    std::time::Instant::now(),
                    None,
                    true,
                    true,
                )
                .unwrap()
        };
        let inactive =
            http("GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        assert!(inactive.contains("nickel_mcp_active_leases_by_scope{scope=\"full_session\"} 0"));
        assert!(
            crate::DesktopPermit::new(control.clone(), client.into(), token.into(), lease)
                .check_live()
                .is_err()
        );
        {
            let mut state = control.lock().unwrap();
            let now = std::time::Instant::now();
            let watch = state.reserve_connection_watch(client, token, now).unwrap();
            state
                .activate_connection_watch(client, token, watch, false, now)
                .unwrap();
        }
        let active = http("GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        assert!(active.contains("nickel_mcp_active_leases_by_scope{scope=\"full_session\"} 1"));
        control.lock().unwrap().leases_mut().disconnect(client);
        assert!(
            control
                .lock()
                .unwrap()
                .leases()
                .iter()
                .any(|entry| entry.id == lease)
        );

        let metrics = http("GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        assert!(metrics.starts_with("HTTP/1.1 200"));
        assert!(metrics.contains("nickel_mcp_active_leases 0"));
        assert!(metrics.contains("nickel_mcp_active_leases_by_scope{scope=\"full_session\"} 0"));
        assert!(metrics.contains(
            "nickel_mcp_requests_total{method=\"get_control_status\",outcome=\"success\"} 1"
        ));
        assert!(
            metrics.contains(
                "nickel_mcp_requests_total{method=\"keyboard_action\",outcome=\"error\"} 1"
            )
        );
        for secret in [
            client,
            token,
            "private-agent-name",
            "private-window-id",
            "private-typed-canary",
        ] {
            assert!(!metrics.contains(secret));
        }
        server.stop();
    }

    #[test]
    fn https_serves_metrics_with_verified_host_certificate_and_releases_port() {
        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = reservation.local_addr().unwrap();
        drop(reservation);
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let config = crate::listener::ListenerConfig::parse(
            Some(&address.to_string()),
            Some(fixtures.join("localhost-cert.pem")),
            Some(fixtures.join("localhost-key.pem")),
        )
        .unwrap();
        let control = Arc::new(Mutex::new(ControlPlane::default()));
        control.lock().unwrap().set_enabled(true);
        let server =
            RemoteControlServer::start_with_config(control.clone(), Arc::new(EmptyDesktop), config)
                .unwrap();
        assert!(server.endpoint().starts_with("https://"));
        let certificate = rustls_pemfile::certs(
            &mut include_bytes!("../tests/fixtures/localhost-cert.pem").as_slice(),
        )
        .next()
        .unwrap()
        .unwrap();
        use sha2::Digest;
        assert_eq!(
            server.host_fingerprint,
            Some(crate::hex(&sha2::Sha256::digest(certificate.as_ref())))
        );
        let mut roots = rustls::RootCertStore::empty();
        roots.add(certificate).unwrap();
        let client = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let client = Arc::new(client);
        let connection =
            rustls::ClientConnection::new(client.clone(), "localhost".try_into().unwrap()).unwrap();
        let socket = std::net::TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let mut tls = rustls::StreamOwned::new(connection, socket);
        tls.write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        // HTTP close may precede TLS close_notify; the complete HTTP response is asserted below.
        let result = tls.read_to_string(&mut response);
        assert!(
            result.is_ok()
                || result.is_err_and(|error| error.kind() == std::io::ErrorKind::UnexpectedEof)
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains("nickel_mcp_active_leases 0"));
        drop(tls);
        let connection =
            rustls::ClientConnection::new(client, "localhost".try_into().unwrap()).unwrap();
        let socket = std::net::TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let mut tls = rustls::StreamOwned::new(connection, socket);
        let body = r#"{"label":"TLS origin fixture"}"#;
        write!(tls, "POST /clients/connect HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nX-Forwarded-For: 203.0.113.9\r\nX-Forwarded-Proto: http\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        let mut response = String::new();
        let result = tls.read_to_string(&mut response);
        assert!(
            result.is_ok()
                || result.is_err_and(|error| error.kind() == std::io::ErrorKind::UnexpectedEof)
        );
        assert!(response.starts_with("HTTP/1.1 200"));
        let identity = response_json(&response);
        let client_id = identity["client_id"].as_str().unwrap();
        assert_eq!(
            control.lock().unwrap().client_origin(client_id),
            Some(crate::ClientOrigin {
                address: std::net::Ipv4Addr::LOCALHOST.into(),
                tls: true,
            })
        );
        assert_eq!(control.lock().unwrap().leases().iter().count(), 0);
        drop(tls);
        server.stop();
        assert!(std::net::TcpListener::bind(address).is_ok());
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
