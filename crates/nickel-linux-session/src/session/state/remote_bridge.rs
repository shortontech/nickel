//! Bounded bridge from remote-control workers to the compositor owner.
//!
//! Requests and prepared results cross this channel; native state stays on the
//! compositor thread, which revalidates permits before applying operations.

use super::{
    native_key_worker, remote_appearance, remote_application_scale, remote_codex_preference,
    remote_device_settings, remote_file_icons, remote_idle_preferences, remote_keyboard_preference,
    remote_launch, remote_launcher_favorites, remote_peripheral_controls, remote_settings,
    remote_shell_actions, remote_terminal_presentation, remote_wallpaper, remote_worker,
};
use nickel_remote_control::DesktopPermit;
use nickel_session_protocol::ShellBehaviorTransaction;
use smithay::reexports::calloop::channel;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) enum RemoteDesktopRequest {
    ReadDeviceSettings {
        permit: DesktopPermit,
        observed: crate::platform::GuardedDeviceObservation,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::device_settings::Snapshot, String>,
        >,
    },
    BeginDeviceSettings {
        permit: DesktopPermit,
        transaction: nickel_remote_control::device_settings::Transaction,
        reply: std::sync::mpsc::SyncSender<Result<remote_device_settings::Prepared, String>>,
    },
    FinishDeviceSettings {
        permit: DesktopPermit,
        registration: u64,
        outcome: crate::platform::GuardedControlOutcome,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::device_settings::Outcome, String>,
        >,
    },
    ReadPeripheralControls {
        permit: DesktopPermit,
        observed: nickel_platform::PeripheralSnapshot,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::peripheral_controls::Snapshot, String>,
        >,
    },
    BeginPeripheralControl {
        permit: DesktopPermit,
        transaction: nickel_remote_control::peripheral_controls::Transaction,
        deadline: Instant,
        reply: std::sync::mpsc::SyncSender<Result<remote_peripheral_controls::Prepared, String>>,
    },
    FinishPeripheralControl {
        permit: DesktopPermit,
        prepared: remote_peripheral_controls::Prepared,
        outcome: nickel_platform::PeripheralOutcome,
        refreshed: Option<nickel_platform::PeripheralSnapshot>,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::peripheral_controls::Outcome, String>,
        >,
    },

    ApplicationScale {
        permit: nickel_remote_control::DesktopPermit,
        request: remote_application_scale::Request,
    },
    ClientConnection {
        permit: nickel_remote_control::ClientConnectionPermit,
        action: nickel_remote_control::ClientConnectionAction,
        reply: std::sync::mpsc::SyncSender<Result<(), String>>,
    },
    Events {
        permit: nickel_remote_control::DesktopPermit,
        after: u64,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::desktop_events::DesktopEventObservation, String>,
        >,
    },
    LaunchApplication {
        permit: nickel_remote_control::DesktopPermit,
        prepared: remote_launch::PreparedLaunch,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::diagnostics::LaunchApplicationOutcome, String>,
        >,
    },
    Applications {
        prepared: remote_launch::PreparedCatalog,
        permit: nickel_remote_control::DesktopPermit,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::diagnostics::ApplicationInventory, String>,
        >,
    },
    Outputs {
        permit: nickel_remote_control::DesktopPermit,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::diagnostics::OutputInventory, String>,
        >,
    },
    ReadDisplayLayout {
        permit: nickel_remote_control::DesktopPermit,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::display_layout::Snapshot, String>,
        >,
    },
    DisplayLayoutTransaction {
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::display_layout::Transaction,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::display_layout::Snapshot, String>,
        >,
    },
    WorkspaceAction {
        permit: nickel_remote_control::DesktopPermit,
        action: nickel_remote_control::diagnostics::WorkspaceAction,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::diagnostics::WorkspaceOutcome, String>,
        >,
    },
    ReadAppearance {
        permit: nickel_remote_control::DesktopPermit,
        prepared: remote_appearance::PreparedRead,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::appearance::Snapshot, String>,
        >,
    },
    AppearanceTransaction {
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::appearance::Transaction,
        prepared: remote_appearance::PreparedChange,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::appearance::Snapshot, String>,
        >,
    },
    ReadLauncherFavorites {
        permit: nickel_remote_control::DesktopPermit,
        prepared: remote_launcher_favorites::PreparedRead,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::launcher_favorites::Snapshot, String>,
        >,
    },
    LauncherFavoritesTransaction {
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::launcher_favorites::Transaction,
        prepared: remote_launcher_favorites::PreparedChange,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::launcher_favorites::Snapshot, String>,
        >,
    },
    ReadPreferredApplications {
        permit: nickel_remote_control::DesktopPermit,
        prepared: crate::remote_preferred_applications::PreparedRead,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::preferred_applications::Snapshot, String>,
        >,
    },
    PreferredApplicationsTransaction {
        permit: nickel_remote_control::DesktopPermit,
        prepared: crate::remote_preferred_applications::PreparedChange,
        deadline: Instant,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::preferred_applications::Snapshot, String>,
        >,
    },
    ReadWallpaper {
        permit: nickel_remote_control::DesktopPermit,
        prepared: remote_wallpaper::PreparedRead,
        reply:
            std::sync::mpsc::SyncSender<Result<nickel_remote_control::wallpaper::Snapshot, String>>,
    },
    WallpaperTransaction {
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::wallpaper::Transaction,
        prepared: remote_wallpaper::PreparedChange,
        reply:
            std::sync::mpsc::SyncSender<Result<nickel_remote_control::wallpaper::Snapshot, String>>,
    },
    ReadFileIcons {
        permit: nickel_remote_control::DesktopPermit,
        prepared: remote_file_icons::PreparedRead,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::file_icons::Snapshot, String>,
        >,
    },
    FileIconsTransaction {
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::file_icons::Transaction,
        prepared: remote_file_icons::PreparedChange,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::file_icons::Snapshot, String>,
        >,
    },
    ReadCodexPreference {
        permit: nickel_remote_control::DesktopPermit,
        prepared: remote_codex_preference::PreparedRead,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::codex_preference::Snapshot, String>,
        >,
    },
    CodexPreferenceTransaction {
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::codex_preference::Transaction,
        prepared: remote_codex_preference::PreparedChange,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::codex_preference::Snapshot, String>,
        >,
    },
    ReadIdlePreferences {
        permit: nickel_remote_control::DesktopPermit,
        prepared: remote_idle_preferences::PreparedRead,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::idle_preferences::Snapshot, String>,
        >,
    },
    IdlePreferencesTransaction {
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::idle_preferences::Transaction,
        prepared: remote_idle_preferences::PreparedChange,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::idle_preferences::Snapshot, String>,
        >,
    },
    ReadTerminalPresentation {
        permit: nickel_remote_control::DesktopPermit,
        prepared: remote_terminal_presentation::PreparedRead,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::terminal_presentation::Snapshot, String>,
        >,
    },
    TerminalPresentationTransaction {
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::terminal_presentation::Transaction,
        prepared: remote_terminal_presentation::PreparedChange,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::terminal_presentation::Snapshot, String>,
        >,
    },
    ReadTerminalLaunchPolicy {
        permit: nickel_remote_control::DesktopPermit,
        prepared: crate::remote_terminal_launch_policy::PreparedRead,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::terminal_launch_policy::Snapshot, String>,
        >,
    },
    TerminalLaunchPolicyTransaction {
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::terminal_launch_policy::Transaction,
        prepared: crate::remote_terminal_launch_policy::PreparedChange,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::terminal_launch_policy::TransactionOutcome, String>,
        >,
    },
    ReadKeyboardPreference {
        permit: nickel_remote_control::DesktopPermit,
        prepared: remote_keyboard_preference::PreparedRead,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::keyboard_preference::Snapshot, String>,
        >,
    },
    KeyboardPreferenceTransaction {
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::keyboard_preference::Transaction,
        prepared: remote_keyboard_preference::PreparedChange,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::keyboard_preference::Snapshot, String>,
        >,
    },
    ShellBehavior {
        permit: nickel_remote_control::DesktopPermit,
        transaction: ShellBehaviorTransaction,
        prepared: remote_settings::PreparedShellBehavior,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::diagnostics::ShellBehaviorDiagnostic, String>,
        >,
    },
    SemanticAction {
        permit: nickel_remote_control::DesktopPermit,
        request: nickel_remote_control::semantics::SemanticActionRequest,
        reply: std::sync::mpsc::SyncSender<Result<bool, String>>,
    },
    SurfaceSemanticAction {
        permit: nickel_remote_control::DesktopPermit,
        request: nickel_remote_control::semantics::SurfaceSemanticActionRequest,
        reply: std::sync::mpsc::SyncSender<Result<remote_shell_actions::ShellActionPlan, String>>,
    },
    ShellSemanticStep {
        permit: nickel_remote_control::DesktopPermit,
        origin: nickel_remote_control::leases::ResourceId,
        output: nickel_remote_control::leases::ResourceId,
        tree_generation: u64,
        prepared: remote_shell_actions::PreparedShellStep,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::semantics::SurfaceSemanticCompletion, String>,
        >,
    },
    NativeKeyboardState {
        source: smithay::input::keyboard::KeyboardSource,
        sequence: u64,
        result: Result<native_key_worker::NativeKeyObservation, String>,
    },
    DiagnosticAction {
        permit: nickel_remote_control::DesktopPermit,
        action: nickel_remote_control::diagnostics::DiagnosticAction,
        application_discovery: Option<PreparedApplicationDiscovery>,
        platform_refresh: Option<PreparedPlatformRefresh>,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::diagnostics::DiagnosticActionOutcome, String>,
        >,
    },
    ListSurfaces {
        permit: nickel_remote_control::DesktopPermit,
        reply: std::sync::mpsc::SyncSender<
            Result<Vec<nickel_remote_control::diagnostics::ShellSurfaceDiagnostic>, String>,
        >,
    },
    ValidateSurfaceCapture {
        permit: nickel_remote_control::DesktopPermit,
        id: String,
        generation: u64,
        reply: std::sync::mpsc::SyncSender<Result<(), String>>,
    },
    CaptureSurface {
        permit: nickel_remote_control::DesktopPermit,
        id: String,
        generation: u64,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::capture::CapturedWindow, String>,
        >,
    },
    ValidateOutputCapture {
        permit: nickel_remote_control::DesktopPermit,
        id: String,
        generation: u64,
        reply: std::sync::mpsc::SyncSender<Result<(), String>>,
    },
    CaptureOutput {
        permit: nickel_remote_control::DesktopPermit,
        id: String,
        generation: u64,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::capture::CapturedWindow, String>,
        >,
    },
    ValidateCapture {
        permit: nickel_remote_control::DesktopPermit,
        id: String,
        generation: u64,
        reply: std::sync::mpsc::SyncSender<Result<(), String>>,
    },
    Capture {
        permit: nickel_remote_control::DesktopPermit,
        id: String,
        generation: u64,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::capture::CapturedWindow, String>,
        >,
    },
    Keyboard {
        permit: nickel_remote_control::DesktopPermit,
        id: String,
        generation: u64,
        action: nickel_remote_control::keyboard::KeyboardAction,
        reply: std::sync::mpsc::SyncSender<Result<(), String>>,
    },
    Pointer {
        permit: nickel_remote_control::DesktopPermit,
        target: nickel_remote_control::pointer::PointerTarget,
        x: i32,
        y: i32,
        action: nickel_remote_control::pointer::PointerAction,
        reply: std::sync::mpsc::SyncSender<Result<(), String>>,
    },
    Semantics {
        permit: nickel_remote_control::DesktopPermit,
        id: String,
        generation: u64,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::semantics::SemanticSnapshot, String>,
        >,
    },
    PrepareNativeSemantics {
        permit: nickel_remote_control::DesktopPermit,
        id: String,
        generation: u64,
        application_root: bool,
        reply:
            std::sync::mpsc::SyncSender<Result<super::super::remote_accessibility::Proof, String>>,
    },
    ValidateNativeSemantics {
        permit: nickel_remote_control::DesktopPermit,
        proof: super::super::remote_accessibility::Proof,
        reply: std::sync::mpsc::SyncSender<Result<(), String>>,
    },
    FinishNativeSemantics {
        permit: nickel_remote_control::DesktopPermit,
        proof: super::super::remote_accessibility::Proof,
        result: super::super::remote_accessibility::Observation,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::native_semantics::NativeSemanticSnapshot, String>,
        >,
    },
    SurfaceSemantics {
        permit: nickel_remote_control::DesktopPermit,
        id: String,
        generation: u64,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::semantics::SurfaceSemanticSnapshot, String>,
        >,
    },
    Diagnostic {
        permit: nickel_remote_control::DesktopPermit,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::diagnostics::DiagnosticSnapshot, String>,
        >,
    },
    List {
        permit: nickel_remote_control::DesktopPermit,
        reply:
            std::sync::mpsc::SyncSender<Result<Vec<nickel_remote_control::WindowSummary>, String>>,
    },
    WindowAction {
        permit: nickel_remote_control::DesktopPermit,
        id: String,
        generation: u64,
        action: nickel_remote_control::window_actions::WindowAction,
        reply: std::sync::mpsc::SyncSender<
            Result<nickel_remote_control::window_actions::WindowOutcome, String>,
        >,
    },
}

