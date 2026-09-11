//! Windows winit-owned remote-control lifecycle and private Settings requests.
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
static LOCAL_INPUT_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static EMERGENCY: std::sync::OnceLock<nickel_remote_control::EmergencyStopHandle> =
    std::sync::OnceLock::new();
static CHORD: crate::windows_emergency_chord::WindowsEmergencyChord =
    crate::windows_emergency_chord::WindowsEmergencyChord::new();

pub(crate) fn observe_physical_key(event: nickel_input::windows::NativeKeyboardEvent) {
    use std::sync::atomic::Ordering;
    if !event.injected {
        crate::windows_remote_input::release_all();
        advance_local_input_epoch();
    }
    if CHORD.observe(event) {
        if let Some(handle) = EMERGENCY.get() {
            handle.trigger();
        }
        STOP_REQUESTED.store(true, Ordering::Release);
    }
}

pub(crate) fn observe_physical_pointer(event: nickel_input::windows::NativePointerEvent) {
    if !event.injected {
        crate::windows_remote_input::release_all();
        advance_local_input_epoch();
    }
}

fn advance_local_input_epoch() {
    use std::sync::atomic::Ordering;
    if LOCAL_INPUT_EPOCH
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |epoch| {
            epoch.checked_add(1)
        })
        .is_err()
    {
        if let Some(handle) = EMERGENCY.get() {
            handle.trigger();
        }
        STOP_REQUESTED.store(true, Ordering::Release);
    }
}

fn local_input_epoch() -> u64 {
    LOCAL_INPUT_EPOCH.load(std::sync::atomic::Ordering::Acquire)
}

