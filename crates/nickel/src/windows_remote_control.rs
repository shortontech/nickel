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
struct LaunchPreparationState {
    started: Instant,
    state: std::sync::Mutex<LaunchPreparationDiagnosticState>,
}

#[derive(Default)]
struct LaunchPreparationDiagnosticState {
    busy: bool,
    generation: u64,
    changed_us: u64,
}

fn launch_preparation_state() -> &'static LaunchPreparationState {
    static STATE: std::sync::OnceLock<LaunchPreparationState> = std::sync::OnceLock::new();
    STATE.get_or_init(|| LaunchPreparationState {
        started: Instant::now(),
        state: std::sync::Mutex::new(LaunchPreparationDiagnosticState::default()),
    })
}

impl LaunchPreparationState {
    fn uptime_us(&self) -> u64 {
        self.started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
    }

    fn snapshot(&self) -> Option<nickel_remote_control::diagnostics::BackgroundWorkerDiagnostic> {
        let state = self.state.try_lock().ok()?;
        Some(
            nickel_remote_control::diagnostics::BackgroundWorkerDiagnostic {
                generation: state.generation,
                collector_uptime_us: self.uptime_us(),
                last_changed_uptime_us: state.changed_us,
                busy: state.busy,
            },
        )
    }

    fn begin(&self) -> Result<(), String> {
        let mut diagnostic = self
            .state
            .try_lock()
            .map_err(|_| "Windows application launch preparation is busy".to_owned())?;
        if diagnostic.busy {
            return Err("Windows application launch preparation is busy".into());
        }
        diagnostic.busy = true;
        diagnostic.generation = diagnostic.generation.saturating_add(1);
        diagnostic.changed_us = self.uptime_us();
        Ok(())
    }

    fn end(&self) {
        if let Ok(mut diagnostic) = self.state.lock() {
            diagnostic.busy = false;
            diagnostic.generation = diagnostic.generation.saturating_add(1);
            diagnostic.changed_us = self.uptime_us();
        }
    }
}
const MAX_PENDING_OUTPUT_LAUNCHES: usize = 8;
const OUTPUT_LAUNCH_PLACEMENT_TTL: Duration = Duration::from_secs(30);
const OUTPUT_LAUNCH_ROOT_TTL: Duration = Duration::from_secs(2);

struct LaunchPreparationAdmission;

impl LaunchPreparationAdmission {
    fn acquire() -> Result<Self, String> {
        let state = launch_preparation_state();
        state.begin()?;
        Ok(Self)
    }
}

impl Drop for LaunchPreparationAdmission {
    fn drop(&mut self) {
        launch_preparation_state().end();
    }
}

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

