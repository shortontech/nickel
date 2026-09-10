//! Windows winit-owned remote-control lifecycle and private Settings requests.
//! Native resource operations remain denied until trusted Windows indication,
//! input arbitration and resource identities are integrated.
use crate::remote_indicator::{IndicatorGrant, RemoteIndicator};
use crate::winit_shell::{DisplayGeometry, ShellEvent, SurfaceId, WinitShell, WinitWindowCompat};
use nickel_remote_control::{
    DesktopAuthority, DesktopPermit, RemoteAiControlSettings, RemoteControlRuntime,
};
use nickel_session_protocol::{Command, ErrorCode, Query, Request, ServerMessage};
use std::{
    io,
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

static STOP_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static EMERGENCY: std::sync::OnceLock<nickel_remote_control::EmergencyStopHandle> =
    std::sync::OnceLock::new();
static CHORD: crate::windows_emergency_chord::WindowsEmergencyChord =
    crate::windows_emergency_chord::WindowsEmergencyChord::new();

pub(crate) fn observe_physical_key(event: nickel_input::windows::NativeKeyboardEvent) {
    use std::sync::atomic::Ordering;
    if CHORD.observe(event) {
        if let Some(handle) = EMERGENCY.get() {
            handle.trigger();
        }
        STOP_REQUESTED.store(true, Ordering::Release);
    }
}

const NOT_READY: &str = "Windows trusted indication and native control are not available yet";
fn error(message: impl Into<String>) -> ServerMessage {
    ServerMessage::Error {
        code: ErrorCode::InvalidRequest,
        message: message.into(),
    }
}
fn revoke_native_resource(
    control: &Arc<std::sync::Mutex<nickel_remote_control::ControlPlane>>,
    identity: &nickel_remote_control::leases::ResourceId,
) {
    use nickel_remote_control::leases::ResourceScope;
    if let Ok(mut control) = control.lock() {
        let ids: Vec<_> = control
            .leases()
            .iter()
            .filter_map(|lease| match &lease.scope {
                ResourceScope::Window(id) | ResourceScope::Output(id) if id == identity => {
                    Some(lease.id)
                }
                _ => None,
            })
            .collect();
        for id in ids {
            control.leases_mut().revoke(id);
        }
        // Native input remains unavailable. Its future release owner must drain
        // cancelled holds before event dispatch; permit invalidation happens here.
    }
}
fn resource_label(scope: &nickel_remote_control::leases::ResourceScope) -> String {
    use nickel_remote_control::leases::ResourceScope;
    match scope {
        ResourceScope::FullSession => "Full Nickel session",
        ResourceScope::Application { .. } => "Application",
        ResourceScope::Window { .. } => "Window",
        ResourceScope::Surface { .. } => "Surface",
        ResourceScope::Output { .. } => "Output",
    }
    .into()
}
struct LocalRequest {
    envelope: nickel_session_protocol::ClientEnvelope,
    queued_at: Instant,
    deadline: Instant,
    reply: SyncSender<ServerMessage>,
}
enum ObservationKind {
    Windows,
    Outputs,
    Inspect { id: String, generation: u64 },
}
enum ObservationResult {
    Windows(Vec<nickel_remote_control::WindowSummary>),
    Outputs(nickel_remote_control::diagnostics::OutputInventory),
}
enum OwnerRequest {
    Local(LocalRequest),
    Observation {
        permit: DesktopPermit,
        prepared: Box<crate::platform::remote_observation::Prepared>,
        kind: ObservationKind,
        reply: SyncSender<Result<ObservationResult, String>>,
    },
    Connection {
        permit: nickel_remote_control::ClientConnectionPermit,
        action: nickel_remote_control::ClientConnectionAction,
        reply: SyncSender<Result<(), String>>,
    },
}
struct WindowsDesktopAuthority {
    cleanup_wake: nickel_remote_control::ConnectionCleanupWake,
    sender: SyncSender<OwnerRequest>,
}
impl WindowsDesktopAuthority {
    fn observe(
        &self,
        permit: DesktopPermit,
        kind: ObservationKind,
    ) -> Result<ObservationResult, String> {
        let _admission = crate::platform::remote_observation::Admission::acquire()?;
        let prepared = Box::new(crate::platform::remote_observation::Prepared::prepare(
            &permit,
        )?);
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::Observation {
                permit,
                prepared,
                kind,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows desktop owner timed out".to_owned())?;
        completion.check_live()?;
        result
    }
}
impl DesktopAuthority for WindowsDesktopAuthority {
    fn connection_cleanup_wake(&self) -> Option<nickel_remote_control::ConnectionCleanupWake> {
        Some(self.cleanup_wake.clone())
    }

    fn client_connection(
        &self,
        permit: nickel_remote_control::ClientConnectionPermit,
        action: nickel_remote_control::ClientConnectionAction,
    ) -> Result<(), String> {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::Connection {
                permit,
                action,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows desktop owner timed out".to_owned())?
    }
    fn keyboard_action(
        &self,
        _permit: DesktopPermit,
        _id: &str,
        _generation: u64,
        _action: nickel_remote_control::keyboard::KeyboardAction,
    ) -> Result<(), String> {
        Err(NOT_READY.into())
    }
    fn pointer_action(
        &self,
        _permit: DesktopPermit,
        _id: &str,
        _generation: u64,
        _x: i32,
        _y: i32,
        _action: nickel_remote_control::pointer::PointerAction,
    ) -> Result<(), String> {
        Err(NOT_READY.into())
    }
    fn diagnostic_snapshot(
        &self,
        _permit: DesktopPermit,
    ) -> Result<nickel_remote_control::diagnostics::DiagnosticSnapshot, String> {
        Err(NOT_READY.into())
    }
    fn list_windows(
        &self,
        permit: DesktopPermit,
    ) -> Result<Vec<nickel_remote_control::WindowSummary>, String> {
        match self.observe(permit, ObservationKind::Windows)? {
            ObservationResult::Windows(windows) => Ok(windows),
            _ => Err("Windows observation mismatch".into()),
        }
    }
    fn list_outputs(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::diagnostics::OutputInventory, String> {
        match self.observe(permit, ObservationKind::Outputs)? {
            ObservationResult::Outputs(outputs) => Ok(outputs),
            _ => Err("Windows observation mismatch".into()),
        }
    }
    fn inspect_window(
        &self,
        permit: DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<nickel_remote_control::semantics::SemanticSnapshot, String> {
        self.observe(
            permit,
            ObservationKind::Inspect {
                id: id.into(),
                generation,
            },
        )?;
        Err("Native Windows UI Automation observation is not available".into())
    }
    fn window_action(
        &self,
        _permit: DesktopPermit,
        _id: &str,
        _generation: u64,
        _action: nickel_remote_control::window_actions::WindowAction,
    ) -> Result<nickel_remote_control::window_actions::WindowOutcome, String> {
        Err(NOT_READY.into())
    }
}

pub(crate) struct WindowsRemoteControl {
    _transport: Option<nickel_platform::local_control::LocalControlServer>,
    receiver: Receiver<OwnerRequest>,
    remote_control: RemoteControlRuntime,
    local_cues: crate::local_cues::LocalCues,
    applications: crate::windows_application_registry::native::OwnerRegistry,
    resources: crate::windows_resource_owner::Owner,
    resource_lifecycle: Option<crate::platform::remote_observation::Lifecycle>,
    observation_generation: u64,
    indicators: std::collections::HashMap<String, IndicatorSurface>,
    authority: Arc<WindowsDesktopAuthority>,
    start_time: Instant,
    last_stop: Option<Instant>,
}
struct IndicatorSurface {
    id: SurfaceId,
    geometry: DisplayGeometry,
    host: crate::EmbeddedUiSurface<RemoteIndicator>,
    accessibility: crate::trusted_accessibility::native::IndicatorAccessibility,
    authority_revision: Vec<(u64, u64, u64)>,
}

impl WindowsRemoteControl {
    pub(crate) fn start(
        cleanup_wake: nickel_remote_control::ConnectionCleanupWake,
    ) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(16);
        let authority = Arc::new(WindowsDesktopAuthority {
            cleanup_wake,
            sender: sender.clone(),
        });
        let transport = nickel_platform::local_control::LocalControlServer::start(move |frame| {
            let envelope: nickel_session_protocol::ClientEnvelope =
                nickel_session_protocol::decode(&frame).map_err(io::Error::other)?;
            let request_id = envelope.request_id;
            // Authentication belongs to the OS pipe, never an inherited token.
            if !envelope.token.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "unexpected local token",
                ));
            }
            let (reply, response) = mpsc::sync_channel(1);
            sender
                .try_send(OwnerRequest::Local(LocalRequest {
                    envelope,
                    queued_at: Instant::now(),
                    deadline: Instant::now() + Duration::from_secs(1),
                    reply,
                }))
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::WouldBlock, "Windows owner queue full")
                })?;
            let message = response.recv_timeout(Duration::from_secs(1)).map_err(|_| {
                io::Error::new(io::ErrorKind::TimedOut, "Windows owner unavailable")
            })?;
            nickel_session_protocol::encode(&nickel_session_protocol::ServerEnvelope {
                request_id,
                message,
            })
            .map_err(io::Error::other)
        })?;
        let mut owner = Self {
            _transport: Some(transport),
            receiver,
            remote_control: RemoteControlRuntime::default(),
            local_cues: Default::default(),
            applications: Default::default(),
            resources: Default::default(),
            resource_lifecycle: crate::platform::remote_observation::Lifecycle::install().ok(),
            observation_generation: 0,
            indicators: Default::default(),
            authority,
            start_time: Instant::now(),
            last_stop: None,
        };
        // Publish the lock-free cancellation handle before any listener starts.
        // A second owner in the same process must not activate with a stale hook.
        EMERGENCY
            .set(owner.remote_control.emergency_stop_handle())
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "Windows emergency owner already registered",
                )
            })?;
        match RemoteAiControlSettings::load_default() {
            Ok(settings) => owner
                .remote_control
                .apply(&settings, owner.authority.clone()),
            Err(_) => owner
                .remote_control
                .set_diagnostic("Remote control settings could not be read"),
        }
        owner.sync_emergency_state();
        Ok(owner)
    }
    fn sync_emergency_state(&self) {
        use std::sync::atomic::Ordering;
        let enabled = self.remote_control.status().effective
            == nickel_remote_control::EffectiveState::Enabled;
        if !CHORD.set_enabled(enabled) {
            if let Some(handle) = EMERGENCY.get() {
                handle.trigger();
            }
            STOP_REQUESTED.store(true, Ordering::Release);
        }
    }
    pub(crate) fn poll(&mut self) {
        // Service transport loss before ordinary requests, even if their queue is full.
        if self.authority.cleanup_wake.take_wake_failure() {
            tracing::warn!("Remote connection cleanup wake failed; owner fallback is active");
        }
        if self.authority.cleanup_wake.take_pending() {
            self.remote_control
                .control()
                .lock()
                .unwrap()
                .reconcile_pending_lease_requests(Instant::now());
            // Native input is still unavailable on Windows. When it is enabled,
            // its cancellation drain must run here before any owner replies.
        }
        if STOP_REQUESTED.swap(false, std::sync::atomic::Ordering::AcqRel) {
            self.handle(Request::Command(Command::EmergencyStopRemoteControl));
        }
        self.drain_resource_lifecycle();
        for _ in 0..8 {
            let Ok(request) = self.receiver.try_recv() else {
                break;
            };
            match request {
                OwnerRequest::Observation {
                    permit,
                    prepared,
                    kind,
                    reply,
                } => {
                    let result = self.observe_resources(permit, *prepared, kind);
                    let _ = reply.try_send(result);
                }
                OwnerRequest::Local(request) => {
                    if Instant::now() >= request.deadline
                        || self
                            .last_stop
                            .is_some_and(|stopped| request.queued_at <= stopped)
                    {
                        continue;
                    }
                    let result = self.handle(request.envelope.request);
                    let _ = request.reply.try_send(result);
                }
                OwnerRequest::Connection {
                    permit,
                    action,
                    reply,
                } => {
                    // There is no Windows lock/secure-desktop observation yet.
                    // Reconnection must not assume the desktop is unlocked.
                    let _ = reply.try_send(permit.apply(action, true));
                }
            }
        }
        let control = self.remote_control.control();
        if let Ok(mut control) = control.lock() {
            self.applications.poll(&mut control);
            self.local_cues.update(control.leases(), Instant::now());
        }
    }
    fn drain_resource_lifecycle(&mut self) {
        let control = self.remote_control.control();
        let Some(lifecycle) = &self.resource_lifecycle else {
            self.resources
                .clear(|id| revoke_native_resource(&control, id));
            return;
        };
        match lifecycle.drain() {
            Ok(events) => {
                for native in events {
                    self.resources
                        .retire_window(native, |id| revoke_native_resource(&control, id));
                }
            }
            Err(_) => self
                .resources
                .clear(|id| revoke_native_resource(&control, id)),
        }
    }

    fn observe_resources(
        &mut self,
        permit: DesktopPermit,
        mut prepared: crate::platform::remote_observation::Prepared,
        kind: ObservationKind,
    ) -> Result<ObservationResult, String> {
        permit.check_live()?;
        self.drain_resource_lifecycle();
        let lifecycle = self
            .resource_lifecycle
            .as_ref()
            .ok_or("Windows native lifecycle observation is unavailable")?;
        if lifecycle.serial() != prepared.serial {
            return Err("Windows resources changed; retry observation".into());
        }
        prepared.revalidate()?;
        for window in &mut prepared.windows {
            let process = prepared
                .processes
                .get(&window.native)
                .ok_or("Windows process evidence is unavailable")?;
            window.application = self
                .applications
                .verified_application(process)
                .map(str::to_owned);
        }
        let control = self.remote_control.control();
        self.resources
            .reconcile(prepared.windows.clone(), prepared.outputs.clone(), |id| {
                revoke_native_resource(&control, id)
            })?;
        let scope = permit.resource_scope()?;
        let result = match kind {
            ObservationKind::Windows => {
                let mut result = Vec::new();
                for (window, evidence) in self.resources.windows(&scope) {
                    result.push(permit.with_resource(&evidence, || Ok(window))?);
                }
                ObservationResult::Windows(result)
            }
            ObservationKind::Outputs => {
                let mut outputs = Vec::new();
                for (output, evidence) in self.resources.outputs(&scope) {
                    outputs.push(permit.with_resource(&evidence, || Ok(output))?);
                }
                self.observation_generation = self
                    .observation_generation
                    .checked_add(1)
                    .ok_or("Windows observation generations exhausted")?;
                ObservationResult::Outputs(nickel_remote_control::diagnostics::OutputInventory {
                    observation_generation: self.observation_generation,
                    observed_at_us: self.start_time.elapsed().as_micros().min(u64::MAX as u128)
                        as u64,
                    outputs,
                    truncated: false,
                })
            }
            ObservationKind::Inspect { id, generation } => {
                self.resources
                    .window(&id, generation)
                    .ok_or("Windows resource is unavailable")?;
                let (_, evidence) = self
                    .resources
                    .windows(&scope)
                    .find(|(window, _)| window.id == id && window.generation == generation)
                    .ok_or("Windows resource is unavailable")?;
                permit.with_resource(&evidence, || Ok(()))?;
                // inspect_window means a real semantic tree. Do not manufacture a
                // Nickel surface/tree identity from an external window caption.
                return Err("Native Windows UI Automation observation is not available".into());
            }
        };
        prepared.revalidate()?;
        permit.check_live()?;
        Ok(result)
    }

    /// This is the real production host path. It does not relax the separate
    /// approval gate: native UIA is registered, but native validation and
    /// resource/input protection are still incomplete.
    pub(crate) fn reconcile_indicators(
        &mut self,
        shell: &mut WinitShell,
        theme: nickel_ui::SemanticTheme,
    ) {
        if let Err(error) = self.sync_indicators(shell, theme) {
            self.stop_indicators(shell);
            self.remote_control
                .set_diagnostic(format!("Trusted indication unavailable: {error}"));
        }
    }

    fn clear_indicators(&mut self, shell: &mut WinitShell) {
        for (_, indicator) in self.indicators.drain() {
            let id = indicator.id;
            // Invalidate queued native actions and remove the UIA subclass while
            // the actual winit Window is still retained by the shell.
            drop(indicator);
            shell.destroy_surface(id);
        }
    }

    fn stop_indicators(&mut self, shell: &mut WinitShell) {
        // Cancel permits before native teardown, including a partial create/show.
        self.remote_control.emergency_stop_handle().trigger();
        self.handle(Request::Command(Command::EmergencyStopRemoteControl));
        self.clear_indicators(shell);
    }

    fn sync_indicators(
        &mut self,
        shell: &mut WinitShell,
        theme: nickel_ui::SemanticTheme,
    ) -> Result<(), String> {
        let control = self.remote_control.control();
        let control = control
            .lock()
            .map_err(|_| "control owner unavailable".to_owned())?;
        let now = Instant::now();
        let grants = if control.enabled() {
            control
                .leases()
                .iter()
                .filter(|lease| lease.expires_at.is_none_or(|deadline| now < deadline))
                .map(|lease| IndicatorGrant {
                    suspended: lease.suspended,
                    connected: lease.is_connected(),
                    id: lease.id,
                    client: control
                        .granted_clients()
                        .find(|client| client.id == lease.client_identity)
                        .map(|client| client.label)
                        .unwrap_or_else(|| "Unknown client".to_owned()),
                    scope: if lease.full_debug {
                        "Full Control & Debug Nickel".to_owned()
                    } else {
                        resource_label(&lease.scope)
                    },
                    remaining: lease.expires_at.map_or_else(
                        || "until logout".to_owned(),
                        |deadline| {
                            format!("{}s", deadline.saturating_duration_since(now).as_secs())
                        },
                    ),
                    peer: control.client_origin(&lease.client_identity).map_or_else(
                        || "Peer unavailable".to_owned(),
                        |origin| {
                            format!(
                                "{} {}",
                                origin.address,
                                if origin.tls { "TLS" } else { "HTTP" }
                            )
                        },
                    ),
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut authority_revision = control
            .leases()
            .iter()
            .filter(|lease| lease.expires_at.is_none_or(|deadline| now < deadline))
            .map(|lease| {
                (
                    lease.id,
                    lease.operation_generation,
                    lease.renewal_generation,
                )
            })
            .collect::<Vec<_>>();
        authority_revision.sort_unstable();
        drop(control);
        if grants.is_empty() {
            self.clear_indicators(shell);
            return Ok(());
        }
        let transport = if self.remote_control.status().endpoint.starts_with("https:") {
            "HTTPS"
        } else {
            "Local HTTP"
        };
        let outputs = shell.trusted_control_outputs();
        if outputs.is_empty() {
            return Err("no local output for active-control indication".to_owned());
        }
        if !self.indicators.is_empty()
            && (self.indicators.len() != outputs.len()
                || self.indicators.iter().any(|(name, indicator)| {
                    !outputs
                        .iter()
                        .any(|(output, geometry)| output == name && *geometry == indicator.geometry)
                }))
        {
            // Never leave permits live while replacing an indicated output.
            return Err("display topology changed during remote control".to_owned());
        }
        for (name, geometry) in outputs {
            if !self.indicators.contains_key(&name) {
                let id = shell.create_trusted_control_surface(&name, grants.len())?;
                let (width, height) = shell
                    .surface(id)
                    .ok_or("trusted surface disappeared")?
                    .window()
                    .size();
                let host = crate::EmbeddedUiSurface::new(
                    RemoteIndicator {
                        theme,
                        transport: transport.to_owned(),
                        grants: grants.clone(),
                        stop_requested: false,
                    },
                    width,
                    height,
                    now,
                );
                let accessibility =
                    match crate::trusted_accessibility::native::IndicatorAccessibility::new(
                        shell
                            .surface(id)
                            .ok_or("trusted surface disappeared")?
                            .window(),
                        host.host.accessibility_nodes(),
                    ) {
                        Ok(accessibility) => accessibility,
                        Err(error) => {
                            shell.destroy_surface(id);
                            return Err(error);
                        }
                    };
                self.indicators.insert(
                    name.clone(),
                    IndicatorSurface {
                        id,
                        geometry,
                        host,
                        accessibility,
                        authority_revision: authority_revision.clone(),
                    },
                );
            }
            let indicator = self.indicators.get_mut(&name).expect("indicator inserted");
            shell.resize_trusted_control_surface(indicator.id, grants.len())?;
            let (width, height) = shell
                .surface(indicator.id)
                .ok_or("trusted surface disappeared")?
                .window()
                .size();
            let app = indicator.host.application_mut();
            let changed = app.grants != grants || app.theme != theme || app.transport != transport;
            app.transport = transport.to_owned();
            app.grants = grants.clone();
            app.theme = theme;
            indicator.host.step(nickel_ui::HostBatch {
                surface_size: Some((width, height)),
                application_changed: changed,
                now: Some(now),
                events: vec![nickel_ui::HostEvent::Poll],
                ..Default::default()
            });
            let scale = shell
                .surface(indicator.id)
                .ok_or("trusted surface disappeared")?
                .window()
                .scale_factor();
            indicator.accessibility.update(
                indicator.host.host.accessibility_nodes(),
                scale,
                indicator.authority_revision != authority_revision,
            )?;
            indicator.authority_revision = authority_revision.clone();
            if let Some(target) = indicator.accessibility.take_stop() {
                let outcome = indicator.host.step(nickel_ui::HostBatch {
                    events: vec![nickel_ui::HostEvent::Accessibility {
                        target,
                        action: nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
                    }],
                    ..Default::default()
                });
                if !outcome.semantic_failures.is_empty() {
                    return Err("trusted local Stop target changed".to_owned());
                }
                if indicator.host.application_mut().stop_requested {
                    self.stop_indicators(shell);
                    return Ok(());
                }
            }
            shell.present_host_frame(
                indicator.id,
                indicator.host.change_token,
                indicator.host.commands(),
            )?;
            shell.expose_trusted_control_surface(indicator.id)?;
        }
        Ok(())
    }

    /// Consume all owned indicator events before ordinary shell routing. Native
    /// UI Automation actions arrive through the separate bounded owner mailbox;
    /// no MCP semantic action enters either local route.
    pub(crate) fn indicator_event(&mut self, shell: &mut WinitShell, event: &ShellEvent) -> bool {
        let id = match event {
            ShellEvent::Input { surface, .. }
            | ShellEvent::FocusChanged { surface, .. }
            | ShellEvent::LogicalResize { surface, .. }
            | ShellEvent::PixelResize { surface, .. }
            | ShellEvent::PointerEntered { surface, .. }
            | ShellEvent::FileDrop { surface, .. }
            | ShellEvent::Shown(surface)
            | ShellEvent::Hidden(surface)
            | ShellEvent::CloseRequested(surface)
            | ShellEvent::Redraw(surface) => *surface,
            _ => return false,
        };
        let Some(indicator) = self
            .indicators
            .values_mut()
            .find(|indicator| indicator.id == id)
        else {
            // Already-retired events must not reach ordinary close/quit behavior.
            return shell
                .surface(id)
                .is_some_and(|surface| surface.role() == crate::SurfaceRole::TrustedControl);
        };
        if matches!(event, ShellEvent::CloseRequested(_) | ShellEvent::Hidden(_)) {
            self.stop_indicators(shell);
            return true;
        }
        match event {
            ShellEvent::Input { event, .. } => {
                indicator.host.normalized_input(event.clone(), None);
            }
            ShellEvent::FocusChanged { focused, .. } => {
                indicator.host.window_focus(*focused);
            }
            ShellEvent::LogicalResize { .. } | ShellEvent::PixelResize { .. } => {
                if let Some(surface) = shell.surface(id) {
                    indicator.host.step(nickel_ui::HostBatch {
                        surface_size: Some(surface.window().size()),
                        ..Default::default()
                    });
                }
            }
            _ => {}
        }
        if indicator.host.application_mut().stop_requested {
            self.stop_indicators(shell);
            return true;
        }
        let result = if matches!(event, ShellEvent::Redraw(_)) {
            shell.present(id, indicator.host.commands()).map(|_| ())
        } else {
            shell
                .present_host_frame(id, indicator.host.change_token, indicator.host.commands())
                .map(|_| ())
        };
        if result.is_err() || shell.trusted_control_capture_affinity(id).is_err() {
            self.stop_indicators(shell);
        }
        true
    }

    pub(crate) fn indicator_controller(
        &mut self,
        shell: &mut WinitShell,
        action: nickel_ui::ControllerAction,
    ) -> bool {
        let Some(indicator) = self.indicators.values_mut().find(|indicator| {
            shell
                .surface(indicator.id)
                .is_some_and(|surface| surface.window().has_input_focus())
        }) else {
            return false;
        };
        indicator.host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Controller(action)],
            ..Default::default()
        });
        if indicator.host.application_mut().stop_requested
            || shell
                .present_host_frame(
                    indicator.id,
                    indicator.host.change_token,
                    indicator.host.commands(),
                )
                .is_err()
        {
            self.stop_indicators(shell);
        }
        true
    }

    fn handle(&mut self, request: Request) -> ServerMessage {
        let command = match request {
            Request::Query(Query::RemoteControl) => return self.remote_control_snapshot(),
            Request::Command(command) => command,
            _ => return error("request is unavailable on the Windows Settings transport"),
        };
        match command {
            Command::ApplyRemoteControl {
                requested_enabled,
                generation,
            } => {
                let mut settings = RemoteAiControlSettings::default();
                settings.requested_enabled = requested_enabled;
                settings.generation = generation;
                self.remote_control.apply(&settings, self.authority.clone());
            }
            Command::EmergencyStopRemoteControl => {
                self.last_stop = Some(Instant::now());
                let mut settings = RemoteAiControlSettings::load_default().unwrap_or_default();
                settings.set_requested(false);
                self.remote_control.emergency_stop_at(settings.generation);
                if RemoteAiControlSettings::default_path()
                    .and_then(|path| settings.save(path))
                    .is_err()
                {
                    self.remote_control
                        .set_diagnostic("Remote control stopped, but Disabled could not be saved");
                }
            }
            Command::StartRemotePairing { now_unix_secs } => {
                return match self
                    .remote_control
                    .control()
                    .lock()
                    .unwrap()
                    .start_pairing(now_unix_secs)
                {
                    Ok(pairing) => ServerMessage::RemotePairing(
                        nickel_session_protocol::RemotePairingSnapshot {
                            ceremony_id: pairing.ceremony_id,
                            qr_payload: pairing.qr_payload,
                            short_code: pairing.short_code,
                            expires_at: pairing.expires_at,
                        },
                    ),
                    Err(reason) => error(reason.to_string()),
                };
            }
            Command::CancelRemotePairing => self
                .remote_control
                .control()
                .lock()
                .unwrap()
                .cancel_pairing(),
            Command::DecideRemoteClient {
                client_id,
                decision,
                capabilities,
            } => {
                let decision = match decision {
                    nickel_session_protocol::RemoteClientDecision::Deny => {
                        nickel_remote_control::Approval::Deny
                    }
                    nickel_session_protocol::RemoteClientDecision::AllowOnce => {
                        nickel_remote_control::Approval::AllowOnce
                    }
                    nickel_session_protocol::RemoteClientDecision::Remember => {
                        nickel_remote_control::Approval::Remember
                    }
                };
                if let Err(reason) = self.remote_control.control().lock().unwrap().approve(
                    &client_id,
                    decision,
                    capabilities.into_iter().map(control_capability).collect(),
                ) {
                    return error(reason.to_string());
                }
            }
            Command::BlockRemoteClient { client_id, blocked } => {
                if let Err(reason) = self
                    .remote_control
                    .control()
                    .lock()
                    .unwrap()
                    .block_client_local(&client_id, blocked)
                {
                    return error(reason.to_string());
                }
            }
            Command::RevokeRemoteClient { client_id } => {
                self.remote_control
                    .control()
                    .lock()
                    .unwrap()
                    .revoke(&client_id);
            }
            Command::DecideRemoteLease {
                client_id,
                request,
                pending_generation,
                allow: false,
            } => {
                if !self
                    .remote_control
                    .control()
                    .lock()
                    .unwrap()
                    .lease_requests_mut()
                    .deny_displayed_local(
                        &client_id,
                        &request.into(),
                        pending_generation,
                        Instant::now(),
                    )
                {
                    return error("permission request changed");
                }
            }
            Command::DecideRemoteLease { .. } | Command::ApproveRemoteLeaseDuration { .. } => {
                return error(NOT_READY);
            }
            Command::ManageRemoteLease { lease_id, action } => {
                use nickel_session_protocol::RemoteLeaseAction;
                let control = self.remote_control.control();
                let mut control = control.lock().unwrap();
                match action {
                    RemoteLeaseAction::Revoke => {
                        control.leases_mut().revoke(lease_id);
                    }
                    RemoteLeaseAction::Pause => {
                        if let Err(reason) = control.leases_mut().suspend_local(lease_id) {
                            return error(reason.to_string());
                        }
                    }
                    RemoteLeaseAction::Resume => return error(NOT_READY),
                }
            }
            _ => return error("request is unavailable on the Windows Settings transport"),
        }
        self.sync_emergency_state();
        self.remote_control_snapshot()
    }
    fn remote_control_snapshot(&self) -> ServerMessage {
        let status = self.remote_control.status();
        let effective = match status.effective {
            nickel_remote_control::EffectiveState::Disabled => {
                nickel_session_protocol::RemoteControlEffectiveState::Disabled
            }
            nickel_remote_control::EffectiveState::Enabled => {
                nickel_session_protocol::RemoteControlEffectiveState::Enabled
            }
            nickel_remote_control::EffectiveState::Rejected => {
                nickel_session_protocol::RemoteControlEffectiveState::Rejected
            }
        };
        let control = self.remote_control.control();
        let mut control = control.lock().unwrap();
        control.reconcile_pending_lease_requests(Instant::now());
        let pending_clients = control
            .pending_clients()
            .map(
                |client| nickel_session_protocol::RemotePendingClientSnapshot {
                    id: client.id.clone(),
                    label: client.label.clone(),
                    requested: client
                        .requested
                        .iter()
                        .copied()
                        .map(remote_capability)
                        .collect(),
                    connected_at: client.connected_at,
                },
            )
            .collect();
        let granted_clients = control
            .granted_clients()
            .map(
                |client| nickel_session_protocol::RemoteGrantedClientSnapshot {
                    origin: control.client_origin(&client.id).map(|origin| {
                        nickel_session_protocol::RemoteClientOrigin {
                            address: origin.address.to_string(),
                            tls: origin.tls,
                        }
                    }),
                    blocked: control.lease_requests().is_blocked(&client.id),
                    id: client.id,
                    label: client.label,
                    capabilities: client
                        .capabilities
                        .into_iter()
                        .map(remote_capability)
                        .collect(),
                    remembered: client.remembered,
                },
            )
            .collect();
        let (trace_events, trace_audit_evicted) =
            control.trace_audit().snapshot().unwrap_or_default();
        ServerMessage::RemoteControl(nickel_session_protocol::RemoteControlSnapshot {
            requested_enabled: status.requested_enabled,
            effective,
            generation: status.generation,
            acknowledged_generation: status.acknowledged_generation,
            endpoint: status.endpoint.clone(),
            host_fingerprint: status.host_fingerprint.clone(),
            environment_override: status.environment_override,
            diagnostic: status.diagnostic.clone(),
            pending_clients,
            granted_clients,
            connection_audit: control
                .connection_audit()
                .map(
                    |event| nickel_session_protocol::RemoteConnectionAuditEvent {
                        generation: event.generation,
                        observed_at_us: event
                            .observed_at
                            .saturating_duration_since(self.start_time)
                            .as_micros()
                            .min(u64::MAX as u128) as u64,
                        client_id: event.client_id.clone(),
                        address: event.origin.address,
                        tls: event.origin.tls,
                    },
                )
                .collect(),
            connection_audit_evicted: control.connection_audit_evicted(),
            lease_audit: control
                .leases()
                .audit()
                .events()
                .map(|event| nickel_session_protocol::RemoteLeaseAuditEvent {
                    generation: event.generation,
                    observed_at_us: event
                        .observed_at
                        .saturating_duration_since(self.start_time)
                        .as_micros()
                        .min(u64::MAX as u128) as u64,
                    lease_id: event.lease_id,
                    transition: event.transition,
                    scope: event.scope,
                    lifetime_limit_seconds: event.lifetime_limit.map(|duration| duration.as_secs()),
                    full_debug: event.full_debug,
                    allow_resumption: event.allow_resumption,
                })
                .collect(),
            lease_audit_evicted: control.leases().audit().evicted(),
            permission_audit: control
                .lease_requests()
                .audit()
                .map(
                    |event| nickel_session_protocol::RemotePermissionAuditEvent {
                        generation: event.generation,
                        observed_at_us: event
                            .observed_at
                            .saturating_duration_since(self.start_time)
                            .as_micros()
                            .min(u64::MAX as u128) as u64,
                        client_id: event.client_id,
                        outcome: event.outcome,
                    },
                )
                .collect(),
            permission_audit_evicted: control.lease_requests().audit_evicted(),
            trace_audit: {
                trace_events
                    .into_iter()
                    .map(|event| nickel_session_protocol::RemoteTraceAuditEvent {
                        generation: event.generation,
                        observed_at_us: event
                            .observed_at
                            .saturating_duration_since(self.start_time)
                            .as_micros()
                            .min(u128::from(u64::MAX))
                            as u64,
                        client_id: event.client_id,
                        lease_id: event.lease_id,
                        trace_id: event.trace_id,
                        category: event.category,
                        transition: event.transition,
                        duration_limit_seconds: event.duration_limit_seconds,
                        elapsed_us: event.elapsed_us,
                    })
                    .collect()
            },
            trace_audit_evicted,

            active_leases: control
                .leases()
                .iter()
                .filter(|lease| {
                    lease
                        .expires_at
                        .is_none_or(|deadline| Instant::now() < deadline)
                })
                .map(|lease| nickel_session_protocol::RemoteActiveLease {
                    lease_id: lease.id,
                    client_label: control
                        .granted_clients()
                        .find(|client| client.id == lease.client_identity)
                        .map(|client| client.label)
                        .unwrap_or_else(|| "Agent".into()),
                    scope: lease.scope.clone(),
                    resource_label: Some(resource_label(&lease.scope)),
                    remaining_seconds: lease.expires_at.map(|deadline| {
                        deadline.saturating_duration_since(Instant::now()).as_secs()
                    }),
                    suspended: lease.suspended,
                    full_debug: lease.full_debug,
                })
                .collect(),
            pending_leases: control
                .lease_requests()
                .pending()
                .map(
                    |(client_id, request)| nickel_session_protocol::RemotePendingLease {
                        pending_generation: control
                            .lease_requests()
                            .pending_generation(client_id)
                            .expect("pending request has an incarnation"),
                        client_id: client_id.to_owned(),
                        client_label: control
                            .granted_clients()
                            .find(|client| client.id == client_id)
                            .map(|client| client.label)
                            .unwrap_or_else(|| "Connected client".into()),
                        resource_label: Some(resource_label(&request.scope)),
                        changes: control.lease_requests().pending_changes(client_id),
                        request: request.into(),
                    },
                )
                .collect(),
        })
    }
}
impl Drop for WindowsRemoteControl {
    fn drop(&mut self) {
        CHORD.set_enabled(false);
        self.remote_control.shutdown_session();
    }
}
fn remote_capability(
    capability: nickel_remote_control::Capability,
) -> nickel_session_protocol::RemoteCapability {
    use nickel_remote_control::Capability as Source;
    use nickel_session_protocol::RemoteCapability as Target;
    match capability {
        Source::Observe => Target::Observe,
        Source::WindowManagement => Target::WindowManagement,
        Source::SettingsRead => Target::SettingsRead,
        Source::SettingsChange => Target::SettingsChange,
        Source::ApplicationLaunch => Target::ApplicationLaunch,
        Source::PointerInput => Target::PointerInput,
        Source::KeyboardInput => Target::KeyboardInput,
        Source::ScreenCapture => Target::ScreenCapture,
    }
}