fn windows_key_chord(keysym: u32, modifiers: &[u32]) -> Result<Vec<u8>, String> {
    fn virtual_key(keysym: u32) -> Option<u8> {
        match keysym {
            0x20..=0x7e => match char::from_u32(keysym)? {
                'a'..='z' => Some((keysym as u8).to_ascii_uppercase()),
                'A'..='Z' | '0'..='9' => Some(keysym as u8),
                ' ' => Some(0x20),
                _ => None,
            },
            0xff08 => Some(0x08),
            0xff09 => Some(0x09),
            0xff0d => Some(0x0d),
            0xff1b => Some(0x1b),
            0xff50 => Some(0x24),
            0xff51 => Some(0x25),
            0xff52 => Some(0x26),
            0xff53 => Some(0x27),
            0xff54 => Some(0x28),
            0xff55 => Some(0x21),
            0xff56 => Some(0x22),
            0xff57 => Some(0x23),
            0xffff => Some(0x2e),
            0xffbe..=0xffc9 => Some(0x70 + (keysym - 0xffbe) as u8),
            _ => None,
        }
    }
    fn modifier(keysym: u32) -> Option<u8> {
        match keysym {
            0xffe1 => Some(0x10),
            0xffe3 => Some(0x11),
            0xffe9 => Some(0x12),
            0xffeb => Some(0x5b),
            0xfe03 => Some(0xa5),
            _ => None,
        }
    }
    let mut keys = Vec::with_capacity(modifiers.len() + 1);
    for keysym in modifiers {
        let key = modifier(*keysym).ok_or("unsupported Windows modifier")?;
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let key = virtual_key(keysym).ok_or("unsupported Windows keysym")?;
    if !keys.contains(&key) {
        keys.push(key);
    }
    Ok(keys)
}

fn windows_diagnostic_logs() -> Option<nickel_remote_control::diagnostics::DiagnosticLogSnapshot> {
    use nickel_remote_control::diagnostics::{DiagnosticLogRecord, DiagnosticLogSnapshot};
    let snapshot = nickel_logging::diagnostics::snapshot()?;
    Some(DiagnosticLogSnapshot {
        collecting: snapshot.collecting,
        generation: snapshot.generation,
        observed_at_us: snapshot.observed_at_us,
        evicted: snapshot.evicted,
        contention_drops: snapshot.contention_drops,
        records: snapshot
            .records
            .into_iter()
            .map(|record| DiagnosticLogRecord {
                generation: record.generation,
                observed_at_us: record.observed_at_us,
                level: record.level.to_owned(),
                target: record.target.chars().take(128).collect(),
                source_file: record.file.map(|file| file.chars().take(256).collect()),
                source_line: record.line,
            })
            .collect(),
    })
}

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
    CaptureSource { id: String, generation: u64 },
}
enum ObservationResult {
    Windows(Vec<nickel_remote_control::WindowSummary>),
    Outputs(nickel_remote_control::diagnostics::OutputInventory),
    CaptureSource(crate::windows_resource_owner::Window),
}
struct PointerOwnerAction {
    id: String,
    generation: u64,
    x: i32,
    y: i32,
    action: nickel_remote_control::pointer::PointerAction,
}
enum OwnerRequest {
    Local(LocalRequest),
    Observation {
        permit: DesktopPermit,
        prepared: Box<crate::platform::remote_observation::Prepared>,
        kind: ObservationKind,
        reply: SyncSender<Result<ObservationResult, String>>,
    },
    WindowAction {
        permit: DesktopPermit,
        prepared: Box<crate::platform::remote_observation::Prepared>,
        id: String,
        generation: u64,
        action: nickel_remote_control::window_actions::WindowAction,
        reply: SyncSender<Result<nickel_remote_control::window_actions::WindowOutcome, String>>,
    },
    Keyboard {
        permit: DesktopPermit,
        prepared: Box<crate::platform::remote_observation::Prepared>,
        id: String,
        generation: u64,
        action: nickel_remote_control::keyboard::KeyboardAction,
        reply: SyncSender<Result<(), String>>,
    },
    Pointer {
        permit: DesktopPermit,
        prepared: Box<crate::platform::remote_observation::Prepared>,
        request: PointerOwnerAction,
        reply: SyncSender<Result<(), String>>,
    },
    Diagnostic {
        permit: DesktopPermit,
        prepared: Box<crate::platform::remote_observation::Prepared>,
        reply: SyncSender<Result<nickel_remote_control::diagnostics::DiagnosticSnapshot, String>>,
    },
    DiagnosticAction {
        permit: DesktopPermit,
        action: nickel_remote_control::diagnostics::DiagnosticAction,
        reply:
            SyncSender<Result<nickel_remote_control::diagnostics::DiagnosticActionOutcome, String>>,
    },
    Events {
        permit: DesktopPermit,
        after: u64,
        reply: SyncSender<
            Result<nickel_remote_control::desktop_events::DesktopEventObservation, String>,
        >,
    },
    Applications {
        permit: DesktopPermit,
        reply: SyncSender<Result<nickel_remote_control::diagnostics::ApplicationInventory, String>>,
    },
    LaunchApplication {
        permit: DesktopPermit,
        request: nickel_remote_control::diagnostics::LaunchApplicationRequest,
        reply: SyncSender<
            Result<nickel_remote_control::diagnostics::LaunchApplicationOutcome, String>,
        >,
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
    started: Instant,
    desktop_session: Option<u32>,
    capture_generation: std::sync::atomic::AtomicU64,
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
    fn read_desktop_events(
        &self,
        permit: DesktopPermit,
        after: u64,
    ) -> Result<nickel_remote_control::desktop_events::DesktopEventObservation, String> {
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::Events {
                permit,
                after,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows desktop owner timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn list_installed_applications(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::diagnostics::ApplicationInventory, String> {
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::Applications { permit, reply })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows desktop owner timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn launch_installed_application(
        &self,
        permit: DesktopPermit,
        request: nickel_remote_control::diagnostics::LaunchApplicationRequest,
    ) -> Result<nickel_remote_control::diagnostics::LaunchApplicationOutcome, String> {
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::LaunchApplication {
                permit,
                request,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows desktop owner timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn keyboard_action(
        &self,
        permit: DesktopPermit,
        id: &str,
        generation: u64,
        action: nickel_remote_control::keyboard::KeyboardAction,
    ) -> Result<(), String> {
        let _admission = crate::platform::remote_observation::Admission::acquire()?;
        let prepared = Box::new(crate::platform::remote_observation::Prepared::prepare(
            &permit,
        )?);
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::Keyboard {
                permit,
                prepared,
                id: id.to_owned(),
                generation,
                action,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows desktop owner timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn pointer_action(
        &self,
        permit: DesktopPermit,
        id: &str,
        generation: u64,
        x: i32,
        y: i32,
        action: nickel_remote_control::pointer::PointerAction,
    ) -> Result<(), String> {
        let _admission = crate::platform::remote_observation::Admission::acquire()?;
        let prepared = Box::new(crate::platform::remote_observation::Prepared::prepare(
            &permit,
        )?);
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::Pointer {
                permit,
                prepared,
                request: PointerOwnerAction {
                    id: id.to_owned(),
                    generation,
                    x,
                    y,
                    action,
                },
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows desktop owner timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn diagnostic_snapshot(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::diagnostics::DiagnosticSnapshot, String> {
        let _admission = crate::platform::remote_observation::Admission::acquire()?;
        let prepared = Box::new(crate::platform::remote_observation::Prepared::prepare(
            &permit,
        )?);
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::Diagnostic {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows desktop owner timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn diagnostic_action(
        &self,
        permit: DesktopPermit,
        action: nickel_remote_control::diagnostics::DiagnosticAction,
    ) -> Result<nickel_remote_control::diagnostics::DiagnosticActionOutcome, String> {
        action.validate()?;
        permit.with_debug(false, || Ok(()))?;
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::DiagnosticAction {
                permit,
                action,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows desktop owner timed out".to_owned())?;
        completion.check_live()?;
        result
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
    fn validate_window_capture(
        &self,
        permit: DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<(), String> {
        match self.observe(
            permit,
            ObservationKind::CaptureSource {
                id: id.into(),
                generation,
            },
        )? {
            ObservationResult::CaptureSource(_) => Ok(()),
            _ => Err("Windows observation mismatch".into()),
        }
    }
    fn capture_window(
        &self,
        permit: DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<nickel_remote_control::capture::CapturedWindow, String> {
        let submitted_at_us = self.started.elapsed().as_micros().min(u64::MAX as u128) as u64;
        let source = match self.observe(
            permit.clone(),
            ObservationKind::CaptureSource {
                id: id.into(),
                generation,
            },
        )? {
            ObservationResult::CaptureSource(source) => source,
            _ => return Err("Windows observation mismatch".into()),
        };
        permit.check_live()?;
        let session = self
            .desktop_session
            .ok_or("Windows desktop evidence is unavailable")?;
        let image = crate::platform::remote_observation::capture_window_client(&source, session)?;
        permit.check_live()?;
        self.validate_window_capture(permit.clone(), id, generation)?;
        let capture_generation = self
            .capture_generation
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |value| value.checked_add(1),
            )
            .map_err(|_| "Windows capture generations exhausted")?
            + 1;
        Ok(nickel_remote_control::capture::CapturedWindow {
            window_id: id.into(),
            generation,
            capture_generation,
            submitted_at_us,
            completed_at_us: self.started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            width: image.width() as u16,
            height: image.height() as u16,
            rgba: image.into_raw(),
        })
    }
    fn window_action(
        &self,
        permit: DesktopPermit,
        id: &str,
        generation: u64,
        action: nickel_remote_control::window_actions::WindowAction,
    ) -> Result<nickel_remote_control::window_actions::WindowOutcome, String> {
        action.validate()?;
        let _admission = crate::platform::remote_observation::Admission::acquire()?;
        let prepared = Box::new(crate::platform::remote_observation::Prepared::prepare(
            &permit,
        )?);
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::WindowAction {
                permit,
                prepared,
                id: id.to_owned(),
                generation,
                action,
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
    desktop_session: Option<u32>,
    desktop_unlocked: bool,
    local_input_epoch: u64,
    keyboard_hold: Option<WindowsKeyboardHold>,
    pointer_hold: Option<WindowsPointerHold>,
    desktop_events: nickel_remote_control::desktop_events::DesktopEvents,
    pending_indicator_activation: std::collections::BTreeSet<u64>,
    start_time: Instant,
    last_stop: Option<Instant>,
}
struct WindowsKeyboardHold {
    authority: nickel_remote_control::HeldInput,
    keys: Vec<u8>,
    window_id: String,
    generation: u64,
    native: usize,
    deadline: Instant,
}
struct WindowsPointerHold {
    authority: nickel_remote_control::HeldInput,
    button: nickel_remote_control::pointer::PointerButton,
    window_id: String,
    generation: u64,
    native: usize,
    deadline: Instant,
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
        let desktop_session =
            nickel_platform::process_identity::WindowsProcessIdentity::probe(std::process::id())
                .ok()
                .map(|identity| identity.session_id());
        let started = Instant::now();
        let authority = Arc::new(WindowsDesktopAuthority {
            cleanup_wake,
            sender: sender.clone(),
            started,
            desktop_session,
            capture_generation: std::sync::atomic::AtomicU64::new(0),
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
        let desktop_unlocked =
            desktop_session.is_some_and(crate::platform::remote_observation::desktop_is_unlocked);
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
            desktop_session,
            desktop_unlocked,
            local_input_epoch: local_input_epoch(),
            keyboard_hold: None,
            pointer_hold: None,
            desktop_events: Default::default(),
            pending_indicator_activation: Default::default(),
            start_time: started,
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
    pub(crate) fn poll(&mut self, shell: &mut WinitShell) {
        self.poll_with_shell(Some(shell));
    }

    fn poll_with_shell(&mut self, mut shell: Option<&mut WinitShell>) {
        self.reconcile_desktop_authority();
        self.reconcile_local_input();
        self.reconcile_keyboard_hold();
        self.reconcile_pointer_hold();
        self.sync_input_ownership_event();
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
            self.reconcile_keyboard_hold();
            self.reconcile_pointer_hold();
        }
        if STOP_REQUESTED.swap(false, std::sync::atomic::Ordering::AcqRel) {
            crate::windows_remote_input::release_all();
            self.keyboard_hold.take();
            self.pointer_hold.take();
            self.handle(Request::Command(Command::EmergencyStopRemoteControl));
        }
        self.drain_resource_lifecycle();
        self.reconcile_keyboard_hold();
        self.reconcile_pointer_hold();
        // Catalog and launch receipts are owner evidence used by the fresh
        // resource probe below, so consume them before trusted decisions.
        if let Ok(mut control) = self.remote_control.control().lock() {
            self.applications.poll(&mut control);
        }
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
                OwnerRequest::WindowAction {
                    permit,
                    prepared,
                    id,
                    generation,
                    action,
                    reply,
                } => {
                    let result =
                        self.perform_window_action(permit, *prepared, &id, generation, action);
                    let _ = reply.try_send(result);
                }
                OwnerRequest::Keyboard {
                    permit,
                    prepared,
                    id,
                    generation,
                    action,
                    reply,
                } => {
                    let result =
                        self.perform_keyboard_action(permit, *prepared, &id, generation, action);
                    self.sync_input_ownership_event();
                    let _ = reply.try_send(result);
                }
                OwnerRequest::Pointer {
                    permit,
                    prepared,
                    request,
                    reply,
                } => {
                    let result = self.perform_pointer_action(permit, *prepared, request);
                    self.sync_input_ownership_event();
                    let _ = reply.try_send(result);
                }
                OwnerRequest::Diagnostic {
                    permit,
                    prepared,
                    reply,
                } => {
                    let result = self.perform_diagnostic_snapshot(permit, *prepared);
                    let _ = reply.try_send(result);
                }
                OwnerRequest::DiagnosticAction {
                    permit,
                    action,
                    reply,
                } => {
                    let result = shell.as_deref_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |shell| self.perform_diagnostic_action(shell, permit, action),
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::Events {
                    permit,
                    after,
                    reply,
                } => {
                    let result = self.read_desktop_events(permit, after);
                    let _ = reply.try_send(result);
                }
                OwnerRequest::Applications { permit, reply } => {
                    let result = self.application_inventory(permit);
                    let _ = reply.try_send(result);
                }
                OwnerRequest::LaunchApplication {
                    permit,
                    request,
                    reply,
                } => {
                    let result = self.prepare_application_launch(permit, request);
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
                    // Recheck at the commit boundary. A stale unlocked sample may
                    // only deny a connection; it must never restore authority on a
                    // protected input desktop.
                    let locked = !self
                        .desktop_session
                        .is_some_and(crate::platform::remote_observation::desktop_is_unlocked);
                    let _ = reply.try_send(permit.apply(action, locked));
                }
            }
        }
        let control = self.remote_control.control();
        if let Ok(mut control) = control.lock() {
            self.applications.poll(&mut control);
            self.local_cues.update(control.leases(), Instant::now());
        }
        self.sync_input_ownership_event();
    }
    fn sync_input_ownership_event(&mut self) {
        self.desktop_events.record_remote_input_ownership(
            self.keyboard_hold.is_some(),
            self.pointer_hold.is_some(),
            self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
        );
    }
    fn reconcile_local_input(&mut self) {
        let observed = local_input_epoch();
        self.reconcile_local_input_observation(observed);
    }
    fn reconcile_local_input_observation(&mut self, observed: u64) {
        if observed == self.local_input_epoch {
            return;
        }
        self.local_input_epoch = observed;
        crate::windows_remote_input::release_all();
        self.keyboard_hold.take();
        self.pointer_hold.take();
        if let Ok(mut control) = self.remote_control.control().lock() {
            // The hook has already released registered keys. Clear shared
            // ownership so no continuation can blend with this newly observed
            // physical key or pointer event.
            control.leases_mut().cancel_input();
        }
    }
    fn reconcile_keyboard_hold(&mut self) {
        let expired = self.keyboard_hold.as_ref().is_some_and(|held| {
            Instant::now() >= held.deadline
                || held.authority.check_live().is_err()
                || self
                    .resources
                    .window(&held.window_id, held.generation)
                    .is_none_or(|window| window.native != held.native)
                || !crate::windows_remote_input::foreground_is(held.native)
        });
        if expired {
            crate::windows_remote_input::release_all();
            self.keyboard_hold.take();
        }
    }
    fn reconcile_pointer_hold(&mut self) {
        let expired = self.pointer_hold.as_ref().is_some_and(|held| {
            Instant::now() >= held.deadline
                || held.authority.check_live().is_err()
                || self
                    .resources
                    .window(&held.window_id, held.generation)
                    .is_none_or(|window| window.native != held.native)
        });
        if expired {
            crate::windows_remote_input::release_all();
            self.pointer_hold.take();
        }
    }
    fn reconcile_desktop_authority(&mut self) {
        let unlocked = self
            .desktop_session
            .is_some_and(crate::platform::remote_observation::desktop_is_unlocked);
        self.reconcile_desktop_authority_observation(unlocked);
    }
    fn reconcile_desktop_authority_observation(&mut self, unlocked: bool) {
        if self.desktop_unlocked && !unlocked {
            // Revoke before any queued owner work can run. ControlPlane::lock
            // cancels pending requests, leases, watches, traces, streams, held
            // input ownership and every outstanding operation permit.
            self.last_stop = Some(Instant::now());
            self.remote_control.lock();
            let control = self.remote_control.control();
            self.resources
                .clear(|id| revoke_native_resource(&control, id));
        }
        self.desktop_unlocked = unlocked;
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
        self.reconcile_prepared_resources(&permit, &mut prepared)?;
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
            ObservationKind::CaptureSource { id, generation } => {
                let (window, evidence) = self
                    .resources
                    .window_resource(&scope, &id, generation)
                    .ok_or("Windows resource is unavailable")?;
                let window = permit.with_resource(&evidence, || Ok(window.clone()))?;
                ObservationResult::CaptureSource(window)
            }
        };
        prepared.revalidate()?;
        permit.check_live()?;
        Ok(result)
    }

    fn reconcile_prepared_resources(
        &mut self,
        permit: &DesktopPermit,
        prepared: &mut crate::platform::remote_observation::Prepared,
    ) -> Result<(), String> {
        permit.check_live()?;
        self.reconcile_native_resources(prepared)?;
        permit.check_live()
    }

    fn reconcile_native_resources(
        &mut self,
        prepared: &mut crate::platform::remote_observation::Prepared,
    ) -> Result<(), String> {
        self.drain_resource_lifecycle();
        self.reconcile_keyboard_hold();
        self.reconcile_pointer_hold();
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
        prepared.revalidate()
    }

    /// Refresh native evidence at the trusted local decision boundary. A
    /// cached card or generation can only be approved while that exact scope is
    /// still represented by the owner after a bounded Win32 re-observation.
    fn remote_lease_target_live(
        &mut self,
        scope: &nickel_remote_control::leases::ResourceScope,
    ) -> bool {
        if !self.desktop_unlocked
            || self.desktop_session.is_none_or(|session| {
                !crate::platform::remote_observation::desktop_is_unlocked(session)
            })
        {
            return false;
        }
        let Ok(_admission) = crate::platform::remote_observation::Admission::acquire() else {
            return false;
        };
        let Ok(mut prepared) = crate::platform::remote_observation::Prepared::prepare_local()
        else {
            return false;
        };
        if Some(prepared.session) != self.desktop_session
            || self.reconcile_native_resources(&mut prepared).is_err()
        {
            return false;
        }
        match scope {
            nickel_remote_control::leases::ResourceScope::Application(identity) => {
                self.applications.contains_application(identity)
                    || self.resources.scope_is_live(scope)
            }
            _ => self.resources.scope_is_live(scope),
        }
    }

    fn perform_keyboard_action(
        &mut self,
        permit: DesktopPermit,
        mut prepared: crate::platform::remote_observation::Prepared,
        id: &str,
        generation: u64,
        action: nickel_remote_control::keyboard::KeyboardAction,
    ) -> Result<(), String> {
        self.reconcile_prepared_resources(&permit, &mut prepared)?;
        let scope = permit.resource_scope()?;
        let (window, evidence) = self
            .resources
            .window_resource(&scope, id, generation)
            .ok_or("Windows resource is unavailable")?;
        if !window.active {
            return Err("Windows keyboard target is not focused".into());
        }
        let native = window.native;
        if !crate::windows_remote_input::foreground_is(native) {
            if self
                .keyboard_hold
                .as_ref()
                .is_some_and(|held| held.window_id == id && held.generation == generation)
            {
                crate::windows_remote_input::release_all();
                self.keyboard_hold.take();
            }
            return Err("Windows keyboard target lost focus".into());
        }
        match action {
            nickel_remote_control::keyboard::KeyboardAction::Text { text } => {
                if self.keyboard_hold.is_some()
                    || !crate::windows_remote_input::physical_input_idle()
                {
                    return Err("local or remote input is already active".into());
                }
                let input_epoch = local_input_epoch();
                permit.with_input(&evidence, || {
                    if local_input_epoch() != input_epoch
                        || !crate::windows_remote_input::foreground_is(native)
                    {
                        return Err("local input or focus change cancelled the transaction".into());
                    }
                    crate::windows_remote_input::send_text(&text)?;
                    if local_input_epoch() != input_epoch
                        || !crate::windows_remote_input::foreground_is(native)
                    {
                        return Err(
                            "local input or focus change interrupted the transaction".into()
                        );
                    }
                    Ok(())
                })
            }
            nickel_remote_control::keyboard::KeyboardAction::Key { keysym, modifiers } => {
                if self.keyboard_hold.is_some()
                    || !crate::windows_remote_input::physical_input_idle()
                {
                    return Err("local or remote input is already active".into());
                }
                let keys = windows_key_chord(keysym, &modifiers)?;
                let input_epoch = local_input_epoch();
                permit.with_input(&evidence, || {
                    if local_input_epoch() != input_epoch
                        || !crate::windows_remote_input::foreground_is(native)
                    {
                        return Err("local input or focus change cancelled the transaction".into());
                    }
                    crate::windows_remote_input::press_keys(&keys)?;
                    let collision = local_input_epoch() != input_epoch
                        || !crate::windows_remote_input::foreground_is(native);
                    let released = crate::windows_remote_input::release_keys(&keys);
                    if collision || released.is_err() {
                        crate::windows_remote_input::release_all();
                        return Err(released.err().unwrap_or_else(|| {
                            "local input or focus change interrupted the transaction".into()
                        }));
                    }
                    Ok(())
                })
            }
            nickel_remote_control::keyboard::KeyboardAction::HoldStart { keysym, modifiers } => {
                if self.keyboard_hold.is_some()
                    || !crate::windows_remote_input::physical_input_idle()
                {
                    return Err("local or remote input is already active".into());
                }
                let keys = windows_key_chord(keysym, &modifiers)?;
                let input_epoch = local_input_epoch();
                let authority = permit.begin_input(&evidence, || {
                    if local_input_epoch() != input_epoch
                        || !crate::windows_remote_input::foreground_is(native)
                    {
                        return Err("local input or focus change cancelled the gesture".into());
                    }
                    crate::windows_remote_input::press_keys(&keys)?;
                    if local_input_epoch() != input_epoch
                        || !crate::windows_remote_input::foreground_is(native)
                    {
                        crate::windows_remote_input::release_all();
                        return Err("local input or focus change interrupted the gesture".into());
                    }
                    Ok(())
                })?;
                self.keyboard_hold = Some(WindowsKeyboardHold {
                    authority,
                    keys,
                    window_id: id.to_owned(),
                    generation,
                    native,
                    deadline: Instant::now() + Duration::from_secs(30),
                });
                Ok(())
            }
            nickel_remote_control::keyboard::KeyboardAction::HoldKeepAlive => {
                let held = self
                    .keyboard_hold
                    .as_mut()
                    .ok_or("no Windows keyboard gesture is active")?;
                if held.window_id != id
                    || held.generation != generation
                    || !held.authority.owned_by(&permit)
                {
                    return Err("request does not own this input gesture".into());
                }
                permit.continue_input(&held.authority, &evidence, || Ok(()))?;
                held.deadline = Instant::now() + Duration::from_secs(30);
                Ok(())
            }
            nickel_remote_control::keyboard::KeyboardAction::HoldEnd
            | nickel_remote_control::keyboard::KeyboardAction::HoldCancel => {
                let owned = self.keyboard_hold.as_ref().is_some_and(|held| {
                    held.window_id == id
                        && held.generation == generation
                        && held.authority.owned_by(&permit)
                });
                if !owned {
                    return Err("request does not own this input gesture".into());
                }
                let held = self.keyboard_hold.take().unwrap();
                let result = permit.continue_input(&held.authority, &evidence, || {
                    crate::windows_remote_input::release_keys(&held.keys)
                });
                if result.is_err() {
                    crate::windows_remote_input::release_all();
                }
                result
            }
        }
    }

    fn perform_pointer_action(
        &mut self,
        permit: DesktopPermit,
        mut prepared: crate::platform::remote_observation::Prepared,
        request: PointerOwnerAction,
    ) -> Result<(), String> {
        let PointerOwnerAction {
            id,
            generation,
            x,
            y,
            action,
        } = request;
        self.reconcile_prepared_resources(&permit, &mut prepared)?;
        let scope = permit.resource_scope()?;
        let (window, evidence) = self
            .resources
            .window_resource(&scope, &id, generation)
            .ok_or("Windows resource is unavailable")?;
        let native = window.native;

        match action {
            nickel_remote_control::pointer::PointerAction::Move
            | nickel_remote_control::pointer::PointerAction::Click { .. }
            | nickel_remote_control::pointer::PointerAction::DoubleClick { .. }
            | nickel_remote_control::pointer::PointerAction::Scroll { .. } => {
                if self.keyboard_hold.is_some()
                    || self.pointer_hold.is_some()
                    || !crate::windows_remote_input::physical_input_idle()
                {
                    return Err("local or remote input is already active".into());
                }
                let input_epoch = local_input_epoch();
                permit.with_input(&evidence, || {
                    if local_input_epoch() != input_epoch {
                        return Err("local input cancelled the pointer transaction".into());
                    }
                    let point = crate::windows_remote_input::target_point(native, x, y)?;
                    crate::windows_remote_input::move_pointer(point.0, point.1)?;
                    if local_input_epoch() != input_epoch {
                        return Err("local input interrupted the pointer transaction".into());
                    }
                    match action {
                        nickel_remote_control::pointer::PointerAction::Move => Ok(()),
                        nickel_remote_control::pointer::PointerAction::Click { button } => {
                            crate::windows_remote_input::target_point(native, x, y)?;
                            crate::windows_remote_input::click(button, 1)
                        }
                        nickel_remote_control::pointer::PointerAction::DoubleClick { button } => {
                            crate::windows_remote_input::target_point(native, x, y)?;
                            crate::windows_remote_input::click(button, 2)
                        }
                        nickel_remote_control::pointer::PointerAction::Scroll {
                            horizontal_v120,
                            vertical_v120,
                        } => {
                            crate::windows_remote_input::target_point(native, x, y)?;
                            crate::windows_remote_input::scroll(horizontal_v120, vertical_v120)
                        }
                        _ => unreachable!("held pointer actions use the other owner branch"),
                    }?;
                    if local_input_epoch() != input_epoch {
                        crate::windows_remote_input::release_all();
                        return Err("local input interrupted the pointer transaction".into());
                    }
                    Ok(())
                })
            }
            nickel_remote_control::pointer::PointerAction::DragStart { button } => {
                if self.keyboard_hold.is_some()
                    || self.pointer_hold.is_some()
                    || !crate::windows_remote_input::physical_input_idle()
                {
                    return Err("local or remote input is already active".into());
                }
                let input_epoch = local_input_epoch();
                let authority = permit.begin_input(&evidence, || {
                    if local_input_epoch() != input_epoch {
                        return Err("local input cancelled the pointer gesture".into());
                    }
                    let point = crate::windows_remote_input::target_point(native, x, y)?;
                    crate::windows_remote_input::move_pointer(point.0, point.1)?;
                    if local_input_epoch() != input_epoch {
                        return Err("local input interrupted the pointer gesture".into());
                    }
                    crate::windows_remote_input::press_button(button)?;
                    if local_input_epoch() != input_epoch {
                        crate::windows_remote_input::release_all();
                        return Err("local input interrupted the pointer gesture".into());
                    }
                    Ok(())
                })?;
                self.pointer_hold = Some(WindowsPointerHold {
                    authority,
                    button,
                    window_id: id.clone(),
                    generation,
                    native,
                    deadline: Instant::now() + Duration::from_secs(30),
                });
                Ok(())
            }
            nickel_remote_control::pointer::PointerAction::DragMove => {
                let held = self
                    .pointer_hold
                    .as_mut()
                    .ok_or("no Windows pointer gesture is active")?;
                if held.window_id != id
                    || held.generation != generation
                    || held.native != native
                    || !held.authority.owned_by(&permit)
                {
                    return Err("request does not own this input gesture".into());
                }
                let input_epoch = local_input_epoch();
                permit.continue_input(&held.authority, &evidence, || {
                    if local_input_epoch() != input_epoch {
                        return Err("local input cancelled the pointer gesture".into());
                    }
                    let point = crate::windows_remote_input::target_point(native, x, y)?;
                    crate::windows_remote_input::move_pointer(point.0, point.1)?;
                    if local_input_epoch() != input_epoch {
                        crate::windows_remote_input::release_all();
                        return Err("local input interrupted the pointer gesture".into());
                    }
                    Ok(())
                })?;
                held.deadline = Instant::now() + Duration::from_secs(30);
                Ok(())
            }
            nickel_remote_control::pointer::PointerAction::DragEnd
            | nickel_remote_control::pointer::PointerAction::DragCancel => {
                let owned = self.pointer_hold.as_ref().is_some_and(|held| {
                    held.window_id == id
                        && held.generation == generation
                        && held.native == native
                        && held.authority.owned_by(&permit)
                });
                if !owned {
                    return Err("request does not own this input gesture".into());
                }
                let held = self.pointer_hold.take().unwrap();
                let cancelled = matches!(
                    action,
                    nickel_remote_control::pointer::PointerAction::DragCancel
                );
                let input_epoch = local_input_epoch();
                let result = permit.continue_input(&held.authority, &evidence, || {
                    if !cancelled {
                        if local_input_epoch() != input_epoch {
                            return Err("local input cancelled the pointer gesture".into());
                        }
                        let point = crate::windows_remote_input::target_point(native, x, y)?;
                        crate::windows_remote_input::move_pointer(point.0, point.1)?;
                    }
                    let released = crate::windows_remote_input::release_button(held.button);
                    if !cancelled && local_input_epoch() != input_epoch {
                        crate::windows_remote_input::release_all();
                        return Err("local input interrupted the pointer gesture".into());
                    }
                    released
                });
                if result.is_err() {
                    crate::windows_remote_input::release_all();
                }
                result
            }
        }
    }

    fn perform_diagnostic_snapshot(
        &mut self,
        permit: DesktopPermit,
        mut prepared: crate::platform::remote_observation::Prepared,
    ) -> Result<nickel_remote_control::diagnostics::DiagnosticSnapshot, String> {
        use nickel_remote_control::diagnostics::*;

        self.reconcile_prepared_resources(&permit, &mut prepared)?;
        let observed_at_us = self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64;
        let lease_metrics = permit.lease_metrics_snapshot(observed_at_us);
        let scope = permit.resource_scope()?;
        let snapshot = permit.with_debug(!self.desktop_unlocked, || {
            self.observation_generation = self
                .observation_generation
                .checked_add(1)
                .ok_or("Windows observation generations exhausted")?;
            let generation = self.observation_generation;
            let windows: Vec<_> = self
                .resources
                .windows(&scope)
                .map(|(window, _)| window)
                .collect();
            let outputs: Vec<_> = self
                .resources
                .outputs(&scope)
                .map(|(output, _)| output)
                .collect();
            let mut active = windows.iter().filter(|window| window.active);
            let focused_window = active.next().map(|window| window.id.clone());
            let focused_window = active.next().is_none().then_some(focused_window).flatten();
            let keyboard_held = self.keyboard_hold.is_some();
            let pointer_held = self.pointer_hold.is_some();
            let keyboard = focused_window.as_ref().map(|window| InputDeviceDiagnostic {
                focused_window: Some(window.clone()),
                focused_surface: None,
                compositor_grabbed: false,
                remote_hold_active: keyboard_held,
            });

            let pointer = self
                .pointer_hold
                .as_ref()
                .map(|held| InputDeviceDiagnostic {
                    focused_window: Some(held.window_id.clone()),
                    focused_surface: None,
                    compositor_grabbed: false,
                    remote_hold_active: pointer_held,
                });
            Ok(DiagnosticSnapshot {
                observation_generation: generation,
                observed_at_us,
                windows,
                outputs,
                workspaces: Vec::new(),
                internal_applications: Vec::new(),
                internal_renderers: Vec::new(),
                shell_renderers: Vec::new(),
                shell_image_cache: None,
                projected_resources: ProjectedResourceDiagnostic {
                    observation_generation: generation,
                    observed_at_us,
                    renderer_surfaces: 0,
                    software_frame_bytes: 0,
                    fallback_raster_bytes: 0,
                    shell_image_entries: 0,
                    shell_image_bytes: 0,
                },
                pending_effects: PendingEffectsDiagnostic {
                    observation_generation: generation,
                    observed_at_us,
                    desktop_scene_updates: 0,
                    image_copy_frames: 0,
                    launch_observations: 0,
                    output_retirements: 0,
                    shell_focus_pending: false,
                },
                shell_surfaces: Vec::new(),
                focused_window,
                input: InputDiagnostic {
                    observation_generation: generation,
                    observed_at_us,
                    keyboard,
                    pointer,
                    pointer_hit_test: None,
                },
                shortcuts: ShortcutDiagnostic {
                    observation_generation: generation,
                    observed_at_us,
                    registration_revision: None,
                    capability: ShortcutDiagnosticCapability::BackendUnavailable,
                    registrations: Vec::new(),
                    unprojected_bindings: 0,
                    truncated: false,
                },
                stacking_front_to_back: Vec::new(),
                preview: PreviewDiagnostic {
                    presentation_generation: 0,
                    readback_bytes: 0,
                    capture_failures: 0,
                },
                metrics: permit.operation_metrics_snapshot(),
                admission: permit.admission_snapshot(),
                lease_metrics,
                platform: PlatformDiagnostic {
                    observation_generation: generation,
                    observed_at_us,
                    backend: Some(CompositorBackend::Winit),
                    keyboard_present: true,
                    pointer_present: true,
                    touch_present: false,
                    xwayland_connected: false,
                    xwayland_restart_pending: false,
                    isolated_x11_keyboard_initialized: false,
                    native_keyboard_worker_initialized: false,
                },
                platform_refreshes: Vec::new(),
                application_inventory_refresh: None,
                codex_feature: None,
                shell_behavior: ShellBehaviorDiagnostic {
                    observation_generation: generation,
                    observed_at_us,
                    topology_generation: 0,
                    bar_on_all_displays: false,
                    all_windows_on_every_bar: false,
                    configured_desktop_count: 0,
                    runtime_desktop_count: 0,
                },
                settings_worker: None,
                diagnostic_worker: None,
                application_launch: ApplicationLaunchDiagnostic {
                    preparation: None,
                    tracked_children: 0,
                    child_capacity: 0,
                },
                external_accessibility: None,
                recent_events: self.desktop_events.snapshot(),
                diagnostic_logs: windows_diagnostic_logs(),
                frame_trace: None,
                trace_lifecycle: trace_lifecycle_snapshot(&permit, self.start_time),
                truncated: false,
                unavailable_domains: vec![
                    "windows_virtual_workspaces".into(),
                    "windows_internal_application_and_shell_surfaces".into(),
                    "windows_renderer_and_resource_accounting".into(),
                    "windows_pointer_recipient_and_hit_testing".into(),
                    "windows_shortcut_inventory".into(),
                    "windows_preview_state".into(),
                    "windows_platform_refreshes".into(),
                    "windows_shell_behavior".into(),
                    "windows_settings_and_diagnostic_workers".into(),
                    "windows_application_launch_state".into(),
                    "windows_external_accessibility".into(),
                    "windows_frame_trace".into(),
                ],
            })
        })?;
        prepared.revalidate()?;
        if self
            .resource_lifecycle
            .as_ref()
            .is_none_or(|lifecycle| lifecycle.serial() != prepared.serial)
        {
            return Err("Windows resources changed during diagnostic collection".into());
        }
        permit.check_live()?;
        Ok(snapshot)
    }

    fn perform_diagnostic_action(
        &mut self,
        shell: &mut WinitShell,
        permit: DesktopPermit,
        action: nickel_remote_control::diagnostics::DiagnosticAction,
    ) -> Result<nickel_remote_control::diagnostics::DiagnosticActionOutcome, String> {
        use nickel_remote_control::desktop_events::{
            DesktopEventKind, ProductionEffectKind, ProductionEffectOutcome,
        };
        use nickel_remote_control::diagnostics::{DiagnosticAction, DiagnosticActionOutcome};

        action.validate()?;
        permit.with_debug(!self.desktop_unlocked, || {
            if matches!(
                &action,
                DiagnosticAction::Repaint
                    | DiagnosticAction::RefreshScene
                    | DiagnosticAction::RefreshApplicationInventory
            ) {
                Ok(())
            } else {
                Err("diagnostic action is unavailable on the Windows backend".into())
            }
        })?;
        permit.check_live()?;
        match &action {
            DiagnosticAction::Repaint => shell.request_all_redraws(),
            DiagnosticAction::RefreshScene => {
                let mut prepared = crate::platform::remote_observation::Prepared::prepare(&permit)?;
                self.reconcile_native_resources(&mut prepared)?;
                permit.check_live()?;
                shell.request_all_redraws();
            }
            DiagnosticAction::RefreshApplicationInventory => {
                self.applications.request_refresh();
            }
            _ => unreachable!("unsupported diagnostic action rejected above"),
        }
        permit.check_live()?;
        self.observation_generation = self
            .observation_generation
            .checked_add(1)
            .ok_or("Windows observation generations exhausted")?;
        let submitted_at_us = self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64;
        if let Some(operation_id) = permit.operation_id() {
            self.desktop_events.record(
                DesktopEventKind::ProductionEffectCompleted {
                    operation_id,
                    effect: ProductionEffectKind::DiagnosticAction,
                    outcome: ProductionEffectOutcome::Requested,
                },
                submitted_at_us,
            );
        }
        Ok(DiagnosticActionOutcome {
            action,
            observation_generation: self.observation_generation,
            submitted_at_us,
            presentation_confirmed: false,
            output_identification: None,
            application_inventory_refresh: None,
            platform_refresh: None,
        })
    }

    fn read_desktop_events(
        &mut self,
        permit: DesktopPermit,
        after: u64,
    ) -> Result<nickel_remote_control::desktop_events::DesktopEventObservation, String> {
        permit.with_debug(!self.desktop_unlocked, || {
            self.observation_generation = self
                .observation_generation
                .checked_add(1)
                .ok_or("Windows observation generations exhausted")?;
            let observed_at_us = self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64;
            Ok(
                nickel_remote_control::desktop_events::DesktopEventObservation {
                    observation_generation: self.observation_generation,
                    observed_at_us,
                    history: self.desktop_events.since(after)?,
                },
            )
        })
    }

    fn application_inventory(
        &mut self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::diagnostics::ApplicationInventory, String> {
        use nickel_remote_control::leases::{ResourceEvidence, ResourceScope};

        if !self.desktop_unlocked {
            return Err("Windows input desktop is protected".into());
        }
        let scope = permit.resource_scope()?;
        let expected = match &scope {
            ResourceScope::FullSession => None,
            ResourceScope::Application(identity) => Some(identity.as_str()),
            ResourceScope::Surface(_) | ResourceScope::Window(_) | ResourceScope::Output(_) => {
                return Err("this Windows lease cannot enumerate launch targets".into());
            }
        };
        self.observation_generation = self
            .observation_generation
            .checked_add(1)
            .ok_or("Windows observation generations exhausted")?;
        let inventory =
            self.applications
                .inventory(self.start_time, self.observation_generation, expected);
        let evidence = ResourceEvidence {
            surface: None,
            window: None,
            verified_application: expected,
            output: None,
            authorized_surface_ancestors: &[],
            protected: false,
        };
        permit.with_resource(&evidence, || Ok(inventory))
    }

    fn prepare_application_launch(
        &mut self,
        permit: DesktopPermit,
        request: nickel_remote_control::diagnostics::LaunchApplicationRequest,
    ) -> Result<nickel_remote_control::diagnostics::LaunchApplicationOutcome, String> {
        use nickel_remote_control::leases::{ResourceEvidence, ResourceScope};

        if !self.desktop_unlocked {
            return Err("Windows input desktop is protected".into());
        }
        if request.application_id.is_empty() || request.application_id.len() > 512 {
            return Err("invalid application launch target".into());
        }
        let capture = self
            .applications
            .prepare_launch(request.catalog_generation, &request.application_id)?;
        let staged = crate::windows_launch_broker::StagedLaunch::new(capture, Instant::now());
        let staged_identity = staged.application_identity().to_owned();
        let scope = permit.resource_scope()?;
        let expected = match &scope {
            ResourceScope::FullSession => None,
            ResourceScope::Application(identity) => Some(identity.as_str()),
            ResourceScope::Output(_) => {
                return Err(
                    "Windows output-scoped launch awaits verified placement support".into(),
                );
            }
            ResourceScope::Surface(_) | ResourceScope::Window(_) => {
                return Err("this Windows lease cannot launch applications".into());
            }
        };
        if expected.is_some_and(|expected| expected != staged_identity) {
            return Err("installed launch target is outside the application lease".into());
        }
        let evidence = ResourceEvidence {
            surface: None,
            window: None,
            verified_application: Some(&staged_identity),
            output: None,
            authorized_surface_ancestors: &[],
            protected: false,
        };
        let _capture = staged.commit(&permit, &evidence, Instant::now(), || {
            // Broker construction and inherited-handle transfer are not yet
            // available. Refuse before a process can be resumed.
            Err("Windows launch broker is unavailable".into())
        })?;
        Err("Windows launch broker is unavailable".into())
    }

    fn perform_window_action(
        &mut self,
        permit: DesktopPermit,
        mut prepared: crate::platform::remote_observation::Prepared,
        id: &str,
        generation: u64,
        action: nickel_remote_control::window_actions::WindowAction,
    ) -> Result<nickel_remote_control::window_actions::WindowOutcome, String> {
        self.reconcile_prepared_resources(&permit, &mut prepared)?;
        let scope = permit.resource_scope()?;
        let session = prepared.session;
        let (window, evidence) = self
            .resources
            .window_resource(&scope, id, generation)
            .ok_or("Windows resource is unavailable")?;
        permit.with_resource(&evidence, || {
            crate::platform::remote_observation::request_window_action(window, session, action)
        })?;
        permit.check_live()?;

        // Confirmation is a new native inventory, never the Win32 request's
        // return value. This may truthfully report requested-but-unconfirmed.
        let mut observed = crate::platform::remote_observation::Prepared::prepare(&permit)?;
        self.reconcile_prepared_resources(&permit, &mut observed)?;
        let window = self.resources.windows(&scope).find_map(|(window, _)| {
            (window.id == id && window.generation == generation).then_some(window)
        });
        permit.check_live()?;
        Ok(nickel_remote_control::window_actions::WindowOutcome::observed(action, window))
    }

    /// This is the real production host path. It does not relax the separate
    /// approval gate: native UIA is registered, but native validation and
    /// resource/input protection are still incomplete.
    pub(crate) fn reconcile_indicators(
        &mut self,
        shell: &mut WinitShell,
        theme: nickel_ui::SemanticTheme,
    ) {
        let result = self.sync_indicators(shell, theme).and_then(|()| {
            if self.pending_indicator_activation.is_empty() {
                return Ok(());
            }
            let pending = std::mem::take(&mut self.pending_indicator_activation);
            let control = self.remote_control.control();
            let mut control = control
                .lock()
                .map_err(|_| "control owner unavailable".to_owned())?;
            for lease in pending {
                let resumable = control
                    .leases()
                    .iter()
                    .find(|candidate| candidate.id == lease)
                    .is_some_and(|candidate| {
                        candidate.suspended
                            && candidate
                                .expires_at
                                .is_none_or(|deadline| Instant::now() < deadline)
                    });
                if !resumable {
                    continue;
                }
                control
                    .leases_mut()
                    .resume_local(lease, Instant::now())
                    .map_err(|error| error.to_string())?;
            }
            drop(control);
            // Authority becomes usable only after every output exposed the
            // trusted paused grant. Publish its active state immediately; any
            // failure revokes the just-activated authority below.
            self.sync_indicators(shell, theme)
        });
        if let Err(error) = result {
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
                crate::windows_remote_input::release_all();
                self.keyboard_hold.take();
                self.pointer_hold.take();
                self.pending_indicator_activation.clear();
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
            Command::DecideRemoteLease {
                client_id,
                request,
                pending_generation,
                allow: true,
            } => {
                let request: nickel_remote_control::lease_requests::LeaseRequest = request.into();
                if !self.remote_lease_target_live(&request.scope) {
                    return error("lease target is unavailable or protected");
                }
                let lease = match self
                    .remote_control
                    .control()
                    .lock()
                    .unwrap()
                    .approve_lease_local(&client_id, &request, pending_generation, Instant::now())
                {
                    Ok(lease) => lease,
                    Err(reason) => return error(reason.to_string()),
                };
                self.remote_control
                    .control()
                    .lock()
                    .unwrap()
                    .leases_mut()
                    .suspend_local(lease)
                    .expect("new Windows lease exists");
                self.pending_indicator_activation.insert(lease);
            }
            Command::ApproveRemoteLeaseDuration {
                client_id,
                request,
                pending_generation,
                duration_seconds,
            } => {
                let request: nickel_remote_control::lease_requests::LeaseRequest = request.into();
                if !self.remote_lease_target_live(&request.scope) {
                    return error("lease target is unavailable or protected");
                }
                let lease = match self
                    .remote_control
                    .control()
                    .lock()
                    .unwrap()
                    .approve_lease_with_duration_local(
                        &client_id,
                        &request,
                        pending_generation,
                        duration_seconds.map(Duration::from_secs),
                        Instant::now(),
                    ) {
                    Ok(lease) => lease,
                    Err(reason) => return error(reason.to_string()),
                };
                self.remote_control
                    .control()
                    .lock()
                    .unwrap()
                    .leases_mut()
                    .suspend_local(lease)
                    .expect("new Windows lease exists");
                self.pending_indicator_activation.insert(lease);
            }
            Command::ManageRemoteLease { lease_id, action } => {
                use nickel_session_protocol::RemoteLeaseAction;
                match action {
                    RemoteLeaseAction::Revoke => {
                        self.pending_indicator_activation.remove(&lease_id);
                        self.remote_control
                            .control()
                            .lock()
                            .unwrap()
                            .leases_mut()
                            .revoke(lease_id);
                    }
                    RemoteLeaseAction::Pause => {
                        self.pending_indicator_activation.remove(&lease_id);
                        if let Err(reason) = self
                            .remote_control
                            .control()
                            .lock()
                            .unwrap()
                            .leases_mut()
                            .suspend_local(lease_id)
                        {
                            return error(reason.to_string());
                        }
                    }
                    RemoteLeaseAction::Resume => {
                        let lease = self
                            .remote_control
                            .control()
                            .lock()
                            .unwrap()
                            .leases()
                            .iter()
                            .find(|lease| lease.id == lease_id)
                            .map(|lease| {
                                (
                                    lease.scope.clone(),
                                    lease.suspended,
                                    lease
                                        .expires_at
                                        .is_none_or(|deadline| Instant::now() < deadline),
                                )
                            });
                        let Some((scope, suspended, unexpired)) = lease else {
                            return error("lease is unavailable");
                        };
                        if !unexpired {
                            return error("lease is expired");
                        }
                        if suspended {
                            if !self.remote_lease_target_live(&scope) {
                                return error("lease target is unavailable or protected");
                            }
                            self.pending_indicator_activation.insert(lease_id);
                        }
                    }
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

    #[test]
    fn windows_key_chords_are_bounded_and_reject_unmapped_keysyms() {
        assert_eq!(
            windows_key_chord(u32::from('a'), &[0xffe3, 0xffeb]).unwrap(),
            vec![0x11, 0x5b, b'A']
        );
        assert_eq!(windows_key_chord(0xff0d, &[]).unwrap(), vec![0x0d]);
        assert_eq!(windows_key_chord(0xffbe, &[]).unwrap(), vec![0x70]);
        assert!(windows_key_chord(u32::from('!'), &[]).is_err());
        assert!(windows_key_chord(u32::from('a'), &[0x61]).is_err());
    }
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
                started: Instant::now(),
                desktop_session: None,
                capture_generation: std::sync::atomic::AtomicU64::new(0),
            }),
            desktop_session: None,
            desktop_unlocked: false,
            local_input_epoch: local_input_epoch(),
            keyboard_hold: None,
            pointer_hold: None,
            desktop_events: Default::default(),
            pending_indicator_activation: Default::default(),
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
    fn protected_desktop_transition_revokes_runtime_authority_before_owner_work() {
        let mut owner = owner();
        let control = owner.remote_control.control();
        let client = {
            let mut control = control.lock().unwrap();
            control.set_enabled(true);
            let client = control.connect_identity("Windows lock transition").unwrap();
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
                    nickel_remote_control::lease_requests::LeaseRequest {
                        renewal: None,
                        scope: nickel_remote_control::leases::ResourceScope::FullSession,
                        duration: Some(Duration::from_secs(1200)),
                        allow_resumption: false,
                        full_debug: false,
                    },
                    now,
                )
                .unwrap();
            client
        };
        owner.desktop_unlocked = true;
        owner.reconcile_desktop_authority_observation(false);
        let control = control.lock().unwrap();
        assert!(control.lease_requests().pending().next().is_none());
        assert!(!control.has_ready_connection(&client.client_id, Instant::now()));
        assert!(control.leases().iter().next().is_none());
    }
    #[test]
    fn physical_input_epoch_cancels_shared_input_without_revoking_lease() {
        let mut owner = owner();
        let control = owner.remote_control.control();
        let lease = {
            let mut control = control.lock().unwrap();
            control.set_enabled(true);
            let client = control.connect_identity("Windows local collision").unwrap();
            let now = Instant::now();
            let watch = control
                .reserve_connection_watch(&client.client_id, &client.token, now)
                .unwrap();
            control
                .activate_connection_watch(&client.client_id, &client.token, watch, false, now)
                .unwrap();
            let request = nickel_remote_control::lease_requests::LeaseRequest {
                renewal: None,
                scope: nickel_remote_control::leases::ResourceScope::FullSession,
                duration: Some(Duration::from_secs(1200)),
                allow_resumption: false,
                full_debug: false,
            };
            control
                .request_lease(&client.client_id, &client.token, request.clone(), now)
                .unwrap();
            let generation = control
                .lease_requests()
                .pending_generation(&client.client_id)
                .unwrap();
            let lease = control
                .approve_lease_local(&client.client_id, &request, generation, now)
                .unwrap();
            control.leases_mut().reserve_input(lease, 7).unwrap();
            lease
        };
        let next = owner.local_input_epoch.checked_add(1).unwrap();
        owner.reconcile_local_input_observation(next);
        let mut control = control.lock().unwrap();
        assert!(control.leases().iter().any(|active| active.id == lease));
        assert!(control.leases_mut().reserve_input(lease, 8).is_ok());
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
        owner.poll_with_shell(None);
        assert_eq!(owner.remote_control.status().generation, 0);
        assert!(receiver.try_recv().is_err());
    }
}
