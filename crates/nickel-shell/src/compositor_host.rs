//! Typed interface used by the trusted compositor host.
//!
//! Native windows and input remain owned by the runtime. Shell policy and
//! presentation remain owned here; this module exports only its host-facing
//! vocabulary. Rust API visibility does not grant remote or package authority.

pub mod bundled_plugin_assets {
    pub use crate::bundled_plugin_assets::load_package;
}
pub mod control_view {
    pub use crate::control_view::ControlAction;
}
pub mod feature_capabilities {
    pub use crate::feature_capabilities::read_settings;
}
pub mod internal_shell {
    pub use crate::internal_shell::{
        InternalOutput, InternalShellCoordinator, control_center_size,
    };
}
pub mod live_shell {
    pub use crate::live_shell::remote_semantics;
    pub use crate::live_shell::{CodexApprovalOwner, ShellProfile, passive_pointer_batch};
}
pub mod local_cues {
    pub use crate::local_cues::LocalCues;
}
pub mod model {
    pub use crate::model::{
        Application, ApplicationDiscovery, clear_trusted_session_capability,
        install_trusted_session_capability,
    };
}
pub mod platform {
    pub use crate::platform::status_mailbox;
    pub use crate::platform::{
        AudioRefresh, AudioStatus, BluetoothStatus, ConnectivityRefresh,
        DefaultAssociationsRefresh, DesktopCapture, GuardedControlOrigin,
        GuardedControlOriginOwner, GuardedControlOutcome, GuardedDeviceObservation,
        MaintenanceRefresh, NetworkStatus, PeripheralRefresh, SecureStorageState,
        SessionRequestError, ShellCommand, SystemStatusUpdate, installed_application_signatures,
        prepare_application_discovery, prepare_audio_environment, publish_application_discovery,
        read_guarded_device, refresh_audio_status, refresh_connectivity_status,
        refresh_default_associations, refresh_maintenance_status, refresh_peripheral_status,
        run_signature_diagnostics, save_temp_image, secure_storage_allows_application_launch,
        shell_command_payload, submit_guarded_control, system_status_receiver,
    };
}
pub mod plugin_panel {
    pub use crate::plugin_panel::bottom_offset;
}
pub mod process_memory {
    pub use crate::process_memory::{diagnostics, maybe_trim, trim_diagnostics};
}
pub mod remote_default_associations {
    pub use crate::remote_default_associations::{read, transact};
}
pub mod remote_indicator {
    pub use crate::remote_indicator::{IndicatorGrant, RemoteIndicator, indicator_height};
}
pub mod remote_peripheral_controls {
    pub use crate::remote_peripheral_controls::{Prepared, State, control_failure_reason};
}
pub mod remote_policy {
    pub use crate::remote_policy::{
        FavoriteCatalog, apply_terminal_preferences, changed_favorites, favorite_projection,
        favorite_snapshot, requested_application_scale_policy, scale_observation_generation,
        scale_observation_matches, scale_outcome, scale_snapshot, scale_toolkits,
        terminal_preferences, terminal_snapshot,
    };
}
pub mod remote_preferred_applications {
    pub use crate::remote_preferred_applications::{
        PreparedChange, PreparedRead, production_catalog,
    };
}
pub mod remote_surface_authority {
    pub use crate::remote_surface_authority::SurfaceAuthority;
}
pub mod remote_terminal_launch_policy {
    pub use crate::remote_terminal_launch_policy::{PreparedChange, PreparedRead, State, outcome};
}
pub mod session_host {
    pub use crate::session_host::{
        DesktopCapturePoll, SessionHost, StagedSessionHost, record_clipboard_outcome,
    };
}
pub mod trusted_accessibility {
    pub use crate::trusted_accessibility::native;
}
pub mod window_preview {
    pub use crate::window_preview::semantic_theme_from_palette;
}
pub mod winit_shell {
    pub use crate::winit_shell::{PANEL_HEIGHT, PanelEdge, SurfaceRole};
}
pub use crate::is_clipboard_paste;

pub use crate::{appearance_service, wallpaper_service};
pub mod launcher {
    pub use crate::launcher::{
        DashboardProject, DashboardSection, ProjectActivity, normalize_dashboard_projects,
    };
}
pub use crate::{codex_project_application_id, codex_projection};