fn control_capability(
    capability: nickel_session_protocol::RemoteCapability,
) -> nickel_remote_control::Capability {
    use nickel_remote_control::Capability as Target;
    use nickel_session_protocol::RemoteCapability as Source;
    match capability {
        Source::Observe => Target::Observe,
        Source::WindowManagement => Target::WindowManagement,
        Source::SettingsRead => Target::SettingsRead,
        Source::SettingsChange => Target::SettingsChange,
        Source::ApplicationLaunch => Target::ApplicationLaunch,
        Source::PointerInput => Target::PointerInput,
        Source::KeyboardInput => Target::KeyboardInput,
        Source::ScreenCapture => Target::ScreenCapture,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner() -> WindowsRemoteControl {
        let (sender, receiver) = mpsc::sync_channel(16);
        WindowsRemoteControl {
            _transport: None,
            receiver,
            remote_control: RemoteControlRuntime::default(),
            local_cues: Default::default(),
            applications: Default::default(),
            resources: Default::default(),
            resource_lifecycle: crate::platform::remote_observation::Lifecycle::install().ok(),
            observation_generation: 0,
            indicators: Default::default(),
            authority: Arc::new(WindowsDesktopAuthority {
                sender,
                cleanup_wake: nickel_remote_control::ConnectionCleanupWake::new(|| true),
            }),
            start_time: Instant::now(),
            last_stop: None,
        }
    }
    #[test]
    fn windows_owner_preserves_pending_request_when_trusted_chrome_is_unavailable() {
        let mut owner = owner();
        let control = owner.remote_control.control();
        let request = nickel_remote_control::lease_requests::LeaseRequest {
            renewal: None,
            scope: nickel_remote_control::leases::ResourceScope::FullSession,
            duration: Some(Duration::from_secs(1200)),
            allow_resumption: false,
            full_debug: false,
        };
        let client = {
            let mut control = control.lock().unwrap();
            control.set_enabled(true);
            let client = control.connect_identity("Windows acceptance").unwrap();
            let now = Instant::now();
            let watch = control
                .reserve_connection_watch(&client.client_id, &client.token, now)
                .unwrap();
            control
                .activate_connection_watch(&client.client_id, &client.token, watch, false, now)
                .unwrap();
            control
                .request_lease(
                    &client.client_id,
                    &client.token,
                    request.clone(),
                    Instant::now(),
                )
                .unwrap();
            client
        };
        let pending_generation = control
            .lock()
            .unwrap()
            .lease_requests()
            .pending_generation(&client.client_id)
            .unwrap();
        let reply = owner.handle(Request::Command(Command::DecideRemoteLease {
            pending_generation,
            client_id: client.client_id.clone(),
            request: (&request).into(),
            allow: true,
        }));
        assert!(matches!(reply, ServerMessage::Error { .. }));
        assert_eq!(
            control.lock().unwrap().lease_requests().pending().count(),
            1
        );
        assert_eq!(control.lock().unwrap().leases().iter().count(), 0);
        let pending_generation = control
            .lock()
            .unwrap()
            .lease_requests()
            .pending_generation(&client.client_id)
            .unwrap();
        let reply = owner.handle(Request::Command(Command::DecideRemoteLease {
            pending_generation,
            client_id: client.client_id,
            request: (&request).into(),
            allow: false,
        }));
        let ServerMessage::RemoteControl(snapshot) = reply else {
            panic!("missing runtime observation");
        };
        assert!(snapshot.pending_leases.is_empty());
        drop(owner);
        assert!(!control.lock().unwrap().enabled());
    }
    #[test]
    fn expired_settings_request_cannot_change_owner_generation() {
        let mut owner = owner();
        let (reply, receiver) = mpsc::sync_channel(1);
        owner
            .authority
            .sender
            .try_send(OwnerRequest::Local(LocalRequest {
                envelope: nickel_session_protocol::ClientEnvelope {
                    token: String::new(),
                    request_id: 1,
                    request: Request::Command(Command::ApplyRemoteControl {
                        requested_enabled: false,
                        generation: 99,
                    }),
                },
                queued_at: Instant::now() - Duration::from_millis(2),
                deadline: Instant::now() - Duration::from_millis(1),
                reply,
            }))
            .ok()
            .unwrap();
        owner.poll();
        assert_eq!(owner.remote_control.status().generation, 0);
        assert!(receiver.try_recv().is_err());
    }
}