pub(super) struct RemoteDesktopBridge {
    cleanup_wake: nickel_remote_control::ConnectionCleanupWake,
    sender: channel::SyncSender<RemoteDesktopRequest>,
    settings_staging: Arc<remote_settings::SettingsStaging>,
    diagnostic_staging: Arc<remote_worker::WorkerStaging>,
}

pub(super) struct PreparedApplicationDiscovery {
    pub(super) discovery: crate::model::ApplicationDiscovery,
    pub(super) observation_started: Instant,
    pub(super) observed: Instant,
    pub(super) preparation_duration_us: u64,
}

const DIAGNOSTIC_PREPARED_RESULT_MAX_AGE: Duration = Duration::from_secs(2);

pub(super) fn require_fresh_diagnostic_preparation(observed: Instant) -> Result<(), String> {
    if Instant::now().saturating_duration_since(observed) >= DIAGNOSTIC_PREPARED_RESULT_MAX_AGE {
        return Err("diagnostic preparation expired before owner commit".into());
    }
    Ok(())
}

pub(super) struct PreparedPlatformRefresh {
    pub(super) domain: nickel_remote_control::diagnostics::PlatformRefreshDomain,
    pub(super) data: PreparedPlatformRefreshData,
    pub(super) observation_started: Instant,
    pub(super) observed: Instant,
    pub(super) preparation_duration_us: u64,
}

