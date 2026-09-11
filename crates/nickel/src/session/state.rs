use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    hash::Hash,
    os::fd::AsFd,
    os::fd::AsRawFd,
    os::unix::net::UnixDatagram,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

/// A stalled local subscriber must never block compositor input or revocation.
/// Existing send-error handling retires subscribers whose bounded queue is full.
fn notification_socket() -> std::io::Result<UnixDatagram> {
    let socket = UnixDatagram::unbound()?;
    socket.set_nonblocking(true)?;
    Ok(socket)
}

enum RemoteDesktopRequest {
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
        id: String,
        generation: u64,
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
        reply: std::sync::mpsc::SyncSender<Result<super::remote_accessibility::Proof, String>>,
    },
    ValidateNativeSemantics {
        permit: nickel_remote_control::DesktopPermit,
        proof: super::remote_accessibility::Proof,
        reply: std::sync::mpsc::SyncSender<Result<(), String>>,
    },
    FinishNativeSemantics {
        permit: nickel_remote_control::DesktopPermit,
        proof: super::remote_accessibility::Proof,
        result: super::remote_accessibility::Observation,
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

struct RemoteDesktopBridge {
    cleanup_wake: nickel_remote_control::ConnectionCleanupWake,
    sender: channel::SyncSender<RemoteDesktopRequest>,
    settings_staging: Arc<remote_settings::SettingsStaging>,
    diagnostic_staging: Arc<remote_worker::WorkerStaging>,
}

struct PreparedApplicationDiscovery {
    discovery: crate::model::ApplicationDiscovery,
    observation_started: Instant,
    observed: Instant,
    preparation_duration_us: u64,
}

struct PreparedPlatformRefresh {
    domain: nickel_remote_control::diagnostics::PlatformRefreshDomain,
    data: PreparedPlatformRefreshData,
    observation_started: Instant,
    observed: Instant,
    preparation_duration_us: u64,
}

enum PreparedPlatformRefreshData {
    Connectivity(crate::platform::ConnectivityRefresh),
    Audio(crate::platform::AudioRefresh),
    Peripherals(crate::platform::PeripheralRefresh),
    Maintenance(crate::platform::MaintenanceRefresh),
    DefaultAssociations(crate::platform::DefaultAssociationsRefresh),
}

impl RemoteDesktopBridge {
    fn inspect_native_accessibility(
        &self,
        permit: nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
        application_root: bool,
    ) -> Result<nickel_remote_control::native_semantics::NativeSemanticSnapshot, String> {
        use super::remote_accessibility::{Admission, observe};
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
    fn read_wallpaper(
        &self,
        permit: nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::wallpaper::Snapshot, String> {
        let _staging = self.settings_staging.acquire()?;
        permit.with_debug(false, || Ok(()))?;
        let prepared = remote_wallpaper::PreparedRead::prepare()?;
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
        let prepared = remote_wallpaper::PreparedChange::prepare(&transaction)?;
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
        id: &str,
        generation: u64,
        x: i32,
        y: i32,
        action: nickel_remote_control::pointer::PointerAction,
    ) -> Result<(), String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(RemoteDesktopRequest::Pointer {
                permit,
                id: id.to_owned(),
                generation,
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

use nickel_core::{
    active_output::{
        ActiveOutputContext, InvocationSource, resolve_active_output, resolve_new_window_output,
    },
    focus::FocusTransactions,
    hotkeys::{CompositorShortcutAdapter, HotkeyAction},
    idle::{IdleController, IdleEffect, IdlePolicy},
    launcher::{LauncherPointerTarget, LauncherVisibility},
    shell_settings::ShellSettings,
    task_switcher::{SwitchWindow, TaskSwitchEffect, TaskSwitcher},
    workspaces::{WorkspaceError, WorkspaceId, WorkspaceTransition, Workspaces},
};
use nickel_session_protocol::{
    ClientEnvelope, Command as SessionCommand, ErrorCode, Event as SessionEvent,
    Geometry as ProtocolGeometry, OutputSnapshot, OutputTransform, PreviewFrame as ProtocolPreview,
    Query, Request, SecureStorageState as ProtocolSecureStorage, ServerEnvelope, ServerMessage,
    ShellBehaviorSetting, ShellBehaviorSnapshot, ShellBehaviorTransaction, ShellBehaviorValue,
    ShellPopoverAnchor, ShellRole, ShellSurfaceIdentity, ShellSurfaceSnapshot,
    Snapshot as SessionSnapshot, TestOutput, WindowAction as ProtocolWindowAction,
    WindowId as ProtocolWindowId, WindowSnapshot, WorkspaceId as ProtocolWorkspaceId,
    WorkspaceSnapshot, WorkspaceState, decode, encode,
};
use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType, find_popup_root_surface},
    input::{Seat, SeatState},
    output::{Mode as OutputMode, Output, PhysicalProperties, Scale as OutputScale, Subpixel},
    reexports::{
        calloop::{
            EventLoop, Interest, LoopSignal, Mode, PostAction, channel,
            generic::Generic,
            timer::{TimeoutAction, Timer},
        },
        wayland_server::{
            Display, DisplayHandle, Resource,
            backend::{ClientData, ClientId, DisconnectReason, GlobalId, ObjectId},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{IsAlive, Logical, Point, Rectangle, SERIAL_COUNTER, Size, Transform},
    wayland::{
        compositor::{
            CompositorClientState, CompositorState, get_parent, send_surface_state, with_states,
        },
        fractional_scale::{FractionalScaleManagerState, with_fractional_scale},
        idle_inhibit::IdleInhibitManagerState,
        image_capture_source::{ImageCaptureSourceState, OutputCaptureSourceState},
        image_copy_capture::{ImageCopyCaptureState, Session},
        input_method::InputMethodManagerState,
        output::OutputManagerState,
        pointer_constraints::PointerConstraintsState,
        relative_pointer::RelativePointerManagerState,
        seat::WaylandFocus,
        selection::{data_device::DataDeviceState, primary_selection::PrimarySelectionState},
        shell::xdg::{
            ToplevelSurface, XdgShellState, decoration::XdgDecorationState, dialog::XdgDialogState,
        },
        shm::ShmState,
        socket::ListeningSocketSource,
        viewporter::ViewporterState,
        xdg_activation::XdgActivationState,
        xwayland_shell::XWaylandShellState,
    },
    xwayland::{X11Wm, xwm::XwmId},
};

/// Nickel-owned overlay color used to dim shell content without painting a
/// pure-black translucent background.
pub(crate) const fn shell_scrim(alpha: f32) -> [f32; 4] {
    [0.035, 0.043, 0.055, alpha]
}

#[cfg(test)]
mod internal_shell_placement_tests {
    use super::{
        avoid_trusted_control_collision, internal_codex_chat_placement,
        internal_codex_project_menu_placement, internal_shell_surface_placement,
    };
    use crate::{internal_shell::InternalOutput, winit_shell::SurfaceRole};
    use nickel_session_protocol::{AnchorSide, Geometry, ShellPopoverAnchor};

    fn outputs() -> Vec<(InternalOutput, i32, i32)> {
        vec![
            (
                InternalOutput {
                    x: -1920,
                    y: -120,
                    name: "left".into(),
                    width: 1920,
                    height: 1080,
                    scale: 1.0,
                },
                -1920,
                -120,
            ),
            (
                InternalOutput {
                    x: 0,
                    y: 240,
                    name: "right".into(),
                    width: 2560,
                    height: 1440,
                    scale: 1.0,
                },
                0,
                240,
            ),
        ]
    }

    #[test]
    fn launcher_uses_active_output_global_origin() {
        let placement = internal_shell_surface_placement(
            SurfaceRole::Launcher,
            None,
            (960, 720),
            &outputs(),
            Some("right"),
        );

        assert_eq!(placement.output.as_deref(), Some("right"));
        assert_eq!(placement.geometry, (18, 896, 960, 720));
    }

    #[test]
    fn context_menu_moves_away_from_trusted_control_without_leaving_output() {
        assert_eq!(
            avoid_trusted_control_collision(
                (760, 113, 220, 282),
                (0, 0, 1280, 800),
                &[(788, 12, 480, 210)],
            ),
            (560, 113, 220, 282)
        );
        assert_eq!(
            avoid_trusted_control_collision(
                (10, 10, 300, 250),
                (0, 0, 640, 480),
                &[(0, 0, 320, 200)],
            ),
            (10, 208, 300, 250)
        );
    }

    #[test]
    fn native_keyboard_uses_authority_height_dock_and_output_without_rescaling() {
        let mut outputs = outputs();
        outputs[0].0.scale = 1.5;
        for (top, y) in [(true, -120), (false, 592)] {
            let placement =
                super::internal_keyboard_surface_placement(Some("left"), top, 368, &outputs)
                    .unwrap();
            assert_eq!(placement.geometry, (-1920, y, 1920, 368));
            assert_eq!(placement.output.as_deref(), Some("left"));
        }
        let resized =
            super::internal_keyboard_surface_placement(Some("right"), false, 280, &outputs)
                .unwrap();
        assert_eq!(resized.geometry, (0, 1400, 2560, 280));
        assert!(
            super::internal_keyboard_surface_placement(Some("removed"), false, 368, &outputs)
                .is_none()
        );
        assert!(super::internal_keyboard_surface_placement(None, false, 368, &[]).is_none());
    }

    #[test]
    fn volume_osd_uses_requested_interaction_output_without_launcher_affinity() {
        let placement = internal_shell_surface_placement(
            SurfaceRole::VolumeOsd,
            Some("right"),
            (320, 88),
            &outputs(),
            Some("left"),
        );
        assert_eq!(placement.output.as_deref(), Some("right"));
        assert_eq!(placement.geometry, (0, 240, 320, 88));
        let fallback = internal_shell_surface_placement(
            SurfaceRole::VolumeOsd,
            Some("removed"),
            (320, 88),
            &outputs(),
            None,
        );
        assert_eq!(fallback.output.as_deref(), Some("left"));
    }

    #[test]
    fn switching_active_output_relocates_one_launcher_to_negative_origin() {
        let right = internal_shell_surface_placement(
            SurfaceRole::Launcher,
            None,
            (960, 720),
            &outputs(),
            Some("right"),
        );
        let left = internal_shell_surface_placement(
            SurfaceRole::Launcher,
            None,
            (960, 720),
            &outputs(),
            Some("left"),
        );

        assert_eq!(right.output.as_deref(), Some("right"));
        assert_eq!(left.output.as_deref(), Some("left"));
        assert_eq!(left.geometry, (-1902, 176, 960, 720));
        assert_ne!(right.geometry, left.geometry);
    }

    #[test]
    fn codex_menu_uses_clicked_panel_output_global_coordinates() {
        let anchor = ShellPopoverAnchor {
            control: "panel-codex".into(),
            output: "right".into(),
            bounds: Geometry {
                x: 2200,
                y: 1392,
                width: 48,
                height: 48,
            },
            preferred: AnchorSide::Above,
        };

        let placement =
            internal_codex_project_menu_placement(Some(&anchor), &outputs(), Some("left"));

        assert_eq!(placement.output.as_deref(), Some("right"));
        assert_eq!(placement.origin, (1964, 936));
        assert_eq!(placement.scale, 1.0);
    }

    #[test]
    fn codex_menu_fallback_includes_negative_output_origin() {
        let placement = internal_codex_project_menu_placement(None, &outputs(), Some("left"));

        assert_eq!(placement.output.as_deref(), Some("left"));
        assert_eq!(placement.origin, (-1920, 216));
    }

    #[test]
    fn codex_chat_frame_is_centered_inside_nonzero_output_work_area() {
        let placement = internal_codex_chat_placement(&outputs(), Some("right"));

        assert_eq!(placement.output.as_deref(), Some("right"));
        assert_eq!(placement.origin, (720, 572));
        assert_eq!(placement.scale, 1.0);
        let outer =
            crate::session::window_frame::outer_geometry(crate::session::shell_layout::Geometry {
                x: placement.origin.0,
                y: placement.origin.1,
                width: crate::internal_codex::CHAT_SIZE.0 as i32,
                height: crate::internal_codex::CHAT_SIZE.1 as i32,
            });
        assert!(outer.x >= 0);
        assert!(outer.y >= 240);
        assert!(outer.x + outer.width <= 2560);
        assert!(outer.y + outer.height <= 240 + 1440 - crate::winit_shell::PANEL_HEIGHT as i32);
    }

    #[test]
    fn codex_chat_frame_preserves_negative_output_origin() {
        let placement = internal_codex_chat_placement(&outputs(), Some("left"));

        assert_eq!(placement.output.as_deref(), Some("left"));
        assert_eq!(placement.origin, (-1520, 32));
        let outer =
            crate::session::window_frame::outer_geometry(crate::session::shell_layout::Geometry {
                x: placement.origin.0,
                y: placement.origin.1,
                width: crate::internal_codex::CHAT_SIZE.0 as i32,
                height: crate::internal_codex::CHAT_SIZE.1 as i32,
            });
        assert!(outer.x >= -1920);
        assert!(outer.y >= -120);
        assert!(outer.x + outer.width <= 0);
        assert!(outer.y + outer.height <= -120 + 1080 - crate::winit_shell::PANEL_HEIGHT as i32);
    }
}

use crate::session::{
    output_retirement::{DeferredRetirements, RetirementAction, capacity_available},
    shell_layout::{self, Geometry},
    window_registry::{WindowAdmission, WindowId, WindowMetadataSource, WindowRegistry},
};

fn stable_output_identity(output: &Output) -> String {
    let physical = output.physical_properties();
    let hardware = format!(
        "{}|{}|{}|{}x{}",
        physical.make, physical.model, physical.serial_number, physical.size.w, physical.size.h
    );
    if physical.serial_number.is_empty() {
        format!("{hardware}|{}", output.name())
    } else {
        hardware
    }
}

fn protocol_error(code: ErrorCode, message: impl Into<String>) -> ServerMessage {
    ServerMessage::Error {
        code,
        message: message.into(),
    }
}

fn shell_behavior_value(
    settings: &ShellSettings,
    setting: ShellBehaviorSetting,
) -> ShellBehaviorValue {
    match setting {
        ShellBehaviorSetting::BarDisplayScope => {
            ShellBehaviorValue::Toggle(settings.bar_on_all_displays)
        }
        ShellBehaviorSetting::BarWindowScope => {
            ShellBehaviorValue::Toggle(settings.all_windows_on_every_bar)
        }
        ShellBehaviorSetting::DesktopCount => ShellBehaviorValue::Count(settings.desktop_count),
    }
}

fn apply_shell_behavior_value(
    settings: &mut ShellSettings,
    setting: ShellBehaviorSetting,
    value: ShellBehaviorValue,
) -> Result<(), &'static str> {
    match (setting, value) {
        (ShellBehaviorSetting::BarDisplayScope, ShellBehaviorValue::Toggle(value)) => {
            settings.bar_on_all_displays = value;
        }
        (ShellBehaviorSetting::BarWindowScope, ShellBehaviorValue::Toggle(value)) => {
            settings.all_windows_on_every_bar = value;
        }
        (ShellBehaviorSetting::DesktopCount, ShellBehaviorValue::Count(value))
            if (1..=nickel_core::shell_settings::MAX_CONFIGURED_WORKSPACES).contains(&value) =>
        {
            settings.desktop_count = value;
        }
        (ShellBehaviorSetting::DesktopCount, ShellBehaviorValue::Count(_)) => {
            return Err("desktop count is outside the supported range");
        }
        _ => return Err("setting and value types do not match"),
    }
    Ok(())
}

fn prepare_shell_behavior_update(
    current: &ShellSettings,
    topology_generation: u64,
    transaction: &ShellBehaviorTransaction,
) -> Result<ShellSettings, &'static str> {
    if transaction.topology_generation != topology_generation {
        return Err("stale output topology generation");
    }
    if transaction.prior != shell_behavior_value(current, transaction.setting) {
        return Err("shell setting changed before this transaction was applied");
    }
    let mut requested = current.clone();
    apply_shell_behavior_value(&mut requested, transaction.setting, transaction.requested)?;
    Ok(requested)
}

fn retain_live_idle_inhibitors<K: Eq + std::hash::Hash>(
    inhibitors: &mut HashMap<K, usize>,
    mut is_alive: impl FnMut(&K) -> bool,
) {
    inhibitors.retain(|surface, _| is_alive(surface));
}

fn identification_expiry_is_current(current_generation: u64, scheduled_generation: u64) -> bool {
    current_generation == scheduled_generation
}

fn workspace_error(error: WorkspaceError) -> &'static str {
    match error {
        WorkspaceError::UnknownWorkspace => "unknown workspace",
        WorkspaceError::LastWorkspace => "cannot remove the last workspace",
        WorkspaceError::LimitReached => "workspace limit reached",
        WorkspaceError::UnknownWindow => "window has no workspace",
    }
}

fn clamp_window_location(
    location: Point<i32, Logical>,
    size: Size<i32, Logical>,
    work_area: Geometry,
) -> Point<i32, Logical> {
    (
        location
            .x
            .clamp(work_area.x, work_area.x + (work_area.width - size.w).max(0)),
        location.y.clamp(
            work_area.y,
            work_area.y + (work_area.height - size.h).max(0),
        ),
    )
        .into()
}

pub(crate) fn drag_icon_location(
    pointer: Point<f64, Logical>,
    output: Rectangle<i32, Logical>,
) -> Option<Point<i32, Logical>> {
    output
        .to_f64()
        .contains(pointer)
        .then(|| (pointer - output.loc.to_f64()).to_i32_round())
}

fn output_contains_logical_point(output: Rectangle<i32, Logical>, x: i32, y: i32) -> bool {
    output.contains(Point::from((x, y)))
}

fn process_uid(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("Uid:\t"))
        .and_then(|value| value.split_whitespace().next())
        .map(str::to_owned)
}

fn same_session_user(pid: u32) -> bool {
    process_uid(pid).is_some_and(|uid| process_uid(std::process::id()).as_deref() == Some(&uid))
}

#[derive(Clone, Debug)]
struct PendingLaunchObservation {
    generation: u64,
    root_pid: u32,
    root_start_time: u64,
    registered_at: Instant,
    deadline: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RemoteWindowEventState {
    geometry: Option<[i32; 4]>,
    workspace: u64,
    active: bool,
    minimized: bool,
    maximized: bool,
    fullscreen: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RemoteOutputEventState {
    geometry: [i32; 4],
    work_area: [i32; 4],
    scale_120: u32,
    primary: bool,
    enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RemoteFocusEventState {
    Window(u64),
    Shell(u64, nickel_remote_control::desktop_events::ShellEventRole),
    Cleared,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingLaunchWindowDisposition {
    AwaitExpiry,
    Unrelated,
    Attributed { descendant: bool },
}

fn pending_launch_window_disposition(
    pending: &PendingLaunchObservation,
    now: Instant,
    client_pid: u32,
) -> PendingLaunchWindowDisposition {
    if now.saturating_duration_since(pending.registered_at) > pending.deadline {
        return PendingLaunchWindowDisposition::AwaitExpiry;
    }
    if process_descends_from(client_pid, pending.root_pid, pending.root_start_time) {
        PendingLaunchWindowDisposition::Attributed {
            descendant: client_pid != pending.root_pid,
        }
    } else {
        PendingLaunchWindowDisposition::Unrelated
    }
}

fn linux_process_parent(pid: u32) -> Option<u32> {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("PPid:\t"))?
        .trim()
        .parse()
        .ok()
}

fn linux_process_start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

fn process_descends_from(mut pid: u32, root_pid: u32, root_start_time: u64) -> bool {
    for _ in 0..64 {
        if pid == root_pid {
            return linux_process_start_time(pid) == Some(root_start_time);
        }
        let Some(parent) = linux_process_parent(pid) else {
            return false;
        };
        if parent == 0 || parent == pid {
            return false;
        }
        pid = parent;
    }
    false
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellRegistrationRejection {
    ClaimedPeerMismatch,
    NoActiveGeneration,
    OutsideActiveGeneration,
    OutsideSessionUser,
}

fn shell_registration_rejection(
    expected_pid: u32,
    claimed_pid: u32,
    peer_pid: u32,
    same_user: bool,
) -> Option<ShellRegistrationRejection> {
    if claimed_pid != peer_pid {
        Some(ShellRegistrationRejection::ClaimedPeerMismatch)
    } else if expected_pid == 0 {
        Some(ShellRegistrationRejection::NoActiveGeneration)
    } else if expected_pid != claimed_pid {
        Some(ShellRegistrationRejection::OutsideActiveGeneration)
    } else if !same_user {
        Some(ShellRegistrationRejection::OutsideSessionUser)
    } else {
        None
    }
}

fn command_requires_shell_identity(command: &SessionCommand) -> bool {
    matches!(
        command,
        SessionCommand::LogOut
            | SessionCommand::ObservePendingLaunch { .. }
            | SessionCommand::CancelPendingLaunch { .. }
            | SessionCommand::Unlock
            | SessionCommand::SessionAction { .. }
            | SessionCommand::FocusShellRole { .. }
            | SessionCommand::RestoreApplicationFocus
            | SessionCommand::ConfigureOnScreenKeyboard { .. }
            | SessionCommand::OnScreenKeyboardInput { .. }
            | SessionCommand::RegisterShellSurface { .. }
    )
}

fn test_control_may_invoke(command: &SessionCommand) -> bool {
    matches!(
        command,
        SessionCommand::LogOut
            | SessionCommand::Unlock
            | SessionCommand::SessionAction {
                action: nickel_session_protocol::SessionAction::Lock
            }
    )
}

fn production_control_token() -> String {
    use std::io::Read;

    let mut bytes = [0_u8; 32];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .is_err()
    {
        let fallback = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        bytes[..16].copy_from_slice(&fallback.to_ne_bytes());
        bytes[16..24].copy_from_slice(&u64::from(std::process::id()).to_ne_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn output_index_for_shell_surface(output_name: &str, output_names: &[String]) -> Option<usize> {
    if let Some(index) = output_names.iter().position(|name| name == output_name) {
        return Some(index);
    }

    let mut matches = output_names
        .iter()
        .enumerate()
        .filter(|(_, name)| {
            output_name
                .strip_suffix(name.as_str())
                .is_some_and(|prefix| prefix.ends_with(" - "))
        })
        .map(|(index, _)| index);
    let index = matches.next()?;
    matches.next().is_none().then_some(index)
}

fn shell_role_accepts_ordinary_focus(role: ShellRole) -> bool {
    matches!(
        role,
        ShellRole::ControlCenter
            | ShellRole::ProjectMenu
            | ShellRole::Preview
            | ShellRole::ContextMenu
            | ShellRole::VolumeOsd
            | ShellRole::Screenshot
    )
}

fn recv_control_frame(
    socket: &UnixDatagram,
    frame: &mut [u8],
) -> Result<(usize, Option<PathBuf>, u32), nix::errno::Errno> {
    use nix::sys::socket::{ControlMessageOwned, MsgFlags, UnixAddr, UnixCredentials, recvmsg};
    use std::io::IoSliceMut;

    let mut slices = [IoSliceMut::new(frame)];
    let mut credentials = nix::cmsg_space!(UnixCredentials);
    let message = recvmsg::<UnixAddr>(
        socket.as_raw_fd(),
        &mut slices,
        Some(&mut credentials),
        MsgFlags::MSG_DONTWAIT,
    )?;
    let length = message.bytes;
    let source = message
        .address
        .as_ref()
        .and_then(UnixAddr::path)
        .map(Path::to_path_buf);
    let peer_pid = message
        .cmsgs()?
        .find_map(|message| match message {
            ControlMessageOwned::ScmCredentials(credentials) => {
                u32::try_from(credentials.pid()).ok()
            }
            _ => None,
        })
        .ok_or(nix::errno::Errno::EACCES)?;
    Ok((length, source, peer_pid))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InternalShellTimerCounters {
    pub armed: u64,
    pub cancelled: u64,
    pub fired: u64,
    pub polls: u64,
    pub redraw_requests: u64,
}

#[derive(Debug, Default)]
struct InternalShellTimer {
    deadline: Option<Instant>,
    token: Option<smithay::reexports::calloop::RegistrationToken>,
    generation: u64,
    counters: InternalShellTimerCounters,
}

struct CompatibilityControlState {
    protocol_token: String,
    authenticated_shell_pids: HashSet<u32>,
    expected_shell_pid: u32,
    socket_path: PathBuf,
}

#[derive(Debug)]
pub(crate) enum InternalCaptureState {
    Idle,
    Pending(PathBuf),
    Complete(PathBuf, nickel_session_protocol::CaptureResult),
}

pub struct NickelSession {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub event_loop_handle: smithay::reexports::calloop::LoopHandle<'static, NickelSession>,
    pub(crate) native_clipboard: super::native_clipboard::NativeClipboardState,
    #[cfg(feature = "backend-udev")]
    pub native: Option<crate::session::backend::udev::UdevData>,

    pub space: Space<Window>,
    /// UI applications hosted directly by the compositor, without a Wayland client.
    pub internal_ui: crate::session::InternalUiRuntime,
    /// Built-in shell state when no supervised shell client is requested.
    pub(crate) internal_shell: Option<crate::internal_shell::InternalShellCoordinator>,
    /// Codex menu/chat applications hosted in `internal_ui` on Linux.
    pub(crate) internal_codex: Option<crate::internal_codex::InternalCodexHost>,
    pub(crate) internal_shell_surfaces:
        HashMap<nickel_ui::InternalSurfaceId, nickel_ui::InternalSurfaceId>,
    /// Compositor-owned overlays shown above ordinary clients while remote authority exists.
    pub(crate) remote_indicator_surfaces: HashMap<String, nickel_ui::InternalSurfaceId>,
    internal_file_surfaces: HashMap<nickel_ui::InternalSurfaceId, nickel_ui::InternalSurfaceId>,
    /// Latest motion is reduced immediately; scene work is bounded by frames.
    pending_desktop_scenes: HashSet<nickel_ui::InternalSurfaceId>,
    internal_shell_timer: InternalShellTimer,
    internal_system_status_source: Option<smithay::reexports::calloop::RegistrationToken>,
    pub loop_signal: LoopSignal,

    // Smithay State
    pub compositor_state: CompositorState,
    pub fractional_scale_manager_state: FractionalScaleManagerState,
    // Fractional-scale clients require wp_viewporter to submit buffers at the
    // advertised non-integer scale. Without it they fall back to wl_output's
    // ceil-rounded integer scale (125% therefore rendered as 200%).
    pub viewporter_state: ViewporterState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_dialog_state: XdgDialogState,
    pub activation_state: XdgActivationState,
    pub decoration_state: XdgDecorationState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<NickelSession>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub dnd_icon: Option<WlSurface>,
    pub relative_pointer_state: RelativePointerManagerState,
    pub pointer_constraints_state: PointerConstraintsState,
    pub(crate) pointer_lock_hints: HashMap<ObjectId, Point<f64, Logical>>,
    pub(crate) active_pointer_locks: HashSet<ObjectId>,
    pub(crate) active_pointer_constraint_origins: HashMap<ObjectId, Point<f64, Logical>>,
    pub idle_inhibit_state: IdleInhibitManagerState,
    pub input_method_state: InputMethodManagerState,
    pub xwayland_shell_state: XWaylandShellState,
    pub image_capture_source_state: ImageCaptureSourceState,
    pub output_capture_source_state: OutputCaptureSourceState,
    pub image_copy_capture_state: ImageCopyCaptureState,
    pub image_copy_sessions: Vec<Session>,
    pub(crate) pending_image_copy_frames: Vec<crate::session::handlers::PendingImageCopyFrame>,
    pub xwm: Option<(XwmId, X11Wm)>,
    pub xwayland_restart_pending: bool,
    pub xwayland_display: Option<u32>,
    pub xwayland_registration: Option<smithay::reexports::calloop::RegistrationToken>,
    pub popups: PopupManager,

    pub seat: Seat<Self>,
    pub(crate) on_screen_keyboard: crate::session::on_screen_keyboard::OnScreenKeyboardState,
    pub windows: WindowRegistry,
    pub surface_windows: HashMap<ObjectId, WindowId>,
    /// Stable canonical identities for application surfaces hosted in-process.
    internal_surface_windows: HashMap<nickel_ui::InternalSurfaceId, WindowId>,
    internal_window_surfaces: HashMap<WindowId, nickel_ui::InternalSurfaceId>,
    internal_minimized_windows: HashSet<WindowId>,
    internal_maximized_restore: HashMap<WindowId, crate::session::InternalSurfacePlacement>,
    surface_effective_outputs: HashMap<ObjectId, String>,
    /// Live XDG protocol roles outlive their mapped compositor representation.
    pub(crate) xdg_toplevel_windows: HashMap<ObjectId, Window>,
    pub(crate) mapped_xdg_toplevels: HashSet<ObjectId>,
    pub(crate) restored_xdg_toplevels: HashSet<ObjectId>,
    pub(crate) xdg_toplevel_locations: HashMap<ObjectId, Point<i32, Logical>>,
    pub(crate) shell_owned_windows: HashSet<WindowId>,
    pub x11_windows: HashMap<u32, WindowId>,
    pub launcher_window: Option<Window>,
    pub launcher_visibility: LauncherVisibility,
    launcher_output_name: Option<String>,
    last_interaction_output_name: Option<String>,
    launcher_focus: FocusTransactions<ObjectId>,
    launcher_restore_window: Option<WindowId>,
    launcher_subscribers: Vec<PathBuf>,
    pending_launch_observations: Vec<PendingLaunchObservation>,
    /// Legacy datagram compatibility is absent from normal compositor-owned
    /// sessions. It exists only when an explicit external-control mode asks
    /// for it.
    compatibility_control: Option<CompatibilityControlState>,
    shell_surface_identities: HashMap<String, ShellSurfaceIdentity>,
    registered_shell_role_slots: Vec<RegisteredShellRole>,
    last_logged_shell_readiness: Option<nickel_session_protocol::ShellReadinessSnapshot>,
    test_control_enabled: bool,
    #[cfg(target_os = "linux")]
    pub(crate) test_controller: Option<crate::session::test_input::TestController>,
    pub launcher_show_requested_at: Option<std::time::Instant>,
    pub desktop_windows: Vec<Window>,
    pub panel_windows: Vec<Window>,
    pub lock_windows: Vec<Window>,
    pub locked: bool,
    pub(crate) held_consumer_controls: HashMap<
        nickel_session_protocol::ConsumerControl,
        (u64, Option<smithay::reexports::calloop::RegistrationToken>),
    >,
    pub(crate) consumer_repeat_epoch: u64,
    lock_restore_window: Option<WindowId>,
    shell_focus_restore_window: Option<WindowId>,
    pub(crate) pending_shell_focus_role: Option<ShellRole>,
    pub utility_windows: Vec<Window>,
    hidden_shell_roles: HashSet<ShellRole>,
    hidden_shell_role_locations: HashMap<ShellRole, Point<i32, Logical>>,
    screenshot_output_name: Option<String>,
    pub context_menu_window: Option<Window>,
    pub preview_window: Option<Window>,
    pub server_decorated: HashSet<ObjectId>,
    pub primary_output_name: Option<String>,
    output_topology_generation: u64,
    last_protocol_outputs: Vec<OutputSnapshot>,
    output_scale_preferences: nickel_core::dpi::PersistedOutputScales,
    virtual_test_outputs: HashMap<String, (Output, Option<GlobalId>)>,
    pending_output_global_retirements: DeferredRetirements<GlobalId>,
    pub preview_frames: HashMap<WindowId, PreviewFrame>,
    preview_spares: HashMap<WindowId, Vec<u8>>,
    preview_switcher_interest: Vec<WindowId>,
    preview_overlay_interest: Vec<WindowId>,
    preview_admitted: HashSet<WindowId>,
    preview_dirty: HashSet<WindowId>,
    preview_content_generation: HashMap<WindowId, u64>,
    preview_attempted: HashMap<WindowId, (u64, u64)>,
    preview_render_wave: u64,
    preview_retry_pending: HashSet<WindowId>,
    preview_failures: HashMap<WindowId, preview::PreviewFailure>,
    preview_retry_scheduled: Option<(u64, smithay::reexports::calloop::RegistrationToken)>,
    preview_retry_epoch: u64,
    preview_counters: PreviewCacheCounters,
    pub hotkeys: CompositorShortcutAdapter,
    pub(crate) remote_control: nickel_remote_control::RemoteControlRuntime,
    pub(crate) local_cues: crate::local_cues::LocalCues,
    remote_controller_observer: nickel_ui::ControllerInput,
    remote_cleanup_wake: nickel_remote_control::ConnectionCleanupWake,
    remote_settings_staging: Arc<remote_settings::SettingsStaging>,
    remote_diagnostic_staging: Arc<remote_worker::WorkerStaging>,
    remote_observation_generation: u64,
    remote_application_inventory_generation: u64,
    remote_application_inventory_refresh:
        Option<nickel_remote_control::diagnostics::ApplicationInventoryRefreshOutcome>,
    remote_platform_refresh_generation: u64,
    remote_platform_refreshes: Vec<nickel_remote_control::diagnostics::PlatformRefreshOutcome>,
    remote_external_accessibility:
        Option<nickel_remote_control::diagnostics::ExternalAccessibilityDiagnostic>,
    remote_appearance: remote_appearance::AppearanceState,
    remote_application_scale: remote_application_scale::ScaleState,
    remote_launcher_favorites: remote_launcher_favorites::FavoritesState,
    remote_wallpaper: remote_wallpaper::WallpaperState,
    remote_file_icons: remote_file_icons::FileIconState,
    remote_idle_preferences: remote_idle_preferences::IdlePreferenceState,
    remote_codex_runtime_generation: u64,
    remote_terminal_presentation: remote_terminal_presentation::TerminalPresentationState,
    remote_desktop_events: nickel_remote_control::desktop_events::DesktopEvents,
    remote_window_event_states: HashMap<u64, RemoteWindowEventState>,
    remote_output_event_states: HashMap<u64, RemoteOutputEventState>,
    remote_focus_event_state: Option<RemoteFocusEventState>,
    remote_frame_trace: Option<nickel_remote_control::frame_trace::FrameTrace>,
    remote_event_windows: HashSet<WindowId>,
    remote_launched_children: Vec<std::process::Child>,
    remote_launch_placements: Vec<remote_launch::LaunchPlacement>,
    remote_launch_maps: HashMap<WindowId, (Instant, remote_launch::DeferredLaunchMap)>,
    remote_shell_origins: HashMap<u64, remote_shell_actions::PendingDeviceOrigin>,
    remote_shell_origin_generation: u64,
    pub(crate) remote_held_keyboard: Option<remote_keyboard::RemoteHeldKeyboard>,
    remote_keyboard_timer_armed: bool,
    remote_native_key_worker: Option<native_key_worker::NativeKeyWorker>,
    pub(crate) remote_held_pointer: Option<remote_pointer::RemoteHeldPointer>,
    pub(crate) remote_input_dispatching: bool,
    pub(crate) remote_native_press: Option<(nickel_remote_control::DesktopPermit, WindowId)>,
    remote_gtk_menu: Option<remote_accessibility::RemoteGtkMenu>,
    remote_gtk_menu_timer_armed: bool,
    pub(crate) remote_gtk_epoch: u64,
    remote_pointer_timer_armed: bool,
    #[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
    remote_capture_work: Option<remote_capture::RemoteCaptureWork>,
    remote_resource_recheck_pending: bool,
    remote_output_generations: HashMap<String, (Output, u64)>,
    remote_next_output_generation: u64,
    remote_identity_worker: Option<super::remote_identity::IdentityWorker>,
    remote_window_identities: HashMap<WindowId, super::remote_identity::WindowIdentity>,
    pub(crate) remote_emergency_chord: nickel_remote_control::EmergencyChord,
    remote_stop_confirmation_until: Option<Instant>,
    pub(crate) remote_desktop_authority: Arc<dyn nickel_remote_control::DesktopAuthority>,
    pub task_switcher: TaskSwitcher<WindowId>,
    pub workspaces: Workspaces<WindowId>,
    pub workspace_hidden_windows: HashMap<WindowId, (Window, Point<i32, Logical>)>,
    displaced_output_windows: HashMap<String, Vec<DisplacedWindow>>,
    pub preview_highlight: Option<WindowId>,
    pub minimized_windows: HashMap<WindowId, (Window, Point<i32, Logical>)>,
    shortcut_desktop_windows: Vec<WindowId>,
    shortcut_desktop_focus: Option<WindowId>,
    shortcut_snap_restore: HashMap<WindowId, smithay::utils::Rectangle<i32, Logical>>,
    maximized_restore: HashMap<ObjectId, Geometry>,
    x11_maximized_restore: HashMap<u32, smithay::utils::Rectangle<i32, Logical>>,
    fullscreen_restore: HashMap<ObjectId, Geometry>,
    x11_fullscreen_restore: HashMap<u32, smithay::utils::Rectangle<i32, Logical>>,
    pub last_titlebar_click: Option<(ObjectId, u32, Point<f64, Logical>)>,
    pub suppress_left_button_release: bool,
    pub idle_inhibitors: HashMap<WlSurface, usize>,
    pub(crate) active_touch_slots: HashSet<smithay::backend::input::TouchSlot>,
    idle_controller: IdleController,
    pub dimmed: bool,
    pub frame_cursor: crate::session::window_frame::FrameCursor,
    pub buffer_commit_tx: Option<smithay::reexports::calloop::channel::Sender<SurfaceBufferCommit>>,
    pub identify_outputs_until: Option<std::time::Instant>,
    identify_outputs_generation: u64,
    identify_outputs_timer: Option<smithay::reexports::calloop::RegistrationToken>,
    remote_output_identification: Option<RemoteOutputIdentification>,
    pub output_capture_path: Option<PathBuf>,
    pub output_capture_name: Option<String>,
    pub output_capture_reply_path: Option<PathBuf>,
    pub output_capture_request_id: Option<u64>,
    pub(crate) internal_capture: Arc<std::sync::Mutex<InternalCaptureState>>,
    pub(crate) internal_projection_outputs: Arc<std::sync::RwLock<Vec<OutputSnapshot>>>,
    pub(crate) internal_keyboard_snapshot:
        Arc<std::sync::RwLock<Option<nickel_session_protocol::OnScreenKeyboardSnapshot>>>,
    pub shell_failure_count: u8,
    pub(crate) recovery_ui: crate::session::recovery_ui::RecoveryUi,
    secure_storage_state: Arc<AtomicU8>,
    secure_storage_retry: Arc<std::sync::atomic::AtomicBool>,
    deferred_focus_restore: channel::Sender<WindowId>,
    #[cfg(feature = "backend-winit")]
    winit_redraw_window: Option<*const dyn smithay::reexports::winit::window::Window>,
}

mod control_protocol;
mod native_key_worker;
mod preview;
mod remote_accessibility;
mod remote_appearance;
mod remote_application_scale;
#[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
mod remote_capture;
mod remote_codex_preference;
mod remote_controller;
mod remote_diagnostics;
mod remote_file_icons;
mod remote_idle_preferences;
mod remote_keyboard;
mod remote_keyboard_preference;
pub(super) mod remote_launch;
pub(super) mod remote_launcher_favorites;
mod remote_pointer;
mod remote_settings;
mod remote_shell_actions;
mod remote_terminal_presentation;
mod remote_wallpaper;
mod remote_worker;

#[allow(unused_imports)]
pub use preview::{
    PREVIEW_BYTE_CAPACITY, PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER, PREVIEW_ENTRY_CAPACITY,
    PREVIEW_FRAME_BYTES, PREVIEW_HEIGHT, PREVIEW_WIDTH, PreviewFrame,
};
pub(crate) use preview::{
    PreviewCacheCounters, bounded_preview_ids, preview_capture_dimensions,
    preview_mapping_has_exact_size, protocol_preview_from_cached, reuse_preview_pixels,
};
#[cfg(test)]
use preview::{
    admitted_preview_ids, advance_preview_content_generation, record_preview_capture_attempt,
};
impl NickelSession {
    pub(super) fn schedule_remote_window_identity(
        &mut self,
        id: WindowId,
        source: super::remote_identity::IdentitySource,
    ) {
        use super::remote_identity::WindowIdentity;
        let queued = self
            .remote_identity_worker
            .as_ref()
            .is_some_and(|worker| worker.request(id, source));
        self.remote_window_identities.insert(
            id,
            if queued {
                WindowIdentity::Pending
            } else {
                WindowIdentity::Unavailable
            },
        );
    }

    fn refresh_remote_window_identities(&mut self) {
        use super::remote_identity::{IdentitySource, WindowIdentity};
        let catalog_generation = crate::platform::run_signature_diagnostics().generation;
        let stale = self
            .remote_window_identities
            .iter()
            .filter_map(|(id, identity)| match identity {
                WindowIdentity::Pending => None,
                WindowIdentity::Unavailable => Some(*id),
                WindowIdentity::Verified(process) => (!process.is_current()
                    || process.catalog_generation != catalog_generation)
                    .then_some(*id),
            })
            .collect::<Vec<_>>();
        for id in stale {
            let Some(window) = self.registry_native_window(id) else {
                continue;
            };
            let source = if let Some(x11) = window.x11_surface() {
                Some(IdentitySource::X11Client(Box::new(x11.clone())))
            } else {
                window
                    .wl_surface()
                    .and_then(|surface| surface.client())
                    .and_then(|client| client.get_credentials(&self.display_handle).ok())
                    .and_then(|credentials| u32::try_from(credentials.pid).ok())
                    .map(|pid| IdentitySource::WaylandPeer {
                        pid,
                        app_id: self
                            .windows
                            .app_id(id)
                            .filter(|app_id| !app_id.is_empty())
                            .map(str::to_owned),
                    })
            };
            if let Some(source) = source {
                self.schedule_remote_window_identity(id, source);
            }
        }
    }

    fn remote_window_is_protected(&self, id: WindowId) -> bool {
        self.locked
            || self.shell_recovery_visible()
            || self.shell_owned_windows.contains(&id)
            || self
                .internal_window_surfaces
                .get(&id)
                .is_some_and(|surface| self.internal_ui.remote_access_protected(*surface))
            || (!self.internal_window_surfaces.contains_key(&id)
                && self
                    .remote_window_identities
                    .get(&id)
                    .is_none_or(|identity| identity.is_protected()))
    }

    fn remote_verified_application(&self, id: WindowId) -> Option<String> {
        self.remote_verified_application_inner(id, &mut Vec::new())
    }

    fn remote_verified_application_inner(
        &self,
        id: WindowId,
        visited: &mut Vec<WindowId>,
    ) -> Option<String> {
        if self.remote_window_is_protected(id) {
            return None;
        }
        if let Some(surface) = self.internal_window_surfaces.get(&id).copied() {
            if self
                .internal_ui
                .application::<nickel_file::FileApp>(surface)
                .is_some()
            {
                return Some("nickel-file".into());
            }
            if self
                .internal_ui
                .application::<nickel_codex_ui::ChatApplication>(surface)
                .is_some()
            {
                return Some("nickel-codex".into());
            }
            return None;
        }
        let identity = self.remote_window_identities.get(&id)?;
        if let Some(application) = identity.application() {
            return Some(application.to_owned());
        }
        // A native transient may use a shared helper/runtime which has no safe
        // standalone application identity. Inherit only through the live
        // compositor relationship and the exact same OS process incarnation.
        // X11's forgeable WM_TRANSIENT_FOR therefore cannot cross a process
        // boundary, while Wayland's server-owned parent graph receives the same
        // conservative process check.
        if visited.len() >= 16 || visited.contains(&id) {
            return None;
        }
        visited.push(id);
        let parent = self.remote_native_parent(id)?;
        let child_process = identity.observation_process()?;
        let parent_process = self
            .remote_window_identities
            .get(&parent)?
            .observation_process()?;
        if !child_process.same_current_process(&parent_process) {
            return None;
        }
        self.remote_verified_application_inner(parent, visited)
    }

    fn remote_native_parent(&self, id: WindowId) -> Option<WindowId> {
        let window = self.window_for_registry_id(id)?;
        if let Some(surface) = window.x11_surface() {
            return surface
                .is_transient_for()
                .and_then(|parent| self.x11_windows.get(&parent).copied());
        }
        let parent = window.toplevel()?.parent()?;
        self.surface_windows
            .get(&parent.wl_surface()?.id())
            .copied()
    }

    fn remote_lease_target_live(
        &self,
        scope: &nickel_session_protocol::RemoteResourceScope,
    ) -> bool {
        use nickel_session_protocol::RemoteResourceScope;
        if self.locked {
            return false;
        }
        match scope {
            RemoteResourceScope::Window(resource) => {
                let id = WindowId(resource.generation);
                resource.id == resource.generation.to_string()
                    && self.windows.contains(id)
                    && !self.remote_window_is_protected(id)
            }
            RemoteResourceScope::Surface(resource) => self
                .internal_ui
                .resolve_surface_identity(&resource.id, resource.generation)
                .is_some_and(|id| !self.internal_ui.remote_access_protected(id)),
            RemoteResourceScope::Output(resource) => self
                .remote_output_generations
                .get(&resource.id)
                .is_some_and(|(output, generation)| {
                    *generation == resource.generation
                        && self.space.outputs().any(|live| live == output)
                }),
            // Application and session grants may precede a window's creation.
            RemoteResourceScope::Application(_) | RemoteResourceScope::FullSession => true,
        }
    }

    fn remote_resource_label(
        &self,
        scope: &nickel_session_protocol::RemoteResourceScope,
    ) -> Option<String> {
        use nickel_session_protocol::RemoteResourceScope;
        match scope {
            RemoteResourceScope::Application(application) if application == "nickel-file" => {
                Some("Nickel File windows".into())
            }
            RemoteResourceScope::Application(application) if application == "nickel-codex" => {
                Some("Nickel Codex windows".into())
            }
            RemoteResourceScope::Application(application) => self
                .remote_window_identities
                .values()
                .find_map(|identity| match identity {
                    super::remote_identity::WindowIdentity::Verified(process)
                        if identity.application() == Some(application.as_str()) =>
                    {
                        process
                            .application_name
                            .as_ref()
                            .map(|name| format!("{name} windows"))
                    }
                    _ => None,
                }),
            RemoteResourceScope::Window(resource) => {
                let id = resource.id.parse::<u64>().ok()?;
                if id != resource.generation || self.remote_window_is_protected(WindowId(id)) {
                    return None;
                }
                self.windows.title(WindowId(id)).map(|title| {
                    let title: String = title
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(256)
                        .collect();
                    format!("{title} (window {id})")
                })
            }
            _ => None,
        }
    }

    fn schedule_remote_resource_retirement(&mut self) {
        if self.remote_resource_recheck_pending {
            return;
        }
        self.remote_resource_recheck_pending = true;
        // A window may be destroyed from inside an authorized synchronous operation. Defer
        // mutex acquisition until that owner dispatch ends, coalescing all retired identities.
        // Queued remote requests independently reject nonexistent targets before any effect.
        self.event_loop_handle
            .insert_idle(|data| data.flush_remote_resource_retirement());
    }

    fn refresh_remote_output_identities(&mut self) {
        self.invalidate_departed_shell_outputs();
        let outputs = self.space.outputs().cloned().collect::<Vec<_>>();
        let before = self.remote_output_generations.len();
        self.remote_output_generations
            .retain(|_, (output, _)| outputs.contains(output));
        let mut changed = before != self.remote_output_generations.len();
        for output in outputs {
            if self
                .remote_output_generations
                .get(&output.name())
                .is_some_and(|(previous, _)| previous == &output)
            {
                continue;
            }
            let Some(generation) = self.remote_next_output_generation.checked_add(1) else {
                continue;
            };
            self.remote_next_output_generation = generation;
            self.remote_output_generations
                .insert(output.name(), (output, generation));
            changed = true;
        }
        if changed {
            if !self.locked && !self.shell_recovery_visible() {
                self.remote_desktop_events.record(
                    nickel_remote_control::desktop_events::DesktopEventKind::OutputMembershipChanged {
                        latest_output_identity_generation: self.remote_next_output_generation,
                        outputs: self.remote_output_generations.len(),
                    },
                    self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
                );
            }
            self.schedule_remote_resource_retirement();
        }
    }

    fn remote_window_geometry(&self, id: WindowId) -> Option<nickel_session_protocol::Geometry> {
        // MCP coordinates describe the client area, not the buffer's shadows or
        // decorations. Use the same geometry as production pointer dispatch.
        self.window_for_registry_id(id)
            .and_then(|window| self.space.element_geometry(&window))
            .map(|bounds| nickel_session_protocol::Geometry {
                x: bounds.loc.x,
                y: bounds.loc.y,
                width: bounds.size.w,
                height: bounds.size.h,
            })
            .or_else(|| self.registry_window_geometry(id))
    }

    fn remote_window_output(
        &self,
        id: WindowId,
    ) -> Option<nickel_remote_control::leases::ResourceId> {
        let bounds = self.remote_window_geometry(id)?;
        let geometry = self.output_geometry_for_bounds(Geometry {
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
        })?;
        let name = self.output_name_matching_geometry(geometry)?;
        self.remote_output_identity(name)
    }

    fn remote_output_identity(
        &self,
        name: String,
    ) -> Option<nickel_remote_control::leases::ResourceId> {
        let (_, generation) = self.remote_output_generations.get(&name)?;
        Some(nickel_remote_control::leases::ResourceId {
            id: name,
            generation: *generation,
        })
    }

    fn flush_remote_resource_retirement(&mut self) {
        if !std::mem::take(&mut self.remote_resource_recheck_pending) {
            return;
        }
        let live = self
            .windows
            .snapshot()
            .into_iter()
            .map(|window| window.id.0)
            .collect::<HashSet<_>>();
        self.remote_window_identities
            .retain(|id, _| live.contains(&id.0));
        let control = self.remote_control.control();
        let mut control = control.lock().unwrap();
        let retired = control
            .leases()
            .iter()
            .filter_map(|lease| {
                if let nickel_remote_control::leases::ResourceScope::Window(resource) = &lease.scope
                {
                    let valid = resource
                        .id
                        .parse::<u64>()
                        .ok()
                        .is_some_and(|id| id == resource.generation && live.contains(&id));
                    (!valid).then_some(lease.id)
                } else if let nickel_remote_control::leases::ResourceScope::Surface(resource) =
                    &lease.scope
                {
                    (!self
                        .internal_ui
                        .has_surface_identity(&resource.id, resource.generation))
                    .then_some(lease.id)
                } else if let nickel_remote_control::leases::ResourceScope::Output(resource) =
                    &lease.scope
                {
                    self.remote_output_generations
                        .get(&resource.id)
                        .is_none_or(|(_, generation)| *generation != resource.generation)
                        .then_some(lease.id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for lease in &retired {
            control.leases_mut().revoke(*lease);
        }
        drop(control);
        if !retired.is_empty() {
            self.sync_remote_control_indicators();
        }
    }

    pub(crate) fn remote_keyboard_target_matches(&self, id: WindowId) -> bool {
        self.seat
            .get_keyboard()
            .is_some_and(|keyboard| keyboard.pressed_keys().is_empty())
            && self.remote_keyboard_focus_matches(id)
    }

    fn remote_keyboard_focus_matches(&self, id: WindowId) -> bool {
        use smithay::wayland::seat::WaylandFocus;

        if self.remote_window_is_protected(id) || self.internal_ui.focused().is_some() {
            return false;
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return false;
        };
        if keyboard.is_grabbed() {
            return false;
        }
        let Some(window) = self.window_for_registry_id(id) else {
            return false;
        };
        let Some(focus) = keyboard.current_focus() else {
            return false;
        };
        // Compare native recipients, not the cached active-window flag. A popup
        // or a compositor-owned surface must not inherit its parent's approval.
        match focus {
            super::focus::KeyboardFocusTarget::X11(surface) => {
                surface.has_observed_keyboard_focus() && window.x11_surface() == Some(&surface)
            }
            super::focus::KeyboardFocusTarget::Wayland(surface) => window
                .wl_surface()
                .is_some_and(|recipient| *recipient == surface),
        }
    }

    pub(crate) fn remote_pointer_target_matches(&self, id: WindowId, x: i32, y: i32) -> bool {
        let point = (f64::from(x), f64::from(y)).into();
        if self.remote_window_is_protected(id) {
            return false;
        }
        if self
            .internal_ui
            .surface_at(
                (f64::from(x), f64::from(y)),
                self.client_scene_under(point) && !self.internal_applications_are_foremost(),
            )
            .is_some()
        {
            return false;
        }
        let Some(window) = self.window_for_registry_id(id) else {
            return false;
        };
        self.space
            .element_under(point)
            .is_some_and(|(hit, _)| hit == &window)
            && self.surface_under(point).is_some()
    }

    fn remote_window_summary(
        &self,
        window: nickel_session_protocol::WindowSnapshot,
    ) -> nickel_remote_control::WindowSummary {
        let (x, y, width, height) = window.geometry.map_or((0, 0, 0, 0), |geometry| {
            (geometry.x, geometry.y, geometry.width, geometry.height)
        });
        let verified_application = self.remote_verified_application(WindowId(window.id.0));
        nickel_remote_control::WindowSummary {
            id: window.id.0.to_string(),
            application_id: window.application_id.chars().take(256).collect(),
            title: window.title.chars().take(512).collect(),
            active: window.active,
            minimized: window.minimized,
            maximized: window.maximized,
            fullscreen: window.fullscreen,
            x,
            y,
            width: u32::try_from(width).unwrap_or_default(),
            height: u32::try_from(height).unwrap_or_default(),
            // Registry IDs are monotonic and never reused during a session.
            generation: window.id.0,
            workspace: window.workspace.0,
            verified_application,
        }
    }

    fn register_remote_connection_cleanup_wake(
        handle: &smithay::reexports::calloop::LoopHandle<'static, Self>,
        descriptor: std::io::Result<std::os::fd::OwnedFd>,
    ) -> nickel_remote_control::ConnectionCleanupWake {
        use smithay::reexports::{
            calloop::{Interest, Mode, PostAction, generic::Generic},
            rustix,
        };
        let fallback = || {
            tracing::warn!(
                "Remote connection cleanup wake unavailable; periodic owner fallback is active"
            );
            nickel_remote_control::ConnectionCleanupWake::new(|| false)
        };
        let Ok(descriptor) = descriptor else {
            return fallback();
        };
        let cleanup_fd = Arc::new(descriptor);
        let writer = cleanup_fd.clone();
        let wake = nickel_remote_control::ConnectionCleanupWake::new(move || {
            matches!(
                rustix::io::write(&*writer, &1_u64.to_ne_bytes()),
                Ok(8) | Err(rustix::io::Errno::AGAIN)
            )
        });
        let source = Generic::new(cleanup_fd, Interest::READ, Mode::Level);
        if handle.insert_source(source, |_, fd, data| {
            let mut counter = [0_u8; 8];
            if let Err(error) = rustix::io::read(&**fd, &mut counter)
                && error != rustix::io::Errno::AGAIN {
                tracing::warn!("Remote connection cleanup wake read failed; periodic owner fallback is active");
                data.service_remote_connection_cleanup();
                return Ok(PostAction::Remove);
            }
            data.service_remote_connection_cleanup();
            Ok(PostAction::Continue)
        }).is_err() {
            return fallback();
        }
        wake
    }

    fn service_remote_connection_cleanup(&mut self) {
        if self.remote_cleanup_wake.take_wake_failure() {
            tracing::warn!("Remote connection cleanup wake failed; owner fallback is active");
        }
        if !self.remote_cleanup_wake.take_pending() {
            return;
        }
        self.remote_control
            .control()
            .lock()
            .unwrap()
            .reconcile_pending_lease_requests(Instant::now());
        self.sync_remote_control_indicators();
    }

    fn handle_remote_desktop_request(&mut self, request: RemoteDesktopRequest) {
        self.service_remote_connection_cleanup();
        self.refresh_remote_output_identities();
        self.record_remote_output_state_events();
        self.revalidate_remote_frame_trace();
        match request {
            RemoteDesktopRequest::ClientConnection {
                permit,
                action,
                reply,
            } => {
                let result = permit.apply(action, self.locked || self.shell_recovery_visible());
                self.sync_remote_control_indicators();
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::LaunchApplication {
                permit,
                prepared,
                reply,
            } => {
                let result = self.remote_launch_installed_application(&permit, prepared);
                if result.is_ok() {
                    self.record_remote_production_effect_outcome(
                        &permit,
                        nickel_remote_control::desktop_events::ProductionEffectKind::ApplicationLaunch,
                        nickel_remote_control::desktop_events::ProductionEffectOutcome::Confirmed,
                    );
                }
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::Events {
                permit,
                after,
                reply,
            } => {
                let result =
                    permit.with_debug(self.locked || self.shell_recovery_visible(), || {
                        let history = self.remote_desktop_events.since(after)?;
                        self.remote_observation_generation =
                            self.remote_observation_generation.saturating_add(1);
                        Ok(
                            nickel_remote_control::desktop_events::DesktopEventObservation {
                                observation_generation: self.remote_observation_generation,
                                observed_at_us: self
                                    .start_time
                                    .elapsed()
                                    .as_micros()
                                    .min(u64::MAX as u128)
                                    as u64,
                                history,
                            },
                        )
                    });
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::Applications {
                permit,
                prepared,
                reply,
            } => {
                let result = self.remote_scoped_application_inventory(&permit, prepared);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::Outputs { permit, reply } => {
                let result = self.remote_list_outputs(&permit);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::WorkspaceAction {
                permit,
                action,
                reply,
            } => {
                let result = self.remote_workspace_action(&permit, action);
                if result.is_ok()
                    && action != nickel_remote_control::diagnostics::WorkspaceAction::List
                {
                    self.record_remote_production_effect_outcome(
                        &permit,
                        nickel_remote_control::desktop_events::ProductionEffectKind::WorkspaceAction,
                        nickel_remote_control::desktop_events::ProductionEffectOutcome::Confirmed,
                    );
                }
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::ApplicationScale { permit, request } => {
                self.remote_application_scale(permit, request);
            }
            RemoteDesktopRequest::ReadAppearance {
                permit,
                prepared,
                reply,
            } => {
                let result = self.remote_read_appearance(&permit, prepared);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::AppearanceTransaction {
                permit,
                transaction,
                prepared,
                reply,
            } => {
                let result = self.remote_change_appearance(&permit, transaction, prepared);
                self.record_remote_settings_transaction(&permit, &result);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::ReadLauncherFavorites {
                permit,
                prepared,
                reply,
            } => {
                let result = self.remote_read_launcher_favorites(&permit, prepared);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::LauncherFavoritesTransaction {
                permit,
                transaction,
                prepared,
                reply,
            } => {
                let result = self.remote_change_launcher_favorites(&permit, transaction, prepared);
                self.record_remote_settings_transaction(&permit, &result);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::ReadWallpaper {
                permit,
                prepared,
                reply,
            } => {
                let result = self.remote_read_wallpaper(&permit, prepared);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::WallpaperTransaction {
                permit,
                transaction,
                prepared,
                reply,
            } => {
                let result = self.remote_change_wallpaper(&permit, transaction, prepared);
                self.record_remote_settings_transaction(&permit, &result);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::ReadFileIcons {
                permit,
                prepared,
                reply,
            } => {
                let result = self.remote_read_file_icons(&permit, prepared);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::FileIconsTransaction {
                permit,
                transaction,
                prepared,
                reply,
            } => {
                let result = self.remote_change_file_icons(&permit, transaction, prepared);
                self.record_remote_settings_transaction(&permit, &result);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::ReadCodexPreference {
                permit,
                prepared,
                reply,
            } => {
                let result = self.remote_read_codex_preference(&permit, prepared);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::CodexPreferenceTransaction {
                permit,
                transaction,
                prepared,
                reply,
            } => {
                let result = self.remote_change_codex_preference(&permit, transaction, prepared);
                self.record_remote_settings_transaction(&permit, &result);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::ReadIdlePreferences {
                permit,
                prepared,
                reply,
            } => {
                let result = self.remote_read_idle_preferences(&permit, prepared);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::IdlePreferencesTransaction {
                permit,
                transaction,
                prepared,
                reply,
            } => {
                let result = self.remote_change_idle_preferences(&permit, transaction, prepared);
                self.record_remote_settings_transaction(&permit, &result);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::ReadTerminalPresentation {
                permit,
                prepared,
                reply,
            } => {
                let result = self.remote_read_terminal_presentation(&permit, prepared);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::TerminalPresentationTransaction {
                permit,
                transaction,
                prepared,
                reply,
            } => {
                let result =
                    self.remote_change_terminal_presentation(&permit, transaction, prepared);
                self.record_remote_settings_transaction(&permit, &result);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::ReadKeyboardPreference {
                permit,
                prepared,
                reply,
            } => {
                let result = self.remote_read_keyboard_preference(&permit, prepared);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::KeyboardPreferenceTransaction {
                permit,
                transaction,
                prepared,
                reply,
            } => {
                let result = self.remote_change_keyboard_preference(&permit, transaction, prepared);
                self.record_remote_settings_transaction(&permit, &result);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::ShellBehavior {
                permit,
                transaction,
                prepared,
                reply,
            } => {
                let result = self.remote_shell_behavior_transaction(&permit, transaction, prepared);
                self.record_remote_settings_transaction(&permit, &result);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::NativeKeyboardState {
                source,
                sequence,
                result,
            } => {
                self.complete_native_keyboard_query(source, sequence, result);
            }

            RemoteDesktopRequest::DiagnosticAction {
                permit,
                action,
                application_discovery,
                platform_refresh,
                reply,
            } => {
                let protected = self.locked || self.shell_recovery_visible();
                let refresh_applications = matches!(
                    &action,
                    nickel_remote_control::diagnostics::DiagnosticAction::RefreshApplicationInventory
                );
                let refresh_platform = matches!(
                    &action,
                    nickel_remote_control::diagnostics::DiagnosticAction::RefreshPlatformStatus { .. }
                );
                let effect = || {
                    #[cfg(not(any(feature = "backend-udev", feature = "backend-winit")))]
                    {
                        let _ = action;
                        Err("diagnostic rendering backend is unavailable".into())
                    }
                    #[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
                    {
                        let mut output_identification = None;
                        let mut application_inventory_refresh = None;
                        let mut platform_refresh_outcome = None;
                        match action.clone() {
                            nickel_remote_control::diagnostics::DiagnosticAction::IdentifyOutput { output } => {
                                action.validate()?;
                                if self.remote_output_identity(output.id.clone()).as_ref() != Some(&output) {
                                    return Err("output incarnation is unavailable".into());
                                }
                                let generation = self.start_output_identification(Some(RemoteOutputIdentification { permit: permit.clone(), output: output.clone() }))?;
                                output_identification = Some(nickel_remote_control::diagnostics::OutputIdentificationOutcome { output, generation, duration_ms: 3000 });
                            }
                            nickel_remote_control::diagnostics::DiagnosticAction::Repaint => {}
                            nickel_remote_control::diagnostics::DiagnosticAction::StartFrameTrace { duration_seconds } => {
                                let category = self.remote_frame_trace_category().ok_or("frame trace backend is unavailable")?;
                                if self.remote_frame_trace.as_ref().is_some_and(|trace| trace.active()) {
                                    return Err("frame trace capacity reached".into());
                                }
                                self.remote_frame_trace = Some(nickel_remote_control::frame_trace::FrameTrace::new_authorized(permit.clone(), duration_seconds, category)?);
                            }
                            nickel_remote_control::diagnostics::DiagnosticAction::StopFrameTrace => {
                                if let Some(trace) = self.remote_frame_trace.as_mut() {
                                    if !trace.owned_by(&permit) { return Err("frame trace belongs to another lease".into()); }
                                    trace.stop();
                                }
                            }

                            nickel_remote_control::diagnostics::DiagnosticAction::RefreshScene => {
                                use nickel_remote_control::diagnostics::{
                                    MAX_DIAGNOSTIC_OUTPUTS, MAX_DIAGNOSTIC_WINDOWS,
                                };
                                if self
                                    .space
                                    .elements()
                                    .take(MAX_DIAGNOSTIC_WINDOWS + 1)
                                    .count()
                                    > MAX_DIAGNOSTIC_WINDOWS
                                    || self
                                        .space
                                        .outputs()
                                        .take(MAX_DIAGNOSTIC_OUTPUTS + 1)
                                        .count()
                                        > MAX_DIAGNOSTIC_OUTPUTS
                                {
                                    return Err("scene exceeds diagnostic refresh budget".into());
                                }
                                // The same production housekeeping pass used by
                                // both rendering backends. No native round trips,
                                // permission changes, or requested geometry edits.
                                self.space.refresh();
                            }
                            nickel_remote_control::diagnostics::DiagnosticAction::RefreshApplicationInventory => {
                                let prepared = application_discovery.ok_or(
                                    "application inventory preparation is unavailable",
                                )?;
                                let generation = self
                                    .remote_application_inventory_generation
                                    .checked_add(1)
                                    .ok_or("application inventory generation exhausted")?;
                                let controller_busy = self.poll_remote_controller_ownership();
                                if controller_busy
                                    || self.remote_held_keyboard.is_some()
                                    || self.remote_held_pointer.is_some()
                                    || !self.active_touch_slots.is_empty()
                                    || self.internal_ui.pointer_interaction_active()
                                    || self.internal_ui.desktop_keyboard_interaction_active()
                                    || self.seat.get_keyboard().is_some_and(|keyboard| {
                                        !keyboard.pressed_keys().is_empty() || keyboard.is_grabbed()
                                    })
                                    || self
                                        .seat
                                        .get_pointer()
                                        .is_some_and(|pointer| pointer.is_grabbed())
                                    || self
                                        .internal_shell
                                        .as_ref()
                                        .is_some_and(|shell| shell.pointer_interaction_active())
                                {
                                    return Err("shared input is busy or unavailable".into());
                                }
                                crate::platform::publish_application_discovery(&prepared.discovery);
                                let shell = self.internal_shell.as_mut().unwrap();
                                let (changed, applications, partial) =
                                    shell.apply_application_discovery(prepared.discovery);
                                self.remote_application_inventory_generation = generation;
                                application_inventory_refresh = Some(
                                    nickel_remote_control::diagnostics::ApplicationInventoryRefreshOutcome {
                                        generation: self.remote_application_inventory_generation,
                                        observation_started_at_us: prepared
                                            .observation_started
                                            .saturating_duration_since(self.start_time)
                                            .as_micros()
                                            .min(u128::from(u64::MAX)) as u64,
                                        observed_at_us: prepared
                                            .observed
                                            .saturating_duration_since(self.start_time)
                                            .as_micros()
                                            .min(u128::from(u64::MAX)) as u64,
                                        preparation_duration_us: prepared.preparation_duration_us,
                                        stale: false,
                                        applications: applications.min(u32::MAX as usize) as u32,
                                        partial,
                                        reconciliation_confirmed: true,
                                    },
                                );
                                self.remote_application_inventory_refresh =
                                    application_inventory_refresh.clone();
                                self.remote_desktop_events.record(
                                    nickel_remote_control::desktop_events::DesktopEventKind::ApplicationInventoryRefreshCompleted {
                                        generation,
                                        partial,
                                    },
                                    self.start_time.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
                                );
                                self.sync_internal_shell_changes(Some(&changed));
                            }
                            nickel_remote_control::diagnostics::DiagnosticAction::RefreshPlatformStatus { domain } => {
                                let prepared = platform_refresh.ok_or(
                                    "platform refresh preparation is unavailable",
                                )?;
                                if prepared.domain != domain {
                                    return Err("platform refresh domain changed before commit".into());
                                }
                                if self.internal_shell.is_none() {
                                    return Err("internal shell unavailable".into());
                                }
                                let generation = self
                                    .remote_platform_refresh_generation
                                    .checked_add(1)
                                    .ok_or("platform refresh generation exhausted")?;
                                let controller_busy = self.poll_remote_controller_ownership();
                                if controller_busy
                                    || self.remote_held_keyboard.is_some()
                                    || self.remote_held_pointer.is_some()
                                    || !self.active_touch_slots.is_empty()
                                    || self.internal_ui.pointer_interaction_active()
                                    || self.internal_ui.desktop_keyboard_interaction_active()
                                    || self.seat.get_keyboard().is_some_and(|keyboard| {
                                        !keyboard.pressed_keys().is_empty() || keyboard.is_grabbed()
                                    })
                                    || self
                                        .seat
                                        .get_pointer()
                                        .is_some_and(|pointer| pointer.is_grabbed())
                                    || self
                                        .internal_shell
                                        .as_ref()
                                        .is_some_and(|shell| shell.pointer_interaction_active())
                                {
                                    return Err("shared input is busy or unavailable".into());
                                }
                                let (mut changed, network_available, bluetooth_available, audio_available,
                                    printers_available, volumes_available, filesystems_available,
                                    printer_count, volume_count, filesystem_count,
                                    maintenance_available, updates_available, restart_required,
                                    firewall_healthy, malware_protection_healthy,
                                    known_permission_states, secure_storage_status_available,
                                    associations_available, association_targets_queried,
                                    effective_associations, directly_writable_associations,
                                    partial, reconciliation_confirmed) =
                                    match (domain, prepared.data) {
                                        (
                                            nickel_remote_control::diagnostics::PlatformRefreshDomain::Connectivity,
                                            PreparedPlatformRefreshData::Connectivity(refresh),
                                        ) => {
                                            let shell = self
                                                .internal_shell
                                                .as_mut()
                                                .ok_or("internal shell unavailable")?;
                                            let network_available = refresh.network.available;
                                            let bluetooth_available = refresh.bluetooth.available;
                                            let mut changed = shell.apply_system_status_update(
                                                crate::platform::SystemStatusUpdate::Network(refresh.network),
                                            );
                                            changed.extend(shell.apply_system_status_update(
                                                crate::platform::SystemStatusUpdate::Bluetooth(refresh.bluetooth),
                                            ));
                                            (changed, network_available, bluetooth_available, false,
                                                false, false, false, 0, 0, 0,
                                                false, None, None, None, None, 0, false,
                                                false, 0, 0, 0,
                                                refresh.partial, true)
                                        }
                                        (
                                            nickel_remote_control::diagnostics::PlatformRefreshDomain::Audio,
                                            PreparedPlatformRefreshData::Audio(refresh),
                                        ) => {
                                            let shell = self
                                                .internal_shell
                                                .as_mut()
                                                .ok_or("internal shell unavailable")?;
                                            let available = refresh.audio.available;
                                            let changed = shell.apply_system_status_update(
                                                crate::platform::SystemStatusUpdate::Audio(refresh.audio),
                                            );
                                            (changed, false, false, available,
                                                false, false, false, 0, 0, 0,
                                                false, None, None, None, None, 0, false,
                                                false, 0, 0, 0,
                                                refresh.partial, true)
                                        }
                                        (
                                            nickel_remote_control::diagnostics::PlatformRefreshDomain::Peripherals,
                                            PreparedPlatformRefreshData::Peripherals(refresh),
                                        ) => {
                                            (Vec::new(), false, false, false,
                                                refresh.printers_available, refresh.volumes_available,
                                                refresh.filesystems_available, refresh.printer_count,
                                                refresh.volume_count, refresh.filesystem_count,
                                                false, None, None, None, None, 0, false,
                                                false, 0, 0, 0,
                                                refresh.partial, false)
                                        }
                                        (
                                            nickel_remote_control::diagnostics::PlatformRefreshDomain::Maintenance,
                                            PreparedPlatformRefreshData::Maintenance(refresh),
                                        ) => {
                                            (Vec::new(), false, false, false,
                                                false, false, false, 0, 0, 0,
                                                refresh.maintenance_available, refresh.updates_available,
                                                refresh.restart_required, refresh.firewall_healthy,
                                                refresh.malware_protection_healthy,
                                                refresh.known_permission_states,
                                                refresh.secure_storage_status_available,
                                                false, 0, 0, 0,
                                                refresh.partial, false)
                                        }
                                        (
                                            nickel_remote_control::diagnostics::PlatformRefreshDomain::DefaultAssociations,
                                            PreparedPlatformRefreshData::DefaultAssociations(refresh),
                                        ) => {
                                            (Vec::new(), false, false, false,
                                                false, false, false, 0, 0, 0,
                                                false, None, None, None, None, 0, false,
                                                refresh.associations_available,
                                                refresh.targets_queried,
                                                refresh.effective_associations,
                                                refresh.directly_writable_associations,
                                                refresh.partial, false)
                                        }
                                        _ => return Err("platform refresh data changed before commit".into()),
                                    };
                                changed.sort_unstable();
                                changed.dedup();
                                self.remote_platform_refresh_generation = generation;
                                platform_refresh_outcome = Some(
                                    nickel_remote_control::diagnostics::PlatformRefreshOutcome {
                                        domain,
                                        generation,
                                        observation_started_at_us: prepared
                                            .observation_started
                                            .saturating_duration_since(self.start_time)
                                            .as_micros()
                                            .min(u128::from(u64::MAX)) as u64,
                                        observed_at_us: prepared
                                            .observed
                                            .saturating_duration_since(self.start_time)
                                            .as_micros()
                                            .min(u128::from(u64::MAX)) as u64,
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
                                    },
                                );
                                if let Some(outcome) = platform_refresh_outcome.as_ref() {
                                    self.remote_platform_refreshes
                                        .retain(|entry| entry.domain != outcome.domain);
                                    self.remote_platform_refreshes.push(outcome.clone());
                                    self.remote_desktop_events.record(
                                        nickel_remote_control::desktop_events::DesktopEventKind::PlatformRefreshCompleted {
                                            domain: outcome.domain,
                                            generation: outcome.generation,
                                            partial: outcome.partial,
                                        },
                                        self.start_time.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
                                    );
                                }
                                self.sync_internal_shell_changes(Some(&changed));
                            }
                        }
                        #[cfg(feature = "backend-udev")]
                        self.invalidate_native_outputs();
                        self.request_output_redraw();
                        self.remote_observation_generation =
                            self.remote_observation_generation.saturating_add(1);
                        Ok(
                            nickel_remote_control::diagnostics::DiagnosticActionOutcome {
                                action,
                                observation_generation: self.remote_observation_generation,
                                submitted_at_us: self
                                    .start_time
                                    .elapsed()
                                    .as_micros()
                                    .min(u128::from(u64::MAX))
                                    as u64,
                                presentation_confirmed: false,
                                output_identification,
                                application_inventory_refresh,
                                platform_refresh: platform_refresh_outcome,
                            },
                        )
                    }
                };
                let result = if refresh_applications || refresh_platform {
                    permit.with_debug_input(protected, effect)
                } else {
                    permit.with_debug(protected, effect)
                };
                if let Ok(outcome) = &result {
                    use nickel_remote_control::desktop_events::ProductionEffectOutcome;
                    let completion = if outcome.output_identification.is_some()
                        || outcome
                            .application_inventory_refresh
                            .as_ref()
                            .is_some_and(|refresh| refresh.reconciliation_confirmed)
                        || outcome
                            .platform_refresh
                            .as_ref()
                            .is_some_and(|refresh| refresh.reconciliation_confirmed)
                    {
                        ProductionEffectOutcome::UiUpdated
                    } else if matches!(
                        outcome.action,
                        nickel_remote_control::diagnostics::DiagnosticAction::Repaint
                    ) {
                        ProductionEffectOutcome::Requested
                    } else {
                        ProductionEffectOutcome::Confirmed
                    };
                    self.record_remote_production_effect_outcome(
                        &permit,
                        nickel_remote_control::desktop_events::ProductionEffectKind::DiagnosticAction,
                        completion,
                    );
                }
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::ListSurfaces { permit, reply } => {
                #[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
                let result = self.list_remote_surfaces(&permit);
                #[cfg(not(any(feature = "backend-udev", feature = "backend-winit")))]
                let result = {
                    let _ = permit;
                    Err("shell surfaces unavailable".into())
                };
                let _ = reply.try_send(result);
            }
            RemoteDesktopRequest::ValidateSurfaceCapture {
                permit,
                id,
                generation,
                reply,
            } => {
                #[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
                let result = self.with_surface_capture_authority(
                    &nickel_remote_control::leases::ResourceId { id, generation },
                    &permit,
                    || Ok(()),
                );
                #[cfg(not(any(feature = "backend-udev", feature = "backend-winit")))]
                let result = {
                    let _ = (permit, id, generation);
                    Err("capture renderer is unavailable".into())
                };
                let _ = reply.try_send(result);
            }
            RemoteDesktopRequest::CaptureSurface {
                permit,
                id,
                generation,
                reply,
            } => {
                #[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
                self.enqueue_remote_capture(
                    permit,
                    remote_capture::CaptureTarget::Surface(
                        nickel_remote_control::leases::ResourceId { id, generation },
                    ),
                    reply,
                );
                #[cfg(not(any(feature = "backend-udev", feature = "backend-winit")))]
                {
                    let _ = (permit, id, generation);
                    let _ = reply.try_send(Err("capture renderer is unavailable".into()));
                }
            }
            RemoteDesktopRequest::ValidateCapture {
                permit,
                id,
                generation,
                reply,
            } => {
                #[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
                let result = id
                    .parse::<u64>()
                    .map_err(|_| "invalid window identity".to_owned())
                    .and_then(|id| {
                        if id != generation {
                            return Err("stale window generation".into());
                        }
                        self.with_capture_authority(WindowId(id), &permit, || Ok(()))
                    });
                #[cfg(not(any(feature = "backend-udev", feature = "backend-winit")))]
                let result = {
                    let _ = (permit, id, generation);
                    Err("capture renderer is unavailable".into())
                };
                let _ = reply.try_send(result);
            }
            RemoteDesktopRequest::Capture {
                permit,
                id,
                generation,
                reply,
            } => {
                #[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
                match id.parse::<u64>() {
                    Ok(parsed) if parsed == generation && id == parsed.to_string() => self
                        .enqueue_remote_capture(
                            permit,
                            remote_capture::CaptureTarget::Window(WindowId(parsed)),
                            reply,
                        ),
                    _ => {
                        let _ = reply.try_send(Err("invalid or stale window identity".into()));
                    }
                }
                #[cfg(not(any(feature = "backend-udev", feature = "backend-winit")))]
                {
                    let _ = (permit, id, generation);
                    let _ = reply.try_send(Err("capture renderer is unavailable".into()));
                }
            }
            RemoteDesktopRequest::Keyboard {
                permit,
                id,
                generation,
                action,
                reply,
            } => {
                let result =
                    self.dispatch_remote_keyboard(permit, id, generation, action, reply.clone());
                if !matches!(result, Ok(true)) {
                    let _ = reply.try_send(result.map(|_| ()));
                }
            }
            RemoteDesktopRequest::Pointer {
                permit,
                id,
                generation,
                x,
                y,
                action,
                reply,
            } => {
                let result = self.dispatch_remote_pointer(permit, id, generation, x, y, action);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::SemanticAction {
                permit,
                request,
                reply,
            } => {
                let result = self.remote_semantic_action(&permit, request);
                if let Ok(changed) = &result {
                    self.record_remote_production_effect_outcome(
                        &permit,
                        nickel_remote_control::desktop_events::ProductionEffectKind::SemanticAction,
                        if *changed {
                            nickel_remote_control::desktop_events::ProductionEffectOutcome::UiUpdated
                        } else {
                            nickel_remote_control::desktop_events::ProductionEffectOutcome::Confirmed
                        },
                    );
                }
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::ShellSemanticStep {
                permit,
                origin,
                output,
                tree_generation,
                prepared,
                reply,
            } => {
                let result = self.commit_shell_semantic_step(
                    &permit,
                    &origin,
                    &output,
                    tree_generation,
                    prepared,
                );
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::SurfaceSemanticAction {
                permit,
                request,
                reply,
            } => {
                let result = self.remote_surface_semantic_action(&permit, request);
                if result.as_ref().is_ok_and(|plan| plan.changed) {
                    self.record_remote_production_effect_outcome(
                        &permit,
                        nickel_remote_control::desktop_events::ProductionEffectKind::SemanticAction,
                        nickel_remote_control::desktop_events::ProductionEffectOutcome::UiUpdated,
                    );
                }
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::Semantics {
                permit,
                id,
                generation,
                reply,
            } => {
                let result = self.remote_window_semantics(&permit, &id, generation);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::PrepareNativeSemantics {
                permit,
                id,
                generation,
                application_root,
                reply,
            } => {
                let result =
                    self.native_accessibility_proof(&permit, &id, generation, application_root);
                let _ = reply.try_send(result);
            }
            RemoteDesktopRequest::ValidateNativeSemantics {
                permit,
                proof,
                reply,
            } => {
                let result = self.validate_native_accessibility(&permit, &proof);
                let _ = reply.try_send(result);
            }
            RemoteDesktopRequest::FinishNativeSemantics {
                permit,
                proof,
                result,
                reply,
            } => {
                let result = self.finish_native_accessibility(&permit, &proof, result);
                let _ = reply.try_send(result);
            }
            RemoteDesktopRequest::SurfaceSemantics {
                permit,
                id,
                generation,
                reply,
            } => {
                let result = self.remote_surface_semantics(&permit, &id, generation);
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::Diagnostic { permit, reply } => {
                if let Err(error) =
                    permit.with_debug(self.locked || self.shell_recovery_visible(), || Ok(()))
                {
                    let _ = reply.send(Err(error));
                    return;
                }
                // Match the local Settings query: refresh topology before
                // publishing a version for compare-and-set transactions. Shell
                // reconciliation consults control state, so it runs outside the
                // authority lock; collection below revalidates the permit.
                self.refresh_output_topology_generation();
                let lease_metrics = permit.lease_metrics_snapshot(
                    self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
                );
                let result =
                    permit.with_debug(self.locked || self.shell_recovery_visible(), || {
                        use nickel_remote_control::diagnostics::*;
                        self.remote_observation_generation =
                            self.remote_observation_generation.saturating_add(1);
                        let all_windows = self
                            .remote_protocol_windows()
                            .into_iter()
                            .filter(|window| {
                                !self.shell_owned_windows.contains(&WindowId(window.id.0))
                            })
                            .collect::<Vec<_>>();
                        let all_outputs = self.protocol_outputs();
                        let (shell_surfaces, shell_surfaces_truncated) =
                            self.remote_shell_surface_diagnostics();
                        let truncated = shell_surfaces_truncated
                            || self.workspaces.ordered().len() > MAX_DIAGNOSTIC_WORKSPACES
                            || all_windows.len() > MAX_DIAGNOSTIC_WINDOWS
                            || all_outputs.len() > MAX_DIAGNOSTIC_OUTPUTS;
                        let windows = all_windows
                            .into_iter()
                            .take(MAX_DIAGNOSTIC_WINDOWS)
                            .map(|window| self.remote_window_summary(window))
                            .collect::<Vec<_>>();
                        let geometry = |value: nickel_session_protocol::Geometry| {
                            [value.x, value.y, value.width, value.height]
                        };
                        let observed_at_us =
                            self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64;
                        let internal_applications =
                            self.remote_internal_application_diagnostics(&windows);
                        let internal_renderers = self.remote_internal_renderer_diagnostics(
                            &internal_applications,
                            observed_at_us,
                        );
                        let shell_renderers =
                            self.remote_shell_renderer_diagnostics(observed_at_us);
                        let shell_image_cache = self.internal_shell.as_ref().map(|shell| {
                            let cache = shell.image_cache_diagnostics_for_previews(|id| {
                                windows.iter().any(|window| window.generation == id.0)
                            });
                            ShellImageCacheDiagnostic {
                                observation_generation: self.remote_observation_generation,
                                observed_at_us,
                                launcher_icon_entries: cache.launcher_icon_entries as u64,
                                launcher_icon_bytes: cache.launcher_icon_bytes as u64,
                                wallpaper_entries: cache.wallpaper_entries as u64,
                                wallpaper_bytes: cache.wallpaper_bytes as u64,
                                tray_entries: cache.tray_entries as u64,
                                tray_bytes: cache.tray_bytes as u64,
                                preview_entries: cache.preview_entries as u64,
                                preview_bytes: cache.preview_bytes as u64,
                            }
                        });
                        let projected_resources = ProjectedResourceDiagnostic {
                            observation_generation: self.remote_observation_generation,
                            observed_at_us,
                            renderer_surfaces: internal_renderers
                                .len()
                                .saturating_add(shell_renderers.len())
                                as u64,
                            software_frame_bytes: internal_renderers
                                .iter()
                                .chain(&shell_renderers)
                                .fold(0_u64, |total, renderer| {
                                    total.saturating_add(renderer.software_frame_bytes)
                                }),
                            fallback_raster_bytes: internal_renderers
                                .iter()
                                .chain(&shell_renderers)
                                .fold(0_u64, |total, renderer| {
                                    total.saturating_add(renderer.fallback_raster_bytes)
                                }),
                            shell_image_entries: shell_image_cache.as_ref().map_or(0, |cache| {
                                cache
                                    .launcher_icon_entries
                                    .saturating_add(cache.wallpaper_entries)
                                    .saturating_add(cache.tray_entries)
                                    .saturating_add(cache.preview_entries)
                            }),
                            shell_image_bytes: shell_image_cache.as_ref().map_or(0, |cache| {
                                cache
                                    .launcher_icon_bytes
                                    .saturating_add(cache.wallpaper_bytes)
                                    .saturating_add(cache.tray_bytes)
                                    .saturating_add(cache.preview_bytes)
                            }),
                        };
                        let input = self.remote_input_diagnostic(
                            &windows,
                            &internal_applications,
                            self.remote_observation_generation,
                            observed_at_us,
                        );
                        Ok(DiagnosticSnapshot {
                            observation_generation: self.remote_observation_generation,
                            observed_at_us,
                            workspaces: self.remote_workspace_diagnostics(&windows),
                            focused_window: input
                                .keyboard
                                .as_ref()
                                .and_then(|device| device.focused_window.clone()),
                            input,
                            shortcuts: remote_diagnostics::shortcut_diagnostic(Some(&self.hotkeys), self.remote_observation_generation, observed_at_us),
                            stacking_front_to_back: windows
                                .iter()
                                .map(|window| window.id.clone())
                                .collect(),
                            internal_applications,
                            internal_renderers,
                            shell_renderers,
                            shell_image_cache,
                            shared_presenter_cache: None,
                            projected_resources,
                            pending_effects: self.remote_pending_effects_diagnostic(
                                self.remote_observation_generation,
                                observed_at_us,
                            ),
                            shell_surfaces,
                            windows,
                            outputs: all_outputs
                                .into_iter()
                                .take(MAX_DIAGNOSTIC_OUTPUTS)
                                .map(|output| OutputDiagnostic {
                                    generation: self
                                        .remote_output_generations
                                        .get(&output.name)
                                        .map_or(0, |(_, generation)| *generation),
                                    name: output.name.chars().take(128).collect(),
                                    geometry: geometry(output.geometry),
                                    work_area: geometry(output.work_area),
                                    scale_120: output.scale_120,
                                    primary: output.primary,
                                    enabled: output.enabled,
                                })
                                .collect(),
                            preview: PreviewDiagnostic {
                                presentation_generation: self
                                    .preview_counters
                                    .presentation_generation,
                                readback_bytes: self.preview_counters.readback_bytes,
                                capture_failures: self.preview_counters.capture_failures,
                                native_presentation_generation: None,
                                native_presentation_failures: None,
                            },
                            metrics: permit.operation_metrics_snapshot(),
                            admission: permit.admission_snapshot(),
                            lease_metrics,
                            platform: self.remote_platform_diagnostic(
                                self.remote_observation_generation,
                                observed_at_us,
                            ),
                            platform_refreshes: self
                                .remote_platform_refreshes
                                .iter()
                                .map(|refresh| refresh.retained_at(observed_at_us))
                                .collect(),
                            application_inventory_refresh: self
                                .remote_application_inventory_refresh
                                .as_ref()
                                .map(|refresh| refresh.retained_at(observed_at_us)),
                            codex_feature: self.remote_codex_feature_diagnostic(
                                self.remote_observation_generation,
                                observed_at_us,
                            ),
                            shell_behavior: self.remote_shell_behavior_diagnostic(
                                self.remote_observation_generation,
                                observed_at_us,
                            ),
                            settings_worker: self.remote_settings_staging.snapshot(),
                            diagnostic_worker: self.remote_diagnostic_staging.snapshot(),
                            application_launch: self.remote_application_launch_diagnostic(),
                            external_accessibility: self
                                .remote_external_accessibility
                                .as_ref()
                                .map(|observation| observation.retained_at(observed_at_us)),
                            recent_events: self.remote_desktop_events.snapshot(),
                            diagnostic_logs: self.remote_diagnostic_logs(),
                            frame_trace: self
                                .remote_frame_trace
                                .as_ref()
                                .filter(|trace| trace.owned_by(&permit))
                                .map(|trace| trace.snapshot()),
                            trace_lifecycle: self.remote_trace_lifecycle(&permit),
                            truncated,
                            unavailable_domains: {
                                [
                                    "shell_transients_without_host_owned_protection_and_codex_content",
                                    "native_gpu_renderer_timing",
                                    "shell_gpu_resources_and_external_renderer_resources_and_shared_caches",
                                    "other_compositor_event_categories",
                                    "structured_log_details_and_other_trace_categories",
                                ]
                                .into_iter()
                                .map(str::to_owned)
                                .collect::<Vec<_>>()
                            },
                        })
                    });
                let _ = reply.send(result);
            }
            RemoteDesktopRequest::List { permit, reply } => {
                if let Err(error) = permit.check_live() {
                    let _ = reply.send(Err(error));
                    return;
                }
                let windows = self
                    .remote_protocol_windows()
                    .into_iter()
                    .filter_map(|window| {
                        let identity = nickel_remote_control::leases::ResourceId {
                            id: window.id.0.to_string(),
                            generation: window.id.0,
                        };
                        let output = self.remote_window_output(WindowId(identity.generation));
                        let verified_application =
                            self.remote_verified_application(WindowId(identity.generation));
                        let evidence = nickel_remote_control::leases::ResourceEvidence {
                            window: Some(&identity),
                            surface: None,
                            verified_application: verified_application.as_deref(),
                            output: output.as_ref(),
                            authorized_surface_ancestors: &[],
                            protected: self.remote_window_is_protected(WindowId(window.id.0)),
                        };
                        permit
                            .with_resource(&evidence, || Ok(self.remote_window_summary(window)))
                            .ok()
                    })
                    .collect();
                let _ = reply.send(Ok(windows));
            }
            RemoteDesktopRequest::WindowAction {
                permit,
                id,
                generation,
                action,
                reply,
            } => {
                let result = id
                    .parse::<u64>()
                    .map_err(|_| "invalid window identity".to_owned())
                    .and_then(|numeric| {
                        if generation != numeric {
                            return Err("stale window generation".into());
                        }
                        let id = WindowId(numeric);
                        if !self
                            .remote_protocol_windows()
                            .iter()
                            .any(|window| window.id.0 == numeric)
                        {
                            return Err("window is not available for remote control".into());
                        }
                        let identity = nickel_remote_control::leases::ResourceId {
                            id: numeric.to_string(),
                            generation,
                        };
                        let output = self.remote_window_output(WindowId(identity.generation));
                        let verified_application =
                            self.remote_verified_application(WindowId(identity.generation));
                        let evidence = nickel_remote_control::leases::ResourceEvidence {
                            window: Some(&identity),
                            surface: None,
                            verified_application: verified_application.as_deref(),
                            output: output.as_ref(),
                            authorized_surface_ancestors: &[],
                            protected: self.remote_window_is_protected(id),
                        };
                        action.validate()?;
                        if let nickel_remote_control::window_actions::WindowAction::MoveToWorkspace { workspace } = action {
                            return self.remote_move_window_to_workspace(&permit, &evidence, id, workspace);
                        }
                        if let nickel_remote_control::window_actions::WindowAction::SetBounds {
                            x,
                            y,
                            width,
                            height,
                        } = action
                        {
                            let destination = self
                                .output_geometry_for_bounds(Geometry {
                                    x,
                                    y,
                                    width: width as i32,
                                    height: height as i32,
                                })
                                .and_then(|geometry| self.output_name_matching_geometry(geometry))
                                .and_then(|name| self.remote_output_identity(name))
                                .ok_or("requested window bounds do not intersect an output")?;
                            // Source and destination use the same production output-membership
                            // policy. The owner cannot change the layout between these checks;
                            // the input reservation below rechecks cancellation and expiry.
                            permit.with_resource(
                                &nickel_remote_control::leases::ResourceEvidence {
                                    output: Some(&destination),
                                    ..evidence
                                },
                                || Ok(()),
                            )?;
                        }
                        permit.with_input(&evidence, || {
                            use nickel_remote_control::window_actions::{
                                WindowAction, WindowOutcome,
                            };
                            let window = self.registry_native_window(id);
                            let fullscreen = window
                                .as_ref()
                                .is_some_and(|window| self.is_fullscreen_window(window));
                            let maximized = window
                                .as_ref()
                                .is_some_and(|window| self.is_maximized_window(window));
                            match action {
                                WindowAction::MoveToWorkspace { .. } => return Err("workspace action was not routed to its owner".into()),
                                WindowAction::SetBounds {
                                    x,
                                    y,
                                    width,
                                    height,
                                } => {
                                    if fullscreen || maximized {
                                        return Err(
                                            "restore the window before changing its bounds".into(),
                                        );
                                    }
                                    if self
                                        .seat
                                        .get_pointer()
                                        .is_none_or(|pointer| pointer.is_grabbed())
                                    {
                                        return Err(
                                            "pointer is busy with another interaction".into()
                                        );
                                    }
                                    let window = window.ok_or("native window is unavailable")?;
                                    let geometry = Geometry {
                                        x,
                                        y,
                                        width: width as i32,
                                        height: height as i32,
                                    };
                                    Self::configure_window(&window, geometry);
                                    if let Some(surface) = window.x11_surface() {
                                        surface
                                            .configure(Rectangle::new(
                                                (x, y).into(),
                                                (width as i32, height as i32).into(),
                                            ))
                                            .map_err(|error| {
                                                format!("window configuration failed: {error}")
                                            })?;
                                    }
                                    if let Some((_, location)) = self.minimized_windows.get_mut(&id)
                                    {
                                        *location = (x, y).into();
                                    } else if let Some((_, location)) =
                                        self.workspace_hidden_windows.get_mut(&id)
                                    {
                                        *location = (x, y).into();
                                    } else {
                                        self.map_compositor_moved_window(
                                            window,
                                            (x, y).into(),
                                            false,
                                        );
                                    }
                                    self.notify_protocol_snapshot();
                                    self.display_handle.flush_clients().map_err(|error| {
                                        format!("window configuration flush failed: {error}")
                                    })?;
                                }
                                WindowAction::Activate => self.activate_window(id),
                                WindowAction::Close => self.close_window(id),
                                WindowAction::Minimize => self.minimize_window(id),
                                WindowAction::Maximize => {
                                    self.activate_window(id);
                                    if fullscreen {
                                        self.toggle_fullscreen_window(id);
                                    }
                                    if !maximized {
                                        self.maximize_window(id);
                                    }
                                }
                                WindowAction::Restore => {
                                    self.activate_window(id);
                                    if fullscreen {
                                        self.toggle_fullscreen_window(id);
                                    }
                                    if maximized {
                                        self.maximize_window(id);
                                    }
                                }
                                WindowAction::Fullscreen => {
                                    self.activate_window(id);
                                    if !fullscreen {
                                        self.toggle_fullscreen_window(id);
                                    }
                                }
                                WindowAction::ExitFullscreen => {
                                    if fullscreen {
                                        self.toggle_fullscreen_window(id);
                                    }
                                }
                            }
                            let window = self
                                .remote_protocol_windows()
                                .into_iter()
                                .find(|window| window.id.0 == numeric)
                                .map(|window| self.remote_window_summary(window));
                            Ok(WindowOutcome::observed(action, window))
                        })
                    });
                if let Ok(outcome) = &result {
                    self.record_remote_production_effect_outcome(
                        &permit,
                        nickel_remote_control::desktop_events::ProductionEffectKind::WindowAction,
                        if outcome.confirmed {
                            nickel_remote_control::desktop_events::ProductionEffectOutcome::Confirmed
                        } else {
                            nickel_remote_control::desktop_events::ProductionEffectOutcome::Requested
                        },
                    );
                }
                let _ = reply.send(result);
            }
        }
    }

    pub(crate) fn enable_internal_shell(
        &mut self,
        host: std::sync::Arc<dyn crate::session_host::SessionHost>,
    ) -> Result<(), String> {
        self.enable_internal_shell_with_system_updates(
            host,
            crate::platform::system_status_receiver(),
        )
    }

    fn enable_internal_shell_with_system_updates(
        &mut self,
        host: std::sync::Arc<dyn crate::session_host::SessionHost>,
        platform_updates: crate::platform::status_mailbox::StatusReceiver,
    ) -> Result<(), String> {
        use crate::{internal_shell::InternalShellCoordinator, winit_shell::PanelEdge};

        let mut shell = InternalShellCoordinator::new(host, PanelEdge::Bottom)?;
        self.publish_internal_keyboard_snapshot();
        // Apply updates that were already available without delaying shell
        // construction. Later transitions remain calloop-driven.
        for update in platform_updates.drain() {
            let _ = shell.apply_system_status_update(update);
        }
        let (ping, source) = smithay::reexports::calloop::ping::make_ping()
            .map_err(|error| format!("could not create system status wake: {error}"))?;
        platform_updates.set_waker(move || ping.ping());
        let token = self
            .event_loop_handle
            .insert_source(source, move |_, _, state| {
                let mut changed = Vec::new();
                for update in platform_updates.drain() {
                    if let Some(shell) = state.internal_shell.as_mut() {
                        changed.extend(shell.apply_system_status_update(update));
                    }
                }
                changed.sort_unstable();
                changed.dedup();
                if !changed.is_empty() {
                    state.sync_internal_shell_changes(Some(&changed));
                    state.request_output_redraw();
                }
            })
            .map_err(|error| format!("could not register internal system feed: {error}"))?;
        // Retire quiet subscriptions as well as their event-loop wake on replacement.
        if let Some(previous) = self.internal_system_status_source.replace(token) {
            self.event_loop_handle.remove(previous);
        }
        let feature_settings =
            nickel_core::optional_features::OptionalFeatureSettings::load_default();
        self.remote_codex_runtime_generation = feature_settings.codex_generation;
        let codex_enabled = feature_settings.effective_codex_enabled();
        if codex_enabled {
            use nickel_core::optional_features::{
                CodexAvailabilityProjection, FeatureHealth, FeatureInstallation, FeatureSupport,
            };
            // The in-process host is the selected Codex installation. Publish a
            // recoverable loading projection before the first menu is opened;
            // otherwise LiveShell's unavailable default hides the only affordance
            // capable of starting the host and discovering its real state.
            shell.apply_codex_projection(CodexAvailabilityProjection::new(
                FeatureSupport::Supported,
                FeatureInstallation::Installed,
                true,
                FeatureHealth::Loading,
                feature_settings.codex_generation,
                Some("Checking the selected Codex backend…".into()),
            ));
        }
        self.internal_codex = codex_enabled.then(|| {
            crate::internal_codex::InternalCodexHost::new(
                feature_settings.clone(),
                shell.semantic_theme(),
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/")),
            )
        });
        self.internal_shell = Some(shell);
        self.reconcile_internal_shell_outputs();
        if codex_enabled {
            let outputs = self.internal_outputs();
            let fallback = self.resolve_interaction_output(InvocationSource::RecentInteraction);
            let placement =
                internal_codex_project_menu_placement(None, &outputs, fallback.as_deref());
            if let Some(mut host) = self.internal_codex.take() {
                match host.ensure_project_menu(&mut self.internal_ui, placement) {
                    Ok(_) => {
                        host.set_project_menu_visible(&mut self.internal_ui, false);
                        if let Some(shell) = self.internal_shell.as_mut() {
                            host.sync_shell_projection(&self.internal_ui, shell);
                        }
                    }
                    Err(error) => tracing::warn!(%error, "could not start Codex project discovery"),
                }
                self.internal_codex = Some(host);
            }
            self.schedule_internal_shell_deadline();
        }
        Ok(())
    }

    /// Arm exactly one compositor-loop wakeup for the shell's earliest real
    /// deadline. Re-arming the same deadline is a no-op, so damage and input
    /// paths may call this freely without recreating a frame-rate poller.
    pub(crate) fn schedule_internal_shell_deadline(&mut self) {
        let shell_deadline = self
            .internal_shell
            .as_ref()
            .and_then(crate::internal_shell::InternalShellCoordinator::next_deadline);
        let codex_deadline = self
            .internal_codex
            .as_ref()
            .and_then(|host| host.next_deadline(&self.internal_ui));
        let deadline = shell_deadline.into_iter().chain(codex_deadline).min();
        self.arm_internal_shell_timer(deadline);
    }

    /// Wake the shell once after input or an externally-driven state change.
    /// The callback replaces this immediate wakeup with the next application
    /// deadline (if any).
    pub(super) fn wake_internal_shell(&mut self) {
        if self.internal_shell.is_some() {
            self.arm_internal_shell_timer(Some(Instant::now()));
        }
    }

    fn arm_internal_shell_timer(&mut self, deadline: Option<Instant>) {
        if self.internal_shell_timer.deadline == deadline {
            return;
        }
        if let Some(token) = self.internal_shell_timer.token.take() {
            self.event_loop_handle.remove(token);
            self.internal_shell_timer.counters.cancelled = self
                .internal_shell_timer
                .counters
                .cancelled
                .saturating_add(1);
        }
        self.internal_shell_timer.deadline = deadline;
        self.internal_shell_timer.generation = self.internal_shell_timer.generation.wrapping_add(1);
        let generation = self.internal_shell_timer.generation;
        let Some(deadline) = deadline else {
            return;
        };
        match self.event_loop_handle.insert_source(
            smithay::reexports::calloop::timer::Timer::from_deadline(deadline),
            move |_, _, state| {
                if state.internal_shell_timer.generation != generation {
                    return smithay::reexports::calloop::timer::TimeoutAction::Drop;
                }
                state.internal_shell_timer.deadline = None;
                state.internal_shell_timer.token = None;
                state.internal_shell_timer.counters.fired =
                    state.internal_shell_timer.counters.fired.saturating_add(1);
                state.internal_shell_timer.counters.polls =
                    state.internal_shell_timer.counters.polls.saturating_add(1);
                state.poll_internal_shell(Instant::now());
                state.schedule_internal_shell_deadline();
                let counters = state.internal_shell_timer_counters();
                tracing::trace!(
                    armed = counters.armed,
                    cancelled = counters.cancelled,
                    fired = counters.fired,
                    polls = counters.polls,
                    redraw_requests = counters.redraw_requests,
                    next_deadline = ?state.internal_shell_timer.deadline,
                    "internal shell one-shot timer counters"
                );
                smithay::reexports::calloop::timer::TimeoutAction::Drop
            },
        ) {
            Ok(token) => {
                self.internal_shell_timer.token = Some(token);
                self.internal_shell_timer.counters.armed =
                    self.internal_shell_timer.counters.armed.saturating_add(1);
            }
            Err(error) => {
                self.internal_shell_timer.deadline = None;
                tracing::error!(%error, "could not schedule internal shell deadline");
            }
        }
    }

    pub(crate) fn internal_shell_timer_counters(&self) -> InternalShellTimerCounters {
        self.internal_shell_timer.counters
    }

    fn internal_outputs(&self) -> Vec<(crate::internal_shell::InternalOutput, i32, i32)> {
        let mut outputs = self
            .space
            .outputs()
            .filter_map(|output| {
                let geometry = self.space.output_geometry(output)?;
                Some((
                    crate::internal_shell::InternalOutput {
                        x: geometry.loc.x,
                        y: geometry.loc.y,
                        name: output.name(),
                        width: geometry.size.w.max(0) as u32,
                        height: geometry.size.h.max(0) as u32,
                        scale: output.current_scale().fractional_scale() as f32,
                    },
                    geometry.loc.x,
                    geometry.loc.y,
                ))
            })
            .collect::<Vec<_>>();
        if let Some(primary) = self.primary_output_name.as_deref()
            && let Some(index) = outputs
                .iter()
                .position(|(output, _, _)| output.name == primary)
        {
            outputs.swap(0, index);
        }
        outputs
    }

    pub(crate) fn reconcile_internal_shell_outputs(&mut self) {
        self.invalidate_remote_shell_actions();
        // Runtime slots are replaced below, but a surviving coordinator surface
        // must retain keyboard ownership across output reconciliation.
        let focused_owner = self.internal_ui.focused().and_then(|focused| {
            self.internal_shell_surfaces
                .iter()
                .find_map(|(owner, runtime)| (*runtime == focused).then_some(*owner))
        });
        self.pending_desktop_scenes.clear();
        // Deliver cancellation while old runtime-to-coordinator identities still
        // exist. Tombstones retain ownership of eventual releases after rebuild.
        self.internal_ui.retire_normalized_touch_surfaces();
        self.flush_internal_shell_input();
        let outputs = self.internal_outputs();
        for id in self
            .internal_shell_surfaces
            .drain()
            .map(|(_, runtime)| runtime)
            .collect::<Vec<_>>()
        {
            self.internal_ui.remove(id);
        }
        let Some(shell) = self.internal_shell.as_mut() else {
            return;
        };
        shell.set_outputs(
            &outputs
                .iter()
                .map(|(output, _, _)| output.clone())
                .collect::<Vec<_>>(),
        );

        for surface in shell.surfaces().to_vec() {
            if !shell.visible(surface.id) {
                continue;
            }
            let Some(scene) = shell.scene(surface.id) else {
                continue;
            };
            let placement = internal_shell_surface_placement(
                surface.role,
                surface.output.as_deref(),
                surface.size,
                &outputs,
                self.launcher_output_name.as_deref(),
            );
            let scale = surface
                .output
                .as_deref()
                .and_then(|name| outputs.iter().find(|(output, _, _)| output.name == name))
                .map_or(1.0, |(output, _, _)| output.scale);
            let runtime_id = self.internal_ui.insert_scene(scene, placement, scale);
            self.internal_shell_surfaces.insert(surface.id, runtime_id);
        }
        if let Some(runtime) =
            focused_owner.and_then(|owner| self.internal_shell_surfaces.get(&owner).copied())
        {
            self.focus_internal_surface(runtime);
        }
        self.schedule_internal_ui_frame();
        self.wake_internal_shell();
        self.sync_remote_control_indicators();
    }

    pub(crate) fn poll_internal_shell(&mut self, now: Instant) {
        if self.internal_ui.take_surface_retirement() {
            self.schedule_remote_resource_retirement();
        }
        // Focus may change without another device event. Consume deferred lifecycle
        // batches here, outside Smithay's focus callback/keyboard lock.
        self.flush_internal_shell_input();
        self.publish_internal_keyboard_snapshot();
        if self.internal_shell.is_some() {
            let snapshot = self.protocol_snapshot();
            *self.internal_projection_outputs.write().unwrap() = snapshot.outputs.clone();
            let shell = self.internal_shell.as_mut().unwrap();
            let mut changed = shell.apply_session_snapshot_changes(snapshot);
            changed.extend(shell.poll(now));
            let actions = shell.drain_file_actions();
            let shell_changed = !changed.is_empty();
            let codex_menu_visible = shell.codex_project_menu_visible();
            let codex_menu_anchor = shell
                .popover_anchor(nickel_session_protocol::AnchorSide::Above)
                .and_then(|(role, anchor)| {
                    (role == nickel_session_protocol::ShellRole::ProjectMenu).then_some(anchor)
                });
            let requested_codex_project = shell.take_requested_codex_project();
            let _ = shell;
            let outputs = self.internal_outputs();
            let menu_output = codex_menu_anchor
                .as_ref()
                .map(|anchor| anchor.output.as_str())
                .or_else(|| {
                    self.internal_codex
                        .as_ref()
                        .and_then(crate::internal_codex::InternalCodexHost::project_menu)
                        .and_then(|id| self.internal_ui.placement(id))
                        .and_then(|placement| placement.output.as_deref())
                })
                .map(str::to_owned);
            let fallback = self.resolve_interaction_output(InvocationSource::RecentInteraction);
            if shell_changed {
                self.sync_internal_shell_changes(Some(&changed));
            }
            for action in actions {
                self.apply_internal_file_action(action);
            }
            if codex_menu_visible {
                let placement = internal_codex_project_menu_placement(
                    codex_menu_anchor.as_ref(),
                    &outputs,
                    fallback.as_deref(),
                );
                if let Err(error) = self.show_internal_codex_project_menu(placement) {
                    tracing::warn!(%error, "could not host Codex project menu internally");
                }
            } else if let Some(host) = self.internal_codex.take() {
                if let Some(menu) = host.project_menu() {
                    self.internal_ui.set_visible(menu, false);
                }
                self.internal_codex = Some(host);
            }
            if let Some(project_id) = requested_codex_project
                && let Some(mut host) = self.internal_codex.take()
            {
                let placement = internal_codex_chat_placement(
                    &outputs,
                    menu_output.as_deref().or(fallback.as_deref()),
                );
                let opened = host.open_project_by_id(&mut self.internal_ui, placement, &project_id);
                self.internal_codex = Some(host);
                match opened {
                    Ok(surface) => {
                        self.register_internal_application(surface);
                    }
                    Err(error) => {
                        tracing::warn!(%error, %project_id, "could not open internal Codex project");
                    }
                }
            }
        }
        let chat_output = self
            .internal_codex
            .as_ref()
            .and_then(crate::internal_codex::InternalCodexHost::project_menu)
            .and_then(|id| self.internal_ui.placement(id))
            .and_then(|placement| placement.output.as_deref())
            .map(str::to_owned)
            .or_else(|| self.resolve_interaction_output(InvocationSource::RecentInteraction));
        let chat_placement =
            internal_codex_chat_placement(&self.internal_outputs(), chat_output.as_deref());
        if let Some(mut codex) = self.internal_codex.take() {
            let changed = codex.poll_due(&mut self.internal_ui, now);
            let opened = codex
                .service_requests(&mut self.internal_ui, chat_placement)
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "could not service internal Codex request");
                    Vec::new()
                });
            let projection_changed = self
                .internal_shell
                .as_mut()
                .is_some_and(|shell| codex.sync_shell_projection(&self.internal_ui, shell));
            self.internal_codex = Some(codex);
            for surface in &opened {
                self.register_internal_application(*surface);
            }
            self.refresh_internal_application_metadata();
            if projection_changed {
                self.sync_internal_shell();
            }
            if !changed.is_empty() || !opened.is_empty() || projection_changed {
                self.schedule_internal_ui_frame();
            }
        }
        let closing = self
            .internal_file_surfaces
            .iter()
            .filter_map(|(coordinator, runtime)| {
                self.internal_ui
                    .application::<nickel_file::FileApp>(*runtime)
                    .is_some_and(nickel_file::FileApp::close_requested)
                    .then_some(*coordinator)
            })
            .collect::<Vec<_>>();
        for id in closing {
            let action = self
                .internal_shell
                .as_mut()
                .unwrap()
                .file_windows_mut()
                .handle(nickel_file::FileWindowRequest::Close(id));
            self.apply_internal_file_action(action);
        }
        if self.internal_ui.has_damage() {
            self.schedule_internal_ui_frame();
        }
    }

    pub(crate) fn refresh_internal_shell_system(&mut self) {
        let mut changed = self
            .internal_shell
            .as_mut()
            .map(|shell| shell.refresh_system())
            .unwrap_or_default();
        let theme = self
            .internal_shell
            .as_ref()
            .map(crate::internal_shell::InternalShellCoordinator::semantic_theme);
        if let (Some(theme), Some(mut codex)) = (theme, self.internal_codex.take()) {
            changed.extend(codex.set_theme(&mut self.internal_ui, theme));
            self.internal_codex = Some(codex);
        }
        changed.sort_unstable();
        changed.dedup();
        if !changed.is_empty() {
            self.sync_internal_shell_changes(Some(&changed));
        }
        self.schedule_internal_shell_deadline();
    }

    pub(crate) fn show_internal_codex_project_menu(
        &mut self,
        placement: crate::internal_codex::CodexSurfacePlacement,
    ) -> Result<nickel_ui::InternalSurfaceId, String> {
        let mut host = self
            .internal_codex
            .take()
            .ok_or_else(|| "Codex integration is disabled".to_owned())?;
        let was_visible = host
            .project_menu()
            .is_some_and(|id| self.internal_ui.is_visible(id));
        let result = host.ensure_project_menu(&mut self.internal_ui, placement);
        self.internal_codex = Some(host);
        if let Ok(id) = result {
            self.internal_ui.set_visible(id, true);
            if !was_visible {
                self.focus_internal_surface(id);
            }
        }
        result
    }

    /// Admit a compositor-hosted application into the same canonical window
    /// model used by Wayland and X11 clients. Internal surface ids remain opaque
    /// and are never reinterpreted as protocol window ids.
    fn register_internal_application(
        &mut self,
        surface: nickel_ui::InternalSurfaceId,
    ) -> Option<WindowId> {
        if let Some(id) = self.internal_surface_windows.get(&surface).copied() {
            self.activate_window(id);
            return Some(id);
        }
        let id = self.windows.insert(WindowAdmission::AuthenticatedShell)?;
        let is_file = self
            .internal_ui
            .application::<nickel_file::FileApp>(surface)
            .is_some();
        let title = self
            .internal_ui
            .title(surface)
            .unwrap_or(if is_file {
                "Nickel File"
            } else {
                "Nickel Codex"
            })
            .to_owned();
        self.windows.update_metadata(
            id,
            WindowMetadataSource::Internal,
            Some(title),
            Some(
                if is_file {
                    "nickel-file"
                } else {
                    "nickel-codex"
                }
                .to_owned(),
            ),
        );
        self.internal_surface_windows.insert(surface, id);
        self.internal_window_surfaces.insert(id, surface);
        self.workspaces.add_window(id);
        self.activate_window(id);
        Some(id)
    }

    fn unregister_internal_application(&mut self, surface: nickel_ui::InternalSurfaceId) {
        let Some(id) = self.internal_surface_windows.remove(&surface) else {
            return;
        };
        self.internal_window_surfaces.remove(&id);
        self.internal_minimized_windows.remove(&id);
        self.internal_maximized_restore.remove(&id);
        self.workspaces.remove_window(&id);
        self.remove_window_from_switcher(id);
        self.windows.remove(id);
        self.remote_window_identities.remove(&id);
        self.schedule_remote_resource_retirement();
        self.notify_protocol_snapshot();
    }

    fn sync_internal_window_decorations(&mut self) {
        let Some(theme) = self
            .internal_shell
            .as_ref()
            .map(crate::internal_shell::InternalShellCoordinator::semantic_theme)
        else {
            return;
        };
        let windows = self.windows.snapshot();
        let decorations = self
            .internal_surface_windows
            .iter()
            .filter_map(|(surface, window)| {
                let entry = windows.iter().find(|entry| entry.id == *window)?;
                Some((
                    *surface,
                    super::internal_ui::InternalWindowDecoration {
                        owner: window.0,
                        title: entry.title.clone(),
                        active: entry.active,
                        maximized: self.internal_maximized_restore.contains_key(window),
                        background: theme.surfaces.sidebar,
                        foreground: if entry.active {
                            theme.text.primary
                        } else {
                            theme.text.secondary
                        },
                    },
                ))
            })
            .collect::<Vec<_>>();
        let mut changed = false;
        for (surface, decoration) in decorations {
            changed |= self.internal_ui.set_window_decoration(surface, decoration);
        }
        if changed {
            self.schedule_internal_ui_frame();
        }
    }

    fn refresh_internal_application_metadata(&mut self) {
        let updates = self
            .internal_surface_windows
            .iter()
            .filter_map(|(surface, window)| {
                self.internal_ui
                    .title(*surface)
                    .map(|title| (*window, title.to_owned()))
            })
            .collect::<Vec<_>>();
        let mut changed = false;
        for (window, title) in updates {
            if self.windows.title(window) != Some(title.as_str()) {
                self.windows.update_metadata(
                    window,
                    WindowMetadataSource::Internal,
                    Some(title),
                    None,
                );
                changed = true;
            }
        }
        if changed {
            self.sync_internal_window_decorations();
            self.notify_protocol_snapshot();
        }
    }

    pub(crate) fn reconcile_internal_application_focus(&mut self) {
        self.reconcile_keyboard_internal_recipient();
        let Some(surface) = self.internal_ui.focused() else {
            return;
        };
        if self
            .internal_ui
            .placement(surface)
            .is_some_and(|placement| {
                matches!(
                    placement.role,
                    super::internal_ui::InternalSurfaceRole::Desktop
                        | super::internal_ui::InternalSurfaceRole::Overlay
                )
            })
        {
            // Desktop menus and shell overlays own input without a registered
            // application window.
            // Clear the old Wayland seat target through the shared focus boundary.
            self.focus_internal_surface(surface);
            return;
        }
        let Some(window) = self.internal_window_for_surface(surface) else {
            return;
        };
        self.internal_ui.raise(surface);
        self.seat.get_keyboard().unwrap().set_focus(
            self,
            Option::<crate::session::focus::KeyboardFocusTarget>::None,
            SERIAL_COUNTER.next_serial(),
        );
        self.windows.raise(window);
        self.workspaces.focused(&window);
        self.notify_protocol_snapshot();
    }

    pub(crate) fn internal_surface_for_window(
        &self,
        window: WindowId,
    ) -> Option<nickel_ui::InternalSurfaceId> {
        self.internal_window_surfaces.get(&window).copied()
    }

    pub(crate) fn internal_window_for_surface(
        &self,
        surface: nickel_ui::InternalSurfaceId,
    ) -> Option<WindowId> {
        self.internal_surface_windows.get(&surface).copied()
    }

    /// Whether the foremost ordinary window is compositor-hosted.
    ///
    /// Internal applications currently form one contiguous scene group. The
    /// canonical registry decides which side of the external client group it
    /// occupies, so activating either kind produces ordinary raise behavior.
    pub(crate) fn internal_applications_are_foremost(&self) -> bool {
        self.windows
            .snapshot()
            .last()
            .is_some_and(|window| self.internal_surface_for_window(window.id).is_some())
    }

    fn apply_internal_file_action(&mut self, action: nickel_file::FileWindowAction) {
        use nickel_file::FileWindowAction;
        match action {
            FileWindowAction::Opened(id) => {
                let Some(shell) = self.internal_shell.as_mut() else {
                    return;
                };
                let Some(surface) = shell.file_windows_mut().take_surface(id) else {
                    return;
                };
                let Some((output, x, y)) = self.internal_outputs().into_iter().next() else {
                    return;
                };
                let size = surface.logical_size();
                let offset = (self.internal_file_surfaces.len() as i32 * 32) % 192;
                let runtime = self.internal_ui.insert_boxed(
                    surface,
                    crate::session::InternalSurfacePlacement {
                        role: crate::session::InternalSurfaceRole::Application,
                        geometry: (
                            x + 48 + offset,
                            y + 64 + offset,
                            size.0.min(output.width),
                            size.1.min(output.height),
                        ),
                        output: Some(output.name),
                    },
                    1.0,
                );
                self.internal_file_surfaces.insert(id, runtime);
                // File windows are ordinary applications. Seat focus alone does
                // not admit them into the stacking/workspace/task-switcher model.
                self.register_internal_application(runtime);
            }
            FileWindowAction::Focused(id) => {
                if let Some(runtime) = self.internal_file_surfaces.get(&id).copied() {
                    self.register_internal_application(runtime);
                }
            }
            FileWindowAction::Closed(id) => {
                if let Some(runtime) = self.internal_file_surfaces.remove(&id) {
                    self.unregister_internal_application(runtime);
                    self.internal_ui.remove(runtime);
                }
            }
            FileWindowAction::NotFound(_) => {}
        }
    }

    pub(crate) fn toggle_internal_launcher(&mut self) -> bool {
        let Some(visible) = self
            .internal_shell
            .as_ref()
            .map(|shell| shell.launcher_visible())
        else {
            return false;
        };
        self.set_launcher_visible_from(!visible, InvocationSource::Keyboard);
        true
    }

    /// Hide the compositor-hosted launcher because an ordinary client is
    /// about to receive a pointer press. The press itself establishes the new
    /// focus, so do not briefly restore the window displaced when Launcher
    /// opened.
    pub(crate) fn dismiss_internal_launcher_for_client_press(&mut self) -> bool {
        let location = self.seat.get_pointer().unwrap().current_location();
        if self
            .internal_ui
            .surface_at((location.x, location.y), true)
            .is_some()
        {
            return false;
        }
        let Some(shell) = self.internal_shell.as_mut() else {
            return false;
        };
        if !shell.launcher_visible() || !shell.toggle_launcher() {
            return false;
        }
        self.launcher_restore_window = None;
        self.sync_internal_shell();
        self.wake_internal_shell();
        true
    }

    /// Ephemeral surfaces relinquish visibility with keyboard ownership. Reconcile
    /// both hosted applications and coordinator scenes at the same input boundary.
    fn dismiss_unfocused_internal_popovers(&mut self) {
        use crate::winit_shell::SurfaceRole;
        let focused = self.internal_ui.focused();
        let menu = self
            .internal_codex
            .as_ref()
            .and_then(|host| host.project_menu())
            .filter(|id| self.internal_ui.is_visible(*id) && focused != Some(*id));
        let control_blurred = self.internal_shell.as_ref().is_some_and(|shell| {
            shell.surfaces().iter().any(|surface| {
                surface.role == SurfaceRole::ControlCenter
                    && shell.visible(surface.id)
                    && self
                        .internal_shell_surfaces
                        .get(&surface.id)
                        .is_some_and(|id| self.internal_ui.is_visible(*id) && focused != Some(*id))
            })
        });
        if menu.is_none() && !control_blurred {
            return;
        }
        if let Some(menu) = menu {
            self.internal_ui.set_visible(menu, false);
        }
        if let Some(shell) = self.internal_shell.as_mut() {
            if menu.is_some() {
                shell.dismiss_ephemeral_on_focus_loss(SurfaceRole::CodexProjectMenu);
            }
            if control_blurred {
                shell.dismiss_ephemeral_on_focus_loss(SurfaceRole::ControlCenter);
            }
        }
        self.sync_internal_shell();
        self.wake_internal_shell();
    }

    pub(crate) fn flush_internal_shell_input(&mut self) {
        let stop = self
            .remote_indicator_surfaces
            .values()
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .any(|id| {
                self.internal_ui
                    .application_mut::<super::remote_indicator::RemoteIndicator>(id)
                    .is_some_and(|app| app.stop_requested)
            });
        if stop {
            self.emergency_stop_remote_control();
        }
        self.flush_native_clipboard_results();
        let events = self.internal_ui.drain_routed_events();
        if events.is_empty() || self.internal_shell.is_none() {
            self.dismiss_unfocused_internal_popovers();
            return;
        }
        let reverse = self
            .internal_shell
            .as_ref()
            .unwrap()
            .surfaces()
            .iter()
            .filter_map(|surface| {
                self.internal_shell_surfaces
                    .get(&surface.id)
                    .map(|runtime| (*runtime, (surface.id, surface.role, surface.output.clone())))
            })
            .collect::<HashMap<_, _>>();
        let desktop_motion_only = events.iter().all(|(runtime, batch, _)| {
            reverse
                .get(runtime)
                .is_some_and(|(_, role, _)| *role == crate::winit_shell::SurfaceRole::Desktop)
                && batch.window_focused.is_none()
                && !batch.events.is_empty()
                && batch.events.iter().all(|event| {
                    matches!(
                        event,
                        nickel_ui::HostEvent::Normalized {
                            input: nickel_input::InputEvent::Pointer(
                                nickel_input::PointerEvent::Motion { .. }
                            ),
                            ..
                        }
                    )
                })
        });
        let output_origins = self
            .internal_outputs()
            .into_iter()
            .map(|(output, x, y)| (output.name, (x, y)))
            .collect::<HashMap<_, _>>();
        let launcher_was_visible = self
            .internal_shell
            .as_ref()
            .is_some_and(crate::internal_shell::InternalShellCoordinator::launcher_visible);
        let file_clipboard_available = self.native_file_clipboard_available();
        let shell = self.internal_shell.as_mut().unwrap();
        shell.set_file_clipboard_available(file_clipboard_available);
        let mut changed = Vec::new();
        for (runtime_id, batch, modifiers) in events {
            let Some((shell_id, role, output)) = reverse.get(&runtime_id).cloned() else {
                continue;
            };
            if role == crate::winit_shell::SurfaceRole::Panel
                && let Some(output) = output
            {
                let origin = output_origins.get(&output).copied().unwrap_or_default();
                shell.set_panel_context(output, origin);
            }
            if let Some(modifiers) = modifiers {
                shell.set_desktop_input_modifiers(shell_id, &modifiers);
            }
            changed.extend(shell.step_slot_changes(shell_id, batch));
        }
        let launcher_is_visible = shell.launcher_visible();
        let _ = shell;
        if desktop_motion_only {
            // Do not turn mouse polling frequency into layout frequency or
            // rearm an immediate shell timer for every motion sample. Rendering
            // consumes the latest reducer state once, even after a large burst.
            if !changed.is_empty() {
                self.pending_desktop_scenes.extend(changed);
                self.request_output_redraw();
                #[cfg(feature = "backend-udev")]
                self.schedule_native_ui_frame();
            }
            return;
        }
        changed.extend(self.pending_desktop_scenes.drain());
        self.flush_native_clipboard_results();
        if !launcher_was_visible && launcher_is_visible {
            self.launcher_output_name =
                self.resolve_interaction_output(InvocationSource::RecentInteraction);
            if self.launcher_restore_window.is_none() {
                self.launcher_restore_window = self
                    .windows
                    .snapshot()
                    .into_iter()
                    .find(|window| window.active)
                    .map(|window| window.id);
            }
        }
        if !changed.is_empty() {
            self.sync_internal_shell_changes(Some(&changed));
        }
        if launcher_was_visible && !launcher_is_visible {
            self.restore_launcher_focus();
        } else if !launcher_was_visible
            && launcher_is_visible
            && let Some(runtime) = self.internal_shell.as_ref().and_then(|shell| {
                shell
                    .surfaces()
                    .iter()
                    .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Launcher)
                    .and_then(|surface| self.internal_shell_surfaces.get(&surface.id).copied())
            })
        {
            self.focus_internal_surface(runtime);
        }
        self.dismiss_unfocused_internal_popovers();
        self.wake_internal_shell();
    }

    pub(crate) fn sync_internal_shell(&mut self) {
        self.sync_internal_shell_changes(None);
    }

    pub(crate) fn flush_desktop_scenes_for_frame(&mut self) {
        if self.pending_desktop_scenes.is_empty() {
            return;
        }
        let changed = self.pending_desktop_scenes.drain().collect::<Vec<_>>();
        self.update_internal_shell_scenes(Some(&changed), false);
    }

    pub(crate) fn refresh_internal_preview_pixels(&mut self) {
        self.sync_internal_shell_changes(Some(&[]));
    }

    /// `None` is an explicit global dependency change (theme, locale, wallpaper,
    /// topology, scale or application replacement). Local service/input updates
    /// carry identities; visibility and placement are reconciled independently.
    fn sync_internal_shell_changes(&mut self, changed: Option<&[nickel_ui::InternalSurfaceId]>) {
        self.update_internal_shell_scenes(changed, true);
    }

    fn update_internal_shell_scenes(
        &mut self,
        changed: Option<&[nickel_ui::InternalSurfaceId]>,
        request_frame: bool,
    ) {
        let Some(mut shell) = self.internal_shell.take() else {
            return;
        };
        // Native hover cards have no external feed socket. Reuse the session's
        // existing admission and completed-pixel owners instead of self-RPC.
        let interest = shell
            .native_preview_windows()
            .into_iter()
            .map(|id| WindowId(id.0))
            .collect::<Vec<_>>();
        let interest_changed = interest != self.preview_overlay_interest;
        if interest_changed {
            self.set_overlay_preview_interest(interest);
        }
        let preview_pixels_changed = shell.sync_native_preview_pixels(
            self.preview_counters.presentation_generation,
            interest_changed,
            |id| {
                self.preview_frames
                    .get(&WindowId(id.0))
                    .map(|frame| (frame.width, frame.height, frame.rgba.as_slice()))
            },
        );
        let entries = shell.surfaces().to_vec();
        let current_shell_ids = entries.iter().map(|entry| entry.id).collect::<HashSet<_>>();
        let retired = self
            .internal_shell_surfaces
            .keys()
            .filter(|id| !current_shell_ids.contains(id))
            .copied()
            .collect::<Vec<_>>();
        for id in retired {
            if let Some(runtime_id) = self.internal_shell_surfaces.remove(&id) {
                self.invalidate_remote_shell_surface(runtime_id);
                self.internal_ui.remove(runtime_id);
            }
            self.pending_desktop_scenes.remove(&id);
        }
        let outputs = self.internal_outputs();
        let mut focus_on_show = None;
        for mut surface in entries {
            // The real ChatApplication host owns this role. LiveShell retains
            // only its visibility policy and must not paint a second shell
            // scene over the compositor-owned menu.
            if surface.role == crate::winit_shell::SurfaceRole::CodexProjectMenu {
                if let Some(runtime_id) = self.internal_shell_surfaces.remove(&surface.id) {
                    self.invalidate_remote_shell_surface(runtime_id);
                    self.internal_ui.remove(runtime_id);
                }
                continue;
            }
            let visible = shell.visible(surface.id);
            if !visible {
                if let Some(runtime_id) = self.internal_shell_surfaces.remove(&surface.id) {
                    self.invalidate_remote_shell_surface(runtime_id);
                    self.internal_ui.remove(runtime_id);
                    if let Some(role) = remote_shell_event_role(surface.role) {
                        self.remote_desktop_events.record(
                            nickel_remote_control::desktop_events::DesktopEventKind::ShellSurfaceVisibilityChanged {
                                surface_generation: runtime_id.snapshot_token(),
                                role,
                                visible: false,
                            },
                            self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
                        );
                    }
                }
                continue;
            }
            let interaction_output = match surface.role {
                crate::winit_shell::SurfaceRole::ControlCenter => shell
                    .popover_anchor(nickel_session_protocol::AnchorSide::Above)
                    .filter(|(role, _)| *role == nickel_session_protocol::ShellRole::ControlCenter)
                    .map(|(_, anchor)| anchor.output),
                crate::winit_shell::SurfaceRole::VolumeOsd => {
                    self.preferred_interaction_output_name()
                }
                crate::winit_shell::SurfaceRole::Screenshot => {
                    shell.screenshot_output().map(str::to_owned)
                }
                _ => None,
            };
            let mut placement = internal_shell_surface_placement(
                surface.role,
                interaction_output.as_deref().or(surface.output.as_deref()),
                surface.size,
                &outputs,
                self.launcher_output_name.as_deref(),
            );
            let mut resized = false;
            if matches!(
                surface.role,
                crate::winit_shell::SurfaceRole::Launcher
                    | crate::winit_shell::SurfaceRole::ControlCenter
            ) {
                surface.size = (placement.geometry.2, placement.geometry.3);
                resized = shell.set_surface_size(surface.id, surface.size);
            }
            if surface.role == crate::winit_shell::SurfaceRole::WindowContextMenu
                && let Some((x, y, width, height)) = shell.window_menu_geometry()
                && let Some((output, origin_x, origin_y)) = outputs
                    .iter()
                    .find(|(output, ox, oy)| {
                        x >= *ox
                            && y >= *oy
                            && i64::from(x) < i64::from(*ox) + i64::from(output.width)
                            && i64::from(y) < i64::from(*oy) + i64::from(output.height)
                    })
                    .or_else(|| outputs.first())
            {
                let width = width.min(output.width).max(1);
                let height = height
                    .min(
                        output
                            .height
                            .saturating_sub(crate::winit_shell::PANEL_HEIGHT),
                    )
                    .max(1);
                placement.output = Some(output.name.clone());
                placement.geometry = (
                    x.clamp(
                        *origin_x,
                        origin_x.saturating_add(output.width.saturating_sub(width) as i32),
                    ),
                    y.clamp(
                        *origin_y,
                        origin_y.saturating_add(
                            output
                                .height
                                .saturating_sub(crate::winit_shell::PANEL_HEIGHT)
                                .saturating_sub(height) as i32,
                        ),
                    ),
                    width,
                    height,
                );
                let trusted_controls = self
                    .remote_indicator_surfaces
                    .values()
                    .filter_map(|id| self.internal_ui.placement(*id))
                    .filter(|candidate| {
                        candidate.role == crate::session::InternalSurfaceRole::TrustedControl
                            && candidate.output.as_deref() == Some(output.name.as_str())
                    })
                    .map(|candidate| candidate.geometry)
                    .collect::<Vec<_>>();
                placement.geometry = avoid_trusted_control_collision(
                    placement.geometry,
                    (*origin_x, *origin_y, output.width, output.height),
                    &trusted_controls,
                );
                surface.size = (width, height);

                resized = shell.set_surface_size(surface.id, surface.size);
            }
            if surface.role == crate::winit_shell::SurfaceRole::Screenshot
                && let Some((output, _, _)) = outputs.iter().find(|(output, _, _)| {
                    Some(output.name.as_str()) == placement.output.as_deref()
                })
            {
                surface.size = (output.width, output.height);
                placement.geometry.2 = output.width;
                placement.geometry.3 = output.height;
                resized = shell.set_surface_size(surface.id, surface.size);
            }
            if surface.role == crate::winit_shell::SurfaceRole::OnScreenKeyboard {
                let Some(keyboard) = internal_keyboard_surface_placement(
                    self.on_screen_keyboard.output_name.as_deref(),
                    self.on_screen_keyboard.dock_top,
                    self.on_screen_keyboard.height,
                    &outputs,
                ) else {
                    if let Some(runtime_id) = self.internal_shell_surfaces.remove(&surface.id) {
                        self.invalidate_remote_shell_surface(runtime_id);
                        self.internal_ui.remove(runtime_id);
                    }
                    continue;
                };
                placement = keyboard;
                surface.size = (placement.geometry.2, placement.geometry.3);
                resized = shell.set_surface_size(surface.id, surface.size);
            }
            let output_scale = placement
                .output
                .as_deref()
                .and_then(|name| outputs.iter().find(|(output, _, _)| output.name == name))
                .map_or(1.0, |(output, _, _)| output.scale);
            if let Some(runtime_id) = self.internal_shell_surfaces.get(&surface.id).copied() {
                if shell.remote_access_protected(surface.id)
                    || self
                        .internal_ui
                        .placement(runtime_id)
                        .is_none_or(|current| current != &placement)
                {
                    self.invalidate_remote_shell_surface(runtime_id);
                }
                let geometry_changed =
                    self.internal_ui
                        .configure_surface(runtime_id, placement, output_scale);
                if (resized
                    || geometry_changed
                    || (preview_pixels_changed
                        && surface.role == crate::winit_shell::SurfaceRole::WindowPreview)
                    || changed.is_none_or(|ids| ids.contains(&surface.id)))
                    && let Some(scene) = shell.scene(surface.id)
                {
                    tracing::trace!(surface = ?surface.id, role = ?surface.role, reason = if changed.is_none() { "global" } else { "content" }, "rebuild internal shell scene");
                    self.pending_desktop_scenes.remove(&surface.id);
                    self.internal_ui.update_scene(runtime_id, scene);
                }
                continue;
            }
            let Some(scene) = shell.scene(surface.id) else {
                continue;
            };
            let runtime_id = self
                .internal_ui
                .insert_scene(scene, placement, output_scale);
            self.internal_shell_surfaces.insert(surface.id, runtime_id);
            if let Some(role) = remote_shell_event_role(surface.role) {
                self.remote_desktop_events.record(
                    nickel_remote_control::desktop_events::DesktopEventKind::ShellSurfaceVisibilityChanged {
                        surface_generation: runtime_id.snapshot_token(),
                        role,
                        visible: true,
                    },
                    self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
                );
            }
            if matches!(
                surface.role,
                crate::winit_shell::SurfaceRole::ControlCenter
                    | crate::winit_shell::SurfaceRole::Screenshot
                    | crate::winit_shell::SurfaceRole::WindowContextMenu
            ) {
                focus_on_show = Some(runtime_id);
            }
        }
        self.internal_shell = Some(shell);
        if let Some(surface) = focus_on_show {
            self.focus_internal_surface(surface);
        }
        if request_frame {
            self.schedule_internal_ui_frame();
        } else {
            // The caller is already rendering this frame; do not queue another
            // frame merely because it consumed deferred scene work.
            self.schedule_internal_shell_deadline();
        }
    }

    pub fn insert_internal_surface<A: nickel_ui::Application + 'static>(
        &mut self,
        application: A,
        placement: crate::session::InternalSurfacePlacement,
        scale: f32,
    ) -> nickel_ui::InternalSurfaceId {
        let id = self.internal_ui.insert(application, placement, scale);
        self.schedule_internal_ui_frame();
        id
    }

    pub fn remove_internal_surface(&mut self, id: nickel_ui::InternalSurfaceId) -> bool {
        self.invalidate_remote_shell_surface(id);
        let removed = self.internal_ui.remove(id);
        if removed {
            self.schedule_internal_ui_frame();
        }
        removed
    }

    pub fn step_internal_surface(
        &mut self,
        id: nickel_ui::InternalSurfaceId,
        batch: nickel_ui::HostBatch,
    ) -> bool {
        let changed = self.internal_ui.step(id, batch);
        if changed {
            self.schedule_internal_ui_frame();
        }
        changed
    }

    pub(super) fn schedule_internal_ui_frame(&mut self) {
        self.internal_shell_timer.counters.redraw_requests = self
            .internal_shell_timer
            .counters
            .redraw_requests
            .saturating_add(1);
        self.request_output_redraw();
        #[cfg(feature = "backend-udev")]
        if self.native.is_some() {
            // Rendering must not recurse from input/layout dispatch. The
            // backend keeps one paced request per device, with no retry tail.
            self.schedule_native_ui_frame();
        }
        self.schedule_internal_shell_deadline();
    }

    pub(crate) fn configured_output_scale(&self, output: &Output) -> OutputScale {
        self.output_scale_preferences
            .get(&stable_output_identity(output))
            .map(|scale| OutputScale::Fractional(scale.factor()))
            .unwrap_or(OutputScale::Integer(1))
    }
    /// Map a compositor-managed window only while its Wayland client has a
    /// buffer attached. X11 windows and non-XDG surfaces are unaffected.
    pub(crate) fn map_buffered_window(
        &mut self,
        window: Window,
        location: impl Into<Point<i32, Logical>>,
        activate: bool,
    ) -> bool {
        if let Some(surface) = window.wl_surface()
            && self.xdg_toplevel_windows.contains_key(&surface.id())
            && !self.mapped_xdg_toplevels.contains(&surface.id())
        {
            return false;
        }
        if self.panel_hidden_by_keyboard(&window) {
            self.space.unmap_elem(&window);
            return false;
        }
        self.space.map_element(window.clone(), location, activate);
        self.fit_window_above_keyboard(&window);
        true
    }
}

fn remote_shell_event_role(
    role: crate::winit_shell::SurfaceRole,
) -> Option<nickel_remote_control::desktop_events::ShellEventRole> {
    use crate::winit_shell::SurfaceRole;
    use nickel_remote_control::desktop_events::ShellEventRole;
    match role {
        SurfaceRole::Desktop => Some(ShellEventRole::Desktop),
        SurfaceRole::Panel => Some(ShellEventRole::Panel),
        SurfaceRole::Launcher => Some(ShellEventRole::Launcher),
        SurfaceRole::ControlCenter => Some(ShellEventRole::ControlCenter),
        SurfaceRole::Notification => Some(ShellEventRole::Notification),
        SurfaceRole::VolumeOsd => Some(ShellEventRole::VolumeOsd),
        SurfaceRole::WindowPreview => Some(ShellEventRole::WindowPreview),
        SurfaceRole::WindowContextMenu => Some(ShellEventRole::WindowContextMenu),
        SurfaceRole::Screenshot => Some(ShellEventRole::Screenshot),
        SurfaceRole::OnScreenKeyboard => Some(ShellEventRole::OnScreenKeyboard),
        SurfaceRole::CodexProjectMenu | SurfaceRole::CodexChat | SurfaceRole::Lock => None,
        #[cfg(target_os = "windows")]
        SurfaceRole::TrustedControl => None,
    }
}
struct DisplacedWindow {
    id: WindowId,
    relative_location: Point<i32, Logical>,
    rescue_location: Point<i32, Logical>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RegisteredShellRole {
    role: ShellRole,
    output: Option<String>,
    surface: ObjectId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LifecycleCollectionCounts {
    pub pointer_hints: usize,
    pub pointer_locks: usize,
    pub pointer_origins: usize,
    pub displaced_outputs: usize,
    pub displaced_windows: usize,
    pub shell_roles: usize,
}

fn retire_pointer_surface<K: Eq + Hash>(
    hints: &mut HashMap<K, Point<f64, Logical>>,
    locks: &mut HashSet<K>,
    origins: &mut HashMap<K, Point<f64, Logical>>,
    surface: &K,
) -> Option<Point<f64, Logical>> {
    let hint = hints.remove(surface);
    let was_locked = locks.remove(surface);
    let origin = origins.remove(surface);
    if hints.is_empty() {
        hints.shrink_to_fit();
    }
    if locks.is_empty() {
        locks.shrink_to_fit();
    }
    if origins.is_empty() {
        origins.shrink_to_fit();
    }
    was_locked.then(|| origin.zip(hint).map(|(origin, hint)| origin + hint))?
}

fn retire_displaced_window(
    outputs: &mut HashMap<String, Vec<DisplacedWindow>>,
    window_id: WindowId,
) {
    for displaced in outputs.values_mut() {
        displaced.retain(|window| window.id != window_id);
    }
    outputs.retain(|_, displaced| !displaced.is_empty());
    if outputs.is_empty() {
        outputs.shrink_to_fit();
    }
}

fn retire_shell_surface(registrations: &mut Vec<RegisteredShellRole>, surface_id: &ObjectId) {
    registrations.retain(|registration| registration.surface != *surface_id);
    if registrations.is_empty() {
        registrations.shrink_to_fit();
    }
}

fn shell_registration_role_changed(
    registrations: &[RegisteredShellRole],
    surface_id: &ObjectId,
    next_role: Option<ShellRole>,
) -> bool {
    registrations
        .iter()
        .find(|registration| registration.surface == *surface_id)
        .is_some_and(|registration| Some(registration.role) != next_role)
}

fn shell_registration_is_active(
    registration: &RegisteredShellRole,
    output_names: &HashSet<String>,
    expected_panel_outputs: &HashSet<String>,
) -> bool {
    match registration.role {
        ShellRole::Desktop | ShellRole::Lock => registration
            .output
            .as_ref()
            .is_some_and(|output| output_names.contains(output)),
        ShellRole::Panel => registration
            .output
            .as_ref()
            .is_some_and(|output| expected_panel_outputs.contains(output)),
        _ => true,
    }
}

struct RemoteOutputIdentification {
    permit: nickel_remote_control::DesktopPermit,
    output: nickel_remote_control::leases::ResourceId,
}

pub struct SurfaceBufferCommit {
    pub surface: WlSurface,
    pub render_visible: bool,
}

impl NickelSession {
    fn expire_output_identification(&mut self, scheduled_generation: u64) -> bool {
        if !identification_expiry_is_current(self.identify_outputs_generation, scheduled_generation)
        {
            return false;
        }
        if let Some(token) = self.identify_outputs_timer.take() {
            self.event_loop_handle.remove(token);
        }
        self.identify_outputs_until = None;
        self.remote_output_identification = None;
        #[cfg(feature = "backend-udev")]
        if let Some(native) = self.native.as_mut() {
            native.retire_identify_badges();
        }
        true
    }

    fn begin_output_identification(&mut self) {
        if let Err(error) = self.start_output_identification(None) {
            tracing::warn!(%error, "output identification unavailable");
            return;
        }
        self.request_output_redraw();
        #[cfg(feature = "backend-udev")]
        if self.native.is_some() {
            self.render_all_outputs();
        }
    }

    fn start_output_identification(
        &mut self,
        owner: Option<RemoteOutputIdentification>,
    ) -> Result<u64, String> {
        const DURATION: Duration = Duration::from_secs(3);
        let generation = self
            .identify_outputs_generation
            .checked_add(1)
            .ok_or("output identification generation exhausted")?;
        if let Some(token) = self.identify_outputs_timer.take() {
            self.event_loop_handle.remove(token);
        }
        self.identify_outputs_generation = generation;
        self.identify_outputs_until = Some(Instant::now() + DURATION);
        let remote = owner.is_some();
        self.remote_output_identification = owner;
        let timer = Timer::from_duration(if remote {
            Duration::from_millis(100)
        } else {
            DURATION
        });
        let registration = self
            .event_loop_handle
            .insert_source(timer, move |_, _, data| {
                if data.identify_outputs_generation != generation {
                    return TimeoutAction::Drop;
                }
                let token = data.identify_outputs_timer.take();
                data.revalidate_remote_output_identification();
                if data
                    .identify_outputs_until
                    .is_none_or(|until| Instant::now() >= until)
                {
                    data.expire_output_identification(generation);
                    data.request_output_redraw();
                    #[cfg(feature = "backend-udev")]
                    if data.native.is_some() {
                        data.render_all_outputs();
                    }
                    TimeoutAction::Drop
                } else {
                    data.identify_outputs_timer = token;
                    TimeoutAction::ToDuration(Duration::from_millis(100))
                }
            });
        match registration {
            Ok(token) => self.identify_outputs_timer = Some(token),
            Err(_) => {
                self.expire_output_identification(generation);
                return Err("output identification timer is unavailable".into());
            }
        }
        Ok(generation)
    }

    pub(crate) fn revalidate_remote_output_identification(&mut self) {
        let invalid = self
            .remote_output_identification
            .as_ref()
            .is_some_and(|owner| {
                self.locked
                    || self.shell_recovery_visible()
                    || self
                        .remote_output_identity(owner.output.id.clone())
                        .as_ref()
                        != Some(&owner.output)
                    || owner
                        .permit
                        .continued_observation()
                        .and_then(|permit| permit.with_debug(false, || Ok(())))
                        .is_err()
            });
        if invalid {
            self.expire_output_identification(self.identify_outputs_generation);
            self.request_output_redraw();
        }
    }

    #[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
    pub(crate) fn output_identification_index(&mut self, output: &Output) -> Option<(u64, usize)> {
        self.revalidate_remote_output_identification();
        self.identify_outputs_until
            .filter(|until| Instant::now() < *until)?;
        if self.locked || self.shell_recovery_visible() {
            return None;
        }
        if self
            .remote_output_identification
            .as_ref()
            .is_some_and(|owner| owner.output.id != output.name())
        {
            return None;
        }
        let mut outputs = self
            .space
            .outputs()
            .take(nickel_remote_control::diagnostics::MAX_DIAGNOSTIC_OUTPUTS)
            .cloned()
            .collect::<Vec<_>>();
        outputs.sort_by_key(|output| {
            self.space
                .output_geometry(output)
                .map(|geometry| (geometry.loc.x, geometry.loc.y))
                .unwrap_or_default()
        });
        outputs
            .iter()
            .position(|candidate| candidate == output)
            .map(|index| (self.identify_outputs_generation, index))
    }

    pub(crate) fn note_input_activity(&mut self) {
        if self
            .idle_controller
            .note_activity(self.start_time.elapsed())
            == Some(IdleEffect::Undim)
        {
            self.dimmed = false;
            self.request_output_redraw();
            #[cfg(feature = "backend-udev")]
            if self.native.is_some() {
                self.render_all_outputs();
            }
        }
    }

    pub(crate) fn poll_idle_policy(&mut self) {
        self.reap_output_global_retirements(Instant::now());
        self.prune_dead_idle_inhibitors();
        let effects = self.idle_controller.poll(
            self.start_time.elapsed(),
            !self.idle_inhibitors.is_empty(),
            self.locked,
        );
        for effect in effects {
            match effect {
                IdleEffect::Dim => {
                    self.dimmed = true;
                    self.request_output_redraw();
                    #[cfg(feature = "backend-udev")]
                    if self.native.is_some() {
                        self.render_all_outputs();
                    }
                }
                IdleEffect::Undim => {
                    self.dimmed = false;
                    self.request_output_redraw();
                    #[cfg(feature = "backend-udev")]
                    if self.native.is_some() {
                        self.render_all_outputs();
                    }
                }
                IdleEffect::Lock => {
                    self.dimmed = false;
                    self.lock_session();
                }
                IdleEffect::Suspend => {
                    crate::session::session_services::request(
                        crate::session::session_services::SystemAction::Suspend,
                    );
                }
            }
        }
    }

    pub(crate) fn output_global_admission_available(&mut self) -> bool {
        self.reap_output_global_retirements(Instant::now());
        capacity_available(
            self.pending_output_global_retirements.len(),
            self.space.outputs().count(),
        )
    }

    pub(crate) fn defer_output_global_retirement(&mut self, identity: String, global: GlobalId) {
        self.defer_output_global_retirement_at(identity, global, Instant::now());
    }

    fn defer_output_global_retirement_at(
        &mut self,
        identity: String,
        global: GlobalId,
        now: Instant,
    ) {
        self.pending_output_global_retirements
            .defer(now, identity, global)
            .expect("output-global admission keeps the retirement queue bounded");
    }

    pub(crate) fn output_global_identity_available(&self, identity: &str) -> bool {
        !self
            .pending_output_global_retirements
            .has_enabled_identity(identity)
    }

    fn reap_output_global_retirements(&mut self, now: Instant) {
        let mut disabled = false;
        let mut identities_to_publish = HashSet::new();
        for action in self.pending_output_global_retirements.advance(now) {
            match action {
                RetirementAction::Disable { identity, value } => {
                    self.display_handle.disable_global::<NickelSession>(value);
                    identities_to_publish.insert(identity);
                    disabled = true;
                }
                RetirementAction::Remove { value, .. } => {
                    self.display_handle.remove_global::<NickelSession>(value);
                }
            }
        }
        for identity in identities_to_publish {
            if self.output_global_identity_available(&identity) {
                self.publish_live_output_global(&identity);
            }
        }
        if disabled {
            let mut display = self.display_handle.clone();
            let _ = display.flush_clients();
        }
    }

    fn publish_live_output_global(&mut self, identity: &str) {
        let virtual_output = self
            .virtual_test_outputs
            .get(identity)
            .filter(|(_, global)| global.is_none())
            .map(|(output, _)| output.clone());
        #[cfg(feature = "backend-udev")]
        let native_output = self.native_output_without_global(identity);
        #[cfg(not(feature = "backend-udev"))]
        let native_output: Option<Output> = None;
        let Some(output) = virtual_output.or(native_output) else {
            return;
        };
        let global = output.create_global::<NickelSession>(&self.display_handle);
        if let Some((_, slot)) = self.virtual_test_outputs.get_mut(identity) {
            *slot = Some(global);
            return;
        }
        #[cfg(feature = "backend-udev")]
        let _ = self.set_native_output_global(identity, global);
    }

    pub(crate) fn prune_dead_idle_inhibitors(&mut self) {
        // Smithay calls `uninhibit` for an explicit protocol destroy, but the
        // protocol also destroys every inhibitor when its client disconnects.
        // Those resources do not produce an `uninhibit` callback, so retain
        // the surface proxy and discard entries whose Wayland resource died.
        retain_live_idle_inhibitors(&mut self.idle_inhibitors, Resource::is_alive);
    }

    pub(crate) fn is_authenticated_shell_pid(&self, pid: u32) -> bool {
        self.compatibility_control.as_ref().is_some_and(|control| {
            control.expected_shell_pid == pid && control.authenticated_shell_pids.contains(&pid)
        })
    }

    pub fn new(
        event_loop: &mut EventLoop<'static, NickelSession>,
        display: Display<Self>,
        test_control_enabled: bool,
    ) -> Self {
        let start_time = std::time::Instant::now();
        let shell_settings = ShellSettings::load_default();
        let idle_controller = IdleController::new(
            IdlePolicy::from_seconds(
                shell_settings.idle_dim_seconds,
                shell_settings.idle_lock_seconds,
                shell_settings.idle_suspend_seconds,
            ),
            std::time::Duration::ZERO,
        );

        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        super::remote_accessibility::install(&dh);
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_dialog_state = XdgDialogState::new::<Self>(&dh);
        let activation_state = XdgActivationState::new::<Self>(&dh);
        let decoration_state = XdgDecorationState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let _text_input_manager_state =
            smithay::wayland::text_input::TextInputManagerState::new::<Self>(&dh);
        let mut seat_state = SeatState::new();
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let relative_pointer_state = RelativePointerManagerState::new::<Self>(&dh);
        let pointer_constraints_state = PointerConstraintsState::new::<Self>(&dh);
        let idle_inhibit_state = IdleInhibitManagerState::new::<Self>(&dh);
        // The protocol itself permits only one active input method per seat.
        // Visibility is session-local: every connected client already passed
        // the compositor socket's same-user boundary.
        let input_method_state = InputMethodManagerState::new::<Self, _>(&dh, |_| true);
        let xwayland_shell_state = XWaylandShellState::new::<Self>(&dh);
        let image_capture_source_state = ImageCaptureSourceState::new();
        let output_capture_source_state = OutputCaptureSourceState::new_with_filter::<Self, _>(
            &dh,
            crate::session::handlers::is_portal_capture_client,
        );
        let image_copy_capture_state = ImageCopyCaptureState::new_with_filter::<Self, _>(
            &dh,
            crate::session::handlers::is_portal_capture_client,
        );
        let popups = PopupManager::default();

        // A seat is a group of keyboards, pointer and touch devices.
        // A seat typically has a pointer and maintains a keyboard focus and a pointer focus.
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "winit");

        // Notify clients that we have a keyboard, for the sake of the example we assume that keyboard is always present.
        // You may want to track keyboard hot-plug in real compositor.
        // Match ordinary desktop repeat behavior. A 200 ms delay caused normal
        // key presses to enter repeat before users could release the key.
        seat.add_keyboard(Default::default(), 600, 25).unwrap();

        // Notify clients that we have a pointer (mouse)
        // Here we assume that there is always pointer plugged in
        seat.add_pointer();
        seat.add_touch();

        // A space represents a two-dimensional plane. Windows and Outputs can be mapped onto it.
        //
        // Windows get a position and stacking order through mapping.
        // Outputs become views of a part of the Space and can be rendered via Space::render_output.
        let space = Space::default();

        // The shell itself uses `InProcessSessionHost`; this authenticated
        // endpoint exists only for trusted out-of-process Nickel utilities
        // such as `nickel-settings`. Ordinary applications have both values
        // stripped from their launch environment.
        let compatibility_control = {
            let protocol_token = production_control_token();
            // SAFETY: session initialization is single-threaded and precedes clients.
            unsafe { std::env::set_var("NICKEL_SESSION_TOKEN", &protocol_token) };
            let control_socket_path = Self::init_control_socket(event_loop);
            crate::model::install_trusted_session_capability(
                control_socket_path.as_os_str().to_owned(),
                protocol_token.clone().into(),
            );
            let control_socket_name = control_socket_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("nickel");
            if test_control_enabled {
                let shell_test_path = control_socket_path
                    .with_file_name(format!("nickel-shell-test-{control_socket_name}"));
                // SAFETY: initialization still precedes all client launches.
                unsafe { std::env::set_var("NICKEL_SHELL_TEST_CONTROL", shell_test_path) };
            } else {
                unsafe {
                    std::env::remove_var("NICKEL_SESSION_CONTROL");
                    std::env::remove_var("NICKEL_SESSION_TOKEN");
                    std::env::remove_var("NICKEL_SHELL_TEST_CONTROL");
                }
            }
            Some(CompatibilityControlState {
                protocol_token,
                authenticated_shell_pids: HashSet::new(),
                expected_shell_pid: 0,
                socket_path: control_socket_path,
            })
        };
        let secure_storage_state = Arc::new(AtomicU8::new(
            crate::session::login_services::SecureStorageState::Starting as u8,
        ));
        let secure_storage_retry = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (deferred_focus_restore, deferred_focus_restore_rx) = channel::channel();
        event_loop
            .handle()
            .insert_source(deferred_focus_restore_rx, |event, _, data| {
                if let channel::Event::Msg(window) = event {
                    data.activate_window(window);
                    data.request_output_redraw();
                }
            })
            .expect("failed to register deferred focus restoration");
        let remote_cleanup_wake = Self::register_remote_connection_cleanup_wake(
            &event_loop.handle(),
            smithay::reexports::rustix::event::eventfd(
                0,
                smithay::reexports::rustix::event::EventfdFlags::NONBLOCK
                    | smithay::reexports::rustix::event::EventfdFlags::CLOEXEC,
            )
            .map_err(Into::into),
        );
        let (remote_desktop_tx, remote_desktop_rx) = channel::sync_channel(32);
        let remote_native_key_worker =
            native_key_worker::NativeKeyWorker::start(remote_desktop_tx.clone()).ok();
        let (identity_tx, identity_rx) = channel::sync_channel(32);
        let remote_identity_worker =
            super::remote_identity::IdentityWorker::start(identity_tx).ok();
        event_loop
            .handle()
            .insert_source(identity_rx, |event, _, data| {
                if let channel::Event::Msg(super::remote_identity::IdentityResult {
                    window,
                    identity,
                }) = event
                    && data.windows.title(window).is_some()
                {
                    data.remote_window_identities.insert(window, identity);
                    data.resume_remote_launch_maps();
                    data.observe_pending_launch_window(window);
                    if !data.remote_window_is_protected(window) {
                        if data.remote_event_windows.insert(window) {
                            data.remote_desktop_events.record(
                                nickel_remote_control::desktop_events::DesktopEventKind::WindowIdentityVerified { window_id: window.0 },
                                data.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
                            );
                        }
                    } else {
                        data.remote_event_windows.remove(&window);
                    }
                }
            })
            .expect("failed to register resource identity worker");
        event_loop
            .handle()
            .insert_source(remote_desktop_rx, |event, _, data| {
                if let channel::Event::Msg(request) = event {
                    data.handle_remote_desktop_request(request);
                }
            })
            .expect("failed to register remote desktop authority");
        event_loop
            .handle()
            .insert_source(
                smithay::reexports::calloop::timer::Timer::from_duration(Duration::from_secs(1)),
                |_, _, data| {
                    data.service_remote_connection_cleanup();
                    data.expire_remote_shell_origins();
                    data.reap_remote_launched_children();
                    data.resume_remote_launch_maps();
                    data.refresh_remote_output_identities();
                    data.refresh_remote_window_identities();
                    let control = data.remote_control.control();
                    let mut control = control.lock().unwrap();
                    let had_leases = control.leases().iter().next().is_some();
                    control.leases_mut().expire(Instant::now());
                    control.reconcile_pending_lease_requests(Instant::now());
                    // Native held input is revalidated below after releasing the authority lock.
                    control.leases_mut().take_cancellations();
                    drop(control);
                    data.revalidate_remote_pointer();
                    data.revalidate_remote_keyboard();
                    if had_leases {
                        data.sync_remote_control_indicators();
                    }
                    smithay::reexports::calloop::timer::TimeoutAction::ToDuration(
                        Duration::from_secs(1),
                    )
                },
            )
            .expect("failed to register remote lease expiration");
        let remote_settings_staging = Arc::new(remote_settings::SettingsStaging::default());
        let remote_diagnostic_staging = Arc::new(remote_worker::WorkerStaging::default());
        let remote_desktop_authority: Arc<dyn nickel_remote_control::DesktopAuthority> =
            Arc::new(RemoteDesktopBridge {
                cleanup_wake: remote_cleanup_wake.clone(),
                sender: remote_desktop_tx,
                settings_staging: remote_settings_staging.clone(),
                diagnostic_staging: remote_diagnostic_staging.clone(),
            });

        let socket_name = Self::init_wayland_listener(display, event_loop);

        // Get the loop signal, used to stop the event loop
        let loop_signal = event_loop.get_signal();
        let mut workspaces = Workspaces::default();
        let configured_desktops = ShellSettings::load_default().desktop_count;
        let _ = workspaces.set_count(usize::from(configured_desktops));

        let mut session = Self {
            start_time,
            display_handle: dh,
            event_loop_handle: event_loop.handle(),
            native_clipboard: Default::default(),
            #[cfg(feature = "backend-udev")]
            native: None,

            space,
            internal_ui: Default::default(),
            internal_shell: None,
            internal_codex: None,
            internal_shell_surfaces: HashMap::new(),
            remote_indicator_surfaces: HashMap::new(),
            pending_desktop_scenes: HashSet::new(),
            internal_file_surfaces: HashMap::new(),
            internal_shell_timer: InternalShellTimer::default(),
            internal_system_status_source: None,
            loop_signal,
            socket_name,

            compositor_state,
            fractional_scale_manager_state,
            viewporter_state,
            xdg_shell_state,
            xdg_dialog_state,
            activation_state,
            decoration_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            primary_selection_state,
            dnd_icon: None,
            relative_pointer_state,
            pointer_constraints_state,
            pointer_lock_hints: HashMap::new(),
            active_pointer_locks: HashSet::new(),
            active_pointer_constraint_origins: HashMap::new(),
            idle_inhibit_state,
            input_method_state,
            xwayland_shell_state,
            image_capture_source_state,
            output_capture_source_state,
            image_copy_capture_state,
            image_copy_sessions: Vec::new(),
            pending_image_copy_frames: Vec::new(),
            xwm: None,
            xwayland_restart_pending: false,
            xwayland_display: None,
            xwayland_registration: None,
            popups,
            seat,
            on_screen_keyboard: Default::default(),
            windows: WindowRegistry::default(),
            surface_windows: HashMap::new(),
            internal_surface_windows: HashMap::new(),
            internal_window_surfaces: HashMap::new(),
            internal_minimized_windows: HashSet::new(),
            internal_maximized_restore: HashMap::new(),
            surface_effective_outputs: HashMap::new(),
            xdg_toplevel_windows: HashMap::new(),
            mapped_xdg_toplevels: HashSet::new(),
            restored_xdg_toplevels: HashSet::new(),
            xdg_toplevel_locations: HashMap::new(),
            shell_owned_windows: HashSet::new(),
            x11_windows: HashMap::new(),
            launcher_window: None,
            launcher_visibility: LauncherVisibility::default(),
            launcher_output_name: None,
            last_interaction_output_name: None,
            launcher_focus: FocusTransactions::default(),
            launcher_restore_window: None,
            launcher_subscribers: Vec::new(),
            pending_launch_observations: Vec::new(),
            compatibility_control,
            shell_surface_identities: HashMap::new(),
            registered_shell_role_slots: Vec::new(),
            last_logged_shell_readiness: None,
            test_control_enabled,
            #[cfg(target_os = "linux")]
            test_controller: None,
            launcher_show_requested_at: None,
            desktop_windows: Vec::new(),
            panel_windows: Vec::new(),
            lock_windows: Vec::new(),
            locked: false,
            held_consumer_controls: HashMap::new(),
            consumer_repeat_epoch: 0,
            lock_restore_window: None,
            shell_focus_restore_window: None,
            pending_shell_focus_role: None,
            utility_windows: Vec::new(),
            hidden_shell_roles: HashSet::new(),
            hidden_shell_role_locations: HashMap::new(),
            screenshot_output_name: None,
            context_menu_window: None,
            preview_window: None,
            server_decorated: HashSet::new(),
            primary_output_name: None,
            output_topology_generation: 0,
            last_protocol_outputs: Vec::new(),
            output_scale_preferences: nickel_core::dpi::PersistedOutputScales::load_default()
                .unwrap_or_default(),
            virtual_test_outputs: HashMap::new(),
            pending_output_global_retirements: DeferredRetirements::default(),
            preview_frames: HashMap::new(),
            preview_spares: HashMap::new(),
            preview_switcher_interest: Vec::new(),
            preview_overlay_interest: Vec::new(),
            preview_admitted: HashSet::new(),
            preview_dirty: HashSet::new(),
            preview_content_generation: HashMap::new(),
            preview_attempted: HashMap::new(),
            preview_render_wave: 0,
            preview_retry_pending: HashSet::new(),
            preview_failures: HashMap::new(),
            preview_retry_scheduled: None,
            preview_retry_epoch: 1,
            preview_counters: PreviewCacheCounters::default(),
            hotkeys: CompositorShortcutAdapter::default(),
            remote_control: Default::default(),
            local_cues: Default::default(),
            remote_controller_observer: nickel_ui::ControllerInput::new(),
            remote_cleanup_wake,
            remote_settings_staging,
            remote_diagnostic_staging,
            remote_observation_generation: 0,
            remote_application_inventory_generation: 0,
            remote_application_inventory_refresh: None,
            remote_platform_refresh_generation: 0,
            remote_platform_refreshes: Vec::new(),
            remote_external_accessibility: None,
            remote_appearance: Default::default(),
            remote_application_scale: Default::default(),
            remote_launcher_favorites: Default::default(),
            remote_wallpaper: Default::default(),
            remote_file_icons: Default::default(),
            remote_idle_preferences: Default::default(),
            remote_codex_runtime_generation: 0,
            remote_terminal_presentation: Default::default(),
            remote_desktop_events: Default::default(),
            remote_window_event_states: HashMap::new(),
            remote_output_event_states: HashMap::new(),
            remote_focus_event_state: None,
            remote_frame_trace: None,
            remote_event_windows: HashSet::new(),
            remote_launched_children: Vec::new(),
            remote_launch_placements: Vec::new(),
            remote_launch_maps: HashMap::new(),
            remote_shell_origins: HashMap::new(),
            remote_shell_origin_generation: 0,
            remote_held_keyboard: None,
            remote_keyboard_timer_armed: false,
            remote_native_key_worker,
            remote_held_pointer: None,
            remote_input_dispatching: false,
            remote_native_press: None,
            remote_gtk_menu: None,
            remote_gtk_menu_timer_armed: false,
            remote_gtk_epoch: 0,
            remote_pointer_timer_armed: false,
            #[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
            remote_capture_work: None,
            remote_resource_recheck_pending: false,
            remote_output_generations: HashMap::new(),
            remote_next_output_generation: 0,
            remote_identity_worker,
            remote_window_identities: HashMap::new(),
            remote_emergency_chord: Default::default(),
            remote_stop_confirmation_until: None,
            remote_desktop_authority,
            task_switcher: TaskSwitcher::default(),
            workspaces,
            workspace_hidden_windows: HashMap::new(),
            displaced_output_windows: HashMap::new(),
            preview_highlight: None,
            minimized_windows: HashMap::new(),
            shortcut_desktop_windows: Vec::new(),
            shortcut_desktop_focus: None,
            shortcut_snap_restore: HashMap::new(),
            maximized_restore: HashMap::new(),
            x11_maximized_restore: HashMap::new(),
            fullscreen_restore: HashMap::new(),
            x11_fullscreen_restore: HashMap::new(),
            last_titlebar_click: None,
            suppress_left_button_release: false,
            idle_inhibitors: HashMap::new(),
            active_touch_slots: HashSet::new(),
            idle_controller,
            dimmed: false,
            frame_cursor: crate::session::window_frame::FrameCursor::Arrow,
            buffer_commit_tx: None,
            identify_outputs_until: None,
            identify_outputs_generation: 0,
            identify_outputs_timer: None,
            remote_output_identification: None,
            output_capture_path: None,
            output_capture_name: None,
            output_capture_reply_path: None,
            output_capture_request_id: None,
            internal_capture: Arc::new(std::sync::Mutex::new(InternalCaptureState::Idle)),
            internal_projection_outputs: Arc::new(std::sync::RwLock::new(Vec::new())),
            internal_keyboard_snapshot: Arc::new(std::sync::RwLock::new(None)),
            shell_failure_count: 0,
            recovery_ui: crate::session::recovery_ui::RecoveryUi::new(),
            secure_storage_state,
            secure_storage_retry,
            deferred_focus_restore,
            #[cfg(feature = "backend-winit")]
            winit_redraw_window: None,
        };
        if !cfg!(test) {
            let settings =
                nickel_remote_control::RemoteAiControlSettings::load_default().unwrap_or_default();
            let desktop = session.remote_desktop_authority.clone();
            session.remote_control.apply(&settings, desktop);
        }
        session
    }

    #[cfg(feature = "backend-winit")]
    pub fn set_winit_redraw_window(
        &mut self,
        window: &dyn smithay::reexports::winit::window::Window,
    ) {
        self.winit_redraw_window = Some(std::ptr::from_ref(window));
    }

    #[cfg(feature = "backend-winit")]
    pub fn request_output_redraw(&self) {
        let Some(window) = self.winit_redraw_window else {
            return;
        };
        // SAFETY: Smithay owns this window in an Arc for exactly the lifetime
        // of the winit backend and this session state. Moving the backend does
        // not move the Arc allocation, and all calls occur on the event thread.
        unsafe { &*window }.request_redraw();
    }

    #[cfg(not(feature = "backend-winit"))]
    pub fn request_output_redraw(&self) {}

    pub fn secure_storage_state_handle(&self) -> Arc<AtomicU8> {
        Arc::clone(&self.secure_storage_state)
    }

    pub fn secure_storage_retry_handle(&self) -> Arc<std::sync::atomic::AtomicBool> {
        Arc::clone(&self.secure_storage_retry)
    }

    pub(crate) fn secure_storage_state(
        &self,
    ) -> crate::session::login_services::SecureStorageState {
        crate::session::login_services::SecureStorageState::from_u8(
            self.secure_storage_state.load(Ordering::Acquire),
        )
    }

    pub fn shell_recovery_visible(&self) -> bool {
        crate::session::shell_recovery_visible_for(self.shell_failure_count)
    }

    pub(crate) fn retry_shell_from_recovery(&mut self) -> bool {
        if !self.shell_recovery_visible() {
            return false;
        }
        false
    }

    pub(crate) fn exit_from_recovery(&mut self) -> bool {
        if !self.shell_recovery_visible() {
            return false;
        }
        self.loop_signal.stop();
        true
    }

    pub fn toggle_launcher(&mut self) {
        // IPC callers do not carry a native event object. Pointer and touch input
        // update interaction history before the shell requests this transition.
        self.toggle_launcher_from(InvocationSource::RecentInteraction);
    }

    fn toggle_launcher_from(&mut self, source: InvocationSource) {
        let visible = self.internal_shell.as_ref().map_or_else(
            || self.launcher_visibility.is_visible(),
            |shell| shell.launcher_visible(),
        );
        self.set_launcher_visible_from(!visible, source);
    }

    fn apply_output_layout(
        &mut self,
        layout: nickel_session_protocol::OutputLayout,
    ) -> Result<(), &'static str> {
        let primary = layout.primary;
        let mut placements = layout.placements;
        if primary.is_empty() {
            return Err("missing primary output");
        }
        if placements.is_empty() {
            return Err("output layout is empty");
        }
        let connected: HashMap<_, _> = self
            .space
            .outputs()
            .filter_map(|output| {
                self.space
                    .output_geometry(output)
                    .map(|geometry| (output.name(), (output.clone(), geometry.size)))
            })
            .collect();
        #[cfg(feature = "backend-udev")]
        let mut connected = connected;
        #[cfg(feature = "backend-udev")]
        for (name, size) in self.native_output_inventory() {
            connected.entry(name.clone()).or_insert_with(|| {
                let output = self
                    .native
                    .as_ref()
                    .and_then(|native| {
                        native
                            .disabled_outputs()
                            .find(|output| output.name() == name)
                            .cloned()
                    })
                    .expect("disabled inventory retains its output");
                (output, size)
            });
        }
        if placements.len() != connected.len() {
            return Err("layout must include every connected output");
        }
        let mut names = HashSet::new();
        if placements.iter().any(|placement| {
            !connected.contains_key(&placement.name)
                || !names.insert(&placement.name)
                || !(60..=480).contains(&placement.scale_120)
        }) {
            return Err("layout contains an unknown, duplicate, or invalidly scaled output");
        }
        if !placements
            .iter()
            .any(|placement| placement.name == primary && placement.enabled)
        {
            return Err("primary output must be enabled");
        }
        let minimum_x = placements
            .iter()
            .map(|placement| placement.x)
            .min()
            .unwrap_or(0);
        let minimum_y = placements
            .iter()
            .map(|placement| placement.y)
            .min()
            .unwrap_or(0);
        for placement in &mut placements {
            placement.x -= minimum_x;
            placement.y -= minimum_y;
        }
        for (index, left) in placements
            .iter()
            .enumerate()
            .filter(|(_, output)| output.enabled)
        {
            let scaled_size = |placement: &nickel_session_protocol::OutputPlacement| {
                let (output, fallback) = &connected[&placement.name];
                output.current_mode().map_or(*fallback, |mode| {
                    output
                        .current_transform()
                        .transform_size(mode.size)
                        .to_f64()
                        .to_logical(f64::from(placement.scale_120) / 120.0)
                        .to_i32_round()
                })
            };
            let left_size = scaled_size(left);
            for right in placements
                .iter()
                .skip(index + 1)
                .filter(|output| output.enabled)
            {
                let right_size = scaled_size(right);
                let overlaps = left.x < right.x + right_size.w
                    && left.x + left_size.w > right.x
                    && left.y < right.y + right_size.h
                    && left.y + left_size.h > right.y;
                if overlaps {
                    return Err("outputs may touch but cannot overlap");
                }
            }
        }

        #[cfg(feature = "backend-udev")]
        for placement in placements.iter().filter(|placement| placement.enabled) {
            self.set_native_output_enabled(&placement.name, true)?;
        }
        #[cfg(not(feature = "backend-udev"))]
        if placements.iter().any(|placement| !placement.enabled) {
            return Err("output disabling requires the native DRM backend");
        }
        self.primary_output_name = Some(primary.clone());
        #[cfg(feature = "backend-udev")]
        for placement in placements.iter().filter(|placement| !placement.enabled) {
            self.set_native_output_enabled(&placement.name, false)?;
        }
        for placement in placements.iter().filter(|placement| placement.enabled) {
            let output = self
                .space
                .outputs()
                .find(|output| output.name() == placement.name)
                .cloned()
                .ok_or("enabled output did not become active")?;
            let location = (placement.x, placement.y).into();
            output.change_current_state(
                None,
                None,
                Some(OutputScale::Fractional(
                    f64::from(placement.scale_120) / 120.0,
                )),
                Some(location),
            );
            self.space.map_output(&output, location);
            self.output_scale_preferences.set(
                stable_output_identity(&output),
                nickel_core::dpi::Scale120::new(placement.scale_120).unwrap_or_default(),
            );
        }
        if !self.test_control_enabled {
            self.output_scale_preferences
                .save_default()
                .map_err(|_| "could not persist output scales")?;
        }
        self.rescue_stranded_windows();
        self.relayout_shell_surfaces();
        self.reconstrain_all_reactive_popups();
        self.space.refresh();
        self.refresh_surface_scales();
        self.notify_protocol_snapshot();
        Ok(())
    }

    fn rescue_stranded_windows(&mut self) {
        let outputs: Vec<_> = self
            .space
            .outputs()
            .filter_map(|output| self.space.output_geometry(output))
            .collect();
        let Some(fallback) = self
            .output_geometry()
            .map(|geometry| (geometry.x, geometry.y))
        else {
            return;
        };
        let stranded: Vec<_> = self
            .space
            .elements()
            .filter(|window| !self.is_shell_owned_window(window))
            .filter(|window| {
                self.space
                    .element_bbox(window)
                    .is_some_and(|bounds| !outputs.iter().any(|output| output.overlaps(bounds)))
            })
            .cloned()
            .collect();
        for window in stranded {
            self.map_compositor_moved_window(window, fallback.into(), false);
        }
    }

    pub(crate) fn stage_output_removal(&mut self, output: &Output) {
        self.fail_image_copy_frames(
            output,
            smithay::wayland::image_copy_capture::CaptureFailureReason::Stopped,
        );
        let Some(removed) = self.space.output_geometry(output) else {
            return;
        };
        let Some(fallback) = self
            .space
            .outputs()
            .filter(|candidate| *candidate != output)
            .find_map(|candidate| self.space.output_geometry(candidate))
        else {
            return;
        };
        let removed_geometry = Geometry {
            x: removed.loc.x,
            y: removed.loc.y,
            width: removed.size.w,
            height: removed.size.h,
        };
        let fallback_geometry = shell_layout::work_area(Geometry {
            x: fallback.loc.x,
            y: fallback.loc.y,
            width: fallback.size.w,
            height: fallback.size.h,
        });
        let output_geometries = self
            .space
            .outputs()
            .filter_map(|output| self.space.output_geometry(output))
            .map(|geometry| Geometry {
                x: geometry.loc.x,
                y: geometry.loc.y,
                width: geometry.size.w,
                height: geometry.size.h,
            })
            .collect::<Vec<_>>();
        let mut displaced = Vec::new();
        let mapped = self
            .space
            .elements()
            .filter_map(|window| {
                let id = window
                    .wl_surface()
                    .and_then(|surface| self.surface_windows.get(&surface.id()))
                    .copied()?;
                self.workspaces.workspace_for(&id)?;
                let bounds = self.space.element_bbox(window)?;
                let geometry = Geometry {
                    x: bounds.loc.x,
                    y: bounds.loc.y,
                    width: bounds.size.w,
                    height: bounds.size.h,
                };
                (shell_layout::output_for_window(geometry, &output_geometries)
                    == Some(removed_geometry))
                .then(|| (id, window.clone(), bounds.loc, bounds.size))
            })
            .collect::<Vec<_>>();
        for (id, window, location, size) in mapped {
            let relative_location: Point<i32, Logical> =
                (location.x - removed.loc.x, location.y - removed.loc.y).into();
            let rescue_location = clamp_window_location(
                (
                    fallback_geometry.x + relative_location.x,
                    fallback_geometry.y + relative_location.y,
                )
                    .into(),
                size,
                fallback_geometry,
            );
            self.map_compositor_moved_window(window, rescue_location, false);
            displaced.push(DisplacedWindow {
                id,
                relative_location,
                rescue_location,
            });
        }
        for (id, (window, location)) in &mut self.minimized_windows {
            let size = window.geometry().size;
            let geometry = Geometry {
                x: location.x,
                y: location.y,
                width: size.w,
                height: size.h,
            };
            if shell_layout::output_for_window(geometry, &output_geometries)
                != Some(removed_geometry)
            {
                continue;
            }
            let relative_location: Point<i32, Logical> =
                (location.x - removed.loc.x, location.y - removed.loc.y).into();
            let rescue_location = clamp_window_location(
                (
                    fallback_geometry.x + relative_location.x,
                    fallback_geometry.y + relative_location.y,
                )
                    .into(),
                size,
                fallback_geometry,
            );
            *location = rescue_location;
            displaced.push(DisplacedWindow {
                id: *id,
                relative_location,
                rescue_location,
            });
        }
        for (id, (window, location)) in &mut self.workspace_hidden_windows {
            let size = window.geometry().size;
            let geometry = Geometry {
                x: location.x,
                y: location.y,
                width: size.w,
                height: size.h,
            };
            if shell_layout::output_for_window(geometry, &output_geometries)
                != Some(removed_geometry)
            {
                continue;
            }
            let relative_location: Point<i32, Logical> =
                (location.x - removed.loc.x, location.y - removed.loc.y).into();
            let rescue_location = clamp_window_location(
                (
                    fallback_geometry.x + relative_location.x,
                    fallback_geometry.y + relative_location.y,
                )
                    .into(),
                size,
                fallback_geometry,
            );
            *location = rescue_location;
            displaced.push(DisplacedWindow {
                id: *id,
                relative_location,
                rescue_location,
            });
        }
        self.displaced_output_windows
            .insert(output.name(), displaced);
    }

    pub(crate) fn restore_output_windows(&mut self, output: &Output) {
        let Some(displaced) = self.displaced_output_windows.remove(&output.name()) else {
            return;
        };
        let Some(geometry) = self.space.output_geometry(output) else {
            return;
        };
        let work_area = shell_layout::work_area(Geometry {
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w,
            height: geometry.size.h,
        });
        for displaced in displaced {
            let desired = (
                geometry.loc.x + displaced.relative_location.x,
                geometry.loc.y + displaced.relative_location.y,
            )
                .into();
            if let Some((window, location)) = self.minimized_windows.get_mut(&displaced.id) {
                if *location == displaced.rescue_location {
                    *location = clamp_window_location(desired, window.geometry().size, work_area);
                }
                continue;
            }
            if let Some((window, location)) = self.workspace_hidden_windows.get_mut(&displaced.id) {
                if *location == displaced.rescue_location {
                    *location = clamp_window_location(desired, window.geometry().size, work_area);
                }
                continue;
            }
            let window = self.space.elements().find(|window| {
                window
                    .wl_surface()
                    .and_then(|surface| self.surface_windows.get(&surface.id()))
                    .copied()
                    == Some(displaced.id)
            });
            if let Some(window) = window.cloned()
                && self.space.element_location(&window) == Some(displaced.rescue_location)
            {
                let location = clamp_window_location(desired, window.geometry().size, work_area);
                self.map_compositor_moved_window(window, location, false);
            }
        }
        self.relayout_maximized_windows();
        self.relayout_fullscreen_windows();
    }

    fn apply_workspace_transition(&mut self, transition: WorkspaceTransition<WindowId>) {
        self.cancel_remote_keyboard();
        self.hide_overlays();
        for id in transition.hide {
            if let Some(surface) = self.internal_surface_for_window(id) {
                self.internal_ui.set_visible(surface, false);
                continue;
            }
            if self.minimized_windows.contains_key(&id)
                || self.workspace_hidden_windows.contains_key(&id)
            {
                continue;
            }
            let window = self.space.elements().find(|window| {
                window
                    .wl_surface()
                    .and_then(|surface| self.surface_windows.get(&surface.id()))
                    .copied()
                    == Some(id)
            });
            if let Some(window) = window.cloned() {
                let location = self.space.element_location(&window).unwrap_or_default();
                window.set_activated(false);
                self.space.unmap_elem(&window);
                self.workspace_hidden_windows.insert(id, (window, location));
            }
        }
        for id in transition.show {
            if let Some(surface) = self.internal_surface_for_window(id) {
                if !self.internal_minimized_windows.contains(&id) {
                    self.internal_ui.set_visible(surface, true);
                }
                continue;
            }
            if self.minimized_windows.contains_key(&id) {
                continue;
            }
            if let Some((window, location)) = self.workspace_hidden_windows.remove(&id) {
                self.map_buffered_window(window, location, true);
            }
        }
        if let Some(focus) = transition.focus {
            self.activate_window(focus);
        } else {
            self.windows.deactivate_all();
            self.seat
                .get_keyboard()
                .unwrap()
                .set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        self.sync_internal_window_decorations();
        self.schedule_internal_ui_frame();
        self.raise_panels();
        self.request_output_redraw();
        self.notify_workspace_state();
        self.notify_protocol_snapshot();
    }

    fn apply_configured_workspace_count(&mut self) {
        let requested = usize::from(ShellSettings::load_default().desktop_count);
        let Ok(transitions) = self.workspaces.set_count(requested) else {
            return;
        };
        for transition in transitions {
            self.apply_workspace_transition(transition);
        }
        self.notify_workspace_state();
    }

    pub fn switch_workspace_direction(
        &mut self,
        direction: nickel_core::workspaces::WorkspaceDirection,
    ) {
        let target = self.workspaces.neighbor(direction);
        let output = self.output_name_at_pointer();
        if let Ok(transition) = self.workspaces.switch_to(target, output) {
            self.apply_workspace_transition(transition);
        }
    }

    pub fn switch_workspace_number(&mut self, number: usize) {
        let Some(target) = self
            .workspaces
            .ordered()
            .get(number.saturating_sub(1))
            .map(|workspace| workspace.id)
        else {
            tracing::debug!(number, "workspace shortcut target is not configured");
            return;
        };
        let output = self.output_name_at_pointer();
        if let Ok(transition) = self.workspaces.switch_to(target, output) {
            self.apply_workspace_transition(transition);
        }
    }

    pub fn move_active_window_to_workspace(
        &mut self,
        direction: nickel_core::workspaces::WorkspaceDirection,
    ) {
        let Some(window) = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)
            .map(|window| window.id)
        else {
            return;
        };
        let target = self.workspaces.neighbor(direction);
        if let Ok(transition) = self.workspaces.move_window(&window, target) {
            self.apply_workspace_transition(transition);
        }
    }

    pub fn create_workspace_and_switch(&mut self) {
        let Ok(target) = self.workspaces.create() else {
            return;
        };
        let output = self.output_name_at_pointer();
        if let Ok(transition) = self.workspaces.switch_to(target, output) {
            self.apply_workspace_transition(transition);
        }
    }

    pub fn remove_active_workspace(&mut self) {
        if let Ok(transition) = self.workspaces.remove(self.workspaces.active()) {
            self.apply_workspace_transition(transition);
        }
    }

    pub fn close_active_window(&mut self) {
        if let Some(id) = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)
            .map(|window| window.id)
        {
            self.close_window(id);
        }
    }

    pub fn maximize_active_window(&mut self) {
        if let Some(id) = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)
            .map(|window| window.id)
        {
            self.maximize_window(id);
        }
    }

    pub fn minimize_active_window(&mut self) {
        if let Some(id) = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)
            .map(|window| window.id)
        {
            self.minimize_window(id);
        }
    }

    pub fn toggle_show_desktop(&mut self) {
        if self.locked {
            return;
        }
        if self.shortcut_desktop_windows.is_empty() {
            let ids = self
                .windows
                .snapshot()
                .into_iter()
                .filter(|window| {
                    !self.minimized_windows.contains_key(&window.id)
                        && self.workspaces.is_visible(&window.id)
                })
                .map(|window| window.id)
                .collect::<Vec<_>>();
            self.shortcut_desktop_focus = self
                .windows
                .snapshot()
                .into_iter()
                .find(|window| window.active)
                .map(|window| window.id);
            for id in ids.iter().copied() {
                self.minimize_window(id);
            }
            self.shortcut_desktop_windows = ids;
        } else {
            let ids = std::mem::take(&mut self.shortcut_desktop_windows);
            let focus = self.shortcut_desktop_focus.take();
            for id in ids.iter().copied().filter(|id| Some(*id) != focus) {
                if self.window_exists(nickel_session_protocol::WindowId(id.0)) {
                    self.activate_window(id);
                }
            }
            if let Some(id) = focus.filter(|id| ids.contains(id))
                && self.window_exists(nickel_session_protocol::WindowId(id.0))
            {
                self.activate_window(id);
            }
        }
    }

    pub fn snap_active_window(&mut self, leading: bool) {
        let Some(id) = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)
            .map(|window| window.id)
        else {
            return;
        };
        if self
            .protocol_windows()
            .into_iter()
            .any(|candidate| candidate.id.0 == id.0 && candidate.fullscreen)
        {
            return;
        }
        let Some(window) = self.window_for_registry_id(id) else {
            return;
        };
        let Some(output) = self.space.outputs_for_element(&window).into_iter().next() else {
            return;
        };
        let Some(frame) = self.space.element_geometry(&window) else {
            return;
        };
        let output = self.space.output_geometry(&output).unwrap_or(frame);
        let area = self.work_area_for_output(Geometry {
            x: output.loc.x,
            y: output.loc.y,
            width: output.size.w,
            height: output.size.h,
        });
        self.shortcut_snap_restore.entry(id).or_insert(frame);
        let width = (area.width / 2).max(1);
        let leading = if locale_is_rtl() { !leading } else { leading };
        let x = if leading {
            area.x
        } else {
            area.x + area.width - width
        };
        let target =
            smithay::utils::Rectangle::new((x, area.y).into(), (width, area.height).into());
        if let Some(surface) = window.x11_surface() {
            let _ = surface.configure(target);
        }
        self.map_compositor_moved_window(window, target.loc, true);
        if let Some(surface) = self
            .window_for_registry_id(id)
            .and_then(|window| window.toplevel().cloned())
        {
            surface.with_pending_state(|state| state.size = Some((width, area.height).into()));
            surface.send_pending_configure();
        }
        self.notify_protocol_snapshot();
    }

    pub fn restore_or_minimize_active_window(&mut self) {
        let Some(snapshot) = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)
            .cloned()
        else {
            return;
        };
        if let Some(restore) = self.shortcut_snap_restore.remove(&snapshot.id) {
            let Some(window) = self.window_for_registry_id(snapshot.id) else {
                return;
            };
            let location = self
                .output_geometry_for_window(&window)
                .map(|output| {
                    clamp_window_location(
                        restore.loc,
                        restore.size,
                        self.work_area_for_output(output),
                    )
                })
                .unwrap_or(restore.loc);
            self.map_compositor_moved_window(window.clone(), location, true);
            if let Some(surface) = window.toplevel() {
                surface.with_pending_state(|state| state.size = Some(restore.size));
                surface.send_pending_configure();
            }
        } else if self
            .protocol_windows()
            .into_iter()
            .any(|window| window.id.0 == snapshot.id.0 && window.maximized)
        {
            self.maximize_window(snapshot.id);
        } else {
            self.minimize_window(snapshot.id);
        }
    }

    pub fn move_active_window_to_output_direction(&mut self, previous: bool) {
        let Some(id) = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)
            .map(|window| window.id)
        else {
            return;
        };
        let Some(window) = self.window_for_registry_id(id) else {
            return;
        };
        let current = self.output_name_for_window(&window);
        let mut outputs = self
            .space
            .outputs()
            .filter_map(|output| {
                self.space
                    .output_geometry(output)
                    .map(|geometry| (geometry.loc.x, output.name()))
            })
            .collect::<Vec<_>>();
        outputs.sort_by_key(|entry| entry.0);
        let Some(index) = outputs
            .iter()
            .position(|(_, name)| Some(name) == current.as_ref())
        else {
            return;
        };
        let target = if previous {
            index.checked_sub(1)
        } else if index + 1 < outputs.len() {
            Some(index + 1)
        } else {
            None
        };
        if let Some(target) = target {
            self.move_window_to_output(id, &outputs[target].1);
        }
    }

    fn output_name_at_pointer(&self) -> Option<String> {
        let location = self.seat.get_pointer()?.current_location();
        self.output_name_at(location)
    }

    /// Resolves a logical point through the compositor's single output hit
    /// authority. Input handlers record the result rather than reproducing the
    /// output geometry walk themselves.
    fn output_name_at(&self, location: Point<f64, Logical>) -> Option<String> {
        self.space.outputs().find_map(|output| {
            self.space
                .output_geometry(output)
                .filter(|geometry| geometry.to_f64().contains(location))
                .map(|_| output.name())
        })
    }

    pub(crate) fn record_interaction_output(&mut self, location: Point<f64, Logical>) {
        self.last_interaction_output_name = self.output_name_at(location);
    }

    pub fn set_launcher_visible(&mut self, visible: bool) {
        // Panel pointer/touch input has already updated interaction history.
        self.set_launcher_visible_from(visible, InvocationSource::RecentInteraction);
    }

    fn set_launcher_visible_from(&mut self, visible: bool, source: InvocationSource) {
        self.set_launcher_visible_on_output(visible, self.resolve_interaction_output(source));
    }

    pub(super) fn set_launcher_visible_on_output(&mut self, visible: bool, output: Option<String>) {
        let was_visible = self.internal_shell.as_ref().map_or_else(
            || self.launcher_visibility.is_visible(),
            |shell| shell.launcher_visible(),
        );
        let changed = was_visible != visible;
        if changed && visible {
            self.launcher_show_requested_at = Some(std::time::Instant::now());
            self.launcher_output_name = output;
            if self.internal_shell.is_some() && self.launcher_restore_window.is_none() {
                self.launcher_restore_window = self
                    .windows
                    .snapshot()
                    .into_iter()
                    .find(|window| window.active)
                    .map(|window| window.id);
            }
        }
        self.launcher_visibility.set(visible);
        self.hotkeys.launcher_visibility_applied(visible);
        if let Some(shell) = self.internal_shell.as_mut() {
            shell.apply_launcher_visibility(visible);
            if changed {
                self.sync_internal_shell();
                if visible
                    && let Some(runtime) = self.internal_shell.as_ref().and_then(|shell| {
                        shell
                            .surfaces()
                            .iter()
                            .find(|surface| {
                                surface.role == crate::winit_shell::SurfaceRole::Launcher
                            })
                            .and_then(|surface| {
                                self.internal_shell_surfaces.get(&surface.id).copied()
                            })
                    })
                {
                    self.focus_internal_surface(runtime);
                }
                self.wake_internal_shell();
            }
        } else {
            self.apply_launcher_visibility(visible);
        }
        if changed {
            if !visible {
                self.restore_launcher_focus();
            }
            self.notify_launcher_visibility(visible);
        }
    }

    pub fn toggle_launcher_visibility(&mut self) {
        if self.toggle_internal_launcher() {
            return;
        }
        self.set_launcher_visible_from(
            !self.launcher_visibility.is_visible(),
            InvocationSource::Keyboard,
        );
    }

    pub fn launcher_pointer_press(
        &mut self,
        target: LauncherPointerTarget,
        restore_window_focus: bool,
    ) -> bool {
        if self.launcher_visibility.pointer_press(target) {
            self.hotkeys.launcher_visibility_applied(false);
            self.apply_launcher_visibility(false);
            if restore_window_focus {
                self.restore_launcher_focus();
            } else {
                self.launcher_restore_window = None;
            }
            self.notify_launcher_visibility(false);
            restore_window_focus
        } else {
            false
        }
    }

    pub fn launcher_keyboard_focus_changed(&mut self, focused: Option<&WlSurface>) {
        if let Some(focused) = focused
            && let Some(request) = self.launcher_focus.requested().cloned()
            && focused.id() == request.surface
        {
            let _ = self.launcher_focus.acknowledge(&request);
            return;
        }
        let Some(acknowledged) = self.launcher_focus.acknowledged().cloned() else {
            return;
        };
        if self.launcher_visibility.is_visible() && self.launcher_focus.loses_current(&acknowledged)
        {
            self.launcher_visibility.set(false);
            self.hotkeys.launcher_visibility_applied(false);
            self.apply_launcher_visibility(false);
            if let Some(window) = self.launcher_restore_window.take() {
                let _ = self.deferred_focus_restore.send(window);
            }
            self.notify_launcher_visibility(false);
        }
    }

    fn notify_launcher_visibility(&mut self, visible: bool) {
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::LauncherVisibility { visible }),
        }) else {
            return;
        };
        let Ok(socket) = notification_socket() else {
            return;
        };
        self.launcher_subscribers
            .retain(|path| socket.send_to(&event, path).is_ok());
        self.notify_protocol_snapshot();
    }

    pub(crate) fn observe_pending_launch_window(&mut self, window: WindowId) {
        // Identity may complete before the first Wayland buffer is mapped.
        // Acknowledgement requires both verified ownership and a mapped window.
        if self.window_for_registry_id(window).is_none() {
            return;
        }
        let Some(client_pid) = self
            .remote_window_identities
            .get(&window)
            .and_then(super::remote_identity::WindowIdentity::current_process_id)
        else {
            return;
        };
        let now = Instant::now();
        let mut observations = Vec::new();
        self.pending_launch_observations
            .retain(
                |pending| match pending_launch_window_disposition(pending, now, client_pid) {
                    PendingLaunchWindowDisposition::AwaitExpiry
                    | PendingLaunchWindowDisposition::Unrelated => true,
                    PendingLaunchWindowDisposition::Attributed { descendant } => {
                        let elapsed = now.saturating_duration_since(pending.registered_at);
                        observations.push((
                            pending.generation,
                            elapsed.as_millis().min(u128::from(u16::MAX)) as u16,
                            descendant,
                        ));
                        false
                    }
                },
            );
        if observations.is_empty() {
            return;
        }
        let Ok(socket) = notification_socket() else {
            return;
        };
        for (generation, observed_after_ms, descendant) in observations {
            let Ok(event) = encode(&ServerEnvelope {
                request_id: 0,
                message: ServerMessage::Event(SessionEvent::PendingLaunchWindow {
                    generation,
                    observed_after_ms,
                    descendant,
                }),
            }) else {
                continue;
            };
            self.launcher_subscribers
                .retain(|path| socket.send_to(&event, path).is_ok());
        }
    }

    fn notify_pending_launch_expired(&mut self, generation: u64) {
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::PendingLaunchExpired { generation }),
        }) else {
            return;
        };
        let Ok(socket) = notification_socket() else {
            return;
        };
        self.launcher_subscribers
            .retain(|path| socket.send_to(&event, path).is_ok());
    }

    fn notify_workspace_state(&mut self) {
        self.remote_desktop_events.record_workspace_state(
            self.workspaces.active().0,
            self.workspaces.ordered().len(),
            self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
        );
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::Workspaces(self.protocol_workspaces())),
        }) else {
            return;
        };
        let Ok(socket) = notification_socket() else {
            return;
        };
        self.launcher_subscribers
            .retain(|path| socket.send_to(&event, path).is_ok());
    }

    fn notify_shell_settings_changed(&mut self) {
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::ShellSettingsChanged),
        }) else {
            return;
        };
        let Ok(socket) = notification_socket() else {
            return;
        };
        self.launcher_subscribers
            .retain(|path| socket.send_to(&event, path).is_ok());
    }

    fn refresh_output_topology_generation(&mut self) -> bool {
        let outputs = self.protocol_outputs();
        if outputs != self.last_protocol_outputs {
            self.last_protocol_outputs = outputs;
            self.output_topology_generation =
                self.output_topology_generation.wrapping_add(1).max(1);
            if self.internal_shell.is_some() {
                self.reconcile_internal_shell_outputs();
            }
            true
        } else {
            false
        }
    }

    fn protocol_shell_behavior(&self) -> ShellBehaviorSnapshot {
        let settings = ShellSettings::load_default();
        ShellBehaviorSnapshot {
            bar_on_all_displays: settings.bar_on_all_displays,
            all_windows_on_every_bar: settings.all_windows_on_every_bar,
            desktop_count: settings.desktop_count,
            topology_generation: self.output_topology_generation,
        }
    }

    fn notify_shell_behavior_snapshot(&mut self, snapshot: ShellBehaviorSnapshot) {
        let event = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::ShellBehaviorChanged(snapshot)),
        });
        if let Ok(event) = event
            && let Ok(socket) = notification_socket()
        {
            self.launcher_subscribers
                .retain(|path| socket.send_to(&event, path).is_ok());
        }
    }

    fn apply_shell_behavior_transaction(
        &mut self,
        transaction: ShellBehaviorTransaction,
    ) -> ServerMessage {
        let _ = self.refresh_output_topology_generation();
        match self.commit_shell_behavior_transaction(transaction, None) {
            Ok((effective, transitions)) => {
                self.reconcile_shell_behavior_commit(&effective, transitions);
                ServerMessage::ShellBehavior(effective)
            }
            Err(error) => *error,
        }
    }

    /// Commit configuration and workspace policy together. No shell polling or
    /// control-authority callbacks occur here; callers reconcile the committed
    /// state synchronously on this same owner before processing another request.
    fn commit_shell_behavior_transaction(
        &mut self,
        transaction: ShellBehaviorTransaction,
        commit_deadline: Option<Instant>,
    ) -> Result<(ShellBehaviorSnapshot, Vec<WorkspaceTransition<WindowId>>), Box<ServerMessage>>
    {
        if transaction.topology_generation != self.output_topology_generation {
            return Err(Box::new(protocol_error(
                ErrorCode::InvalidRequest,
                format!(
                    "stale output topology generation {}; current generation is {}",
                    transaction.topology_generation, self.output_topology_generation
                ),
            )));
        }
        let previous = nickel_core::shell_settings::settings_path()
            .and_then(ShellSettings::load_for_update)
            .map_err(|error| {
                Box::new(protocol_error(
                    ErrorCode::Internal,
                    format!("could not read shell settings for transaction: {error}"),
                ))
            })?;
        let requested = match prepare_shell_behavior_update(
            &previous,
            self.output_topology_generation,
            &transaction,
        ) {
            Ok(requested) => requested,
            Err(error) => return Err(Box::new(protocol_error(ErrorCode::InvalidRequest, error))),
        };
        let persistence = nickel_core::shell_settings::settings_path().and_then(|path| {
            requested.save_checked(path, || {
                if commit_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "settings commit expired",
                    ))
                } else {
                    Ok(())
                }
            })
        });
        if let Err(error) = persistence {
            return Err(Box::new(protocol_error(
                ErrorCode::Internal,
                format!("could not persist shell setting: {error}"),
            )));
        }
        let requested_count = usize::from(requested.desktop_count);
        let transitions = match self.workspaces.set_count(requested_count) {
            Ok(transitions) => transitions,
            Err(error) => {
                let rollback = previous.save_default();
                return Err(Box::new(protocol_error(
                    ErrorCode::Internal,
                    format!(
                        "could not apply shell setting: {error:?}; persistence rollback: {}",
                        if rollback.is_ok() {
                            "complete"
                        } else {
                            "failed"
                        }
                    ),
                )));
            }
        };
        Ok((self.protocol_shell_behavior(), transitions))
    }

    fn reconcile_shell_behavior_commit(
        &mut self,
        effective: &ShellBehaviorSnapshot,
        transitions: Vec<WorkspaceTransition<WindowId>>,
    ) {
        for transition in transitions {
            self.apply_workspace_transition(transition);
        }
        if let Some(shell) = self.internal_shell.as_mut()
            && shell.set_bar_on_all_displays(effective.bar_on_all_displays)
        {
            self.reconcile_internal_shell_outputs();
        }
        self.notify_shell_behavior_snapshot(effective.clone());
    }

    pub(crate) fn notify_global_shortcut(
        &mut self,
        action: nickel_session_protocol::ShortcutAction,
    ) {
        tracing::info!(?action, "global shortcut activated");
        let screenshot_output = (action
            == nickel_session_protocol::ShortcutAction::ShowScreenshotTool)
            .then(|| self.preferred_interaction_output_name());
        if let Some(shell) = self.internal_shell.as_mut() {
            if let Some(output) = screenshot_output {
                shell.set_screenshot_output(output);
            }
            let changed = shell.global_shortcut(action);
            if changed {
                self.sync_internal_shell();
                self.schedule_internal_ui_frame();
            }
            self.wake_internal_shell();
            return;
        }
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::GlobalShortcut { action }),
        }) else {
            return;
        };
        let Ok(socket) = notification_socket() else {
            return;
        };
        self.launcher_subscribers
            .retain(|path| socket.send_to(&event, path).is_ok());
    }

    pub(crate) fn notify_consumer_control(
        &mut self,
        control: nickel_session_protocol::ConsumerControl,
    ) {
        tracing::info!(
            ?control,
            subscribers = self.launcher_subscribers.len(),
            "consumer control activated"
        );
        if let Some(shell) = self.internal_shell.as_mut() {
            if shell.consumer_control(control) {
                let changed = shell
                    .surfaces()
                    .iter()
                    .filter(|surface| surface.role == crate::winit_shell::SurfaceRole::VolumeOsd)
                    .map(|surface| surface.id)
                    .collect::<Vec<_>>();
                self.sync_internal_shell_changes(Some(&changed));
            }
            self.wake_internal_shell();
            return;
        }
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::ConsumerControl { control }),
        }) else {
            return;
        };
        let Ok(socket) = notification_socket() else {
            return;
        };
        self.launcher_subscribers
            .retain(|path| socket.send_to(&event, path).is_ok());
    }

    pub(crate) fn notify_protocol_snapshot(&mut self) {
        // The in-process shell consumes this same canonical snapshot on its
        // event-loop deadline. State changes must wake it just as external
        // subscribers are notified, otherwise the panel can retain stale
        // focus and a pinned-only task list until an unrelated timer fires.
        self.wake_internal_shell();
        if self.refresh_output_topology_generation() {
            self.notify_shell_behavior_snapshot(self.protocol_shell_behavior());
        }
        self.record_remote_window_state_events();
        let event = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::Snapshot(self.protocol_snapshot())),
        });
        self.windows.finish_snapshot();
        let Ok(event) = event else {
            return;
        };
        let Ok(socket) = notification_socket() else {
            return;
        };
        self.launcher_subscribers
            .retain(|path| socket.send_to(&event, path).is_ok());
    }

    pub(crate) fn record_remote_input_ownership(&mut self) {
        let observed_at_us = self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64;
        self.remote_desktop_events.record_remote_input_ownership(
            self.remote_held_keyboard.is_some(),
            self.remote_held_pointer.is_some(),
            observed_at_us,
        );
    }

    fn record_remote_production_effect_outcome(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
        effect: nickel_remote_control::desktop_events::ProductionEffectKind,
        outcome: nickel_remote_control::desktop_events::ProductionEffectOutcome,
    ) {
        let Some(operation_id) = permit.operation_id() else {
            return;
        };
        self.remote_desktop_events.record(
            nickel_remote_control::desktop_events::DesktopEventKind::ProductionEffectCompleted {
                operation_id,
                effect,
                outcome,
            },
            self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
        );
    }

    fn record_remote_settings_transaction<T>(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
        result: &Result<T, String>,
    ) {
        if result.is_ok() {
            self.record_remote_production_effect_outcome(
                permit,
                nickel_remote_control::desktop_events::ProductionEffectKind::SettingsTransaction,
                nickel_remote_control::desktop_events::ProductionEffectOutcome::Confirmed,
            );
        }
    }

    fn record_remote_window_state_events(&mut self) {
        use nickel_remote_control::desktop_events::DesktopEventKind;
        let current = self
            .remote_protocol_windows()
            .into_iter()
            .map(|window| {
                let geometry = window
                    .geometry
                    .map(|value| [value.x, value.y, value.width, value.height]);
                let state = RemoteWindowEventState {
                    geometry,
                    workspace: window.workspace.0,
                    active: window.active,
                    minimized: window.minimized,
                    maximized: window.maximized,
                    fullscreen: window.fullscreen,
                };
                (window.id.0, state)
            })
            .collect::<HashMap<_, _>>();
        let observed_at_us = self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64;
        for (&window_id, &state) in &current {
            if self
                .remote_window_event_states
                .get(&window_id)
                .is_some_and(|previous| previous != &state)
            {
                self.remote_desktop_events.record(
                    DesktopEventKind::WindowStateChanged {
                        window_id,
                        geometry: state.geometry,
                        workspace: state.workspace,
                        active: state.active,
                        minimized: state.minimized,
                        maximized: state.maximized,
                        fullscreen: state.fullscreen,
                    },
                    observed_at_us,
                );
            }
        }
        self.remote_window_event_states = current;
    }

    fn record_remote_output_state_events(&mut self) {
        use nickel_remote_control::desktop_events::DesktopEventKind;
        if self.locked || self.shell_recovery_visible() {
            self.remote_output_event_states.clear();
            return;
        }
        let current = self
            .protocol_outputs()
            .into_iter()
            .filter_map(|output| {
                let generation = self.remote_output_generations.get(&output.name)?.1;
                let geometry = output.geometry;
                let work_area = output.work_area;
                Some((
                    generation,
                    RemoteOutputEventState {
                        geometry: [geometry.x, geometry.y, geometry.width, geometry.height],
                        work_area: [work_area.x, work_area.y, work_area.width, work_area.height],
                        scale_120: output.scale_120,
                        primary: output.primary,
                        enabled: output.enabled,
                    },
                ))
            })
            .collect::<HashMap<_, _>>();
        let observed_at_us = self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64;
        for (&output_generation, &state) in &current {
            if self.remote_output_event_states.get(&output_generation) != Some(&state) {
                self.remote_desktop_events.record(
                    DesktopEventKind::OutputStateChanged {
                        output_generation,
                        geometry: state.geometry,
                        work_area: state.work_area,
                        scale_120: state.scale_120,
                        primary: state.primary,
                        enabled: state.enabled,
                    },
                    observed_at_us,
                );
            }
        }
        self.remote_output_event_states = current;
    }

    fn apply_launcher_visibility(&mut self, visible: bool) {
        let Some(window) = self.launcher_window.clone() else {
            return;
        };
        if visible {
            if self.launcher_restore_window.is_none() {
                self.launcher_restore_window = self
                    .windows
                    .snapshot()
                    .into_iter()
                    .find(|window| window.active)
                    .map(|window| window.id);
            }
            let geometry = self.launcher_geometry(&window);
            let location = Self::shell_surface_location(&window, geometry);
            if !self.map_buffered_window(window.clone(), location, true) {
                return;
            }
            let surface = window.toplevel().unwrap().wl_surface().clone();
            let _request = self.launcher_focus.request(surface.id());
            self.surrender_internal_focus();
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                Some(crate::session::focus::KeyboardFocusTarget::Wayland(surface)),
                SERIAL_COUNTER.next_serial(),
            );
            self.space.elements().for_each(|window| {
                if let Some(toplevel) = window.toplevel() {
                    toplevel.send_pending_configure();
                }
            });
            self.raise_panels();
        } else {
            self.space.unmap_elem(&window);
        }
        eprintln!(
            "nickel: launcher {}",
            if visible { "shown" } else { "hidden" }
        );
    }

    fn restore_launcher_focus(&mut self) {
        if let Some(window) = self.launcher_restore_window.take() {
            self.activate_window(window);
        }
    }

    pub(crate) fn focus_shell_role(&mut self, role: ShellRole) -> bool {
        if !shell_role_accepts_ordinary_focus(role) {
            return false;
        }
        if role == ShellRole::Screenshot {
            self.screenshot_output_name = self.preferred_interaction_output_name();
        }
        let internal_role = match role {
            ShellRole::ControlCenter => Some(crate::winit_shell::SurfaceRole::ControlCenter),
            ShellRole::ProjectMenu => Some(crate::winit_shell::SurfaceRole::CodexProjectMenu),
            ShellRole::Preview => Some(crate::winit_shell::SurfaceRole::WindowPreview),
            ShellRole::ContextMenu => Some(crate::winit_shell::SurfaceRole::WindowContextMenu),
            ShellRole::Screenshot => Some(crate::winit_shell::SurfaceRole::Screenshot),
            _ => None,
        };
        if let Some(runtime) = internal_role.and_then(|role| {
            self.internal_shell.as_ref().and_then(|shell| {
                shell
                    .surfaces()
                    .iter()
                    .find(|surface| surface.role == role && shell.visible(surface.id))
                    .and_then(|surface| self.internal_shell_surfaces.get(&surface.id).copied())
            })
        }) {
            self.pending_shell_focus_role = None;
            return self.focus_internal_surface(runtime);
        }
        self.pending_shell_focus_role = Some(role);
        let registry = self.windows.snapshot();
        let target = self.shell_windows().find_map(|window| {
            let id = window
                .wl_surface()
                .and_then(|surface| self.surface_windows.get(&surface.id()))?;
            let app_id = registry
                .iter()
                .find(|entry| entry.id == *id)?
                .app_id
                .as_str();
            (ShellRole::from_application_id(app_id) == Some(role)).then(|| window.clone())
        });
        let Some(target) = target else {
            return false;
        };
        if self.shell_focus_restore_window.is_none() {
            let shell_ids = self
                .shell_windows()
                .filter_map(|window| {
                    window
                        .wl_surface()
                        .and_then(|surface| self.surface_windows.get(&surface.id()))
                        .copied()
                })
                .collect::<HashSet<_>>();
            let focused = self
                .seat
                .get_keyboard()
                .and_then(|keyboard| keyboard.current_focus())
                .and_then(|focus| match focus {
                    crate::session::focus::KeyboardFocusTarget::Wayland(surface) => {
                        self.surface_windows.get(&surface.id()).copied()
                    }
                    crate::session::focus::KeyboardFocusTarget::X11(surface) => {
                        self.x11_windows.get(&surface.window_id()).copied()
                    }
                })
                .filter(|window| !shell_ids.contains(window));
            self.shell_focus_restore_window = focused.or_else(|| {
                registry
                    .iter()
                    .find(|window| window.active && !shell_ids.contains(&window.id))
                    .map(|window| window.id)
            });
        }
        if role == ShellRole::Screenshot {
            self.place_screenshot_surface(&target);
        }
        self.space.raise_element(&target, true);
        self.surrender_internal_focus();
        self.seat.get_keyboard().unwrap().set_focus(
            self,
            crate::session::focus::KeyboardFocusTarget::for_window(&target),
            SERIAL_COUNTER.next_serial(),
        );
        self.space.elements().for_each(|window| {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_pending_configure();
            }
        });
        true
    }

    fn restore_application_focus(&mut self) {
        if self.pending_shell_focus_role == Some(ShellRole::Screenshot) {
            self.screenshot_output_name = None;
        }
        self.pending_shell_focus_role = None;
        if let Some(window) = self.shell_focus_restore_window.take() {
            self.activate_window(window);
        } else {
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                Option::<crate::session::focus::KeyboardFocusTarget>::None,
                SERIAL_COUNTER.next_serial(),
            );
        }
    }

    pub fn register_launcher(&mut self, window: Window) {
        if let Some(previous) = self.launcher_window.clone()
            && previous != window
        {
            self.retire_replaced_shell_window(previous);
        }
        self.space.unmap_elem(&window);
        self.launcher_window = Some(window);
        self.apply_launcher_visibility(self.launcher_visibility.is_visible());
    }

    fn retire_replaced_shell_window(&mut self, window: Window) {
        self.space.unmap_elem(&window);
        if let Some(toplevel) = window.toplevel() {
            toplevel.send_close();
        }
        let surface_id = window.wl_surface().map(|surface| surface.id());
        let window_id = surface_id
            .as_ref()
            .and_then(|surface| self.surface_windows.get(surface))
            .copied();
        self.retire_surface_window_references(surface_id.as_ref(), window_id);
    }

    fn retire_shell_surface_roles(&mut self) {
        // A replacement shell generation must never expose the previous
        // generation through the ordinary Space render/input paths or retain
        // its derived identity until a later destruction callback.
        let retired = self.shell_windows().cloned().collect::<Vec<_>>();
        for window in retired {
            self.space.unmap_elem(&window);
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_close();
            }
            let surface_id = window.wl_surface().map(|surface| surface.id());
            let window_id = surface_id
                .as_ref()
                .and_then(|surface| self.surface_windows.get(surface))
                .copied();
            self.retire_surface_window_references(surface_id.as_ref(), window_id);
        }
        self.launcher_window = None;
        self.desktop_windows.clear();
        self.panel_windows.clear();
        self.lock_windows.clear();
        self.utility_windows.clear();
        self.context_menu_window = None;
        self.preview_window = None;
        self.registered_shell_role_slots.clear();
        self.clear_all_previews();
    }

    /// Remove every derived reference owned for a surface/window identity.
    ///
    /// Destruction callbacks must use this operation instead of independently
    /// editing lifecycle collections. X11 windows can lack a Wayland surface,
    /// while shell surfaces can be retired before their client destroys the
    /// underlying object, so both halves of the identity are optional.
    pub(crate) fn retire_surface_window_references(
        &mut self,
        surface_id: Option<&ObjectId>,
        window_id: Option<WindowId>,
    ) {
        if let Some(id) = window_id {
            self.remote_launch_maps.remove(&id);
        }
        if let Some(window) = window_id {
            // Native process liveness may already be gone at destruction.
            // Remember only previously accepted ordinary observation, and drop
            // it on retirement regardless of whether history can be recorded.
            if self.remote_event_windows.remove(&window)
                && self.windows.contains(window)
                && !self.locked
                && !self.shell_recovery_visible()
                && !self.shell_owned_windows.contains(&window)
            {
                self.remote_desktop_events.record(
                    nickel_remote_control::desktop_events::DesktopEventKind::WindowRetired {
                        window_id: window.0,
                    },
                    self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
                );
            }
        }
        if let Some(surface_id) = surface_id {
            let _ = retire_pointer_surface(
                &mut self.pointer_lock_hints,
                &mut self.active_pointer_locks,
                &mut self.active_pointer_constraint_origins,
                surface_id,
            );
            self.server_decorated.remove(surface_id);
            self.maximized_restore.remove(surface_id);
            self.fullscreen_restore.remove(surface_id);
            self.surface_windows.remove(surface_id);
            self.mapped_xdg_toplevels.remove(surface_id);
            self.xdg_toplevel_windows.remove(surface_id);
            self.xdg_toplevel_locations.remove(surface_id);
            retire_shell_surface(&mut self.registered_shell_role_slots, surface_id);
            self.idle_inhibitors
                .retain(|surface, _| surface.id() != *surface_id);
            if self
                .last_titlebar_click
                .as_ref()
                .is_some_and(|(surface, _, _)| surface == surface_id)
            {
                self.last_titlebar_click = None;
            }

            let matches_surface = |window: &Window| {
                window
                    .wl_surface()
                    .is_some_and(|surface| surface.id() == *surface_id)
            };
            if self.launcher_window.as_ref().is_some_and(&matches_surface) {
                self.launcher_window = None;
                self.launcher_visibility.set(false);
            }
            self.desktop_windows
                .retain(|window| !matches_surface(window));
            self.panel_windows.retain(|window| !matches_surface(window));
            self.lock_windows.retain(|window| !matches_surface(window));
            self.utility_windows
                .retain(|window| !matches_surface(window));
            if self
                .context_menu_window
                .as_ref()
                .is_some_and(&matches_surface)
            {
                self.context_menu_window = None;
            }
            if self.preview_window.as_ref().is_some_and(&matches_surface) {
                self.preview_window = None;
                self.clear_overlay_preview_interest();
            }
        }

        if let Some(window_id) = window_id {
            self.surface_windows
                .retain(|_, retained| *retained != window_id);
            self.shell_owned_windows.remove(&window_id);
            self.minimized_windows.remove(&window_id);
            self.workspace_hidden_windows.remove(&window_id);
            self.workspaces.remove_window(&window_id);
            self.remove_window_from_switcher(window_id);
            self.windows.remove(window_id);
            self.remote_window_identities.remove(&window_id);
            self.schedule_remote_resource_retirement();
            self.preview_highlight = self
                .preview_highlight
                .filter(|candidate| *candidate != window_id);
            self.launcher_restore_window = self
                .launcher_restore_window
                .filter(|candidate| *candidate != window_id);
            self.lock_restore_window = self
                .lock_restore_window
                .filter(|candidate| *candidate != window_id);
            self.shell_focus_restore_window = self
                .shell_focus_restore_window
                .filter(|candidate| *candidate != window_id);
            retire_displaced_window(&mut self.displaced_output_windows, window_id);
        }

        // Churn of dead identities must not retain admission-sized backing
        // allocations once the logical collections return to baseline.
        let counts = self.lifecycle_collection_counts();
        tracing::trace!(
            pointer_hints = counts.pointer_hints,
            pointer_locks = counts.pointer_locks,
            pointer_origins = counts.pointer_origins,
            displaced_outputs = counts.displaced_outputs,
            displaced_windows = counts.displaced_windows,
            shell_roles = counts.shell_roles,
            "retired session identity references"
        );
        // Wayland invalidates all objects of a disconnected client before
        // invoking each destruction callback. Other identities can therefore
        // be dead but still awaiting their own retirement here. Assert the
        // postcondition of this retirement, not global liveness mid-batch.
        debug_assert!(self.retired_identity_has_no_references(surface_id, window_id));
    }

    pub(crate) fn retire_pointer_constraint_references(
        &mut self,
        surface_id: &ObjectId,
    ) -> Option<Point<f64, Logical>> {
        retire_pointer_surface(
            &mut self.pointer_lock_hints,
            &mut self.active_pointer_locks,
            &mut self.active_pointer_constraint_origins,
            surface_id,
        )
    }

    pub(crate) fn clear_changed_shell_surface_role(
        &mut self,
        surface_id: &ObjectId,
        next_role: Option<ShellRole>,
    ) {
        let role_changed = shell_registration_role_changed(
            &self.registered_shell_role_slots,
            surface_id,
            next_role,
        );
        if !role_changed {
            return;
        }
        retire_shell_surface(&mut self.registered_shell_role_slots, surface_id);
        let matches_surface = |window: &Window| {
            window
                .wl_surface()
                .is_some_and(|surface| surface.id() == *surface_id)
        };
        if self.launcher_window.as_ref().is_some_and(&matches_surface) {
            self.launcher_window = None;
            self.launcher_visibility.set(false);
        }
        self.desktop_windows
            .retain(|window| !matches_surface(window));
        self.panel_windows.retain(|window| !matches_surface(window));
        self.lock_windows.retain(|window| !matches_surface(window));
        self.utility_windows
            .retain(|window| !matches_surface(window));
        if self
            .context_menu_window
            .as_ref()
            .is_some_and(&matches_surface)
        {
            self.context_menu_window = None;
        }
        if self.preview_window.as_ref().is_some_and(&matches_surface) {
            self.preview_window = None;
            self.clear_overlay_preview_interest();
        }
    }

    pub(crate) fn lifecycle_collection_counts(&self) -> LifecycleCollectionCounts {
        LifecycleCollectionCounts {
            pointer_hints: self.pointer_lock_hints.len(),
            pointer_locks: self.active_pointer_locks.len(),
            pointer_origins: self.active_pointer_constraint_origins.len(),
            displaced_outputs: self.displaced_output_windows.len(),
            displaced_windows: self.displaced_output_windows.values().map(Vec::len).sum(),
            shell_roles: self.registered_shell_role_slots.len(),
        }
    }

    fn retired_identity_has_no_references(
        &self,
        surface_id: Option<&ObjectId>,
        window_id: Option<WindowId>,
    ) -> bool {
        surface_id.is_none_or(|surface| {
            !self.pointer_lock_hints.contains_key(surface)
                && !self.active_pointer_locks.contains(surface)
                && !self.active_pointer_constraint_origins.contains_key(surface)
                && !self
                    .registered_shell_role_slots
                    .iter()
                    .any(|registration| &registration.surface == surface)
                && !self.surface_windows.contains_key(surface)
        }) && window_id.is_none_or(|window| {
            !self.windows.contains(window)
                && !self.remote_event_windows.contains(&window)
                && !self
                    .surface_windows
                    .values()
                    .any(|retained| *retained == window)
                && !self
                    .displaced_output_windows
                    .values()
                    .flatten()
                    .any(|displaced| displaced.id == window)
        })
    }

    pub(crate) fn record_shell_role_registration(
        &mut self,
        window: &Window,
        role: ShellRole,
        output: Option<String>,
    ) {
        if matches!(
            role,
            ShellRole::Desktop | ShellRole::Panel | ShellRole::Lock
        ) && output.is_none()
        {
            return;
        }
        let Some(surface) = window.wl_surface() else {
            return;
        };
        let registration = RegisteredShellRole {
            role,
            output,
            surface: surface.id(),
        };
        // A title/app-id update on the same live surface may change its output
        // slot. Replace that registration rather than treating the original
        // tuple as permanent history.
        retire_shell_surface(&mut self.registered_shell_role_slots, &registration.surface);
        let replacements = self
            .registered_shell_role_slots
            .iter()
            .filter(|existing| {
                existing.role == registration.role
                    && existing.output == registration.output
                    && existing.surface != registration.surface
            })
            .map(|existing| existing.surface.clone())
            .collect::<Vec<_>>();
        for replaced_surface in replacements {
            let window_id = self.surface_windows.get(&replaced_surface).copied();
            let replaced_window = self
                .shell_windows()
                .find(|candidate| {
                    candidate
                        .wl_surface()
                        .is_some_and(|surface| surface.id() == replaced_surface)
                })
                .cloned();
            if let Some(replaced_window) = replaced_window {
                self.retire_replaced_shell_window(replaced_window);
            } else {
                self.retire_surface_window_references(Some(&replaced_surface), window_id);
            }
        }
        if !self
            .registered_shell_role_slots
            .iter()
            .any(|existing| existing.surface == registration.surface)
        {
            self.registered_shell_role_slots.push(registration);
        }
    }

    pub(crate) fn registered_shell_identity(
        &self,
        application_id: Option<&str>,
    ) -> Option<ShellSurfaceIdentity> {
        self.shell_surface_identities.get(application_id?).cloned()
    }

    pub fn register_panel(&mut self, window: Window) {
        // Smithay's ordinary xdg windows use z-index 30. Keep the Nickel panel
        // in its top shell layer so later application maps cannot cover it.
        window.override_z_index(40);
        self.panel_windows.retain(IsAlive::alive);
        if !self.panel_windows.contains(&window) {
            self.panel_windows.push(window);
        }
        self.relayout_shell_surfaces();
    }

    pub fn register_desktop(&mut self, window: Window) {
        window.override_z_index(0);
        self.desktop_windows.retain(IsAlive::alive);
        if !self.desktop_windows.contains(&window) {
            self.desktop_windows.push(window);
        }
        self.relayout_shell_surfaces();
    }

    pub fn is_panel_window(&self, window: &Window) -> bool {
        self.panel_windows.contains(window)
    }

    pub fn is_shell_owned_window(&self, window: &Window) -> bool {
        window
            .wl_surface()
            .and_then(|surface| self.surface_windows.get(&surface.id()))
            .is_some_and(|id| self.shell_owned_windows.contains(id))
    }

    pub fn is_fullscreen_window(&self, window: &Window) -> bool {
        window.x11_surface().is_some_and(|surface| {
            self.x11_fullscreen_restore
                .contains_key(&surface.window_id())
        }) || window.toplevel().is_some_and(|surface| {
            self.fullscreen_restore
                .contains_key(&surface.wl_surface().id())
        })
    }

    pub fn is_maximized_window(&self, window: &Window) -> bool {
        window.x11_surface().is_some_and(|surface| {
            self.x11_maximized_restore
                .contains_key(&surface.window_id())
        }) || window.toplevel().is_some_and(|surface| {
            self.maximized_restore
                .contains_key(&surface.wl_surface().id())
        })
    }

    pub fn is_server_decorated(&self, window: &Window) -> bool {
        window.x11_surface().is_some_and(|surface| {
            self.x11_windows.contains_key(&surface.window_id()) && !surface.is_decorated()
        }) || window
            .toplevel()
            .is_some_and(|surface| self.server_decorated.contains(&surface.wl_surface().id()))
    }

    pub(crate) fn clamp_initial_managed_x11_geometry(
        &self,
        geometry: Rectangle<i32, Logical>,
        requested_position: bool,
        parent_output: Option<&str>,
        cascade: i32,
    ) -> Rectangle<i32, Logical> {
        let content = Geometry {
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w.max(1),
            height: geometry.size.h.max(1),
        };
        let active_output = self.new_window_active_output_name();
        let Some(decision) = shell_layout::resolve_window_output(
            &self.placement_outputs(),
            parent_output,
            requested_position.then_some(content),
            None,
            active_output.as_deref(),
        ) else {
            return geometry;
        };
        let content = if requested_position {
            clamp_decorated_content_to_work_area(content, decision.work_area)
        } else {
            shell_layout::initial_window_sized(
                decision.work_area,
                (content.width, content.height),
                cascade,
            )
        };
        tracing::info!(
            output = %decision.output_name,
            reason = ?decision.reason,
            requested_position,
            "diagnostic: captured X11 new-window placement"
        );
        Rectangle::new(
            (content.x, content.y).into(),
            (content.width, content.height).into(),
        )
    }

    pub(crate) fn map_compositor_moved_window(
        &mut self,
        window: Window,
        location: Point<i32, Logical>,
        activate: bool,
    ) {
        if let Some(surface) = window.x11_surface()
            && self.x11_windows.contains_key(&surface.window_id())
        {
            let configured = surface.last_configure();
            let size = if configured.size.w > 0 && configured.size.h > 0 {
                configured.size
            } else {
                window.geometry().size
            };
            let _ = surface.configure(Rectangle::new(location, size));
        }
        let popup_root = window
            .toplevel()
            .map(|surface| surface.wl_surface().clone());
        self.map_buffered_window(window, location, activate);
        if let Some(root) = popup_root {
            self.reconstrain_reactive_popups(&root);
        }
    }

    pub fn shell_windows(&self) -> impl Iterator<Item = &Window> {
        self.launcher_window
            .iter()
            .chain(self.desktop_windows.iter())
            .chain(self.panel_windows.iter())
            .chain(self.lock_windows.iter())
            .chain(self.utility_windows.iter())
            .chain(self.context_menu_window.iter())
            .chain(self.preview_window.iter())
            .filter(|window| window.alive())
    }

    pub fn register_utility_window(&mut self, window: Window, role: ShellRole) {
        self.utility_windows.retain(IsAlive::alive);
        if !self.utility_windows.contains(&window) {
            self.utility_windows.push(window.clone());
        }
        if role == ShellRole::Screenshot {
            // Cropping is modal over the captured desktop, including the OSK.
            // Keep it below the lock surface (100), but above the keyboard (60).
            window.override_z_index(90);
            self.place_screenshot_surface(&window);
        }
        if role == ShellRole::OnScreenKeyboard {
            window.override_z_index(60);
            if self.on_screen_keyboard_snapshot().visible {
                self.place_on_screen_keyboard_surface(&window);
            } else {
                self.hidden_shell_roles.insert(role);
            }
        }
        if self.hidden_shell_roles.contains(&role) {
            if let Some(location) = self.space.element_location(&window) {
                self.hidden_shell_role_locations.insert(role, location);
            }
            self.space.unmap_elem(&window);
        }
    }

    fn place_screenshot_surface(&mut self, window: &Window) {
        let output_name = self
            .screenshot_output_name
            .clone()
            .or_else(|| self.preferred_interaction_output_name());
        let Some(output) = output_name
            .as_deref()
            .and_then(|name| self.output_geometry_named(name))
            .or_else(|| self.output_geometry_for_shell())
        else {
            return;
        };
        if self.screenshot_output_name.is_none() {
            self.screenshot_output_name = output_name;
        }
        let size = window.geometry().size;
        if size.w <= 0 || size.h <= 0 {
            return;
        }
        let target = shell_layout::centered_in(output, (size.w, size.h));
        if size.w != target.width || size.h != target.height {
            Self::configure_window(window, target);
        }
        let location = Self::shell_surface_location(window, target);
        self.map_buffered_window(window.clone(), location, true);
    }

    pub(crate) fn relayout_committed_shell_window(&mut self, window: &Window) {
        if self.is_on_screen_keyboard_window(window) && self.on_screen_keyboard_snapshot().visible {
            self.place_on_screen_keyboard_surface(window);
            return;
        }
        let is_screenshot = {
            let registry = self.windows.snapshot();
            window
                .wl_surface()
                .and_then(|surface| self.surface_windows.get(&surface.id()))
                .and_then(|id| registry.iter().find(|entry| entry.id == *id))
                .and_then(|entry| ShellRole::from_application_id(&entry.app_id))
                == Some(ShellRole::Screenshot)
        };
        if is_screenshot {
            self.place_screenshot_surface(window);
        }
    }

    pub fn register_lock(&mut self, window: Window) {
        window.override_z_index(100);
        self.lock_windows.retain(IsAlive::alive);
        if !self.lock_windows.contains(&window) {
            // A windowing runtime may recreate native Wayland surfaces when a
            // hidden window is shown. The replacement may register before the
            // old surface's unmap reaches Smithay, so mapped-state alone cannot
            // identify the stale identity. There is exactly one lock surface per output;
            // once that capacity is full, a newly registered identity replaces
            // the oldest one and is the surface that receives configuration.
            let output_count = self.space.outputs().count().max(1);
            while self.lock_windows.len() >= output_count {
                let stale = self.lock_windows.remove(0);
                self.retire_replaced_shell_window(stale);
            }
            self.lock_windows.push(window.clone());
        }
        self.relayout_lock_surfaces();
        if self.locked {
            let focus = window.wl_surface().map(|surface| {
                crate::session::focus::KeyboardFocusTarget::Wayland(surface.into_owned())
            });
            self.surrender_internal_focus();
            self.seat
                .get_keyboard()
                .unwrap()
                .set_focus(self, focus, SERIAL_COUNTER.next_serial());
            let pointer = self.seat.get_pointer().unwrap();
            let location = pointer.current_location();
            pointer.motion(
                self,
                self.pointer_surface_under(location),
                &smithay::input::pointer::MotionEvent {
                    location,
                    serial: SERIAL_COUNTER.next_serial(),
                    time: smithay::backend::input::InputTime::now(),
                },
            );
            pointer.frame(self);
        }
    }

    fn relayout_lock_surfaces(&mut self) {
        let outputs = self.space.outputs().cloned().collect::<Vec<_>>();
        let output_names = outputs.iter().map(Output::name).collect::<Vec<_>>();
        for lock in self.lock_windows.clone() {
            let Some(output_name) = self.shell_surface_output_name(&lock) else {
                continue;
            };
            let Some(output_index) = output_index_for_shell_surface(&output_name, &output_names)
            else {
                continue;
            };
            let output = &outputs[output_index];
            let Some(geometry) = self.space.output_geometry(output) else {
                continue;
            };
            Self::configure_window(
                &lock,
                Geometry {
                    x: geometry.loc.x,
                    y: geometry.loc.y,
                    width: geometry.size.w,
                    height: geometry.size.h,
                },
            );
            if self.locked {
                self.map_buffered_window(lock.clone(), geometry.loc, true);
                self.space.raise_element(&lock, true);
            } else {
                self.space.unmap_elem(&lock);
            }
        }
    }

    pub(crate) fn lock_session(&mut self) {
        self.invalidate_remote_shell_actions();
        if self.locked {
            return;
        }
        self.cancel_remote_pointer();
        self.cancel_remote_keyboard();
        self.locked = true;
        self.remote_control.lock();
        self.sync_remote_control_indicators();
        // The focus transition to the compositor-owned lock surface terminates any
        // in-flight desktop chord. Do not let a missing physical release retain Alt,
        // Super, or the lock key across the secure-session boundary.
        self.hotkeys.reset_chord_state_preserving_owned_releases();
        self.cancel_consumer_control_repeats();
        let shell_ids = self
            .shell_windows()
            .filter_map(|window| self.surface_windows.get(&window.wl_surface()?.id()))
            .copied()
            .collect::<HashSet<_>>();
        self.lock_restore_window = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active && !shell_ids.contains(&window.id))
            .map(|window| window.id);
        self.hide_overlays();
        if let Some(shell) = self.internal_shell.as_mut() {
            shell.set_lock_state(true);
            self.sync_internal_shell();
        }
        self.relayout_lock_surfaces();
        // Snapshot publication can reconcile a newly changed output topology,
        // replacing per-output internal surfaces. Do that before selecting the
        // concrete lock focus target so the chosen runtime identity survives.
        self.notify_lock_state();
        let pointer = self.seat.get_pointer().unwrap();
        let pointer_location = pointer.current_location();
        pointer.motion(
            self,
            self.pointer_surface_under(pointer_location),
            &smithay::input::pointer::MotionEvent {
                location: pointer_location,
                serial: SERIAL_COUNTER.next_serial(),
                time: smithay::backend::input::InputTime::now(),
            },
        );
        pointer.frame(self);
        if !self.active_touch_slots.is_empty() {
            self.active_touch_slots.clear();
            self.seat.get_touch().unwrap().cancel(self);
        }
        let preferred_lock_output = self.preferred_interaction_output_name();
        let internal_lock = self.internal_shell.as_ref().and_then(|shell| {
            let locks = || {
                shell
                    .surfaces()
                    .iter()
                    .filter(|surface| surface.role == crate::winit_shell::SurfaceRole::Lock)
            };
            locks()
                .find(|surface| surface.output.as_deref() == preferred_lock_output.as_deref())
                .and_then(|surface| self.internal_shell_surfaces.get(&surface.id).copied())
                .or_else(|| {
                    locks()
                        .find_map(|surface| self.internal_shell_surfaces.get(&surface.id).copied())
                })
        });
        let focus = self
            .lock_windows
            .first()
            .and_then(Window::wl_surface)
            .map(|surface| {
                crate::session::focus::KeyboardFocusTarget::Wayland(surface.into_owned())
            });
        if let Some(lock) = internal_lock {
            self.focus_internal_surface(lock);
        } else {
            self.surrender_internal_focus();
            self.seat
                .get_keyboard()
                .unwrap()
                .set_focus(self, focus, SERIAL_COUNTER.next_serial());
        }
        // A lock transition replaces the entire visible scene. Damage accumulated against the
        // previous scanout age is not reusable after that occlusion change, particularly when a
        // direct-scanout client was visible. The nested backend redraws wholesale; DRM must reset
        // its buffer-age history explicitly.
        #[cfg(feature = "backend-udev")]
        self.invalidate_native_outputs();
        self.request_output_redraw();
    }

    fn unlock_session(&mut self) {
        if !self.locked {
            return;
        }
        self.locked = false;
        self.hotkeys.reset_pressed_state();
        self.note_input_activity();
        if let Some(shell) = self.internal_shell.as_mut() {
            shell.set_lock_state(false);
            self.sync_internal_shell();
        }
        self.relayout_lock_surfaces();
        if let Some(window) = self.lock_restore_window.take() {
            self.activate_window(window);
        } else {
            self.seat
                .get_keyboard()
                .unwrap()
                .set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        self.notify_lock_state();
        // Exposing ordinary clients after removing the full-output lock scene requires a complete
        // native redraw. Without invalidation, a later lock cycle can reuse damage history from a
        // still-occluded frame and leave only newly changing shell surfaces visible.
        #[cfg(feature = "backend-udev")]
        self.invalidate_native_outputs();
        self.request_output_redraw();
    }

    pub(crate) fn keyboard_focus_is_lock_surface(&self) -> bool {
        let Some(crate::session::focus::KeyboardFocusTarget::Wayland(focused)) =
            self.seat.get_keyboard().unwrap().current_focus()
        else {
            return false;
        };
        self.lock_windows.iter().any(|window| {
            window
                .wl_surface()
                .is_some_and(|surface| surface.as_ref() == &focused)
        })
    }

    fn notify_lock_state(&mut self) {
        let Ok(event) = encode(&ServerEnvelope {
            request_id: 0,
            message: ServerMessage::Event(SessionEvent::LockState {
                locked: self.locked,
            }),
        }) else {
            return;
        };
        let Ok(socket) = notification_socket() else {
            return;
        };
        self.launcher_subscribers
            .retain(|path| socket.send_to(&event, path).is_ok());
        self.notify_protocol_snapshot();
    }

    pub fn register_context_menu(&mut self, window: Window) {
        if let Some(previous) = self.context_menu_window.clone()
            && previous != window
        {
            self.retire_replaced_shell_window(previous);
        }
        window.override_z_index(50);
        self.context_menu_window = Some(window.clone());
        if self.hidden_shell_roles.contains(&ShellRole::ContextMenu) {
            if let Some(location) = self.space.element_location(&window) {
                self.hidden_shell_role_locations
                    .insert(ShellRole::ContextMenu, location);
            }
            self.space.unmap_elem(&window);
        }
    }

    pub fn register_preview(&mut self, window: Window) {
        if let Some(previous) = self.preview_window.clone()
            && previous != window
        {
            self.retire_replaced_shell_window(previous);
        }
        window.override_z_index(49);
        if self
            .preview_window
            .as_ref()
            .is_some_and(|registered| registered != &window)
        {
            self.clear_overlay_preview_interest();
        }
        self.preview_window = Some(window.clone());
        if self.hidden_shell_roles.contains(&ShellRole::Preview) {
            if let Some(location) = self.space.element_location(&window) {
                self.hidden_shell_role_locations
                    .insert(ShellRole::Preview, location);
            }
            self.space.unmap_elem(&window);
        }
    }

    pub fn show_context_menu(
        &mut self,
        x: i32,
        y: i32,
        requested_width: i32,
        requested_height: i32,
        focus: bool,
    ) {
        self.show_transient(
            self.context_menu_window.clone(),
            x,
            y,
            requested_width,
            requested_height,
            focus,
            "context menu",
        );
    }

    pub fn show_preview(&mut self, x: i32, y: i32, requested_width: i32, requested_height: i32) {
        self.show_transient(
            self.preview_window.clone(),
            x,
            y,
            requested_width,
            requested_height,
            false,
            "preview",
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn show_transient(
        &mut self,
        window: Option<Window>,
        x: i32,
        y: i32,
        requested_width: i32,
        requested_height: i32,
        focus: bool,
        label: &str,
    ) {
        let Some(window) = window else {
            return;
        };
        let output = self
            .space
            .outputs()
            .filter_map(|output| self.space.output_geometry(output))
            .find(|geometry| output_contains_logical_point(*geometry, x, y))
            .map(|geometry| Geometry {
                x: geometry.loc.x,
                y: geometry.loc.y,
                width: geometry.size.w,
                height: geometry.size.h,
            })
            .or_else(|| self.output_geometry());
        let Some(output) = output else {
            return;
        };
        let width = requested_width.clamp(120, output.width.max(120));
        let maximum_height = (output.height - shell_layout::PANEL_HEIGHT).max(52);
        let height = requested_height.clamp(52, maximum_height);
        let x = x.clamp(output.x, output.x + output.width - width);
        let y = output.y + output.height - shell_layout::PANEL_HEIGHT - height - 4;
        Self::configure_window(
            &window,
            Geometry {
                x,
                y,
                width,
                height,
            },
        );
        // Passive previews must not deactivate the current application merely
        // because their shell surface became mapped. Explicit keyboard/menu
        // focus remains authoritative through the `focus` argument.
        if !self.map_buffered_window(window.clone(), (x, y), focus) {
            return;
        }
        if focus {
            self.surrender_internal_focus();
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                crate::session::focus::KeyboardFocusTarget::for_window(&window),
                SERIAL_COUNTER.next_serial(),
            );
        }
        self.space.elements().for_each(|element| {
            if let Some(toplevel) = element.toplevel() {
                toplevel.send_pending_configure();
            }
        });
        self.space.raise_element(&window, focus);
        self.raise_panels();
        eprintln!("nickel: {label} shown at {x},{y}");
    }

    pub fn hide_context_menu(&mut self) {
        if let Some(window) = self.context_menu_window.clone() {
            self.space.unmap_elem(&window);
        }
        self.preview_highlight = None;
        eprintln!("nickel: context menu hidden");
    }

    pub fn hide_overlays(&mut self) {
        if let Some(window) = self.context_menu_window.clone() {
            self.space.unmap_elem(&window);
        }
        if let Some(window) = self.preview_window.clone() {
            self.space.unmap_elem(&window);
        }
        self.preview_highlight = None;
        self.clear_overlay_preview_interest();
        eprintln!("nickel: transient overlays hidden");
    }

    pub(crate) fn set_shell_role_visible(&mut self, role: ShellRole, visible: bool) {
        if matches!(
            role,
            ShellRole::Desktop | ShellRole::Panel | ShellRole::Lock | ShellRole::Launcher
        ) {
            return;
        }
        let registry = self.windows.snapshot();
        if visible {
            self.hidden_shell_roles.remove(&role);
        } else {
            self.hidden_shell_roles.insert(role);
        }
        let window = self.shell_windows().find_map(|window| {
            let id = window
                .wl_surface()
                .and_then(|surface| self.surface_windows.get(&surface.id()))?;
            (registry
                .iter()
                .find(|entry| entry.id == *id)
                .and_then(|entry| ShellRole::from_application_id(&entry.app_id))
                == Some(role))
            .then(|| window.clone())
        });
        let Some(window) = window else {
            return;
        };
        if visible {
            if role == ShellRole::OnScreenKeyboard {
                self.place_on_screen_keyboard_surface(&window);
                return;
            }
            if self.space.elements().any(|mapped| mapped == &window) {
                return;
            }
            let location = self
                .hidden_shell_role_locations
                .remove(&role)
                .unwrap_or_default();
            self.map_buffered_window(window, location, false);
        } else {
            if let Some(location) = self.space.element_location(&window) {
                self.hidden_shell_role_locations.insert(role, location);
            }
            self.space.unmap_elem(&window);
        }
    }

    fn place_on_screen_keyboard_surface(&mut self, window: &Window) {
        let Some(output) = self
            .on_screen_keyboard
            .output_name
            .as_deref()
            .and_then(|name| self.output_geometry_named(name))
            .or_else(|| self.output_geometry_for_shell())
        else {
            return;
        };
        let target = shell_layout::keyboard_area(
            output,
            self.on_screen_keyboard.dock_top,
            self.on_screen_keyboard.height,
        );
        Self::configure_window(window, target);
        let location = Self::shell_surface_location(window, target);
        self.map_buffered_window(window.clone(), location, false);
    }

    fn show_anchored_shell_role(&mut self, role: ShellRole, anchor: ShellPopoverAnchor) {
        use nickel_session_protocol::AnchorSide;

        if !matches!(role, ShellRole::ControlCenter | ShellRole::ProjectMenu) {
            return;
        }
        let Some(output) = self.output_geometry_named(&anchor.output) else {
            tracing::warn!(?role, output = %anchor.output, "popover anchor output disappeared");
            self.set_shell_role_visible(role, false);
            return;
        };
        let registry = self.windows.snapshot();
        let Some(window) = self.shell_windows().find_map(|window| {
            let id = window
                .wl_surface()
                .and_then(|surface| self.surface_windows.get(&surface.id()))?;
            (registry
                .iter()
                .find(|entry| entry.id == *id)
                .and_then(|entry| ShellRole::from_application_id(&entry.app_id))
                == Some(role))
            .then(|| window.clone())
        }) else {
            return;
        };
        let bounds = anchor.bounds;
        let anchor_geometry = match anchor.preferred {
            AnchorSide::Above => Geometry {
                x: output.x + bounds.x,
                y: output.y + output.height - shell_layout::PANEL_HEIGHT + bounds.y,
                width: bounds.width,
                height: bounds.height,
            },
            AnchorSide::Below => Geometry {
                x: output.x + bounds.x,
                y: output.y + bounds.y,
                width: bounds.width,
                height: bounds.height,
            },
            AnchorSide::Left => Geometry {
                x: output.x + bounds.x,
                y: output.y + bounds.y,
                width: bounds.width,
                height: bounds.height,
            },
            AnchorSide::Right => Geometry {
                x: output.x + output.width - shell_layout::PANEL_HEIGHT + bounds.x,
                y: output.y + bounds.y,
                width: bounds.width,
                height: bounds.height,
            },
        };
        let area = match anchor.preferred {
            AnchorSide::Above => shell_layout::work_area(output),
            AnchorSide::Below => Geometry {
                y: output.y + shell_layout::PANEL_HEIGHT,
                height: (output.height - shell_layout::PANEL_HEIGHT).max(0),
                ..output
            },
            AnchorSide::Left | AnchorSide::Right => output,
        };
        let size = window.geometry().size;
        let target = shell_layout::anchored_popover(
            area,
            anchor_geometry,
            (size.w.max(1), size.h.max(1)),
            anchor.preferred,
        );
        Self::configure_window(&window, target);
        let location = Self::shell_surface_location(&window, target);
        self.hidden_shell_roles.remove(&role);
        self.hidden_shell_role_locations
            .insert(role, location.into());
        self.map_buffered_window(window.clone(), location, true);
        self.space.raise_element(&window, true);
        let control = anchor.control.chars().take(80).collect::<String>();
        tracing::debug!(
            ?role,
            %control,
            output = %anchor.output,
            anchor_x = anchor_geometry.x,
            anchor_y = anchor_geometry.y,
            anchor_width = anchor_geometry.width,
            anchor_height = anchor_geometry.height,
            preferred = ?anchor.preferred,
            final_x = target.x,
            final_y = target.y,
            final_width = target.width,
            final_height = target.height,
            "placed anchored shell popover"
        );
    }

    pub fn close_window(&mut self, id: WindowId) {
        if let Some(surface) = self.internal_surface_for_window(id) {
            if let Some(file) = self
                .internal_file_surfaces
                .iter()
                .find_map(|(file, runtime)| (*runtime == surface).then_some(*file))
                && let Some(shell) = self.internal_shell.as_mut()
            {
                let action = shell
                    .file_windows_mut()
                    .handle(nickel_file::FileWindowRequest::Close(file));
                self.apply_internal_file_action(action);
                self.hide_context_menu();
                self.schedule_internal_ui_frame();
                return;
            }
            if let Some(mut codex) = self.internal_codex.take() {
                if !codex.close(&mut self.internal_ui, surface) {
                    self.internal_ui.remove(surface);
                }
                self.internal_codex = Some(codex);
            } else {
                self.internal_ui.remove(surface);
            }
            self.unregister_internal_application(surface);
            self.hide_context_menu();
            self.schedule_internal_ui_frame();
            return;
        }
        let surface_id = self
            .surface_windows
            .iter()
            .find_map(|(surface, window)| (*window == id).then_some(surface.clone()));
        let Some(surface_id) = surface_id else {
            return;
        };
        if let Some((window, _)) = self.minimized_windows.remove(&id) {
            if let Some(surface) = window.toplevel() {
                surface.send_close();
            } else if let Some(surface) = window.x11_surface() {
                let _ = surface.close();
            }
            self.hide_context_menu();
            return;
        }
        if let Some((window, _)) = self.workspace_hidden_windows.remove(&id) {
            if let Some(surface) = window.toplevel() {
                surface.send_close();
            } else if let Some(surface) = window.x11_surface() {
                let _ = surface.close();
            }
            self.hide_context_menu();
            return;
        }
        if let Some(window) = self.space.elements().find(|window| {
            window
                .wl_surface()
                .is_some_and(|surface| surface.id() == surface_id)
        }) {
            if let Some(surface) = window.toplevel() {
                surface.send_close();
            } else if let Some(surface) = window.x11_surface() {
                let _ = surface.close();
            }
        }
        self.hide_context_menu();
    }

    pub fn activate_window(&mut self, id: WindowId) {
        self.cancel_remote_keyboard_before_focus(id);
        if let Some(surface) = self.internal_surface_for_window(id) {
            if let Some(workspace) = self.workspaces.workspace_for(&id)
                && workspace != self.workspaces.active()
                && let Ok(mut transition) = self.workspaces.switch_to(workspace, None)
            {
                transition.focus = Some(id);
                self.apply_workspace_transition(transition);
                return;
            }
            self.internal_minimized_windows.remove(&id);
            self.internal_ui.set_visible(surface, true);
            self.internal_ui.raise(surface);
            self.focus_internal_surface(surface);
            self.windows.raise(id);
            self.workspaces.focused(&id);
            if let Some(output) = self
                .internal_ui
                .placement(surface)
                .and_then(|placement| placement.output.clone())
            {
                self.last_interaction_output_name = Some(output);
            }
            self.space.elements().for_each(|window| {
                window.set_activated(false);
            });
            self.notify_protocol_snapshot();
            self.sync_internal_window_decorations();
            return;
        }
        if self
            .window_for_registry_id(id)
            .is_some_and(|window| self.is_on_screen_keyboard_window(&window))
        {
            return;
        }
        if let Some(workspace) = self.workspaces.workspace_for(&id)
            && workspace != self.workspaces.active()
            && let Ok(mut transition) = self.workspaces.switch_to(workspace, None)
        {
            transition.focus = Some(id);
            self.apply_workspace_transition(transition);
            return;
        }
        if let Some((window, location)) = self.minimized_windows.remove(&id) {
            if let Some(surface) = window.x11_surface() {
                let _ = surface.set_mapped(true);
            }
            self.map_buffered_window(window, location, true);
        }
        let Some(window) = self.window_for_registry_id(id) else {
            return;
        };
        self.last_interaction_output_name = self.output_name_for_window(&window);
        self.space.raise_element(&window, true);
        if let Some(surface) = window.x11_surface() {
            self.raise_x11_surface(surface);
        }
        self.windows.raise(id);
        self.workspaces.focused(&id);
        self.space.elements().for_each(|candidate| {
            candidate.set_activated(candidate == &window);
        });
        self.surrender_internal_focus();
        self.seat.get_keyboard().unwrap().set_focus(
            self,
            crate::session::focus::KeyboardFocusTarget::for_window(&window),
            SERIAL_COUNTER.next_serial(),
        );
        self.space.elements().for_each(|window| {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_pending_configure();
            }
        });
        self.raise_panels();
        self.notify_protocol_snapshot();
        self.sync_internal_window_decorations();
    }

    /// Transfer keyboard ownership to one compositor-hosted focusable surface.
    fn focus_internal_surface(&mut self, surface: nickel_ui::InternalSurfaceId) -> bool {
        if !self.internal_ui.is_visible(surface) {
            return false;
        }
        self.cancel_remote_keyboard();
        self.seat.get_keyboard().unwrap().set_focus(
            self,
            Option::<crate::session::focus::KeyboardFocusTarget>::None,
            SERIAL_COUNTER.next_serial(),
        );
        if !self.internal_ui.focus_surface(surface) {
            return false;
        }
        self.record_remote_internal_focus_event(surface);
        self.reconcile_keyboard_internal_recipient();
        self.wake_internal_shell();
        self.schedule_internal_ui_frame();
        true
    }

    /// Blur a compositor-hosted owner before assigning a native seat target.
    pub(crate) fn surrender_internal_focus(&mut self) {
        if self.internal_ui.clear_focus().is_some() {
            self.record_remote_focus_cleared();
            self.reconcile_keyboard_internal_recipient();
            self.wake_internal_shell();
            self.schedule_internal_ui_frame();
        }
    }

    pub fn cycle_windows(&mut self, forward: bool) {
        self.apply_task_switch_action(if forward {
            HotkeyAction::SwitchNext
        } else {
            HotkeyAction::SwitchPrevious
        });
    }

    pub fn commit_window_cycle(&mut self) {
        self.apply_task_switch_action(HotkeyAction::CommitSwitch);
    }

    pub fn apply_task_switch_action(&mut self, action: HotkeyAction) {
        let shell_ids = self
            .shell_windows()
            .filter_map(|window| {
                self.surface_windows
                    .get(&window.toplevel()?.wl_surface().id())
            })
            .copied()
            .collect::<HashSet<_>>();
        let windows = self
            .windows
            .snapshot()
            .into_iter()
            .rev()
            .filter(|window| !shell_ids.contains(&window.id))
            .filter(|window| self.workspaces.is_visible(&window.id))
            .map(|window| SwitchWindow {
                id: window.id,
                application_id: window.app_id.clone(),
                active: window.active,
            })
            .collect::<Vec<_>>();
        let effects = self.task_switcher.apply(action, &windows);
        self.apply_task_switch_effects(effects);
    }

    pub(crate) fn remove_window_from_switcher(&mut self, id: WindowId) {
        self.preview_switcher_interest
            .retain(|candidate| *candidate != id);
        self.preview_overlay_interest
            .retain(|candidate| *candidate != id);
        self.preview_admitted.remove(&id);
        self.drop_preview_frame(&id);
        self.reconcile_preview_admission();
        let effects = self.task_switcher.remove_candidate(&id);
        self.apply_task_switch_effects(effects);
    }

    pub(crate) fn restore_focus_after_window_removal(&mut self, restore: bool) {
        if !restore || self.locked {
            return;
        }
        let replacement = self
            .workspaces
            .ordered()
            .iter()
            .find(|workspace| workspace.id == self.workspaces.active())
            .and_then(|workspace| workspace.last_focused)
            .filter(|candidate| !self.minimized_windows.contains_key(candidate))
            .filter(|candidate| self.registry_window_is_mapped(*candidate));
        if let Some(replacement) = replacement {
            self.activate_window(replacement);
        } else {
            self.windows.deactivate_all();
            self.seat
                .get_keyboard()
                .unwrap()
                .set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
    }

    fn apply_task_switch_effects(&mut self, effects: Vec<TaskSwitchEffect<WindowId>>) {
        for effect in effects {
            match effect {
                TaskSwitchEffect::RequestPreviews(ids) => {
                    let ids = bounded_preview_ids(ids, self.task_switcher.selected_index());
                    self.set_switcher_preview_interest(ids);
                }
                TaskSwitchEffect::ActivateWindow(id) => self.activate_window(id),
                TaskSwitchEffect::ShowFlip { .. } => {}
                TaskSwitchEffect::SelectPreview(_) => {
                    let ids = bounded_preview_ids(
                        self.task_switcher.candidates().to_vec(),
                        self.task_switcher.selected_index(),
                    );
                    self.set_switcher_preview_interest(ids);
                }
                TaskSwitchEffect::HideFlip { .. } => self.clear_switcher_preview_interest(),
            }
        }
    }

    pub fn minimize_window(&mut self, id: WindowId) {
        if let Some(surface) = self.internal_surface_for_window(id) {
            self.internal_ui.set_visible(surface, false);
            self.internal_minimized_windows.insert(id);
            self.workspaces.unfocused(&id);
            self.windows.deactivate_all();
            self.sync_internal_window_decorations();
            self.schedule_internal_ui_frame();
            self.notify_protocol_snapshot();
            return;
        }
        let Some(window) = self
            .space
            .elements()
            .find(|window| {
                window
                    .wl_surface()
                    .and_then(|surface| self.surface_windows.get(&surface.id()))
                    .copied()
                    == Some(id)
            })
            .cloned()
        else {
            return;
        };
        let location = self.space.element_location(&window).unwrap_or_default();
        if let Some(surface) = window.x11_surface() {
            let _ = surface.set_mapped(false);
        }
        window.set_activated(false);
        self.space.unmap_elem(&window);
        self.minimized_windows.insert(id, (window, location));
        self.workspaces.unfocused(&id);
        let replacement = self
            .workspaces
            .ordered()
            .iter()
            .find(|workspace| workspace.id == self.workspaces.active())
            .and_then(|workspace| {
                workspace
                    .windows
                    .iter()
                    .rev()
                    .find(|candidate| {
                        **candidate != id && !self.minimized_windows.contains_key(candidate)
                    })
                    .copied()
            });
        if let Some(replacement) = replacement {
            self.activate_window(replacement);
        } else {
            self.windows.deactivate_all();
            self.seat
                .get_keyboard()
                .unwrap()
                .set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        self.raise_panels();
        self.notify_protocol_snapshot();
    }

    fn move_window_to_output(&mut self, id: WindowId, output_name: &str) -> bool {
        let Some(destination) = self.output_geometry_named(output_name) else {
            return false;
        };
        let destination = self.work_area_for_output(destination);
        if let Some((window, location)) = self.minimized_windows.get_mut(&id) {
            *location = clamp_window_location(
                (destination.x, destination.y).into(),
                window.geometry().size,
                destination,
            );
            self.notify_protocol_snapshot();
            return true;
        }
        if let Some((window, location)) = self.workspace_hidden_windows.get_mut(&id) {
            *location = clamp_window_location(
                (destination.x, destination.y).into(),
                window.geometry().size,
                destination,
            );
            self.notify_protocol_snapshot();
            return true;
        }
        let Some(window) = self.window_for_registry_id(id) else {
            return false;
        };
        let location = clamp_window_location(
            (destination.x, destination.y).into(),
            window.geometry().size,
            destination,
        );
        self.map_compositor_moved_window(window, location, false);
        self.notify_protocol_snapshot();
        true
    }

    pub fn maximize_window(&mut self, id: WindowId) {
        if let Some(surface) = self.internal_surface_for_window(id) {
            self.activate_window(id);
            let Some(current) = self.internal_ui.placement(surface).cloned() else {
                return;
            };
            let target = if let Some(restore) = self.internal_maximized_restore.remove(&id) {
                restore
            } else {
                let output = self
                    .internal_outputs()
                    .into_iter()
                    .find(|(output, _, _)| current.output.as_deref() == Some(output.name.as_str()))
                    .or_else(|| self.internal_outputs().into_iter().next());
                let Some((output, _, _)) = output else {
                    return;
                };
                let area = self.work_area_for_output(Geometry {
                    x: output.x,
                    y: output.y,
                    width: i32::try_from(output.width).unwrap_or(i32::MAX),
                    height: i32::try_from(output.height).unwrap_or(i32::MAX),
                });
                self.internal_maximized_restore.insert(id, current.clone());
                crate::session::InternalSurfacePlacement {
                    role: current.role,
                    geometry: (
                        area.x,
                        area.y + crate::session::window_frame::TITLEBAR_HEIGHT,
                        u32::try_from(area.width.max(1)).unwrap_or(u32::MAX),
                        u32::try_from(
                            (area.height - crate::session::window_frame::TITLEBAR_HEIGHT).max(1),
                        )
                        .unwrap_or(u32::MAX),
                    ),
                    output: Some(output.name),
                }
            };
            self.internal_ui.configure_application(surface, target);
            self.sync_internal_window_decorations();
            self.notify_protocol_snapshot();
            self.schedule_internal_ui_frame();
            return;
        }
        self.activate_window(id);
        let x11_window = self.window_for_registry_id(id);
        if let Some(window) = x11_window
            && let Some(surface) = window.x11_surface()
        {
            let maximize = !surface.is_maximized();
            let _ = surface.set_maximized(maximize);
            if maximize {
                self.apply_maximized_x11_geometry(&window, surface, true);
            } else if let Some(restore) = self.x11_maximized_restore.remove(&surface.window_id()) {
                let _ = surface.configure(restore);
                self.map_buffered_window(window, restore.loc, true);
            }
            return;
        }
        let surface = self.space.elements().find_map(|window| {
            let surface = window.toplevel()?;
            (self
                .surface_windows
                .get(&surface.wl_surface().id())
                .copied()
                == Some(id))
            .then(|| surface.clone())
        });
        if let Some(surface) = surface {
            self.toggle_maximized_toplevel(&surface);
        }
    }

    pub fn toggle_fullscreen_window(&mut self, id: WindowId) {
        self.activate_window(id);
        let window = self.window_for_registry_id(id);
        if let Some(surface) = window.as_ref().and_then(Window::x11_surface).cloned() {
            if self
                .x11_fullscreen_restore
                .contains_key(&surface.window_id())
            {
                self.unfullscreen_x11(&surface);
            } else {
                self.fullscreen_x11(&surface);
            }
            return;
        }
        let surface = window.as_ref().and_then(Window::toplevel).cloned();
        if let Some(surface) = surface {
            if self
                .fullscreen_restore
                .contains_key(&surface.wl_surface().id())
            {
                self.unfullscreen_toplevel(&surface);
            } else {
                self.fullscreen_toplevel(&surface);
            }
        }
    }

    fn shell_surface_output_name(&self, window: &Window) -> Option<String> {
        let surface = window.wl_surface()?;
        self.registered_shell_role_slots
            .iter()
            .find(|registration| registration.surface == surface.id())?
            .output
            .clone()
    }

    pub fn relayout_shell_surfaces(&mut self) {
        if self.on_screen_keyboard.visible
            && self
                .on_screen_keyboard
                .output_name
                .as_deref()
                .is_some_and(|name| self.output_geometry_named(name).is_none())
        {
            self.on_screen_keyboard.visible = false;
            self.set_shell_role_visible(ShellRole::OnScreenKeyboard, false);
        }
        if self.output_geometry().is_none() {
            return;
        }
        let outputs = self.space.outputs().cloned().collect::<Vec<_>>();
        let output_names = outputs.iter().map(Output::name).collect::<Vec<_>>();
        for desktop in self.desktop_windows.clone() {
            let Some(output_name) = self.shell_surface_output_name(&desktop) else {
                continue;
            };
            let Some(output_index) = output_index_for_shell_surface(&output_name, &output_names)
            else {
                continue;
            };
            let output = &outputs[output_index];
            let Some(geometry) = self.space.output_geometry(output) else {
                continue;
            };
            let geometry = Geometry {
                x: geometry.loc.x,
                y: geometry.loc.y,
                width: geometry.size.w,
                height: geometry.size.h,
            };
            Self::configure_window(&desktop, geometry);
            let location = Self::shell_surface_location(&desktop, geometry);
            self.map_buffered_window(desktop, location, false);
        }
        for panel in self.panel_windows.clone() {
            let Some(output_name) = self.shell_surface_output_name(&panel) else {
                continue;
            };
            let Some(output_index) = output_index_for_shell_surface(&output_name, &output_names)
            else {
                continue;
            };
            let output = &outputs[output_index];
            let Some(output) = self.space.output_geometry(output) else {
                continue;
            };
            let output = Geometry {
                x: output.loc.x,
                y: output.loc.y,
                width: output.size.w,
                height: output.size.h,
            };
            if self.keyboard_reserves_output(output) {
                self.space.unmap_elem(&panel);
                continue;
            }
            let geometry = shell_layout::panel(output);
            Self::configure_window(&panel, geometry);
            let location = Self::shell_surface_location(&panel, geometry);
            self.map_buffered_window(panel.clone(), location, false);
            self.space.raise_element(&panel, false);
        }
        if self.launcher_visibility.is_visible()
            && let Some(launcher) = self.launcher_window.clone()
        {
            let geometry = self.launcher_geometry(&launcher);
            let location = Self::shell_surface_location(&launcher, geometry);
            self.map_buffered_window(launcher, location, true);
            self.raise_panels();
        }
        let screenshot_utilities = {
            let registry = self.windows.snapshot();
            self.utility_windows
                .iter()
                .filter(|utility| self.space.elements().any(|mapped| mapped == *utility))
                .filter(|utility| {
                    utility
                        .wl_surface()
                        .and_then(|surface| self.surface_windows.get(&surface.id()))
                        .and_then(|id| registry.iter().find(|entry| entry.id == *id))
                        .and_then(|entry| ShellRole::from_application_id(&entry.app_id))
                        == Some(ShellRole::Screenshot)
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        for utility in screenshot_utilities {
            self.place_screenshot_surface(&utility);
        }
        let hidden = !self.on_screen_keyboard.visible || self.locked;
        let displaced = if hidden {
            std::mem::take(&mut self.on_screen_keyboard.displaced)
        } else {
            self.on_screen_keyboard.displaced.clone()
        };
        for (window, original) in displaced {
            if !window.alive()
                || self.space.element_location(&window).is_none()
                || self.is_fullscreen_window(&window)
                || self.is_maximized_window(&window)
            {
                continue;
            }
            let Some(output) = self.output_geometry_for_window(&window) else {
                continue;
            };
            let geometry = if hidden {
                original
            } else {
                if !self.keyboard_reserves_output(output) {
                    continue;
                }
                shell_layout::fit_keyboard_recipient(
                    original,
                    self.work_area_for_output(output),
                    self.is_server_decorated(&window),
                )
            };
            self.apply_keyboard_window_geometry(&window, geometry);
        }
        self.relayout_maximized_windows();
        self.relayout_fullscreen_windows();
        let windows = self.space.elements().cloned().collect::<Vec<_>>();
        for window in windows {
            self.fit_window_above_keyboard(&window);
        }
        let keyboard = self
            .utility_windows
            .iter()
            .find(|window| self.is_on_screen_keyboard_window(window))
            .cloned();
        if self.on_screen_keyboard.visible
            && !self.locked
            && let Some(window) = keyboard
        {
            self.place_on_screen_keyboard_surface(&window);
        }
        self.relayout_lock_surfaces();
    }

    pub fn maximize_toplevel(&mut self, surface: &ToplevelSurface) {
        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            surface.send_configure();
            return;
        };
        let Some(output) = self.output_geometry_for_window(&window) else {
            surface.send_configure();
            return;
        };
        let location = self.space.element_location(&window).unwrap_or_default();
        let size = window.geometry().size;
        self.maximized_restore
            .entry(surface.wl_surface().id())
            .or_insert(Geometry {
                x: location.x,
                y: location.y,
                width: size.w.max(1),
                height: size.h.max(1),
            });

        let work_area = self.work_area_for_output(output);
        let geometry = maximized_content_geometry(
            work_area,
            self.server_decorated.contains(&surface.wl_surface().id()),
        );
        surface.with_pending_state(|state| {
            state
                .states
                .set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
            state.size = Some(Size::from((geometry.width, geometry.height)));
        });
        self.space
            .map_element(window, (geometry.x, geometry.y), true);
        self.raise_panels();
        surface.send_pending_configure();
        self.notify_protocol_snapshot();
    }

    pub(crate) fn reconcile_maximized_toplevel_geometry(
        &mut self,
        surface: &ToplevelSurface,
    ) -> bool {
        if !self
            .maximized_restore
            .contains_key(&surface.wl_surface().id())
        {
            return false;
        }
        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            return false;
        };
        let Some(output) = self.output_geometry_for_window(&window) else {
            return false;
        };
        let geometry = maximized_content_geometry(
            self.work_area_for_output(output),
            self.server_decorated.contains(&surface.wl_surface().id()),
        );
        surface.with_pending_state(|state| {
            state.size = Some(Size::from((geometry.width, geometry.height)));
        });
        self.space
            .map_element(window, (geometry.x, geometry.y), true);
        self.raise_panels();
        self.notify_protocol_snapshot();
        true
    }

    pub fn unmaximize_toplevel(&mut self, surface: &ToplevelSurface) {
        let restore = self.maximized_restore.remove(&surface.wl_surface().id());
        let Some(restore) = restore else {
            return;
        };
        surface.with_pending_state(|state| {
            state
                .states
                .unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
            state.size = Some(Size::from((restore.width, restore.height)));
        });
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.map_buffered_window(window, (restore.x, restore.y), true);
        }
        self.raise_panels();
        surface.send_pending_configure();
        self.notify_protocol_snapshot();
    }

    pub fn fullscreen_toplevel(&mut self, surface: &ToplevelSurface) {
        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            surface.send_configure();
            return;
        };
        let Some(output) = self.output_geometry_for_window(&window) else {
            surface.send_configure();
            return;
        };
        let location = self.space.element_location(&window).unwrap_or_default();
        let size = window.geometry().size;
        self.fullscreen_restore
            .entry(surface.wl_surface().id())
            .or_insert(Geometry {
                x: location.x,
                y: location.y,
                width: size.w.max(1),
                height: size.h.max(1),
            });
        let output = if self.keyboard_reserves_output(output) {
            self.work_area_for_output(output)
        } else {
            output
        };
        surface.with_pending_state(|state| {
            state
                .states
                .set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen);
            state.size = Some(Size::from((output.width, output.height)));
        });
        window.override_z_index(45);
        self.map_buffered_window(window, (output.x, output.y), true);
        surface.send_pending_configure();
    }

    pub fn unfullscreen_toplevel(&mut self, surface: &ToplevelSurface) {
        let Some(restore) = self.fullscreen_restore.remove(&surface.wl_surface().id()) else {
            return;
        };
        surface.with_pending_state(|state| {
            state
                .states
                .unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen);
            state.size = Some(Size::from((restore.width, restore.height)));
        });
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            window.override_z_index(30);
            self.map_buffered_window(window, (restore.x, restore.y), true);
        }
        self.raise_panels();
        surface.send_pending_configure();
    }

    pub fn fullscreen_x11(&mut self, surface: &smithay::xwayland::X11Surface) {
        let Some(window) = self
            .space
            .elements()
            .find(|window| window.x11_surface() == Some(surface))
            .cloned()
        else {
            return;
        };
        let Some(output) = self.space.outputs_for_element(&window).first().cloned() else {
            return;
        };
        let Some(geometry) = self.space.output_geometry(&output) else {
            return;
        };
        self.x11_fullscreen_restore
            .entry(surface.window_id())
            .or_insert_with(|| surface.geometry());
        let bounds = Geometry {
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w,
            height: geometry.size.h,
        };
        let bounds = if self.keyboard_reserves_output(bounds) {
            self.work_area_for_output(bounds)
        } else {
            bounds
        };
        let geometry = smithay::utils::Rectangle::new(
            (bounds.x, bounds.y).into(),
            (bounds.width, bounds.height).into(),
        );
        let _ = surface.set_fullscreen(true);
        let _ = surface.configure(geometry);
        window.override_z_index(45);
        self.map_buffered_window(window, geometry.loc, true);
        self.request_output_redraw();
        self.notify_protocol_snapshot();
    }

    pub fn unfullscreen_x11(&mut self, surface: &smithay::xwayland::X11Surface) {
        let Some(restore) = self.x11_fullscreen_restore.remove(&surface.window_id()) else {
            return;
        };
        let _ = surface.set_fullscreen(false);
        let _ = surface.configure(restore);
        let window = {
            self.space
                .elements()
                .find(|window| window.x11_surface() == Some(surface))
                .cloned()
        };
        if let Some(window) = window {
            window.override_z_index(30);
            self.map_buffered_window(window, restore.loc, true);
        }
        self.raise_panels();
        self.request_output_redraw();
        self.notify_protocol_snapshot();
    }

    pub fn forget_x11_geometry(&mut self, surface: &smithay::xwayland::X11Surface) {
        self.x11_maximized_restore.remove(&surface.window_id());
        self.x11_fullscreen_restore.remove(&surface.window_id());
    }

    pub(crate) fn forget_all_x11_geometry(&mut self) {
        self.x11_maximized_restore.clear();
        self.x11_maximized_restore.shrink_to_fit();
        self.x11_fullscreen_restore.clear();
        self.x11_fullscreen_restore.shrink_to_fit();
    }

    pub fn toggle_maximized_toplevel(&mut self, surface: &ToplevelSurface) {
        if self
            .maximized_restore
            .contains_key(&surface.wl_surface().id())
        {
            self.unmaximize_toplevel(surface);
        } else {
            self.maximize_toplevel(surface);
        }
    }

    pub fn forget_toplevel_geometry(&mut self, surface: &ToplevelSurface) {
        self.maximized_restore.remove(&surface.wl_surface().id());
        self.fullscreen_restore.remove(&surface.wl_surface().id());
    }

    pub(crate) fn relayout_maximized_windows(&mut self) {
        let maximized: Vec<_> = self
            .space
            .elements()
            .filter_map(|window| {
                let surface = window.toplevel()?.wl_surface();
                self.maximized_restore
                    .contains_key(&surface.id())
                    .then_some((window.clone(), window.toplevel()?.clone()))
            })
            .collect();
        for (window, surface) in maximized {
            let Some(output) = self.output_geometry_for_window(&window) else {
                continue;
            };
            let work_area = self.work_area_for_output(output);
            let geometry = maximized_content_geometry(
                work_area,
                self.server_decorated.contains(&surface.wl_surface().id()),
            );
            Self::configure_window(&window, geometry);
            self.space
                .map_element(window, (geometry.x, geometry.y), true);
            surface.send_pending_configure();
        }
        let maximized_x11 = self
            .space
            .elements()
            .filter_map(|window| {
                let surface = window.x11_surface()?;
                self.x11_maximized_restore
                    .contains_key(&surface.window_id())
                    .then_some((window.clone(), surface.clone()))
            })
            .collect::<Vec<_>>();
        for (window, surface) in maximized_x11 {
            self.apply_maximized_x11_geometry(&window, &surface, false);
        }
        self.raise_panels();
    }

    pub(crate) fn apply_maximized_x11_geometry(
        &mut self,
        window: &Window,
        surface: &smithay::xwayland::X11Surface,
        preserve_restore: bool,
    ) {
        let Some(output) = self.output_geometry_for_window(window) else {
            return;
        };
        if preserve_restore {
            let location = self.space.element_location(window).unwrap_or_default();
            let size = window.geometry().size;
            self.x11_maximized_restore
                .entry(surface.window_id())
                .or_insert_with(|| {
                    smithay::utils::Rectangle::new(location, (size.w.max(1), size.h.max(1)).into())
                });
        }
        let geometry = maximized_content_geometry(
            self.work_area_for_output(output),
            self.is_server_decorated(window),
        );
        let geometry = smithay::utils::Rectangle::new(
            (geometry.x, geometry.y).into(),
            (geometry.width, geometry.height).into(),
        );
        let _ = surface.configure(geometry);
        self.map_buffered_window(window.clone(), geometry.loc, true);
    }

    pub(crate) fn restore_maximized_window_for_drag(
        &mut self,
        window: &Window,
        pointer: Point<f64, Logical>,
    ) -> Option<Point<i32, Logical>> {
        let current_location = self.space.element_location(window)?;
        let current_size = window.geometry().size;
        let current = Geometry {
            x: current_location.x,
            y: current_location.y,
            width: current_size.w.max(1),
            height: current_size.h.max(1),
        };
        let output = self.space.outputs().find_map(|output| {
            self.space
                .output_geometry(output)
                .filter(|geometry| geometry.to_f64().contains(pointer))
                .map(|geometry| Geometry {
                    x: geometry.loc.x,
                    y: geometry.loc.y,
                    width: geometry.size.w,
                    height: geometry.size.h,
                })
        })?;
        let decorated = self.is_server_decorated(window);

        if let Some(surface) = window.x11_surface() {
            let restore = self.x11_maximized_restore.remove(&surface.window_id())?;
            let restore = Geometry {
                x: restore.loc.x,
                y: restore.loc.y,
                width: restore.size.w,
                height: restore.size.h,
            };
            let geometry = restored_drag_content_geometry(
                current,
                restore,
                pointer,
                decorated,
                self.work_area_for_output(output),
            );
            let rectangle = Rectangle::new(
                (geometry.x, geometry.y).into(),
                (geometry.width, geometry.height).into(),
            );
            let _ = surface.set_maximized(false);
            let _ = surface.configure(rectangle);
            self.map_buffered_window(window.clone(), rectangle.loc, true);
            self.notify_protocol_snapshot();
            return Some(rectangle.loc);
        }

        let surface = window.toplevel()?.clone();
        let restore = self.maximized_restore.remove(&surface.wl_surface().id())?;
        let geometry = restored_drag_content_geometry(
            current,
            restore,
            pointer,
            decorated,
            self.work_area_for_output(output),
        );
        tracing::info!(
            surface = ?surface.wl_surface().id(),
            current = ?current,
            restore = ?restore,
            decorated,
            pointer = ?pointer,
            result = ?geometry,
            "diagnostic: restoring maximized Wayland window for drag"
        );
        surface.with_pending_state(|state| {
            state
                .states
                .unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
            state.size = Some((geometry.width, geometry.height).into());
        });
        self.space
            .map_element(window.clone(), (geometry.x, geometry.y), true);
        surface.send_pending_configure();
        self.notify_protocol_snapshot();
        Some((geometry.x, geometry.y).into())
    }

    pub(crate) fn relayout_fullscreen_windows(&mut self) {
        let fullscreen = self
            .space
            .elements()
            .filter(|window| self.is_fullscreen_window(window))
            .cloned()
            .collect::<Vec<_>>();
        for window in fullscreen {
            let Some(output) = self.output_geometry_for_window(&window) else {
                continue;
            };
            let output = if self.keyboard_reserves_output(output) {
                self.work_area_for_output(output)
            } else {
                output
            };
            if let Some(surface) = window.toplevel() {
                Self::configure_window(&window, output);
                window.override_z_index(45);
                self.space
                    .map_element(window.clone(), (output.x, output.y), true);
                surface.send_pending_configure();
            } else if let Some(surface) = window.x11_surface() {
                let geometry = smithay::utils::Rectangle::new(
                    (output.x, output.y).into(),
                    (output.width, output.height).into(),
                );
                let _ = surface.configure(geometry);
                window.override_z_index(45);
                self.space
                    .map_element(window.clone(), (output.x, output.y), true);
            }
        }
        self.raise_panels();
        self.request_output_redraw();
    }

    fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|window| window.wl_surface().as_deref() == Some(surface))
            .cloned()
    }

    pub(crate) fn window_for_registry_id(&self, id: WindowId) -> Option<Window> {
        self.space
            .elements()
            .find(|window| {
                window
                    .wl_surface()
                    .and_then(|surface| self.surface_windows.get(&surface.id()))
                    .copied()
                    == Some(id)
                    || window
                        .x11_surface()
                        .and_then(|surface| self.x11_windows.get(&surface.window_id()))
                        .copied()
                        == Some(id)
            })
            .cloned()
    }

    fn registry_native_window(&self, id: WindowId) -> Option<Window> {
        self.window_for_registry_id(id).or_else(|| {
            self.minimized_windows
                .get(&id)
                .or_else(|| self.workspace_hidden_windows.get(&id))
                .map(|(window, _)| window.clone())
        })
    }

    pub(crate) fn raise_panels(&mut self) {
        for panel in self.panel_windows.clone() {
            self.space.raise_element(&panel, false);
        }
    }

    fn output_geometry(&self) -> Option<Geometry> {
        let output = self
            .primary_output_name
            .as_ref()
            .and_then(|name| self.space.outputs().find(|output| output.name() == *name))
            .or_else(|| self.space.outputs().next())?;
        let geometry = self.space.output_geometry(output)?;
        Some(Geometry {
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w,
            height: geometry.size.h,
        })
    }

    /// Reconcile every mapped top-level's effective output after Space has sent
    /// the complete enter/leave set. Geometry remains logical; only buffer
    /// preferences change while the client prepares its next commit.
    pub(crate) fn refresh_surface_scales(&mut self) {
        const HYSTERESIS_AREA: u64 = 4_096;
        let outputs = self
            .space
            .outputs()
            .filter_map(|output| {
                let geometry = self.space.output_geometry(output)?;
                Some((
                    nickel_core::dpi::OutputScale {
                        identity: output.name(),
                        geometry: nickel_core::dpi::LogicalRect {
                            x: geometry.loc.x,
                            y: geometry.loc.y,
                            width: geometry.size.w,
                            height: geometry.size.h,
                        },
                        scale: nickel_core::dpi::Scale120::new(
                            (output.current_scale().fractional_scale() * 120.0).round() as u32,
                        )
                        .unwrap_or_default(),
                    },
                    output.clone(),
                ))
            })
            .collect::<Vec<_>>();
        let candidates = outputs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect::<Vec<_>>();
        let active = self.last_interaction_output_name.as_deref();
        let windows = self.space.elements().cloned().collect::<Vec<_>>();
        for window in windows {
            let Some(surface) = window.wl_surface() else {
                continue;
            };
            let Some(bounds) = self.space.element_bbox(&window) else {
                continue;
            };
            let previous = self
                .surface_effective_outputs
                .get(&surface.id())
                .map(String::as_str);
            let selection = nickel_core::dpi::select_effective_output(
                nickel_core::dpi::LogicalRect {
                    x: bounds.loc.x,
                    y: bounds.loc.y,
                    width: bounds.size.w,
                    height: bounds.size.h,
                },
                &candidates,
                previous,
                active,
                HYSTERESIS_AREA,
            );
            let Some(identity) = selection.output else {
                self.surface_effective_outputs.remove(&surface.id());
                continue;
            };
            let Some((candidate, output)) = outputs
                .iter()
                .find(|(candidate, _)| candidate.identity == identity)
            else {
                continue;
            };
            self.surface_effective_outputs
                .insert(surface.id(), identity);
            let integer_scale = candidate.scale.integer_buffer_scale();
            let fractional_scale = candidate.scale.factor();
            let transform = output.current_transform();
            window.with_surfaces(|surface, states| {
                send_surface_state(surface, states, integer_scale, transform);
                with_fractional_scale(states, |state| state.set_preferred_scale(fractional_scale));
            });
            // Popups are separate surface trees rather than descendants visited by
            // `Window::with_surfaces`. Keep their buffer preference locked to the
            // owning top-level while that window crosses an output boundary.
            for (popup, _) in PopupManager::popups_for_surface(&surface) {
                with_states(popup.wl_surface(), |states| {
                    send_surface_state(popup.wl_surface(), states, integer_scale, transform);
                    with_fractional_scale(states, |state| {
                        state.set_preferred_scale(fractional_scale)
                    });
                });
            }
        }

        // A data-device icon has no xdg parent from which to derive an output.
        // The interaction output is the source-side scale and remains stable for
        // the lifetime of the drag even when the pointer straddles two outputs.
        if let Some(icon) = self.dnd_icon.as_ref() {
            let output = active
                .and_then(|name| self.space.outputs().find(|output| output.name() == name))
                .or_else(|| self.space.outputs().next());
            if let Some(output) = output {
                let fractional = output.current_scale().fractional_scale();
                let integer = fractional.ceil().max(1.0) as i32;
                with_states(icon, |states| {
                    send_surface_state(icon, states, integer, output.current_transform());
                    with_fractional_scale(states, |state| state.set_preferred_scale(fractional));
                });
            }
        }
    }

    pub(crate) fn refresh_new_surface_scale(&mut self, surface: &WlSurface) {
        let mut root = self
            .popups
            .find_popup(surface)
            .and_then(|popup| find_popup_root_surface(&popup).ok())
            .unwrap_or_else(|| surface.clone());
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        let identity = self.surface_effective_outputs.get(&root.id()).cloned();
        let output = identity
            .as_deref()
            .and_then(|identity| {
                self.space
                    .outputs()
                    .find(|output| output.name() == identity)
            })
            .cloned()
            .or_else(|| self.space.outputs().next().cloned());
        let Some(output) = output else { return };
        let fractional = output.current_scale().fractional_scale();
        let integer = fractional.ceil().max(1.0) as i32;
        with_states(surface, |states| {
            send_surface_state(surface, states, integer, output.current_transform());
            with_fractional_scale(states, |state| state.set_preferred_scale(fractional));
        });
    }

    pub(crate) fn output_geometry_named(&self, name: &str) -> Option<Geometry> {
        let output = self.space.outputs().find(|output| output.name() == name)?;
        let geometry = self.space.output_geometry(output)?;
        Some(Geometry {
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w,
            height: geometry.size.h,
        })
    }

    pub(crate) fn preferred_interaction_output_name(&self) -> Option<String> {
        self.resolve_interaction_output(InvocationSource::Pointer)
    }

    pub(crate) fn keyboard_interaction_output_name(&self) -> Option<String> {
        self.resolve_interaction_output(InvocationSource::Keyboard)
    }

    fn focused_surface_output_name(&self) -> Option<String> {
        let id = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active)?
            .id;
        self.output_name_for_window(&self.window_for_registry_id(id)?)
    }

    pub(crate) fn output_name_for_window(&self, window: &Window) -> Option<String> {
        let geometry = self.output_geometry_for_window(window)?;
        self.output_name_matching_geometry(geometry)
    }

    fn output_name_matching_geometry(&self, geometry: Geometry) -> Option<String> {
        self.space.outputs().find_map(|output| {
            let candidate = self.space.output_geometry(output)?;
            (candidate.loc.x == geometry.x
                && candidate.loc.y == geometry.y
                && candidate.size.w == geometry.width
                && candidate.size.h == geometry.height)
                .then(|| output.name())
        })
    }

    fn resolve_interaction_output(&self, source: InvocationSource) -> Option<String> {
        let enabled = self
            .space
            .outputs()
            .map(|output| output.name())
            .collect::<Vec<_>>();
        let enabled_refs = enabled.iter().map(String::as_str).collect::<Vec<_>>();
        let pointer = self.output_name_at_pointer();
        let focused = self.focused_surface_output_name();
        resolve_active_output(
            source,
            ActiveOutputContext {
                pointer: pointer.as_deref(),
                focused_surface: focused.as_deref(),
                recent_interaction: self
                    .last_interaction_output_name
                    .as_deref()
                    .or_else(|| self.workspaces.active_output()),
                primary: self.primary_output_name.as_deref(),
                enabled: &enabled_refs,
            },
        )
    }

    pub(crate) fn new_window_active_output_name(&self) -> Option<String> {
        let focused_surface = self
            .windows
            .snapshot()
            .into_iter()
            .find(|window| window.active && !self.shell_owned_windows.contains(&window.id))
            .and_then(|window| self.window_for_registry_id(window.id))
            .and_then(|window| self.output_name_for_window(&window));
        let enabled = self
            .space
            .outputs()
            .map(|output| output.name())
            .collect::<Vec<_>>();
        let enabled_refs = enabled.iter().map(String::as_str).collect::<Vec<_>>();
        let pointer = self.output_name_at_pointer();
        resolve_new_window_output(ActiveOutputContext {
            pointer: pointer.as_deref(),
            focused_surface: focused_surface.as_deref(),
            recent_interaction: self
                .last_interaction_output_name
                .as_deref()
                .or_else(|| self.workspaces.active_output()),
            primary: self.primary_output_name.as_deref(),
            enabled: &enabled_refs,
        })
    }

    pub(crate) fn placement_outputs(&self) -> Vec<shell_layout::PlacementOutput> {
        let primary = self.primary_output_name.as_deref();
        self.space
            .outputs()
            .filter_map(|output| {
                let geometry = self.space.output_geometry(output)?;
                let geometry = Geometry {
                    x: geometry.loc.x,
                    y: geometry.loc.y,
                    width: geometry.size.w,
                    height: geometry.size.h,
                };
                Some(shell_layout::PlacementOutput {
                    name: output.name(),
                    work_area: self.work_area_for_output(geometry),
                    primary: primary == Some(output.name().as_str()),
                })
            })
            .collect()
    }

    fn output_geometry_for_window(&self, window: &Window) -> Option<Geometry> {
        let bounds = self.space.element_bbox(window)?;
        let window_geometry = Geometry {
            x: bounds.loc.x,
            y: bounds.loc.y,
            width: bounds.size.w,
            height: bounds.size.h,
        };
        self.output_geometry_for_bounds(window_geometry)
    }

    fn output_geometry_for_bounds(&self, window_geometry: Geometry) -> Option<Geometry> {
        let outputs: Vec<_> = self
            .space
            .outputs()
            .filter_map(|output| self.space.output_geometry(output))
            .map(|geometry| Geometry {
                x: geometry.loc.x,
                y: geometry.loc.y,
                width: geometry.size.w,
                height: geometry.size.h,
            })
            .collect();
        shell_layout::output_for_window(window_geometry, &outputs)
    }

    fn keyboard_reserves_output(&self, output: Geometry) -> bool {
        self.on_screen_keyboard.visible
            && !self.locked
            && self
                .on_screen_keyboard
                .output_name
                .as_deref()
                .and_then(|name| self.output_geometry_named(name))
                == Some(output)
    }

    fn panel_hidden_by_keyboard(&self, window: &Window) -> bool {
        self.panel_windows.contains(window)
            && self
                .shell_surface_output_name(window)
                .as_deref()
                .and_then(|name| self.output_geometry_named(name))
                .is_some_and(|output| self.keyboard_reserves_output(output))
    }

    fn work_area_for_output(&self, output: Geometry) -> Geometry {
        if self.keyboard_reserves_output(output) {
            shell_layout::keyboard_work_area(
                output,
                self.on_screen_keyboard.dock_top,
                self.on_screen_keyboard.height,
            )
        } else {
            shell_layout::work_area(output)
        }
    }

    fn apply_keyboard_window_geometry(&mut self, window: &Window, geometry: Geometry) {
        Self::configure_window(window, geometry);
        if let Some(surface) = window.x11_surface() {
            let _ = surface.configure(smithay::utils::Rectangle::new(
                (geometry.x, geometry.y).into(),
                (geometry.width, geometry.height).into(),
            ));
        }
        self.space
            .map_element(window.clone(), (geometry.x, geometry.y), false);
    }

    pub(crate) fn fit_window_above_keyboard(&mut self, window: &Window) {
        if self.is_shell_owned_window(window)
            || self.is_fullscreen_window(window)
            || self.is_maximized_window(window)
        {
            return;
        }
        let Some(output) = self.output_geometry_for_window(window) else {
            return;
        };
        if !self.keyboard_reserves_output(output) {
            return;
        }
        let Some(location) = self.space.element_location(window) else {
            return;
        };
        let size = window.geometry().size;
        let content = Geometry {
            x: location.x,
            y: location.y,
            width: size.w,
            height: size.h,
        };
        let target = shell_layout::fit_keyboard_recipient(
            content,
            self.work_area_for_output(output),
            self.is_server_decorated(window),
        );
        if target != content {
            if !self
                .on_screen_keyboard
                .displaced
                .iter()
                .any(|(saved, _)| saved == window)
            {
                self.on_screen_keyboard
                    .displaced
                    .push((window.clone(), content));
            }
            self.apply_keyboard_window_geometry(window, target);
        }
    }

    pub(crate) fn output_geometry_for_shell(&self) -> Option<Geometry> {
        self.output_geometry()
    }

    fn launcher_geometry(&self, launcher: &Window) -> Geometry {
        let requested = launcher.geometry().size;
        let width = if requested.w > 1 {
            requested.w.min(920)
        } else {
            920
        };
        let height = if requested.h > 1 {
            requested.h.min(680)
        } else {
            680
        };
        let output = self
            .launcher_output_name
            .as_deref()
            .and_then(|name| self.output_geometry_named(name))
            .or_else(|| self.output_geometry())
            .unwrap_or(Geometry {
                x: 0,
                y: 0,
                width,
                height: height + shell_layout::PANEL_HEIGHT + 8,
            });
        let work_area = shell_layout::work_area(output);
        shell_layout::bottom_left_in(work_area, (width, height), 18, 8)
    }

    fn configure_window(window: &Window, geometry: Geometry) {
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|state| {
                state.size = Some(Size::from((geometry.width, geometry.height)));
            });
            toplevel.send_pending_configure();
        }
    }

    fn shell_surface_location(window: &Window, target: Geometry) -> (i32, i32) {
        let surface = window.geometry();
        shell_layout::space_location_for_bounds(
            target,
            Geometry {
                x: surface.loc.x,
                y: surface.loc.y,
                width: surface.size.w,
                height: surface.size.h,
            },
        )
    }

    fn init_wayland_listener(
        display: Display<NickelSession>,
        event_loop: &mut EventLoop<'static, NickelSession>,
    ) -> OsString {
        // Creates a new listening socket, automatically choosing the next available `wayland` socket name.
        let listening_socket = ListeningSocketSource::new_auto().unwrap();

        // Get the name of the listening socket.
        // Clients will connect to this socket.
        let socket_name = listening_socket.socket_name().to_os_string();

        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                // Inside the callback, you should insert the client into the display.
                //
                // You may also associate some data with the client when inserting the client.
                let portal_capture_allowed = nix::sys::socket::getsockopt(
                    &client_stream,
                    nix::sys::socket::sockopt::PeerCredentials,
                )
                .ok()
                .is_some_and(|credentials| {
                    crate::session::handlers::portal_capture_pid_allowed(credentials.pid())
                });
                state
                    .display_handle
                    .insert_client(
                        client_stream,
                        Arc::new(ClientState {
                            compositor_state: CompositorClientState::default(),
                            portal_capture_allowed,
                        }),
                    )
                    .unwrap();
            })
            .expect("Failed to init the wayland event source.");

        // You also need to add the display itself to the event loop, so that client events will be processed by wayland-server.
        // The Rust Wayland backend exposes an epoll descriptor. Polling that
        // epoll descriptor from calloop's epoll can lose readiness on some
        // nested-compositor stacks, so a tiny blocking poller converts it into
        // an ordinary calloop channel. Dispatch and all state mutation remain
        // on the compositor event-loop thread.
        let poll_fd = smithay::reexports::rustix::io::dup(display.as_fd())
            .expect("failed to duplicate Wayland backend poll fd");
        let (ready_tx, ready_rx) = channel::channel();
        let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel(0);
        std::thread::Builder::new()
            .name("nickel-wayland-poll".into())
            .spawn(move || {
                use smithay::reexports::rustix::event::{PollFd, PollFlags, poll};
                let mut descriptors = [PollFd::new(&poll_fd, PollFlags::IN)];
                loop {
                    if poll(&mut descriptors, None).is_err() || ready_tx.send(()).is_err() {
                        break;
                    }
                    if ack_rx.recv().is_err() {
                        break;
                    }
                    descriptors[0].clear_revents();
                }
            })
            .expect("failed to start Wayland backend poller");
        let mut display = display;
        loop_handle
            .insert_source(ready_rx, move |event, _, state| {
                if let channel::Event::Msg(()) = event {
                    if let Err(error) = display.dispatch_clients(state) {
                        tracing::warn!(%error, "Wayland client dispatch failed");
                    }
                    state.revalidate_remote_pointer();
                    state.revalidate_remote_keyboard();
                    if let Err(error) = display.flush_clients() {
                        tracing::debug!(%error, "Wayland client flush deferred");
                    }
                    let _ = ack_tx.send(());
                }
            })
            .expect("failed to register Wayland backend dispatch channel");

        socket_name
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.space
            .element_under(pos)
            .filter(|(window, _)| !self.locked || self.lock_windows.contains(window))
            .and_then(|(window, location)| {
                window
                    .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + location).to_f64()))
            })
    }

    /// Whether the ordinary client scene occupies `pos`, including the
    /// compositor-owned portion of a server-decorated window.
    ///
    /// Server-side titlebars live outside their client's input surface.  They
    /// must nevertheless mask the internal desktop, which is rendered and hit
    /// tested below ordinary windows.  Foreground internal surfaces still get
    /// their usual priority; this value is only used to make the desktop yield.
    pub(crate) fn client_scene_under(&self, pos: Point<f64, Logical>) -> bool {
        let frame = self.space.elements().rev().find_map(|window| {
            if (self.locked && !self.lock_windows.contains(window))
                || self.shell_windows().any(|shell| shell == window)
                || self.is_fullscreen_window(window)
                || !self.is_server_decorated(window)
            {
                return None;
            }
            let bounds = self.space.element_geometry(window)?;
            crate::session::window_frame::hit_test(
                crate::session::shell_layout::Geometry {
                    x: bounds.loc.x,
                    y: bounds.loc.y,
                    width: bounds.size.w,
                    height: bounds.size.h,
                },
                pos.x.round() as i32,
                pos.y.round() as i32,
            )
        });
        crate::session::window_frame::client_scene_occupies(
            self.surface_under(pos).is_some(),
            frame,
        )
    }

    pub fn pointer_surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(
        crate::session::focus::PointerFocusTarget,
        Point<f64, Logical>,
    )> {
        self.space
            .element_under(pos)
            .filter(|(window, _)| !self.locked || self.lock_windows.contains(window))
            .and_then(|(window, location)| {
                window
                    .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(surface, origin)| {
                        let target = window.x11_surface().map_or_else(
                            || crate::session::focus::PointerFocusTarget::Wayland(surface),
                            |x11| crate::session::focus::PointerFocusTarget::X11(x11.clone()),
                        );
                        (target, (origin + location).to_f64())
                    })
            })
    }
}

fn internal_keyboard_surface_placement(
    output_name: Option<&str>,
    dock_top: bool,
    height: u32,
    outputs: &[(crate::internal_shell::InternalOutput, i32, i32)],
) -> Option<crate::session::InternalSurfacePlacement> {
    let (output, x, y) = match output_name {
        Some(name) => outputs.iter().find(|(output, _, _)| output.name == name),
        None => outputs.first(),
    }?;
    // Share the reservation geometry with external keyboard surfaces. These are
    // compositor logical coordinates; fractional output scale is applied later.
    let geometry = shell_layout::keyboard_area(
        shell_layout::Geometry {
            x: *x,
            y: *y,
            width: i32::try_from(output.width).unwrap_or(i32::MAX),
            height: i32::try_from(output.height).unwrap_or(i32::MAX),
        },
        dock_top,
        height,
    );
    let size = (geometry.width.max(0) as u32, geometry.height.max(0) as u32);
    let mut placement = internal_shell_surface_placement(
        crate::winit_shell::SurfaceRole::OnScreenKeyboard,
        Some(&output.name),
        size,
        outputs,
        None,
    );
    placement.geometry = (geometry.x, geometry.y, size.0, size.1);
    Some(placement)
}

fn internal_shell_surface_placement(
    surface_role: crate::winit_shell::SurfaceRole,
    surface_output: Option<&str>,
    surface_size: (u32, u32),
    outputs: &[(crate::internal_shell::InternalOutput, i32, i32)],
    launcher_output: Option<&str>,
) -> crate::session::InternalSurfacePlacement {
    use crate::{session::InternalSurfaceRole, winit_shell::SurfaceRole};

    let requested_output = if surface_role == SurfaceRole::Launcher {
        launcher_output
    } else {
        surface_output
    };
    let selected = requested_output
        .and_then(|name| outputs.iter().find(|(output, _, _)| output.name == name))
        .or_else(|| outputs.first());
    let (output_name, origin_x, origin_y, output_width, output_height) = selected
        .map(|(output, x, y)| {
            (
                Some(output.name.clone()),
                *x,
                *y,
                output.width,
                output.height,
            )
        })
        .unwrap_or((None, 0, 0, surface_size.0, surface_size.1));

    let surface_size = match surface_role {
        SurfaceRole::Launcher => crate::internal_shell::launcher_size(output_width, output_height),
        SurfaceRole::ControlCenter => {
            crate::internal_shell::control_center_size(output_width, output_height)
        }
        _ => surface_size,
    };
    let (x, y) = match surface_role {
        SurfaceRole::Panel => (
            origin_x,
            origin_y + output_height.saturating_sub(crate::winit_shell::PANEL_HEIGHT) as i32,
        ),
        SurfaceRole::Launcher => {
            let work_height = output_height.saturating_sub(crate::winit_shell::PANEL_HEIGHT);
            let x_margin = 18.min(output_width.saturating_sub(surface_size.0)) as i32;
            let y_margin = 8.min(work_height.saturating_sub(surface_size.1)) as i32;
            (
                origin_x + x_margin,
                origin_y + work_height.saturating_sub(surface_size.1) as i32 - y_margin,
            )
        }
        _ => (origin_x, origin_y),
    };
    let role = match surface_role {
        SurfaceRole::Desktop => InternalSurfaceRole::Desktop,
        SurfaceRole::Panel => InternalSurfaceRole::Panel,
        SurfaceRole::OnScreenKeyboard => InternalSurfaceRole::OnScreenKeyboard,
        _ => InternalSurfaceRole::Overlay,
    };
    crate::session::InternalSurfacePlacement {
        role,
        geometry: (x, y, surface_size.0, surface_size.1),
        output: output_name,
    }
}

fn avoid_trusted_control_collision(
    mut menu: (i32, i32, u32, u32),
    output: (i32, i32, u32, u32),
    trusted_controls: &[(i32, i32, u32, u32)],
) -> (i32, i32, u32, u32) {
    const GAP: i32 = 8;
    let (output_x, output_y, output_width, output_height) = output;
    let output_right = i64::from(output_x) + i64::from(output_width);
    let output_bottom = i64::from(output_y) + i64::from(output_height);
    for &(x, y, width, height) in trusted_controls {
        let overlaps = i64::from(menu.0) < i64::from(x) + i64::from(width)
            && i64::from(x) < i64::from(menu.0) + i64::from(menu.2)
            && i64::from(menu.1) < i64::from(y) + i64::from(height)
            && i64::from(y) < i64::from(menu.1) + i64::from(menu.3);
        if !overlaps {
            continue;
        }
        let menu_width = i32::try_from(menu.2).unwrap_or(i32::MAX);
        let menu_height = i32::try_from(menu.3).unwrap_or(i32::MAX);
        let trusted_width = i32::try_from(width).unwrap_or(i32::MAX);
        let trusted_height = i32::try_from(height).unwrap_or(i32::MAX);
        let left = x.saturating_sub(GAP).saturating_sub(menu_width);
        if left >= output_x {
            menu.0 = left;
            continue;
        }
        let below = y.saturating_add(trusted_height).saturating_add(GAP);
        if i64::from(below) + i64::from(menu_height) <= output_bottom {
            menu.1 = below;
            continue;
        }
        let right = x.saturating_add(trusted_width).saturating_add(GAP);
        if i64::from(right) + i64::from(menu.2) <= output_right {
            menu.0 = right;
        }
    }
    menu
}

fn internal_codex_project_menu_placement(
    anchor: Option<&nickel_session_protocol::ShellPopoverAnchor>,
    outputs: &[(crate::internal_shell::InternalOutput, i32, i32)],
    fallback_output: Option<&str>,
) -> crate::internal_codex::CodexSurfacePlacement {
    let requested = anchor
        .map(|anchor| anchor.output.as_str())
        .or(fallback_output);
    let selected = requested
        .and_then(|name| outputs.iter().find(|(output, _, _)| output.name == name))
        .or_else(|| outputs.first());
    let Some((output, origin_x, origin_y)) = selected else {
        return crate::internal_codex::CodexSurfacePlacement::default();
    };
    let (menu_width, menu_height) = crate::internal_codex::MENU_SIZE;
    let max_x = output.width.saturating_sub(menu_width) as i32;
    let anchor_center = anchor
        .filter(|anchor| anchor.output == output.name)
        .map_or(24, |anchor| anchor.bounds.x + anchor.bounds.width / 2);
    let x = (anchor_center - menu_width as i32 / 2).clamp(0, max_x);
    let work_height = output
        .height
        .saturating_sub(crate::winit_shell::PANEL_HEIGHT);
    let y = work_height.saturating_sub(menu_height).saturating_sub(8) as i32;
    crate::internal_codex::CodexSurfacePlacement {
        output: Some(output.name.clone()),
        origin: (origin_x + x, origin_y + y),
        scale: output.scale,
    }
}

fn internal_codex_chat_placement(
    outputs: &[(crate::internal_shell::InternalOutput, i32, i32)],
    requested_output: Option<&str>,
) -> crate::internal_codex::CodexSurfacePlacement {
    let selected = requested_output
        .and_then(|name| outputs.iter().find(|(output, _, _)| output.name == name))
        .or_else(|| outputs.first());
    let Some((output, origin_x, origin_y)) = selected else {
        return crate::internal_codex::CodexSurfacePlacement::default();
    };
    let (content_width, content_height) = crate::internal_codex::CHAT_SIZE;
    let border = crate::session::window_frame::RESIZE_BORDER.max(0) as u32;
    let titlebar = crate::session::window_frame::TITLEBAR_HEIGHT.max(0) as u32;
    let outer_width = content_width.saturating_add(border.saturating_mul(2));
    let outer_height = content_height
        .saturating_add(titlebar)
        .saturating_add(border.saturating_mul(2));
    let work_height = output
        .height
        .saturating_sub(crate::winit_shell::PANEL_HEIGHT);
    let outer_x = output.width.saturating_sub(outer_width) / 2;
    let outer_y = work_height.saturating_sub(outer_height) / 2;
    crate::internal_codex::CodexSurfacePlacement {
        output: Some(output.name.clone()),
        origin: (
            origin_x + outer_x as i32 + border as i32,
            origin_y + outer_y as i32 + titlebar as i32 + border as i32,
        ),
        scale: output.scale,
    }
}

fn locale_is_rtl() -> bool {
    std::env::var("LC_ALL")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var("LANG").ok())
        .is_some_and(|locale| {
            ["ar", "fa", "he", "ur"].iter().any(|language| {
                locale == *language
                    || locale.starts_with(&format!("{language}_"))
                    || locale.starts_with(&format!("{language}-"))
            })
        })
}

fn maximized_content_geometry(frame: Geometry, server_decorated: bool) -> Geometry {
    if server_decorated {
        Geometry {
            x: frame.x + crate::session::window_frame::RESIZE_BORDER,
            y: frame.y
                + crate::session::window_frame::TITLEBAR_HEIGHT
                + crate::session::window_frame::RESIZE_BORDER,
            width: (frame.width - crate::session::window_frame::RESIZE_BORDER * 2).max(1),
            height: (frame.height
                - crate::session::window_frame::TITLEBAR_HEIGHT
                - crate::session::window_frame::RESIZE_BORDER * 2)
                .max(1),
        }
    } else {
        frame
    }
}

fn clamp_decorated_content_to_work_area(content: Geometry, work_area: Geometry) -> Geometry {
    let outer = crate::session::window_frame::outer_geometry(content);
    let location = clamp_window_location(
        (outer.x, outer.y).into(),
        (outer.width, outer.height).into(),
        work_area,
    );
    Geometry {
        x: location.x + (content.x - outer.x),
        y: location.y + (content.y - outer.y),
        ..content
    }
}

fn restored_drag_content_geometry(
    current_content: Geometry,
    restore_content: Geometry,
    pointer: Point<f64, Logical>,
    server_decorated: bool,
    work_area: Geometry,
) -> Geometry {
    let current_outer = if server_decorated {
        crate::session::window_frame::outer_geometry(current_content)
    } else {
        current_content
    };
    let restored_outer_size = if server_decorated {
        crate::session::window_frame::outer_geometry(Geometry {
            x: 0,
            y: 0,
            ..restore_content
        })
    } else {
        Geometry {
            x: 0,
            y: 0,
            ..restore_content
        }
    };
    let horizontal = ((pointer.x - f64::from(current_outer.x))
        / f64::from(current_outer.width.max(1)))
    .clamp(0.0, 1.0);
    let titlebar_offset = (pointer.y - f64::from(current_outer.y)).clamp(
        0.0,
        f64::from(crate::session::window_frame::TITLEBAR_HEIGHT.max(1)),
    );
    let minimum_visible = 32.min(restored_outer_size.width.max(1));
    let outer_x = (pointer.x - horizontal * f64::from(restored_outer_size.width)).round() as i32;
    let outer_y = (pointer.y - titlebar_offset).round() as i32;
    let outer_x = outer_x.clamp(
        work_area.x - restored_outer_size.width + minimum_visible,
        work_area.x + work_area.width - minimum_visible,
    );
    let outer_y = outer_y.clamp(
        work_area.y,
        work_area.y + work_area.height - crate::session::window_frame::TITLEBAR_HEIGHT.max(1),
    );

    if server_decorated {
        Geometry {
            x: outer_x + crate::session::window_frame::RESIZE_BORDER,
            y: outer_y
                + crate::session::window_frame::TITLEBAR_HEIGHT
                + crate::session::window_frame::RESIZE_BORDER,
            width: restore_content.width,
            height: restore_content.height,
        }
    } else {
        Geometry {
            x: outer_x,
            y: outer_y,
            ..restore_content
        }
    }
}

impl Drop for NickelSession {
    fn drop(&mut self) {
        if let Some(control) = &self.compatibility_control {
            let _ = std::fs::remove_file(&control.socket_path);
            crate::model::clear_trusted_session_capability(control.socket_path.as_os_str());
            if std::env::var_os("NICKEL_SESSION_CONTROL").as_deref()
                == Some(control.socket_path.as_os_str())
            {
                // SAFETY: production owns one session; serialized session tests
                // also tear down the capability they installed.
                unsafe {
                    std::env::remove_var("NICKEL_SESSION_CONTROL");
                    if std::env::var("NICKEL_SESSION_TOKEN").as_deref()
                        == Ok(control.protocol_token.as_str())
                    {
                        std::env::remove_var("NICKEL_SESSION_TOKEN");
                    }
                    std::env::remove_var("NICKEL_SHELL_TEST_CONTROL");
                }
            }
        }
    }
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
    pub portal_capture_allowed: bool,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

#[cfg(test)]
mod protocol_tests {
    use super::{
        DisplacedWindow, PREVIEW_BYTE_CAPACITY, PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER,
        PREVIEW_ENTRY_CAPACITY, PREVIEW_FRAME_BYTES, PendingLaunchObservation,
        PendingLaunchWindowDisposition, RegisteredShellRole, ShellRegistrationRejection,
        admitted_preview_ids, advance_preview_content_generation, apply_shell_behavior_value,
        bounded_preview_ids, clamp_decorated_content_to_work_area, clamp_window_location,
        command_requires_shell_identity, drag_icon_location, identification_expiry_is_current,
        maximized_content_geometry, output_contains_logical_point, output_index_for_shell_surface,
        pending_launch_window_disposition, prepare_shell_behavior_update,
        preview_mapping_has_exact_size, protocol_preview_from_cached,
        record_preview_capture_attempt, restored_drag_content_geometry,
        retain_live_idle_inhibitors, retire_displaced_window, retire_pointer_surface,
        retire_shell_surface, reuse_preview_pixels, shell_behavior_value,
        shell_registration_is_active, shell_registration_rejection,
        shell_registration_role_changed, shell_role_accepts_ordinary_focus,
        test_control_may_invoke,
    };
    use crate::session::output_retirement::{
        BIND_SETTLE_GRACE as OUTPUT_GLOBAL_BIND_SETTLE_GRACE,
        DISABLED_GRACE as OUTPUT_GLOBAL_DISABLED_GRACE,
        MAX_PENDING as MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS,
    };
    use crate::session::shell_layout::Geometry;
    use crate::{
        platform::{SessionRequestError, ShellCommand},
        session_host::SessionHost,
    };
    use nickel_session_protocol::{
        Command, OutputTransform, Query, ServerEnvelope, ServerMessage, SessionAction,
        ShellBehaviorSetting, ShellBehaviorTransaction, ShellBehaviorValue, ShellRole, TestOutput,
    };
    use smithay::{
        output::{Output, PhysicalProperties, Subpixel},
        reexports::{
            calloop::EventLoop,
            wayland_server::{Display, backend::ObjectId},
        },
        utils::Point,
    };
    use std::time::{Duration, Instant};
    use std::{
        collections::{HashMap, HashSet},
        sync::Arc,
    };

    struct InternalWindowTestApp;
    struct InternalHitTestApp;

    fn internal_shell_test_session() -> (
        EventLoop<'static, super::NickelSession>,
        super::NickelSession,
    ) {
        let (event_loop, mut session) = preview_test_session();
        let output = Output::new(
            "file-test".into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Nickel".into(),
                model: "Test".into(),
                serial_number: "file-test".into(),
            },
        );
        output.change_current_state(
            Some(smithay::output::Mode {
                size: (1280, 720).into(),
                refresh: 60_000,
            }),
            None,
            None,
            None,
        );
        session.space.map_output(&output, (0, 0));
        let (_sender, receiver) = crate::platform::status_mailbox::channel();
        session
            .enable_internal_shell_with_system_updates(Arc::new(IdleInternalHost), receiver)
            .unwrap();
        (event_loop, session)
    }

    #[test]
    fn remote_codex_owner_creates_hidden_menu_and_tears_down_every_owned_surface() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let mut settings = nickel_core::optional_features::OptionalFeatureSettings {
            codex_enabled: false,
            codex_generation: 41,
            ..Default::default()
        };
        session.apply_remote_codex_preference(&settings).unwrap();
        assert!(session.internal_codex.is_none());
        assert_eq!(session.remote_codex_runtime_generation, 41);

        settings.codex_enabled = true;
        settings.codex_generation = 42;
        session.apply_remote_codex_preference(&settings).unwrap();
        let owned = session
            .internal_codex
            .as_ref()
            .unwrap()
            .surface_ids()
            .collect::<Vec<_>>();
        assert_eq!(owned.len(), 1);
        assert!(!session.internal_ui.is_visible(owned[0]));
        assert_eq!(session.remote_codex_runtime_generation, 42);

        settings.codex_enabled = false;
        settings.codex_generation = 43;
        session.apply_remote_codex_preference(&settings).unwrap();
        assert!(session.internal_codex.is_none());
        assert!(
            owned
                .into_iter()
                .all(|id| !session.internal_ui.is_visible(id))
        );
        assert_eq!(session.remote_codex_runtime_generation, 43);
    }

    #[test]
    fn newly_inserted_window_context_menu_receives_keyboard_focus() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        session
            .internal_shell
            .as_mut()
            .unwrap()
            .apply_session_snapshot(nickel_session_protocol::Snapshot {
                windows: vec![nickel_session_protocol::WindowSnapshot {
                    id: nickel_session_protocol::WindowId(41),
                    application_id: "owned-test".into(),
                    title: "Owned test".into(),
                    active: true,
                    minimized: false,
                    maximized: false,
                    fullscreen: false,
                    geometry: None,
                    workspace: nickel_session_protocol::WorkspaceId(1),
                }],
                ..Default::default()
            });
        assert!(
            session
                .internal_shell
                .as_mut()
                .unwrap()
                .open_window_menu_at(41, 120, 80)
        );
        session.sync_internal_shell();
        let menu = session
            .internal_shell
            .as_ref()
            .unwrap()
            .surface(crate::winit_shell::SurfaceRole::WindowContextMenu, None)
            .unwrap()
            .id;
        let runtime = session.internal_shell_surfaces[&menu];
        assert_eq!(session.internal_ui.focused(), Some(runtime));
    }

    #[test]
    fn removed_shell_output_retires_every_old_surface_presentation() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let old = session
            .internal_shell_surfaces
            .values()
            .copied()
            .collect::<Vec<_>>();
        assert!(!old.is_empty());

        session.internal_shell.as_mut().unwrap().set_outputs(&[]);
        session.sync_internal_shell();

        assert!(session.internal_shell_surfaces.is_empty());
        assert!(
            old.into_iter()
                .all(|id| !session.internal_ui.is_visible(id))
        );
    }

    #[test]
    fn workspace_notifications_record_coarse_owner_state_once() {
        use nickel_remote_control::desktop_events::DesktopEventKind;
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let initial_count = session.workspaces.ordered().len();

        session.notify_workspace_state();
        session.notify_workspace_state();
        assert_eq!(session.remote_desktop_events.snapshot().events.len(), 1);

        session.workspaces.create().unwrap();
        session.notify_workspace_state();
        let snapshot = session.remote_desktop_events.snapshot();
        assert_eq!(snapshot.events.len(), 2);
        assert_eq!(
            snapshot.events.last().unwrap().event,
            DesktopEventKind::WorkspaceStateChanged {
                active_workspace: 1,
                workspaces: initial_count + 1,
            }
        );
    }

    #[test]
    fn protocol_publication_records_ordinary_window_state_changes() {
        use nickel_remote_control::desktop_events::DesktopEventKind;
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let surface = session.internal_ui.insert(
            InternalWindowTestApp,
            crate::session::InternalSurfacePlacement {
                role: crate::session::InternalSurfaceRole::Application,
                geometry: (80, 90, 640, 480),
                output: Some("file-test".into()),
            },
            1.0,
        );
        let window = session.register_internal_application(surface).unwrap();
        session.notify_protocol_snapshot();

        session.maximize_window(window);
        let event = session
            .remote_desktop_events
            .snapshot()
            .events
            .into_iter()
            .rev()
            .find(|event| {
                matches!(
                    event.event,
                    DesktopEventKind::WindowStateChanged { window_id, .. }
                        if window_id == window.0
                )
            })
            .expect("window state event");
        assert!(matches!(
            event.event,
            DesktopEventKind::WindowStateChanged {
                maximized: true,
                minimized: false,
                fullscreen: false,
                ..
            }
        ));
    }

    #[test]
    fn codex_diagnostic_projects_health_without_source_or_failure_details() {
        use nickel_core::optional_features::{
            CodexAvailabilityProjection, FeatureHealth, FeatureInstallation, FeatureSupport,
        };
        use nickel_remote_control::diagnostics::{
            FeatureHealthDiagnostic, FeatureInstallationDiagnostic,
        };
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        session
            .internal_shell
            .as_mut()
            .unwrap()
            .apply_codex_projection(CodexAvailabilityProjection::new(
                FeatureSupport::Supported,
                FeatureInstallation::Incompatible,
                true,
                FeatureHealth::Failed,
                47,
                Some("private path and provider failure".into()),
            ));
        let diagnostic = session
            .remote_codex_feature_diagnostic(8, 19)
            .expect("Codex projection");
        assert_eq!(diagnostic.observation_generation, 8);
        assert_eq!(diagnostic.observed_at_us, 19);
        assert!(diagnostic.supported && diagnostic.enabled);
        assert_eq!(
            diagnostic.installation,
            FeatureInstallationDiagnostic::Incompatible
        );
        assert_eq!(diagnostic.health, FeatureHealthDiagnostic::Failed);
        assert_eq!(diagnostic.configuration_generation, 47);
        let json = serde_json::to_string(&diagnostic).unwrap();
        assert!(!json.contains("private path"));
        assert!(!json.contains("provider failure"));
    }

    #[test]
    fn pending_effect_diagnostic_counts_work_without_payloads_or_targets() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let surface = session
            .internal_shell
            .as_ref()
            .unwrap()
            .surfaces()
            .first()
            .unwrap()
            .id;
        session.pending_desktop_scenes.insert(surface);
        session.pending_shell_focus_role = Some(ShellRole::Launcher);

        let diagnostic = session.remote_pending_effects_diagnostic(12, 34);
        assert_eq!(diagnostic.observation_generation, 12);
        assert_eq!(diagnostic.observed_at_us, 34);
        assert_eq!(diagnostic.desktop_scene_updates, 1);
        assert_eq!(diagnostic.image_copy_frames, 0);
        assert_eq!(diagnostic.launch_observations, 0);
        assert_eq!(diagnostic.output_retirements, 0);
        assert!(diagnostic.shell_focus_pending);

        let json = serde_json::to_string(&diagnostic).unwrap();
        assert!(!json.contains("launcher"));
        assert!(!json.contains("internal:"));
    }

    #[test]
    #[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
    fn shell_capture_evidence_excludes_hidden_retired_and_trusted_surfaces() {
        use nickel_remote_control::leases::ResourceId;
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        session.refresh_remote_output_identities();
        let desktop = session
            .internal_shell
            .as_ref()
            .unwrap()
            .surfaces()
            .iter()
            .find(|entry| entry.role == crate::winit_shell::SurfaceRole::Desktop)
            .unwrap()
            .id;
        let runtime = session.internal_shell_surfaces[&desktop];
        let identity = ResourceId {
            id: format!("internal:{}", runtime.snapshot_token()),
            generation: runtime.snapshot_token(),
        };
        let (_, output) = session.surface_capture_evidence(&identity).unwrap();
        assert_eq!(output.unwrap().id, "file-test");
        session.internal_ui.set_visible(runtime, false);
        assert!(session.surface_capture_evidence(&identity).is_err());
        session.internal_ui.set_visible(runtime, true);
        session.locked = true;
        assert!(session.surface_capture_evidence(&identity).is_err());
        session.locked = false;
        let mut placement = session.internal_ui.placement(runtime).unwrap().clone();
        placement.geometry.0 -= 1;
        session.internal_ui.relocate(runtime, placement);
        assert!(
            session
                .surface_capture_evidence(&identity)
                .unwrap()
                .1
                .is_none(),
            "straddling content cannot claim output membership"
        );
        // Same label with a new native output object cannot reuse old output evidence
        // even before the identity refresh handles topology retirement.
        let native = session.space.outputs().next().unwrap().clone();
        session.space.unmap_output(&native);
        assert!(
            session
                .surface_capture_evidence(&identity)
                .unwrap()
                .1
                .is_none()
        );
        let trusted = session.internal_ui.insert(
            InternalWindowTestApp,
            crate::session::InternalSurfacePlacement {
                role: crate::session::InternalSurfaceRole::TrustedControl,
                geometry: (0, 0, 20, 20),
                output: Some("file-test".into()),
            },
            1.0,
        );
        let trusted_identity = ResourceId {
            id: format!("internal:{}", trusted.snapshot_token()),
            generation: trusted.snapshot_token(),
        };
        assert!(session.surface_capture_evidence(&trusted_identity).is_err());
        assert!(session.internal_ui.remove(runtime));
        assert!(session.surface_capture_evidence(&identity).is_err());
    }

    #[test]
    fn desktop_motion_burst_rebuilds_once_at_frame_boundary_and_focus_cancels_immediately() {
        use crate::session::internal_ui::DesktopPointerAction;
        use nickel_input::{InputEvent, KeyEdge, PointerButton};
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let desktop = session
            .internal_shell
            .as_ref()
            .unwrap()
            .surfaces()
            .iter()
            .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Desktop)
            .unwrap()
            .id;
        let runtime = session.internal_shell_surfaces[&desktop];
        let placement = session.internal_ui.placement(runtime).unwrap().clone();
        let point = (
            f64::from(placement.geometry.0) + f64::from(placement.geometry.2) / 2.0,
            f64::from(placement.geometry.1) + f64::from(placement.geometry.3) / 2.0,
        );
        assert!(session.internal_ui.desktop_pointer_input(
            "test",
            point,
            DesktopPointerAction::Button {
                button: PointerButton::Primary,
                edge: KeyEdge::Pressed
            },
            Default::default(),
            false
        ));
        session.flush_internal_shell_input();
        let generation = |session: &super::NickelSession| {
            session
                .internal_shell
                .as_ref()
                .unwrap()
                .surfaces()
                .iter()
                .find(|surface| surface.id == desktop)
                .unwrap()
                .scene_generation
        };
        let before = generation(&session);
        let armed = session.internal_shell_timer_counters().armed;
        for index in 0..1000 {
            assert!(session.internal_ui.desktop_pointer_input(
                "test",
                (point.0 + f64::from(index % 40), point.1),
                DesktopPointerAction::Motion,
                Default::default(),
                false
            ));
            session.flush_internal_shell_input();
        }
        assert_eq!(
            generation(&session),
            before,
            "input dispatch must not rebuild scenes"
        );
        assert_eq!(session.pending_desktop_scenes.len(), 1);
        assert_eq!(
            session.internal_shell_timer_counters().armed,
            armed,
            "motion must not accumulate wake timers"
        );
        session.flush_desktop_scenes_for_frame();
        assert_eq!(generation(&session), before + 1);
        assert!(session.pending_desktop_scenes.is_empty());
        session.flush_desktop_scenes_for_frame();
        assert_eq!(
            generation(&session),
            before + 1,
            "idle frames must not repeat layout"
        );
        // Cancel through the production lifecycle boundary; never persist a
        // test drag into the user's desktop layout or activate a real file.
        session.internal_ui.step(
            runtime,
            nickel_ui::HostBatch {
                events: vec![nickel_ui::HostEvent::Normalized {
                    input: InputEvent::FocusLost {
                        order: nickel_input::EventOrder(2000),
                    },
                    clipboard_text: None,
                }],
                ..Default::default()
            },
        );
        session.flush_internal_shell_input();
        assert!(generation(&session) > before + 1);
        assert!(session.pending_desktop_scenes.is_empty());
    }

    #[test]
    fn surface_lease_retires_with_runtime_slot_and_cannot_follow_reopened_launcher() {
        use nickel_remote_control::leases::{ResourceId, ResourceScope};
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        session.set_launcher_visible(true);
        let record = session
            .remote_shell_surface_diagnostics()
            .0
            .into_iter()
            .find(|record| {
                matches!(
                    record.role,
                    nickel_remote_control::diagnostics::ShellDiagnosticRole::Launcher
                )
            })
            .unwrap();
        let resource = ResourceId {
            id: record.id,
            generation: record.generation,
        };
        assert!(
            session
                .internal_ui
                .has_surface_identity(&resource.id, resource.generation)
        );
        assert!(!session.internal_ui.has_surface_identity(
            &format!("internal:0{}", resource.generation),
            resource.generation
        ));
        assert!(
            !session
                .internal_ui
                .has_surface_identity(&resource.id, resource.generation + 1)
        );
        let control = session.remote_control.control();
        let lease = control
            .lock()
            .unwrap()
            .leases_mut()
            .approve_local(
                "test-agent".into(),
                ResourceScope::Surface(resource.clone()),
                Instant::now(),
                None,
                false,
                false,
            )
            .unwrap();
        session.schedule_remote_resource_retirement();
        session.flush_remote_resource_retirement();
        assert!(
            control
                .lock()
                .unwrap()
                .leases()
                .iter()
                .any(|candidate| candidate.id == lease)
        );
        session.set_launcher_visible(false);
        assert!(
            !session
                .internal_ui
                .has_surface_identity(&resource.id, resource.generation)
        );
        session.poll_internal_shell(Instant::now());
        session.flush_remote_resource_retirement();
        assert!(
            !control
                .lock()
                .unwrap()
                .leases()
                .iter()
                .any(|candidate| candidate.id == lease)
        );
        session.set_launcher_visible(true);
        assert!(
            !session
                .internal_ui
                .has_surface_identity(&resource.id, resource.generation)
        );
        assert!(
            !control
                .lock()
                .unwrap()
                .leases()
                .iter()
                .any(|candidate| candidate.id == lease)
        );
    }

    #[test]
    fn repeated_native_identity_verification_does_not_duplicate_observation_events() {
        use crate::session::{remote_identity::IdentitySource, window_registry::WindowAdmission};
        use nickel_remote_control::desktop_events::DesktopEventKind;
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (mut event_loop, mut session) = internal_shell_test_session();
        let window = session.windows.insert(WindowAdmission::Ordinary).unwrap();
        for _ in 0..2 {
            session.schedule_remote_window_identity(
                window,
                IdentitySource::WaylandPeer {
                    pid: std::process::id(),
                    app_id: None,
                },
            );
            let deadline = Instant::now() + Duration::from_secs(2);
            while session.remote_window_is_protected(window) && Instant::now() < deadline {
                event_loop
                    .dispatch(Duration::from_millis(10), &mut session)
                    .unwrap();
            }
            assert!(!session.remote_window_is_protected(window));
        }
        let verified = session
            .remote_desktop_events
            .snapshot()
            .events
            .into_iter()
            .filter(|event| {
                event.event
                    == DesktopEventKind::WindowIdentityVerified {
                        window_id: window.0,
                    }
            })
            .count();
        assert_eq!(verified, 1);
        session.retire_surface_window_references(None, Some(window));
        session.retire_surface_window_references(None, Some(window));
        let retired = session
            .remote_desktop_events
            .snapshot()
            .events
            .into_iter()
            .filter(|event| {
                event.event
                    == DesktopEventKind::WindowRetired {
                        window_id: window.0,
                    }
            })
            .count();
        assert_eq!(retired, 1);
        assert!(!session.remote_event_windows.contains(&window));
    }

    #[test]
    fn launch_output_rejects_replaced_native_output_before_inventory_refresh() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        session.refresh_remote_output_identities();
        let identity = session.remote_output_identity("file-test".into()).unwrap();
        let (original, _) = session.remote_output_generations["file-test"].clone();
        assert!(session.validate_launch_output(Some(&identity)).is_ok());
        session.space.map_output(&original, (-1280, 0));
        assert!(session.validate_launch_output(Some(&identity)).is_ok());
        session.space.unmap_output(&original);
        let replacement = Output::new(
            "file-test".into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Nickel".into(),
                model: "Replacement".into(),
                serial_number: "replacement".into(),
            },
        );
        session.space.map_output(&replacement, (0, 0));
        // The published generation has not caught up yet. Comparing the name
        // alone would incorrectly authorize this replacement native object.
        assert_eq!(
            session.remote_output_identity("file-test".into()),
            Some(identity.clone())
        );
        assert!(session.validate_launch_output(Some(&identity)).is_err());
    }

    #[test]
    fn output_lease_generation_survives_repositioning_but_not_output_retirement() {
        use nickel_remote_control::leases::{ResourceId, ResourceScope};
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        session.refresh_remote_output_identities();
        let (output, generation) = session.remote_output_generations["file-test"].clone();
        let initial_history = session.remote_desktop_events.snapshot();
        let control = session.remote_control.control();
        let lease = control
            .lock()
            .unwrap()
            .leases_mut()
            .approve_local(
                "test-agent".into(),
                ResourceScope::Output(ResourceId {
                    id: "file-test".into(),
                    generation,
                }),
                Instant::now(),
                None,
                false,
                false,
            )
            .unwrap();
        session.space.map_output(&output, (-1280, 0));
        session.refresh_remote_output_identities();
        session.flush_remote_resource_retirement();
        assert_eq!(session.remote_output_generations["file-test"].1, generation);
        assert_eq!(
            session.remote_desktop_events.snapshot().generation,
            initial_history.generation,
            "repositioning must not invent an output membership transition"
        );
        assert!(
            control
                .lock()
                .unwrap()
                .leases()
                .iter()
                .any(|candidate| candidate.id == lease)
        );
        session.space.unmap_output(&output);
        session.refresh_remote_output_identities();
        session.flush_remote_resource_retirement();
        assert!(control.lock().unwrap().leases().iter().next().is_none());
        let removed_history = session
            .remote_desktop_events
            .since(initial_history.generation)
            .unwrap();
        assert_eq!(removed_history.events.len(), 1);
        assert_eq!(
            removed_history.events[0].event,
            nickel_remote_control::desktop_events::DesktopEventKind::OutputMembershipChanged {
                latest_output_identity_generation: generation,
                outputs: 0,
            }
        );
        session.refresh_remote_output_identities();
        assert_eq!(
            session.remote_desktop_events.snapshot().generation,
            removed_history.generation,
            "rechecking unchanged membership must not duplicate events"
        );
        session.space.map_output(&output, (0, 0));
        session.refresh_remote_output_identities();
        let replacement_generation = session.remote_output_generations["file-test"].1;
        assert_ne!(replacement_generation, generation);
        let restored_history = session
            .remote_desktop_events
            .since(removed_history.generation)
            .unwrap();
        assert_eq!(restored_history.events.len(), 1);
        assert_eq!(
            restored_history.events[0].event,
            nickel_remote_control::desktop_events::DesktopEventKind::OutputMembershipChanged {
                latest_output_identity_generation: replacement_generation,
                outputs: 1,
            }
        );
        assert!(
            restored_history.events[0].observed_at_us >= removed_history.events[0].observed_at_us
        );
    }

    #[test]
    fn window_retirement_revokes_its_lease_after_the_current_authorized_dispatch() {
        use crate::session::window_registry::{WindowAdmission, WindowId};
        use nickel_remote_control::leases::{ResourceId, ResourceScope};
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let removed = session.windows.insert(WindowAdmission::Ordinary).unwrap();
        let retained = session.windows.insert(WindowAdmission::Ordinary).unwrap();
        let control = session.remote_control.control();
        let mut authority = control.lock().unwrap();
        authority.set_enabled(true);
        let identity = authority.connect_identity("Retirement test").unwrap();
        let scope = |id: WindowId| {
            ResourceScope::Window(ResourceId {
                id: id.0.to_string(),
                generation: id.0,
            })
        };
        let retired_lease = authority
            .leases_mut()
            .approve_local(
                identity.client_id.clone(),
                scope(removed),
                Instant::now(),
                None,
                false,
                false,
            )
            .unwrap();
        let retained_lease = authority
            .leases_mut()
            .approve_local(
                identity.client_id,
                scope(retained),
                Instant::now(),
                None,
                false,
                false,
            )
            .unwrap();
        // Production lifecycle runs while the outer operation still holds authorization.
        session.remote_window_identities.insert(
            removed,
            crate::session::remote_identity::WindowIdentity::Pending,
        );
        session.retire_surface_window_references(None, Some(removed));
        assert!(
            !session.remote_window_identities.contains_key(&removed),
            "retirement must bound identity storage before deferred lease cleanup"
        );
        assert!(session.remote_resource_recheck_pending);
        drop(authority);
        session.flush_remote_resource_retirement();
        let mut authority = control.lock().unwrap();
        assert_eq!(
            authority
                .leases()
                .iter()
                .map(|lease| lease.id)
                .collect::<Vec<_>>(),
            vec![retained_lease]
        );
        assert!(
            authority
                .leases_mut()
                .take_cancellations()
                .contains(&retired_lease)
        );
    }

    #[test]
    fn remote_lease_approval_rejects_retired_and_nonexistent_generation_targets() {
        use nickel_session_protocol::{
            Command, RemoteLeaseRequest, RemoteResourceId, RemoteResourceScope, ServerMessage,
        };
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let control = session.remote_control.control();
        control.lock().unwrap().set_enabled(true);
        let surface = session.internal_ui.insert_scene(
            Vec::new(),
            super::super::internal_ui::InternalSurfacePlacement {
                role: super::super::internal_ui::InternalSurfaceRole::Panel,
                geometry: (0, 0, 100, 40),
                output: None,
            },
            1.0,
        );
        let identity = control
            .lock()
            .unwrap()
            .connect_identity("Pending retirement")
            .unwrap();
        let request = RemoteLeaseRequest {
            renewal: None,
            scope: RemoteResourceScope::Surface(RemoteResourceId {
                id: format!("internal:{}", surface.snapshot_token()),
                generation: surface.snapshot_token(),
            }),
            duration_seconds: Some(1200),
            allow_resumption: false,
            full_debug: false,
        };
        assert!(session.remote_lease_target_live(&request.scope));
        {
            let mut owner = control.lock().unwrap();
            let now = Instant::now();
            let watch = owner
                .reserve_connection_watch(&identity.client_id, &identity.token, now)
                .unwrap();
            owner
                .activate_connection_watch(&identity.client_id, &identity.token, watch, false, now)
                .unwrap();
        }
        control
            .lock()
            .unwrap()
            .request_lease(
                &identity.client_id,
                &identity.token,
                request.clone().into(),
                Instant::now(),
            )
            .unwrap();
        session.internal_ui.remove(surface);
        let pending_generation = control
            .lock()
            .unwrap()
            .lease_requests()
            .pending_generation(&identity.client_id)
            .unwrap();
        let result = session.handle_protocol_command(
            Command::DecideRemoteLease {
                pending_generation,
                client_id: identity.client_id,
                request,
                allow: true,
            },
            None,
            0,
        );
        assert!(matches!(result, ServerMessage::Error { .. }));
        assert_eq!(control.lock().unwrap().leases().iter().count(), 0);
        for scope in [
            RemoteResourceScope::Window(RemoteResourceId {
                id: u64::MAX.to_string(),
                generation: u64::MAX,
            }),
            RemoteResourceScope::Output(RemoteResourceId {
                id: "missing-output".into(),
                generation: u64::MAX,
            }),
        ] {
            let identity = control
                .lock()
                .unwrap()
                .connect_identity("Missing resource")
                .unwrap();
            let request = RemoteLeaseRequest {
                renewal: None,
                scope,
                duration_seconds: Some(1200),
                allow_resumption: false,
                full_debug: false,
            };
            {
                let mut owner = control.lock().unwrap();
                let now = Instant::now();
                let watch = owner
                    .reserve_connection_watch(&identity.client_id, &identity.token, now)
                    .unwrap();
                owner
                    .activate_connection_watch(
                        &identity.client_id,
                        &identity.token,
                        watch,
                        false,
                        now,
                    )
                    .unwrap();
            }
            control
                .lock()
                .unwrap()
                .request_lease(
                    &identity.client_id,
                    &identity.token,
                    request.clone().into(),
                    Instant::now(),
                )
                .unwrap();
            let pending_generation = control
                .lock()
                .unwrap()
                .lease_requests()
                .pending_generation(&identity.client_id)
                .unwrap();
            let result = session.handle_protocol_command(
                Command::DecideRemoteLease {
                    pending_generation,
                    client_id: identity.client_id,
                    request,
                    allow: true,
                },
                None,
                0,
            );
            assert!(matches!(result, ServerMessage::Error { .. }));
        }
        assert_eq!(control.lock().unwrap().leases().iter().count(), 0);
    }

    #[test]
    fn remote_lease_resume_checks_live_protection_and_pending_retirement() {
        use crate::session::internal_ui::{InternalSurfacePlacement, InternalSurfaceRole};
        use nickel_session_protocol::{
            Command, RemoteLeaseAction, RemoteResourceId, RemoteResourceScope, ServerMessage,
        };
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let placement = |role| InternalSurfacePlacement {
            role,
            geometry: (0, 0, 100, 40),
            output: None,
        };
        let surface = session.internal_ui.insert_scene(
            Vec::new(),
            placement(InternalSurfaceRole::Panel),
            1.0,
        );
        let control = session.remote_control.control();
        let mut authority = control.lock().unwrap();
        authority.set_enabled(true);
        let identity = authority.connect_identity("Resume lifecycle").unwrap();
        let lease = authority
            .leases_mut()
            .approve_local(
                identity.client_id,
                RemoteResourceScope::Surface(RemoteResourceId {
                    id: format!("internal:{}", surface.snapshot_token()),
                    generation: surface.snapshot_token(),
                }),
                Instant::now(),
                None,
                false,
                false,
            )
            .unwrap();
        drop(authority);
        let manage = |session: &mut super::NickelSession, action| {
            session.handle_protocol_command(
                Command::ManageRemoteLease {
                    lease_id: lease,
                    action,
                },
                None,
                0,
            )
        };
        assert!(matches!(
            manage(&mut session, RemoteLeaseAction::Pause),
            ServerMessage::RemoteControl(_)
        ));
        session
            .internal_ui
            .relocate(surface, placement(InternalSurfaceRole::TrustedControl));
        assert!(matches!(
            manage(&mut session, RemoteLeaseAction::Resume),
            ServerMessage::Error { .. }
        ));
        assert!(
            control
                .lock()
                .unwrap()
                .leases()
                .iter()
                .find(|item| item.id == lease)
                .unwrap()
                .suspended
        );
        session
            .internal_ui
            .relocate(surface, placement(InternalSurfaceRole::Panel));
        assert!(matches!(
            manage(&mut session, RemoteLeaseAction::Resume),
            ServerMessage::RemoteControl(_)
        ));
        assert!(
            !control
                .lock()
                .unwrap()
                .leases()
                .iter()
                .find(|item| item.id == lease)
                .unwrap()
                .suspended
        );
        manage(&mut session, RemoteLeaseAction::Pause);
        session.internal_ui.remove(surface);
        // Retirement is deferred; resume must check the owner before that queue runs.
        assert!(matches!(
            manage(&mut session, RemoteLeaseAction::Resume),
            ServerMessage::Error { .. }
        ));
        assert!(
            control
                .lock()
                .unwrap()
                .leases()
                .iter()
                .find(|item| item.id == lease)
                .unwrap()
                .suspended
        );
    }

    #[test]
    fn remote_lease_local_commands_approve_pause_resume_and_revoke() {
        use nickel_session_protocol::{
            Command, RemoteLeaseAction, RemoteLeaseRequest, RemoteResourceScope, ServerMessage,
        };
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let control = session.remote_control.control();
        control.lock().unwrap().set_enabled(true);
        let identity = control
            .lock()
            .unwrap()
            .connect_identity("Lease test")
            .unwrap();
        let request = RemoteLeaseRequest {
            renewal: None,
            scope: RemoteResourceScope::FullSession,
            duration_seconds: Some(1200),
            allow_resumption: false,
            full_debug: false,
        };
        {
            let mut owner = control.lock().unwrap();
            let now = Instant::now();
            let watch = owner
                .reserve_connection_watch(&identity.client_id, &identity.token, now)
                .unwrap();
            owner
                .activate_connection_watch(&identity.client_id, &identity.token, watch, false, now)
                .unwrap();
        }
        control
            .lock()
            .unwrap()
            .request_lease(
                &identity.client_id,
                &identity.token,
                request.clone().into(),
                Instant::now(),
            )
            .unwrap();
        let pending_generation = control
            .lock()
            .unwrap()
            .lease_requests()
            .pending_generation(&identity.client_id)
            .unwrap();
        let result = session.handle_protocol_command(
            Command::DecideRemoteLease {
                pending_generation,
                client_id: identity.client_id,
                request,
                allow: true,
            },
            None,
            0,
        );
        let ServerMessage::RemoteControl(snapshot) = result else {
            panic!("approval failed");
        };
        assert!(snapshot.pending_leases.is_empty());
        assert_eq!(snapshot.active_leases.len(), 1);
        let lease_id = snapshot.active_leases[0].lease_id;
        for (action, suspended) in [
            (RemoteLeaseAction::Pause, true),
            (RemoteLeaseAction::Resume, false),
        ] {
            let result = session.handle_protocol_command(
                Command::ManageRemoteLease { lease_id, action },
                None,
                0,
            );
            let ServerMessage::RemoteControl(snapshot) = result else {
                panic!("management failed");
            };
            assert_eq!(snapshot.active_leases[0].suspended, suspended);
            assert_eq!(session.remote_indicator_surfaces.len(), 1);
            let indicator = *session.remote_indicator_surfaces.values().next().unwrap();
            let app = session
                .internal_ui
                .application_mut::<crate::session::remote_indicator::RemoteIndicator>(indicator)
                .unwrap();
            assert_eq!(app.grants.len(), 1);
            assert_eq!(app.grants[0].suspended, suspended);
            assert!(app.grants[0].connected);
        }
        let result = session.handle_protocol_command(
            Command::ManageRemoteLease {
                lease_id,
                action: RemoteLeaseAction::Revoke,
            },
            None,
            0,
        );
        let ServerMessage::RemoteControl(snapshot) = result else {
            panic!("revoke failed");
        };
        assert!(snapshot.active_leases.is_empty());
        assert!(session.remote_indicator_surfaces.is_empty());
    }

    #[test]
    fn active_remote_lease_owns_one_overlay_per_output_and_revoke_removes_it() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let control = session.remote_control.control();
        let mut control_guard = control.lock().unwrap();
        control_guard.set_enabled(true);
        let pairing = control_guard.start_pairing(10).unwrap();
        let pending = control_guard
            .exchange_short_code(
                &pairing.ceremony_id,
                &pairing.short_code,
                "Indicator test",
                vec![nickel_remote_control::Capability::Observe],
                11,
            )
            .unwrap();
        control_guard
            .approve(
                &pending.id,
                nickel_remote_control::Approval::AllowOnce,
                vec![nickel_remote_control::Capability::Observe],
            )
            .unwrap();
        drop(control_guard);

        session.sync_remote_control_indicators();
        assert!(session.remote_indicator_surfaces.is_empty());
        control
            .lock()
            .unwrap()
            .leases_mut()
            .approve_local(
                pending.id.clone(),
                nickel_remote_control::leases::ResourceScope::FullSession,
                Instant::now(),
                None,
                false,
                false,
            )
            .unwrap();
        session.sync_remote_control_indicators();
        assert_eq!(session.remote_indicator_surfaces.len(), 1);
        let indicator = session.remote_indicator_surfaces["file-test"];
        let placement = session.internal_ui.placement(indicator).unwrap();
        assert_eq!(
            placement.role,
            crate::session::InternalSurfaceRole::TrustedControl
        );
        assert_eq!(placement.output.as_deref(), Some("file-test"));
        assert!(
            session
                .internal_ui
                .surface_at((900.0, 30.0), true)
                .is_some()
        );

        let stop = session
            .internal_ui
            .semantic_nodes(indicator)
            .into_iter()
            .find(|node| node.name.as_deref() == Some("Stop"))
            .expect("trusted indicator exposes an accessible Stop button");
        assert!(stop.actions.contains(&nickel_ui::ActionKind::Activate));
        session.internal_ui.step(
            indicator,
            nickel_ui::HostBatch {
                events: vec![nickel_ui::HostEvent::Accessibility {
                    target: stop.id,
                    action: nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
                }],
                ..Default::default()
            },
        );
        assert!(
            session
                .internal_ui
                .application_mut::<super::super::remote_indicator::RemoteIndicator>(indicator)
                .unwrap()
                .stop_requested
        );
        assert!(control.lock().unwrap().revoke(&pending.id));
        session.sync_remote_control_indicators();
        assert!(session.remote_indicator_surfaces.is_empty());
        assert!(session.internal_ui.placement(indicator).is_none());

        let pairing = control.lock().unwrap().start_pairing(20).unwrap();
        let pending = control
            .lock()
            .unwrap()
            .exchange_short_code(
                &pairing.ceremony_id,
                &pairing.short_code,
                "Lock test",
                vec![nickel_remote_control::Capability::Observe],
                21,
            )
            .unwrap();
        control
            .lock()
            .unwrap()
            .approve(
                &pending.id,
                nickel_remote_control::Approval::AllowOnce,
                vec![nickel_remote_control::Capability::Observe],
            )
            .unwrap();
        control
            .lock()
            .unwrap()
            .leases_mut()
            .approve_local(
                pending.id.clone(),
                nickel_remote_control::leases::ResourceScope::FullSession,
                Instant::now(),
                None,
                false,
                false,
            )
            .unwrap();
        session.sync_remote_control_indicators();
        assert_eq!(session.remote_indicator_surfaces.len(), 1);
        session.locked = true;
        session.remote_control.lock();
        session.sync_remote_control_indicators();
        assert!(session.remote_indicator_surfaces.is_empty());
        assert!(control.lock().unwrap().granted_clients().next().is_none());
    }

    #[test]
    fn file_open_focus_close_uses_the_canonical_application_lifecycle() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let directory = tempfile::tempdir().unwrap();
        let request = nickel_file::FileWindowRequest::OpenOrFocus(nickel_file::FileLaunch::Browse(
            directory.path().into(),
        ));
        let action = session
            .internal_shell
            .as_mut()
            .unwrap()
            .file_windows_mut()
            .handle(request.clone());
        session.apply_internal_file_action(action);
        let (&file, &surface) = session.internal_file_surfaces.iter().next().unwrap();
        let window = session.internal_window_for_surface(surface).unwrap();
        assert!(session.internal_applications_are_foremost());
        assert_eq!(session.internal_ui.focused(), Some(surface));
        assert!(
            session
                .protocol_windows()
                .iter()
                .any(|item| item.id.0 == window.0 && item.application_id == "nickel-file")
        );
        session.minimize_window(window);
        let action = session
            .internal_shell
            .as_mut()
            .unwrap()
            .file_windows_mut()
            .handle(request.clone());
        session.apply_internal_file_action(action);
        assert!(session.internal_ui.is_visible(surface));
        session.close_window(window);
        assert!(!session.windows.contains(window));
        assert!(!session.internal_file_surfaces.contains_key(&file));
        let action = session
            .internal_shell
            .as_mut()
            .unwrap()
            .file_windows_mut()
            .handle(request);
        assert!(matches!(action, nickel_file::FileWindowAction::Opened(_)));
    }

    impl nickel_ui::Application for InternalWindowTestApp {
        type Message = ();

        fn update(&mut self, (): ()) {}

        fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<Self::Message> {
            nickel_ui::Text::new("internal window")
        }

        fn title(&self) -> &str {
            "Codex — Nickel"
        }
    }

    impl nickel_ui::Application for InternalHitTestApp {
        type Message = ();

        fn update(&mut self, (): ()) {}

        fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<Self::Message> {
            nickel_ui::Button::new((), "internal action")
        }

        fn title(&self) -> &str {
            "Internal hit test"
        }
    }

    #[test]
    fn asynchronous_native_paste_rejects_field_transfer_and_return() {
        use image::ImageEncoder;
        use nickel_ui::{UiEvent, id, ui};
        use std::io::Write;
        #[derive(Default)]
        struct Fields {
            first: String,
            second: String,
        }
        impl nickel_ui::Application for Fields {
            type Message = (bool, String);
            fn update(&mut self, (second, text): Self::Message) {
                if second {
                    self.second = text;
                } else {
                    self.first = text;
                }
            }
            fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<Self::Message> {
                ui! { <Column>
                    <TextField id={id!(first)} value={&self.first} on_change={|text| (false, text)} />
                    <TextField id={id!(second)} value={&self.second} on_change={|text| (true, text)} />
                </Column> }
            }
            fn title(&self) -> &str {
                "Paste field lease"
            }
        }
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (mut event_loop, mut session) = preview_test_session();
        let recipient = session.internal_ui.insert(
            Fields::default(),
            crate::session::InternalSurfacePlacement {
                role: crate::session::InternalSurfaceRole::Application,
                geometry: (0, 0, 640, 480),
                output: None,
            },
            1.0,
        );
        session.configure_on_screen_keyboard(true, true, 41, false, false, 368);
        session.native_clipboard.text_limit = Some(8);
        session.focus_internal_surface(recipient);
        session.internal_ui.keyboard(UiEvent::FocusNext);
        let epoch = session.on_screen_keyboard_snapshot().epoch;
        let key = crate::session::input::internal_virtual_key(
            'v' as u32,
            &[0xffe3],
            nickel_input::EventOrder(1),
        )
        .unwrap();
        for return_to_original in [false, true] {
            let original = session.internal_ui.focused_field_lease(recipient).unwrap();
            let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
            let permit = session.native_clipboard.reads.acquire(1).unwrap();
            session
                .begin_native_paste_read(reader.into(), epoch, key.clone(), recipient, 8, permit)
                .unwrap();
            session.internal_ui.keyboard(UiEvent::FocusNext);
            if return_to_original {
                session.internal_ui.keyboard(UiEvent::FocusNext);
            }
            let current = session.internal_ui.focused_field_lease(recipient).unwrap();
            assert_ne!(original.1, current.1);
            assert_eq!(original.0 == current.0, return_to_original);
            assert_eq!(session.on_screen_keyboard_snapshot().epoch, epoch);
            writer.write_all(b"stale").unwrap();
            drop(writer);
            let deadline = Instant::now() + std::time::Duration::from_secs(2);
            while session.native_clipboard.pending_read.is_some() && Instant::now() < deadline {
                event_loop
                    .dispatch(Some(std::time::Duration::from_millis(10)), &mut session)
                    .unwrap();
            }
            assert!(session.native_clipboard.pending_read.is_none());
            assert_eq!(
                session.native_clipboard.last_failure.as_deref(),
                Some("clipboard paste field changed")
            );
            let fields = session
                .internal_ui
                .application::<Fields>(recipient)
                .unwrap();
            assert!(fields.first.is_empty() && fields.second.is_empty());
        }

        let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let permit = session.native_clipboard.reads.acquire(1).unwrap();
        session
            .begin_native_image_read(reader.into(), recipient, permit)
            .unwrap();
        session.internal_ui.keyboard(UiEvent::FocusNext);
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[1, 2, 3, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        writer.write_all(&png).unwrap();
        drop(writer);
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while session.native_clipboard.pending_read.is_some() && Instant::now() < deadline {
            event_loop
                .dispatch(Some(std::time::Duration::from_millis(10)), &mut session)
                .unwrap();
        }
        assert!(session.native_clipboard.pending_read.is_none());
        assert_eq!(
            session.native_clipboard.last_failure.as_deref(),
            Some("clipboard paste field changed")
        );

        let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let permit = session.native_clipboard.reads.acquire(1).unwrap();
        session
            .begin_native_direct_paste_read(reader.into(), key, recipient, 8, permit)
            .unwrap();
        writer.write_all(b"direct").unwrap();
        drop(writer);
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while session.native_clipboard.pending_read.is_some() && Instant::now() < deadline {
            event_loop
                .dispatch(Some(std::time::Duration::from_millis(10)), &mut session)
                .unwrap();
        }
        assert!(session.native_clipboard.last_failure.is_none());
        let fields = session
            .internal_ui
            .application::<Fields>(recipient)
            .unwrap();
        assert!(fields.first == "direct" || fields.second == "direct");
    }

    #[test]
    fn native_keyboard_leases_follow_internal_recipients_without_seat_focus() {
        use nickel_session_protocol::OnScreenKeyboardInput;
        use nickel_ui::{UiEvent, id, ui};
        use std::io::Write;

        #[derive(Default)]
        struct TypingApp(String);
        impl nickel_ui::Application for TypingApp {
            type Message = String;
            fn update(&mut self, text: String) {
                self.0 = text;
            }
            fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<String> {
                ui! { <TextField id={id!(query)} value={&self.0} on_change={|text| text} /> }
            }
            fn title(&self) -> &str {
                "Keyboard recipient"
            }
        }

        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (mut event_loop, mut session) = preview_test_session();
        let recipients = [
            crate::session::InternalSurfaceRole::Application,
            crate::session::InternalSurfaceRole::Overlay,
        ]
        .map(|role| {
            session.internal_ui.insert(
                TypingApp::default(),
                crate::session::InternalSurfacePlacement {
                    role,
                    geometry: (0, 0, 640, 480),
                    output: None,
                },
                1.0,
            )
        });
        session.configure_on_screen_keyboard(true, true, 41, false, false, 368);
        assert!(session.focus_internal_surface(recipients[0]));
        session.internal_ui.keyboard(UiEvent::FocusNext);
        let first = session.on_screen_keyboard_snapshot();
        assert!(first.recipient.is_none());
        assert_eq!(
            first.internal_recipient,
            Some(recipients[0].snapshot_token())
        );
        assert_ne!(first.epoch, first.generation);
        session
            .deliver_on_screen_keyboard_input(
                first.epoch,
                OnScreenKeyboardInput::Text {
                    text: "hello".into(),
                },
            )
            .unwrap();
        assert_eq!(
            session
                .internal_ui
                .application::<TypingApp>(recipients[0])
                .unwrap()
                .0,
            "hello"
        );

        // Native clipboard ownership follows real UI copy/cut policy; admission
        // failure must preserve both the selected text and the previous owner.
        session.native_clipboard.text_limit = Some(8);
        let physical = session.seat.get_keyboard().unwrap().modifier_state();
        let chord = |keysym| OnScreenKeyboardInput::Key {
            keysym,
            modifiers: vec![0xffe3],
        };
        session
            .deliver_on_screen_keyboard_input(first.epoch, chord('a' as u32))
            .unwrap();
        session
            .deliver_on_screen_keyboard_input(first.epoch, chord('c' as u32))
            .unwrap();
        {
            let owner =
                smithay::wayland::selection::data_device::current_data_device_selection_userdata(
                    &session.seat,
                )
                .unwrap();
            assert!(
                matches!(&*owner, crate::session::handlers::SelectionOwner::NativeText(text) if text.as_ref() == "hello")
            );
        }
        session.native_clipboard.text_limit = Some(4);
        assert!(
            session
                .deliver_on_screen_keyboard_input(first.epoch, chord('x' as u32))
                .is_err()
        );
        assert_eq!(
            session
                .internal_ui
                .application::<TypingApp>(recipients[0])
                .unwrap()
                .0,
            "hello"
        );
        session.native_clipboard.text_limit = Some(8);
        session
            .deliver_on_screen_keyboard_input(first.epoch, chord('x' as u32))
            .unwrap();
        assert!(
            session
                .internal_ui
                .application::<TypingApp>(recipients[0])
                .unwrap()
                .0
                .is_empty()
        );
        session
            .deliver_on_screen_keyboard_input(first.epoch, chord('v' as u32))
            .unwrap();
        assert_eq!(
            session
                .internal_ui
                .application::<TypingApp>(recipients[0])
                .unwrap()
                .0,
            "hello"
        );
        assert_eq!(
            session.seat.get_keyboard().unwrap().modifier_state(),
            physical
        );

        let paste_key = crate::session::input::internal_virtual_key(
            'v' as u32,
            &[0xffe3],
            nickel_input::EventOrder(55),
        )
        .unwrap();
        let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let permit = session.native_clipboard.reads.acquire(1).unwrap();
        session
            .begin_native_paste_read(
                reader.into(),
                first.epoch,
                paste_key.clone(),
                recipients[0],
                8,
                permit,
            )
            .unwrap();
        writer.write_all(b"!").unwrap();
        drop(writer);
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while session.native_clipboard.pending_read.is_some() && Instant::now() < deadline {
            event_loop
                .dispatch(Some(std::time::Duration::from_millis(10)), &mut session)
                .unwrap();
        }
        assert!(session.native_clipboard.pending_read.is_none());
        assert!(
            session.native_clipboard.last_failure.is_none(),
            "Closed must not overwrite successful completion"
        );
        assert_eq!(
            session
                .internal_ui
                .application::<TypingApp>(recipients[0])
                .unwrap()
                .0,
            "hello!"
        );
        let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let permit = session.native_clipboard.reads.acquire(1).unwrap();
        session
            .begin_native_paste_read(
                reader.into(),
                first.epoch,
                paste_key,
                recipients[0],
                8,
                permit,
            )
            .unwrap();

        // Both owners have a None Smithay target. Their distinct native leases
        // must still reject a release captured before the focus transfer.
        assert!(session.internal_ui.touch_with_client(
            0,
            (10.0, 10.0),
            crate::session::TouchPhase::Started,
            false,
        ));
        session.reconcile_internal_application_focus();
        assert_eq!(session.internal_ui.focused(), Some(recipients[1]));
        session.internal_ui.keyboard(UiEvent::FocusNext);
        assert!(
            session
                .deliver_on_screen_keyboard_input(
                    first.epoch,
                    OnScreenKeyboardInput::Text {
                        text: "stale".into()
                    }
                )
                .is_err()
        );
        writer.write_all(b"stale").unwrap();
        drop(writer);
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while session.native_clipboard.pending_read.is_some() && Instant::now() < deadline {
            event_loop
                .dispatch(Some(std::time::Duration::from_millis(10)), &mut session)
                .unwrap();
        }
        assert!(session.native_clipboard.pending_read.is_none());
        assert_eq!(
            session.native_clipboard.last_failure.as_deref(),
            Some("clipboard paste recipient changed")
        );
        assert!(
            session
                .internal_ui
                .application::<TypingApp>(recipients[1])
                .unwrap()
                .0
                .is_empty()
        );
        let second = session.on_screen_keyboard_snapshot();
        assert_ne!(first.epoch, second.epoch);
        session
            .deliver_on_screen_keyboard_input(
                second.epoch,
                OnScreenKeyboardInput::Text { text: "new".into() },
            )
            .unwrap();
        assert_eq!(
            session
                .internal_ui
                .application::<TypingApp>(recipients[1])
                .unwrap()
                .0,
            "new"
        );
        session.surrender_internal_focus();
        assert!(!session.on_screen_keyboard_snapshot().has_recipient());
        assert!(
            session
                .deliver_on_screen_keyboard_input(
                    second.epoch,
                    OnScreenKeyboardInput::Text {
                        text: "stale".into()
                    }
                )
                .is_err()
        );
    }

    #[test]
    fn internal_protection_hides_remote_inventory_without_hiding_local_window() {
        struct ProtectedApp(bool);
        impl nickel_ui::Application for ProtectedApp {
            type Message = ();
            fn update(&mut self, _: ()) {}
            fn remote_access_protected(&self) -> bool {
                self.0
            }
            fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<()> {
                nickel_ui::Text::new("fixture")
            }
        }
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let surface = session.internal_ui.insert(
            ProtectedApp(false),
            crate::session::InternalSurfacePlacement {
                role: crate::session::InternalSurfaceRole::Application,
                geometry: (80, 90, 640, 480),
                output: Some("file-test".into()),
            },
            1.0,
        );
        let window = session.register_internal_application(surface).unwrap();
        // The registration fallback is presentation metadata, not a verified app identity.
        assert_eq!(session.windows.app_id(window), Some("nickel-codex"));
        assert!(session.remote_verified_application(window).is_none());
        assert!(
            session
                .remote_protocol_windows()
                .iter()
                .any(|entry| entry.id.0 == window.0)
        );
        let observed_windows = session
            .protocol_windows()
            .into_iter()
            .map(|window| session.remote_window_summary(window))
            .collect::<Vec<_>>();
        let observed_apps = session.remote_internal_application_diagnostics(&observed_windows);
        session
            .inject_test_input(nickel_session_protocol::TestInput::PointerMove { x: 100, y: 120 })
            .unwrap();
        let input = session.remote_input_diagnostic(&observed_windows, &observed_apps, 1, 1);
        assert_eq!(
            input.keyboard.unwrap().focused_window,
            Some(window.0.to_string())
        );
        assert_eq!(
            input.pointer_hit_test.unwrap().window,
            Some(window.0.to_string())
        );
        assert!(
            session
                .remote_workspace_diagnostics(&observed_windows)
                .iter()
                .any(|workspace| workspace.windows.contains(&window.0.to_string()))
        );
        assert_eq!(
            session
                .remote_internal_renderer_diagnostics(&observed_apps, 1)
                .len(),
            1
        );
        session
            .internal_ui
            .application_mut::<ProtectedApp>(surface)
            .unwrap()
            .0 = true;
        assert!(session.remote_window_is_protected(window));
        // Previously projected identities cannot outlive a live protection change,
        // including before the next hosted frame reconciles presentation state.
        let input = session.remote_input_diagnostic(&observed_windows, &observed_apps, 2, 2);
        assert!(input.keyboard.is_none());
        assert!(input.pointer_hit_test.is_none());
        assert!(
            session
                .remote_workspace_diagnostics(&observed_windows)
                .iter()
                .all(
                    |workspace| !workspace.windows.contains(&window.0.to_string())
                        && workspace.last_focused_window.as_deref()
                            != Some(window.0.to_string().as_str())
                )
        );
        assert!(
            session
                .remote_internal_renderer_diagnostics(&observed_apps, 2)
                .is_empty()
        );
        // Even a caller retaining the local inventory cannot project a protected host.
        let local_windows = session
            .protocol_windows()
            .into_iter()
            .map(|window| session.remote_window_summary(window))
            .collect::<Vec<_>>();
        assert!(
            session
                .remote_internal_application_diagnostics(&local_windows)
                .is_empty()
        );

        assert!(session.internal_ui.step(
            surface,
            nickel_ui::HostBatch {
                application_changed: true,
                ..nickel_ui::HostBatch::default()
            },
        ));
        assert!(
            !session
                .remote_protocol_windows()
                .iter()
                .any(|entry| entry.id.0 == window.0)
        );
        assert!(
            session
                .protocol_windows()
                .iter()
                .any(|entry| entry.id.0 == window.0)
        );
        session
            .internal_ui
            .application_mut::<ProtectedApp>(surface)
            .unwrap()
            .0 = false;
        assert!(session.remote_window_is_protected(window));
        let pending = session.remote_input_diagnostic(&observed_windows, &observed_apps, 3, 3);
        assert!(pending.keyboard.is_none());
        assert!(pending.pointer_hit_test.is_none());
        assert!(session.internal_ui.step(
            surface,
            nickel_ui::HostBatch {
                application_changed: true,
                ..nickel_ui::HostBatch::default()
            },
        ));
        assert!(!session.remote_window_is_protected(window));
        let restored = session.remote_input_diagnostic(&observed_windows, &observed_apps, 4, 4);
        assert_eq!(
            restored.keyboard.unwrap().focused_window,
            Some(window.0.to_string())
        );
        assert_eq!(
            restored.pointer_hit_test.unwrap().window,
            Some(window.0.to_string())
        );
        assert!(
            session
                .remote_protocol_windows()
                .iter()
                .any(|entry| entry.id.0 == window.0)
        );
    }

    #[test]
    fn internal_diagnostics_follow_production_visibility_and_frame_lifecycle() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let surface = session.internal_ui.insert(
            InternalHitTestApp,
            crate::session::InternalSurfacePlacement {
                role: crate::session::InternalSurfaceRole::Application,
                geometry: (-80, 90, 640, 480),
                output: Some("file-test".into()),
            },
            1.25,
        );
        let window = session.register_internal_application(surface).unwrap();
        let windows = session
            .remote_protocol_windows()
            .into_iter()
            .map(|window| session.remote_window_summary(window))
            .collect::<Vec<_>>();
        let records = session.remote_internal_application_diagnostics(&windows);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.id, format!("internal:{}", surface.snapshot_token()));
        assert_eq!(record.generation, surface.snapshot_token());
        assert_eq!(record.window, window.0.to_string());
        assert_eq!(record.geometry, [-80, 90, 640, 480]);
        assert_eq!(record.output.as_deref(), Some("file-test"));
        assert_eq!(record.scale_factor, 1.25);
        assert!(record.keyboard_focused);
        let (_, semantic_nodes) = session
            .internal_ui
            .bounded_application_semantics(surface)
            .unwrap();
        let semantic_bounds = semantic_nodes.first().unwrap().bounds;
        session
            .inject_test_input(nickel_session_protocol::TestInput::PointerMove {
                x: -80
                    + (semantic_bounds.origin.x + semantic_bounds.size.width / 2.0).round() as i32,
                y: 90
                    + (semantic_bounds.origin.y + semantic_bounds.size.height / 2.0).round() as i32,
            })
            .unwrap();
        let input = session.remote_input_diagnostic(&windows, &records, 7, 11);
        assert_eq!(
            input.pointer_hit_test.as_ref().unwrap().window.as_deref(),
            Some(record.window.as_str())
        );
        assert!(
            input
                .pointer_hit_test
                .as_ref()
                .unwrap()
                .semantic_tree_generation
                .is_some(),
            "bounds={semantic_bounds:?} pointer={:?} hit={:?}",
            session.seat.get_pointer().unwrap().current_location(),
            input.pointer_hit_test
        );
        assert!(
            input
                .pointer_hit_test
                .as_ref()
                .unwrap()
                .semantic_node
                .is_some()
        );
        session
            .inject_test_input(nickel_session_protocol::TestInput::PointerMove { x: 0, y: 70 })
            .unwrap();
        let frame_hit = session.remote_input_diagnostic(&windows, &records, 8, 12);
        assert_eq!(
            frame_hit.pointer_hit_test.unwrap().decoration,
            Some(nickel_remote_control::diagnostics::InternalDecorationHit::Titlebar)
        );
        assert!(
            session
                .remote_input_diagnostic(&windows, &[], 9, 13)
                .pointer_hit_test
                .is_none(),
            "unprojected internal applications must remain unavailable"
        );
        assert_eq!(input.observation_generation, 7);
        assert_eq!(input.observed_at_us, 11);
        assert_eq!(
            input.keyboard.unwrap().focused_window.as_deref(),
            Some(record.window.as_str())
        );
        assert!(
            session
                .remote_input_diagnostic(&windows, &[], 8, 12)
                .keyboard
                .is_none()
        );

        assert!(record.redraw_pending);
        assert!(
            session
                .remote_internal_application_diagnostics(&[])
                .is_empty()
        );
        session.internal_ui.step(
            surface,
            nickel_ui::HostBatch {
                application_changed: true,
                ..nickel_ui::HostBatch::default()
            },
        );
        let next = session.remote_internal_application_diagnostics(&windows);
        assert!(next[0].resolved_frame_generation > record.resolved_frame_generation);
        assert_eq!(next[0].generation, record.generation);
        session.internal_ui.set_visible(surface, false);
        // Hidden ordinary applications remain inspectable under the same lease;
        // visibility and renderer suspension are part of the diagnostic state.
        let hidden = session.remote_internal_application_diagnostics(&windows);
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].id, record.id);
        assert_eq!(hidden[0].generation, record.generation);
        assert!(!hidden[0].visible);
        assert!(!hidden[0].keyboard_focused);
        let hidden_renderers = session.remote_internal_renderer_diagnostics(&hidden, 13);
        assert_eq!(hidden_renderers.len(), 1);
        assert_eq!(hidden_renderers[0].mode, "suspended");
        session.internal_ui.set_visible(surface, true);
        assert_eq!(
            session
                .remote_internal_application_diagnostics(&windows)
                .len(),
            1
        );
        session.internal_ui.remove(surface);
        assert!(
            session
                .remote_internal_application_diagnostics(&windows)
                .is_empty()
        );
    }

    #[test]
    fn internal_application_has_canonical_window_lifecycle() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let surface = session.internal_ui.insert(
            InternalWindowTestApp,
            crate::session::InternalSurfacePlacement {
                role: crate::session::InternalSurfaceRole::Application,
                geometry: (80, 90, 640, 480),
                output: Some("file-test".into()),
            },
            1.0,
        );

        let window = session.register_internal_application(surface).unwrap();
        let snapshot = session
            .protocol_windows()
            .into_iter()
            .find(|candidate| candidate.id.0 == window.0)
            .unwrap();
        assert_eq!(snapshot.title, "Codex — Nickel");
        assert_eq!(snapshot.application_id, "nickel-codex");
        let geometry = snapshot.geometry.unwrap();
        assert_eq!((geometry.x, geometry.y), (80, 90));
        assert!(session.workspaces.is_visible(&window));

        session.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::SwitchNext);
        assert!(session.task_switcher.candidates().contains(&window));

        session.minimize_window(window);
        assert!(!session.internal_ui.is_visible(surface));
        assert!(
            session
                .protocol_windows()
                .iter()
                .any(|entry| entry.id.0 == window.0 && entry.minimized)
        );

        session.activate_window(window);
        assert!(session.internal_ui.is_visible(surface));
        assert_eq!(session.internal_ui.focused(), Some(surface));

        let restored = session.internal_ui.placement(surface).cloned().unwrap();
        session.maximize_window(window);
        assert!(session.internal_maximized_restore.contains_key(&window));
        assert_ne!(session.internal_ui.placement(surface), Some(&restored));
        session.maximize_window(window);
        assert!(!session.internal_maximized_restore.contains_key(&window));
        assert_eq!(session.internal_ui.placement(surface), Some(&restored));

        session.close_window(window);
        assert!(!session.windows.contains(window));
        assert!(session.internal_ui.placement(surface).is_none());
    }

    #[test]
    #[ignore = "native XWayland acceptance: requires Xwayland and a writable XDG_RUNTIME_DIR; run alone"]
    fn native_x11_launch_acknowledgement_rejects_forged_pid() {
        use smithay::reexports::x11rb::{
            connection::Connection,
            protocol::xproto::{AtomEnum, ConnectionExt, CreateWindowAux, PropMode, WindowClass},
            wrapper::ConnectionExt as _,
        };
        struct FixtureEnvironment(Option<std::ffi::OsString>);
        impl Drop for FixtureEnvironment {
            fn drop(&mut self) {
                // SAFETY: this opt-in native test runs alone, like XWayland startup.
                unsafe {
                    match self.0.take() {
                        Some(display) => std::env::set_var("DISPLAY", display),
                        None => std::env::remove_var("DISPLAY"),
                    }
                }
            }
        }
        struct OwnedProcess(std::process::Child);
        impl Drop for OwnedProcess {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let _environment = FixtureEnvironment(std::env::var_os("DISPLAY"));
        let (mut event_loop, mut session) = internal_shell_test_session();
        session.start_xwayland();
        let deadline = Instant::now() + Duration::from_secs(15);
        while session.xwm.is_none() {
            event_loop
                .dispatch(Duration::from_millis(10), &mut session)
                .unwrap();
            assert!(Instant::now() < deadline, "owned XWayland did not start");
        }
        let child = OwnedProcess(
            std::process::Command::new("/usr/bin/sleep")
                .arg("30")
                .spawn()
                .unwrap(),
        );
        let forged_pid = child.0.id();
        session
            .pending_launch_observations
            .push(PendingLaunchObservation {
                generation: 93,
                root_pid: forged_pid,
                root_start_time: super::linux_process_start_time(forged_pid).unwrap(),
                registered_at: Instant::now(),
                deadline: Duration::from_secs(15),
            });
        let display = format!(":{}", session.xwayland_display.unwrap());
        let (stop, stopped) = std::sync::mpsc::channel();
        let client = std::thread::spawn(move || {
            let (connection, screen) = smithay::reexports::x11rb::connect(Some(&display)).unwrap();
            let root = &connection.setup().roots[screen];
            let window = connection.generate_id().unwrap();
            connection
                .create_window(
                    0,
                    window,
                    root.root,
                    20,
                    20,
                    200,
                    100,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    0,
                    &CreateWindowAux::new().background_pixel(root.white_pixel),
                )
                .unwrap();
            let pid_atom = connection
                .intern_atom(false, b"_NET_WM_PID")
                .unwrap()
                .reply()
                .unwrap()
                .atom;
            connection
                .change_property32(
                    PropMode::REPLACE,
                    window,
                    pid_atom,
                    AtomEnum::CARDINAL,
                    &[forged_pid],
                )
                .unwrap();
            connection
                .change_property8(
                    PropMode::REPLACE,
                    window,
                    AtomEnum::WM_NAME,
                    AtomEnum::STRING,
                    b"Forged launch acknowledgement fixture",
                )
                .unwrap();
            connection.map_window(window).unwrap();
            connection.flush().unwrap();
            let _ = stopped.recv_timeout(Duration::from_secs(20));
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        let id = loop {
            event_loop
                .dispatch(Duration::from_millis(10), &mut session)
                .unwrap();
            if let Some(id) = session.x11_windows.values().copied().find(|id| {
                session
                    .remote_window_identities
                    .get(id)
                    .and_then(|identity| identity.current_process_id())
                    == Some(std::process::id())
                    && session.window_for_registry_id(*id).is_some()
            }) {
                break id;
            }
            assert!(
                Instant::now() < deadline,
                "mapped X11 owner was not verified through XRes"
            );
        };
        let window = session.window_for_registry_id(id).unwrap();
        assert_eq!(window.x11_surface().unwrap().pid(), Some(forged_pid));
        assert_eq!(
            session.pending_launch_observations.len(),
            1,
            "forged PID must not acknowledge the unrelated child"
        );
        // The same production observation accepts the actual connection owner.
        session.pending_launch_observations[0].root_pid = std::process::id();
        session.pending_launch_observations[0].root_start_time =
            super::linux_process_start_time(std::process::id()).unwrap();
        session.observe_pending_launch_window(id);
        assert!(session.pending_launch_observations.is_empty());
        let _ = stop.send(());
        client.join().unwrap();
    }

    #[test]
    #[ignore = "native Wayland acceptance: requires /usr/bin/zenity and a writable XDG_RUNTIME_DIR"]
    fn native_launch_acknowledgement_waits_for_mapped_verified_window() {
        use crate::session::remote_identity::{ProcessIdentity, WindowIdentity};
        use crate::session::window_registry::WindowId;
        struct OwnedDialog(std::process::Child);
        impl Drop for OwnedDialog {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (mut event_loop, mut session) = internal_shell_test_session();
        // Deliver the identity result explicitly to exercise both orderings.
        session.remote_identity_worker = None;
        let config = tempfile::tempdir().unwrap();
        let child = OwnedDialog(
            std::process::Command::new("/usr/bin/zenity")
                .args([
                    "--info",
                    "--title=Launch acknowledgement native fixture",
                    "--text=Native launch fixture",
                ])
                .env("WAYLAND_DISPLAY", &session.socket_name)
                .env("GDK_BACKEND", "wayland")
                .env("GSK_RENDERER", "cairo")
                .env("XDG_CONFIG_HOME", config.path())
                .env_remove("DISPLAY")
                .env_remove("NICKEL_SESSION_TOKEN")
                .env_remove("NICKEL_SESSION_CONTROL")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        let id = loop {
            event_loop
                .dispatch(Duration::from_millis(10), &mut session)
                .unwrap();
            if let Some(id) = session
                .protocol_windows()
                .iter()
                .find(|window| window.title == "Launch acknowledgement native fixture")
                .map(|window| WindowId(window.id.0))
                .filter(|id| session.window_for_registry_id(*id).is_some())
            {
                break id;
            }
            assert!(Instant::now() < deadline, "native dialog did not map");
        };
        session
            .pending_launch_observations
            .push(PendingLaunchObservation {
                generation: 91,
                root_pid: child.0.id(),
                root_start_time: super::linux_process_start_time(child.0.id()).unwrap(),
                registered_at: Instant::now(),
                deadline: Duration::from_secs(2),
            });
        let subscriber_path = config.path().join("launch-subscriber.sock");
        let subscriber = std::os::unix::net::UnixDatagram::bind(&subscriber_path).unwrap();
        subscriber.set_nonblocking(true).unwrap();
        session.launcher_subscribers.push(subscriber_path);
        let no_acknowledgement = || {
            let mut bytes = [0; 2048];
            assert_eq!(
                subscriber.recv(&mut bytes).unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock,
                "ineligible or duplicate observation must not emit launch feedback"
            );
        };
        session.observe_pending_launch_window(id);
        no_acknowledgement();
        assert_eq!(
            session.pending_launch_observations.len(),
            1,
            "mapped window without verified identity must wait"
        );
        let window = session.window_for_registry_id(id).unwrap();
        let location = session.space.element_location(&window).unwrap();
        session.space.unmap_elem(&window);
        session.remote_window_identities.insert(
            id,
            WindowIdentity::Verified(ProcessIdentity::inspect(child.0.id()).unwrap()),
        );
        session.observe_pending_launch_window(id);
        assert_eq!(
            session.pending_launch_observations.len(),
            1,
            "verified but unmapped window must wait"
        );
        no_acknowledgement();
        session.space.map_element(window, location, false);
        session.remote_window_identities.insert(
            id,
            WindowIdentity::Verified(ProcessIdentity::inspect(std::process::id()).unwrap()),
        );
        session.observe_pending_launch_window(id);
        assert_eq!(
            session.pending_launch_observations.len(),
            1,
            "an unrelated verified owner cannot satisfy the launched child's lineage"
        );
        no_acknowledgement();
        session.remote_window_identities.insert(
            id,
            WindowIdentity::Verified(ProcessIdentity::inspect(child.0.id()).unwrap()),
        );
        session.observe_pending_launch_window(id);
        assert!(
            session.pending_launch_observations.is_empty(),
            "mapped verified process should acknowledge once"
        );
        let mut bytes = [0; 2048];
        let size = subscriber.recv(&mut bytes).unwrap();
        let response = nickel_session_protocol::decode::<nickel_session_protocol::ServerEnvelope>(
            &bytes[..size],
        )
        .unwrap();
        assert_eq!(response.request_id, 0);
        assert!(matches!(
            response.message,
            nickel_session_protocol::ServerMessage::Event(
                nickel_session_protocol::Event::PendingLaunchWindow {
                    generation: 91,
                    descendant: false,
                    observed_after_ms: 0..=2000,
                }
            )
        ));
        session.observe_pending_launch_window(id);
        assert!(session.pending_launch_observations.is_empty());
        no_acknowledgement();

        let slow_path = config.path().join("stalled-launch-subscriber.sock");
        let slow = std::os::unix::net::UnixDatagram::bind(&slow_path).unwrap();
        slow.set_nonblocking(true).unwrap();
        let mut fillers = vec![super::notification_socket().unwrap()];
        let mut filled = false;
        for _ in 0..10_000 {
            match fillers.last().unwrap().send_to(b"occupied", &slow_path) {
                Ok(_) => (),
                Err(error) => {
                    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
                    let fresh = super::notification_socket().unwrap();
                    match fresh.send_to(b"occupied", &slow_path) {
                        Ok(_) => fillers.push(fresh),
                        Err(error) => {
                            assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
                            filled = true;
                            break;
                        }
                    }
                }
            }
        }
        assert!(filled, "fixture must actually saturate the receiver queue");
        session.launcher_subscribers.insert(0, slow_path.clone());
        // Bound failure time if blocking sends regress: release queue capacity
        // after one second, then fail the latency assertion rather than hanging.
        let recovery = slow.try_clone().unwrap();
        let (cancel, wait) = std::sync::mpsc::channel();
        let recovery_thread = std::thread::spawn(move || {
            if wait.recv_timeout(Duration::from_secs(1)).is_err() {
                let mut bytes = [0; 2048];
                while recovery.recv(&mut bytes).is_ok() {}
            }
        });
        let started = Instant::now();
        session.notify_pending_launch_expired(92);
        let elapsed = started.elapsed();
        let _ = cancel.send(());
        recovery_thread.join().unwrap();
        assert!(
            elapsed < Duration::from_millis(750),
            "stalled subscriber delayed owner by {elapsed:?}"
        );
        assert!(!session.launcher_subscribers.contains(&slow_path));
        let size = subscriber.recv(&mut bytes).unwrap();
        let delivered = nickel_session_protocol::decode::<nickel_session_protocol::ServerEnvelope>(
            &bytes[..size],
        )
        .unwrap();
        assert!(
            matches!(
                delivered.message,
                nickel_session_protocol::ServerMessage::Event(
                    nickel_session_protocol::Event::PendingLaunchExpired { generation: 92 }
                )
            ),
            "healthy subscriber must still receive the event"
        );
    }

    #[test]
    fn late_window_callback_leaves_expiry_owned_by_the_registered_timer() {
        let now = Instant::now();
        let pending = PendingLaunchObservation {
            generation: 7,
            root_pid: std::process::id(),
            root_start_time: 1,
            registered_at: now - Duration::from_millis(101),
            deadline: Duration::from_millis(100),
        };

        assert_eq!(
            pending_launch_window_disposition(&pending, now, std::process::id()),
            PendingLaunchWindowDisposition::AwaitExpiry
        );
    }

    #[test]
    fn shell_behavior_values_are_typed_and_desktop_counts_are_validated() {
        let mut settings = nickel_core::shell_settings::ShellSettings::default();
        assert_eq!(
            shell_behavior_value(&settings, ShellBehaviorSetting::BarDisplayScope),
            ShellBehaviorValue::Toggle(true)
        );
        assert!(
            apply_shell_behavior_value(
                &mut settings,
                ShellBehaviorSetting::BarWindowScope,
                ShellBehaviorValue::Toggle(false),
            )
            .is_ok()
        );
        assert!(!settings.all_windows_on_every_bar);
        assert!(
            apply_shell_behavior_value(
                &mut settings,
                ShellBehaviorSetting::DesktopCount,
                ShellBehaviorValue::Count(0),
            )
            .is_err()
        );
        assert!(
            apply_shell_behavior_value(
                &mut settings,
                ShellBehaviorSetting::BarDisplayScope,
                ShellBehaviorValue::Count(2),
            )
            .is_err()
        );
    }

    #[test]
    fn shell_behavior_transactions_reject_stale_topology_and_concurrent_writers() {
        let current = nickel_core::shell_settings::ShellSettings::default();
        let mut transaction = ShellBehaviorTransaction {
            setting: ShellBehaviorSetting::BarDisplayScope,
            prior: ShellBehaviorValue::Toggle(true),
            requested: ShellBehaviorValue::Toggle(false),
            topology_generation: 4,
        };
        assert_eq!(
            prepare_shell_behavior_update(&current, 5, &transaction),
            Err("stale output topology generation")
        );
        transaction.topology_generation = 5;
        transaction.prior = ShellBehaviorValue::Toggle(false);
        assert_eq!(
            prepare_shell_behavior_update(&current, 5, &transaction),
            Err("shell setting changed before this transaction was applied")
        );
        transaction.prior = ShellBehaviorValue::Toggle(true);
        let requested = prepare_shell_behavior_update(&current, 5, &transaction).unwrap();
        assert!(!requested.bar_on_all_displays);
        assert!(
            current.bar_on_all_displays,
            "planning must not mutate authority"
        );
    }

    #[test]
    fn pointer_identity_churn_returns_all_collections_to_baseline() {
        let mut hints = HashMap::new();
        let mut locks = HashSet::new();
        let mut origins = HashMap::new();

        for surface in 0_u16..300 {
            hints.insert(surface, Point::from((1.0, 2.0)));
            locks.insert(surface);
            origins.insert(surface, Point::from((3.0, 4.0)));
            let restored = retire_pointer_surface(&mut hints, &mut locks, &mut origins, &surface);
            assert_eq!(restored, Some(Point::from((4.0, 6.0))));
            assert!(hints.is_empty());
            assert!(locks.is_empty());
            assert!(origins.is_empty());
        }

        assert_eq!(hints.capacity(), 0);
        assert_eq!(locks.capacity(), 0);
        assert_eq!(origins.capacity(), 0);
        hints.insert(301, Point::from((5.0, 6.0)));
        assert_eq!(hints.len(), 1, "new hints remain admissible after churn");
    }

    #[test]
    fn closed_displaced_windows_do_not_preserve_output_history() {
        let mut outputs = HashMap::new();
        for index in 0_u64..300 {
            let id = super::WindowId(index + 1);
            outputs.insert(
                format!("virtual-{index}"),
                vec![DisplacedWindow {
                    id,
                    relative_location: Point::from((10, 20)),
                    rescue_location: Point::from((30, 40)),
                }],
            );
            retire_displaced_window(&mut outputs, id);
            assert!(outputs.is_empty());
        }
        assert_eq!(outputs.capacity(), 0);
    }

    #[test]
    fn displaced_mapped_minimized_and_hidden_windows_retire_independently() {
        let mapped = super::WindowId(1);
        let minimized = super::WindowId(2);
        let hidden = super::WindowId(3);
        let displaced = |id| DisplacedWindow {
            id,
            relative_location: Point::from((10, 20)),
            rescue_location: Point::from((30, 40)),
        };
        let mut outputs = HashMap::from([(
            "removed-output".to_owned(),
            vec![displaced(mapped), displaced(minimized), displaced(hidden)],
        )]);

        retire_displaced_window(&mut outputs, mapped);
        assert_eq!(outputs["removed-output"].len(), 2);
        retire_displaced_window(&mut outputs, minimized);
        assert_eq!(outputs["removed-output"].len(), 1);
        retire_displaced_window(&mut outputs, hidden);
        assert!(outputs.is_empty());
        assert_eq!(outputs.capacity(), 0);
    }

    #[test]
    fn destroying_a_shell_surface_retires_only_its_registration() {
        let retired = ObjectId::null();
        let retained = retired.clone();
        let mut registrations = vec![RegisteredShellRole {
            role: ShellRole::Launcher,
            output: None,
            surface: retired.clone(),
        }];
        retire_shell_surface(&mut registrations, &retired);
        assert!(registrations.is_empty());
        assert_eq!(registrations.capacity(), 0);

        // Re-registration after independent destruction is not blocked by a
        // historical singleton slot.
        registrations.push(RegisteredShellRole {
            role: ShellRole::Launcher,
            output: None,
            surface: retained,
        });
        assert_eq!(registrations.len(), 1);
    }

    #[test]
    fn live_surface_role_transitions_invalidate_historical_readiness() {
        let surface = ObjectId::null();
        let registrations = vec![RegisteredShellRole {
            role: ShellRole::Launcher,
            output: None,
            surface: surface.clone(),
        }];

        assert!(!shell_registration_role_changed(
            &registrations,
            &surface,
            Some(ShellRole::Launcher)
        ));
        assert!(shell_registration_role_changed(
            &registrations,
            &surface,
            Some(ShellRole::ControlCenter)
        ));
        assert!(shell_registration_role_changed(
            &registrations,
            &surface,
            None
        ));
    }

    #[test]
    fn disconnected_output_roles_are_dormant_until_the_output_returns() {
        let registrations = [
            RegisteredShellRole {
                role: ShellRole::Desktop,
                output: Some("winit".into()),
                surface: ObjectId::null(),
            },
            RegisteredShellRole {
                role: ShellRole::Desktop,
                output: Some("DP-test".into()),
                surface: ObjectId::null(),
            },
            RegisteredShellRole {
                role: ShellRole::Panel,
                output: Some("DP-test".into()),
                surface: ObjectId::null(),
            },
            RegisteredShellRole {
                role: ShellRole::Lock,
                output: Some("DP-test".into()),
                surface: ObjectId::null(),
            },
        ];
        let connected = HashSet::from(["winit".to_owned(), "DP-test".to_owned()]);
        assert!(registrations.iter().all(|registration| {
            shell_registration_is_active(registration, &connected, &connected)
        }));

        let after_disconnect = HashSet::from(["winit".to_owned()]);
        assert!(shell_registration_is_active(
            &registrations[0],
            &after_disconnect,
            &after_disconnect,
        ));
        assert!(registrations[1..].iter().all(|registration| {
            !shell_registration_is_active(registration, &after_disconnect, &after_disconnect)
        }));

        // Keeping the slots dormant preserves the shell's bounded reconnect
        // grace: the same native surfaces become authoritative again if the
        // named output returns instead of requiring an app-id transition.
        assert!(registrations.iter().all(|registration| {
            shell_registration_is_active(registration, &connected, &connected)
        }));
    }

    #[test]
    fn panel_registration_tracks_the_configured_output_set() {
        let registration = RegisteredShellRole {
            role: ShellRole::Panel,
            output: Some("DP-test".into()),
            surface: ObjectId::null(),
        };
        let connected = HashSet::from(["winit".to_owned(), "DP-test".to_owned()]);
        let primary_only = HashSet::from(["winit".to_owned()]);
        assert!(!shell_registration_is_active(
            &registration,
            &connected,
            &primary_only,
        ));
        assert!(shell_registration_is_active(
            &registration,
            &connected,
            &connected,
        ));
    }

    #[test]
    fn locked_test_output_disconnect_projects_readiness_to_live_topology() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        session
            .apply_test_output(TestOutput::Connect {
                name: "DP-test".into(),
                logical_width: 640,
                logical_height: 480,
                scale_120: 120,
                transform: OutputTransform::Normal,
            })
            .unwrap();
        for role in [ShellRole::Desktop, ShellRole::Panel, ShellRole::Lock] {
            session
                .registered_shell_role_slots
                .push(RegisteredShellRole {
                    role,
                    output: Some("DP-test".into()),
                    surface: ObjectId::null(),
                });
        }
        let connected = session.protocol_shell_readiness();
        assert_eq!((connected.outputs, connected.desktops), (1, 1));
        assert_eq!((connected.panels, connected.locks), (1, 1));

        session.locked = true;
        session
            .apply_test_output(TestOutput::Disconnect {
                name: "DP-test".into(),
            })
            .unwrap();

        let disconnected = session.protocol_shell_readiness();
        assert_eq!((disconnected.outputs, disconnected.desktops), (0, 0));
        assert_eq!((disconnected.panels, disconnected.locks), (0, 0));
        assert!(session.registered_shell_role_slots.len() == 3);
    }

    #[test]
    fn touch_output_hint_uses_named_logical_geometry_and_never_falls_back_after_removal() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        for (name, scale_120) in [("fallback", 120), ("touchscreen", 180)] {
            session
                .apply_test_output(TestOutput::Connect {
                    name: name.into(),
                    logical_width: 800,
                    logical_height: 600,
                    scale_120,
                    transform: OutputTransform::Normal,
                })
                .unwrap();
        }
        let output = session
            .space
            .outputs()
            .find(|output| output.name() == "touchscreen")
            .unwrap()
            .clone();
        session.space.map_output(&output, (-800, -120));
        let geometry = session.touch_output_geometry(Some("touchscreen")).unwrap();
        assert_eq!(geometry.loc, (-800, -120).into());
        assert_eq!(geometry.size, (800, 600).into());
        assert_ne!(session.touch_output_geometry(None).unwrap(), geometry);
        session
            .apply_test_output(TestOutput::Disconnect {
                name: "touchscreen".into(),
            })
            .unwrap();
        assert!(session.touch_output_geometry(Some("touchscreen")).is_none());
        assert!(session.touch_output_geometry(None).is_some());
    }

    #[test]
    fn keyboard_reservation_resize_and_close_change_only_the_owner_output() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        for name in ["keyboard-owner", "unaffected"] {
            session
                .apply_test_output(TestOutput::Connect {
                    name: name.into(),
                    logical_width: 1280,
                    logical_height: 720,
                    scale_120: 120,
                    transform: OutputTransform::Normal,
                })
                .unwrap();
        }
        session.configure_on_screen_keyboard(true, true, 0, false, false, 368);
        let outputs = session.protocol_outputs();
        let owner = session.on_screen_keyboard.output_name.clone().unwrap();
        for output in &outputs {
            assert_eq!(
                output.work_area.height,
                if output.name == owner { 352 } else { 664 }
            );
        }
        session.configure_on_screen_keyboard(true, true, 0, false, true, 280);
        assert_eq!(
            session.on_screen_keyboard.output_name.as_deref(),
            Some(owner.as_str())
        );
        for output in session.protocol_outputs() {
            assert_eq!(
                output.work_area.height,
                if output.name == owner { 440 } else { 664 }
            );
            assert_eq!(
                output.work_area.y,
                if output.name == owner { 280 } else { 0 }
            );
        }
        session.configure_on_screen_keyboard(true, false, 0, false, true, 280);
        assert!(
            session
                .protocol_outputs()
                .iter()
                .all(|output| output.work_area.height == 664)
        );
    }

    #[test]
    fn output_identification_local_replacement_survives_stale_expiry_and_exhaustion() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let first = session.start_output_identification(None).unwrap();
        let first_timer = session.identify_outputs_timer.unwrap();
        session.begin_output_identification();
        assert_ne!(session.identify_outputs_timer, Some(first_timer));
        let replacement = session.identify_outputs_generation;
        assert!(replacement > first);
        assert!(!session.expire_output_identification(first));
        assert!(session.identify_outputs_until.is_some());
        #[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
        {
            let output = session.space.outputs().next().unwrap().clone();
            assert_eq!(
                session.output_identification_index(&output),
                Some((replacement, 0))
            );
            session.locked = true;
            assert_eq!(session.output_identification_index(&output), None);
            session.locked = false;
        }
        session.identify_outputs_generation = u64::MAX;
        let until = session.identify_outputs_until;
        let timer = session.identify_outputs_timer;
        assert!(session.start_output_identification(None).is_err());
        assert_eq!(session.identify_outputs_timer, timer);
        assert_eq!(session.identify_outputs_generation, u64::MAX);
        assert_eq!(session.identify_outputs_until, until);
        assert!(session.expire_output_identification(u64::MAX));
        assert!(session.identify_outputs_until.is_none());
        assert!(session.identify_outputs_timer.is_none());
    }

    #[test]
    fn only_the_latest_output_identification_generation_may_expire() {
        assert!(identification_expiry_is_current(7, 7));
        assert!(!identification_expiry_is_current(8, 7));
        assert!(!identification_expiry_is_current(7, 8));
    }

    #[test]
    fn connection_cleanup_precedes_first_request_from_full_ordinary_queue() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let wake = session.remote_cleanup_wake.clone();
        let control = session.remote_control.control();
        let lease = {
            let mut authority = control.lock().unwrap();
            authority.set_enabled(true);
            let client = authority.connect_identity("Cleanup priority test").unwrap();
            let then = Instant::now() - Duration::from_secs(61);
            let watch = authority
                .reserve_connection_watch(&client.client_id, &client.token, then)
                .unwrap();
            authority
                .activate_connection_watch(&client.client_id, &client.token, watch, false, then)
                .unwrap();
            authority
                .leases_mut()
                .approve_local(
                    client.client_id.clone(),
                    nickel_remote_control::leases::ResourceScope::FullSession,
                    then,
                    None,
                    false,
                    false,
                )
                .unwrap()
        };
        let (sender, receiver) = smithay::reexports::calloop::channel::sync_channel(32);
        for sequence in 0..32 {
            assert!(
                sender
                    .try_send(super::RemoteDesktopRequest::NativeKeyboardState {
                        source: smithay::input::keyboard::KeyboardSource::new_focus_bound_auxiliary(
                        ),
                        sequence,
                        result: Err("stale native query".into()),
                    })
                    .is_ok()
            );
        }
        wake.notify();
        // Same production handler used by the registered ordinary channel. Do
        // not dispatch the separate wake source: priority must also hold here.
        session.handle_remote_desktop_request(receiver.try_recv().unwrap());
        assert!(!wake.take_pending());
        assert!(
            control
                .lock()
                .unwrap()
                .leases()
                .iter()
                .all(|entry| entry.id != lease)
        );
    }

    #[test]
    fn connection_cleanup_setup_failure_preserves_owner_fallback() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (event_loop, mut session) = internal_shell_test_session();
        let wake = super::NickelSession::register_remote_connection_cleanup_wake(
            &event_loop.handle(),
            Err(std::io::Error::other("descriptor unavailable")),
        );
        session.remote_cleanup_wake = wake.clone();
        wake.notify();
        assert!(wake.take_wake_failure());
        // Exercise the same owner drain used by the retained periodic timer.
        session.service_remote_connection_cleanup();
        assert!(!wake.take_pending());
        wake.notify();
        assert!(wake.take_wake_failure());
        session.service_remote_connection_cleanup();
        assert!(!wake.take_pending());
    }

    #[test]
    fn connection_cleanup_eventfd_wakes_idle_production_owner() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (mut event_loop, mut session) = internal_shell_test_session();
        let wake = session.remote_cleanup_wake.clone();
        wake.notify();
        event_loop
            .dispatch(Duration::from_millis(50), &mut session)
            .unwrap();
        assert!(!wake.take_pending());
    }

    static PREVIEW_SESSION_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn preview_test_session() -> (
        EventLoop<'static, super::NickelSession>,
        super::NickelSession,
    ) {
        let mut event_loop = EventLoop::try_new().unwrap();
        let display = Display::new().unwrap();
        let session = super::NickelSession::new(&mut event_loop, display, true);
        (event_loop, session)
    }

    struct IdleInternalHost;

    #[test]
    fn native_media_notification_bypasses_legacy_subscribers_and_preserves_focus() {
        use nickel_session_protocol::ConsumerControl;
        struct RecordingMediaHost(std::sync::Mutex<Vec<ConsumerControl>>);
        impl SessionHost for RecordingMediaHost {
            fn dispatch(&self, _: ShellCommand) -> Result<(), SessionRequestError> {
                Ok(())
            }
            fn consumer_control(&self, control: ConsumerControl) -> bool {
                self.0.lock().unwrap().push(control);
                true
            }
        }
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (mut event_loop, mut session) = preview_test_session();
        let (_system_tx, system_rx) = crate::platform::status_mailbox::channel();
        let host = Arc::new(RecordingMediaHost(std::sync::Mutex::new(Vec::new())));
        session
            .enable_internal_shell_with_system_updates(host.clone(), system_rx)
            .unwrap();
        let focus = session.seat.get_keyboard().unwrap().current_focus();
        session.notify_consumer_control(ConsumerControl::VolumeUp);
        // If native dispatch fell through to legacy delivery, this nonexistent
        // subscriber would be removed on send failure.
        let subscriber = std::path::PathBuf::from("/nonexistent-nickel-media-test/subscriber.sock");
        session.launcher_subscribers.push(subscriber.clone());
        for control in [
            ConsumerControl::VolumeDown,
            ConsumerControl::VolumeMute,
            ConsumerControl::PlayPause,
            ConsumerControl::Next,
        ] {
            session.notify_consumer_control(control);
        }
        assert_eq!(session.launcher_subscribers, vec![subscriber]);
        assert_eq!(
            *host.0.lock().unwrap(),
            vec![
                ConsumerControl::VolumeUp,
                ConsumerControl::VolumeDown,
                ConsumerControl::VolumeMute,
                ConsumerControl::PlayPause,
                ConsumerControl::Next
            ]
        );
        assert_eq!(session.seat.get_keyboard().unwrap().current_focus(), focus);
        use smithay::backend::input::KeyState;
        let before = host.0.lock().unwrap().len();
        session.consumer_control_key(ConsumerControl::VolumeUp, KeyState::Pressed);
        let old = session.held_consumer_controls[&ConsumerControl::VolumeUp].0;
        session.consumer_control_key(ConsumerControl::VolumeUp, KeyState::Pressed);
        assert_eq!(host.0.lock().unwrap().len(), before + 1);
        session.consumer_control_key(ConsumerControl::VolumeUp, KeyState::Released);
        session.consumer_control_key(ConsumerControl::VolumeUp, KeyState::Pressed);
        let current = session.held_consumer_controls[&ConsumerControl::VolumeUp].0;
        assert_ne!(old, current);
        assert!(!session.consumer_repeat_is_current(ConsumerControl::VolumeUp, old));
        assert!(session.consumer_repeat_is_current(ConsumerControl::VolumeUp, current));
        session.consumer_control_key(ConsumerControl::VolumeDown, KeyState::Pressed);
        assert!(session.consumer_repeat_is_current(ConsumerControl::VolumeUp, current));
        session.consumer_control_key(ConsumerControl::VolumeDown, KeyState::Released);

        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while host.0.lock().unwrap().len() == before + 3 && Instant::now() < deadline {
            event_loop
                .dispatch(Some(std::time::Duration::from_millis(10)), &mut session)
                .unwrap();
        }
        assert_eq!(
            host.0.lock().unwrap().len(),
            before + 4,
            "only the current hold repeats"
        );
        session.consumer_control_key(ConsumerControl::VolumeUp, KeyState::Released);
        assert!(session.held_consumer_controls.is_empty());
        let deadline = Instant::now() + std::time::Duration::from_millis(100);
        while Instant::now() < deadline {
            event_loop
                .dispatch(Some(std::time::Duration::from_millis(10)), &mut session)
                .unwrap();
        }
        assert_eq!(host.0.lock().unwrap().len(), before + 4);
        session.consumer_control_key(ConsumerControl::VolumeMute, KeyState::Pressed);
        assert!(
            session.held_consumer_controls[&ConsumerControl::VolumeMute]
                .1
                .is_none()
        );
        session.cancel_consumer_control_repeats();
        assert!(session.held_consumer_controls.is_empty());
    }

    impl SessionHost for IdleInternalHost {
        fn dispatch(&self, _command: ShellCommand) -> Result<(), SessionRequestError> {
            Ok(())
        }

        fn secure_storage_state(
            &self,
        ) -> Result<crate::platform::SecureStorageState, SessionRequestError> {
            Ok(crate::platform::SecureStorageState::Ready)
        }

        fn request_secure_storage_retry(&self) -> Result<(), SessionRequestError> {
            Ok(())
        }
    }

    #[test]
    fn unchanged_internal_desktop_has_no_sixty_hertz_poll_or_redraw_loop() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (mut event_loop, mut session) = preview_test_session();
        let (_system_tx, system_rx) = crate::platform::status_mailbox::channel();
        session
            .enable_internal_shell_with_system_updates(Arc::new(IdleInternalHost), system_rx)
            .expect("headless internal shell");

        // Consume the intentionally immediate initialization wakeup. The
        // stable shell then owns a real application deadline well beyond a
        // frame interval (keyboard discovery currently supplies the nearest).
        event_loop
            .dispatch(Duration::from_millis(25), &mut session)
            .unwrap();
        let settled = session.internal_shell_timer_counters();

        // Damage/state paths may redundantly ask to maintain the schedule.
        // Sixty such calls must keep the one existing one-shot instead of
        // manufacturing a 60 Hz timer or redraw stream.
        for _ in 0..60 {
            session.schedule_internal_shell_deadline();
        }
        let after_rearm = session.internal_shell_timer_counters();
        assert_eq!(after_rearm.armed, settled.armed);
        assert_eq!(after_rearm.polls, settled.polls);
        assert_eq!(after_rearm.redraw_requests, settled.redraw_requests);

        event_loop
            .dispatch(Duration::from_millis(75), &mut session)
            .unwrap();
        let after_idle = session.internal_shell_timer_counters();
        assert!(after_idle.polls.saturating_sub(settled.polls) <= 1);
        assert_eq!(after_idle.redraw_requests, settled.redraw_requests);
    }

    #[test]
    fn protocol_snapshot_change_wakes_the_internal_bar_projection() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (mut event_loop, mut session) = preview_test_session();
        session
            .apply_test_output(TestOutput::Connect {
                name: "test".into(),
                logical_width: 1280,
                logical_height: 720,
                scale_120: 120,
                transform: OutputTransform::Normal,
            })
            .unwrap();
        session
            .enable_internal_shell(Arc::new(IdleInternalHost))
            .expect("headless internal shell");
        event_loop
            .dispatch(Duration::from_millis(25), &mut session)
            .unwrap();
        let settled = session.internal_shell_timer_counters();

        session.notify_protocol_snapshot();

        let notified = session.internal_shell_timer_counters();
        assert_eq!(notified.armed, settled.armed + 1);
        assert_eq!(notified.cancelled, settled.cancelled + 1);
    }

    #[test]
    fn first_native_launcher_open_retains_search_focus_through_key_delivery() {
        first_native_launcher_open_accepts_typing(false);
    }

    #[test]
    fn first_native_panel_launcher_open_accepts_typing() {
        first_native_launcher_open_accepts_typing(true);
    }

    fn first_native_launcher_open_accepts_typing(pointer: bool) {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (mut event_loop, mut session) = internal_shell_test_session();
        if pointer {
            let panel = session
                .internal_shell
                .as_ref()
                .unwrap()
                .surface(crate::winit_shell::SurfaceRole::Panel, Some("file-test"))
                .unwrap()
                .id;
            let runtime = session.internal_shell_surfaces[&panel];
            let geometry = session.internal_ui.placement(runtime).unwrap().geometry;
            session
                .inject_test_input(nickel_session_protocol::TestInput::PointerMove {
                    x: geometry.0 + 20,
                    y: geometry.1 + 28,
                })
                .unwrap();
        }
        for state in [
            nickel_session_protocol::InputState::Pressed,
            nickel_session_protocol::InputState::Released,
        ] {
            let input = if pointer {
                nickel_session_protocol::TestInput::PointerButton {
                    button: nickel_session_protocol::TestPointerButton::Left,
                    state,
                }
            } else {
                nickel_session_protocol::TestInput::Key {
                    key: nickel_session_protocol::TestKey::LeftMeta,
                    state,
                }
            };
            session.inject_test_input(input).unwrap();
        }
        event_loop
            .dispatch(Duration::from_millis(25), &mut session)
            .unwrap();
        let shell = session.internal_shell.as_ref().unwrap();
        assert!(shell.launcher_visible());
        let launcher = shell
            .surface(crate::winit_shell::SurfaceRole::Launcher, None)
            .unwrap()
            .id;
        assert!(
            shell.focused_field_lease(launcher).is_some(),
            "first opening must focus search"
        );
        assert_eq!(
            session.internal_ui.focused(),
            session.internal_shell_surfaces.get(&launcher).copied()
        );
        for state in [
            nickel_session_protocol::InputState::Pressed,
            nickel_session_protocol::InputState::Released,
        ] {
            session
                .inject_test_input(nickel_session_protocol::TestInput::Key {
                    key: nickel_session_protocol::TestKey::A,
                    state,
                })
                .unwrap();
        }
        let shell = session.internal_shell.as_mut().unwrap();
        assert!(shell.focused_field_lease(launcher).is_some());
        assert!(
            shell
                .scene(launcher)
                .unwrap()
                .iter()
                .any(|command| matches!(
                    command, nickel_ui::backend::PaintCommand::Text { text, .. } if text == "a"
                )),
            "typed character must reach the launcher scene"
        );
    }

    #[test]
    fn internal_launcher_owns_keyboard_until_it_is_hidden() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        session
            .apply_test_output(TestOutput::Connect {
                name: "test".into(),
                logical_width: 1280,
                logical_height: 720,
                scale_120: 120,
                transform: OutputTransform::Normal,
            })
            .unwrap();
        session
            .enable_internal_shell(Arc::new(IdleInternalHost))
            .expect("headless internal shell");
        let application = session.internal_ui.insert(
            InternalWindowTestApp,
            crate::session::InternalSurfacePlacement {
                role: crate::session::InternalSurfaceRole::Application,
                geometry: (80, 90, 640, 480),
                output: Some("test".into()),
            },
            1.0,
        );
        session.register_internal_application(application).unwrap();

        assert!(session.toggle_internal_launcher());
        let launcher = session
            .internal_shell
            .as_ref()
            .unwrap()
            .surfaces()
            .iter()
            .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Launcher)
            .and_then(|surface| session.internal_shell_surfaces.get(&surface.id))
            .copied()
            .unwrap();
        assert_eq!(session.internal_ui.focused(), Some(launcher));
        assert_eq!(session.seat.get_keyboard().unwrap().current_focus(), None);

        assert!(session.toggle_internal_launcher());
        assert_eq!(session.internal_ui.focused(), Some(application));
    }

    #[test]
    fn shell_diagnostics_follow_owner_scene_visibility_and_exclude_lock() {
        use nickel_remote_control::diagnostics::ShellDiagnosticRole;
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let (records, truncated) = session.remote_shell_surface_diagnostics();
        assert!(!truncated);
        assert!(
            records
                .iter()
                .any(|record| matches!(record.role, ShellDiagnosticRole::Desktop))
        );
        assert!(
            records
                .iter()
                .any(|record| matches!(record.role, ShellDiagnosticRole::Panel))
        );
        assert!(
            !records
                .iter()
                .any(|record| matches!(record.role, ShellDiagnosticRole::Launcher))
        );
        session.set_launcher_visible(true);
        let (shown, _) = session.remote_shell_surface_diagnostics();
        let launcher = shown
            .iter()
            .find(|record| matches!(record.role, ShellDiagnosticRole::Launcher))
            .unwrap();
        let shell = session.internal_shell.as_ref().unwrap();
        let owner = shell
            .surfaces()
            .iter()
            .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Launcher)
            .unwrap();
        let owner_id = owner.id;
        let (tree_generation, semantics) = shell.bounded_shell_semantics(owner_id).unwrap();
        assert!(tree_generation > 0);
        assert!(semantics.iter().any(|node| node.focused));
        assert!(
            semantics
                .iter()
                .any(|node| node.name.as_deref() == Some("Home"))
        );
        assert_eq!(
            shell.bounded_shell_semantics(owner_id).unwrap().0,
            tree_generation,
            "observation must not rebuild or focus another viewport"
        );
        let runtime = session.internal_shell_surfaces[&owner.id];
        let placement = session.internal_ui.placement(runtime).unwrap();
        assert_eq!(launcher.generation, runtime.snapshot_token());
        assert_eq!(launcher.scene_generation, owner.scene_generation);
        assert_eq!(
            launcher.geometry,
            [
                i64::from(placement.geometry.0),
                i64::from(placement.geometry.1),
                i64::from(placement.geometry.2),
                i64::from(placement.geometry.3)
            ]
        );
        assert!(launcher.keyboard_focused);
        let input = session.remote_input_diagnostic(&[], &[], 41, 42);
        let recipient = input.keyboard.unwrap();
        assert!(recipient.focused_window.is_none());
        let focused_surface = recipient.focused_surface.unwrap();
        assert_eq!(focused_surface.id, launcher.id);
        assert_eq!(focused_surface.generation, launcher.generation);
        let placement = placement.clone();
        session
            .internal_ui
            .configure_surface(runtime, placement, 1.5);
        let (scaled, _) = session.remote_shell_surface_diagnostics();
        let scaled = scaled
            .iter()
            .find(|record| record.id == launcher.id)
            .unwrap();
        assert_eq!(scaled.scale_factor, 1.5);
        assert!(scaled.redraw_pending);
        assert_eq!(scaled.generation, launcher.generation);
        let renderer = session
            .remote_shell_renderer_diagnostics(42)
            .into_iter()
            .find(|record| record.surface == launcher.id)
            .unwrap();
        let actual = session.internal_ui.renderer_diagnostics(runtime).unwrap();
        assert_eq!(renderer.surface_generation, launcher.generation);
        assert_eq!(renderer.observed_at_us, 42);
        assert_eq!(renderer.gpu_frames, actual.gpu_frames);
        assert_eq!(renderer.fallback_frames, actual.fallback_frames);
        assert_eq!(
            renderer.software_frame_bytes,
            actual.software_frame_bytes as u64
        );
        session.set_launcher_visible(false);
        assert!(
            session
                .internal_shell
                .as_ref()
                .unwrap()
                .bounded_shell_semantics(owner_id)
                .is_err()
        );
        assert!(
            session
                .remote_shell_renderer_diagnostics(43)
                .iter()
                .all(|record| record.surface != launcher.id)
        );
        assert!(
            !session
                .remote_shell_surface_diagnostics()
                .0
                .iter()
                .any(|record| matches!(record.role, ShellDiagnosticRole::Launcher))
        );
        session.locked = true;
        assert!(session.remote_shell_surface_diagnostics().0.is_empty());
        assert!(session.remote_shell_renderer_diagnostics(44).is_empty());
        let input = session.remote_input_diagnostic(&[], &[], 45, 46);
        assert!(input.keyboard.is_none());
        assert!(input.pointer.is_none());
        assert!(input.pointer_hit_test.is_none());
    }

    #[test]
    fn remote_panel_effect_is_staged_without_changing_local_host_dispatch() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let shell = session.internal_shell.as_mut().unwrap();
        assert!(!shell.launcher_visible());
        let commands = shell
            .shell_mut()
            .stage_remote_shell_effect(
                crate::live_shell::remote_semantics::RemoteShellEffect::Panel(
                    crate::live_shell::PanelAction::Launcher,
                    Some("file-test".into()),
                ),
            )
            .unwrap();
        assert!(matches!(
            commands.as_slice(),
            [crate::platform::ShellCommand::Show]
        ));
        assert!(
            !shell.launcher_visible(),
            "staging must not apply visibility or defer unguarded work"
        );
        session.set_launcher_visible(true);
        assert!(session.internal_shell.as_ref().unwrap().launcher_visible());
    }

    #[test]
    fn ordinary_shell_semantic_mutation_updates_real_launcher_and_rejects_stale_tree() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        session.set_launcher_visible(true);
        let shell = session.internal_shell.as_mut().unwrap();
        let id = shell
            .surfaces()
            .iter()
            .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Launcher)
            .unwrap()
            .id;
        let (generation, nodes) = shell.bounded_shell_semantics(id).unwrap();
        let ordinal = nodes
            .iter()
            .position(|node| node.role == Some(nickel_ui::SemanticRole::TextField))
            .unwrap();
        let action = |text: &str| {
            nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Text(text.into()))
        };
        let outcome = shell
            .perform_bounded_shell_action(
                id,
                generation,
                ordinal,
                action("semantic launcher query"),
                2048,
            )
            .unwrap();
        assert!(outcome.effects.is_empty());
        assert!(outcome.host.semantic_failures.is_empty());
        let (next, nodes) = shell.bounded_shell_semantics(id).unwrap();
        assert_ne!(next, generation);
        assert!(nodes.iter().any(|node| matches!(&node.value, Some(nickel_ui::SemanticValueSnapshot::Text(text)) if text == "semantic launcher query")));
        assert!(
            shell
                .perform_bounded_shell_action(id, generation, ordinal, action("stale query"), 2048)
                .is_err()
        );
        session.set_launcher_visible(false);
        assert!(
            session
                .internal_shell
                .as_mut()
                .unwrap()
                .perform_bounded_shell_action(id, next, ordinal, action("hidden query"), 2048)
                .is_err()
        );
    }

    #[test]
    fn launcher_protocol_visibility_updates_hosted_scene_and_restores_focus() {
        use nickel_session_protocol::Command;
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let application = session.internal_ui.insert(
            InternalWindowTestApp,
            crate::session::InternalSurfacePlacement {
                role: crate::session::InternalSurfaceRole::Application,
                geometry: (80, 90, 640, 480),
                output: Some("file-test".into()),
            },
            1.0,
        );
        session.register_internal_application(application).unwrap();
        for command in [
            Command::SetLauncherVisible { visible: true },
            Command::SetLauncherVisible { visible: true },
        ] {
            assert!(matches!(
                session.handle_protocol_command(command, None, 0),
                ServerMessage::Ack
            ));
            assert!(session.internal_shell.as_ref().unwrap().launcher_visible());
            assert!(session.launcher_visibility.is_visible());
            assert!(
                session
                    .protocol_shell_surfaces()
                    .iter()
                    .any(
                        |surface| surface.role == ShellRole::Launcher && surface.geometry.is_some()
                    )
            );
            assert_ne!(session.internal_ui.focused(), Some(application));
        }
        for command in [
            Command::SetLauncherVisible { visible: false },
            Command::SetLauncherVisible { visible: false },
        ] {
            assert!(matches!(
                session.handle_protocol_command(command, None, 0),
                ServerMessage::Ack
            ));
            assert!(!session.internal_shell.as_ref().unwrap().launcher_visible());
            assert!(!session.launcher_visibility.is_visible());
            assert_eq!(session.internal_ui.focused(), Some(application));
            assert!(
                session
                    .protocol_shell_surfaces()
                    .iter()
                    .any(
                        |surface| surface.role == ShellRole::Launcher && surface.geometry.is_none()
                    )
            );
        }
        session.handle_protocol_command(Command::ToggleLauncher, None, 0);
        assert!(session.internal_shell.as_ref().unwrap().launcher_visible());
        session.handle_protocol_command(Command::ToggleLauncher, None, 0);
        assert!(!session.internal_shell.as_ref().unwrap().launcher_visible());
        assert_eq!(session.internal_ui.focused(), Some(application));
    }

    #[test]
    fn internal_shell_protocol_geometry_uses_authoritative_global_placement() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, session) = internal_shell_test_session();
        let panel = session
            .protocol_shell_surfaces()
            .into_iter()
            .find(|surface| surface.role == ShellRole::Panel)
            .expect("internal panel snapshot");

        assert_eq!(
            panel.geometry,
            Some(nickel_session_protocol::Geometry {
                x: 0,
                y: 664,
                width: 1280,
                height: 56,
            })
        );
    }

    #[test]
    fn session_lock_drives_and_focuses_the_compositor_owned_lock_surface() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let lock = session
            .internal_shell
            .as_ref()
            .unwrap()
            .surfaces()
            .iter()
            .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Lock)
            .unwrap()
            .id;

        assert!(!session.internal_shell.as_ref().unwrap().visible(lock));
        assert!(!session.internal_shell_surfaces.contains_key(&lock));

        session.lock_session();

        assert!(session.locked);
        assert!(session.internal_shell.as_ref().unwrap().visible(lock));
        let focused = session.internal_ui.focused().expect("focused lock surface");
        assert!(
            session
                .internal_shell
                .as_ref()
                .unwrap()
                .surfaces()
                .iter()
                .any(
                    |surface| surface.role == crate::winit_shell::SurfaceRole::Lock
                        && session.internal_shell_surfaces.get(&surface.id) == Some(&focused)
                )
        );
        assert_eq!(session.seat.get_keyboard().unwrap().current_focus(), None);

        session.unlock_session();

        assert!(!session.locked);
        assert!(!session.internal_shell.as_ref().unwrap().visible(lock));
        assert!(!session.internal_shell_surfaces.contains_key(&lock));
    }

    #[test]
    fn native_screenshot_captures_and_opens_on_the_invoking_pointer_output() {
        use crate::session_host::DesktopCapturePoll;
        use crate::winit_shell::SurfaceRole;
        use nickel_session_protocol::{InputState, TestInput, TestKey};
        #[derive(Default)]
        struct CaptureHost(std::sync::Mutex<Vec<Option<String>>>);
        impl SessionHost for CaptureHost {
            fn dispatch(&self, _command: ShellCommand) -> Result<(), SessionRequestError> {
                Ok(())
            }
            fn capture_desktop(&self, output: Option<&str>) -> DesktopCapturePoll {
                self.0.lock().unwrap().push(output.map(str::to_owned));
                DesktopCapturePoll::Ready(Ok(crate::platform::DesktopCapture {
                    image: image::RgbaImage::new(1000, 800),
                }))
            }
        }
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        for (name, width, height, scale_120) in
            [("primary", 1280, 720, 120), ("secondary", 1000, 800, 180)]
        {
            session
                .apply_test_output(TestOutput::Connect {
                    name: name.into(),
                    logical_width: width,
                    logical_height: height,
                    scale_120,
                    transform: OutputTransform::Normal,
                })
                .unwrap();
        }
        let secondary = session
            .space
            .outputs()
            .find(|output| output.name() == "secondary")
            .unwrap()
            .clone();
        session.space.map_output(&secondary, (-1000, -120));
        let geometry = session.space.output_geometry(&secondary).unwrap();
        let host = Arc::new(CaptureHost::default());
        let (_sender, receiver) = crate::platform::status_mailbox::channel();
        session
            .enable_internal_shell_with_system_updates(host.clone(), receiver)
            .unwrap();
        session
            .inject_test_input(TestInput::PointerMove { x: -900, y: 100 })
            .unwrap();
        for state in [InputState::Pressed, InputState::Released] {
            session
                .inject_test_input(TestInput::Key {
                    key: TestKey::PrintScreen,
                    state,
                })
                .unwrap();
        }
        // Moving during the capture delay must not change the target display.
        session
            .inject_test_input(TestInput::PointerMove { x: 100, y: 100 })
            .unwrap();
        let shell = session.internal_shell.as_mut().unwrap();
        shell.poll(Instant::now() + Duration::from_millis(100));
        let screenshot = shell.surface(SurfaceRole::Screenshot, None).unwrap().id;
        assert!(shell.visible(screenshot));
        assert_eq!(*host.0.lock().unwrap(), vec![Some("secondary".into())]);
        session.sync_internal_shell();
        let runtime = session.internal_shell_surfaces[&screenshot];
        let placement = session.internal_ui.placement(runtime).unwrap();
        assert_eq!(placement.output.as_deref(), Some("secondary"));
        assert_eq!(
            placement.geometry,
            (
                geometry.loc.x,
                geometry.loc.y,
                geometry.size.w as u32,
                geometry.size.h as u32
            )
        );
        assert_eq!(session.internal_ui.focused(), Some(runtime));
    }

    #[test]
    fn native_screenshot_clipboard_retains_each_payload_for_repeated_paste() {
        use crate::session::{SessionAuthorityRequest, handlers::SelectionOwner};
        use smithay::wayland::selection::{
            SelectionHandler, SelectionTarget, data_device::current_data_device_selection_userdata,
        };
        use std::io::Read;
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let image = image::RgbaImage::from_pixel(3, 2, image::Rgba([23, 45, 67, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let png = Arc::new(png.into_inner());
        // Screenshot paths must work even when no text editor owns a copy limit.
        session.native_clipboard.text_limit = None;
        for request in [
            SessionAuthorityRequest::PublishClipboardImage(png.clone()),
            SessionAuthorityRequest::PublishClipboardText("/tmp/screenshot.png".into()),
            SessionAuthorityRequest::PublishClipboardImage(png.clone()),
        ] {
            assert!(matches!(
                session.handle_authority_request(request),
                nickel_session_protocol::ServerMessage::Ack
            ));
            let owner = current_data_device_selection_userdata(&session.seat)
                .unwrap()
                .clone();
            let (mime, expected) = match &owner {
                SelectionOwner::NativeImage(bytes) => ("image/png", bytes.as_slice()),
                SelectionOwner::NativeText(text) => ("text/plain;charset=utf-8", text.as_bytes()),
                _ => panic!("clipboard must stay compositor-owned"),
            };
            assert!(
                session
                    .native_clipboard
                    .mime_types
                    .iter()
                    .any(|value| value == mime)
            );
            for _ in 0..3 {
                let (mut reader, writer) = std::os::unix::net::UnixStream::pair().unwrap();
                reader
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let seat = session.seat.clone();
                SelectionHandler::send_selection(
                    &mut session,
                    SelectionTarget::Clipboard,
                    mime.into(),
                    writer.into(),
                    seat,
                    &owner,
                );
                let mut received = Vec::new();
                reader.read_to_end(&mut received).unwrap();
                assert_eq!(received, expected);
            }
        }
    }

    #[test]
    fn native_screenshot_claims_keyboard_on_show_and_escape_hides_without_clicking() {
        use crate::winit_shell::SurfaceRole;
        use nickel_session_protocol::{InputState, ShortcutAction, TestInput, TestKey};
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let shell = session.internal_shell.as_mut().unwrap();
        shell.global_shortcut(ShortcutAction::ShowScreenshotTool);
        shell.poll(Instant::now() + Duration::from_millis(100));
        let screenshot = shell.surface(SurfaceRole::Screenshot, None).unwrap().id;
        assert!(shell.visible(screenshot));
        session.sync_internal_shell();
        let runtime = session.internal_shell_surfaces[&screenshot];
        assert_eq!(session.internal_ui.focused(), Some(runtime));
        assert!(session.remote_desktop_events.snapshot().events.iter().any(
            |event| matches!(
                event.event,
                nickel_remote_control::desktop_events::DesktopEventKind::ShellSurfaceVisibilityChanged {
                    surface_generation,
                    role: nickel_remote_control::desktop_events::ShellEventRole::Screenshot,
                    visible: true,
                } if surface_generation == runtime.snapshot_token()
            )
        ));
        let record = session
            .remote_shell_surface_diagnostics()
            .0
            .into_iter()
            .find(|record| record.generation == runtime.snapshot_token())
            .expect("ordinary screenshot transient is projected");
        assert!(matches!(
            record.role,
            nickel_remote_control::diagnostics::ShellDiagnosticRole::Screenshot
        ));
        let (tree_generation, nodes) = session
            .internal_shell
            .as_ref()
            .unwrap()
            .bounded_shell_semantics(screenshot)
            .expect("bounded screenshot semantics");
        assert!(tree_generation > 0);
        assert!(nodes.len() <= nickel_remote_control::semantics::MAX_RESOLVED_NODES);
        for state in [InputState::Pressed, InputState::Released] {
            session
                .inject_test_input(TestInput::Key {
                    key: TestKey::Escape,
                    state,
                })
                .unwrap();
        }
        assert!(!session.internal_shell.as_ref().unwrap().visible(screenshot));
        assert!(!session.internal_ui.is_visible(runtime));
        assert_ne!(session.internal_ui.focused(), Some(runtime));
        assert!(session.remote_desktop_events.snapshot().events.iter().any(
            |event| matches!(
                event.event,
                nickel_remote_control::desktop_events::DesktopEventKind::ShellSurfaceVisibilityChanged {
                    surface_generation,
                    role: nickel_remote_control::desktop_events::ShellEventRole::Screenshot,
                    visible: false,
                } if surface_generation == runtime.snapshot_token()
            )
        ));
    }

    #[test]
    fn control_center_hides_on_client_or_internal_focus_transfer_and_stays_hidden() {
        use crate::winit_shell::SurfaceRole;
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        let application = session.internal_ui.insert(
            InternalWindowTestApp,
            crate::session::InternalSurfacePlacement {
                role: crate::session::InternalSurfaceRole::Application,
                geometry: (800, 100, 300, 300),
                output: Some("file-test".into()),
            },
            1.0,
        );
        for client in [true, false] {
            session
                .internal_shell
                .as_mut()
                .unwrap()
                .global_shortcut(nickel_session_protocol::ShortcutAction::ShowControlCenter);
            session.sync_internal_shell();
            let control = session
                .internal_shell
                .as_ref()
                .unwrap()
                .surface(SurfaceRole::ControlCenter, None)
                .unwrap()
                .id;
            let runtime = session.internal_shell_surfaces[&control];
            assert_eq!(session.internal_ui.focused(), Some(runtime));
            assert!(session.internal_shell.as_ref().unwrap().visible(control));
            let handled =
                session
                    .internal_ui
                    .pointer_button_with_client((900.0, 200.0), true, client);
            assert_eq!(handled, !client);
            session.flush_internal_shell_input();
            assert!(!session.internal_shell.as_ref().unwrap().visible(control));
            assert!(!session.internal_ui.is_visible(runtime));
            assert_eq!(
                session.internal_ui.focused(),
                (!client).then_some(application)
            );
            session.flush_internal_shell_input();
            session.sync_internal_shell();
            assert!(!session.internal_shell.as_ref().unwrap().visible(control));
            assert_eq!(
                session.internal_ui.focused(),
                (!client).then_some(application)
            );
        }
    }

    #[test]
    fn launcher_sidebar_press_is_not_dismissed_for_a_client_underneath() {
        use nickel_session_protocol::{InputState, TestInput, TestPointerButton};
        use nickel_ui::backend::PaintCommand;
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = internal_shell_test_session();
        assert!(session.toggle_internal_launcher());
        let shell = session.internal_shell.as_mut().unwrap();
        let launcher = shell
            .surface(crate::winit_shell::SurfaceRole::Launcher, None)
            .unwrap()
            .id;
        // Resolve the native presentation's label instead of copying layout coordinates.
        let bounds = shell
            .scene(launcher)
            .unwrap()
            .iter()
            .find_map(|command| match command {
                PaintCommand::Text { bounds, text, .. } if text == "Applications" => Some(*bounds),
                _ => None,
            })
            .expect("Applications sidebar label");
        let runtime = session.internal_shell_surfaces[&launcher];
        let geometry = session.internal_ui.placement(runtime).unwrap().geometry;
        session
            .inject_test_input(TestInput::PointerMove {
                x: geometry.0 + (bounds.origin.x + bounds.size.width / 2.0) as i32,
                y: geometry.1 + (bounds.origin.y + bounds.size.height / 2.0) as i32,
            })
            .unwrap();
        // This boundary is invoked when the native client scene occupies the point.
        // The foreground launcher must keep ownership despite that underlying client.
        assert!(!session.dismiss_internal_launcher_for_client_press());
        for state in [InputState::Pressed, InputState::Released] {
            session
                .inject_test_input(TestInput::PointerButton {
                    button: TestPointerButton::Left,
                    state,
                })
                .unwrap();
            assert!(session.internal_shell.as_ref().unwrap().launcher_visible());
            assert_eq!(session.internal_ui.focused(), Some(runtime));
        }
        let shell = session.internal_shell.as_mut().unwrap();
        assert_eq!(
            shell
                .scene(launcher)
                .unwrap()
                .iter()
                .filter(|command| matches!(
                    command, PaintCommand::Text { text, .. } if text == "Applications"
                ))
                .count(),
            2,
            "sidebar click must show the Applications content heading"
        );
    }

    #[test]
    fn ordinary_client_press_dismisses_internal_launcher_without_restoring_displaced_focus() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        session
            .apply_test_output(TestOutput::Connect {
                name: "test".into(),
                logical_width: 1280,
                logical_height: 720,
                scale_120: 120,
                transform: OutputTransform::Normal,
            })
            .unwrap();
        session
            .enable_internal_shell(Arc::new(IdleInternalHost))
            .expect("headless internal shell");

        assert!(session.toggle_internal_launcher());
        assert!(session.internal_shell.as_ref().unwrap().launcher_visible());
        assert!(session.dismiss_internal_launcher_for_client_press());
        assert!(!session.internal_shell.as_ref().unwrap().launcher_visible());
        assert!(session.launcher_restore_window.is_none());
    }

    #[test]
    fn applying_multi_output_fractional_scale_rebuilds_internal_surfaces_at_native_scale() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        for (name, width, height) in [("high", 1200, 900), ("normal", 1000, 800)] {
            session
                .apply_test_output(TestOutput::Connect {
                    name: name.into(),
                    logical_width: width,
                    logical_height: height,
                    scale_120: 120,
                    transform: OutputTransform::Normal,
                })
                .unwrap();
        }
        session
            .enable_internal_shell(Arc::new(IdleInternalHost))
            .unwrap();

        session
            .apply_output_layout(nickel_session_protocol::OutputLayout {
                primary: "high".into(),
                placements: vec![
                    nickel_session_protocol::OutputPlacement {
                        name: "high".into(),
                        x: 0,
                        y: 0,
                        enabled: true,
                        scale_120: 180,
                    },
                    nickel_session_protocol::OutputPlacement {
                        name: "normal".into(),
                        x: 800,
                        y: 0,
                        enabled: true,
                        scale_120: 120,
                    },
                ],
            })
            .unwrap();

        let outputs = session.protocol_outputs();
        let high = outputs.iter().find(|output| output.name == "high").unwrap();
        let normal = outputs
            .iter()
            .find(|output| output.name == "normal")
            .unwrap();
        assert_eq!((high.geometry.width, high.scale_120), (800, 180));
        assert_eq!((normal.geometry.x, normal.scale_120), (800, 120));

        let shell = session.internal_shell.as_ref().unwrap();
        for (name, expected) in [("high", 1.5_f32), ("normal", 1.0_f32)] {
            let surface = shell
                .surface(crate::winit_shell::SurfaceRole::Desktop, Some(name))
                .unwrap();
            let runtime = session.internal_shell_surfaces[&surface.id];
            assert_eq!(session.internal_ui.scale_factor(runtime), Some(expected));
        }
    }

    #[test]
    fn fractional_scale_is_published_with_required_viewporter_protocol() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, session) = preview_test_session();

        // Retaining both global handles is the Smithay contract that keeps the
        // paired protocols advertised for the session lifetime.
        assert_ne!(
            session.fractional_scale_manager_state.global(),
            session.viewporter_state.global()
        );
    }

    #[test]
    fn ordinary_session_exposes_a_restricted_settings_adapter_without_pid_authority() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let mut event_loop = EventLoop::try_new().unwrap();
        let display = Display::new().unwrap();
        let session = super::NickelSession::new(&mut event_loop, display, false);

        let control = session.compatibility_control.as_ref().unwrap();
        assert!(control.socket_path.exists());
        assert_eq!(control.protocol_token.len(), 64);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&control.socket_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(!session.is_authenticated_shell_pid(std::process::id()));
    }

    #[test]
    fn explicit_test_control_owns_compatibility_pid_state() {
        let (_event_loop, session) = preview_test_session();
        let control = session
            .compatibility_control
            .as_ref()
            .expect("test control should install the compatibility adapter");

        assert_ne!(control.protocol_token, "");
        assert_eq!(control.expected_shell_pid, 0);
        assert!(control.authenticated_shell_pids.is_empty());
        assert!(control.socket_path.exists());
    }

    #[test]
    fn output_global_settles_before_disable_and_remains_until_final_grace_expires() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let output = Output::new(
            "deferred-test".into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Nickel".into(),
                model: "Deferred test output".into(),
                serial_number: "deferred-test".into(),
            },
        );
        let global = output.create_global::<super::NickelSession>(&session.display_handle);
        let retained = global.clone();
        let started = Instant::now();

        session.defer_output_global_retirement_at("deferred-test".into(), global, started);
        let active = session
            .display_handle
            .backend_handle()
            .global_info(retained.clone())
            .expect("settling global remains advertised");
        assert!(!active.disabled);

        session.reap_output_global_retirements(
            started + OUTPUT_GLOBAL_BIND_SETTLE_GRACE - Duration::from_millis(1),
        );
        assert!(
            !session
                .display_handle
                .backend_handle()
                .global_info(retained.clone())
                .unwrap()
                .disabled
        );
        session.reap_output_global_retirements(started + OUTPUT_GLOBAL_BIND_SETTLE_GRACE);
        assert!(
            session
                .display_handle
                .backend_handle()
                .global_info(retained.clone())
                .expect("disabled global remains bindable during final grace")
                .disabled
        );
        assert!(
            session
                .display_handle
                .backend_handle()
                .global_info(retained.clone())
                .is_ok()
        );
        session.reap_output_global_retirements(
            started + OUTPUT_GLOBAL_BIND_SETTLE_GRACE + OUTPUT_GLOBAL_DISABLED_GRACE,
        );
        assert!(
            session
                .display_handle
                .backend_handle()
                .global_info(retained)
                .is_err()
        );
    }

    #[test]
    fn same_name_reconnect_waits_to_publish_until_the_old_global_is_disabled() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let connect = || TestOutput::Connect {
            name: "same".into(),
            logical_width: 640,
            logical_height: 480,
            scale_120: 120,
            transform: OutputTransform::Normal,
        };

        session.apply_test_output(connect()).unwrap();
        let old = session.virtual_test_outputs["same"].1.clone().unwrap();
        session
            .apply_test_output(TestOutput::Disconnect {
                name: "same".into(),
            })
            .unwrap();
        session.apply_test_output(connect()).unwrap();

        assert!(session.virtual_test_outputs["same"].1.is_none());
        assert!(
            !session
                .display_handle
                .backend_handle()
                .global_info(old.clone())
                .unwrap()
                .disabled
        );

        session.reap_output_global_retirements(Instant::now() + OUTPUT_GLOBAL_BIND_SETTLE_GRACE);
        assert!(
            session
                .display_handle
                .backend_handle()
                .global_info(old)
                .unwrap()
                .disabled
        );
        let replacement = session.virtual_test_outputs["same"].1.clone().unwrap();
        assert!(
            !session
                .display_handle
                .backend_handle()
                .global_info(replacement)
                .unwrap()
                .disabled
        );
    }

    #[test]
    fn rapid_same_name_reconnect_keeps_only_the_latest_live_generation_unpublished() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let connect = || TestOutput::Connect {
            name: "repeat".into(),
            logical_width: 640,
            logical_height: 480,
            scale_120: 120,
            transform: OutputTransform::Normal,
        };

        session.apply_test_output(connect()).unwrap();
        for _ in 0..64 {
            session
                .apply_test_output(TestOutput::Disconnect {
                    name: "repeat".into(),
                })
                .unwrap();
            session.apply_test_output(connect()).unwrap();
            assert!(session.virtual_test_outputs["repeat"].1.is_none());
            assert_eq!(session.pending_output_global_retirements.len(), 1);
        }

        let first_disable = Instant::now() + OUTPUT_GLOBAL_BIND_SETTLE_GRACE;
        session.reap_output_global_retirements(first_disable);
        assert!(session.virtual_test_outputs["repeat"].1.is_some());
        assert_eq!(session.pending_output_global_retirements.len(), 1);

        session
            .apply_test_output(TestOutput::Disconnect {
                name: "repeat".into(),
            })
            .unwrap();
        session.apply_test_output(connect()).unwrap();
        assert!(session.virtual_test_outputs["repeat"].1.is_none());
        assert_eq!(session.pending_output_global_retirements.len(), 2);

        let second_disable = first_disable + OUTPUT_GLOBAL_DISABLED_GRACE;
        session.reap_output_global_retirements(second_disable);
        assert!(session.virtual_test_outputs["repeat"].1.is_some());
        assert_eq!(session.pending_output_global_retirements.len(), 1);
        session.reap_output_global_retirements(second_disable + OUTPUT_GLOBAL_DISABLED_GRACE);
        assert_eq!(session.pending_output_global_retirements.len(), 0);
    }

    #[test]
    fn shutdown_drops_pending_and_unpublished_same_name_generations() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (event_loop, mut session) = preview_test_session();
        let connect = || TestOutput::Connect {
            name: "shutdown".into(),
            logical_width: 640,
            logical_height: 480,
            scale_120: 120,
            transform: OutputTransform::Normal,
        };
        session.apply_test_output(connect()).unwrap();
        session
            .apply_test_output(TestOutput::Disconnect {
                name: "shutdown".into(),
            })
            .unwrap();
        session.apply_test_output(connect()).unwrap();
        assert_eq!(session.pending_output_global_retirements.len(), 1);
        assert!(session.virtual_test_outputs["shutdown"].1.is_none());
        drop(session);
        drop(event_loop);
    }

    #[test]
    fn rapid_virtual_output_churn_applies_backpressure_until_reap() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let connect = |name: String| TestOutput::Connect {
            name,
            logical_width: 640,
            logical_height: 480,
            scale_120: 120,
            transform: OutputTransform::Normal,
        };

        for generation in 0..MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS {
            let name = format!("rapid-{generation}");
            session
                .apply_test_output(connect(name.clone()))
                .expect("churn within the deferred-global bound is admitted");
            session
                .apply_test_output(TestOutput::Disconnect { name })
                .expect("admitted output disconnects into the grace queue");
        }
        assert_eq!(
            session.pending_output_global_retirements.len(),
            MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS
        );
        assert_eq!(
            session.apply_test_output(connect("backpressured".into())),
            Err("output global retirement backlog is full")
        );

        let disable_at = Instant::now() + OUTPUT_GLOBAL_BIND_SETTLE_GRACE;
        session.reap_output_global_retirements(disable_at);
        assert_eq!(
            session.apply_test_output(connect("still-backpressured".into())),
            Err("output global retirement backlog is full")
        );
        session.reap_output_global_retirements(disable_at + OUTPUT_GLOBAL_DISABLED_GRACE);
        session
            .apply_test_output(connect("after-reap".into()))
            .expect("global admission resumes after the grace queue drains");
    }

    #[test]
    fn failed_capture_rolls_the_real_frame_and_allocation_back_into_session() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let id = session
            .windows
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.preview_admitted.insert(id);
        session.preview_frames.insert(
            id,
            super::PreviewFrame {
                width: 75,
                height: super::PREVIEW_HEIGHT as u16,
                rgba: vec![41; 75 * super::PREVIEW_HEIGHT * 4],
            },
        );
        let allocation = session.preview_frames[&id].rgba.as_ptr();

        let (rgba, had_frame) = session.take_preview_capture_buffer(id);
        session.preview_capture_failed(id, rgba, had_frame);

        assert_eq!(session.preview_frames[&id].rgba.as_ptr(), allocation);
        assert_eq!(session.preview_frames[&id].rgba[0], 41);
        assert_eq!(session.preview_frames[&id].width, 75);
        assert_eq!(session.preview_frames[&id].height, 135);
        assert_eq!(session.preview_counters.evictions, 0);
        assert_eq!(session.preview_counters.capture_failures, 1);
    }

    #[test]
    fn preview_source_churn_and_failed_capture_preserve_presentation_generation() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let id = session
            .windows
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.set_switcher_preview_interest(vec![id]);
        session.store_preview(
            id,
            super::PreviewFrame {
                width: 1,
                height: 1,
                rgba: vec![41; 4],
            },
        );
        let presented = session.preview_counters.presentation_generation;
        let allocation = session.preview_frames[&id].rgba.as_ptr();

        // Exercise the same content invalidation used by surface commits. Neither
        // repeated commits nor a failed replacement changes the retained pixels.
        for _ in 0..1000 {
            session.invalidate_preview_content(id);
        }
        assert!(session.preview_dirty.contains(&id));
        assert_eq!(session.preview_counters.invalidations, 1000);
        assert_eq!(session.preview_counters.presentation_generation, presented);
        let (pixels, dimensions) = session.take_preview_capture_buffer(id);
        session.preview_capture_failed(id, pixels, dimensions);
        assert_eq!(session.preview_counters.presentation_generation, presented);
        assert_eq!(session.preview_frames[&id].rgba.as_ptr(), allocation);
        assert_eq!(session.preview_frames[&id].rgba, vec![41; 4]);

        session.store_preview(
            id,
            super::PreviewFrame {
                width: 1,
                height: 1,
                rgba: vec![42; 4],
            },
        );
        assert_eq!(
            session.preview_counters.presentation_generation,
            presented + 1
        );
        session.reassociate_preview_surface(id);
        assert!(!session.preview_frames.contains_key(&id));
        assert_eq!(
            session.preview_counters.presentation_generation,
            presented + 2
        );
        session.reassociate_preview_surface(id);
        assert_eq!(
            session.preview_counters.presentation_generation,
            presented + 2
        );
    }

    #[test]
    fn retiring_preview_scratch_does_not_invalidate_presented_pixels() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let id = session
            .windows
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.set_switcher_preview_interest(vec![id]);
        let (pixels, dimensions) = session.take_preview_capture_buffer(id);
        session.preview_capture_failed(id, pixels, dimensions);
        session.clear_switcher_preview_interest();
        assert_eq!(session.preview_counters.presentation_generation, 0);
        assert_eq!(session.preview_bytes(), 0);
        assert_eq!(session.preview_counters.evictions, 1);

        session.set_switcher_preview_interest(vec![id]);
        session.store_preview(
            id,
            super::PreviewFrame {
                width: 1,
                height: 1,
                rgba: vec![41; 4],
            },
        );
        session.clear_switcher_preview_interest();
        assert_eq!(session.preview_counters.presentation_generation, 2);
        assert_eq!(session.preview_bytes(), 0);

        session.set_switcher_preview_interest(vec![id]);
        session.store_preview(
            id,
            super::PreviewFrame {
                width: 1,
                height: 1,
                rgba: vec![42; 4],
            },
        );
        session.clear_all_previews();
        assert_eq!(session.preview_counters.presentation_generation, 4);
        session.clear_all_previews();
        assert_eq!(session.preview_counters.presentation_generation, 4);
    }

    #[test]
    fn fitted_preview_frame_is_stored_at_its_actual_dimensions() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let id = session
            .windows
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.preview_admitted.insert(id);
        let width = 210;
        let height = super::PREVIEW_HEIGHT as u16;

        session.store_preview(
            id,
            super::PreviewFrame {
                width,
                height,
                rgba: vec![17; usize::from(width) * usize::from(height) * 4],
            },
        );

        let stored = &session.preview_frames[&id];
        assert_eq!((stored.width, stored.height), (width, height));
        assert_eq!(stored.rgba.len(), 113_400);
        assert_eq!(session.preview_counters.captures, 1);
    }

    #[cfg(feature = "backend-udev")]
    #[test]
    fn unavailable_preview_renderer_exhausts_shared_retry_budget_without_retiring_pixels() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let ids = (0..3)
            .map(|_| {
                session
                    .windows
                    .insert(crate::session::window_registry::WindowAdmission::Ordinary)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        session.preview_admitted.extend(ids.iter().copied());
        for id in &ids[1..] {
            session.store_preview(
                *id,
                super::PreviewFrame {
                    width: 1,
                    height: 1,
                    rgba: vec![41; 4],
                },
            );
        }
        session.preview_dirty.insert(ids[1]);
        let old_pixels = session.preview_frames[&ids[1]].rgba.as_ptr();
        let generation = session.preview_counters.presentation_generation;
        let started = Instant::now();
        for attempt in 0..5 {
            let now = started + Duration::from_secs(attempt);
            session.ready_preview_retries(now);
            session.preview_renderer_unavailable(now);
            assert_eq!(session.preview_counters.capture_failures, (attempt + 1) * 2);
            // Output/frame activity during cooldown must not charge more failures
            // or turn renderer lookup failure into an unbounded retry loop.
            for _ in 0..100 {
                session.preview_renderer_unavailable(now);
            }
            assert_eq!(session.preview_counters.capture_failures, (attempt + 1) * 2);
        }
        session.preview_renderer_unavailable(started + Duration::from_secs(60));
        assert_eq!(session.preview_counters.capture_failures, 10);
        assert!(session.preview_retry_pending.is_empty());
        assert!(!session.preview_capture_work_pending());
        assert_eq!(session.preview_frames[&ids[1]].rgba.as_ptr(), old_pixels);
        assert_eq!(session.preview_counters.presentation_generation, generation);
        assert!(!session.preview_failures.contains_key(&ids[2]));
    }

    #[test]
    fn fourteen_first_capture_failures_retain_exactly_the_declared_capacity() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let ids = (0..PREVIEW_ENTRY_CAPACITY)
            .map(|_| {
                session
                    .windows
                    .insert(crate::session::window_registry::WindowAdmission::Ordinary)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        session.preview_admitted.extend(ids.iter().copied());
        for id in ids {
            let (rgba, had_frame) = session.take_preview_capture_buffer(id);
            assert!(had_frame.is_none());
            session.preview_capture_failed(id, rgba, None);
        }

        assert_eq!(session.preview_bytes(), PREVIEW_BYTE_CAPACITY);
        assert_eq!(
            session.preview_counters.peak_bytes,
            PREVIEW_BYTE_CAPACITY as u64
        );
        assert_eq!(session.preview_spares.len(), PREVIEW_ENTRY_CAPACITY);
    }

    #[test]
    fn invalid_mapped_length_leaves_the_capture_lease_untouched() {
        let pixels = vec![23; PREVIEW_FRAME_BYTES];
        let allocation = pixels.as_ptr();
        assert!(!preview_mapping_has_exact_size(
            &vec![0; PREVIEW_FRAME_BYTES - 1],
            super::PREVIEW_WIDTH as u16,
            super::PREVIEW_HEIGHT as u16,
        ));
        assert_eq!(pixels.as_ptr(), allocation);
        assert_eq!(pixels[0], 23);
    }

    #[test]
    fn preview_capture_dimensions_are_bounded_and_preserve_window_aspect() {
        for (source, expected) in [
            ((3440, 1440), (240, 100)),
            ((1920, 1080), (240, 135)),
            ((1600, 1200), (180, 135)),
            ((1000, 1000), (135, 135)),
            ((900, 1600), (75, 135)),
            ((1919, 1079), (240, 134)),
        ] {
            let fitted = super::preview_capture_dimensions(source.0, source.1)
                .unwrap_or_else(|| panic!("positive source {source:?} has capture dimensions"));
            assert_eq!(fitted, expected, "source {source:?}");
            assert!(usize::from(fitted.0) <= super::PREVIEW_WIDTH);
            assert!(usize::from(fitted.1) <= super::PREVIEW_HEIGHT);
            if usize::from(fitted.0) == super::PREVIEW_WIDTH {
                let ideal_height = source.1 as f64 * f64::from(fitted.0) / source.0 as f64;
                assert!((ideal_height - f64::from(fitted.1)).abs() <= 1.0);
            } else {
                assert_eq!(usize::from(fitted.1), super::PREVIEW_HEIGHT);
                let ideal_width = source.0 as f64 * f64::from(fitted.1) / source.1 as f64;
                assert!((ideal_width - f64::from(fitted.0)).abs() <= 1.0);
            }
        }
        assert_eq!(super::preview_capture_dimensions(0, 1080), None);
        assert_eq!(super::preview_capture_dimensions(1920, -1), None);
    }

    #[test]
    fn stale_retry_epoch_cannot_consume_new_generation_pending_work() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (mut event_loop, mut session) = preview_test_session();
        let old = session
            .windows
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.set_switcher_preview_interest(vec![old]);
        session.record_preview_failure(old, std::time::Instant::now());
        session.schedule_preview_retry();
        let stale_epoch = session.preview_retry_scheduled.unwrap().0;

        session.clear_all_previews();
        let current = session
            .windows
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.set_switcher_preview_interest(vec![current]);
        session.preview_content_generation.insert(current, 10);
        session.record_preview_failure(current, std::time::Instant::now());
        session.schedule_preview_retry();
        assert_ne!(session.preview_retry_scheduled.unwrap().0, stale_epoch);

        event_loop
            .dispatch(std::time::Duration::from_millis(150), &mut session)
            .unwrap();
        assert_eq!(session.preview_content_generation[&current], 10);
        assert!(session.preview_retry_pending.is_empty());
    }

    #[test]
    fn nested_retry_waits_for_capture_cadence_and_fires_only_once() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (mut event_loop, mut session) = preview_test_session();
        let id = session
            .windows
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.set_switcher_preview_interest(vec![id]);
        session.preview_content_generation.insert(id, 4);
        session.record_preview_failure(id, std::time::Instant::now());
        session.schedule_preview_retry_after(std::time::Duration::from_millis(200));

        event_loop
            .dispatch(std::time::Duration::from_millis(30), &mut session)
            .unwrap();
        assert_eq!(session.preview_content_generation[&id], 4);
        assert_eq!(session.preview_retry_pending.len(), 1);
        assert!(session.preview_retry_scheduled.is_some());

        event_loop
            .dispatch(std::time::Duration::from_millis(220), &mut session)
            .unwrap();
        assert_eq!(session.preview_content_generation[&id], 4);
        assert!(session.preview_retry_pending.is_empty());
        assert!(session.preview_retry_scheduled.is_none());

        event_loop
            .dispatch(std::time::Duration::from_millis(30), &mut session)
            .unwrap();
        assert_eq!(session.preview_content_generation[&id], 4);
        assert!(session.preview_retry_scheduled.is_none());
    }

    #[test]
    fn preview_failure_backoff_survives_source_churn_and_stops_after_five_attempts() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let id = session
            .windows
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.set_switcher_preview_interest(vec![id]);
        let mut now = std::time::Instant::now();
        for delay_ms in [100, 200, 400, 800] {
            assert!(session.preview_retry_ready(id, now));
            session.record_preview_failure(id, now);
            // Video commits coalesce source content but cannot bypass cooldown.
            for _ in 0..1000 {
                session.invalidate_preview_content(id);
            }
            let before = now + std::time::Duration::from_millis(delay_ms - 1);
            assert!(!session.preview_retry_ready(id, before));
            assert!(!session.ready_preview_retries(before));
            now += std::time::Duration::from_millis(delay_ms);
            assert!(session.preview_retry_ready(id, now));
            assert!(session.ready_preview_retries(now));
            assert!(!session.ready_preview_retries(now));
        }
        session.record_preview_failure(id, now);
        let later = now + std::time::Duration::from_secs(3600);
        session.invalidate_preview_content(id);
        assert!(!session.preview_retry_ready(id, later));
        assert!(!session.ready_preview_retries(later));
        assert!(session.preview_retry_pending.is_empty());
        session.schedule_preview_retry();
        assert!(session.preview_retry_scheduled.is_none());
        assert_eq!(session.preview_failures.len(), 1);

        session.reassociate_preview_surface(id);
        assert!(session.preview_retry_ready(id, later));
        session.record_preview_failure(id, later);
        session.clear_switcher_preview_interest();
        assert!(session.preview_failures.is_empty());
        session.set_switcher_preview_interest(vec![id]);
        assert!(session.preview_retry_ready(id, later));
    }

    #[test]
    fn preview_retry_drains_only_due_windows_and_success_retires_failure_state() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let ids = (0..2)
            .map(|_| {
                session
                    .windows
                    .insert(crate::session::window_registry::WindowAdmission::Ordinary)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        session.set_switcher_preview_interest(ids.clone());
        let now = std::time::Instant::now();
        session.record_preview_failure(ids[0], now);
        session.record_preview_failure(ids[1], now + std::time::Duration::from_millis(50));
        assert!(session.ready_preview_retries(now + std::time::Duration::from_millis(100)));
        assert!(!session.preview_retry_pending.contains(&ids[0]));
        assert!(session.preview_retry_pending.contains(&ids[1]));
        session.store_preview(
            ids[1],
            super::PreviewFrame {
                width: 1,
                height: 1,
                rgba: vec![41; 4],
            },
        );
        assert!(!session.preview_failures.contains_key(&ids[1]));
        assert!(session.preview_retry_pending.is_empty());
        session.clear_all_previews();
        assert!(session.preview_failures.is_empty());
    }

    #[test]
    fn real_session_reconcile_preserves_seven_frames_for_each_visible_consumer() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let switcher = (0..7)
            .map(|_| {
                session
                    .windows
                    .insert(crate::session::window_registry::WindowAdmission::Ordinary)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let overlay = (0..7)
            .map(|_| {
                session
                    .windows
                    .insert(crate::session::window_registry::WindowAdmission::Ordinary)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        session.set_switcher_preview_interest(switcher.clone());
        session.set_overlay_preview_interest(overlay.clone());
        for id in switcher.iter().chain(&overlay).copied() {
            session.preview_frames.insert(
                id,
                super::PreviewFrame {
                    width: super::PREVIEW_WIDTH as u16,
                    height: super::PREVIEW_HEIGHT as u16,
                    rgba: vec![id.0 as u8; PREVIEW_FRAME_BYTES],
                },
            );
        }

        session.clear_switcher_preview_interest();

        assert!(
            overlay
                .iter()
                .all(|id| session.preview_frames.contains_key(id))
        );
        assert_eq!(session.preview_bytes(), 7 * PREVIEW_FRAME_BYTES);
    }

    #[test]
    fn real_preview_query_and_encode_counters_partition_the_aggregate() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let id = session
            .windows
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.preview_admitted.insert(id);
        session.preview_frames.insert(
            id,
            super::PreviewFrame {
                width: super::PREVIEW_WIDTH as u16,
                height: super::PREVIEW_HEIGHT as u16,
                rgba: vec![5; PREVIEW_FRAME_BYTES],
            },
        );
        let message = session.handle_protocol_query(Query::Preview {
            window: nickel_session_protocol::WindowId(id.0),
        });
        assert!(matches!(message, ServerMessage::Preview(_)));
        let framed = nickel_session_protocol::encode(&ServerEnvelope {
            request_id: 9,
            message,
        })
        .unwrap();
        session.record_preview_protocol_encoding(
            framed.len() - nickel_session_protocol::FRAME_HEADER_BYTES,
            framed.len(),
        );

        let counters = session.preview_counters;
        assert_eq!(
            counters.protocol_copy_bytes,
            counters.protocol_raw_copy_bytes
                + counters.protocol_base64_bytes
                + counters.protocol_json_payload_bytes
                + counters.protocol_framed_copy_bytes
        );
        assert_eq!(counters.protocol_raw_copy_bytes, PREVIEW_FRAME_BYTES as u64);
    }

    #[test]
    fn real_session_attempt_generation_blocks_other_nodes_until_explicit_retry() {
        let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
        let (_event_loop, mut session) = preview_test_session();
        let id = session
            .windows
            .insert(crate::session::window_registry::WindowAdmission::Ordinary)
            .unwrap();
        session.preview_content_generation.insert(id, 6);
        let first_node_wave = session.begin_preview_render_wave();
        assert!(record_preview_capture_attempt(
            &mut session.preview_attempted,
            id,
            6,
            first_node_wave
        ));
        let second_node_wave = session.begin_preview_render_wave();
        assert!(!record_preview_capture_attempt(
            &mut session.preview_attempted,
            id,
            6,
            second_node_wave
        ));

        session.preview_admitted.insert(id);
        session.preview_renderer_failed(id);
        assert!(!session.ready_preview_retries(std::time::Instant::now()));
        assert!(session.ready_preview_retries(
            std::time::Instant::now() + std::time::Duration::from_millis(100)
        ));
        let retry_wave = session.begin_preview_render_wave();
        assert!(record_preview_capture_attempt(
            &mut session.preview_attempted,
            id,
            6,
            retry_wave
        ));
    }

    #[test]
    fn preview_workload_is_bounded_around_the_selected_window() {
        let ids = (0..nickel_session_protocol::MAX_WINDOWS as u64)
            .map(super::WindowId)
            .collect::<Vec<_>>();
        let selected = nickel_session_protocol::MAX_WINDOWS / 2;
        let admitted = bounded_preview_ids(ids, selected);

        assert_eq!(admitted.len(), PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER);
        assert!(admitted.contains(&super::WindowId(selected as u64)));
        assert_eq!(
            PREVIEW_BYTE_CAPACITY,
            PREVIEW_ENTRY_CAPACITY * PREVIEW_FRAME_BYTES
        );
        assert_eq!(PREVIEW_BYTE_CAPACITY, 1_814_400);
    }

    #[test]
    fn preview_workload_clamps_at_both_candidate_edges() {
        let ids = (0..12).map(super::WindowId).collect::<Vec<_>>();
        assert_eq!(bounded_preview_ids(ids.clone(), 0), ids[..7]);
        assert_eq!(bounded_preview_ids(ids.clone(), 11), ids[5..]);
    }

    #[test]
    fn independent_preview_consumers_share_the_budget_without_destroying_interest() {
        let switcher = (1..=7).map(super::WindowId).collect::<Vec<_>>();
        let overlay = (8..=(nickel_session_protocol::MAX_WINDOWS as u64 + 8))
            .map(super::WindowId)
            .collect::<Vec<_>>();

        let overlapping = admitted_preview_ids(&switcher, &overlay);
        assert_eq!(overlapping.len(), PREVIEW_ENTRY_CAPACITY);
        assert!(switcher.iter().all(|id| overlapping.contains(id)));
        let mut frames = overlapping
            .iter()
            .map(|id| {
                (
                    *id,
                    super::PreviewFrame {
                        width: super::PREVIEW_WIDTH as u16,
                        height: super::PREVIEW_HEIGHT as u16,
                        rgba: vec![id.0 as u8; PREVIEW_FRAME_BYTES],
                    },
                )
            })
            .collect::<HashMap<_, _>>();

        let after_overlay_dismissal = admitted_preview_ids(&switcher, &[]);
        assert_eq!(after_overlay_dismissal, switcher.iter().copied().collect());
        let after_switcher_dismissal = admitted_preview_ids(&[], &overlay);
        assert_eq!(after_switcher_dismissal.len(), PREVIEW_ENTRY_CAPACITY);
        frames.retain(|id, _| after_switcher_dismissal.contains(id));
        assert_eq!(frames.len(), PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER);
        assert!(
            overlay[..PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER]
                .iter()
                .all(|id| protocol_preview_from_cached(
                    nickel_session_protocol::WindowId(id.0),
                    frames.get(id)
                )
                .is_some())
        );
    }

    #[test]
    fn failed_capture_is_attempted_once_per_generation_across_outputs_and_nodes() {
        let id = super::WindowId(7);
        let mut attempted = HashMap::new();
        assert!(record_preview_capture_attempt(&mut attempted, id, 3, 11));
        for _ in 0..3 {
            assert!(!record_preview_capture_attempt(&mut attempted, id, 3, 11));
        }
        assert!(!record_preview_capture_attempt(&mut attempted, id, 3, 12));
        assert!(record_preview_capture_attempt(&mut attempted, id, 4, 12));
    }

    #[test]
    fn preview_churn_never_admits_more_than_the_declared_process_ceiling() {
        for generation in 0..(nickel_session_protocol::MAX_WINDOWS as u64 * 2) {
            let switcher = (generation..generation + 7)
                .map(super::WindowId)
                .collect::<Vec<_>>();
            let overlay = (generation + 7..generation + 1_031)
                .map(super::WindowId)
                .collect::<Vec<_>>();
            let admitted = admitted_preview_ids(&switcher, &overlay);
            assert!(admitted.len() <= PREVIEW_ENTRY_CAPACITY);
            assert!(admitted.len() * PREVIEW_FRAME_BYTES <= PREVIEW_BYTE_CAPACITY);
        }
    }

    #[test]
    fn damage_advances_only_the_authoritative_window_content_generation() {
        let damaged = super::WindowId(1);
        let unchanged = super::WindowId(2);
        let mut generations = HashMap::from([(damaged, 3), (unchanged, 8)]);
        let mut attempted = HashMap::from([(damaged, (3, 4)), (unchanged, (8, 4))]);

        assert_eq!(
            advance_preview_content_generation(&mut generations, &mut attempted, damaged),
            4
        );
        assert_eq!(generations[&unchanged], 8);
        assert!(!attempted.contains_key(&damaged));
        assert_eq!(attempted[&unchanged], (8, 4));
    }

    #[test]
    fn not_ready_query_has_no_interest_side_effect() {
        let switcher = vec![super::WindowId(1)];
        let overlay = vec![super::WindowId(2)];
        let before = admitted_preview_ids(&switcher, &overlay);
        assert!(
            protocol_preview_from_cached(nickel_session_protocol::WindowId(99), None).is_none()
        );
        assert_eq!(admitted_preview_ids(&switcher, &overlay), before);
    }

    #[test]
    fn replacement_reuses_the_retired_frame_allocation() {
        let pixels = vec![7; PREVIEW_FRAME_BYTES];
        let allocation = pixels.as_ptr();
        let replacement = reuse_preview_pixels(pixels, &vec![9; PREVIEW_FRAME_BYTES]);
        assert_eq!(replacement.as_ptr(), allocation);
        assert_eq!(replacement.len(), PREVIEW_FRAME_BYTES);
        assert!(replacement.iter().all(|pixel| *pixel == 9));
    }

    #[test]
    fn only_the_exact_supervised_shell_pid_can_register() {
        let current = std::process::id();
        assert_eq!(
            shell_registration_rejection(current, current, current + 1, true),
            Some(ShellRegistrationRejection::ClaimedPeerMismatch)
        );
        assert_eq!(
            shell_registration_rejection(0, current, current, true),
            Some(ShellRegistrationRejection::NoActiveGeneration)
        );
        assert_eq!(
            shell_registration_rejection(current + 1, current, current, true),
            Some(ShellRegistrationRejection::OutsideActiveGeneration)
        );
        assert_eq!(
            shell_registration_rejection(current, current, current, false),
            Some(ShellRegistrationRejection::OutsideSessionUser)
        );
        assert_eq!(
            shell_registration_rejection(current, current, current, true),
            None
        );
    }

    #[test]
    fn privileged_shell_commands_require_the_registered_shell_pid() {
        for command in [
            Command::LogOut,
            Command::ObservePendingLaunch {
                generation: 7,
                root_pid: 42,
                root_start_time: 99,
                deadline_ms: 100,
            },
            Command::CancelPendingLaunch { generation: 7 },
            Command::Unlock,
            Command::SessionAction {
                action: SessionAction::Lock,
            },
            Command::SessionAction {
                action: SessionAction::PowerOff,
            },
            Command::FocusShellRole {
                role: ShellRole::ControlCenter,
            },
            Command::RestoreApplicationFocus,
        ] {
            assert!(command_requires_shell_identity(&command));
        }
        assert!(!command_requires_shell_identity(&Command::ToggleLauncher));
    }

    #[test]
    fn process_lineage_accepts_self_and_a_live_descendant_only() {
        let start = super::linux_process_start_time(std::process::id()).unwrap();
        assert!(super::process_descends_from(
            std::process::id(),
            std::process::id(),
            start,
        ));
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 2"])
            .spawn()
            .expect("spawn lineage fixture");
        assert!(super::process_descends_from(
            child.id(),
            std::process::id(),
            start,
        ));
        assert!(!super::process_descends_from(
            child.id(),
            std::process::id(),
            start.wrapping_add(1),
        ));
        assert!(!super::process_descends_from(
            std::process::id(),
            child.id(),
            super::linux_process_start_time(child.id()).unwrap(),
        ));
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn ordinary_shell_focus_includes_the_interactive_screenshot_overlay() {
        for role in [
            ShellRole::ControlCenter,
            ShellRole::ProjectMenu,
            ShellRole::Preview,
            ShellRole::ContextMenu,
            ShellRole::Screenshot,
        ] {
            assert!(shell_role_accepts_ordinary_focus(role));
        }
        for role in [
            ShellRole::Desktop,
            ShellRole::Panel,
            ShellRole::Launcher,
            ShellRole::Lock,
            ShellRole::Notification,
        ] {
            assert!(!shell_role_accepts_ordinary_focus(role));
        }
    }

    #[test]
    fn explicit_nested_test_control_can_cross_lock_and_logout_boundaries() {
        assert!(test_control_may_invoke(&Command::LogOut));
        assert!(test_control_may_invoke(&Command::Unlock));
        assert!(test_control_may_invoke(&Command::SessionAction {
            action: SessionAction::Lock,
        }));
        assert!(!test_control_may_invoke(&Command::SessionAction {
            action: SessionAction::PowerOff,
        }));
    }

    #[test]
    fn surface_identity_registration_requires_the_authenticated_shell() {
        assert!(command_requires_shell_identity(
            &Command::RegisterShellSurface {
                identity: nickel_session_protocol::ShellSurfaceIdentity {
                    application_id: "io.nickel.shell.surface.42.1".into(),
                    role: ShellRole::Desktop,
                    output: Some("DP-1".into()),
                },
            }
        ));
    }

    #[test]
    fn disconnected_surfaces_stop_inhibiting_idle_policy() {
        let mut inhibitors = HashMap::from([("alive", 2), ("disconnected", 1)]);
        retain_live_idle_inhibitors(&mut inhibitors, |surface| *surface == "alive");
        assert_eq!(inhibitors, HashMap::from([("alive", 2)]));
    }

    #[test]
    fn descriptive_output_names_resolve_to_authoritative_connector_names() {
        let outputs = vec!["DVI-I-1".into(), "DP-3".into()];

        assert_eq!(
            output_index_for_shell_surface("Unknown - Odyssey G40B - DP-3", &outputs),
            Some(1)
        );
        assert_eq!(
            output_index_for_shell_surface("Unknown - MB16A - DVI-I-1", &outputs),
            Some(0)
        );
    }

    #[test]
    fn descriptive_output_matching_rejects_missing_or_ambiguous_connectors() {
        assert_eq!(
            output_index_for_shell_surface("Unknown - DisplayPort-1", &["DP-1".into()]),
            None
        );
        assert_eq!(
            output_index_for_shell_surface(
                "Unknown - Display - DP-1",
                &["DP-1".into(), "Display - DP-1".into()]
            ),
            None
        );
    }

    #[test]
    fn output_rescue_clamps_windows_to_the_authoritative_work_area() {
        let work_area = Geometry {
            x: 100,
            y: 40,
            width: 800,
            height: 500,
        };
        assert_eq!(
            clamp_window_location((850, 500).into(), (300, 200).into(), work_area),
            (600, 340).into()
        );
        assert_eq!(
            clamp_window_location((-20, -30).into(), (300, 200).into(), work_area),
            (100, 40).into()
        );
    }

    #[test]
    fn maximized_server_frame_exactly_fits_the_work_area() {
        let work_area = Geometry {
            x: -1920,
            y: 30,
            width: 1920,
            height: 1010,
        };

        let content = maximized_content_geometry(work_area, true);

        assert_eq!(
            crate::session::window_frame::outer_geometry(content),
            work_area
        );
    }

    #[test]
    fn initial_managed_x11_content_keeps_its_frame_inside_the_work_area() {
        let work_area = Geometry {
            x: 0,
            y: 0,
            width: 1920,
            height: 1024,
        };
        let content = clamp_decorated_content_to_work_area(
            Geometry {
                x: 0,
                y: 0,
                width: 1200,
                height: 800,
            },
            work_area,
        );

        let outer = crate::session::window_frame::outer_geometry(content);
        assert_eq!(outer.x, work_area.x);
        assert_eq!(outer.y, work_area.y);
        assert_eq!(content.width, 1200);
        assert_eq!(content.height, 800);
    }

    #[test]
    fn maximized_client_decorated_window_receives_the_whole_work_area() {
        let work_area = Geometry {
            x: 1920,
            y: -200,
            width: 1280,
            height: 700,
        };

        assert_eq!(maximized_content_geometry(work_area, false), work_area);
    }

    #[test]
    fn maximized_content_geometry_clamps_undersized_work_areas() {
        let work_area = Geometry {
            x: 7,
            y: 11,
            width: 1,
            height: 1,
        };

        let content = maximized_content_geometry(work_area, true);

        assert_eq!(content.width, 1);
        assert_eq!(content.height, 1);
    }

    #[test]
    fn restored_drag_preserves_horizontal_pointer_proportion() {
        let current = maximized_content_geometry(
            Geometry {
                x: 0,
                y: 0,
                width: 1200,
                height: 700,
            },
            true,
        );
        let restore = Geometry {
            x: 80,
            y: 90,
            width: 600,
            height: 400,
        };
        let work_area = Geometry {
            x: 0,
            y: 0,
            width: 1200,
            height: 700,
        };

        let left = restored_drag_content_geometry(
            current,
            restore,
            Point::from((120.0, 18.0)),
            true,
            work_area,
        );
        let center = restored_drag_content_geometry(
            current,
            restore,
            Point::from((600.0, 18.0)),
            true,
            work_area,
        );
        let right = restored_drag_content_geometry(
            current,
            restore,
            Point::from((1080.0, 18.0)),
            true,
            work_area,
        );

        assert!(left.x < center.x);
        assert!(center.x < right.x);
        assert_eq!(left.width, restore.width);
        assert_eq!(center.height, restore.height);
    }

    #[test]
    fn restored_drag_keeps_titlebar_reachable_on_negative_output() {
        let work_area = Geometry {
            x: -1920,
            y: -200,
            width: 1920,
            height: 1000,
        };
        let geometry = restored_drag_content_geometry(
            maximized_content_geometry(work_area, true),
            Geometry {
                x: 10,
                y: 10,
                width: 900,
                height: 700,
            },
            Point::from((-1910.0, -195.0)),
            true,
            work_area,
        );
        let outer = crate::session::window_frame::outer_geometry(geometry);

        assert!(outer.x + outer.width >= work_area.x + 32);
        assert!(outer.y >= work_area.y);
        assert!(outer.y < work_area.y + work_area.height);
    }

    #[test]
    fn drag_icon_uses_pointer_output_and_output_local_coordinates() {
        let left = smithay::utils::Rectangle::new((0, 0).into(), (1920, 1080).into());
        let right = smithay::utils::Rectangle::new((1920, 0).into(), (1920, 1080).into());
        let pointer = smithay::utils::Point::from((2012.4, 84.6));

        assert_eq!(drag_icon_location(pointer, left), None);
        assert_eq!(drag_icon_location(pointer, right), Some((92, 85).into()));
    }

    #[test]
    fn transient_output_hit_testing_uses_both_global_axes() {
        let upper = smithay::utils::Rectangle::new((0, -1080).into(), (1920, 1080).into());
        let lower = smithay::utils::Rectangle::new((0, 0).into(), (1920, 1080).into());

        assert!(output_contains_logical_point(upper, 960, -40));
        assert!(!output_contains_logical_point(lower, 960, -40));
        assert!(!output_contains_logical_point(upper, 960, 1040));
        assert!(output_contains_logical_point(lower, 960, 1040));
    }
}
