//! Bounded diagnostic projections, without input payloads or raw process logs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DiagnosticAction {
    Repaint,
    /// Reconcile native scene/output membership using production housekeeping.
    RefreshScene,
    /// Rescan the bounded platform-owned installed-application catalog.
    RefreshApplicationInventory,
    /// Ask one allowlisted production platform worker for a fresh observation.
    RefreshPlatformStatus {
        domain: PlatformRefreshDomain,
    },
    StartFrameTrace {
        duration_seconds: u16,
    },
    StopFrameTrace,
    /// Existing compositor-owned output number, on one exact live output only.
    IdentifyOutput {
        output: crate::leases::ResourceId,
    },
}

impl DiagnosticAction {
    pub fn validate(&self) -> Result<(), String> {
        if let Self::IdentifyOutput { output } = self
            && !crate::leases::valid_resource_scope(&crate::leases::ResourceScope::Output(
                output.clone(),
            ))
        {
            return Err("invalid output identity".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DiagnosticActionOutcome {
    pub action: DiagnosticAction,
    pub observation_generation: u64,
    pub submitted_at_us: u64,
    /// Queuing damage does not prove a frame was presented.
    pub presentation_confirmed: bool,
    /// Present only for an accepted output identification; never a presentation claim.
    pub output_identification: Option<OutputIdentificationOutcome>,
    pub application_inventory_refresh: Option<ApplicationInventoryRefreshOutcome>,
    pub platform_refresh: Option<PlatformRefreshOutcome>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlatformRefreshDomain {
    Connectivity,
    Audio,
    Peripherals,
    Maintenance,
    DefaultAssociations,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PlatformRefreshOutcome {
    pub domain: PlatformRefreshDomain,
    pub generation: u64,
    pub observation_started_at_us: u64,
    pub observed_at_us: u64,
    pub preparation_duration_us: u64,
    /// True only when a retained snapshot observation has aged beyond its
    /// bounded freshness interval. Direct action results are always fresh.
    pub stale: bool,
    pub network_available: bool,
    pub bluetooth_available: bool,
    pub audio_available: bool,
    pub printers_available: bool,
    pub volumes_available: bool,
    pub filesystems_available: bool,
    pub printer_count: u32,
    pub volume_count: u32,
    pub filesystem_count: u32,
    pub maintenance_available: bool,
    pub updates_available: Option<u32>,
    pub restart_required: Option<bool>,
    pub firewall_healthy: Option<bool>,
    pub malware_protection_healthy: Option<bool>,
    pub known_permission_states: u32,
    pub secure_storage_status_available: bool,
    pub associations_available: bool,
    pub association_targets_queried: u32,
    pub effective_associations: u32,
    pub directly_writable_associations: u32,
    pub partial: bool,
    /// The compositor reconciled the returned snapshots; this is not presentation confirmation.
    pub reconciliation_confirmed: bool,
}

impl PlatformRefreshOutcome {
    pub const FRESH_FOR_US: u64 = 5_000_000;

    pub fn retained_at(&self, observed_at_us: u64) -> Self {
        let mut retained = self.clone();
        retained.stale = observed_at_us.saturating_sub(self.observed_at_us) > Self::FRESH_FOR_US;
        retained
    }
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ApplicationInventoryRefreshOutcome {
    pub generation: u64,
    pub observation_started_at_us: u64,
    pub observed_at_us: u64,
    pub preparation_duration_us: u64,
    pub stale: bool,
    pub applications: u32,
    pub partial: bool,
    /// The launcher/icon model accepted the catalog; this is not pixel presentation.
    pub reconciliation_confirmed: bool,
}

impl ApplicationInventoryRefreshOutcome {
    pub const FRESH_FOR_US: u64 = 5_000_000;

    pub fn retained_at(&self, observed_at_us: u64) -> Self {
        let mut retained = self.clone();
        retained.stale = observed_at_us.saturating_sub(self.observed_at_us) > Self::FRESH_FOR_US;
        retained
    }
}

pub const MAX_DIAGNOSTIC_WINDOWS: usize = 512;
pub const MAX_DIAGNOSTIC_OUTPUTS: usize = 32;
pub const MAX_DIAGNOSTIC_SHELL_SURFACES: usize = 128;
pub const MAX_DIAGNOSTIC_WORKSPACES: usize = 32;

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct OutputInventory {
    pub observation_generation: u64,
    pub observed_at_us: u64,
    /// Exact output identities, filtered by the supplied lease.
    pub outputs: Vec<OutputDiagnostic>,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAction {
    List,
    Create,
    Switch { workspace: u64 },
    Remove { workspace: u64 },
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WorkspaceOutcome {
    pub requested: WorkspaceAction,
    pub created_workspace: Option<u64>,
    pub observation_generation: u64,
    pub observed_at_us: u64,
    /// Committed production membership and selection, not presentation confirmation.
    pub workspaces: Vec<WorkspaceDiagnostic>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WorkspaceDiagnostic {
    /// Session-owned identity; never a caller-provided workspace label.
    pub id: u64,
    pub active: bool,
    /// Only windows present in this same bounded, protected-filtered snapshot.
    pub windows: Vec<String>,
    /// Remembered focus for this workspace, not necessarily current seat focus.
    pub last_focused_window: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct OutputDiagnostic {
    pub name: String,
    pub generation: u64,
    pub geometry: [i32; 4],
    pub work_area: [i32; 4],
    pub scale_120: u32,
    pub primary: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PreviewDiagnostic {
    pub presentation_generation: u64,
    pub readback_bytes: u64,
    pub capture_failures: u64,
    /// Native compositor-owned preview presentation state where preview pixels
    /// are never read back into Nickel. None means the backend has no such owner.
    pub native_presentation_generation: Option<u64>,
    /// Failed native preview registration/update attempts. This is distinct
    /// from capture_failures because no pixel capture was attempted.
    pub native_presentation_failures: Option<u64>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InputDeviceDiagnostic {
    /// Compositor seat or hosted-application top-level window recipient, drawn
    /// only from diagnostic window IDs. Shell recipients use focused_surface.
    /// Protected/unprojected recipients make the whole device record unavailable.
    /// X11 client-side focus changes require a separate platform query.
    pub focused_window: Option<String>,
    /// Current ordinary shell recipient, with its live surface incarnation.
    pub focused_surface: Option<crate::leases::ResourceId>,
    /// Compositor grab state; does not claim to observe X11 client-side grabs.
    pub compositor_grabbed: bool,
    pub remote_hold_active: bool,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InputDiagnostic {
    /// Same owner-thread observation point as the containing snapshot.
    pub observation_generation: u64,
    pub observed_at_us: u64,
    /// None means unavailable, including protected or unprojected recipients.
    pub keyboard: Option<InputDeviceDiagnostic>,
    pub pointer: Option<InputDeviceDiagnostic>,
    /// Current native or projected hosted-application hit, independent of the
    /// grab recipient. None means absent device or protected/unprojected target.
    pub pointer_hit_test: Option<PointerHitTestDiagnostic>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PointerHitTestDiagnostic {
    /// An ordinary window in this snapshot; None means no native input surface
    /// at the pointer (for example a server decoration or shell surface).
    /// Ordinary shell hits use surface. No raw coordinates are exposed.
    pub window: Option<String>,
    /// Current ordinary shell hit; this is not a captured-pointer recipient.
    pub surface: Option<crate::leases::ResourceId>,
    /// Live bounded semantic tree and ordinal under the pointer for hosted UI.
    /// Both are absent for native clients, decoration-only hits, or a tree with
    /// no actionable/semantic node at this point.
    pub semantic_tree_generation: Option<u64>,
    pub semantic_node: Option<u64>,
    /// Nickel-owned hosted-window frame hit. Fixed roles expose no title,
    /// geometry, cursor position, or client content.
    pub decoration: Option<InternalDecorationHit>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InternalDecorationHit {
    Titlebar,
    Minimize,
    Maximize,
    Close,
    ResizeNorth,
    ResizeNorthEast,
    ResizeEast,
    ResizeSouthEast,
    ResizeSouth,
    ResizeSouthWest,
    ResizeWest,
    ResizeNorthWest,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DurationBucket {
    pub upper_bound_seconds: f64,
    pub cumulative_count: u64,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MethodMetrics {
    /// One of the fixed, server-owned MCP method names; never a caller label.
    pub method: String,
    pub success: u64,
    pub error: u64,
    pub cancelled: u64,
    pub in_flight: u64,
    pub duration_seconds_sum: f64,
    /// Finite cumulative bounds; success + error + cancelled is the +Inf bucket.
    pub duration_buckets: Vec<DurationBucket>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct OperationMetricsSnapshot {
    /// Collector generation, incremented on each method start and completion.
    pub generation: u64,
    /// Monotonic microseconds since this collector was created, not session time.
    pub collector_uptime_us: u64,
    /// Sampled together under the collector lock at the owner observation point.
    /// Covers typed MCP futures, not application processing or response delivery.
    pub methods: Vec<MethodMetrics>,
    /// Bounded method completions, oldest first, sampled under the same collector lock.
    /// Contains no individual input events, caller identities, arguments or error text.
    pub recent_completions: Vec<OperationCompletion>,
    /// Completions evicted since this collector started; history is never persisted.
    pub evicted_completions: u64,
}

pub const MAX_RECENT_OPERATION_COMPLETIONS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OperationOutcome {
    Success,
    Error,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct OperationCompletion {
    /// Collector generation at completion; unrelated to scene or presentation generation.
    pub generation: u64,
    pub collector_uptime_us: u64,
    /// Fixed server-owned method name, never supplied by a caller.
    pub method: String,
    pub outcome: OperationOutcome,
    /// Method future duration, excluding response delivery; cancellation is not rollback.
    pub duration_us: u64,
    /// First successful resource authorization in this method, not proof that
    /// a native effect completed. Absent for pre-authorization rejection.
    pub authorization: Option<OperationAuthorization>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct OperationAuthorization {
    /// Session-local numeric audit identity, never a caller label or credential.
    pub client_id: Option<u64>,
    pub lease_id: u64,
    pub operation_id: u64,
    pub lease_operation_generation: u64,
}

#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompositorBackend {
    Winit,
    Udev,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PlatformDiagnostic {
    pub observation_generation: u64,
    pub observed_at_us: u64,
    /// Runtime backend owner, not compiled-in feature availability. None during setup.
    pub backend: Option<CompositorBackend>,
    /// Logical compositor seat capabilities, not counts of physical devices.
    pub keyboard_present: bool,
    pub pointer_present: bool,
    pub touch_present: bool,
    pub xwayland_connected: bool,
    pub xwayland_restart_pending: bool,
    /// Successful private-device allocation at the current XWM's startup.
    /// Does not imply an independently queried device is still live.
    pub isolated_x11_keyboard_initialized: bool,
    /// Worker creation succeeded; no blocking health probe is performed here.
    pub native_keyboard_worker_initialized: bool,
}

#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShellDiagnosticRole {
    Desktop,
    Panel,
    Launcher,
    ControlCenter,
    Notification,
    VolumeOsd,
    WindowPreview,
    WindowContextMenu,
    Screenshot,
    OnScreenKeyboard,
}

/// Geometry/scene metadata only; never includes shell text, pixels or semantic values.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ShellSurfaceDiagnostic {
    pub id: String,
    pub generation: u64,
    pub role: ShellDiagnosticRole,
    pub geometry: [i64; 4],
    pub output: Option<String>,
    /// Generation of the coordinator's production paint scene, not presented pixels.
    pub scene_generation: u64,
    /// Current compositor presenter scale, sampled with the placement.
    pub scale_factor: f32,
    /// Production presenter has changes pending; false does not prove presentation.
    pub redraw_pending: bool,
    pub keyboard_focused: bool,
}

/// One compositor-hosted application surface at the snapshot observation point.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InternalApplicationDiagnostic {
    /// Diagnostic identity only; this record does not grant surface authority.
    pub id: String,
    pub generation: u64,
    /// Refers to a window in this same bounded snapshot.
    pub window: String,
    pub geometry: [i64; 4],
    pub output: Option<String>,
    pub scale_factor: f32,
    pub visible: bool,
    /// Resolved UI tree generation, not the generation of presented pixels.
    pub resolved_frame_generation: u64,
    /// Production presenter has changes pending; false does not confirm presentation.
    pub redraw_pending: bool,
    pub keyboard_focused: bool,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RendererPolicy {
    Gpu,
    Software,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RendererFallbackReason {
    RequestedSoftware,
    UnsupportedCommands,
    ElementBudget,
    TextureImportFailure,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct InternalRendererDiagnostic {
    pub surface: String,
    pub surface_generation: u64,
    pub observed_at_us: u64,
    /// Current production path, or suspended while hidden; not GPU presentation confirmation.
    pub mode: String,
    pub configured_mode: RendererPolicy,
    pub fallback_reason: Option<RendererFallbackReason>,
    pub gpu_frames: u64,
    pub fallback_frames: u64,
    pub software_frame_bytes: u64,
    pub fallback_raster_bytes: u64,
    pub fallback_buffer_creations: u64,
    pub fallback_buffer_reuses: u64,
    pub fallback_upload_damage_bytes: u64,
    pub fallback_full_repaints: u64,
    pub fallback_partial_repaints: u64,
    pub texture_import_failures: u64,
    pub fallback_import_failures: u64,
    /// Successful native presentation commits observed by this presenter.
    /// None means the backend does not distinguish rendering from presentation.
    pub presentation_generation: Option<u64>,
    /// Native presentation attempts that failed before commit. None means the
    /// backend does not own a truthful presentation-failure counter.
    pub presentation_failures: Option<u64>,
}

/// The existing shell-behavior transaction domain, excluding security settings.
/// Values use the same production read/default policy as local Settings.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ShellBehaviorDiagnostic {
    pub observation_generation: u64,
    pub observed_at_us: u64,
    /// Output-topology version used by local compare-and-set transactions.
    pub topology_generation: u64,
    pub bar_on_all_displays: bool,
    pub all_windows_on_every_bar: bool,
    /// Configured value; loading defaults does not prove a persisted file exists.
    pub configured_desktop_count: u8,
    /// Actual workspace owner state, which can differ before reconciliation.
    pub runtime_desktop_count: usize,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct BackgroundWorkerDiagnostic {
    /// Changes on admission and release, independently of MCP future completion.
    pub generation: u64,
    pub collector_uptime_us: u64,
    pub last_changed_uptime_us: u64,
    /// Includes blocking preparation and its owner reply wait. A timed-out
    /// network request can leave this true until its worker finishes cleanup.
    pub busy: bool,
}

pub type SettingsWorkerDiagnostic = BackgroundWorkerDiagnostic;

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ApplicationLaunchDiagnostic {
    /// Shared bounded preparation for launch and application-scoped enumeration.
    pub preparation: Option<BackgroundWorkerDiagnostic>,
    /// Child handles awaiting nonblocking exit collection, including running apps.
    pub tracked_children: usize,
    pub child_capacity: usize,
}

/// Latest successfully validated external accessibility traversal. The tree,
/// provider, application and window identities are deliberately not retained.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ExternalAccessibilityDiagnostic {
    pub operation_id: u64,
    pub scope: crate::native_semantics::NativeSemanticScope,
    pub observation_started_at_us: u64,
    pub observed_at_us: u64,
    pub owner_validated_at_us: u64,
    pub nodes: u32,
    pub truncated: bool,
    pub stale: bool,
}

impl ExternalAccessibilityDiagnostic {
    pub const FRESH_FOR_US: u64 = 5_000_000;

    pub fn retained_at(&self, observed_at_us: u64) -> Self {
        let mut retained = self.clone();
        retained.stale =
            observed_at_us.saturating_sub(self.owner_validated_at_us) > Self::FRESH_FOR_US;
        retained
    }
}

/// Warning/error source metadata only; no formatted messages, fields, or span values.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DiagnosticLogRecord {
    pub generation: u64,
    pub observed_at_us: u64,
    pub level: String,
    pub target: String,
    pub source_file: Option<String>,
    pub source_line: Option<u32>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DiagnosticLogSnapshot {
    pub collecting: bool,
    pub generation: u64,
    /// Monotonic microseconds since logging collector initialization.
    pub observed_at_us: u64,
    pub evicted: u64,
    pub contention_drops: u64,
    /// At most 256 warning/error locations, ordered by generation.
    pub records: Vec<DiagnosticLogRecord>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TraceLifecycleTransition {
    Started,
    Stopped,
    TimedOut,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct TraceLifecycleDiagnostic {
    pub generation: u64,
    pub observed_at_us: u64,
    pub category: crate::frame_trace::FrameTraceCategory,
    pub transition: TraceLifecycleTransition,
    pub duration_limit_seconds: u16,
    pub elapsed_us: u64,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct TraceLifecycleSnapshot {
    pub evicted: u64,
    pub events: Vec<TraceLifecycleDiagnostic>,
}

pub fn trace_lifecycle_snapshot(
    permit: &crate::DesktopPermit,
    session_start: std::time::Instant,
) -> Option<TraceLifecycleSnapshot> {
    let (events, evicted) = permit.trace_audit.as_ref()?.snapshot()?;
    Some(project_trace_lifecycle(events, evicted, session_start))
}

fn project_trace_lifecycle(
    events: Vec<crate::trace_audit::Event>,
    evicted: u64,
    session_start: std::time::Instant,
) -> TraceLifecycleSnapshot {
    TraceLifecycleSnapshot {
        evicted,
        events: events
            .into_iter()
            .map(|event| TraceLifecycleDiagnostic {
                generation: event.generation,
                observed_at_us: event
                    .observed_at
                    .saturating_duration_since(session_start)
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
                category: event.category,
                transition: match event.transition {
                    crate::trace_audit::Transition::Started => TraceLifecycleTransition::Started,
                    crate::trace_audit::Transition::Stopped => TraceLifecycleTransition::Stopped,
                    crate::trace_audit::Transition::TimedOut => TraceLifecycleTransition::TimedOut,
                    crate::trace_audit::Transition::Cancelled => {
                        TraceLifecycleTransition::Cancelled
                    }
                },
                duration_limit_seconds: event.duration_limit_seconds,
                elapsed_us: event.elapsed_us,
            })
            .collect(),
    }
}

/// Aggregate production HTTP admission state. No client table or identifiers.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct AdmissionDiagnostic {
    pub generation: u64,
    /// Timestamp relative to this listener's admission collector, not session start.
    pub collector_uptime_us: u64,
    pub requests_admitted: u64,
    /// Combined global and authenticated-client admission rejections.
    pub admission_rejections: u64,
    pub active_requests: u64,
    pub active_authenticated_requests: u64,
}

/// Fixed-cardinality lease counts shared with public operational metrics.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct LeaseMetricsDiagnostic {
    /// Monotonic microseconds since compositor start, supplied at collection.
    pub observed_at_us: u64,
    /// Distinct authenticated identities with at least one ready, live watch.
    pub active_connections: u64,
    /// Scope order is surface, window, application, output, full_session.
    pub active_by_scope: [u64; 5],
    pub active_total: u64,
    pub pending_requests: u64,
}

/// Owned CPU image caches only. Byte totals are retained pixel storage, not RSS
/// or GPU allocation. Preview totals include only windows in this snapshot.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ShellImageCacheDiagnostic {
    pub observation_generation: u64,
    pub observed_at_us: u64,
    pub launcher_icon_entries: u64,
    pub launcher_icon_bytes: u64,
    pub wallpaper_entries: u64,
    pub wallpaper_bytes: u64,
    pub tray_entries: u64,
    pub tray_bytes: u64,
    pub preview_entries: u64,
    pub preview_bytes: u64,
}

/// Process-level accounting from the production shell's shared presenter cache.
/// These aggregates contain no cache keys, pixels, text, native handles, or
/// per-surface attribution. Byte values are cache-owned retained estimates,
/// not allocator or GPU memory measurements.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SharedPresenterCacheDiagnostic {
    pub observation_generation: u64,
    pub observed_at_us: u64,
    /// Advances whenever the shared presenter may have changed its cache state.
    pub cache_generation: u64,
    pub cache_owners: u64,
    pub live_entries: u64,
    pub live_bytes: u64,
    pub peak_cache_bytes: u64,
    pub hits: u64,
    pub misses: u64,
    pub insertions: u64,
    pub evictions: u64,
    pub invalidations: u64,
    pub recomputation_nanos: u64,
}

/// Aggregate retained storage derived only from the protected-filtered
/// renderers and shell caches published in this same snapshot. Shared GPU
/// caches and external client renderer allocations remain unavailable.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ProjectedResourceDiagnostic {
    pub observation_generation: u64,
    pub observed_at_us: u64,
    pub renderer_surfaces: u64,
    pub software_frame_bytes: u64,
    pub fallback_raster_bytes: u64,
    pub shell_image_entries: u64,
    pub shell_image_bytes: u64,
}

/// Payload-free compositor work awaiting reconciliation. Counts describe
/// production-owned queues at one observation point; they expose neither
/// clipboard contents, launch commands, output identities, nor shell targets.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PendingEffectsDiagnostic {
    pub observation_generation: u64,
    pub observed_at_us: u64,
    pub desktop_scene_updates: u64,
    pub image_copy_frames: u64,
    pub launch_observations: u64,
    pub output_retirements: u64,
    pub shell_focus_pending: bool,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DiagnosticSnapshot {
    pub observation_generation: u64,
    /// Monotonic microseconds since the current compositor session started.
    pub observed_at_us: u64,
    pub windows: Vec<crate::WindowSummary>,
    pub outputs: Vec<OutputDiagnostic>,
    pub workspaces: Vec<WorkspaceDiagnostic>,
    /// Unprotected hosted applications whose windows are in this snapshot.
    pub internal_applications: Vec<InternalApplicationDiagnostic>,
    /// Cumulative production renderer work for the allowed applications above.
    /// Shared caches, text payloads and presentation completion are not projected.
    pub internal_renderers: Vec<InternalRendererDiagnostic>,
    /// Per-surface renderer work for the visible, unprotected shell surfaces below.
    /// Excludes shared caches and does not confirm GPU presentation.
    pub shell_renderers: Vec<InternalRendererDiagnostic>,
    /// None means the in-process shell is unavailable.
    pub shell_image_cache: Option<ShellImageCacheDiagnostic>,
    /// None means the production shell owner cannot safely account for its
    /// shared presenter caches. This process aggregate is never attributed to
    /// the protected-filtered per-surface renderer records.
    pub shared_presenter_cache: Option<SharedPresenterCacheDiagnostic>,
    /// Current protected-filtered resource totals from the fields above.
    pub projected_resources: ProjectedResourceDiagnostic,
    /// Pending production effects without their targets or payloads.
    pub pending_effects: PendingEffectsDiagnostic,
    pub shell_surfaces: Vec<ShellSurfaceDiagnostic>,
    pub focused_window: Option<String>,
    pub input: InputDiagnostic,
    pub shortcuts: ShortcutDiagnostic,
    pub stacking_front_to_back: Vec<String>,
    pub preview: PreviewDiagnostic,
    /// None explicitly means the bounded operational collector is unavailable.
    pub metrics: Option<OperationMetricsSnapshot>,
    /// None means the listener collector is absent or busy; collection never waits.
    pub admission: Option<AdmissionDiagnostic>,
    /// None means the control collector was busy at its independent observation.
    pub lease_metrics: Option<LeaseMetricsDiagnostic>,
    pub platform: PlatformDiagnostic,
    /// Latest bounded production query for each allowlisted platform domain.
    pub platform_refreshes: Vec<PlatformRefreshOutcome>,
    /// Latest bounded production installed-application catalog refresh.
    pub application_inventory_refresh: Option<ApplicationInventoryRefreshOutcome>,
    /// Current compositor-owned optional-feature projection; no source paths,
    /// account data, project data, or provider diagnostics are retained.
    pub codex_feature: Option<CodexFeatureDiagnostic>,
    pub shell_behavior: ShellBehaviorDiagnostic,
    /// None means the worker-state collector is unavailable.
    pub settings_worker: Option<SettingsWorkerDiagnostic>,
    /// Shared bounded worker for application and platform diagnostic refreshes.
    pub diagnostic_worker: Option<SettingsWorkerDiagnostic>,
    pub application_launch: ApplicationLaunchDiagnostic,
    /// Latest bounded external accessibility observation, if one completed.
    pub external_accessibility: Option<ExternalAccessibilityDiagnostic>,
    /// Bounded protected-safe window, output, workspace, shell, focus, input and
    /// production-effect transitions.
    pub recent_events: crate::desktop_events::DesktopEventSnapshot,
    /// None means the collector is unavailable or busy; never reads log files.
    pub diagnostic_logs: Option<DiagnosticLogSnapshot>,
    pub frame_trace: Option<crate::frame_trace::FrameTraceSnapshot>,
    /// Payload-free lifecycle for bounded traces across clients. Client, lease
    /// and trace identities are removed at projection time.
    pub trace_lifecycle: Option<TraceLifecycleSnapshot>,
    pub truncated: bool,
    /// Explicitly identifies domains not supplied by this projection.
    pub unavailable_domains: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeatureInstallationDiagnostic {
    Installed,
    Missing,
    Incompatible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeatureHealthDiagnostic {
    Unknown,
    Loading,
    SignedOut,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct CodexFeatureDiagnostic {
    pub observation_generation: u64,
    pub observed_at_us: u64,
    pub supported: bool,
    pub installation: FeatureInstallationDiagnostic,
    pub enabled: bool,
    pub health: FeatureHealthDiagnostic,
    pub configuration_generation: u64,
}

/// Installed launch targets from the production catalog, never executable arguments.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct InstalledApplication {
    pub id: String,
    pub name: String,
    /// Executable identity observed during catalog preparation, usable in an
    /// application lease request before any window exists. Launch and window
    /// ownership are independently revalidated; this is not a standing grant.
    /// None includes scripts, shared runtimes and unverifiable launch targets.
    #[serde(default)]
    pub verified_application: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ApplicationInventory {
    pub observation_generation: u64,
    pub observed_at_us: u64,
    /// Executable inspection may precede the final owner observation.
    pub catalog_observed_at_us: u64,
    pub catalog_generation: u64,
    pub available: bool,
    pub applications: Vec<InstalledApplication>,
    pub truncated: bool,
}

pub const MAX_INSTALLED_APPLICATIONS: usize = 512;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LaunchApplicationRequest {
    pub lease_id: u64,
    pub catalog_generation: u64,
    pub application_id: String,
}

/// Bounded launch acknowledgement, not a new control grant.
///
/// Process creation and requested output placement are reported independently:
/// accepting a native spawn does not prove that a resulting window mapped, or
/// that its first-map placement completed.
#[derive(
    Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct LaunchApplicationOutcome {
    pub catalog_generation: u64,
    pub application_id: String,
    /// True once the production desktop owner accepted the launch request.
    pub requested: bool,
    /// True only when the platform confirmed native process creation.
    pub process_spawn_confirmed: bool,
    /// Native process identity when the platform can confirm one.
    pub process_id: Option<u32>,
    /// Exact generation-bearing output requested by an output-scoped lease.
    /// None means no output placement was requested.
    pub output_requested: Option<crate::leases::ResourceId>,
    /// True only after production output placement has completed. A pending
    /// first-map association is not confirmation.
    pub output_confirmed: bool,
}

#[cfg(test)]
mod launch_application_contract_tests {
    use super::*;

    #[test]
    fn launch_outcome_keeps_spawn_and_output_confirmation_independent() {
        let output = crate::leases::ResourceId {
            id: "DP-2".into(),
            generation: 7,
        };
        let outcome = LaunchApplicationOutcome {
            catalog_generation: 11,
            application_id: "org.example.Editor".into(),
            requested: true,
            process_spawn_confirmed: true,
            process_id: Some(42),
            output_requested: Some(output.clone()),
            output_confirmed: false,
        };

        let value = serde_json::to_value(&outcome).unwrap();
        assert_eq!(value["output_requested"]["id"], "DP-2");
        assert_eq!(value["output_requested"]["generation"], 7);
        assert_eq!(value["process_id"], 42);
        assert_eq!(
            serde_json::from_value::<LaunchApplicationOutcome>(value).unwrap(),
            outcome
        );
    }

    #[test]
    fn launch_outcome_can_truthfully_report_unconfirmed_native_evidence() {
        let outcome: LaunchApplicationOutcome = serde_json::from_value(serde_json::json!({
            "catalog_generation": 11,
            "application_id": "org.example.Editor",
            "requested": true,
            "process_spawn_confirmed": false,
            "process_id": null,
            "output_requested": null,
            "output_confirmed": false
        }))
        .unwrap();

        assert!(!outcome.process_spawn_confirmed);
        assert_eq!(outcome.process_id, None);
        assert_eq!(outcome.output_requested, None);
        assert!(!outcome.output_confirmed);
    }

    #[test]
    fn launch_outcome_rejects_undeclared_confirmation_fields() {
        assert!(
            serde_json::from_value::<LaunchApplicationOutcome>(serde_json::json!({
                "catalog_generation": 11,
                "application_id": "org.example.Editor",
                "requested": true,
                "process_spawn_confirmed": true,
                "process_id": 42,
                "output_requested": null,
                "output_confirmed": false,
                "window_mapped": true
            }))
            .is_err()
        );
    }
}

/// Static registration metadata only. No key events, held state or emergency controls.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ShortcutDiagnostic {
    pub observation_generation: u64,
    pub observed_at_us: u64,
    /// None when the backend is unavailable or the revision counter exhausted.
    pub registration_revision: Option<u64>,
    pub capability: ShortcutDiagnosticCapability,
    pub registrations: Vec<ShortcutRegistrationDiagnostic>,
    /// String-bearing logical/native bindings among the bounded inspected prefix.
    /// Their text is never projected; truncation also bounds inspection work.
    pub unprojected_bindings: u64,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutDiagnosticCapability {
    Available,
    BackendUnavailable,
    RevisionExhausted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ShortcutRegistrationDiagnostic {
    pub registration_id: u64,
    /// Fixed physical key enum name, never a logical character or native string.
    pub physical_key: String,
    /// Fixed product action enum name (workspace actions may include an index).
    pub action: String,
    pub modifiers: Vec<String>,
    pub trigger: String,
}

pub const MAX_DIAGNOSTIC_SHORTCUTS: usize = 128;

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct OutputIdentificationOutcome {
    pub output: crate::leases::ResourceId,
    pub generation: u64,
    pub duration_ms: u32,
}

#[cfg(test)]
mod output_identification_tests {
    use super::*;

    #[test]
    fn output_identification_requires_a_bounded_exact_identity_and_no_custom_label() {
        let action: DiagnosticAction = serde_json::from_value(serde_json::json!({
            "identify_output": {"output": {"id": "DP-2", "generation": 7}}
        }))
        .unwrap();
        assert!(action.validate().is_ok());
        for (id, generation) in [
            (String::new(), 7),
            ("DP-2".into(), 0),
            ("x".repeat(1024), 7),
        ] {
            assert!(
                DiagnosticAction::IdentifyOutput {
                    output: crate::leases::ResourceId { id, generation }
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            serde_json::from_value::<DiagnosticAction>(serde_json::json!({
                "identify_output": {"output": {"id": "DP-2", "generation": 7}, "label": "trusted"}
            }))
            .is_err()
        );
    }

    #[test]
    fn application_refresh_has_no_path_or_scan_policy_payload() {
        let action: DiagnosticAction =
            serde_json::from_value(serde_json::json!("refresh_application_inventory")).unwrap();
        assert!(matches!(
            action,
            DiagnosticAction::RefreshApplicationInventory
        ));
        let audio: DiagnosticAction = serde_json::from_value(serde_json::json!({
            "refresh_platform_status": {"domain": "audio"}
        }))
        .unwrap();
        assert!(matches!(
            audio,
            DiagnosticAction::RefreshPlatformStatus {
                domain: PlatformRefreshDomain::Audio
            }
        ));
        let peripherals: DiagnosticAction = serde_json::from_value(serde_json::json!({
            "refresh_platform_status": {"domain": "peripherals"}
        }))
        .unwrap();
        assert!(matches!(
            peripherals,
            DiagnosticAction::RefreshPlatformStatus {
                domain: PlatformRefreshDomain::Peripherals
            }
        ));
        let maintenance: DiagnosticAction = serde_json::from_value(serde_json::json!({
            "refresh_platform_status": {"domain": "maintenance"}
        }))
        .unwrap();
        assert!(matches!(
            maintenance,
            DiagnosticAction::RefreshPlatformStatus {
                domain: PlatformRefreshDomain::Maintenance
            }
        ));
        let associations: DiagnosticAction = serde_json::from_value(serde_json::json!({
            "refresh_platform_status": {"domain": "default_associations"}
        }))
        .unwrap();
        assert!(matches!(
            associations,
            DiagnosticAction::RefreshPlatformStatus {
                domain: PlatformRefreshDomain::DefaultAssociations
            }
        ));
        for field in ["path", "root", "limit", "executable", "icon_theme"] {
            assert!(
                serde_json::from_value::<DiagnosticAction>(serde_json::json!({
                    "refresh_application_inventory": {(field): true}
                }))
                .is_err()
            );
        }
    }

    #[test]
    fn retained_application_inventory_reports_its_own_freshness() {
        let refresh = ApplicationInventoryRefreshOutcome {
            generation: 8,
            observation_started_at_us: 100,
            observed_at_us: 140,
            preparation_duration_us: 40,
            stale: false,
            applications: 12,
            partial: true,
            reconciliation_confirmed: true,
        };
        assert!(
            !refresh
                .retained_at(
                    refresh.observed_at_us + ApplicationInventoryRefreshOutcome::FRESH_FOR_US
                )
                .stale
        );
        let retained = refresh.retained_at(
            refresh.observed_at_us + ApplicationInventoryRefreshOutcome::FRESH_FOR_US + 1,
        );
        assert!(retained.stale);
        let value = serde_json::to_value(retained).unwrap();
        for excluded in ["application_ids", "paths", "errors", "scan_roots"] {
            assert!(value.get(excluded).is_none());
        }
    }

    #[test]
    fn platform_refresh_is_an_exact_allowlisted_domain_without_payloads() {
        let action: DiagnosticAction = serde_json::from_value(serde_json::json!({
            "refresh_platform_status": {"domain": "connectivity"}
        }))
        .unwrap();
        assert!(matches!(
            action,
            DiagnosticAction::RefreshPlatformStatus {
                domain: PlatformRefreshDomain::Connectivity
            }
        ));
        for value in [
            serde_json::json!({"refresh_platform_status": {"domain": "all"}}),
            serde_json::json!({"refresh_platform_status": {"domain": "permissions"}}),
            serde_json::json!({"refresh_platform_status": {"domain": "connectivity", "path": "/"}}),
        ] {
            assert!(serde_json::from_value::<DiagnosticAction>(value).is_err());
        }
    }

    #[test]
    fn retained_platform_refresh_marks_age_without_adding_provider_payloads() {
        let refresh = PlatformRefreshOutcome {
            domain: PlatformRefreshDomain::Connectivity,
            generation: 3,
            observation_started_at_us: 10,
            observed_at_us: 20,
            preparation_duration_us: 10,
            stale: false,
            network_available: true,
            bluetooth_available: false,
            audio_available: false,
            printers_available: false,
            volumes_available: false,
            filesystems_available: false,
            printer_count: 0,
            volume_count: 0,
            filesystem_count: 0,
            maintenance_available: false,
            updates_available: None,
            restart_required: None,
            firewall_healthy: None,
            malware_protection_healthy: None,
            known_permission_states: 0,
            secure_storage_status_available: false,
            associations_available: false,
            association_targets_queried: 0,
            effective_associations: 0,
            directly_writable_associations: 0,
            partial: false,
            reconciliation_confirmed: true,
        };
        assert!(
            !refresh
                .retained_at(refresh.observed_at_us + PlatformRefreshOutcome::FRESH_FOR_US)
                .stale
        );
        let retained =
            refresh.retained_at(refresh.observed_at_us + PlatformRefreshOutcome::FRESH_FOR_US + 1);
        assert!(retained.stale);
        let value = serde_json::to_value(retained).unwrap();
        assert_eq!(value["observation_started_at_us"], 10);
        assert_eq!(value["observed_at_us"], 20);
        for excluded in ["provider", "path", "error", "ssid", "device_name"] {
            assert!(value.get(excluded).is_none());
        }
    }

    #[test]
    fn trace_lifecycle_projection_removes_owner_and_trace_identity() {
        let start = std::time::Instant::now();
        let snapshot = project_trace_lifecycle(
            vec![crate::trace_audit::Event {
                generation: 9,
                observed_at: start + std::time::Duration::from_micros(12),
                client_id: 111,
                lease_id: 222,
                trace_id: 333,
                category: crate::frame_trace::FrameTraceCategory::NestedFrameDispatch,
                transition: crate::trace_audit::Transition::Stopped,
                duration_limit_seconds: 30,
                elapsed_us: 12,
            }],
            4,
            start,
        );
        let value = serde_json::to_value(snapshot).unwrap();
        assert_eq!(value["evicted"], 4);
        assert_eq!(value["events"][0]["observed_at_us"], 12);
        assert_eq!(value["events"][0]["transition"], "stopped");
        let event = value["events"][0].as_object().unwrap();
        for excluded in ["client_id", "lease_id", "trace_id", "client", "payload"] {
            assert!(!event.contains_key(excluded));
        }
    }

    #[test]
    fn retained_external_accessibility_reports_freshness_without_tree_identity_or_content() {
        let observation = ExternalAccessibilityDiagnostic {
            operation_id: 17,
            scope: crate::native_semantics::NativeSemanticScope::ApplicationConnection,
            observation_started_at_us: 10,
            observed_at_us: 20,
            owner_validated_at_us: 30,
            nodes: 42,
            truncated: true,
            stale: false,
        };
        assert!(
            !observation
                .retained_at(30 + ExternalAccessibilityDiagnostic::FRESH_FOR_US)
                .stale
        );
        let value = serde_json::to_value(
            observation.retained_at(31 + ExternalAccessibilityDiagnostic::FRESH_FOR_US),
        )
        .unwrap();
        assert_eq!(value["operation_id"], 17);
        assert_eq!(value["nodes"], 42);
        assert_eq!(value["stale"], true);
        for excluded in [
            "window",
            "application",
            "provider",
            "name",
            "description",
            "text",
            "value",
            "actions",
            "client_id",
            "lease_id",
        ] {
            assert!(value.get(excluded).is_none());
        }
    }

    #[test]
    fn projected_resources_contain_only_bounded_counts_and_bytes() {
        let resources = ProjectedResourceDiagnostic {
            observation_generation: 7,
            observed_at_us: 11,
            renderer_surfaces: 3,
            software_frame_bytes: 100,
            fallback_raster_bytes: 200,
            shell_image_entries: 4,
            shell_image_bytes: 300,
        };
        let value = serde_json::to_value(resources).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 7);
        for excluded in [
            "surface",
            "window",
            "application",
            "title",
            "path",
            "client_id",
            "lease_id",
            "gpu_allocation",
        ] {
            assert!(value.get(excluded).is_none());
        }
    }

    #[test]
    fn shared_presenter_cache_contains_only_aggregate_accounting() {
        let cache = SharedPresenterCacheDiagnostic {
            observation_generation: 7,
            observed_at_us: 11,
            cache_generation: 5,
            cache_owners: 1,
            live_entries: 3,
            live_bytes: 100,
            peak_cache_bytes: 200,
            hits: 9,
            misses: 2,
            insertions: 3,
            evictions: 1,
            invalidations: 4,
            recomputation_nanos: 50,
        };
        let value = serde_json::to_value(cache).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 13);
        for excluded in [
            "surface",
            "role",
            "window",
            "text",
            "pixels",
            "native_handle",
            "process_rss_bytes",
            "gpu_allocation",
        ] {
            assert!(value.get(excluded).is_none());
        }
    }
}