pub(super) enum PreparedPlatformRefreshData {
    Connectivity(crate::platform::ConnectivityRefresh),
    Audio(crate::platform::AudioRefresh),
    Peripherals(crate::platform::PeripheralRefresh),
    Maintenance(crate::platform::MaintenanceRefresh),
    DefaultAssociations(crate::platform::DefaultAssociationsRefresh),
}

impl RemoteDesktopBridge {
    pub(super) fn enqueue(&self, request: RemoteDesktopRequest) -> Result<(), &'static str> {
        self.sender
            .try_send(request)
            .map_err(|_| "desktop request queue is busy or stopped")
    }

    pub(super) fn acquire_settings(&self) -> Result<remote_worker::StagingAdmission<'_>, String> {
        self.settings_staging.acquire()
    }

    pub(super) fn new(
        cleanup_wake: nickel_remote_control::ConnectionCleanupWake,
        sender: channel::SyncSender<RemoteDesktopRequest>,
        settings_staging: Arc<remote_settings::SettingsStaging>,
        diagnostic_staging: Arc<remote_worker::WorkerStaging>,
    ) -> Self {
        Self {
            cleanup_wake,
            sender,
            settings_staging,
            diagnostic_staging,
        }
    }

    fn inspect_native_accessibility(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
        application_root: bool,
    ) -> Result<nickel_remote_control::native_semantics::NativeSemanticSnapshot, String> {
        use super::super::remote_accessibility::{Admission, observe};
        let _admission = Admission::acquire()?;
        let deadline = Instant::now() + Duration::from_secs(1);
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::PrepareNativeSemantics {
                permit: permit.clone(),
                id: id.to_owned(),
                generation,
                application_root,
                reply,
            })
            .map_err(|_| "native accessibility queue is busy")?;
        let proof = response
            .recv_timeout(Duration::from_millis(100))
            .map_err(|_| "native accessibility owner timed out")??;
        let validate = || {
            proof.check_live(&permit)?;
            if Instant::now() >= deadline {
                return Err("native accessibility deadline elapsed".into());
            }
            let (reply, response) = std::sync::mpsc::sync_channel(1);
            self.sender
                .try_send(RemoteDesktopRequest::ValidateNativeSemantics {
                    permit: permit.clone(),
                    proof: proof.clone(),
                    reply,
                })
                .map_err(|_| "native accessibility validation queue is busy")?;
            response
                .recv_timeout(
                    Duration::from_millis(100)
                        .min(deadline.saturating_duration_since(Instant::now())),
                )
                .map_err(|_| "native accessibility validation timed out")?
        };
        let result = observe(&proof, &permit, deadline, validate)?;
        proof.check_live(&permit)?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::FinishNativeSemantics {
                permit,
                proof,
                result,
                reply,
            })
            .map_err(|_| "native accessibility completion queue is busy")?;
        response
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| "native accessibility completion timed out")?
    }
}