pub(crate) fn local_input_epoch() -> u64 {
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
            .filter_map(|record| {
                DiagnosticLogRecord::from_static_metadata(
                    record.generation,
                    record.observed_at_us,
                    record.level,
                    record.target,
                    record.file,
                    record.line,
                )
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
        // The owner drains invalidated native holds immediately after lifecycle
        // reconciliation and before dispatching another queued request.
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
struct PlannedApplicationLaunch {
    plan: crate::windows_application_registry::native::LaunchPlan,
    output: Option<(
        nickel_remote_control::leases::ResourceId,
        crate::windows_resource_owner::Output,
    )>,
    commit_evidence: Option<Box<crate::platform::remote_observation::Prepared>>,
}
struct AuthorizedApplicationLaunch {
    committed: crate::windows_launch_broker::CommittedLaunch,
    placement: Option<OutputLaunchPlacementTicket>,
}
#[derive(Clone)]
struct OutputLaunchPlacementTicket {
    id: u64,
    output: nickel_remote_control::leases::ResourceId,
    deadline: Instant,
}
struct PendingOutputLaunchPlacement {
    permit: DesktopPermit,
    output: nickel_remote_control::leases::ResourceId,
    native_output: crate::windows_resource_owner::Output,
    baseline: std::collections::BTreeSet<WindowIncarnation>,
    root: Option<Arc<nickel_platform::process_identity::WindowsProcessIdentity>>,
    root_deadline: Instant,
    deadline: Instant,
}
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WindowIncarnation {
    native: usize,
    pid: u32,
    created: u64,
    thread: u32,
}
impl From<&crate::windows_resource_owner::Window> for WindowIncarnation {
    fn from(window: &crate::windows_resource_owner::Window) -> Self {
        Self {
            native: window.native,
            pid: window.pid,
            created: window.created,
            thread: window.thread,
        }
    }
}

fn retain_launch_safe_windows(
    prepared: &mut crate::platform::remote_observation::Prepared,
    pending: &std::collections::BTreeMap<u64, PendingOutputLaunchPlacement>,
) {
    if pending.is_empty() {
        return;
    }
    let rooted = pending
        .iter()
        .filter_map(|(id, placement)| {
            placement.root.as_ref().map(|root| {
                (
                    *id,
                    crate::platform::remote_observation::LaunchProcessRoot {
                        pid: root.process_id(),
                        created: root.created_at(),
                    },
                )
            })
        })
        .collect::<Vec<_>>();
    let roots = rooted.iter().map(|(_, root)| *root).collect::<Vec<_>>();
    // Classification failure is ambiguity, never permission to publish a new
    // window. Baseline windows remain unaffected by native ancestry failures.
    let ancestry = if roots.is_empty() {
        Default::default()
    } else {
        prepared.classify_launch_window_ancestry(&roots)
    };
    let root_indices = rooted
        .iter()
        .enumerate()
        .map(|(index, (id, _))| (*id, index))
        .collect::<std::collections::BTreeMap<_, _>>();
    let placements = pending
        .iter()
        .map(|(id, placement)| {
            (
                &placement.baseline,
                placement
                    .root
                    .as_ref()
                    .and_then(|_| root_indices.get(id).copied()),
            )
        })
        .collect::<Vec<_>>();
    retain_windows_for_launch_placements(&mut prepared.windows, &placements, &ancestry);
    let retained = prepared
        .windows
        .iter()
        .map(|window| window.native)
        .collect::<std::collections::BTreeSet<_>>();
    prepared
        .processes
        .retain(|native, _| retained.contains(native));
}

fn retain_windows_for_launch_placements(
    windows: &mut Vec<crate::windows_resource_owner::Window>,
    placements: &[(
        &std::collections::BTreeSet<WindowIncarnation>,
        Option<usize>,
    )],
    ancestry: &std::collections::BTreeMap<
        usize,
        Vec<crate::platform::remote_observation::LaunchProcessAncestry>,
    >,
) {
    windows.retain(|window| {
        let incarnation = WindowIncarnation::from(window);
        placements.iter().all(|(baseline, root_index)| {
            baseline.contains(&incarnation)
                || root_index.is_some_and(|root_index| {
                    ancestry
                        .get(&window.native)
                        .and_then(|relations| relations.get(root_index))
                        .is_some_and(|relation| {
                            *relation
                            == crate::platform::remote_observation::LaunchProcessAncestry::Unrelated
                        })
                })
        })
    });
}

fn start_output_placement_worker(
    sender: SyncSender<OwnerRequest>,
    permit: DesktopPermit,
    ticket: OutputLaunchPlacementTicket,
    root: Arc<nickel_platform::process_identity::WindowsProcessIdentity>,
) -> bool {
    std::thread::Builder::new()
        .name("nickel-windows-launch-placement".to_owned())
        .spawn(move || {
            let (reply, receiver) = mpsc::sync_channel(1);
            if sender
                .try_send(OwnerRequest::BindApplicationPlacementRoot {
                    permit: permit.clone(),
                    ticket: ticket.clone(),
                    root,
                    reply,
                })
                .is_err()
                || !matches!(
                    receiver.recv_timeout(Duration::from_millis(250)),
                    Ok(Ok(()))
                )
            {
                let _ = sender.try_send(OwnerRequest::CancelApplicationPlacement { ticket });
                return;
            }
            loop {
                if Instant::now() >= ticket.deadline {
                    let _ = sender.try_send(OwnerRequest::CancelApplicationPlacement { ticket });
                    return;
                }
                let current = match permit.continued_observation() {
                    Ok(current) => current,
                    Err(_) => {
                        let _ =
                            sender.try_send(OwnerRequest::CancelApplicationPlacement { ticket });
                        return;
                    }
                };
                let prepared =
                    match crate::platform::remote_observation::Prepared::prepare(&current) {
                        Ok(prepared) => prepared,
                        Err(_) => {
                            std::thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                    };
                let (reply, receiver) = mpsc::sync_channel(1);
                if sender
                    .try_send(OwnerRequest::AttemptApplicationPlacement {
                        permit: current,
                        ticket: ticket.clone(),
                        prepared: Box::new(prepared),
                        reply,
                    })
                    .is_err()
                {
                    // Queue overload cannot authorize publication. Leave the
                    // quarantine for owner-side expiry/revocation cleanup.
                    return;
                }
                match receiver.recv_timeout(Duration::from_millis(250)) {
                    Ok(Ok(true)) => return,
                    Ok(Ok(false)) => std::thread::sleep(Duration::from_millis(10)),
                    Ok(Err(_)) | Err(_) => return,
                }
            }
        })
        .is_ok()
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
    PrepareNativeAccessibility {
        permit: DesktopPermit,
        prepared: Box<crate::platform::remote_observation::Prepared>,
        id: String,
        generation: u64,
        application_wide: bool,
        reply: SyncSender<Result<crate::windows_external_accessibility::Proof, String>>,
    },
    FinishNativeAccessibility {
        permit: DesktopPermit,
        prepared: Box<crate::platform::remote_observation::Prepared>,
        proof: Box<crate::windows_external_accessibility::Proof>,
        observation: crate::windows_external_accessibility::Observation,
        reply: SyncSender<
            Result<nickel_remote_control::native_semantics::NativeSemanticSnapshot, String>,
        >,
    },
    PrepareNativeAccessibilityAction {
        permit: DesktopPermit,
        prepared: Box<crate::platform::remote_observation::Prepared>,
        request: nickel_remote_control::native_semantics::NativeSemanticActionRequest,
        reply: SyncSender<Result<crate::windows_external_accessibility::ActionPlan, String>>,
    },
    CommitNativeAccessibilityAction {
        permit: DesktopPermit,
        prepared: Box<crate::platform::remote_observation::Prepared>,
        plan: crate::windows_external_accessibility::ActionPlan,
        deadline: Instant,
        expected_local_input_epoch: u64,
        dispatch: Arc<std::sync::atomic::AtomicU8>,
        reply:
            SyncSender<Result<crate::windows_external_accessibility::ActionAuthorization, String>>,
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
    ShellSurfaces {
        permit: DesktopPermit,
        reply: SyncSender<
            Result<Vec<nickel_remote_control::diagnostics::ShellSurfaceDiagnostic>, String>,
        >,
    },
    InspectShellSurface {
        permit: DesktopPermit,
        id: String,
        generation: u64,
        reply:
            SyncSender<Result<nickel_remote_control::semantics::SurfaceSemanticSnapshot, String>>,
    },
    ShellSemanticAction {
        permit: DesktopPermit,
        request: nickel_remote_control::semantics::SurfaceSemanticActionRequest,
        deadline: Instant,
        expected_local_input_epoch: u64,
        reply: SyncSender<
            Result<nickel_remote_control::semantics::SurfaceSemanticActionOutcome, String>,
        >,
    },
    WorkspaceAction {
        permit: DesktopPermit,
        prepared: Box<crate::platform::remote_observation::Prepared>,
        action: nickel_remote_control::diagnostics::WorkspaceAction,
        reply: SyncSender<Result<nickel_remote_control::diagnostics::WorkspaceOutcome, String>>,
    },
    Diagnostic {
        permit: DesktopPermit,
        prepared: Box<crate::platform::remote_observation::Prepared>,
        reply: SyncSender<Result<nickel_remote_control::diagnostics::DiagnosticSnapshot, String>>,
    },
    DiagnosticAction {
        permit: DesktopPermit,
        action: nickel_remote_control::diagnostics::DiagnosticAction,
        platform_refresh: Option<PreparedWindowsPlatformRefresh>,
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
    ReadApplicationScale {
        permit: DesktopPermit,
        prepared: crate::windows_remote_application_scale::PreparedRead,
        reply: SyncSender<Result<nickel_remote_control::application_scale::Snapshot, String>>,
    },
    ApplicationScaleTransaction {
        permit: DesktopPermit,
        transaction: nickel_remote_control::application_scale::Transaction,
        prepared: Box<crate::windows_remote_application_scale::PreparedChange>,
        deadline: Instant,
        reply: SyncSender<
            Result<nickel_remote_control::application_scale::TransactionOutcome, String>,
        >,
    },
    ReadAppearance {
        permit: DesktopPermit,
        prepared: crate::windows_remote_settings::PreparedAppearanceRead,
        reply: SyncSender<Result<nickel_remote_control::appearance::Snapshot, String>>,
    },
    AppearanceTransaction {
        permit: DesktopPermit,
        transaction: nickel_remote_control::appearance::Transaction,
        prepared: crate::windows_remote_settings::PreparedAppearanceChange,
        deadline: Instant,
        reply: SyncSender<Result<nickel_remote_control::appearance::Snapshot, String>>,
    },
    ReadFileIcons {
        permit: DesktopPermit,
        prepared: crate::windows_remote_settings::PreparedFileIconsRead,
        reply: SyncSender<Result<nickel_remote_control::file_icons::Snapshot, String>>,
    },
    FileIconsTransaction {
        permit: DesktopPermit,
        transaction: nickel_remote_control::file_icons::Transaction,
        prepared: crate::windows_remote_settings::PreparedFileIconsChange,
        deadline: Instant,
        reply: SyncSender<Result<nickel_remote_control::file_icons::Snapshot, String>>,
    },
    ReadWallpaper {
        permit: DesktopPermit,
        prepared: crate::windows_remote_settings::PreparedWallpaperRead,
        reply: SyncSender<Result<nickel_remote_control::wallpaper::Snapshot, String>>,
    },
    WallpaperTransaction {
        permit: DesktopPermit,
        transaction: nickel_remote_control::wallpaper::Transaction,
        prepared: crate::windows_remote_settings::PreparedWallpaperChange,
        deadline: Instant,
        expected_local_input_epoch: u64,
        reply: SyncSender<Result<nickel_remote_control::wallpaper::Snapshot, String>>,
    },
    ReadTerminalPresentation {
        permit: DesktopPermit,
        prepared: crate::windows_remote_terminal_presentation::PreparedRead,
        reply: SyncSender<Result<nickel_remote_control::terminal_presentation::Snapshot, String>>,
    },
    TerminalPresentationTransaction {
        permit: DesktopPermit,
        transaction: nickel_remote_control::terminal_presentation::Transaction,
        prepared: Box<crate::windows_remote_terminal_presentation::PreparedChange>,
        deadline: Instant,
        expected_local_input_epoch: u64,
        reply: SyncSender<Result<nickel_remote_control::terminal_presentation::Snapshot, String>>,
    },
    ReadTerminalLaunchPolicy {
        permit: DesktopPermit,
        prepared: crate::remote_terminal_launch_policy::PreparedRead,
        reply: SyncSender<Result<nickel_remote_control::terminal_launch_policy::Snapshot, String>>,
    },
    TerminalLaunchPolicyTransaction {
        permit: DesktopPermit,
        transaction: nickel_remote_control::terminal_launch_policy::Transaction,
        prepared: Box<crate::remote_terminal_launch_policy::PreparedChange>,
        deadline: Instant,
        expected_local_input_epoch: u64,
        reply: SyncSender<
            Result<nickel_remote_control::terminal_launch_policy::TransactionOutcome, String>,
        >,
    },
    ReadCodexPreference {
        permit: DesktopPermit,
        prepared: crate::windows_remote_codex::PreparedRead,
        deadline: Instant,
        reply: SyncSender<Result<nickel_remote_control::codex_preference::Snapshot, String>>,
    },
    CodexPreferenceTransaction {
        permit: DesktopPermit,
        transaction: nickel_remote_control::codex_preference::Transaction,
        prepared: crate::windows_remote_codex::PreparedChange,
        deadline: Instant,
        expected_local_input_epoch: u64,
        reply: SyncSender<Result<nickel_remote_control::codex_preference::Snapshot, String>>,
    },
    ShellBehaviorTransaction {
        permit: DesktopPermit,
        transaction: nickel_session_protocol::ShellBehaviorTransaction,
        prepared: crate::windows_remote_settings::PreparedShellBehaviorChange,
        deadline: Instant,
        expected_local_input_epoch: u64,
        reply:
            SyncSender<Result<nickel_remote_control::diagnostics::ShellBehaviorDiagnostic, String>>,
    },
    ReadIdlePreferences {
        permit: DesktopPermit,
        prepared: crate::windows_remote_settings::PreparedIdleRead,
        reply: SyncSender<Result<nickel_remote_control::idle_preferences::Snapshot, String>>,
    },
    IdlePreferencesTransaction {
        permit: DesktopPermit,
        transaction: nickel_remote_control::idle_preferences::Transaction,
        prepared: crate::windows_remote_settings::PreparedIdleChange,
        deadline: Instant,
        expected_local_input_epoch: u64,
        reply: SyncSender<Result<nickel_remote_control::idle_preferences::Snapshot, String>>,
    },
    ReadKeyboardPreference {
        permit: DesktopPermit,
        prepared: crate::windows_remote_settings::PreparedKeyboardRead,
        reply: SyncSender<Result<nickel_remote_control::keyboard_preference::Snapshot, String>>,
    },
    KeyboardPreferenceTransaction {
        permit: DesktopPermit,
        transaction: nickel_remote_control::keyboard_preference::Transaction,
        prepared: crate::windows_remote_settings::PreparedKeyboardChange,
        deadline: Instant,
        expected_local_input_epoch: u64,
        reply: SyncSender<Result<nickel_remote_control::keyboard_preference::Snapshot, String>>,
    },
    LauncherFavoritesCatalog {
        permit: DesktopPermit,
        deadline: Instant,
        reply: SyncSender<Result<crate::windows_remote_launcher_favorites::Catalog, String>>,
    },
    ReadLauncherFavorites {
        permit: DesktopPermit,
        prepared: crate::windows_remote_launcher_favorites::PreparedRead,
        deadline: Instant,
        reply: SyncSender<Result<nickel_remote_control::launcher_favorites::Snapshot, String>>,
    },
    LauncherFavoritesTransaction {
        permit: DesktopPermit,
        transaction: nickel_remote_control::launcher_favorites::Transaction,
        prepared: crate::windows_remote_launcher_favorites::PreparedChange,
        deadline: Instant,
        expected_local_input_epoch: u64,
        reply: SyncSender<Result<nickel_remote_control::launcher_favorites::Snapshot, String>>,
    },
    Applications {
        permit: DesktopPermit,
        prepared: Option<Box<crate::platform::remote_observation::Prepared>>,
        reply: SyncSender<Result<nickel_remote_control::diagnostics::ApplicationInventory, String>>,
    },
    LaunchApplication {
        permit: DesktopPermit,
        request: nickel_remote_control::diagnostics::LaunchApplicationRequest,
        prepared: Option<Box<crate::platform::remote_observation::Prepared>>,
        deadline: Instant,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
        reply: SyncSender<Result<PlannedApplicationLaunch, String>>,
    },
    CommitApplicationLaunch {
        permit: DesktopPermit,
        request: nickel_remote_control::diagnostics::LaunchApplicationRequest,
        plan: Box<PlannedApplicationLaunch>,
        staged: Box<crate::windows_launch_broker::StagedLaunch>,
        deadline: Instant,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
        reply: SyncSender<Result<AuthorizedApplicationLaunch, String>>,
    },
    BindApplicationPlacementRoot {
        permit: DesktopPermit,
        ticket: OutputLaunchPlacementTicket,
        root: Arc<nickel_platform::process_identity::WindowsProcessIdentity>,
        reply: SyncSender<Result<(), String>>,
    },
    AttemptApplicationPlacement {
        permit: DesktopPermit,
        ticket: OutputLaunchPlacementTicket,
        prepared: Box<crate::platform::remote_observation::Prepared>,
        reply: SyncSender<Result<bool, String>>,
    },
    CancelApplicationPlacement {
        ticket: OutputLaunchPlacementTicket,
    },
    Connection {
        permit: nickel_remote_control::ClientConnectionPermit,
        action: nickel_remote_control::ClientConnectionAction,
        reply: SyncSender<Result<(), String>>,
    },
}

struct PreparedWindowsPlatformRefresh {
    domain: nickel_remote_control::diagnostics::PlatformRefreshDomain,
    data: PreparedWindowsPlatformRefreshData,
    observation_started: Instant,
    observed: Instant,
    preparation_duration_us: u64,
}

enum PreparedWindowsPlatformRefreshData {
    Connectivity(crate::platform::ConnectivityRefresh),
    Audio(crate::platform::AudioRefresh),
    Peripherals(crate::platform::PeripheralRefresh),
    Maintenance(crate::platform::MaintenanceRefresh),
    DefaultAssociations(crate::platform::DefaultAssociationsRefresh),
}

#[derive(Default)]
struct WindowsPlatformRefreshState {
    busy: bool,
    generation: u64,
    changed_us: u64,
}

struct WindowsPlatformRefreshWorker {
    started: Instant,
    state: std::sync::Mutex<WindowsPlatformRefreshState>,
}

impl Default for WindowsPlatformRefreshWorker {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            state: Default::default(),
        }
    }
}

struct WindowsPlatformRefreshAdmission(Arc<WindowsPlatformRefreshWorker>);

impl WindowsPlatformRefreshWorker {
    fn uptime_us(&self) -> u64 {
        self.started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
    }

    fn acquire(self: &Arc<Self>) -> Result<WindowsPlatformRefreshAdmission, String> {
        let mut state = self
            .state
            .try_lock()
            .map_err(|_| "Windows platform refresh worker is busy or unavailable".to_owned())?;
        if state.busy {
            return Err("Windows platform refresh worker is already in progress".to_owned());
        }
        state.busy = true;
        state.generation = state.generation.saturating_add(1);
        state.changed_us = self.uptime_us();
        drop(state);
        Ok(WindowsPlatformRefreshAdmission(self.clone()))
    }

    fn snapshot(&self) -> Option<nickel_remote_control::diagnostics::BackgroundWorkerDiagnostic> {
        let state = self.state.try_lock().ok()?;
        Some(
            nickel_remote_control::diagnostics::BackgroundWorkerDiagnostic {
                generation: state.generation,
                collector_uptime_us: self.uptime_us(),
                last_changed_uptime_us: state.changed_us,
                busy: state.busy,
            },
        )
    }
}

impl Drop for WindowsPlatformRefreshAdmission {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.busy = false;
            state.generation = state.generation.saturating_add(1);
            state.changed_us = self.0.uptime_us();
        }
    }
}

fn prepare_windows_platform_refresh(
    permit: &DesktopPermit,
    worker: Arc<WindowsPlatformRefreshWorker>,
    domain: nickel_remote_control::diagnostics::PlatformRefreshDomain,
) -> Result<PreparedWindowsPlatformRefresh, String> {
    use nickel_remote_control::diagnostics::PlatformRefreshDomain;
    const DEADLINE: Duration = Duration::from_secs(2);

    permit.with_debug(false, || Ok(()))?;
    let observation_started = Instant::now();
    let maintenance_deadline =
        observation_started + DEADLINE.saturating_sub(Duration::from_millis(100));
    let cancellation_permit = permit.clone();
    let data = run_windows_platform_refresh_worker(worker, DEADLINE, move || match domain {
        PlatformRefreshDomain::Connectivity => crate::platform::refresh_connectivity_status()
            .map(PreparedWindowsPlatformRefreshData::Connectivity),
        PlatformRefreshDomain::Audio => {
            crate::platform::refresh_audio_status().map(PreparedWindowsPlatformRefreshData::Audio)
        }
        PlatformRefreshDomain::Peripherals => crate::platform::refresh_peripheral_status()
            .map(PreparedWindowsPlatformRefreshData::Peripherals),
        PlatformRefreshDomain::DefaultAssociations => {
            crate::platform::refresh_default_associations()
                .map(PreparedWindowsPlatformRefreshData::DefaultAssociations)
        }
        PlatformRefreshDomain::Maintenance => crate::platform::refresh_windows_maintenance_status(
            maintenance_deadline,
            Arc::new(move || cancellation_permit.check_live().is_err()),
        )
        .map(PreparedWindowsPlatformRefreshData::Maintenance),
    })?;
    let observed = Instant::now();
    permit.with_debug(false, || Ok(()))?;
    Ok(PreparedWindowsPlatformRefresh {
        domain,
        data,
        observation_started,
        observed,
        preparation_duration_us: observed
            .saturating_duration_since(observation_started)
            .as_micros()
            .min(u128::from(u64::MAX)) as u64,
    })
}

fn run_windows_platform_refresh_worker<T: Send + 'static>(
    worker: Arc<WindowsPlatformRefreshWorker>,
    deadline: Duration,
    query: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let admission = worker.acquire()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("nickel-windows-platform-refresh".into())
        .spawn(move || {
            let _admission = admission;
            let _ = sender.try_send(query());
        })
        .map_err(|_| "Windows platform refresh worker is unavailable".to_owned())?;
    receiver
        .recv_timeout(deadline)
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => {
                "Windows platform refresh exceeded its deadline".to_owned()
            }
            mpsc::RecvTimeoutError::Disconnected => {
                "Windows platform refresh worker stopped".to_owned()
            }
        })?
}
struct WindowsDesktopAuthority {
    cleanup_wake: nickel_remote_control::ConnectionCleanupWake,
    sender: SyncSender<OwnerRequest>,
    started: Instant,
    desktop_session: Option<u32>,
    capture_generation: std::sync::atomic::AtomicU64,
    peripheral_generation: std::sync::atomic::AtomicU64,
    platform_refresh_worker: Arc<WindowsPlatformRefreshWorker>,
}
impl WindowsDesktopAuthority {
    fn prepare_accessibility_inventory(
        permit: &DesktopPermit,
        deadline: Instant,
    ) -> Result<Box<crate::platform::remote_observation::Prepared>, String> {
        let worker_permit = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("nickel-windows-uia-inventory".into())
            .spawn(move || {
                let result = crate::platform::remote_observation::Admission::acquire().and_then(
                    |_admission| {
                        let prepared =
                            crate::platform::remote_observation::Prepared::prepare(&worker_permit)?;
                        worker_permit.check_live()?;
                        if Instant::now() >= deadline {
                            return Err("Windows UI Automation observation timed out".into());
                        }
                        Ok(Box::new(prepared))
                    },
                );
                let _ = reply.try_send(result);
            })
            .map_err(|_| "Windows UI Automation inventory worker is unavailable".to_owned())?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows UI Automation observation timed out")?;
        receiver
            .recv_timeout(remaining)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => {
                    "Windows UI Automation observation timed out".to_owned()
                }
                mpsc::RecvTimeoutError::Disconnected => {
                    "Windows UI Automation inventory worker stopped".to_owned()
                }
            })?
    }

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

    fn inspect_native_accessibility(
        &self,
        permit: DesktopPermit,
        id: &str,
        generation: u64,
        application_wide: bool,
    ) -> Result<nickel_remote_control::native_semantics::NativeSemanticSnapshot, String> {
        let admission = crate::windows_external_accessibility::Admission::acquire()?;
        let deadline = crate::windows_external_accessibility::deadline();
        let prepared = Self::prepare_accessibility_inventory(&permit, deadline)?;
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::PrepareNativeAccessibility {
                permit: permit.clone(),
                prepared,
                id: id.to_owned(),
                generation,
                application_wide,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows UI Automation observation timed out")?
            .min(Duration::from_millis(200));
        let proof = receiver
            .recv_timeout(remaining)
            .map_err(|_| "Windows UI Automation owner timed out".to_owned())??;
        permit.check_live()?;
        let (worker_reply, worker_receiver) = mpsc::sync_channel(1);
        let worker_permit = permit.clone();
        let worker_proof = proof.clone();
        std::thread::Builder::new()
            .name("nickel-windows-uia-observe".into())
            .spawn(move || {
                let _admission = admission;
                let result = crate::windows_external_accessibility::observe(
                    &worker_proof,
                    &worker_permit,
                    deadline,
                )
                .and_then(|observation| {
                    let _inventory_admission =
                        crate::platform::remote_observation::Admission::acquire()?;
                    let prepared = Box::new(
                        crate::platform::remote_observation::Prepared::prepare(&worker_permit)?,
                    );
                    worker_permit.check_live()?;
                    if Instant::now() >= deadline {
                        return Err("Windows UI Automation observation timed out".into());
                    }
                    Ok((observation, prepared))
                });
                let _ = worker_reply.try_send(result);
            })
            .map_err(|_| "Windows UI Automation worker is unavailable".to_owned())?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows UI Automation observation timed out")?;
        let (observation, prepared) =
            worker_receiver
                .recv_timeout(remaining)
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => {
                        "Windows UI Automation observation timed out".to_owned()
                    }
                    mpsc::RecvTimeoutError::Disconnected => {
                        "Windows UI Automation worker stopped".to_owned()
                    }
                })??;
        permit.check_live()?;
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::FinishNativeAccessibility {
                permit,
                prepared,
                proof: Box::new(proof),
                observation,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows UI Automation observation timed out")?;
        let result = receiver
            .recv_timeout(remaining)
            .map_err(|_| "Windows UI Automation owner timed out".to_owned())?;
        completion.check_live()?;
        result
    }

    fn perform_native_accessibility_action(
        &self,
        permit: DesktopPermit,
        request: nickel_remote_control::native_semantics::NativeSemanticActionRequest,
    ) -> Result<nickel_remote_control::native_semantics::NativeSemanticActionOutcome, String> {
        request.validate().map_err(str::to_owned)?;
        let admission = crate::windows_external_accessibility::Admission::acquire()?;
        let deadline = crate::windows_external_accessibility::deadline();
        let expected_local_input_epoch = local_input_epoch();
        let prepared = Self::prepare_accessibility_inventory(&permit, deadline)?;
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::PrepareNativeAccessibilityAction {
                permit: permit.clone(),
                prepared,
                request,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows UI Automation action timed out")?
            .min(Duration::from_millis(200));
        let plan = receiver
            .recv_timeout(remaining)
            .map_err(|_| "Windows UI Automation owner timed out".to_owned())??;
        permit.check_live()?;

        let (worker_reply, worker_receiver) = mpsc::sync_channel(1);
        let worker_permit = permit.clone();
        let sender = self.sender.clone();
        let worker_plan = plan.clone();
        let dispatch = Arc::new(std::sync::atomic::AtomicU8::new(
            crate::windows_external_accessibility::ACTION_PENDING,
        ));
        let worker_dispatch = dispatch.clone();
        std::thread::Builder::new()
            .name("nickel-windows-uia-action".into())
            .spawn(move || {
                let _admission = admission;
                let result = crate::windows_external_accessibility::execute_action(
                    &worker_plan,
                    &worker_permit,
                    deadline,
                    || {
                        let prepared =
                            Self::prepare_accessibility_inventory(&worker_permit, deadline)?;
                        let (reply, receiver) = mpsc::sync_channel(1);
                        sender
                            .try_send(OwnerRequest::CommitNativeAccessibilityAction {
                                permit: worker_permit.clone(),
                                prepared,
                                plan: worker_plan.clone(),
                                deadline,
                                expected_local_input_epoch,
                                dispatch: worker_dispatch.clone(),
                                reply,
                            })
                            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
                        let remaining = deadline
                            .checked_duration_since(Instant::now())
                            .ok_or("Windows UI Automation action timed out")?
                            .min(Duration::from_millis(200));
                        receiver
                            .recv_timeout(remaining)
                            .map_err(|_| "Windows UI Automation owner timed out".to_owned())?
                    },
                    worker_dispatch.as_ref(),
                );
                let _ = worker_reply.try_send(result);
            })
            .map_err(|_| "Windows UI Automation worker is unavailable".to_owned())?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows UI Automation action timed out")?;
        let uncertain_or_cancel = |message: &str| {
            use crate::windows_external_accessibility::ACTION_ATTEMPTED;
            if crate::windows_external_accessibility::cancel_action_dispatch(&dispatch) {
                Err(message.to_owned())
            } else if dispatch.load(std::sync::atomic::Ordering::Acquire) == ACTION_ATTEMPTED {
                Ok(
                    nickel_remote_control::native_semantics::NativeSemanticActionOutcome {
                        requested: true,
                        confirmed: false,
                        uncertain: true,
                    },
                )
            } else {
                Err(message.to_owned())
            }
        };
        match worker_receiver.recv_timeout(remaining) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                uncertain_or_cancel("Windows UI Automation worker stopped before dispatch")
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                uncertain_or_cancel("Windows UI Automation action timed out before dispatch")
            }
        }
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
    fn workspace_action(
        &self,
        permit: DesktopPermit,
        action: nickel_remote_control::diagnostics::WorkspaceAction,
    ) -> Result<nickel_remote_control::diagnostics::WorkspaceOutcome, String> {
        let _admission = crate::platform::remote_observation::Admission::acquire()?;
        let prepared = Box::new(crate::platform::remote_observation::Prepared::prepare(
            &permit,
        )?);
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::WorkspaceAction {
                permit,
                prepared,
                action,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows workspace observation timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn read_peripheral_controls(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::peripheral_controls::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        let generation = self
            .peripheral_generation
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |generation| generation.checked_add(1),
            )
            .map_err(|_| "Windows peripheral observation generation exhausted")?
            .checked_add(1)
            .ok_or("Windows peripheral observation generation exhausted")?;
        permit.with_debug(false, || Ok(()))?;
        Ok(nickel_remote_control::peripheral_controls::Snapshot {
            generation,
            observed_at_micros: self.started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
            printers: nickel_remote_control::peripheral_controls::Availability::Unavailable,
            removable_volumes:
                nickel_remote_control::peripheral_controls::Availability::Unavailable,
            printer_controls: nickel_remote_control::peripheral_controls::Availability::Unavailable,
            printer_entries: Vec::new(),
            removable_volume_entries: Vec::new(),
            omitted_printers: 0,
            omitted_print_jobs: 0,
            omitted_removable_volumes: 0,
        })
    }
    fn control_peripherals(
        &self,
        permit: DesktopPermit,
        transaction: nickel_remote_control::peripheral_controls::Transaction,
    ) -> Result<nickel_remote_control::peripheral_controls::Outcome, String> {
        if !transaction.valid() {
            return Err("invalid peripheral transaction".into());
        }
        permit.with_debug_input(false, || Ok(()))?;
        Ok(nickel_remote_control::peripheral_controls::Outcome {
            completion: nickel_remote_control::semantics::SurfaceSemanticCompletion::Unavailable,
        })
    }
    fn read_application_scale(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::application_scale::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::windows_remote_application_scale::PreparedRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::ReadApplicationScale {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows application scale observation timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn application_scale_transaction(
        &self,
        permit: DesktopPermit,
        transaction: nickel_remote_control::application_scale::Transaction,
    ) -> Result<nickel_remote_control::application_scale::TransactionOutcome, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        permit.with_debug(false, || Ok(()))?;
        let prepared =
            crate::windows_remote_application_scale::PreparedChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        if Instant::now() >= deadline {
            return Err("Windows application scale preparation timed out".into());
        }
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::ApplicationScaleTransaction {
                permit,
                transaction,
                prepared: Box::new(prepared),
                deadline,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows application scale transaction expired before dispatch")?;
        receiver.recv_timeout(remaining).map_err(|_| {
            "Windows application scale result uncertain; read current state before retrying"
                .to_owned()
        })?
    }
    fn read_appearance(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::appearance::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::windows_remote_settings::PreparedAppearanceRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::ReadAppearance {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows appearance observation timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn appearance_transaction(
        &self,
        permit: DesktopPermit,
        transaction: nickel_remote_control::appearance::Transaction,
    ) -> Result<nickel_remote_control::appearance::Snapshot, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        permit.with_debug(false, || Ok(()))?;
        let prepared =
            crate::windows_remote_settings::PreparedAppearanceChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        if Instant::now() >= deadline {
            return Err("Windows appearance preparation timed out".into());
        }
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::AppearanceTransaction {
                permit,
                transaction,
                prepared,
                deadline,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows appearance transaction expired before dispatch")?;
        receiver.recv_timeout(remaining).map_err(|_| {
            "Windows appearance result uncertain; read current appearance before retrying"
                .to_owned()
        })?
    }
    fn read_default_association(
        &self,
        permit: DesktopPermit,
        target: nickel_remote_control::default_associations::Target,
    ) -> Result<nickel_remote_control::default_associations::Snapshot, String> {
        crate::remote_default_associations::read(permit, target)
    }
    fn default_association_transaction(
        &self,
        permit: DesktopPermit,
        transaction: nickel_remote_control::default_associations::Transaction,
    ) -> Result<nickel_remote_control::default_associations::TransactionOutcome, String> {
        crate::remote_default_associations::transact(permit, transaction)
    }
    fn read_file_icons(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::file_icons::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::windows_remote_settings::PreparedFileIconsRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::ReadFileIcons {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows file icon observation timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn file_icons_transaction(
        &self,
        permit: DesktopPermit,
        transaction: nickel_remote_control::file_icons::Transaction,
    ) -> Result<nickel_remote_control::file_icons::Snapshot, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        permit.with_debug(false, || Ok(()))?;
        let prepared =
            crate::windows_remote_settings::PreparedFileIconsChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        if Instant::now() >= deadline {
            return Err("Windows file icon preparation timed out".into());
        }
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::FileIconsTransaction {
                permit,
                transaction,
                prepared,
                deadline,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows file icon transaction expired before dispatch")?;
        receiver.recv_timeout(remaining).map_err(|_| {
            "Windows file icon result uncertain; read current state before retrying".to_owned()
        })?
    }
    fn read_wallpaper(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::wallpaper::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        let prepared =
            crate::windows_remote_settings::PreparedWallpaperRead::prepare_with_check(|| {
                permit.check_live()
            })?;
        permit.with_debug(false, || Ok(()))?;
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::ReadWallpaper {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows wallpaper observation timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn wallpaper_transaction(
        &self,
        permit: DesktopPermit,
        transaction: nickel_remote_control::wallpaper::Transaction,
    ) -> Result<nickel_remote_control::wallpaper::Snapshot, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let expected_local_input_epoch = local_input_epoch();
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::windows_remote_settings::PreparedWallpaperChange::prepare_with_check(
            &transaction,
            || permit.check_live(),
        )?;
        permit.with_debug(false, || Ok(()))?;
        if Instant::now() >= deadline {
            return Err("Windows wallpaper preparation timed out".into());
        }
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::WallpaperTransaction {
                permit,
                transaction,
                prepared,
                deadline,
                expected_local_input_epoch,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows wallpaper transaction expired before dispatch")?;
        receiver.recv_timeout(remaining).map_err(|_| {
            "Windows wallpaper result uncertain; read current wallpaper before retrying".to_owned()
        })?
    }
    fn read_terminal_presentation(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::terminal_presentation::Snapshot, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::windows_remote_terminal_presentation::PreparedRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows terminal presentation observation expired before dispatch")?;
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::ReadTerminalPresentation {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(remaining)
            .map_err(|_| "Windows terminal presentation observation timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn terminal_presentation_transaction(
        &self,
        permit: DesktopPermit,
        transaction: nickel_remote_control::terminal_presentation::Transaction,
    ) -> Result<nickel_remote_control::terminal_presentation::Snapshot, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let expected_local_input_epoch = local_input_epoch();
        permit.with_debug(false, || Ok(()))?;
        let prepared =
            crate::windows_remote_terminal_presentation::PreparedChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows terminal presentation transaction expired before dispatch")?;
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::TerminalPresentationTransaction {
                permit,
                transaction,
                prepared: Box::new(prepared),
                deadline,
                expected_local_input_epoch,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        receiver.recv_timeout(remaining).map_err(|_| {
            "Windows terminal presentation result uncertain; read current state before retrying"
                .to_owned()
        })?
    }
    fn read_terminal_launch_policy(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::terminal_launch_policy::Snapshot, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::remote_terminal_launch_policy::PreparedRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows terminal launch-policy observation expired before dispatch")?;
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::ReadTerminalLaunchPolicy {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(remaining)
            .map_err(|_| "Windows terminal launch-policy observation timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn terminal_launch_policy_transaction(
        &self,
        permit: DesktopPermit,
        transaction: nickel_remote_control::terminal_launch_policy::Transaction,
    ) -> Result<nickel_remote_control::terminal_launch_policy::TransactionOutcome, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let expected_local_input_epoch = local_input_epoch();
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::remote_terminal_launch_policy::PreparedChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows terminal launch-policy transaction expired before dispatch")?;
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::TerminalLaunchPolicyTransaction {
                permit,
                transaction,
                prepared: Box::new(prepared),
                deadline,
                expected_local_input_epoch,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        receiver.recv_timeout(remaining).map_err(|_| {
            "Windows terminal launch-policy result uncertain; read current state before retrying"
                .to_owned()
        })?
    }
    fn read_codex_preference(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::codex_preference::Snapshot, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::windows_remote_codex::PreparedRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows Codex preference observation expired before dispatch")?;
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::ReadCodexPreference {
                permit,
                prepared,
                deadline,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(remaining)
            .map_err(|_| "Windows Codex preference observation timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn codex_preference_transaction(
        &self,
        permit: DesktopPermit,
        transaction: nickel_remote_control::codex_preference::Transaction,
    ) -> Result<nickel_remote_control::codex_preference::Snapshot, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let expected_local_input_epoch = local_input_epoch();
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::windows_remote_codex::PreparedChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows Codex preference transaction expired before dispatch")?;
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::CodexPreferenceTransaction {
                permit,
                transaction,
                prepared,
                deadline,
                expected_local_input_epoch,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        receiver.recv_timeout(remaining).map_err(|_| {
            "Windows Codex preference result uncertain; read current state before retrying"
                .to_owned()
        })?
    }
    fn shell_behavior_transaction(
        &self,
        permit: DesktopPermit,
        transaction: nickel_session_protocol::ShellBehaviorTransaction,
    ) -> Result<nickel_remote_control::diagnostics::ShellBehaviorDiagnostic, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let expected_local_input_epoch = local_input_epoch();
        permit.with_debug(false, || Ok(()))?;
        let prepared =
            crate::windows_remote_settings::PreparedShellBehaviorChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows shell behavior transaction expired before dispatch")?;
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::ShellBehaviorTransaction {
                permit,
                transaction,
                prepared,
                deadline,
                expected_local_input_epoch,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        receiver.recv_timeout(remaining).map_err(|_| {
            "Windows shell behavior result uncertain; read diagnostics before retrying".to_owned()
        })?
    }
    fn read_idle_preferences(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::idle_preferences::Snapshot, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::windows_remote_settings::PreparedIdleRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows idle preference observation expired before dispatch")?;
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::ReadIdlePreferences {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(remaining)
            .map_err(|_| "Windows idle preference observation timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn idle_preferences_transaction(
        &self,
        permit: DesktopPermit,
        transaction: nickel_remote_control::idle_preferences::Transaction,
    ) -> Result<nickel_remote_control::idle_preferences::Snapshot, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let expected_local_input_epoch = local_input_epoch();
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::windows_remote_settings::PreparedIdleChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows idle preference transaction expired before dispatch")?;
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::IdlePreferencesTransaction {
                permit,
                transaction,
                prepared,
                deadline,
                expected_local_input_epoch,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        receiver.recv_timeout(remaining).map_err(|_| {
            "Windows idle preference result uncertain; read current state before retrying"
                .to_owned()
        })?
    }
    fn read_keyboard_preference(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::keyboard_preference::Snapshot, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::windows_remote_settings::PreparedKeyboardRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows keyboard preference observation expired before dispatch")?;
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::ReadKeyboardPreference {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(remaining)
            .map_err(|_| "Windows keyboard preference observation timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn keyboard_preference_transaction(
        &self,
        permit: DesktopPermit,
        transaction: nickel_remote_control::keyboard_preference::Transaction,
    ) -> Result<nickel_remote_control::keyboard_preference::Snapshot, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let expected_local_input_epoch = local_input_epoch();
        permit.with_debug(false, || Ok(()))?;
        let prepared =
            crate::windows_remote_settings::PreparedKeyboardChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows keyboard preference transaction expired before dispatch")?;
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::KeyboardPreferenceTransaction {
                permit,
                transaction,
                prepared,
                deadline,
                expected_local_input_epoch,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        receiver.recv_timeout(remaining).map_err(|_| {
            "Windows keyboard preference result uncertain; read current state before retrying"
                .to_owned()
        })?
    }
    fn read_launcher_favorites(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::launcher_favorites::Snapshot, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        permit.with_debug(false, || Ok(()))?;
        let catalog = self.launcher_favorites_catalog(permit.clone(), deadline)?;
        let prepared = crate::windows_remote_launcher_favorites::PreparedRead::prepare(catalog)?;
        permit.with_debug(false, || Ok(()))?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows launcher favorites observation expired before dispatch")?;
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::ReadLauncherFavorites {
                permit,
                prepared,
                deadline,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(remaining)
            .map_err(|_| "Windows launcher favorites observation timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn launcher_favorites_transaction(
        &self,
        permit: DesktopPermit,
        transaction: nickel_remote_control::launcher_favorites::Transaction,
    ) -> Result<nickel_remote_control::launcher_favorites::Snapshot, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let expected_local_input_epoch = local_input_epoch();
        permit.with_debug(false, || Ok(()))?;
        let catalog = self.launcher_favorites_catalog(permit.clone(), deadline)?;
        let prepared = crate::windows_remote_launcher_favorites::PreparedChange::prepare(
            catalog,
            &transaction,
        )?;
        permit.with_debug(false, || Ok(()))?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows launcher favorites transaction expired before dispatch")?;
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::LauncherFavoritesTransaction {
                permit,
                transaction,
                prepared,
                deadline,
                expected_local_input_epoch,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        receiver.recv_timeout(remaining).map_err(|_| {
            "Windows launcher favorites result uncertain; read current launcher favorites before retrying"
                .to_owned()
        })?
    }
    fn inspect_native_window(
        &self,
        permit: DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<nickel_remote_control::native_semantics::NativeSemanticSnapshot, String> {
        self.inspect_native_accessibility(permit, id, generation, false)
    }
    fn inspect_native_application(
        &self,
        permit: DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<nickel_remote_control::native_semantics::NativeSemanticSnapshot, String> {
        self.inspect_native_accessibility(permit, id, generation, true)
    }
    fn native_semantic_action(
        &self,
        permit: DesktopPermit,
        request: nickel_remote_control::native_semantics::NativeSemanticActionRequest,
    ) -> Result<nickel_remote_control::native_semantics::NativeSemanticActionOutcome, String> {
        self.perform_native_accessibility_action(permit, request)
    }
    fn list_installed_applications(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::diagnostics::ApplicationInventory, String> {
        let prepared = match permit.resource_scope()? {
            nickel_remote_control::leases::ResourceScope::Output(_) => Some(Box::new(
                crate::platform::remote_observation::Prepared::prepare(&permit)?,
            )),
            _ => None,
        };
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::Applications {
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
    fn launch_installed_application(
        &self,
        permit: DesktopPermit,
        request: nickel_remote_control::diagnostics::LaunchApplicationRequest,
    ) -> Result<nickel_remote_control::diagnostics::LaunchApplicationOutcome, String> {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct CancelOnDrop(Arc<AtomicBool>);
        impl Drop for CancelOnDrop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let completion = permit.clone();
        let deadline = Instant::now() + Duration::from_secs(2);
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel_on_drop = CancelOnDrop(cancelled.clone());
        let prepared = match permit.resource_scope()? {
            nickel_remote_control::leases::ResourceScope::Output(_) => Some(Box::new(
                crate::platform::remote_observation::Prepared::prepare(&permit)?,
            )),
            _ => None,
        };
        let (plan_reply, plan_receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::LaunchApplication {
                permit: permit.clone(),
                request: request.clone(),
                prepared,
                deadline,
                cancelled: cancelled.clone(),
                reply: plan_reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows application launch timed out")?;
        let plan = plan_receiver
            .recv_timeout(remaining)
            .map_err(|_| "Windows desktop owner timed out".to_owned())??;
        completion.check_live()?;
        if Instant::now() >= deadline {
            return Err("Windows application launch timed out".into());
        }
        // Shortcut reads, hashing and pinning occur on a single-flight worker,
        // never on the winit presentation owner or its request-serving caller.
        // A blocked filesystem operation retains admission, so later requests
        // fail closed instead of creating an unbounded set of stuck threads.
        let admission = LaunchPreparationAdmission::acquire()?;
        let (prepare_reply, prepare_receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("nickel-windows-launch-prepare".to_owned())
            .spawn(move || {
                let _admission = admission;
                let prepared = plan.plan.prepare().and_then(|(native_plan, capture)| {
                    let staged =
                        crate::windows_launch_broker::StagedLaunch::new(capture, deadline)?;
                    Ok((
                        PlannedApplicationLaunch {
                            plan: native_plan,
                            output: plan.output,
                            commit_evidence: None,
                        },
                        staged,
                    ))
                });
                // If the caller's bounded receive has expired, dropping the
                // staged result terminates its still-suspended broker.
                let _ = prepare_reply.try_send(prepared);
            })
            .map_err(|_| "Windows application launch worker is unavailable".to_owned())?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows application launch timed out")?;
        let (mut plan, staged) =
            prepare_receiver
                .recv_timeout(remaining)
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => {
                        "Windows application launch timed out".to_owned()
                    }
                    mpsc::RecvTimeoutError::Disconnected => {
                        "Windows application launch worker stopped".to_owned()
                    }
                })??;
        completion.check_live()?;
        if Instant::now() >= deadline {
            return Err("Windows application launch timed out".into());
        }
        plan.commit_evidence = if plan.output.is_some() {
            Some(Box::new(
                crate::platform::remote_observation::Prepared::prepare(&permit)?,
            ))
        } else {
            None
        };
        let outcome_generation = request.catalog_generation;
        let outcome_application = request.application_id.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::CommitApplicationLaunch {
                permit: permit.clone(),
                request,
                plan: Box::new(plan),
                staged: Box::new(staged),
                deadline,
                cancelled: cancelled.clone(),
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows application launch timed out")?;
        let authorized = receiver
            .recv_timeout(remaining)
            .map_err(|_| "Windows desktop owner timed out".to_owned())??;
        // ResumeThread was the irreversible, freshly authorized commit. A
        // revocation after that point must not report failure and invite retry.
        let launched = authorized.committed.finish(deadline);
        let output_requested = authorized
            .placement
            .as_ref()
            .map(|placement| placement.output.clone());
        match (authorized.placement, launched.process.clone()) {
            (Some(ticket), Some(root)) => {
                if !start_output_placement_worker(self.sender.clone(), permit, ticket.clone(), root)
                {
                    let _ = self
                        .sender
                        .try_send(OwnerRequest::CancelApplicationPlacement { ticket });
                }
            }
            (Some(ticket), None) => {
                let _ = self
                    .sender
                    .try_send(OwnerRequest::CancelApplicationPlacement { ticket });
            }
            (None, _) => {}
        }
        Ok(
            nickel_remote_control::diagnostics::LaunchApplicationOutcome {
                catalog_generation: outcome_generation,
                application_id: outcome_application,
                requested: true,
                process_spawn_confirmed: launched.process_spawn_confirmed,
                process_id: launched.process_id,
                output_requested,
                output_confirmed: false,
            },
        )
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
        target: nickel_remote_control::pointer::PointerTarget,
        x: i32,
        y: i32,
        action: nickel_remote_control::pointer::PointerAction,
    ) -> Result<(), String> {
        let nickel_remote_control::pointer::PointerTarget::Window {
            window_id: id,
            generation,
        } = target
        else {
            return Err("non-window pointer targets are unavailable on Windows".into());
        };
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
                    id,
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
        let prepared =
            Box::new(crate::platform::remote_observation::Prepared::prepare_with_input(&permit)?);
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
        let platform_refresh = match &action {
            nickel_remote_control::diagnostics::DiagnosticAction::RefreshPlatformStatus {
                domain,
            } => Some(prepare_windows_platform_refresh(
                &permit,
                self.platform_refresh_worker.clone(),
                *domain,
            )?),
            _ => None,
        };
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::DiagnosticAction {
                permit,
                action,
                platform_refresh,
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
    fn read_display_layout(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::display_layout::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        let inventory = self.list_outputs(permit.clone())?;
        if inventory.truncated || inventory.outputs.is_empty() {
            return Err("complete Windows display layout is unavailable".into());
        }
        let primary = inventory
            .outputs
            .iter()
            .find(|output| output.primary && output.enabled)
            .ok_or("Windows display layout has no enabled primary output")?;
        let primary = nickel_remote_control::leases::ResourceId {
            id: primary.name.clone(),
            generation: primary.generation,
        };
        let mut outputs = inventory
            .outputs
            .iter()
            .map(|output| nickel_remote_control::display_layout::Placement {
                output: nickel_remote_control::leases::ResourceId {
                    id: output.name.clone(),
                    generation: output.generation,
                },
                x: output.geometry[0],
                y: output.geometry[1],
                enabled: output.enabled,
                scale_120: output.scale_120,
            })
            .collect::<Vec<_>>();
        outputs.sort_by(|left, right| left.output.id.cmp(&right.output.id));
        let layout = nickel_remote_control::display_layout::Layout { primary, outputs };
        if !layout.valid_representation() {
            return Err("Windows display owner reported an invalid layout".into());
        }
        permit.check_live()?;
        Ok(nickel_remote_control::display_layout::Snapshot {
            observation_generation: inventory.observation_generation,
            observed_at_us: inventory.observed_at_us,
            topology_generation: inventory.topology_generation,
            transaction_supported: false,
            transaction_unavailable_reason: Some(
                "Nickel has no production Windows display reconfiguration owner".into(),
            ),
            requested: layout.clone(),
            confirmed: layout,
            recovery: nickel_remote_control::display_layout::Recovery {
                state: nickel_remote_control::display_layout::RecoveryState::Confirmed,
                generation: 0,
                deadline_uptime_us: None,
            },
        })
    }
    fn display_layout_transaction(
        &self,
        permit: DesktopPermit,
        _transaction: nickel_remote_control::display_layout::Transaction,
    ) -> Result<nickel_remote_control::display_layout::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        Err("Windows display layout transactions are unavailable: Nickel has no production Windows display reconfiguration owner".into())
    }
    fn list_surfaces(
        &self,
        permit: DesktopPermit,
    ) -> Result<Vec<nickel_remote_control::diagnostics::ShellSurfaceDiagnostic>, String> {
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::ShellSurfaces { permit, reply })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows desktop owner timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn inspect_surface(
        &self,
        permit: DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<nickel_remote_control::semantics::SurfaceSemanticSnapshot, String> {
        let completion = permit.clone();
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::InspectShellSurface {
                permit,
                id: id.to_owned(),
                generation,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Windows desktop owner timed out".to_owned())?;
        completion.check_live()?;
        result
    }
    fn surface_semantic_action(
        &self,
        permit: DesktopPermit,
        request: nickel_remote_control::semantics::SurfaceSemanticActionRequest,
    ) -> Result<nickel_remote_control::semantics::SurfaceSemanticActionOutcome, String> {
        request.validate().map_err(str::to_owned)?;
        let completion = permit.clone();
        let expected_local_input_epoch = local_input_epoch();
        let deadline = Instant::now() + Duration::from_secs(2);
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::ShellSemanticAction {
                permit,
                request,
                deadline,
                expected_local_input_epoch,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        let result = receiver.recv_timeout(Duration::from_secs(2)).map_err(|_| {
            "semantic action result uncertain: Windows desktop owner timed out; do not retry"
                .to_owned()
        })?;
        completion.check_live()?;
        result
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
    fn validate_output_capture(
        &self,
        _permit: DesktopPermit,
        _id: &str,
        _generation: u64,
    ) -> Result<(), String> {
        Err("Windows output pixel capture is unavailable".into())
    }
    fn capture_output(
        &self,
        _permit: DesktopPermit,
        _id: &str,
        _generation: u64,
    ) -> Result<nickel_remote_control::capture::CapturedWindow, String> {
        Err("Windows output pixel capture is unavailable".into())
    }
    fn window_action(
        &self,
        permit: DesktopPermit,
        id: &str,
        generation: u64,
        action: nickel_remote_control::window_actions::WindowAction,
    ) -> Result<nickel_remote_control::window_actions::WindowOutcome, String> {
        action.validate()?;
        let workspace_move = matches!(
            &action,
            nickel_remote_control::window_actions::WindowAction::MoveToWorkspace { .. }
        );
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
        // A workspace move is irreversible once the native manager accepts
        // it. Its owner result remains truthful if revocation races after that
        // boundary; returning a denial here would invite an unsafe retry.
        if !workspace_move || result.is_err() {
            completion.check_live()?;
        }
        result
    }
}

impl WindowsRemoteControl {
    fn read_idle_preferences(
        &mut self,
        permit: &DesktopPermit,
        prepared: crate::windows_remote_settings::PreparedIdleRead,
        state: &crate::live_shell::LiveShell,
    ) -> Result<nickel_remote_control::idle_preferences::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.ensure_current()?;
        let protected =
            !self.desktop_unlocked || state.surface_visible(crate::winit_shell::SurfaceRole::Lock);
        permit.with_debug(protected, || {
            self.idle_preferences.observe(
                &prepared,
                state.windows_idle_preferences(),
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn change_idle_preferences(
        &mut self,
        shell: &WinitShell,
        state: &mut crate::live_shell::LiveShell,
        permit: &DesktopPermit,
        transaction: nickel_remote_control::idle_preferences::Transaction,
        prepared: crate::windows_remote_settings::PreparedIdleChange,
        request_deadline: Instant,
        expected_local_input_epoch: u64,
    ) -> Result<nickel_remote_control::idle_preferences::Snapshot, String> {
        let protected = !self.desktop_unlocked
            || state.surface_visible(crate::winit_shell::SurfaceRole::Lock)
            || shell
                .remote_shell_surface_observations(state)
                .iter()
                .any(|surface| surface.keyboard_focused && surface.protected);
        let input_busy = self.keyboard_hold.is_some()
            || self.pointer_hold.is_some()
            || state.pointer_interaction_active()
            || !crate::windows_remote_input::physical_input_idle();
        let mut committed = None;
        let authorization = permit.with_debug_input_deadline(protected, |boundary| {
            if Instant::now() >= request_deadline {
                return Err("idle preference transaction expired before commit".into());
            }
            if input_busy
                || local_input_epoch() != expected_local_input_epoch
                || !crate::windows_remote_input::physical_input_idle()
            {
                return Err("shared input is busy".into());
            }
            self.idle_preferences.validate(&prepared, &transaction)?;
            committed = Some(
                prepared.commit(boundary.deadline().min(request_deadline), || {
                    if local_input_epoch() != expected_local_input_epoch
                        || !crate::windows_remote_input::physical_input_idle()
                    {
                        return Err("local input interrupted the settings transaction".into());
                    }
                    permit.check_commit_boundary(boundary)
                })?,
            );
            self.idle_preferences.invalidate();
            Ok(())
        });
        let (requested, revision) = match committed {
            Some(value) => value,
            None => {
                authorization?;
                return Err(
                    "idle preferences unavailable; read current state before retrying".into(),
                );
            }
        };
        // The production shell owner must consume a committed policy even if
        // authority is revoked immediately after the filesystem replacement.
        state.apply_shell_settings(requested);
        authorization?;
        let read = crate::windows_remote_settings::PreparedIdleRead::prepare()?;
        if read.revision != revision {
            return Err("idle preferences changed; read current state before retrying".into());
        }
        self.idle_preferences.observe(
            &read,
            state.windows_idle_preferences(),
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
        )
    }

    fn read_keyboard_preference(
        &self,
        permit: &DesktopPermit,
        prepared: crate::windows_remote_settings::PreparedKeyboardRead,
        state: &crate::live_shell::LiveShell,
    ) -> Result<nickel_remote_control::keyboard_preference::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.ensure_current()?;
        let protected =
            !self.desktop_unlocked || state.surface_visible(crate::winit_shell::SurfaceRole::Lock);
        permit.with_debug(protected, || {
            Ok(crate::windows_remote_settings::keyboard_snapshot(
                &prepared,
                state.windows_keyboard_snapshot(),
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
            ))
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn change_keyboard_preference(
        &mut self,
        shell: &mut WinitShell,
        state: &mut crate::live_shell::LiveShell,
        permit: &DesktopPermit,
        transaction: nickel_remote_control::keyboard_preference::Transaction,
        prepared: crate::windows_remote_settings::PreparedKeyboardChange,
        request_deadline: Instant,
        expected_local_input_epoch: u64,
    ) -> Result<nickel_remote_control::keyboard_preference::Snapshot, String> {
        let protected = !self.desktop_unlocked
            || state.surface_visible(crate::winit_shell::SurfaceRole::Lock)
            || shell
                .remote_shell_surface_observations(state)
                .iter()
                .any(|surface| surface.keyboard_focused && surface.protected);
        let input_busy = self.keyboard_hold.is_some()
            || self.pointer_hold.is_some()
            || state.pointer_interaction_active()
            || !crate::windows_remote_input::physical_input_idle();
        let mut committed = None;
        let authorization = permit.with_debug_input_deadline(protected, |boundary| {
            if Instant::now() >= request_deadline {
                return Err("keyboard preference transaction expired before commit".into());
            }
            if input_busy
                || local_input_epoch() != expected_local_input_epoch
                || !crate::windows_remote_input::physical_input_idle()
            {
                return Err("shared input is busy".into());
            }
            prepared.validate(&transaction)?;
            committed = Some(
                prepared.commit(boundary.deadline().min(request_deadline), || {
                    if local_input_epoch() != expected_local_input_epoch
                        || !crate::windows_remote_input::physical_input_idle()
                    {
                        return Err("local input interrupted the settings transaction".into());
                    }
                    permit.check_commit_boundary(boundary)
                })?,
            );
            Ok(())
        });
        let (settings, revision) = match committed {
            Some(value) => value,
            None => {
                authorization?;
                return Err(
                    "keyboard preference unavailable; read current state before retrying".into(),
                );
            }
        };
        // Reconcile the production LiveShell owner after the durable boundary,
        // including a raced post-commit revocation.
        let changed = state.apply_windows_keyboard_settings(&settings);
        if changed {
            crate::sync_visibility(shell, state);
            shell.request_all_redraws();
        }
        authorization?;
        let read = crate::windows_remote_settings::PreparedKeyboardRead::prepare()?;
        if read.revision != revision {
            return Err("keyboard preference changed; read current state before retrying".into());
        }
        Ok(crate::windows_remote_settings::keyboard_snapshot(
            &read,
            state.windows_keyboard_snapshot(),
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
        ))
    }
}

impl WindowsDesktopAuthority {
    fn launcher_favorites_catalog(
        &self,
        permit: DesktopPermit,
        deadline: Instant,
    ) -> Result<crate::windows_remote_launcher_favorites::Catalog, String> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Windows launcher catalog observation expired before dispatch")?;
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(OwnerRequest::LauncherFavoritesCatalog {
                permit,
                deadline,
                reply,
            })
            .map_err(|_| "Windows desktop owner is busy or stopped".to_owned())?;
        receiver
            .recv_timeout(remaining)
            .map_err(|_| "Windows launcher catalog observation timed out".to_owned())?
    }
}

pub(crate) struct WindowsRemoteControl {
    _transport: Option<nickel_platform::local_control::LocalControlServer>,
    receiver: Receiver<OwnerRequest>,
    remote_control: RemoteControlRuntime,
    local_cues: crate::local_cues::LocalCues,
    applications: crate::windows_application_registry::native::OwnerRegistry,
    resources: crate::windows_resource_owner::Owner,
    workspaces: crate::windows_virtual_workspaces::Owner,
    resource_lifecycle: Option<crate::platform::remote_observation::Lifecycle>,
    observation_generation: u64,
    platform_refresh_generation: u64,
    platform_refreshes: Vec<nickel_remote_control::diagnostics::PlatformRefreshOutcome>,
    platform_refresh_worker: Arc<WindowsPlatformRefreshWorker>,
    remote_frame_trace: Option<nickel_remote_control::frame_trace::FrameTrace>,
    indicators: std::collections::HashMap<String, IndicatorSurface>,
    authority: Arc<WindowsDesktopAuthority>,
    desktop_session: Option<u32>,
    desktop_unlocked: bool,
    local_input_epoch: u64,
    keyboard_hold: Option<WindowsKeyboardHold>,
    pointer_hold: Option<WindowsPointerHold>,
    desktop_events: nickel_remote_control::desktop_events::DesktopEvents,
    appearance: crate::windows_remote_settings::AppearanceState,
    application_scale: crate::windows_remote_application_scale::State,
    file_icons: crate::windows_remote_settings::FileIconState,
    wallpaper: crate::windows_remote_settings::WallpaperState,
    terminal_presentation: crate::windows_remote_terminal_presentation::State,
    terminal_launch_policy: crate::remote_terminal_launch_policy::State,
    idle_preferences: crate::windows_remote_settings::IdleState,
    launcher_favorites: crate::windows_remote_launcher_favorites::FavoritesState,
    shell_focus: Option<ShellFocusState>,
    external_accessibility:
        Option<nickel_remote_control::diagnostics::ExternalAccessibilityDiagnostic>,
    native_action_observations:
        std::collections::VecDeque<crate::windows_external_accessibility::ActionObservation>,
    pending_indicator_activation: std::collections::BTreeSet<u64>,
    pending_output_launches: std::collections::BTreeMap<u64, PendingOutputLaunchPlacement>,
    next_output_launch: u64,
    start_time: Instant,
    last_stop: Option<Instant>,
    stop_confirmation_until: Option<Instant>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellFocusState {
    Cleared,
    Surface {
        generation: u64,
        role: nickel_remote_control::desktop_events::ShellEventRole,
    },
}

fn shell_behavior_diagnostic(
    observation_generation: u64,
    observed_at_us: u64,
    topology_generation: u64,
    bar_on_all_displays: bool,
    state: (bool, u8, usize),
) -> nickel_remote_control::diagnostics::ShellBehaviorDiagnostic {
    nickel_remote_control::diagnostics::ShellBehaviorDiagnostic {
        observation_generation,
        observed_at_us,
        topology_generation,
        bar_on_all_displays,
        all_windows_on_every_bar: state.0,
        configured_desktop_count: state.1,
        runtime_desktop_count: state.2,
    }
}

fn codex_feature_diagnostic(
    observation_generation: u64,
    observed_at_us: u64,
    state: &crate::live_shell::LiveShell,
) -> Option<nickel_remote_control::diagnostics::CodexFeatureDiagnostic> {
    use nickel_core::optional_features::{FeatureHealth, FeatureInstallation, FeatureSupport};
    use nickel_remote_control::diagnostics::{
        CodexFeatureDiagnostic, FeatureHealthDiagnostic, FeatureInstallationDiagnostic,
    };
    let projection = state.codex_projection()?;
    Some(CodexFeatureDiagnostic {
        observation_generation,
        observed_at_us,
        supported: projection.support == FeatureSupport::Supported,
        installation: match projection.installation {
            FeatureInstallation::Installed => FeatureInstallationDiagnostic::Installed,
            FeatureInstallation::Missing => FeatureInstallationDiagnostic::Missing,
            FeatureInstallation::Incompatible => FeatureInstallationDiagnostic::Incompatible,
        },
        enabled: projection.enabled,
        health: match projection.health {
            FeatureHealth::Unknown => FeatureHealthDiagnostic::Unknown,
            FeatureHealth::Loading => FeatureHealthDiagnostic::Loading,
            FeatureHealth::SignedOut => FeatureHealthDiagnostic::SignedOut,
            FeatureHealth::Ready => FeatureHealthDiagnostic::Ready,
            FeatureHealth::Failed => FeatureHealthDiagnostic::Failed,
        },
        configuration_generation: projection.generation,
    })
}

impl WindowsRemoteControl {
    fn application_launch_diagnostic(
        &self,
    ) -> nickel_remote_control::diagnostics::ApplicationLaunchDiagnostic {
        let (tracked_children, child_capacity) = self.applications.launch_process_diagnostic();
        nickel_remote_control::diagnostics::ApplicationLaunchDiagnostic {
            preparation: launch_preparation_state().snapshot(),
            tracked_children,
            child_capacity,
        }
    }

    pub(crate) fn start(
        cleanup_wake: nickel_remote_control::ConnectionCleanupWake,
    ) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(16);
        let desktop_session =
            nickel_platform::process_identity::WindowsProcessIdentity::probe(std::process::id())
                .ok()
                .map(|identity| identity.session_id());
        let started = Instant::now();
        let platform_refresh_worker = Arc::new(WindowsPlatformRefreshWorker::default());
        let authority = Arc::new(WindowsDesktopAuthority {
            cleanup_wake,
            sender: sender.clone(),
            started,
            desktop_session,
            capture_generation: std::sync::atomic::AtomicU64::new(0),
            peripheral_generation: std::sync::atomic::AtomicU64::new(0),
            platform_refresh_worker: platform_refresh_worker.clone(),
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
            workspaces: Default::default(),
            resource_lifecycle: crate::platform::remote_observation::Lifecycle::install().ok(),
            observation_generation: 0,
            platform_refresh_generation: 0,
            platform_refreshes: Vec::new(),
            platform_refresh_worker,
            remote_frame_trace: None,
            indicators: Default::default(),
            authority,
            desktop_session,
            desktop_unlocked,
            local_input_epoch: local_input_epoch(),
            keyboard_hold: None,
            pointer_hold: None,
            desktop_events: Default::default(),
            appearance: Default::default(),
            application_scale: Default::default(),
            file_icons: Default::default(),
            wallpaper: Default::default(),
            terminal_presentation: Default::default(),
            terminal_launch_policy: Default::default(),
            idle_preferences: Default::default(),
            launcher_favorites: Default::default(),
            shell_focus: None,
            external_accessibility: None,
            native_action_observations: Default::default(),
            pending_indicator_activation: Default::default(),
            pending_output_launches: Default::default(),
            next_output_launch: 0,
            start_time: started,
            last_stop: None,
            stop_confirmation_until: None,
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
    pub(crate) fn poll(
        &mut self,
        shell: &mut WinitShell,
        state: &mut crate::live_shell::LiveShell,
        codex: &mut crate::CodexSurfaces,
        feature_settings: &mut nickel_core::optional_features::OptionalFeatureSettings,
    ) {
        self.collect_frame_dispatches(shell, state);
        self.poll_with_shell(Some((shell, state)), Some((codex, feature_settings)));
    }

    fn collect_frame_dispatches(
        &mut self,
        shell: &mut WinitShell,
        state: &crate::live_shell::LiveShell,
    ) {
        let protected =
            !self.desktop_unlocked || state.surface_visible(crate::winit_shell::SurfaceRole::Lock);
        if self
            .remote_frame_trace
            .as_mut()
            .is_some_and(|trace| !trace.revalidate(protected))
        {
            self.remote_frame_trace = None;
        }
        let (dropped, dispatches) = shell.take_remote_frame_dispatches();
        if let Some(trace) = self.remote_frame_trace.as_mut()
            && !trace.record_drops(protected, dropped)
        {
            self.remote_frame_trace = None;
            return;
        }
        for (output, elapsed_us) in dispatches {
            let generation = self.resources.output_generation(&output);
            if let (Some(trace), Some(generation)) = (self.remote_frame_trace.as_mut(), generation)
                && !trace.record(protected, generation, Duration::from_micros(elapsed_us))
            {
                self.remote_frame_trace = None;
                break;
            }
        }
    }

    /// Retain only fixed, production-owned shell visibility and keyboard-focus
    /// transitions. Input payloads, geometry, output names and protected shell
    /// identities never enter the event history.
    pub(crate) fn observe_shell_event(
        &mut self,
        shell: &WinitShell,
        state: &crate::live_shell::LiveShell,
        event: &ShellEvent,
    ) {
        use nickel_remote_control::desktop_events::DesktopEventKind;

        let (surface, visibility, focus) = match event {
            ShellEvent::Shown(surface) => (*surface, Some(true), None),
            ShellEvent::Hidden(surface) => (*surface, Some(false), None),
            ShellEvent::FocusChanged { surface, focused } => (*surface, None, Some(*focused)),
            _ => return,
        };
        let Some(observation) = shell.remote_shell_surface_observation(surface, state) else {
            return;
        };
        let native_desktop_unlocked = self.desktop_unlocked
            && self
                .desktop_session
                .is_some_and(crate::platform::remote_observation::desktop_is_unlocked);
        let protected_desktop = !native_desktop_unlocked
            || state.surface_visible(crate::winit_shell::SurfaceRole::Lock);
        let observed_at_us = self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64;

        if let Some(visible) = visibility {
            if let Some((surface_generation, role)) =
                crate::windows_shell_diagnostics::project_visibility_event(
                    protected_desktop,
                    &observation,
                    visible,
                )
            {
                self.desktop_events.record(
                    DesktopEventKind::ShellSurfaceVisibilityChanged {
                        surface_generation,
                        role,
                        visible,
                    },
                    observed_at_us,
                );
            }
            if !visible
                && !observation.keyboard_focused
                && matches!(
                    self.shell_focus,
                    Some(ShellFocusState::Surface { generation, .. })
                        if generation == observation.generation
                )
            {
                self.record_shell_focus(ShellFocusState::Cleared, observed_at_us);
            }
            return;
        }

        let Some(focused) = focus else {
            return;
        };
        if focused {
            // Ignore an obsolete queued focus event. If the current native
            // recipient is protected or unsupported, expose only a clear.
            if !observation.keyboard_focused {
                return;
            }
            let next = crate::windows_shell_diagnostics::project_focus_event(
                protected_desktop,
                &observation,
            )
            .map_or(ShellFocusState::Cleared, |(generation, role)| {
                ShellFocusState::Surface { generation, role }
            });
            self.record_shell_focus(next, observed_at_us);
        } else if !observation.keyboard_focused
            && matches!(
                self.shell_focus,
                Some(ShellFocusState::Surface { generation, .. })
                    if generation == observation.generation
            )
        {
            self.record_shell_focus(ShellFocusState::Cleared, observed_at_us);
        }
    }

    fn record_shell_focus(&mut self, next: ShellFocusState, observed_at_us: u64) {
        use nickel_remote_control::desktop_events::DesktopEventKind;

        if self.shell_focus == Some(next) {
            return;
        }
        self.shell_focus = Some(next);
        let event = match next {
            ShellFocusState::Cleared => DesktopEventKind::KeyboardFocusCleared,
            ShellFocusState::Surface { generation, role } => {
                DesktopEventKind::ShellKeyboardFocusChanged {
                    surface_generation: generation,
                    role,
                }
            }
        };
        self.desktop_events.record(event, observed_at_us);
    }

    fn poll_with_shell(
        &mut self,
        mut shell: Option<(&mut WinitShell, &mut crate::live_shell::LiveShell)>,
        mut codex: Option<(
            &mut crate::CodexSurfaces,
            &mut nickel_core::optional_features::OptionalFeatureSettings,
        )>,
    ) {
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
            if let Some((shell, state)) = shell.as_mut() {
                let theme = state.semantic_theme();
                let _ = self.sync_indicators(shell, theme);
            }
        }
        if self
            .stop_confirmation_until
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.stop_confirmation_until = None;
            if let Some((shell, _)) = shell.as_mut() {
                self.clear_indicators(shell);
            }
        }
        self.prune_output_launch_placements();
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
                OwnerRequest::PrepareNativeAccessibility {
                    permit,
                    mut prepared,
                    id,
                    generation,
                    application_wide,
                    reply,
                } => {
                    let result = self.prepare_native_accessibility(
                        &permit,
                        &mut prepared,
                        &id,
                        generation,
                        application_wide,
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::FinishNativeAccessibility {
                    permit,
                    mut prepared,
                    proof,
                    mut observation,
                    reply,
                } => {
                    let result = self.finish_native_accessibility(
                        &permit,
                        &mut prepared,
                        &proof,
                        &mut observation,
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::PrepareNativeAccessibilityAction {
                    permit,
                    mut prepared,
                    request,
                    reply,
                } => {
                    let result =
                        self.prepare_native_accessibility_action(&permit, &mut prepared, &request);
                    let _ = reply.try_send(result);
                }
                OwnerRequest::CommitNativeAccessibilityAction {
                    permit,
                    mut prepared,
                    plan,
                    deadline,
                    expected_local_input_epoch,
                    dispatch,
                    reply,
                } => {
                    let result = self.commit_native_accessibility_action(
                        &permit,
                        &mut prepared,
                        &plan,
                        deadline,
                        expected_local_input_epoch,
                        &dispatch,
                    );
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
                OwnerRequest::ShellSurfaces { permit, reply } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| self.list_shell_surfaces(shell, state, &permit),
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::InspectShellSurface {
                    permit,
                    id,
                    generation,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| {
                            self.inspect_shell_surface(shell, state, &permit, &id, generation)
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::ShellSemanticAction {
                    permit,
                    request,
                    deadline,
                    expected_local_input_epoch,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| {
                            self.perform_shell_semantic_action(
                                shell,
                                state,
                                &permit,
                                request,
                                deadline,
                                expected_local_input_epoch,
                            )
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::WorkspaceAction {
                    permit,
                    prepared,
                    action,
                    reply,
                } => {
                    let result = self.perform_workspace_action(permit, *prepared, action);
                    let _ = reply.try_send(result);
                }
                OwnerRequest::Diagnostic {
                    permit,
                    prepared,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| {
                            self.perform_diagnostic_snapshot(shell, state, permit, *prepared)
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::DiagnosticAction {
                    permit,
                    action,
                    platform_refresh,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, _)| {
                            self.perform_diagnostic_action(shell, permit, action, platform_refresh)
                        },
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
                OwnerRequest::ReadApplicationScale {
                    permit,
                    prepared,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(_, state)| self.read_application_scale(&permit, prepared, state),
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::ApplicationScaleTransaction {
                    permit,
                    transaction,
                    prepared,
                    deadline,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| {
                            self.change_application_scale(
                                shell,
                                state,
                                &permit,
                                transaction,
                                *prepared,
                                deadline,
                            )
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::ReadAppearance {
                    permit,
                    prepared,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(_, state)| self.read_appearance(&permit, prepared, state),
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::AppearanceTransaction {
                    permit,
                    transaction,
                    prepared,
                    deadline,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| {
                            self.change_appearance(
                                shell,
                                state,
                                &permit,
                                transaction,
                                prepared,
                                deadline,
                            )
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::ReadFileIcons {
                    permit,
                    prepared,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(_, state)| self.read_file_icons(&permit, prepared, state),
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::FileIconsTransaction {
                    permit,
                    transaction,
                    prepared,
                    deadline,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| {
                            self.change_file_icons(
                                shell,
                                state,
                                &permit,
                                transaction,
                                prepared,
                                deadline,
                            )
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::ReadWallpaper {
                    permit,
                    prepared,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(_, state)| self.read_wallpaper(&permit, prepared, state),
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::WallpaperTransaction {
                    permit,
                    transaction,
                    prepared,
                    deadline,
                    expected_local_input_epoch,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| {
                            self.change_wallpaper(
                                shell,
                                state,
                                &permit,
                                transaction,
                                prepared,
                                deadline,
                                expected_local_input_epoch,
                            )
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::ReadTerminalPresentation {
                    permit,
                    prepared,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| {
                            self.read_terminal_presentation(&permit, prepared, shell, state)
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::TerminalPresentationTransaction {
                    permit,
                    transaction,
                    prepared,
                    deadline,
                    expected_local_input_epoch,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| {
                            self.change_terminal_presentation(
                                shell,
                                state,
                                &permit,
                                transaction,
                                *prepared,
                                deadline,
                                expected_local_input_epoch,
                            )
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::ReadTerminalLaunchPolicy {
                    permit,
                    prepared,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| {
                            self.read_terminal_launch_policy(&permit, prepared, shell, state)
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::TerminalLaunchPolicyTransaction {
                    permit,
                    transaction,
                    prepared,
                    deadline,
                    expected_local_input_epoch,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| {
                            self.change_terminal_launch_policy(
                                shell,
                                state,
                                &permit,
                                transaction,
                                *prepared,
                                deadline,
                                expected_local_input_epoch,
                            )
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::ReadCodexPreference {
                    permit,
                    prepared,
                    deadline,
                    reply,
                } => {
                    let result = match (shell.as_mut(), codex.as_mut()) {
                        (Some((shell, state)), Some((codex, feature_settings))) => self
                            .read_codex_preference(
                                shell,
                                state,
                                codex,
                                feature_settings,
                                &permit,
                                prepared,
                                deadline,
                            ),
                        _ => Err("Windows Codex owner is unavailable".into()),
                    };
                    let _ = reply.try_send(result);
                }
                OwnerRequest::CodexPreferenceTransaction {
                    permit,
                    transaction,
                    prepared,
                    deadline,
                    expected_local_input_epoch,
                    reply,
                } => {
                    let result = match (shell.as_mut(), codex.as_mut()) {
                        (Some((shell, state)), Some((codex, feature_settings))) => self
                            .change_codex_preference(
                                shell,
                                state,
                                codex,
                                feature_settings,
                                &permit,
                                transaction,
                                prepared,
                                deadline,
                                expected_local_input_epoch,
                            ),
                        _ => Err("Windows Codex owner is unavailable".into()),
                    };
                    let _ = reply.try_send(result);
                }
                OwnerRequest::ShellBehaviorTransaction {
                    permit,
                    transaction,
                    prepared,
                    deadline,
                    expected_local_input_epoch,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| {
                            self.change_shell_behavior(
                                shell,
                                state,
                                &permit,
                                transaction,
                                prepared,
                                deadline,
                                expected_local_input_epoch,
                            )
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::ReadIdlePreferences {
                    permit,
                    prepared,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(_, state)| self.read_idle_preferences(&permit, prepared, state),
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::IdlePreferencesTransaction {
                    permit,
                    transaction,
                    prepared,
                    deadline,
                    expected_local_input_epoch,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| {
                            self.change_idle_preferences(
                                shell,
                                state,
                                &permit,
                                transaction,
                                prepared,
                                deadline,
                                expected_local_input_epoch,
                            )
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::ReadKeyboardPreference {
                    permit,
                    prepared,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(_, state)| self.read_keyboard_preference(&permit, prepared, state),
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::KeyboardPreferenceTransaction {
                    permit,
                    transaction,
                    prepared,
                    deadline,
                    expected_local_input_epoch,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| {
                            self.change_keyboard_preference(
                                shell,
                                state,
                                &permit,
                                transaction,
                                prepared,
                                deadline,
                                expected_local_input_epoch,
                            )
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::LauncherFavoritesCatalog {
                    permit,
                    deadline,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(_, state)| self.launcher_favorites_catalog(&permit, state, deadline),
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::ReadLauncherFavorites {
                    permit,
                    prepared,
                    deadline,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(_, state)| {
                            self.read_launcher_favorites(&permit, prepared, state, deadline)
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::LauncherFavoritesTransaction {
                    permit,
                    transaction,
                    prepared,
                    deadline,
                    expected_local_input_epoch,
                    reply,
                } => {
                    let result = shell.as_mut().map_or_else(
                        || Err("Windows presentation owner is unavailable".into()),
                        |(shell, state)| {
                            self.change_launcher_favorites(
                                shell,
                                state,
                                &permit,
                                transaction,
                                prepared,
                                deadline,
                                expected_local_input_epoch,
                            )
                        },
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::Applications {
                    permit,
                    prepared,
                    reply,
                } => {
                    let result = self.application_inventory(permit, prepared.map(|value| *value));
                    let _ = reply.try_send(result);
                }
                OwnerRequest::LaunchApplication {
                    permit,
                    request,
                    prepared,
                    deadline,
                    cancelled,
                    reply,
                } => {
                    let result = if Instant::now() >= deadline
                        || cancelled.load(std::sync::atomic::Ordering::Acquire)
                    {
                        Err("Windows application launch timed out".into())
                    } else {
                        self.plan_application_launch(
                            &permit,
                            &request,
                            prepared.map(|value| *value),
                        )
                    };
                    let _ = reply.try_send(result);
                }
                OwnerRequest::CommitApplicationLaunch {
                    permit,
                    request,
                    plan,
                    staged,
                    deadline,
                    cancelled,
                    reply,
                } => {
                    let result = self.commit_application_launch(
                        permit, request, *plan, *staged, deadline, &cancelled,
                    );
                    let _ = reply.try_send(result);
                }
                OwnerRequest::BindApplicationPlacementRoot {
                    permit,
                    ticket,
                    root,
                    reply,
                } => {
                    let result = self.bind_application_placement_root(permit, &ticket, root);
                    let _ = reply.try_send(result);
                }
                OwnerRequest::AttemptApplicationPlacement {
                    permit,
                    ticket,
                    prepared,
                    reply,
                } => {
                    let result = self.attempt_application_placement(permit, &ticket, &prepared);
                    let _ = reply.try_send(result);
                }
                OwnerRequest::CancelApplicationPlacement { ticket } => {
                    self.pending_output_launches.remove(&ticket.id);
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

    fn read_application_scale(
        &mut self,
        permit: &DesktopPermit,
        prepared: crate::windows_remote_application_scale::PreparedRead,
        state: &crate::live_shell::LiveShell,
    ) -> Result<nickel_remote_control::application_scale::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        let protected =
            !self.desktop_unlocked || state.surface_visible(crate::winit_shell::SurfaceRole::Lock);
        permit.with_debug(protected, || {
            self.application_scale
                .observe(&prepared, self.start_time, Instant::now())
        })
    }

    fn change_application_scale(
        &mut self,
        shell: &WinitShell,
        state: &mut crate::live_shell::LiveShell,
        permit: &DesktopPermit,
        transaction: nickel_remote_control::application_scale::Transaction,
        prepared: crate::windows_remote_application_scale::PreparedChange,
        request_deadline: Instant,
    ) -> Result<nickel_remote_control::application_scale::TransactionOutcome, String> {
        let protected = !self.desktop_unlocked
            || state.surface_visible(crate::winit_shell::SurfaceRole::Lock)
            || shell
                .remote_shell_surface_observations(state)
                .iter()
                .any(|surface| surface.keyboard_focused && surface.protected);
        let expected_input_epoch = local_input_epoch();
        let input_busy = self.keyboard_hold.is_some()
            || self.pointer_hold.is_some()
            || state.pointer_interaction_active()
            || !crate::windows_remote_input::physical_input_idle();
        let mut committed = None;
        let authorization = permit.with_debug_input_deadline(protected, |boundary| {
            if Instant::now() >= request_deadline {
                return Err("application scale transaction expired before commit".into());
            }
            if input_busy {
                return Err("shared input is busy".into());
            }
            self.application_scale
                .validate(&prepared, &transaction, Instant::now())?;
            committed = Some(
                prepared.commit(boundary.deadline().min(request_deadline), || {
                    if local_input_epoch() != expected_input_epoch {
                        return Err("local input interrupted the settings transaction".into());
                    }
                    if !crate::windows_remote_input::physical_input_idle() {
                        return Err("shared input is busy".into());
                    }
                    permit.check_commit_boundary(boundary)
                })?,
            );
            Ok(())
        });
        let committed = match committed {
            Some(value) => value,
            None => {
                authorization?;
                return Err(
                    "application scale unavailable; read current state before retrying".into(),
                );
            }
        };
        // Once write-through replacement is accepted, a later revocation must
        // not turn the result into a denial that invites an unsafe retry.
        authorization?;
        let observed_at_us = self
            .start_time
            .elapsed()
            .as_micros()
            .min(u128::from(u64::MAX)) as u64;
        let snapshot = self
            .application_scale
            .observe_committed(&committed, observed_at_us)?;
        Ok(
            nickel_remote_control::application_scale::TransactionOutcome {
                snapshot,
                outcomes: committed.outcomes,
            },
        )
    }

    fn read_appearance(
        &mut self,
        permit: &DesktopPermit,
        prepared: crate::windows_remote_settings::PreparedAppearanceRead,
        state: &crate::live_shell::LiveShell,
    ) -> Result<nickel_remote_control::appearance::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.ensure_current()?;
        let protected =
            !self.desktop_unlocked || state.surface_visible(crate::winit_shell::SurfaceRole::Lock);
        permit.with_debug(protected, || {
            self.appearance.observe(
                &prepared,
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
            )
        })
    }

    fn change_appearance(
        &mut self,
        shell: &WinitShell,
        state: &mut crate::live_shell::LiveShell,
        permit: &DesktopPermit,
        transaction: nickel_remote_control::appearance::Transaction,
        prepared: crate::windows_remote_settings::PreparedAppearanceChange,
        request_deadline: Instant,
    ) -> Result<nickel_remote_control::appearance::Snapshot, String> {
        let protected = !self.desktop_unlocked
            || state.surface_visible(crate::winit_shell::SurfaceRole::Lock)
            || shell
                .remote_shell_surface_observations(state)
                .iter()
                .any(|surface| surface.keyboard_focused && surface.protected);
        let expected_input_epoch = local_input_epoch();
        let input_busy = self.keyboard_hold.is_some()
            || self.pointer_hold.is_some()
            || state.pointer_interaction_active()
            || !crate::windows_remote_input::physical_input_idle();
        let mut committed = None;
        let authorization = permit.with_debug_input_deadline(protected, |deadline| {
            if Instant::now() >= request_deadline {
                return Err("appearance transaction expired before commit".into());
            }
            if input_busy {
                return Err("shared input is busy".into());
            }
            self.appearance.validate(&prepared, &transaction)?;
            committed = Some(
                prepared.commit(deadline.deadline().min(request_deadline), || {
                    if local_input_epoch() != expected_input_epoch {
                        return Err("local input interrupted the settings transaction".into());
                    }
                    permit.check_commit_boundary(deadline)
                })?,
            );
            self.appearance.invalidate();
            Ok(())
        });
        let (requested, revision) = match committed {
            Some(value) => value,
            None => {
                authorization?;
                return Err("appearance unavailable; inspect current state before retrying".into());
            }
        };
        // A successful replacement must be reflected by the production model,
        // even if revocation raced immediately after the commit boundary.
        state.apply_shell_settings(requested.clone());
        authorization?;
        self.appearance.observe_committed(
            revision?,
            &requested,
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
        )
    }

    fn read_file_icons(
        &mut self,
        permit: &DesktopPermit,
        prepared: crate::windows_remote_settings::PreparedFileIconsRead,
        state: &crate::live_shell::LiveShell,
    ) -> Result<nickel_remote_control::file_icons::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.ensure_current()?;
        let protected =
            !self.desktop_unlocked || state.surface_visible(crate::winit_shell::SurfaceRole::Lock);
        permit.with_debug(protected, || {
            self.file_icons.observe(
                &prepared,
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
                false,
            )
        })
    }

    fn change_file_icons(
        &mut self,
        shell: &WinitShell,
        state: &mut crate::live_shell::LiveShell,
        permit: &DesktopPermit,
        transaction: nickel_remote_control::file_icons::Transaction,
        prepared: crate::windows_remote_settings::PreparedFileIconsChange,
        request_deadline: Instant,
    ) -> Result<nickel_remote_control::file_icons::Snapshot, String> {
        let protected = !self.desktop_unlocked
            || state.surface_visible(crate::winit_shell::SurfaceRole::Lock)
            || shell
                .remote_shell_surface_observations(state)
                .iter()
                .any(|surface| surface.keyboard_focused && surface.protected);
        let expected_input_epoch = local_input_epoch();
        let input_busy = self.keyboard_hold.is_some()
            || self.pointer_hold.is_some()
            || state.pointer_interaction_active()
            || !crate::windows_remote_input::physical_input_idle();
        let mut committed = None;
        let authorization = permit.with_debug_input_deadline(protected, |boundary| {
            if Instant::now() >= request_deadline {
                return Err("file icon transaction expired before commit".into());
            }
            if input_busy {
                return Err("shared input is busy".into());
            }
            self.file_icons.validate(&prepared, &transaction)?;
            committed = Some(
                prepared.commit(boundary.deadline().min(request_deadline), || {
                    if local_input_epoch() != expected_input_epoch {
                        return Err("local input interrupted the settings transaction".into());
                    }
                    permit.check_commit_boundary(boundary)
                })?,
            );
            self.file_icons.invalidate();
            Ok(())
        });
        let (requested, revision) = match committed {
            Some(value) => value,
            None => {
                authorization?;
                return Err(
                    "file icon settings unavailable; read current state before retrying".into(),
                );
            }
        };
        state.apply_file_icon_settings(requested);
        authorization?;
        let read = crate::windows_remote_settings::PreparedFileIconsRead::prepare()?;
        if read.revision != revision {
            return Err("file icon settings changed; read current state before retrying".into());
        }
        self.file_icons.observe(
            &read,
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
            true,
        )
    }

    fn read_wallpaper(
        &mut self,
        permit: &DesktopPermit,
        prepared: crate::windows_remote_settings::PreparedWallpaperRead,
        state: &crate::live_shell::LiveShell,
    ) -> Result<nickel_remote_control::wallpaper::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.ensure_current()?;
        let protected =
            !self.desktop_unlocked || state.surface_visible(crate::winit_shell::SurfaceRole::Lock);
        permit.with_debug(protected, || {
            self.wallpaper.observe(
                &prepared,
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
                false,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn change_wallpaper(
        &mut self,
        shell: &WinitShell,
        state: &mut crate::live_shell::LiveShell,
        permit: &DesktopPermit,
        transaction: nickel_remote_control::wallpaper::Transaction,
        prepared: crate::windows_remote_settings::PreparedWallpaperChange,
        request_deadline: Instant,
        expected_local_input_epoch: u64,
    ) -> Result<nickel_remote_control::wallpaper::Snapshot, String> {
        let protected = !self.desktop_unlocked
            || state.surface_visible(crate::winit_shell::SurfaceRole::Lock)
            || shell
                .remote_shell_surface_observations(state)
                .iter()
                .any(|surface| surface.keyboard_focused && surface.protected);
        let input_busy = self.keyboard_hold.is_some()
            || self.pointer_hold.is_some()
            || state.pointer_interaction_active();
        let mut committed = None;
        let authorization = permit.with_debug_input_deadline(protected, |boundary| {
            if Instant::now() >= request_deadline {
                return Err("wallpaper transaction expired before commit".into());
            }
            if input_busy
                || local_input_epoch() != expected_local_input_epoch
                || !crate::windows_remote_input::physical_input_idle()
            {
                return Err("shared input or physical local input is busy".into());
            }
            self.wallpaper.validate(&prepared, &transaction)?;
            committed = Some(
                prepared.commit(boundary.deadline().min(request_deadline), || {
                    if local_input_epoch() != expected_local_input_epoch
                        || !crate::windows_remote_input::physical_input_idle()
                    {
                        return Err("physical input interrupted the wallpaper transaction".into());
                    }
                    permit.check_commit_boundary(boundary)
                })?,
            );
            self.wallpaper.invalidate();
            Ok(())
        });
        let requested = match committed {
            Some(requested) => requested,
            None => {
                authorization?;
                return Err("wallpaper unavailable; read current state before retrying".into());
            }
        };
        state.apply_wallpaper_settings(requested.clone());
        authorization?;
        let read = crate::windows_remote_settings::PreparedWallpaperRead::prepare()?;
        if read.settings() != &requested {
            state.apply_wallpaper_settings(read.settings().clone());
            return Err("wallpaper changed; read current wallpaper before retrying".into());
        }
        let mut snapshot = self.wallpaper.observe(
            &read,
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
            true,
        )?;
        snapshot.selected_image_decoded = matches!(
            transaction.change,
            nickel_remote_control::wallpaper::Change::SelectApprovedImage { .. }
        );
        Ok(snapshot)
    }

    fn read_terminal_presentation(
        &mut self,
        permit: &DesktopPermit,
        prepared: crate::windows_remote_terminal_presentation::PreparedRead,
        shell: &WinitShell,
        state: &crate::live_shell::LiveShell,
    ) -> Result<nickel_remote_control::terminal_presentation::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.ensure_current(Instant::now())?;
        let protected = !self.desktop_unlocked
            || state.surface_visible(crate::winit_shell::SurfaceRole::Lock)
            || shell
                .remote_shell_surface_observations(state)
                .iter()
                .any(|surface| surface.keyboard_focused && surface.protected);
        permit.with_debug(protected, || {
            self.terminal_presentation.observe(
                &prepared,
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn change_terminal_presentation(
        &mut self,
        shell: &WinitShell,
        state: &mut crate::live_shell::LiveShell,
        permit: &DesktopPermit,
        transaction: nickel_remote_control::terminal_presentation::Transaction,
        prepared: crate::windows_remote_terminal_presentation::PreparedChange,
        request_deadline: Instant,
        expected_local_input_epoch: u64,
    ) -> Result<nickel_remote_control::terminal_presentation::Snapshot, String> {
        let protected = !self.desktop_unlocked
            || state.surface_visible(crate::winit_shell::SurfaceRole::Lock)
            || shell
                .remote_shell_surface_observations(state)
                .iter()
                .any(|surface| surface.keyboard_focused && surface.protected);
        let input_busy = self.keyboard_hold.is_some()
            || self.pointer_hold.is_some()
            || state.pointer_interaction_active()
            || !crate::windows_remote_input::physical_input_idle();
        let mut committed = None;
        let authorization = permit.with_debug_input_deadline(protected, |boundary| {
            if Instant::now() >= request_deadline {
                return Err("terminal presentation transaction expired before commit".into());
            }
            if input_busy {
                return Err("shared input is busy".into());
            }
            self.terminal_presentation
                .validate(&prepared, &transaction, Instant::now())?;
            committed = Some(
                prepared.commit(boundary.deadline().min(request_deadline), || {
                    if local_input_epoch() != expected_local_input_epoch {
                        return Err("local input interrupted the settings transaction".into());
                    }
                    if !crate::windows_remote_input::physical_input_idle() {
                        return Err("shared input is busy".into());
                    }
                    permit.check_commit_boundary(boundary)
                })?,
            );
            Ok(())
        });
        let committed = match committed {
            Some(committed) => committed,
            None => {
                authorization?;
                return Err(
                    "terminal presentation unavailable; read current state before retrying".into(),
                );
            }
        };
        // nickel-terminal loads this file when creating a new process. Record
        // the exact committed file revision even when revocation wins the
        // post-effect check, so the owner never reports stale runtime state.
        let snapshot = self.terminal_presentation.observe_committed(
            &committed,
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
        )?;
        authorization?;
        Ok(snapshot)
    }

    fn read_terminal_launch_policy(
        &mut self,
        permit: &DesktopPermit,
        prepared: crate::remote_terminal_launch_policy::PreparedRead,
        shell: &WinitShell,
        state: &crate::live_shell::LiveShell,
    ) -> Result<nickel_remote_control::terminal_launch_policy::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.ensure_current(Instant::now())?;
        let protected = !self.desktop_unlocked
            || state.surface_visible(crate::winit_shell::SurfaceRole::Lock)
            || shell
                .remote_shell_surface_observations(state)
                .iter()
                .any(|surface| surface.keyboard_focused && surface.protected);
        permit.with_debug(protected, || {
            self.terminal_launch_policy.observe(
                &prepared,
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn change_terminal_launch_policy(
        &mut self,
        shell: &WinitShell,
        state: &mut crate::live_shell::LiveShell,
        permit: &DesktopPermit,
        transaction: nickel_remote_control::terminal_launch_policy::Transaction,
        prepared: crate::remote_terminal_launch_policy::PreparedChange,
        request_deadline: Instant,
        expected_local_input_epoch: u64,
    ) -> Result<nickel_remote_control::terminal_launch_policy::TransactionOutcome, String> {
        let protected = !self.desktop_unlocked
            || state.surface_visible(crate::winit_shell::SurfaceRole::Lock)
            || shell
                .remote_shell_surface_observations(state)
                .iter()
                .any(|surface| surface.keyboard_focused && surface.protected);
        let input_busy = self.keyboard_hold.is_some()
            || self.pointer_hold.is_some()
            || state.pointer_interaction_active()
            || !crate::windows_remote_input::physical_input_idle();
        let mut committed = None;
        let authorization = permit.with_debug_input_deadline(protected, |boundary| {
            if Instant::now() >= request_deadline {
                return Err("terminal launch-policy transaction expired before commit".into());
            }
            if input_busy {
                return Err("shared input is busy".into());
            }
            self.terminal_launch_policy
                .validate(&prepared, &transaction, Instant::now())?;
            committed = Some(
                prepared.commit(boundary.deadline().min(request_deadline), || {
                    if local_input_epoch() != expected_local_input_epoch {
                        return Err("local input interrupted the settings transaction".into());
                    }
                    if !crate::windows_remote_input::physical_input_idle() {
                        return Err("shared input is busy".into());
                    }
                    permit.check_commit_boundary(boundary)
                })?,
            );
            Ok(())
        });
        let committed = match committed {
            Some(committed) => committed,
            None => {
                authorization?;
                return Err(
                    "terminal launch policy unavailable; read current state before retrying".into(),
                );
            }
        };
        let snapshot = self.terminal_launch_policy.observe_committed(
            &committed,
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
        )?;
        authorization?;
        Ok(crate::remote_terminal_launch_policy::outcome(snapshot))
    }

    #[allow(clippy::too_many_arguments)]
    fn read_codex_preference(
        &mut self,
        shell: &WinitShell,
        state: &crate::live_shell::LiveShell,
        codex: &crate::CodexSurfaces,
        feature_settings: &nickel_core::optional_features::OptionalFeatureSettings,
        permit: &DesktopPermit,
        prepared: crate::windows_remote_codex::PreparedRead,
        request_deadline: Instant,
    ) -> Result<nickel_remote_control::codex_preference::Snapshot, String> {
        if Instant::now() >= request_deadline {
            return Err("Windows Codex preference observation expired".into());
        }
        prepared.ensure_current()?;
        let protected = !self.desktop_unlocked
            || state.surface_visible(crate::winit_shell::SurfaceRole::Lock)
            || shell
                .remote_shell_surface_observations(state)
                .iter()
                .any(|surface| surface.keyboard_focused && surface.protected);
        permit.with_debug(protected, || {
            if Instant::now() >= request_deadline {
                return Err("Windows Codex preference observation expired".into());
            }
            prepared.ensure_current()?;
            Ok(prepared.snapshot(
                codex.remote_runtime_state(feature_settings.codex_generation),
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
            ))
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn change_codex_preference(
        &mut self,
        shell: &mut WinitShell,
        state: &mut crate::live_shell::LiveShell,
        codex: &mut crate::CodexSurfaces,
        feature_settings: &mut nickel_core::optional_features::OptionalFeatureSettings,
        permit: &DesktopPermit,
        transaction: nickel_remote_control::codex_preference::Transaction,
        prepared: crate::windows_remote_codex::PreparedChange,
        request_deadline: Instant,
        expected_local_input_epoch: u64,
    ) -> Result<nickel_remote_control::codex_preference::Snapshot, String> {
        use nickel_core::optional_features::{
            CodexAvailabilityProjection, FeatureHealth, FeatureSupport,
        };

        let protected = !self.desktop_unlocked
            || state.surface_visible(crate::winit_shell::SurfaceRole::Lock)
            || shell
                .remote_shell_surface_observations(state)
                .iter()
                .any(|surface| surface.keyboard_focused && surface.protected);
        let input_busy = self.keyboard_hold.is_some()
            || self.pointer_hold.is_some()
            || state.pointer_interaction_active()
            || !crate::windows_remote_input::physical_input_idle();
        let mut committed = None;
        let authorization = permit.with_debug_input_deadline(protected, |boundary| {
            if Instant::now() >= request_deadline {
                return Err("Codex preference transaction expired before commit".into());
            }
            if input_busy {
                return Err("shared input is busy".into());
            }
            if !prepared.requested_enabled() && codex.active_chat_count() > 0 {
                return Err("close active Codex chat windows before disabling Codex".into());
            }
            prepared.ensure_current(&transaction)?;
            committed = Some(
                prepared.commit(boundary.deadline().min(request_deadline), || {
                    if local_input_epoch() != expected_local_input_epoch
                        || !crate::windows_remote_input::physical_input_idle()
                    {
                        return Err(
                            "local input interrupted the Codex preference transaction".into()
                        );
                    }
                    permit.check_commit_boundary(boundary)
                })?,
            );
            Ok(())
        });
        let mut settings = match committed {
            Some(settings) => settings,
            None => {
                authorization?;
                return Err(
                    "Codex preference unavailable; read current state before retrying".into(),
                );
            }
        };

        // The write is now authoritative. Reconcile it with the actual winit
        // Codex owner before propagating any post-commit failure.
        settings.codex_enabled = settings.effective_codex_enabled();
        *feature_settings = settings.clone();
        codex.apply_settings(shell, &settings);
        let runtime_error = if settings.codex_enabled {
            match codex.ensure_project_menu(shell) {
                Ok(()) => {
                    state.apply_codex_projection(CodexAvailabilityProjection::new(
                        FeatureSupport::Supported,
                        codex.installation(),
                        true,
                        FeatureHealth::Loading,
                        settings.codex_generation,
                        Some("Checking the selected Codex backend…".into()),
                    ));
                    None
                }
                Err(_) => {
                    state.apply_codex_projection(CodexAvailabilityProjection::new(
                        FeatureSupport::Supported,
                        codex.installation(),
                        true,
                        FeatureHealth::Failed,
                        settings.codex_generation,
                        Some("Codex integration could not be enabled".into()),
                    ));
                    Some("Codex preference committed but its runtime could not start".to_owned())
                }
            }
        } else {
            state.apply_codex_projection(CodexAvailabilityProjection::new(
                FeatureSupport::Supported,
                codex.installation(),
                false,
                FeatureHealth::Unknown,
                settings.codex_generation,
                Some("Codex integration is disabled".into()),
            ));
            None
        };
        crate::sync_visibility(shell, state);
        let render_result = crate::render_all(shell, state);
        authorization?;
        if let Some(error) = runtime_error {
            return Err(error);
        }
        render_result?;
        let read = crate::windows_remote_codex::PreparedRead::prepare()?;
        read.ensure_current()?;
        Ok(read.snapshot(
            codex.remote_runtime_state(feature_settings.codex_generation),
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn change_shell_behavior(
        &mut self,
        shell: &mut WinitShell,
        state: &mut crate::live_shell::LiveShell,
        permit: &DesktopPermit,
        transaction: nickel_session_protocol::ShellBehaviorTransaction,
        prepared: crate::windows_remote_settings::PreparedShellBehaviorChange,
        request_deadline: Instant,
        expected_local_input_epoch: u64,
    ) -> Result<nickel_remote_control::diagnostics::ShellBehaviorDiagnostic, String> {
        let protected = !self.desktop_unlocked
            || state.surface_visible(crate::winit_shell::SurfaceRole::Lock)
            || shell
                .remote_shell_surface_observations(state)
                .iter()
                .any(|surface| surface.keyboard_focused && surface.protected);
        let input_busy = self.keyboard_hold.is_some()
            || self.pointer_hold.is_some()
            || state.pointer_interaction_active()
            || !crate::windows_remote_input::physical_input_idle();
        let mut committed = None;
        let authorization = permit.with_debug_input_deadline(protected, |boundary| {
            if Instant::now() >= request_deadline {
                return Err("shell behavior transaction expired before commit".into());
            }
            if input_busy {
                return Err("shared input is busy".into());
            }
            if transaction.topology_generation != self.resources.output_topology_generation() {
                return Err("shell behavior transaction has a stale output topology".into());
            }
            prepared.ensure_current(&transaction)?;
            committed = Some(
                prepared.commit(boundary.deadline().min(request_deadline), || {
                    if local_input_epoch() != expected_local_input_epoch
                        || !crate::windows_remote_input::physical_input_idle()
                    {
                        return Err("local input interrupted the shell behavior transaction".into());
                    }
                    permit.check_commit_boundary(boundary)
                })?,
            );
            Ok(())
        });
        let requested = match committed {
            Some(settings) => settings,
            None => {
                authorization?;
                return Err(
                    "shell behavior unavailable; read current diagnostics before retrying".into(),
                );
            }
        };

        // Persistence has committed, so the production owners must observe it
        // before any later presentation or authority error is returned.
        state.apply_shell_settings(requested.clone());
        let shell_result = shell.set_bar_on_all_displays(requested.bar_on_all_displays);
        crate::sync_visibility(shell, state);
        let render_result = shell_result.and_then(|_| crate::render_all(shell, state));
        authorization?;
        render_result?;
        self.observation_generation = self
            .observation_generation
            .checked_add(1)
            .ok_or("Windows observation generations exhausted")?;
        Ok(shell_behavior_diagnostic(
            self.observation_generation,
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
            self.resources.output_topology_generation(),
            shell.bar_on_all_displays(),
            state.remote_shell_behavior_state(),
        ))
    }

    fn launcher_favorites_catalog(
        &self,
        permit: &DesktopPermit,
        state: &crate::live_shell::LiveShell,
        request_deadline: Instant,
    ) -> Result<crate::windows_remote_launcher_favorites::Catalog, String> {
        if Instant::now() >= request_deadline {
            return Err("Windows launcher catalog observation expired".into());
        }
        let protected =
            !self.desktop_unlocked || state.surface_visible(crate::winit_shell::SurfaceRole::Lock);
        permit.with_debug(protected, || {
            if state.launcher_preferences_busy() {
                return Err("local launcher preference action is pending".into());
            }
            state.launcher_favorite_catalog()
        })
    }

    fn read_launcher_favorites(
        &mut self,
        permit: &DesktopPermit,
        prepared: crate::windows_remote_launcher_favorites::PreparedRead,
        state: &crate::live_shell::LiveShell,
        request_deadline: Instant,
    ) -> Result<nickel_remote_control::launcher_favorites::Snapshot, String> {
        if Instant::now() >= request_deadline {
            return Err("Windows launcher favorites observation expired".into());
        }
        let protected =
            !self.desktop_unlocked || state.surface_visible(crate::winit_shell::SurfaceRole::Lock);
        permit.with_debug(protected, || {
            if state.launcher_preferences_busy() {
                return Err("local launcher preference action is pending".into());
            }
            let catalog = state.launcher_favorite_catalog()?;
            prepared.ensure_current(&catalog)?;
            self.launcher_favorites.observe(
                &prepared,
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
                state.launcher_favorites_match(prepared.preferences()),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn change_launcher_favorites(
        &mut self,
        shell: &WinitShell,
        state: &mut crate::live_shell::LiveShell,
        permit: &DesktopPermit,
        transaction: nickel_remote_control::launcher_favorites::Transaction,
        prepared: crate::windows_remote_launcher_favorites::PreparedChange,
        request_deadline: Instant,
        expected_local_input_epoch: u64,
    ) -> Result<nickel_remote_control::launcher_favorites::Snapshot, String> {
        let protected = !self.desktop_unlocked
            || state.surface_visible(crate::winit_shell::SurfaceRole::Lock)
            || shell
                .remote_shell_surface_observations(state)
                .iter()
                .any(|surface| surface.keyboard_focused && surface.protected);
        let input_busy = self.keyboard_hold.is_some()
            || self.pointer_hold.is_some()
            || state.pointer_interaction_active()
            || !crate::windows_remote_input::physical_input_idle();
        let mut committed = None;
        let authorization = permit.with_debug_input_deadline(protected, |boundary| {
            if Instant::now() >= request_deadline {
                return Err("launcher favorites transaction expired before commit".into());
            }
            if input_busy || state.launcher_preferences_busy() {
                return Err("shared input or local launcher preferences are busy".into());
            }
            let catalog = state.launcher_favorite_catalog()?;
            prepared.ensure_current(&catalog)?;
            self.launcher_favorites.validate(&prepared, &transaction)?;
            committed = Some(
                prepared.commit(boundary.deadline().min(request_deadline), || {
                    if local_input_epoch() != expected_local_input_epoch
                        || !crate::windows_remote_input::physical_input_idle()
                    {
                        return Err("local input interrupted the favorites transaction".into());
                    }
                    permit.check_commit_boundary(boundary)
                })?,
            );
            self.launcher_favorites.invalidate();
            Ok(())
        });
        let committed = match committed {
            Some(value) => value,
            None => {
                authorization?;
                return Err(
                    "launcher favorites unavailable; read current state before retrying".into(),
                );
            }
        };
        // Once the replacement succeeds, reconcile the accepted state even if
        // authority changes before a response can be returned.
        state.apply_committed_launcher_preferences(committed.preferences().clone())?;
        authorization?;
        let observation = committed.into_observation()?;
        let runtime_applied = state.launcher_favorites_match(observation.preferences());
        self.launcher_favorites.observe(
            &observation,
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
            runtime_applied,
        )
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
                    topology_generation: self.resources.output_topology_generation(),
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

    fn prepare_native_accessibility(
        &mut self,
        permit: &DesktopPermit,
        prepared: &mut crate::platform::remote_observation::Prepared,
        id: &str,
        generation: u64,
        application_wide: bool,
    ) -> Result<crate::windows_external_accessibility::Proof, String> {
        self.reconcile_prepared_resources(permit, prepared)?;
        let scope = permit.resource_scope()?;
        let (window, evidence) = self
            .resources
            .window_resource(&scope, id, generation)
            .ok_or("Windows resource is unavailable or protected")?;
        permit.with_resource(&evidence, || Ok(()))?;
        let application = window.application.clone();
        if application_wide {
            use nickel_remote_control::leases::ResourceScope;
            let application = application
                .as_ref()
                .ok_or("Windows application identity is unverified")?;
            match &scope {
                ResourceScope::FullSession => {}
                ResourceScope::Application(identity) if identity == application => {}
                _ => return Err("lease does not authorize application-wide observation".into()),
            }
        }
        let anchor = crate::windows_external_accessibility::RootProof {
            id: id.to_owned(),
            generation,
            native: window.native,
            pid: window.pid,
            created: window.created,
            thread: window.thread,
            bounds: window.bounds,
        };
        let roots = if application_wide {
            let candidates: Vec<_> = self
                .resources
                .windows(&scope)
                .filter(|(summary, _)| {
                    summary.verified_application.as_deref() == application.as_deref()
                })
                .collect();
            if candidates.len() > crate::windows_external_accessibility::MAX_WINDOWS {
                return Err("Windows application window inventory exceeds its bound".into());
            }
            let mut roots = Vec::with_capacity(candidates.len());
            for (summary, evidence) in candidates {
                permit.with_resource(&evidence, || Ok(()))?;
                let candidate = self
                    .resources
                    .window(&summary.id, summary.generation)
                    .ok_or("Windows application window changed")?;
                roots.push(crate::windows_external_accessibility::RootProof {
                    id: summary.id,
                    generation: summary.generation,
                    native: candidate.native,
                    pid: candidate.pid,
                    created: candidate.created,
                    thread: candidate.thread,
                    bounds: candidate.bounds,
                });
            }
            roots
        } else {
            vec![anchor.clone()]
        };
        if !roots.contains(&anchor) {
            return Err("Windows application anchor changed".into());
        }
        Ok(crate::windows_external_accessibility::Proof {
            id: id.to_owned(),
            generation,
            native: window.native,
            pid: window.pid,
            created: window.created,
            thread: window.thread,
            bounds: window.bounds,
            application,
            roots,
            application_wide,
        })
    }

    fn finish_native_accessibility(
        &mut self,
        permit: &DesktopPermit,
        prepared: &mut crate::platform::remote_observation::Prepared,
        proof: &crate::windows_external_accessibility::Proof,
        observation: &mut crate::windows_external_accessibility::Observation,
    ) -> Result<nickel_remote_control::native_semantics::NativeSemanticSnapshot, String> {
        self.reconcile_prepared_resources(permit, prepared)?;
        let scope = permit.resource_scope()?;
        let (window, evidence) = self
            .resources
            .window_resource(&scope, &proof.id, proof.generation)
            .ok_or("Windows resource is unavailable or protected")?;
        if (
            window.native,
            window.pid,
            window.created,
            window.thread,
            window.bounds,
        ) != (
            proof.native,
            proof.pid,
            proof.created,
            proof.thread,
            proof.bounds,
        ) {
            return Err("Windows UI Automation identity changed".into());
        }
        if window.application != proof.application {
            return Err("Windows UI Automation application identity changed".into());
        }
        if proof.application_wide {
            use nickel_remote_control::leases::ResourceScope;
            let application = proof
                .application
                .as_ref()
                .ok_or("Windows UI Automation application identity is unverified")?;
            match &scope {
                ResourceScope::FullSession => {}
                ResourceScope::Application(identity) if identity == application => {}
                _ => return Err("lease no longer authorizes application observation".into()),
            }
            let current: Vec<_> = self
                .resources
                .windows(&scope)
                .filter(|(summary, _)| {
                    summary.verified_application.as_deref() == proof.application.as_deref()
                })
                .map(|(summary, _)| {
                    let window = self.resources.window(&summary.id, summary.generation)?;
                    Some(crate::windows_external_accessibility::RootProof {
                        id: summary.id,
                        generation: summary.generation,
                        native: window.native,
                        pid: window.pid,
                        created: window.created,
                        thread: window.thread,
                        bounds: window.bounds,
                    })
                })
                .collect::<Option<Vec<_>>>()
                .ok_or("Windows application window changed")?;
            if !crate::windows_external_accessibility::same_roots(&proof.roots, &current) {
                return Err("Windows application window set changed".into());
            }
        }
        self.observation_generation = self
            .observation_generation
            .checked_add(1)
            .ok_or("Windows observation generations exhausted")?;
        let validated = Instant::now();
        let to_us = |time: Instant| {
            time.saturating_duration_since(self.start_time)
                .as_micros()
                .min(u64::MAX as u128) as u64
        };
        observation.snapshot.observation_generation = self.observation_generation;
        observation.snapshot.observation_started_at_us = to_us(observation.started);
        observation.snapshot.observed_at_us = to_us(observation.completed);
        observation.snapshot.owner_validated_at_us = to_us(validated);
        let result = permit.with_resource(&evidence, || Ok(observation.snapshot.clone()))?;
        if !observation.actions.is_empty() {
            self.native_action_observations.push_back(
                crate::windows_external_accessibility::ActionObservation {
                    observation_generation: result.observation_generation,
                    proof: proof.clone(),
                    actions: observation.actions.clone(),
                },
            );
            while self.native_action_observations.len()
                > crate::windows_external_accessibility::MAX_ACTION_OBSERVATIONS
            {
                self.native_action_observations.pop_front();
            }
        }
        if let Some(operation_id) = permit.operation_id() {
            let diagnostic = nickel_remote_control::diagnostics::ExternalAccessibilityDiagnostic {
                operation_id,
                scope: result.scope,
                observation_started_at_us: result.observation_started_at_us,
                observed_at_us: result.observed_at_us,
                owner_validated_at_us: result.owner_validated_at_us,
                nodes: result.nodes.len().min(u32::MAX as usize) as u32,
                truncated: result.truncated,
                stale: false,
            };
            self.desktop_events.record(
                nickel_remote_control::desktop_events::DesktopEventKind::ExternalAccessibilityCompleted {
                    operation_id,
                    scope: diagnostic.scope,
                    nodes: diagnostic.nodes,
                    truncated: diagnostic.truncated,
                },
                diagnostic.owner_validated_at_us,
            );
            self.external_accessibility = Some(diagnostic);
        }
        Ok(result)
    }

    fn prepare_native_accessibility_action(
        &mut self,
        permit: &DesktopPermit,
        prepared: &mut crate::platform::remote_observation::Prepared,
        request: &nickel_remote_control::native_semantics::NativeSemanticActionRequest,
    ) -> Result<crate::windows_external_accessibility::ActionPlan, String> {
        request.validate().map_err(str::to_owned)?;
        self.reconcile_prepared_resources(permit, prepared)?;
        let plan = self
            .native_action_observations
            .iter()
            .find_map(|observation| observation.plan(request))
            .ok_or("native semantic action was not advertised by the bounded observation")?;
        self.validate_native_accessibility_action(permit, &plan)?;
        Ok(plan)
    }

    fn commit_native_accessibility_action(
        &mut self,
        permit: &DesktopPermit,
        prepared: &mut crate::platform::remote_observation::Prepared,
        plan: &crate::windows_external_accessibility::ActionPlan,
        deadline: Instant,
        expected_local_input_epoch: u64,
        dispatch: &std::sync::atomic::AtomicU8,
    ) -> Result<crate::windows_external_accessibility::ActionAuthorization, String> {
        self.reconcile_prepared_resources(permit, prepared)?;
        self.validate_native_accessibility_action(permit, plan)?;
        let scope = permit.resource_scope()?;
        let (_, evidence) = self
            .resources
            .window_resource(&scope, &plan.proof.id, plan.proof.generation)
            .ok_or("Windows resource is unavailable or protected")?;
        match permit.begin_input(&evidence, || {
            crate::windows_external_accessibility::commit_action_dispatch(
                dispatch,
                expected_local_input_epoch,
                local_input_epoch(),
                Instant::now(),
                deadline,
            )
        }) {
            Ok(input) => Ok(crate::windows_external_accessibility::ActionAuthorization {
                input: Some(input),
            }),
            Err(_)
                if dispatch.load(std::sync::atomic::Ordering::Acquire)
                    == crate::windows_external_accessibility::ACTION_ATTEMPTED =>
            {
                Ok(crate::windows_external_accessibility::ActionAuthorization { input: None })
            }
            Err(error) => Err(error),
        }
    }

    fn validate_native_accessibility_action(
        &self,
        permit: &DesktopPermit,
        plan: &crate::windows_external_accessibility::ActionPlan,
    ) -> Result<(), String> {
        let retained = self.native_action_observations.iter().any(|observation| {
            observation.observation_generation == plan.observation_generation
                && observation.proof.id == plan.proof.id
                && observation.proof.generation == plan.proof.generation
                && observation.proof.application == plan.proof.application
                && observation.proof.application_wide == plan.proof.application_wide
                && crate::windows_external_accessibility::same_roots(
                    &observation.proof.roots,
                    &plan.proof.roots,
                )
                && observation.actions.contains(&plan.target)
        });
        if !retained || !plan.proof.roots.contains(&plan.target.root) {
            return Err("native semantic action observation has retired".into());
        }
        let scope = permit.resource_scope()?;
        let (window, evidence) = self
            .resources
            .window_resource(&scope, &plan.proof.id, plan.proof.generation)
            .ok_or("Windows resource is unavailable or protected")?;
        if (
            window.native,
            window.pid,
            window.created,
            window.thread,
            window.bounds,
            &window.application,
        ) != (
            plan.proof.native,
            plan.proof.pid,
            plan.proof.created,
            plan.proof.thread,
            plan.proof.bounds,
            &plan.proof.application,
        ) {
            return Err("Windows UI Automation identity changed".into());
        }
        if plan.proof.application_wide {
            use nickel_remote_control::leases::ResourceScope;
            let application = plan
                .proof
                .application
                .as_ref()
                .ok_or("Windows UI Automation application identity is unverified")?;
            match &scope {
                ResourceScope::FullSession => {}
                ResourceScope::Application(identity) if identity == application => {}
                _ => return Err("lease no longer authorizes application action".into()),
            }
            let current: Vec<_> = self
                .resources
                .windows(&scope)
                .filter(|(summary, _)| {
                    summary.verified_application.as_deref() == plan.proof.application.as_deref()
                })
                .map(|(summary, evidence)| {
                    permit.with_resource(&evidence, || Ok(()))?;
                    let window = self
                        .resources
                        .window(&summary.id, summary.generation)
                        .ok_or("Windows application window changed")?;
                    Ok(crate::windows_external_accessibility::RootProof {
                        id: summary.id,
                        generation: summary.generation,
                        native: window.native,
                        pid: window.pid,
                        created: window.created,
                        thread: window.thread,
                        bounds: window.bounds,
                    })
                })
                .collect::<Result<_, String>>()?;
            if !crate::windows_external_accessibility::same_roots(&plan.proof.roots, &current) {
                return Err("Windows application window set changed".into());
            }
        } else if plan.proof.roots.as_slice() != [plan.target.root.clone()] {
            return Err("Windows UI Automation window root changed".into());
        }
        permit.with_resource(&evidence, || Ok(()))
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
        self.prune_output_launch_placements();
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
        // A pending launch never blocks established unrelated resources. Only
        // post-commit window incarnations are withheld until the exact launch
        // root is attributed, placed and freshly confirmed on its output.
        retain_launch_safe_windows(prepared, &self.pending_output_launches);
        let control = self.remote_control.control();
        self.resources
            .reconcile(prepared.windows.clone(), prepared.outputs.clone(), |id| {
                revoke_native_resource(&control, id)
            })?;
        prepared.revalidate()
    }

    fn prune_output_launch_placements(&mut self) {
        use nickel_remote_control::leases::ResourceScope;

        let now = Instant::now();
        self.pending_output_launches.retain(|_, pending| {
            self.desktop_unlocked
                && now < pending.deadline
                && (pending.root.is_some() || now < pending.root_deadline)
                && pending.permit.continued_observation().is_ok()
                && pending
                    .permit
                    .resource_scope()
                    .is_ok_and(|scope| scope == ResourceScope::Output(pending.output.clone()))
        });
    }

    fn bind_application_placement_root(
        &mut self,
        permit: DesktopPermit,
        ticket: &OutputLaunchPlacementTicket,
        root: Arc<nickel_platform::process_identity::WindowsProcessIdentity>,
    ) -> Result<(), String> {
        use nickel_remote_control::leases::{ResourceEvidence, ResourceScope};

        self.prune_output_launch_placements();
        let pending = self
            .pending_output_launches
            .get_mut(&ticket.id)
            .ok_or("Windows output launch placement is no longer authorized")?;
        if ticket.output != pending.output
            || ticket.deadline != pending.deadline
            || !pending.permit.same_lease_as(&permit)
            || permit.resource_scope()? != ResourceScope::Output(pending.output.clone())
        {
            return Err("Windows launch root does not match its placement authority".into());
        }
        let evidence = ResourceEvidence {
            surface: None,
            window: None,
            verified_application: None,
            output: Some(&pending.output),
            authorized_surface_ancestors: &[],
            protected: !self.desktop_unlocked,
        };
        permit.with_resource(&evidence, || Ok(()))?;
        if !root.is_live() {
            return Err("Windows launch root exited before placement".into());
        }
        pending.root = Some(root);
        Ok(())
    }

    fn attempt_application_placement(
        &mut self,
        permit: DesktopPermit,
        ticket: &OutputLaunchPlacementTicket,
        prepared: &crate::platform::remote_observation::Prepared,
    ) -> Result<bool, String> {
        use crate::platform::remote_observation::LaunchPlacementResult;
        use nickel_remote_control::leases::{ResourceEvidence, ResourceScope};

        self.prune_output_launch_placements();
        let Some(pending) = self.pending_output_launches.get(&ticket.id) else {
            return Err("Windows output launch placement is no longer authorized".into());
        };
        let root = pending
            .root
            .clone()
            .ok_or("Windows launch root is not yet attributed")?;
        if ticket.output != pending.output
            || ticket.deadline != pending.deadline
            || permit.resource_scope()? != ResourceScope::Output(pending.output.clone())
            || prepared
                .outputs
                .iter()
                .filter(|value| *value == &pending.native_output)
                .count()
                != 1
        {
            self.pending_output_launches.remove(&ticket.id);
            return Err("Windows launch output changed before placement".into());
        };
        let evidence = ResourceEvidence {
            surface: None,
            window: None,
            verified_application: None,
            output: Some(&pending.output),
            authorized_surface_ancestors: &[],
            protected: !self.desktop_unlocked,
        };
        let result = permit.with_input(&evidence, || {
            crate::platform::remote_observation::place_first_launch_window(
                prepared,
                &root,
                &pending.native_output,
            )
        });
        match result {
            Ok(LaunchPlacementResult::Confirmed) => {
                self.pending_output_launches.remove(&ticket.id);
                Ok(true)
            }
            Ok(LaunchPlacementResult::Waiting | LaunchPlacementResult::Requested) => Ok(false),
            // Native ambiguity or a transient adapter failure cannot publish
            // the launched window. Retain quarantine until expiry/revocation.
            Err(_) => Ok(false),
        }
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

    fn list_shell_surfaces(
        &self,
        shell: &WinitShell,
        state: &crate::live_shell::LiveShell,
        permit: &DesktopPermit,
    ) -> Result<Vec<nickel_remote_control::diagnostics::ShellSurfaceDiagnostic>, String> {
        use nickel_remote_control::leases::{ResourceEvidence, ResourceId};
        let scope = permit.resource_scope()?;
        let protected =
            !self.desktop_unlocked || state.surface_visible(crate::winit_shell::SurfaceRole::Lock);
        let (surfaces, truncated) = crate::windows_shell_diagnostics::project(
            protected,
            shell.remote_shell_surface_observations(state),
        );
        if truncated {
            return Err("Windows shell surface ancestry exceeds its bound".into());
        }
        let authority =
            crate::remote_surface_authority::SurfaceAuthority::from_shell_surfaces(&surfaces)?;
        let mut result = Vec::new();
        for surface in surfaces {
            let identity = ResourceId {
                id: surface.id.clone(),
                generation: surface.generation,
            };
            let output = surface.output.as_ref().and_then(|name| {
                self.resources
                    .output_generation(name)
                    .map(|generation| ResourceId {
                        id: name.clone(),
                        generation,
                    })
            });
            let ancestors = authority.ancestors(&identity);
            let evidence = ResourceEvidence {
                surface: Some(&identity),
                window: None,
                verified_application: None,
                output: output.as_ref(),
                authorized_surface_ancestors: &ancestors,
                protected: output.is_none(),
            };
            if self.resources.shell_surface_authorized(
                &scope,
                &identity,
                surface.output.as_deref(),
                &ancestors,
            ) {
                result.push(permit.with_resource(&evidence, || Ok(surface))?);
            }
        }
        permit.check_live()?;
        Ok(result)
    }

    fn current_shell_surface(
        &self,
        shell: &WinitShell,
        state: &crate::live_shell::LiveShell,
        id: &str,
        generation: u64,
    ) -> Result<crate::windows_shell_diagnostics::SurfaceObservation, String> {
        let protected =
            !self.desktop_unlocked || state.surface_visible(crate::winit_shell::SurfaceRole::Lock);
        let observation = shell
            .remote_shell_surface_observations(state)
            .into_iter()
            .find(|surface| surface.generation == generation)
            .ok_or("shell surface has retired")?;
        let projected = crate::windows_shell_diagnostics::project(protected, [observation.clone()]);
        projected
            .0
            .first()
            .filter(|surface| surface.id == id && surface.generation == generation)
            .map(|_| observation)
            .ok_or_else(|| "shell surface is unavailable or protected".into())
    }

    fn shell_surface_ancestors(
        &self,
        shell: &WinitShell,
        state: &crate::live_shell::LiveShell,
        identity: &nickel_remote_control::leases::ResourceId,
    ) -> Result<Vec<nickel_remote_control::leases::ResourceId>, String> {
        let protected =
            !self.desktop_unlocked || state.surface_visible(crate::winit_shell::SurfaceRole::Lock);
        let (surfaces, truncated) = crate::windows_shell_diagnostics::project(
            protected,
            shell.remote_shell_surface_observations(state),
        );
        if truncated {
            return Err("Windows shell surface ancestry exceeds its bound".into());
        }
        Ok(
            crate::remote_surface_authority::SurfaceAuthority::from_shell_surfaces(&surfaces)?
                .ancestors(identity),
        )
    }

    fn inspect_shell_surface(
        &self,
        shell: &WinitShell,
        state: &crate::live_shell::LiveShell,
        permit: &DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<nickel_remote_control::semantics::SurfaceSemanticSnapshot, String> {
        use nickel_remote_control::leases::{ResourceEvidence, ResourceId};
        let observation = self.current_shell_surface(shell, state, id, generation)?;
        let identity = ResourceId {
            id: id.into(),
            generation,
        };
        let output_name = observation
            .output
            .as_deref()
            .ok_or("shell output is unavailable")?;
        let output = ResourceId {
            id: output_name.to_owned(),
            generation: self
                .resources
                .output_generation(output_name)
                .ok_or("shell output has retired")?,
        };
        let ancestors = self.shell_surface_ancestors(shell, state, &identity)?;
        let evidence = ResourceEvidence {
            surface: Some(&identity),
            window: None,
            verified_application: None,
            output: Some(&output),
            authorized_surface_ancestors: &ancestors,
            protected: false,
        };
        permit.with_resource(&evidence, || {
            let (tree_generation, nodes) =
                state.bounded_shell_semantics(observation.role, observation.output.as_deref())?;
            Ok(nickel_remote_control::semantics::SurfaceSemanticSnapshot {
                surface: id.to_owned(),
                surface_generation: generation,
                tree_generation,
                observed_at_us: self
                    .start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
                nodes: windows_semantic_projection(observation.role, nodes)?,
            })
        })
    }

    fn perform_shell_semantic_action(
        &mut self,
        shell: &WinitShell,
        state: &mut crate::live_shell::LiveShell,
        permit: &DesktopPermit,
        request: nickel_remote_control::semantics::SurfaceSemanticActionRequest,
        deadline: Instant,
        expected_local_input_epoch: u64,
    ) -> Result<nickel_remote_control::semantics::SurfaceSemanticActionOutcome, String> {
        use nickel_remote_control::{
            leases::{ResourceEvidence, ResourceId},
            semantics::{SurfaceSemanticActionOutcome, SurfaceSemanticCompletion},
        };
        request.validate().map_err(str::to_owned)?;
        if Instant::now() >= deadline {
            return Err("semantic action expired before dispatch".into());
        }
        let observation = self.current_shell_surface(
            shell,
            state,
            &request.surface_id,
            request.surface_generation,
        )?;
        // Text edits in the launcher/run field and volume-OSD actions update
        // only their production UiHost. Actions that can emit a platform effect
        // stay closed until Windows has an authority-preserving continuation.
        if !windows_semantic_action_has_guarded_disposition(
            observation.role,
            windows_semantic_mutation_kind(&request.action),
        ) {
            return Err(
                "semantic mutation effects are unavailable for this Windows shell role".into(),
            );
        }
        if expected_local_input_epoch != local_input_epoch()
            || self.keyboard_hold.is_some()
            || self.pointer_hold.is_some()
            || state.pointer_interaction_active()
            || !crate::windows_remote_input::physical_input_idle()
        {
            return Err("shared input is busy".into());
        }
        let identity = ResourceId {
            id: request.surface_id.clone(),
            generation: request.surface_generation,
        };
        let output_name = observation
            .output
            .as_deref()
            .ok_or("shell output is unavailable")?;
        let output = ResourceId {
            id: output_name.to_owned(),
            generation: self
                .resources
                .output_generation(output_name)
                .ok_or("shell output has retired")?,
        };
        let ancestors = self.shell_surface_ancestors(shell, state, &identity)?;
        let evidence = ResourceEvidence {
            surface: Some(&identity),
            window: None,
            verified_application: None,
            output: Some(&output),
            authorized_surface_ancestors: &ancestors,
            protected: false,
        };
        permit.with_input(&evidence, || {
            if Instant::now() >= deadline {
                return Err("semantic action expired before dispatch".into());
            }
            if expected_local_input_epoch != local_input_epoch() {
                return Err("local input interrupted the semantic action".into());
            }
            if !crate::windows_remote_input::physical_input_idle() {
                return Err("shared input is busy".into());
            }
            let outcome = state.perform_bounded_shell_action(
                observation.role,
                observation.output.as_deref(),
                request.tree_generation,
                request.node as usize,
                windows_semantic_mutation(request.action),
                nickel_remote_control::semantics::MAX_MUTATION_TEXT_BYTES,
            )?;
            if expected_local_input_epoch != local_input_epoch() {
                return Err(
                    "local input overlapped the semantic action; result uncertain, do not retry"
                        .into(),
                );
            }
            if !outcome.effects.is_empty() {
                return Err("unexpected deferred semantic effect; result uncertain".into());
            }
            if !outcome.host.semantic_failures.is_empty()
                || !outcome.host.failures.is_empty()
                || !outcome.host.completion_failures.is_empty()
            {
                return Err("semantic action completed with a local failure; do not retry".into());
            }
            if outcome.host.clipboard_text.is_some() {
                return Err("semantic clipboard effect is unavailable; do not retry".into());
            }
            Ok(SurfaceSemanticActionOutcome {
                changed: outcome.host.changed,
                completion: SurfaceSemanticCompletion::UiUpdated,
                partial: false,
            })
        })
    }

    fn perform_diagnostic_snapshot(
        &mut self,
        shell: &WinitShell,
        state: &crate::live_shell::LiveShell,
        permit: DesktopPermit,
        mut prepared: crate::platform::remote_observation::Prepared,
    ) -> Result<nickel_remote_control::diagnostics::DiagnosticSnapshot, String> {
        use nickel_remote_control::diagnostics::*;

        self.reconcile_prepared_resources(&permit, &mut prepared)?;
        let observed_at_us = self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64;
        let lease_metrics = permit.lease_metrics_snapshot(observed_at_us);
        let scope = permit.resource_scope()?;
        let native_windows = self.resources.native_windows(&scope).collect::<Vec<_>>();
        let workspaces_available = permit
            .with_debug(!self.desktop_unlocked, || {
                crate::windows_virtual_workspaces::native::observe(&native_windows)
            })
            .and_then(|observation| self.workspaces.reconcile(observation))
            .is_ok();
        let snapshot = permit.with_debug(!self.desktop_unlocked, || {
            self.observation_generation = self
                .observation_generation
                .checked_add(1)
                .ok_or("Windows observation generations exhausted")?;
            let generation = self.observation_generation;
            let mut windows: Vec<_> = self
                .resources
                .windows(&scope)
                .map(|(window, _)| window)
                .collect();
            for window in &mut windows {
                if let Some(native) = self
                    .resources
                    .window(&window.id, window.generation)
                    .map(|window| window.native)
                {
                    window.workspace = self.workspaces.workspace_for_window(native).unwrap_or(0);
                }
            }
            let projected_workspace_windows = self
                .resources
                .projected_native_windows(&scope)
                .collect::<Vec<_>>();
            let (workspaces, workspaces_truncated) = if workspaces_available {
                self.workspaces.diagnostics(&projected_workspace_windows)
            } else {
                (Vec::new(), false)
            };
            let outputs: Vec<_> = self
                .resources
                .outputs(&scope)
                .map(|(output, _)| output)
                .collect();
            let projected_native_windows = self
                .resources
                .native_windows(&scope)
                .collect::<std::collections::HashSet<_>>();
            let mut active = windows.iter().filter(|window| window.active);
            let focused_window = active.next().map(|window| window.id.clone());
            let focused_window = active.next().is_none().then_some(focused_window).flatten();
            let keyboard_held = self.keyboard_hold.is_some();
            let pointer_held = self.pointer_hold.is_some();
            let shell_behavior_state = state.remote_shell_behavior_state();
            let shell_input_observations = shell.remote_shell_surface_observations(state);
            let protected_desktop = !self.desktop_unlocked
                || state.surface_visible(crate::winit_shell::SurfaceRole::Lock);
            let (shell_surfaces, shell_surfaces_truncated) =
                crate::windows_shell_diagnostics::project(
                    protected_desktop,
                    shell_input_observations.iter().cloned(),
                );
            let shell_surface_authority =
                crate::remote_surface_authority::SurfaceAuthority::from_shell_surfaces(
                    &shell_surfaces,
                )?;
            let shell_renderers = crate::windows_shell_diagnostics::project_presenters(
                protected_desktop,
                observed_at_us,
                shell_input_observations.iter().cloned(),
            );
            let (shell_image_cache, mut projected_resources) =
                crate::windows_shell_diagnostics::project_image_cache(
                    generation,
                    observed_at_us,
                    state.image_cache_diagnostics_for_previews(|window| {
                        usize::try_from(window.0)
                            .ok()
                            .is_some_and(|native| projected_native_windows.contains(&native))
                    }),
                );
            let shared_presenter_cache =
                Some(crate::windows_shell_diagnostics::project_presenter_cache(
                    generation,
                    observed_at_us,
                    shell.memory_diagnostics(),
                ));
            projected_resources.renderer_surfaces = shell_renderers.len() as u64;
            projected_resources.software_frame_bytes =
                shell_renderers.iter().fold(0_u64, |total, renderer| {
                    total.saturating_add(renderer.software_frame_bytes)
                });
            let native_preview =
                crate::platform::native_preview_diagnostics(&projected_native_windows);
            let map_recipient =
                |native: Option<usize>, compositor_grabbed: bool, remote_hold_active: bool| {
                    let Some(native) = native else {
                        return Some(InputDeviceDiagnostic {
                            focused_window: None,
                            focused_surface: None,
                            compositor_grabbed,
                            remote_hold_active,
                        });
                    };
                    if let Some(id) = self.resources.diagnostic_window_id(&scope, native) {
                        return Some(InputDeviceDiagnostic {
                            focused_window: Some(id),
                            focused_surface: None,
                            compositor_grabbed,
                            remote_hold_active,
                        });
                    }
                    let surface = crate::windows_shell_diagnostics::input_surface(
                        protected_desktop,
                        native,
                        &shell_input_observations,
                    )?;
                    let identity = nickel_remote_control::leases::ResourceId {
                        id: format!("windows-shell:{}", surface.generation),
                        generation: surface.generation,
                    };
                    let ancestors = shell_surface_authority.ancestors(&identity);
                    self.resources
                        .shell_surface_authorized(
                            &scope,
                            &identity,
                            surface.output.as_deref(),
                            &ancestors,
                        )
                        .then_some(InputDeviceDiagnostic {
                            focused_window: None,
                            focused_surface: Some(identity),
                            compositor_grabbed,
                            remote_hold_active,
                        })
                };
            let keyboard = map_recipient(prepared.input.keyboard_root, false, keyboard_held);
            let pointer = prepared
                .input
                .pointer_recipient_available
                .then(|| {
                    map_recipient(
                        prepared.input.pointer_recipient_root,
                        prepared.input.pointer_grabbed,
                        pointer_held,
                    )
                })
                .flatten();
            let pointer_hit_test = if let Some(native) = prepared.input.pointer_root {
                if let Some(id) = self.resources.diagnostic_window_id(&scope, native) {
                    Some(PointerHitTestDiagnostic {
                        window: Some(id),
                        surface: None,
                        semantic_tree_generation: None,
                        semantic_node: None,
                        decoration: None,
                    })
                } else {
                    crate::windows_shell_diagnostics::input_surface(
                        protected_desktop,
                        native,
                        &shell_input_observations,
                    )
                    .and_then(|surface| {
                        let identity = nickel_remote_control::leases::ResourceId {
                            id: format!("windows-shell:{}", surface.generation),
                            generation: surface.generation,
                        };
                        let ancestors = shell_surface_authority.ancestors(&identity);
                        if !self.resources.shell_surface_authorized(
                            &scope,
                            &identity,
                            surface.output.as_deref(),
                            &ancestors,
                        ) {
                            return None;
                        }
                        let point = prepared.input.pointer_client?;
                        let decoration = prepared.input.pointer_decoration;
                        let semantic = if decoration.is_none() {
                            state
                                .bounded_shell_semantics(surface.role, surface.output.as_deref())
                                .ok()
                                .and_then(|(generation, nodes)| {
                                    crate::windows_shell_diagnostics::semantic_node_at(
                                        surface.scale_factor,
                                        point,
                                        nodes.iter().map(|node| {
                                            [
                                                node.bounds.origin.x,
                                                node.bounds.origin.y,
                                                node.bounds.size.width,
                                                node.bounds.size.height,
                                            ]
                                        }),
                                    )
                                    .map(|ordinal| (generation, ordinal))
                                })
                        } else {
                            None
                        };
                        Some(PointerHitTestDiagnostic {
                            window: None,
                            surface: Some(identity),
                            semantic_tree_generation: semantic.map(|value| value.0),
                            semantic_node: semantic.map(|value| value.1),
                            decoration,
                        })
                    })
                }
            } else {
                Some(PointerHitTestDiagnostic {
                    window: None,
                    surface: None,
                    semantic_tree_generation: None,
                    semantic_node: None,
                    decoration: None,
                })
            };
            Ok(DiagnosticSnapshot {
                observation_generation: generation,
                observed_at_us,
                windows,
                outputs,
                workspaces,
                internal_applications: windows_internal_application_diagnostics(),
                internal_renderers: Vec::new(),
                shell_renderers,
                shell_image_cache: Some(shell_image_cache),
                shared_presenter_cache,
                projected_resources,
                pending_effects: PendingEffectsDiagnostic {
                    observation_generation: generation,
                    observed_at_us,
                    desktop_scene_updates: 0,
                    image_copy_frames: 0,
                    launch_observations: 0,
                    output_retirements: 0,
                    shell_focus_pending: false,
                },
                shell_surfaces,
                focused_window,
                input: InputDiagnostic {
                    observation_generation: generation,
                    observed_at_us,
                    keyboard,
                    pointer,
                    pointer_hit_test,
                },
                shortcuts: shell.shortcut_diagnostic(generation, observed_at_us),
                stacking_front_to_back: Vec::new(),
                preview: PreviewDiagnostic {
                    presentation_generation: 0,
                    readback_bytes: 0,
                    capture_failures: 0,
                    native_presentation_generation: native_preview
                        .map(|preview| preview.presentation_generation),
                    native_presentation_failures: native_preview
                        .map(|preview| preview.presentation_failures),
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
                platform_refreshes: self
                    .platform_refreshes
                    .iter()
                    .map(|refresh| refresh.retained_at(observed_at_us))
                    .collect(),
                application_inventory_refresh: None,
                codex_feature: codex_feature_diagnostic(generation, observed_at_us, state),
                shell_behavior: shell_behavior_diagnostic(
                    generation,
                    observed_at_us,
                    self.resources.output_topology_generation(),
                    shell.bar_on_all_displays(),
                    shell_behavior_state,
                ),
                settings_worker: None,
                diagnostic_worker: self.platform_refresh_worker.snapshot(),
                application_launch: self.application_launch_diagnostic(),
                external_accessibility: self
                    .external_accessibility
                    .as_ref()
                    .map(|snapshot| snapshot.retained_at(observed_at_us)),
                recent_events: self.desktop_events.snapshot(),
                diagnostic_logs: windows_diagnostic_logs(),
                frame_trace: self
                    .remote_frame_trace
                    .as_ref()
                    .map(|trace| trace.snapshot()),
                trace_lifecycle: trace_lifecycle_snapshot(&permit, self.start_time),
                truncated: shell_surfaces_truncated || workspaces_truncated,
                unavailable_domains: windows_unavailable_diagnostic_domains(),
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
        platform_refresh: Option<PreparedWindowsPlatformRefresh>,
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
                    | DiagnosticAction::RefreshPlatformStatus { .. }
                    | DiagnosticAction::StartFrameTrace { .. }
                    | DiagnosticAction::StopFrameTrace
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
            DiagnosticAction::StartFrameTrace { duration_seconds } => {
                permit.with_debug(!self.desktop_unlocked, || {
                    if self
                        .remote_frame_trace
                        .as_ref()
                        .is_some_and(|trace| trace.active())
                    {
                        return Err("frame trace capacity reached".into());
                    }
                    self.remote_frame_trace = Some(
                        nickel_remote_control::frame_trace::FrameTrace::new_authorized(
                            permit.clone(),
                            *duration_seconds,
                            nickel_remote_control::frame_trace::FrameTraceCategory::NestedFrameDispatch,
                        )?,
                    );
                    Ok(())
                })?;
            }
            DiagnosticAction::StopFrameTrace => {
                permit.with_debug(!self.desktop_unlocked, || {
                    if let Some(trace) = self.remote_frame_trace.as_mut() {
                        if !trace.owned_by(&permit) {
                            return Err("frame trace belongs to another lease".into());
                        }
                        trace.stop();
                    }
                    Ok(())
                })?;
            }
            DiagnosticAction::RefreshPlatformStatus { domain } => {
                let prepared = platform_refresh
                    .ok_or("Windows platform refresh preparation is unavailable")?;
                if prepared.domain != *domain {
                    return Err("Windows platform refresh domain changed before commit".into());
                }
                permit.with_debug(!self.desktop_unlocked, || {
                    self.platform_refresh_generation = self
                        .platform_refresh_generation
                        .checked_add(1)
                        .ok_or("Windows platform refresh generation exhausted")?;
                    let (
                        network_available,
                        bluetooth_available,
                        audio_available,
                        printers_available,
                        volumes_available,
                        filesystems_available,
                        printer_count,
                        volume_count,
                        filesystem_count,
                        associations_available,
                        association_targets_queried,
                        effective_associations,
                        directly_writable_associations,
                        maintenance_available,
                        updates_available,
                        restart_required,
                        firewall_healthy,
                        malware_protection_healthy,
                        known_permission_states,
                        secure_storage_status_available,
                        partial,
                        reconciliation_confirmed,
                    ) = match prepared.data {
                        PreparedWindowsPlatformRefreshData::Connectivity(refresh) => (
                            refresh.network.available,
                            refresh.bluetooth.available,
                            false,
                            false,
                            false,
                            false,
                            0,
                            0,
                            0,
                            false,
                            0,
                            0,
                            0,
                            false,
                            None,
                            None,
                            None,
                            None,
                            0,
                            false,
                            refresh.partial,
                            false,
                        ),
                        PreparedWindowsPlatformRefreshData::Audio(refresh) => (
                            false,
                            false,
                            refresh.audio.available,
                            false,
                            false,
                            false,
                            0,
                            0,
                            0,
                            false,
                            0,
                            0,
                            0,
                            false,
                            None,
                            None,
                            None,
                            None,
                            0,
                            false,
                            refresh.partial,
                            false,
                        ),
                        PreparedWindowsPlatformRefreshData::Peripherals(refresh) => (
                            false,
                            false,
                            false,
                            refresh.printers_available,
                            refresh.volumes_available,
                            refresh.filesystems_available,
                            refresh.printer_count,
                            refresh.volume_count,
                            refresh.filesystem_count,
                            false,
                            0,
                            0,
                            0,
                            false,
                            None,
                            None,
                            None,
                            None,
                            0,
                            false,
                            refresh.partial,
                            false,
                        ),
                        PreparedWindowsPlatformRefreshData::Maintenance(refresh) => (
                            false,
                            false,
                            false,
                            false,
                            false,
                            false,
                            0,
                            0,
                            0,
                            false,
                            0,
                            0,
                            0,
                            refresh.maintenance_available,
                            refresh.updates_available,
                            refresh.restart_required,
                            refresh.firewall_healthy,
                            refresh.malware_protection_healthy,
                            refresh.known_permission_states,
                            refresh.secure_storage_status_available,
                            refresh.partial,
                            false,
                        ),
                        PreparedWindowsPlatformRefreshData::DefaultAssociations(refresh) => (
                            false,
                            false,
                            false,
                            false,
                            false,
                            false,
                            0,
                            0,
                            0,
                            refresh.associations_available,
                            refresh.targets_queried,
                            refresh.effective_associations,
                            refresh.directly_writable_associations,
                            false,
                            None,
                            None,
                            None,
                            None,
                            0,
                            false,
                            refresh.partial,
                            false,
                        ),
                    };
                    let outcome = nickel_remote_control::diagnostics::PlatformRefreshOutcome {
                        domain: *domain,
                        generation: self.platform_refresh_generation,
                        observation_started_at_us: prepared
                            .observation_started
                            .saturating_duration_since(self.start_time)
                            .as_micros()
                            .min(u128::from(u64::MAX))
                            as u64,
                        observed_at_us: prepared
                            .observed
                            .saturating_duration_since(self.start_time)
                            .as_micros()
                            .min(u128::from(u64::MAX))
                            as u64,
                        preparation_duration_us: prepared.preparation_duration_us,
                        stale: false,
                        network_available,
                        bluetooth_available,
                        audio_available,
                        printers_available,
                        volumes_available,
                        filesystems_available,
                        printer_count,
                        volume_count,
                        filesystem_count,
                        maintenance_available,
                        updates_available,
                        restart_required,
                        firewall_healthy,
                        malware_protection_healthy,
                        known_permission_states,
                        secure_storage_status_available,
                        associations_available,
                        association_targets_queried,
                        effective_associations,
                        directly_writable_associations,
                        partial,
                        reconciliation_confirmed,
                    };
                    self.platform_refreshes
                        .retain(|entry| entry.domain != *domain);
                    self.platform_refreshes.push(outcome);
                    self.desktop_events.record(
                        DesktopEventKind::PlatformRefreshCompleted {
                            domain: *domain,
                            generation: self.platform_refresh_generation,
                            partial,
                        },
                        self.start_time
                            .elapsed()
                            .as_micros()
                            .min(u128::from(u64::MAX)) as u64,
                    );
                    Ok(())
                })?;
            }
            _ => unreachable!("unsupported diagnostic action rejected above"),
        }
        permit.with_debug(!self.desktop_unlocked, || {
            self.observation_generation = self
                .observation_generation
                .checked_add(1)
                .ok_or("Windows observation generations exhausted")?;
            let submitted_at_us =
                self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64;
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
            let platform_refresh = match &action {
                DiagnosticAction::RefreshPlatformStatus { domain } => self
                    .platform_refreshes
                    .iter()
                    .find(|entry| entry.domain == *domain)
                    .cloned(),
                _ => None,
            };
            Ok(DiagnosticActionOutcome {
                action,
                observation_generation: self.observation_generation,
                submitted_at_us,
                presentation_confirmed: false,
                output_identification: None,
                application_inventory_refresh: None,
                platform_refresh,
            })
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
        prepared: Option<crate::platform::remote_observation::Prepared>,
    ) -> Result<nickel_remote_control::diagnostics::ApplicationInventory, String> {
        use nickel_remote_control::leases::{ResourceEvidence, ResourceScope};

        if !self.desktop_unlocked {
            return Err("Windows input desktop is protected".into());
        }
        let scope = permit.resource_scope()?;
        let (expected, output) = match &scope {
            ResourceScope::FullSession => (None, None),
            ResourceScope::Application(identity) => (Some(identity.as_str()), None),
            ResourceScope::Output(identity) => {
                let mut prepared =
                    prepared.ok_or("Windows output evidence is required for launch inventory")?;
                self.reconcile_prepared_resources(&permit, &mut prepared)?;
                let output = self
                    .resources
                    .output_resource(identity)
                    .ok_or("Windows launch output is unavailable")?;
                if prepared
                    .outputs
                    .iter()
                    .filter(|value| *value == output)
                    .count()
                    != 1
                {
                    return Err("Windows launch output changed during inventory".into());
                }
                (None, Some(identity))
            }
            ResourceScope::Surface(_) | ResourceScope::Window(_) => {
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
            output,
            authorized_surface_ancestors: &[],
            protected: false,
        };
        permit.with_resource(&evidence, || Ok(inventory))
    }

    fn plan_application_launch(
        &mut self,
        permit: &DesktopPermit,
        request: &nickel_remote_control::diagnostics::LaunchApplicationRequest,
        prepared: Option<crate::platform::remote_observation::Prepared>,
    ) -> Result<PlannedApplicationLaunch, String> {
        use nickel_remote_control::leases::{ResourceEvidence, ResourceScope};

        if !self.desktop_unlocked {
            return Err("Windows input desktop is protected".into());
        }
        if request.application_id.is_empty() || request.application_id.len() > 512 {
            return Err("invalid application launch target".into());
        }
        let plan = self
            .applications
            .plan_launch(request.catalog_generation, &request.application_id)?;
        let scope = permit.resource_scope()?;
        let output = match &scope {
            ResourceScope::FullSession => None,
            ResourceScope::Application(identity) if identity == plan.identity() => None,
            ResourceScope::Application(_) => {
                return Err("installed launch target is outside the application lease".into());
            }
            ResourceScope::Output(identity) => {
                let mut prepared =
                    prepared.ok_or("Windows output evidence is required for application launch")?;
                self.reconcile_prepared_resources(permit, &mut prepared)?;
                let output = self
                    .resources
                    .output_resource(identity)
                    .cloned()
                    .ok_or("Windows launch output is unavailable")?;
                if prepared
                    .outputs
                    .iter()
                    .filter(|value| *value == &output)
                    .count()
                    != 1
                {
                    return Err("Windows launch output changed during preparation".into());
                }
                Some((identity.clone(), output))
            }
            ResourceScope::Surface(_) | ResourceScope::Window(_) => {
                return Err("this Windows lease cannot launch applications".into());
            }
        };
        let evidence = ResourceEvidence {
            surface: None,
            window: None,
            verified_application: output.is_none().then_some(plan.identity()),
            output: output.as_ref().map(|(identity, _)| identity),
            authorized_surface_ancestors: &[],
            protected: false,
        };
        permit.with_resource(&evidence, || Ok(()))?;
        Ok(PlannedApplicationLaunch {
            plan,
            output,
            commit_evidence: None,
        })
    }

    fn commit_application_launch(
        &mut self,
        permit: DesktopPermit,
        request: nickel_remote_control::diagnostics::LaunchApplicationRequest,
        mut plan: PlannedApplicationLaunch,
        staged: crate::windows_launch_broker::StagedLaunch,
        deadline: Instant,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<AuthorizedApplicationLaunch, String> {
        use nickel_remote_control::leases::{ResourceEvidence, ResourceScope};
        use std::sync::atomic::Ordering;

        if Instant::now() >= deadline || cancelled.load(Ordering::Acquire) {
            return Err("Windows application launch timed out".into());
        }
        if !self.desktop_unlocked {
            return Err("Windows input desktop is protected".into());
        }
        if request.catalog_generation != plan.plan.catalog_generation()
            || request.application_id != plan.plan.application_id()
        {
            return Err("Windows application launch plan does not match its request".into());
        }
        self.applications.revalidate_launch(&plan.plan)?;
        let staged_identity = staged.application_identity().to_owned();
        if staged_identity != plan.plan.identity() {
            return Err("installed application changed; enumerate it again".into());
        }
        let scope = permit.resource_scope()?;
        let (expected, output, baseline) = match &scope {
            ResourceScope::FullSession => (None, None, Default::default()),
            ResourceScope::Application(identity) => {
                (Some(identity.as_str()), None, Default::default())
            }
            ResourceScope::Output(identity) => {
                let (planned_identity, planned_output) = plan
                    .output
                    .as_ref()
                    .ok_or("Windows launch output plan is unavailable")?;
                if identity != planned_identity {
                    return Err("Windows launch output changed before commit".into());
                }
                let mut prepared = *plan
                    .commit_evidence
                    .take()
                    .ok_or("Windows output evidence is required at launch commit")?;
                self.reconcile_prepared_resources(&permit, &mut prepared)?;
                let current = self
                    .resources
                    .output_resource(identity)
                    .ok_or("Windows launch output is unavailable")?;
                if current != planned_output
                    || prepared
                        .outputs
                        .iter()
                        .filter(|value| *value == current)
                        .count()
                        != 1
                {
                    return Err("Windows launch output changed before commit".into());
                }
                let baseline = prepared
                    .windows
                    .iter()
                    .map(WindowIncarnation::from)
                    .collect();
                (None, Some((identity.clone(), current.clone())), baseline)
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
            verified_application: expected.map(|_| staged_identity.as_str()),
            output: output.as_ref().map(|(identity, _)| identity),
            authorized_surface_ancestors: &[],
            protected: false,
        };
        if Instant::now() >= deadline || cancelled.load(Ordering::Acquire) {
            return Err("Windows application launch timed out".into());
        }
        let placement = if let Some((output, native_output)) = output.clone() {
            if self.pending_output_launches.len() == MAX_PENDING_OUTPUT_LAUNCHES {
                return Err("Windows output launch placement capacity reached".into());
            }
            self.next_output_launch = self
                .next_output_launch
                .checked_add(1)
                .ok_or("Windows output launch placement generations exhausted")?;
            let ticket = OutputLaunchPlacementTicket {
                id: self.next_output_launch,
                output: output.clone(),
                deadline: Instant::now() + OUTPUT_LAUNCH_PLACEMENT_TTL,
            };
            self.pending_output_launches.insert(
                ticket.id,
                PendingOutputLaunchPlacement {
                    permit: permit.clone(),
                    output,
                    native_output,
                    baseline,
                    root: None,
                    root_deadline: Instant::now() + OUTPUT_LAUNCH_ROOT_TTL,
                    deadline: ticket.deadline,
                },
            );
            Some(ticket)
        } else {
            None
        };
        match staged.commit(&permit, &evidence, Instant::now()) {
            Ok(committed) => Ok(AuthorizedApplicationLaunch {
                committed,
                placement,
            }),
            Err(error) => {
                if let Some(placement) = placement {
                    self.pending_output_launches.remove(&placement.id);
                }
                Err(error)
            }
        }
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
        let mutation = self.resources.prepare_window_mutation(
            &scope,
            action,
            crate::windows_resource_owner::WindowMutationInputState {
                keyboard_held: self.keyboard_hold.is_some(),
                pointer_held: self.pointer_hold.is_some(),
                physical_input_idle: crate::windows_remote_input::physical_input_idle(),
            },
        )?;
        self.resources.revalidate_window_mutation(&mutation)?;
        let expected_input_epoch = local_input_epoch();
        if let nickel_remote_control::window_actions::WindowAction::MoveToWorkspace { workspace } =
            action
        {
            let native = window.native;
            let native_windows = self.resources.native_windows(&scope).collect::<Vec<_>>();
            self.workspaces
                .reconcile(crate::windows_virtual_workspaces::native::observe(
                    &native_windows,
                )?)?;
            let target = self
                .workspaces
                .native_id(workspace)
                .ok_or("Windows workspace is unavailable")?;
            permit.with_input(&evidence, || {
                if expected_input_epoch != local_input_epoch()
                    || !crate::windows_remote_input::physical_input_idle()
                {
                    return Err("local input cancelled the workspace move".into());
                }
                prepared.revalidate()?;
                crate::windows_virtual_workspaces::native::move_window(native, target)
            })?;

            // The public call only confirms request acceptance. Re-observe the
            // native desktop membership before claiming that the move landed.
            let mut observed = crate::platform::remote_observation::Prepared::prepare(&permit)?;
            self.reconcile_prepared_resources(&permit, &mut observed)?;
            let native_windows = self.resources.native_windows(&scope).collect::<Vec<_>>();
            self.workspaces
                .reconcile(crate::windows_virtual_workspaces::native::observe(
                    &native_windows,
                )?)?;
            let mut window = self.resources.windows(&scope).find_map(|(window, _)| {
                (window.id == id && window.generation == generation).then_some(window)
            });
            if let Some(window) = &mut window {
                window.workspace = self.workspaces.workspace_for_window(native).unwrap_or(0);
            }
            return Ok(
                nickel_remote_control::window_actions::WindowOutcome::observed(action, window),
            );
        }
        permit.with_input(&evidence, || {
            if expected_input_epoch != local_input_epoch()
                || !crate::windows_remote_input::physical_input_idle()
            {
                return Err("local input cancelled the window mutation".into());
            }
            prepared.revalidate()?;
            let result =
                crate::platform::remote_observation::request_window_action(window, session, action);
            if expected_input_epoch != local_input_epoch()
                || !crate::windows_remote_input::physical_input_idle()
            {
                return Err("local input interrupted the window mutation".into());
            }
            result
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

    fn perform_workspace_action(
        &mut self,
        permit: DesktopPermit,
        mut prepared: crate::platform::remote_observation::Prepared,
        action: nickel_remote_control::diagnostics::WorkspaceAction,
    ) -> Result<nickel_remote_control::diagnostics::WorkspaceOutcome, String> {
        use nickel_remote_control::{
            diagnostics::{WorkspaceAction, WorkspaceOutcome},
            leases::ResourceEvidence,
        };
        let evidence = ResourceEvidence {
            surface: None,
            window: None,
            verified_application: None,
            output: None,
            authorized_surface_ancestors: &[],
            protected: !self.desktop_unlocked,
        };
        permit.with_resource(&evidence, || match action {
            WorkspaceAction::List => Ok(()),
            WorkspaceAction::Create | WorkspaceAction::Switch { .. } | WorkspaceAction::Remove { .. } => Err(
                "Windows does not expose supported create, switch, or remove virtual-desktop authority"
                    .into(),
            ),
        })?;
        self.reconcile_prepared_resources(&permit, &mut prepared)?;
        let scope = permit.resource_scope()?;
        let native_windows = self.resources.native_windows(&scope).collect::<Vec<_>>();
        self.workspaces
            .reconcile(crate::windows_virtual_workspaces::native::observe(
                &native_windows,
            )?)?;
        prepared.revalidate()?;
        permit.check_live()?;
        let projected = self
            .resources
            .projected_native_windows(&scope)
            .collect::<Vec<_>>();
        let (workspaces, truncated) = self.workspaces.diagnostics(&projected);
        self.observation_generation = self
            .observation_generation
            .checked_add(1)
            .ok_or("Windows observation generations exhausted")?;
        Ok(WorkspaceOutcome {
            requested: action,
            created_workspace: None,
            observation_generation: self.observation_generation,
            observed_at_us: self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
            workspaces,
            truncated,
        })
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
        let theme = self
            .indicators
            .values()
            .next()
            .map(|indicator| indicator.host.host.application().theme);
        if theme.is_none_or(|theme| self.sync_indicators(shell, theme).is_err()) {
            self.clear_indicators(shell);
        }
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
        let stopped_confirmation = self
            .stop_confirmation_until
            .is_some_and(|deadline| now < deadline);
        if grants.is_empty() && !stopped_confirmation {
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
                let id = shell.create_trusted_control_surface(&name, grants.len().max(1))?;
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
                        stopped_confirmation,
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
            shell.resize_trusted_control_surface(indicator.id, grants.len().max(1))?;
            let (width, height) = shell
                .surface(indicator.id)
                .ok_or("trusted surface disappeared")?
                .window()
                .size();
            let app = indicator.host.application_mut();
            let changed = app.grants != grants
                || app.theme != theme
                || app.transport != transport
                || app.stopped_confirmation != stopped_confirmation;
            app.transport = transport.to_owned();
            app.grants = grants.clone();
            app.theme = theme;
            app.stopped_confirmation = stopped_confirmation;
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
                self.remote_frame_trace = None;
                self.pending_output_launches.clear();
                self.stop_confirmation_until = Some(Instant::now() + Duration::from_secs(3));
                let mut settings = RemoteAiControlSettings::load_default().unwrap_or_default();
                settings.set_requested(false);
                self.remote_control.emergency_stop_at(settings.generation);
                if let Ok(control) = self.remote_control.control().lock() {
                    self.local_cues
                        .emergency_confirmation(control.leases(), Instant::now());
                }
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

fn windows_unavailable_diagnostic_domains()
-> Vec<nickel_remote_control::diagnostics::UnavailableDiagnosticDomain> {
    use nickel_remote_control::diagnostics::UnavailableDiagnosticDomain as Domain;
    vec![
        Domain::NativeGpuRendererTiming,
        Domain::OtherProductionEffectEventCategories,
        Domain::OtherTraceCategories,
        Domain::WindowsVirtualWorkspaceCreateSwitchRemove,
        Domain::WindowsPerSurfaceRendererCacheAttribution,
        Domain::WindowsPreviewPixelReadback,
        Domain::WindowsOutputPixelCapture,
        Domain::WindowsNonWindowPointerTargets,
        Domain::WindowsSettingsWorker,
    ]
}

/// Windows does not embed ordinary application clients in the compositor.
/// Nickel File, Settings, and other native tools are projected through the
/// scoped `windows` inventory; Winit-owned chrome is projected through scoped
/// `shell_surfaces`. The only application-class Winit surface is Codex and is
/// protected. Returning an empty inventory while omitting this domain from
/// `unavailable_domains` is the protocol's explicit supported-empty evidence.
fn windows_internal_application_diagnostics()
-> Vec<nickel_remote_control::diagnostics::InternalApplicationDiagnostic> {
    Vec::new()
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

fn windows_semantic_projection(
    role: crate::winit_shell::SurfaceRole,
    mut projection: Vec<nickel_ui::SemanticNodeSnapshot>,
) -> Result<Vec<nickel_remote_control::semantics::SemanticNode>, String> {
    use nickel_remote_control::semantics::{SemanticNode, SemanticValue};
    for node in &mut projection {
        node.actions
            .retain(|action| windows_semantic_action_has_guarded_disposition(role, *action));
        node.enabled = !node.actions.is_empty();
    }
    projection
        .into_iter()
        .enumerate()
        .map(|(index, node)| {
            let bounds = [
                node.bounds.origin.x,
                node.bounds.origin.y,
                node.bounds.size.width,
                node.bounds.size.height,
            ];
            if !bounds.iter().all(|value| value.is_finite()) {
                return Err("semantic geometry unavailable".to_owned());
            }
            let value = match node.value {
                Some(nickel_ui::SemanticValueSnapshot::Boolean(value)) => {
                    Some(SemanticValue::Boolean(value))
                }
                Some(nickel_ui::SemanticValueSnapshot::Text(value)) => {
                    Some(SemanticValue::Text(value))
                }
                Some(nickel_ui::SemanticValueSnapshot::Number {
                    value,
                    minimum,
                    maximum,
                    step,
                }) => {
                    if ![value, minimum, maximum, step]
                        .iter()
                        .all(|value| value.is_finite())
                    {
                        return Err("semantic value unavailable".to_owned());
                    }
                    Some(SemanticValue::Number {
                        value,
                        minimum,
                        maximum,
                        step,
                    })
                }
                Some(nickel_ui::SemanticValueSnapshot::ProtectedText { .. }) => {
                    return Err("protected surface".into());
                }
                None => None,
            };
            Ok(SemanticNode {
                id: u32::try_from(index).map_err(|_| "semantic node exceeds limit")?,
                role: node.role.map(|role| format!("{role:?}")),
                bounds,
                name: node.name,
                description: node.description,
                enabled: node.enabled,
                focused: node.focused,
                actions: node
                    .actions
                    .into_iter()
                    .map(|action| format!("{action:?}"))
                    .collect(),
                value,
            })
        })
        .collect()
}

fn windows_semantic_mutation(
    action: nickel_remote_control::semantics::SemanticMutation,
) -> nickel_ui::SemanticAction {
    use nickel_remote_control::semantics::{SemanticInvocation, SemanticMutation};
    match action {
        SemanticMutation::SetBoolean(value) => {
            nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Boolean(value))
        }
        SemanticMutation::SetNumber(value) => {
            nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Number(value))
        }
        SemanticMutation::SetText(value) => {
            nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Text(value))
        }
        SemanticMutation::Invoke(value) => nickel_ui::SemanticAction::Invoke(match value {
            SemanticInvocation::Activate => nickel_ui::ActionKind::Activate,
            SemanticInvocation::Cancel => nickel_ui::ActionKind::Cancel,
            SemanticInvocation::ContextMenu => nickel_ui::ActionKind::ContextMenu,
            SemanticInvocation::Increment => nickel_ui::ActionKind::Increment,
            SemanticInvocation::Decrement => nickel_ui::ActionKind::Decrement,
            SemanticInvocation::Expand => nickel_ui::ActionKind::Expand,
            SemanticInvocation::Collapse => nickel_ui::ActionKind::Collapse,
            SemanticInvocation::Select => nickel_ui::ActionKind::Select,
            SemanticInvocation::Dismiss => nickel_ui::ActionKind::Dismiss,
            SemanticInvocation::Scroll => nickel_ui::ActionKind::Scroll,
            SemanticInvocation::EnterNavigation => nickel_ui::ActionKind::EnterNavigation,
            SemanticInvocation::ExitNavigation => nickel_ui::ActionKind::ExitNavigation,
        }),
    }
}

fn windows_semantic_action_has_guarded_disposition(
    role: crate::winit_shell::SurfaceRole,
    action: nickel_ui::ActionKind,
) -> bool {
    role == crate::winit_shell::SurfaceRole::VolumeOsd
        || (role == crate::winit_shell::SurfaceRole::Launcher
            && action == nickel_ui::ActionKind::SetValue)
}

fn windows_semantic_mutation_kind(
    action: &nickel_remote_control::semantics::SemanticMutation,
) -> nickel_ui::ActionKind {
    use nickel_remote_control::semantics::{SemanticInvocation, SemanticMutation};
    match action {
        SemanticMutation::SetBoolean(_)
        | SemanticMutation::SetNumber(_)
        | SemanticMutation::SetText(_) => nickel_ui::ActionKind::SetValue,
        SemanticMutation::Invoke(invocation) => match invocation {
            SemanticInvocation::Activate => nickel_ui::ActionKind::Activate,
            SemanticInvocation::Cancel => nickel_ui::ActionKind::Cancel,
            SemanticInvocation::ContextMenu => nickel_ui::ActionKind::ContextMenu,
            SemanticInvocation::Increment => nickel_ui::ActionKind::Increment,
            SemanticInvocation::Decrement => nickel_ui::ActionKind::Decrement,
            SemanticInvocation::Expand => nickel_ui::ActionKind::Expand,
            SemanticInvocation::Collapse => nickel_ui::ActionKind::Collapse,
            SemanticInvocation::Select => nickel_ui::ActionKind::Select,
            SemanticInvocation::Dismiss => nickel_ui::ActionKind::Dismiss,
            SemanticInvocation::Scroll => nickel_ui::ActionKind::Scroll,
            SemanticInvocation::EnterNavigation => nickel_ui::ActionKind::EnterNavigation,
            SemanticInvocation::ExitNavigation => nickel_ui::ActionKind::ExitNavigation,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_semantic_policy_admits_only_guarded_windows_actions() {
        assert!(windows_semantic_action_has_guarded_disposition(
            crate::winit_shell::SurfaceRole::Launcher,
            nickel_ui::ActionKind::SetValue,
        ));
        assert!(windows_semantic_action_has_guarded_disposition(
            crate::winit_shell::SurfaceRole::VolumeOsd,
            nickel_ui::ActionKind::SetValue,
        ));
        assert!(!windows_semantic_action_has_guarded_disposition(
            crate::winit_shell::SurfaceRole::Launcher,
            nickel_ui::ActionKind::Activate,
        ));
        assert!(!windows_semantic_action_has_guarded_disposition(
            crate::winit_shell::SurfaceRole::ControlCenter,
            nickel_ui::ActionKind::SetValue,
        ));
    }

    #[test]
    fn launch_preparation_diagnostic_tracks_bounded_worker_transitions() {
        let state = LaunchPreparationState {
            started: Instant::now(),
            state: std::sync::Mutex::new(LaunchPreparationDiagnosticState::default()),
        };
        let idle = state.snapshot().unwrap();
        assert!(!idle.busy);
        assert_eq!(idle.generation, 0);

        state.begin().unwrap();
        let busy = state.snapshot().unwrap();
        assert!(busy.busy);
        assert_eq!(busy.generation, 1);
        assert!(state.begin().is_err());

        state.end();
        let released = state.snapshot().unwrap();
        assert!(!released.busy);
        assert_eq!(released.generation, 2);
        assert!(released.last_changed_uptime_us <= released.collector_uptime_us);
    }

    #[test]
    fn shell_behavior_projection_preserves_production_generations_and_counts() {
        let diagnostic = shell_behavior_diagnostic(31, 900, 17, true, (false, 6, 4));
        assert_eq!(diagnostic.observation_generation, 31);
        assert_eq!(diagnostic.observed_at_us, 900);
        assert_eq!(diagnostic.topology_generation, 17);
        assert!(diagnostic.bar_on_all_displays);
        assert!(!diagnostic.all_windows_on_every_bar);
        assert_eq!(diagnostic.configured_desktop_count, 6);
        assert_eq!(diagnostic.runtime_desktop_count, 4);
    }

    fn placement_test_window(
        native: usize,
        pid: u32,
        created: u64,
        title: &str,
    ) -> crate::windows_resource_owner::Window {
        crate::windows_resource_owner::Window {
            native,
            pid,
            created,
            thread: pid + 100,
            title: title.into(),
            label: "ordinary.exe".into(),
            bounds: crate::windows_resource_owner::Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            active: false,
            minimized: false,
            maximized: false,
            fullscreen: false,
            protected: false,
            application: None,
        }
    }

    #[test]
    fn rooted_launch_withholds_descendant_but_publishes_verified_unrelated_window() {
        use crate::platform::remote_observation::LaunchProcessAncestry;
        use nickel_remote_control::leases::ResourceScope;

        let existing = placement_test_window(1, 10, 100, "Existing");
        let unrelated = placement_test_window(2, 20, 200, "Unrelated");
        let descendant = placement_test_window(3, 30, 300, "Descendant");
        let baseline = std::collections::BTreeSet::from([WindowIncarnation::from(&existing)]);
        let mut before_root = vec![existing.clone(), unrelated.clone(), descendant.clone()];
        retain_windows_for_launch_placements(
            &mut before_root,
            &[(&baseline, None)],
            &Default::default(),
        );
        assert_eq!(before_root.as_slice(), std::slice::from_ref(&existing));

        let ancestry = std::collections::BTreeMap::from([
            (2, vec![LaunchProcessAncestry::Unrelated]),
            (3, vec![LaunchProcessAncestry::Descendant]),
        ]);
        let mut filtered = vec![existing.clone(), unrelated.clone(), descendant.clone()];
        retain_windows_for_launch_placements(&mut filtered, &[(&baseline, Some(0))], &ancestry);

        let mut resources = crate::windows_resource_owner::Owner::default();
        resources.reconcile(filtered, Vec::new(), |_| {}).unwrap();
        let published = resources
            .windows(&ResourceScope::FullSession)
            .map(|(window, _)| window)
            .collect::<Vec<_>>();
        assert_eq!(published.len(), 2);
        assert!(published.iter().any(|window| window.title == "Existing"));
        let unrelated_summary = published
            .iter()
            .find(|window| window.title == "Unrelated")
            .unwrap();
        assert!(
            resources
                .window_resource(
                    &ResourceScope::FullSession,
                    &unrelated_summary.id,
                    unrelated_summary.generation,
                )
                .is_some(),
            "the verified unrelated post-root window remains actionable"
        );
        assert!(!published.iter().any(|window| window.title == "Descendant"));

        resources
            .reconcile(vec![existing, unrelated, descendant], Vec::new(), |_| {})
            .unwrap();
        assert_eq!(resources.windows(&ResourceScope::FullSession).count(), 3);
    }

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
    #[test]
    fn timed_out_platform_refresh_retains_single_flight_until_worker_exits() {
        let worker = Arc::new(WindowsPlatformRefreshWorker::default());
        let (release, blocked) = mpsc::sync_channel(1);
        let result = run_windows_platform_refresh_worker(
            worker.clone(),
            Duration::from_millis(10),
            move || {
                blocked.recv().map_err(|_| "release stopped".to_owned())?;
                Ok(7_u8)
            },
        );
        assert_eq!(
            result.unwrap_err(),
            "Windows platform refresh exceeded its deadline"
        );
        assert_eq!(
            run_windows_platform_refresh_worker(worker.clone(), Duration::from_millis(10), || Ok(
                8_u8
            ))
            .unwrap_err(),
            "Windows platform refresh worker is already in progress"
        );
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !matches!(worker.snapshot(), Some(snapshot) if !snapshot.busy) {
            assert!(
                Instant::now() < deadline,
                "worker did not release admission"
            );
            std::thread::yield_now();
        }
        assert_eq!(
            run_windows_platform_refresh_worker(worker, Duration::from_secs(1), || Ok(9_u8))
                .unwrap(),
            9
        );
    }
    fn owner() -> WindowsRemoteControl {
        let (sender, receiver) = mpsc::sync_channel(16);
        let platform_refresh_worker = Arc::new(WindowsPlatformRefreshWorker::default());
        WindowsRemoteControl {
            _transport: None,
            receiver,
            remote_control: RemoteControlRuntime::default(),
            local_cues: Default::default(),
            applications: Default::default(),
            resources: Default::default(),
            workspaces: Default::default(),
            resource_lifecycle: crate::platform::remote_observation::Lifecycle::install().ok(),
            observation_generation: 0,
            platform_refresh_generation: 0,
            platform_refreshes: Vec::new(),
            platform_refresh_worker: platform_refresh_worker.clone(),
            remote_frame_trace: None,
            indicators: Default::default(),
            authority: Arc::new(WindowsDesktopAuthority {
                sender,
                cleanup_wake: nickel_remote_control::ConnectionCleanupWake::new(|| true),
                started: Instant::now(),
                desktop_session: None,
                capture_generation: std::sync::atomic::AtomicU64::new(0),
                peripheral_generation: std::sync::atomic::AtomicU64::new(0),
                platform_refresh_worker,
            }),
            desktop_session: None,
            desktop_unlocked: false,
            local_input_epoch: local_input_epoch(),
            keyboard_hold: None,
            pointer_hold: None,
            desktop_events: Default::default(),
            appearance: Default::default(),
            application_scale: Default::default(),
            file_icons: Default::default(),
            wallpaper: Default::default(),
            terminal_presentation: Default::default(),
            terminal_launch_policy: Default::default(),
            idle_preferences: Default::default(),
            launcher_favorites: Default::default(),
            shell_focus: None,
            external_accessibility: None,
            native_action_observations: Default::default(),
            pending_indicator_activation: Default::default(),
            pending_output_launches: Default::default(),
            next_output_launch: 0,
            start_time: Instant::now(),
            last_stop: None,
            stop_confirmation_until: None,
        }
    }
    #[test]
    fn windows_internal_application_inventory_is_supported_and_empty() {
        // Winit does not host ordinary application clients. Native Nickel
        // tools belong to the native window inventory and Winit chrome belongs
        // to the shell-surface inventory. In particular, this domain must not
        // become an alias that exposes the protected Codex surface.
        assert!(windows_internal_application_diagnostics().is_empty());
        assert!(
            !serde_json::to_string(&windows_unavailable_diagnostic_domains())
                .unwrap()
                .contains("windows_internal_applications")
        );
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
    fn shell_focus_events_coalesce_and_retain_only_fixed_identity() {
        let mut owner = owner();
        let launcher = ShellFocusState::Surface {
            generation: 41,
            role: nickel_remote_control::desktop_events::ShellEventRole::Launcher,
        };
        owner.record_shell_focus(launcher, 10);
        owner.record_shell_focus(launcher, 11);
        owner.record_shell_focus(ShellFocusState::Cleared, 12);
        owner.record_shell_focus(ShellFocusState::Cleared, 13);

        let snapshot = owner.desktop_events.snapshot();
        assert_eq!(snapshot.events.len(), 2);
        assert_eq!(snapshot.events[0].observed_at_us, 10);
        assert!(matches!(
            snapshot.events[0].event,
            nickel_remote_control::desktop_events::DesktopEventKind::ShellKeyboardFocusChanged {
                surface_generation: 41,
                role: nickel_remote_control::desktop_events::ShellEventRole::Launcher,
            }
        ));
        assert!(matches!(
            snapshot.events[1].event,
            nickel_remote_control::desktop_events::DesktopEventKind::KeyboardFocusCleared
        ));
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
        owner.poll_with_shell(None, None);
        assert_eq!(owner.remote_control.status().generation, 0);
        assert!(receiver.try_recv().is_err());
    }
}