impl nickel_remote_control::DesktopAuthority for RemoteDesktopBridge {
    fn connection_cleanup_wake(&self) -> Option<nickel_remote_control::ConnectionCleanupWake> {
        Some(self.cleanup_wake.clone())
    }

    fn client_connection(
        &self,
        permit: nickel_remote_control::ClientConnectionPermit,
        action: nickel_remote_control::ClientConnectionAction,
    ) -> Result<(), String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ClientConnection {
                permit,
                action,
                reply,
            })
            .map_err(|_| "desktop connection queue is busy or stopped".to_owned())?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "desktop connection transition timed out".to_owned())?
    }

    fn read_desktop_events(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        after: u64,
    ) -> Result<nickel_remote_control::desktop_events::DesktopEventObservation, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::Events {
                permit,
                after,
                reply,
            })
            .map_err(|_| "desktop event queue is busy or stopped".to_owned())?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "desktop event observation timed out".to_owned())?
    }

    fn launch_installed_application(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        request: nickel_remote_control::diagnostics::LaunchApplicationRequest,
    ) -> Result<nickel_remote_control::diagnostics::LaunchApplicationOutcome, String> {
        // Called by the bounded blocking desktop adapter, never the owner loop.
        let prepared = remote_launch::PreparedLaunch::prepare(&permit, &request)?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::LaunchApplication {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "desktop launch queue is busy or stopped".to_owned())?;
        response.recv_timeout(Duration::from_secs(2))
            .map_err(|_| "launch result uncertain: desktop authority timed out; inspect windows before retrying".to_owned())?
    }

    fn list_installed_applications(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::diagnostics::ApplicationInventory, String> {
        let prepared = remote_launch::PreparedCatalog::prepare(&permit)?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::Applications {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "desktop application queue is busy or stopped".to_owned())?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "desktop application observation timed out".to_owned())?
    }

    fn list_outputs(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::diagnostics::OutputInventory, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::Outputs { permit, reply })
            .map_err(|_| "desktop output queue is busy or stopped".to_owned())?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "desktop output observation timed out".to_owned())?
    }
    fn read_display_layout(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::display_layout::Snapshot, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ReadDisplayLayout { permit, reply })
            .map_err(|_| "display layout queue is busy or stopped".to_owned())?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "display layout observation timed out".to_owned())?
    }
    fn display_layout_transaction(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::display_layout::Transaction,
    ) -> Result<nickel_remote_control::display_layout::Snapshot, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::DisplayLayoutTransaction {
                permit,
                transaction,
                reply,
            })
            .map_err(|_| "display layout queue is busy or stopped".to_owned())?;
        response.recv_timeout(Duration::from_secs(2)).map_err(|_| {
            "display layout result uncertain; read current layout before retrying".to_owned()
        })?
    }
    fn workspace_action(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        action: nickel_remote_control::diagnostics::WorkspaceAction,
    ) -> Result<nickel_remote_control::diagnostics::WorkspaceOutcome, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::WorkspaceAction {
                permit,
                action,
                reply,
            })
            .map_err(|_| "desktop workspace queue is busy or stopped".to_owned())?;
        response.recv_timeout(Duration::from_secs(2)).map_err(|_| "workspace result uncertain: desktop authority timed out; inspect state before retrying".to_owned())?
    }
    fn read_application_scale(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::application_scale::Snapshot, String> {
        remote_application_scale::read(self, permit)
    }
    fn application_scale_transaction(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::application_scale::Transaction,
    ) -> Result<nickel_remote_control::application_scale::TransactionOutcome, String> {
        remote_application_scale::transact(self, permit, transaction)
    }
    fn read_device_settings(
        &self,
        permit: DesktopPermit,
        domain: nickel_remote_control::device_settings::Domain,
    ) -> Result<nickel_remote_control::device_settings::Snapshot, String> {
        let observed = crate::platform::read_guarded_device(domain, &permit)?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ReadDeviceSettings {
                permit,
                observed,
                reply,
            })
            .map_err(|_| "device owner queue unavailable")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "device observation timed out")?
    }
    fn control_device_settings(
        &self,
        permit: DesktopPermit,
        transaction: nickel_remote_control::device_settings::Transaction,
    ) -> Result<nickel_remote_control::device_settings::Outcome, String> {
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::BeginDeviceSettings {
                permit: permit.clone(),
                transaction,
                reply,
            })
            .map_err(|_| "device owner queue unavailable")?;
        let prepared = response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "device admission timed out")??;
        let outcome = match crate::platform::submit_guarded_control(
            prepared.action,
            permit.clone(),
            prepared.ticket,
        ) {
            Ok(response) => response
                .recv_timeout(Duration::from_secs(2))
                .unwrap_or(crate::platform::GuardedControlOutcome::Uncertain),
            Err(_) => crate::platform::GuardedControlOutcome::Unavailable,
        };
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::FinishDeviceSettings {
                permit,
                registration: prepared.registration,
                outcome,
                reply,
            })
            .map_err(|_| "device result uncertain")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "device result uncertain")?
    }
    fn read_peripheral_controls(
        &self,
        permit: DesktopPermit,
    ) -> Result<nickel_remote_control::peripheral_controls::Snapshot, String> {
        let deadline = Instant::now() + Duration::from_millis(1900);
        permit.with_debug(false, || Ok(()))?;
        let cancellation_permit = permit.clone();
        let observed = nickel_platform::inspect_remote_peripherals(
            deadline,
            Arc::new(move || cancellation_permit.check_live().is_err()),
        )
        .map_err(|_| "bounded peripheral observation unavailable".to_owned())?;
        permit.check_live()?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ReadPeripheralControls {
                permit,
                observed,
                reply,
            })
            .map_err(|_| "peripheral owner queue unavailable")?;
        response
            .recv_timeout(
                deadline
                    .checked_duration_since(Instant::now())
                    .ok_or("peripheral observation expired")?,
            )
            .map_err(|_| "peripheral observation timed out")?
    }
    fn control_peripherals(
        &self,
        permit: DesktopPermit,
        transaction: nickel_remote_control::peripheral_controls::Transaction,
    ) -> Result<nickel_remote_control::peripheral_controls::Outcome, String> {
        let deadline = Instant::now() + Duration::from_millis(1900);
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::BeginPeripheralControl {
                permit: permit.clone(),
                transaction,
                deadline,
                reply,
            })
            .map_err(|_| "peripheral owner queue unavailable")?;
        let prepared = response
            .recv_timeout(
                deadline
                    .checked_duration_since(Instant::now())
                    .ok_or("peripheral control expired before admission")?,
            )
            .map_err(|_| "peripheral control admission timed out")??;
        let guard = prepared.held.clone();
        let (outcome, refreshed) = nickel_platform::control_remote_peripherals(
            prepared.native.action.clone(),
            deadline,
            Arc::new(move || Instant::now() >= deadline || guard.check_live().is_err()),
        )
        .unwrap_or((
            nickel_platform::PeripheralOutcome::Unsupported {
                detail: "guarded peripheral provider unavailable".into(),
            },
            None,
        ));
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::FinishPeripheralControl {
                permit,
                prepared,
                outcome,
                refreshed,
                reply,
            })
            .map_err(|_| "peripheral result uncertain")?;
        response
            .recv_timeout(
                deadline
                    .checked_duration_since(Instant::now())
                    .ok_or("peripheral result uncertain")?,
            )
            .map_err(|_| "peripheral result uncertain")?
    }
    fn read_appearance(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::appearance::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_appearance::PreparedRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ReadAppearance {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "appearance queue is busy or stopped")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "appearance observation timed out")?
    }
    fn appearance_transaction(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::appearance::Transaction,
    ) -> Result<nickel_remote_control::appearance::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_appearance::PreparedChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::AppearanceTransaction {
                permit,
                transaction,
                prepared,
                reply,
            })
            .map_err(|_| "appearance queue is busy or stopped")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "appearance result uncertain; read current appearance before retrying")?
    }
    fn read_default_association(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        target: nickel_remote_control::default_associations::Target,
    ) -> Result<nickel_remote_control::default_associations::Snapshot, String> {
        crate::remote_default_associations::read(permit, target)
    }
    fn default_association_transaction(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::default_associations::Transaction,
    ) -> Result<nickel_remote_control::default_associations::TransactionOutcome, String> {
        crate::remote_default_associations::transact(permit, transaction)
    }
    fn read_launcher_favorites(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::launcher_favorites::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_launcher_favorites::PreparedRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ReadLauncherFavorites {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "launcher_favorites queue is busy or stopped")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "launcher_favorites observation timed out")?
    }
    fn launcher_favorites_transaction(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::launcher_favorites::Transaction,
    ) -> Result<nickel_remote_control::launcher_favorites::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_launcher_favorites::PreparedChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::LauncherFavoritesTransaction {
                permit,
                transaction,
                prepared,
                reply,
            })
            .map_err(|_| "launcher_favorites queue is busy or stopped")?;
        response.recv_timeout(Duration::from_secs(2)).map_err(|_| {
            "launcher_favorites result uncertain; read current launcher_favorites before retrying"
        })?
    }
    fn read_preferred_applications(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::preferred_applications::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::remote_preferred_applications::PreparedRead::prepare(
            crate::remote_preferred_applications::production_catalog()?,
        )?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ReadPreferredApplications {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "preferred application queue is busy or stopped")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "preferred application observation timed out".to_owned())?
    }
    fn preferred_applications_transaction(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::preferred_applications::Transaction,
    ) -> Result<nickel_remote_control::preferred_applications::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        let deadline = Instant::now() + Duration::from_secs(2);
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::remote_preferred_applications::PreparedChange::prepare(
            crate::remote_preferred_applications::production_catalog()?,
            &transaction,
        )?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::PreferredApplicationsTransaction {
                permit,
                prepared,
                deadline,
                reply,
            })
            .map_err(|_| "preferred application queue is busy or stopped")?;
        response
            .recv_timeout(
                deadline
                    .checked_duration_since(Instant::now())
                    .ok_or("preferred application result uncertain")?,
            )
            .map_err(|_| "preferred application result uncertain".to_owned())?
    }
    fn read_wallpaper(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::wallpaper::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_wallpaper::PreparedRead::prepare_with_check(|| permit.check_live())?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ReadWallpaper {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "wallpaper queue is busy or stopped")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "wallpaper observation timed out")?
    }
    fn wallpaper_transaction(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::wallpaper::Transaction,
    ) -> Result<nickel_remote_control::wallpaper::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_wallpaper::PreparedChange::prepare_with_check(&transaction, || {
            permit.check_live()
        })?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::WallpaperTransaction {
                permit,
                transaction,
                prepared,
                reply,
            })
            .map_err(|_| "wallpaper queue is busy or stopped")?;
        response.recv_timeout(Duration::from_secs(2)).map_err(|_| {
            "wallpaper result uncertain; read current wallpaper before retrying".to_owned()
        })?
    }
    fn read_file_icons(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::file_icons::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_file_icons::PreparedRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ReadFileIcons {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "file icon settings queue is busy or stopped")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "file icon settings observation timed out")?
    }
    fn file_icons_transaction(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::file_icons::Transaction,
    ) -> Result<nickel_remote_control::file_icons::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_file_icons::PreparedChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::FileIconsTransaction {
                permit,
                transaction,
                prepared,
                reply,
            })
            .map_err(|_| "file icon settings queue is busy or stopped")?;
        response.recv_timeout(Duration::from_secs(2)).map_err(|_| {
            "file icon settings result uncertain; read current state before retrying".to_owned()
        })?
    }
    fn read_codex_preference(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::codex_preference::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_codex_preference::PreparedRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ReadCodexPreference {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "Codex preference queue is busy or stopped")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "Codex preference observation timed out")?
    }
    fn codex_preference_transaction(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::codex_preference::Transaction,
    ) -> Result<nickel_remote_control::codex_preference::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_codex_preference::PreparedChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::CodexPreferenceTransaction {
                permit,
                transaction,
                prepared,
                reply,
            })
            .map_err(|_| "Codex preference queue is busy or stopped")?;
        response.recv_timeout(Duration::from_secs(2)).map_err(|_| {
            "Codex preference result uncertain; read current state before retrying".to_owned()
        })?
    }
    fn read_idle_preferences(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::idle_preferences::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_idle_preferences::PreparedRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ReadIdlePreferences {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "idle preference queue is busy or stopped")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "idle preference observation timed out")?
    }
    fn idle_preferences_transaction(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::idle_preferences::Transaction,
    ) -> Result<nickel_remote_control::idle_preferences::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_idle_preferences::PreparedChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::IdlePreferencesTransaction {
                permit,
                transaction,
                prepared,
                reply,
            })
            .map_err(|_| "idle preference queue is busy or stopped")?;
        response.recv_timeout(Duration::from_secs(2)).map_err(|_| {
            "idle preference result uncertain; read current state before retrying".to_owned()
        })?
    }
    fn read_terminal_presentation(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::terminal_presentation::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_terminal_presentation::PreparedRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ReadTerminalPresentation {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "terminal presentation queue is busy or stopped")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "terminal presentation observation timed out")?
    }
    fn terminal_presentation_transaction(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::terminal_presentation::Transaction,
    ) -> Result<nickel_remote_control::terminal_presentation::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_terminal_presentation::PreparedChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::TerminalPresentationTransaction {
                permit,
                transaction,
                prepared,
                reply,
            })
            .map_err(|_| "terminal presentation queue is busy or stopped")?;
        response.recv_timeout(Duration::from_secs(2)).map_err(|_| {
            "terminal presentation result uncertain; read current state before retrying".to_owned()
        })?
    }
    fn read_terminal_launch_policy(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::terminal_launch_policy::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::remote_terminal_launch_policy::PreparedRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ReadTerminalLaunchPolicy {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "terminal launch-policy queue is busy or stopped")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "terminal launch-policy observation timed out")?
    }
    fn terminal_launch_policy_transaction(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::terminal_launch_policy::Transaction,
    ) -> Result<nickel_remote_control::terminal_launch_policy::TransactionOutcome, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = crate::remote_terminal_launch_policy::PreparedChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::TerminalLaunchPolicyTransaction {
                permit,
                transaction,
                prepared,
                reply,
            })
            .map_err(|_| "terminal launch-policy queue is busy or stopped")?;
        response.recv_timeout(Duration::from_secs(2)).map_err(|_| {
            "terminal launch-policy result uncertain; read current state before retrying".to_owned()
        })?
    }
    fn read_keyboard_preference(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::keyboard_preference::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_keyboard_preference::PreparedRead::prepare()?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ReadKeyboardPreference {
                permit,
                prepared,
                reply,
            })
            .map_err(|_| "keyboard preference queue is busy or stopped")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "keyboard preference observation timed out")?
    }
    fn keyboard_preference_transaction(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        transaction: nickel_remote_control::keyboard_preference::Transaction,
    ) -> Result<nickel_remote_control::keyboard_preference::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_keyboard_preference::PreparedChange::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::KeyboardPreferenceTransaction {
                permit,
                transaction,
                prepared,
                reply,
            })
            .map_err(|_| "keyboard preference queue is busy or stopped")?;
        response.recv_timeout(Duration::from_secs(2)).map_err(|_| {
            "keyboard preference result uncertain; read current state before retrying".to_owned()
        })?
    }
    fn shell_behavior_transaction(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        transaction: ShellBehaviorTransaction,
    ) -> Result<nickel_remote_control::diagnostics::ShellBehaviorDiagnostic, String> {
        // This method runs in desktop_call's blocking worker, never on the
        // compositor. Hold admission until the worker and reply wait finish,
        // even if the network caller drops its future during filesystem I/O.
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_settings::PreparedShellBehavior::prepare(&transaction)?;
        permit.with_debug(false, || Ok(()))?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ShellBehavior {
                permit,
                transaction,
                prepared,
                reply,
            })
            .map_err(|_| "desktop settings queue is busy or stopped".to_owned())?;
        response.recv_timeout(Duration::from_secs(2)).map_err(|_| {
            "settings transaction result uncertain: desktop authority timed out; inspect current state before retrying".to_owned()
        })?
    }

    fn semantic_action(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        request: nickel_remote_control::semantics::SemanticActionRequest,
    ) -> Result<bool, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::SemanticAction {
                permit,
                request,
                reply,
            })
            .map_err(|_| "desktop request queue is unavailable".to_owned())?;
        response.recv_timeout(Duration::from_secs(2)).map_err(|_| {
            "semantic action result uncertain: desktop authority timed out; do not retry".to_owned()
        })?
    }

    fn surface_semantic_action(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        request: nickel_remote_control::semantics::SurfaceSemanticActionRequest,
    ) -> Result<nickel_remote_control::semantics::SurfaceSemanticActionOutcome, String> {
        let lease_id = request.lease_id;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::SurfaceSemanticAction {
                permit: permit.clone(),
                request,
                reply,
            })
            .map_err(|_| "desktop request queue is unavailable".to_owned())?;
        let plan = response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| {
                "semantic action result uncertain: desktop authority timed out; do not retry"
                    .to_owned()
            })??;
        self.finish_shell_action(permit, lease_id, plan)
    }

    fn diagnostic_action(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        action: nickel_remote_control::diagnostics::DiagnosticAction,
    ) -> Result<nickel_remote_control::diagnostics::DiagnosticActionOutcome, String> {
        let refresh_applications = matches!(
            action,
            nickel_remote_control::diagnostics::DiagnosticAction::RefreshApplicationInventory
        );
        let platform_domain = match &action {
            nickel_remote_control::diagnostics::DiagnosticAction::RefreshPlatformStatus {
                domain,
            } => Some(*domain),
            _ => None,
        };
        let _diagnostic_admission = (refresh_applications || platform_domain.is_some())
            .then(|| self.diagnostic_staging.acquire())
            .transpose()?;
        let application_discovery = if refresh_applications {
            permit.with_debug(false, || Ok(()))?;
            let started = Instant::now();
            let discovery = crate::platform::prepare_application_discovery();
            let preparation_duration_us =
                started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
            let observed = Instant::now();
            permit.with_debug(false, || Ok(()))?;
            Some(PreparedApplicationDiscovery {
                discovery,
                observation_started: started,
                observed,
                preparation_duration_us,
            })
        } else {
            None
        };
        let platform_refresh = if let Some(domain) = platform_domain {
            permit.with_debug(false, || Ok(()))?;
            let started = Instant::now();
            let data = match domain {
                nickel_remote_control::diagnostics::PlatformRefreshDomain::Connectivity => {
                    PreparedPlatformRefreshData::Connectivity(
                        crate::platform::refresh_connectivity_status()?,
                    )
                }
                nickel_remote_control::diagnostics::PlatformRefreshDomain::Audio => {
                    PreparedPlatformRefreshData::Audio(crate::platform::refresh_audio_status()?)
                }
                nickel_remote_control::diagnostics::PlatformRefreshDomain::Peripherals => {
                    PreparedPlatformRefreshData::Peripherals(
                        crate::platform::refresh_peripheral_status()?,
                    )
                }
                nickel_remote_control::diagnostics::PlatformRefreshDomain::Maintenance => {
                    PreparedPlatformRefreshData::Maintenance(
                        crate::platform::refresh_maintenance_status()?,
                    )
                }
                nickel_remote_control::diagnostics::PlatformRefreshDomain::DefaultAssociations => {
                    PreparedPlatformRefreshData::DefaultAssociations(
                        crate::platform::refresh_default_associations()?,
                    )
                }
            };
            let preparation_duration_us =
                started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
            let observed = Instant::now();
            permit.with_debug(false, || Ok(()))?;
            Some(PreparedPlatformRefresh {
                domain,
                data,
                observation_started: started,
                observed,
                preparation_duration_us,
            })
        } else {
            None
        };
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::DiagnosticAction {
                permit,
                action,
                application_discovery,
                platform_refresh,
                reply,
            })
            .map_err(|_| "desktop diagnostic queue is busy or stopped".to_owned())?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "desktop authority timed out".to_owned())?
    }
    fn list_surfaces(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<Vec<nickel_remote_control::diagnostics::ShellSurfaceDiagnostic>, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ListSurfaces { permit, reply })
            .map_err(|_| "surface inventory queue is busy")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "surface inventory timed out".to_owned())?
    }

    fn validate_surface_capture(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<(), String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ValidateSurfaceCapture {
                permit,
                id: id.into(),
                generation,
                reply,
            })
            .map_err(|_| "desktop capture validation queue is busy or unavailable")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "capture validation timed out".to_owned())?
    }

    fn capture_surface(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<nickel_remote_control::capture::CapturedWindow, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::CaptureSurface {
                permit,
                id: id.into(),
                generation,
                reply,
            })
            .map_err(|_| "desktop capture queue is busy or unavailable")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "desktop capture timed out".to_owned())?
    }

    fn validate_window_capture(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<(), String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ValidateCapture {
                permit,
                id: id.into(),
                generation,
                reply,
            })
            .map_err(|_| "desktop capture validation queue is busy or unavailable")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "capture validation timed out".to_owned())?
    }

    fn validate_output_capture(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<(), String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::ValidateOutputCapture {
                permit,
                id: id.into(),
                generation,
                reply,
            })
            .map_err(|_| "output capture validation queue is busy or unavailable")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "output capture validation timed out".to_owned())?
    }

    fn capture_output(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<nickel_remote_control::capture::CapturedWindow, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::CaptureOutput {
                permit,
                id: id.into(),
                generation,
                reply,
            })
            .map_err(|_| "output capture queue is busy or unavailable")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "output capture timed out".to_owned())?
    }

    fn capture_window(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<nickel_remote_control::capture::CapturedWindow, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::Capture {
                permit,
                id: id.into(),
                generation,
                reply,
            })
            .map_err(|_| "desktop capture queue is busy or unavailable")?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "desktop capture timed out".to_owned())?
    }

    fn keyboard_action(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
        action: nickel_remote_control::keyboard::KeyboardAction,
    ) -> Result<(), String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::Keyboard {
                permit,
                id: id.to_owned(),
                generation,
                action,
                reply,
            })
            .map_err(|error| match error {
                std::sync::mpsc::TrySendError::Full(_) => {
                    "desktop request queue is busy".to_owned()
                }
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    "desktop authority stopped".to_owned()
                }
            })?;
        response
            // Delivery authority expires after two seconds. Allow the owner
            // thread's bounded cancellation timer to release input and report
            // partial progress before the HTTP handler gives up waiting. This
            // grace period does not extend the permit or allow more input.
            .recv_timeout(Duration::from_millis(2250))
            .map_err(|_| {
                "keyboard outcome unavailable; input may have been partially delivered; do not automatically retry"
                    .to_owned()
            })?
    }
    fn pointer_action(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        target: nickel_remote_control::pointer::PointerTarget,
        x: i32,
        y: i32,
        action: nickel_remote_control::pointer::PointerAction,
    ) -> Result<(), String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::Pointer {
                permit,
                target,
                x,
                y,
                action,
                reply,
            })
            .map_err(|error| match error {
                std::sync::mpsc::TrySendError::Full(_) => {
                    "desktop request queue is busy".to_owned()
                }
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    "desktop authority stopped".to_owned()
                }
            })?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "desktop authority timed out".to_owned())?
    }
    fn inspect_window(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<nickel_remote_control::semantics::SemanticSnapshot, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::Semantics {
                permit,
                id: id.to_owned(),
                generation,
                reply,
            })
            .map_err(|_| "desktop request queue is unavailable".to_owned())?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "desktop authority timed out".to_owned())?
    }
    fn inspect_native_window(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<nickel_remote_control::native_semantics::NativeSemanticSnapshot, String> {
        self.inspect_native_accessibility(permit, id, generation, false)
    }
    fn inspect_native_application(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<nickel_remote_control::native_semantics::NativeSemanticSnapshot, String> {
        self.inspect_native_accessibility(permit, id, generation, true)
    }

    fn inspect_surface(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<nickel_remote_control::semantics::SurfaceSemanticSnapshot, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::SurfaceSemantics {
                permit,
                id: id.to_owned(),
                generation,
                reply,
            })
            .map_err(|_| "desktop request queue is unavailable".to_owned())?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "desktop authority timed out".to_owned())?
    }

    fn diagnostic_snapshot(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::diagnostics::DiagnosticSnapshot, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::Diagnostic { permit, reply })
            .map_err(|error| match error {
                std::sync::mpsc::TrySendError::Full(_) => {
                    "desktop request queue is busy".to_owned()
                }
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    "desktop authority stopped".to_owned()
                }
            })?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "desktop authority timed out".to_owned())?
    }
    fn list_windows(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<Vec<nickel_remote_control::WindowSummary>, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::List { permit, reply })
            .map_err(|error| match error {
                std::sync::mpsc::TrySendError::Full(_) => {
                    "desktop request queue is busy".to_owned()
                }
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    "desktop authority stopped".to_owned()
                }
            })?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "desktop authority timed out".to_owned())?
    }

    fn window_action(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
        action: nickel_remote_control::window_actions::WindowAction,
    ) -> Result<nickel_remote_control::window_actions::WindowOutcome, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::WindowAction {
                permit,
                id: id.to_owned(),
                generation,
                action,
                reply,
            })
            .map_err(|error| match error {
                std::sync::mpsc::TrySendError::Full(_) => {
                    "desktop request queue is busy".to_owned()
                }
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    "desktop authority stopped".to_owned()
                }
            })?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "desktop authority timed out".to_owned())?
    }
}
