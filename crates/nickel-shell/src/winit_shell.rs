//! Winit window and event ownership for the Nickel shell.
//!
//! Rendering is deliberately outside this module. A renderer receives stable
//! [`SurfaceId`] values and can attach either a software surface or an
//! accelerated backend without owning the application event pump.

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
#[cfg(target_os = "windows")]
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::Instant;

use nickel_input::InputEvent;
use nickel_session_protocol::ShellRole as SessionShellRole;
use nickel_ui::backend::PaintCommand;
use nickel_ui::{AggregatePresenterCacheDiagnostics, DamageRegion, HostChangeToken};
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{Event, WindowEvent};
#[cfg(not(target_os = "windows"))]
use winit::event_loop::EventLoopProxy;
use winit::event_loop::{ControlFlow, EventLoop};
use winit::platform::pump_events::EventLoopExtPumpEvents;
#[cfg(target_os = "linux")]
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::window::{Window, WindowId};

use crate::softbuffer_presenter::{PresentationGeometry, SharedGraphics, SoftbufferPresenter};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{LPARAM, WPARAM};
#[cfg(target_os = "windows")]
use windows::Win32::System::Threading::GetCurrentThreadId;
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    MsgWaitForMultipleObjectsEx, PostThreadMessageW, QS_ALLINPUT, WM_APP,
};

pub const DESKTOP_TITLE: &str = "Nickel Desktop";
pub const PANEL_TITLE: &str = "Nickel Panel";
pub const LAUNCHER_TITLE: &str = "Nickel Launcher";
pub const CONTROL_CENTER_TITLE: &str = "Nickel Control Center";
pub const NOTIFICATION_TITLE: &str = "Nickel Notification";
pub const VOLUME_OSD_TITLE: &str = "Nickel Volume";
pub const WINDOW_PREVIEW_TITLE: &str = "Nickel Window Preview";
pub const WINDOW_CONTEXT_MENU_TITLE: &str = "Nickel Window Menu";
pub const CODEX_PROJECT_MENU_TITLE: &str = "Nickel Codex Projects";
pub const LOCK_TITLE: &str = "Nickel Lock";
pub const SCREENSHOT_TITLE: &str = "Nickel Screenshot";

#[cfg(any(test, target_os = "windows"))]
fn windows_wait_timeout_millis(timeout: Duration) -> u32 {
    const MAX_FINITE_WAIT_MS: u128 = u32::MAX as u128 - 1;
    let nanos = timeout.as_nanos();
    let rounded_up = nanos.saturating_add(999_999) / 1_000_000;
    rounded_up.min(MAX_FINITE_WAIT_MS) as u32
}
pub const PANEL_HEIGHT: u32 = 56;
const RUNTIME_SAMPLE_CAPACITY: usize = 64;
const OUTPUT_RETIREMENT_SETTLE: Duration = Duration::from_millis(500);
const OUTPUT_CREATION_RETRY_MIN: Duration = Duration::from_millis(50);
const OUTPUT_CREATION_RETRY_MAX: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PanelEdge {
    Top,
    #[default]
    Bottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellOptions {
    pub create_desktop_surfaces: bool,
    pub panel_edge: PanelEdge,
    pub bar_on_all_displays: bool,
}

impl Default for ShellOptions {
    fn default() -> Self {
        Self {
            create_desktop_surfaces: true,
            panel_edge: PanelEdge::Bottom,
            bar_on_all_displays: true,
        }
    }
}

#[derive(Default)]
struct OutputRetirementTracker {
    missing_since: HashMap<String, Instant>,
}

#[derive(Default)]
struct OutputCreationRetry {
    failures: u32,
    deadline: Option<Instant>,
}

impl OutputCreationRetry {
    fn failed(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        let multiplier = 1_u32 << self.failures.saturating_sub(1).min(6);
        self.deadline = Some(
            now + OUTPUT_CREATION_RETRY_MIN
                .saturating_mul(multiplier)
                .min(OUTPUT_CREATION_RETRY_MAX),
        );
    }

    fn succeeded(&mut self) {
        self.failures = 0;
        self.deadline = None;
    }
}

impl OutputRetirementTracker {
    fn observe<'a>(
        &mut self,
        now: Instant,
        live_outputs: &[String],
        owned_outputs: impl IntoIterator<Item = &'a str>,
    ) -> Vec<String> {
        self.missing_since
            .retain(|output, _| !live_outputs.iter().any(|live| live == output));

        for output in owned_outputs {
            if live_outputs.iter().any(|live| live == output) {
                self.missing_since.remove(output);
            } else {
                self.missing_since.entry(output.to_owned()).or_insert(now);
            }
        }

        self.missing_since
            .iter()
            .filter(|(_, missing_since)| {
                now.saturating_duration_since(**missing_since) >= OUTPUT_RETIREMENT_SETTLE
            })
            .map(|(output, _)| output.clone())
            .collect()
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.missing_since
            .values()
            .map(|missing_since| *missing_since + OUTPUT_RETIREMENT_SETTLE)
            .min()
    }
}

fn push_bounded(samples: &mut VecDeque<u64>, sample: u64) {
    if samples.len() == RUNTIME_SAMPLE_CAPACITY {
        samples.pop_front();
    }
    samples.push_back(sample);
}

fn durable_presenter_peak(
    previous_peak_bytes: usize,
    current: &AggregatePresenterCacheDiagnostics,
) -> usize {
    previous_peak_bytes.max(current.peak_cache_bytes)
}

fn desired_output_surfaces(
    output_names: &[String],
    create_desktops: bool,
    bar_on_all_displays: bool,
    primary_output: Option<&str>,
) -> HashSet<(String, SurfaceRole)> {
    let panel_outputs = panel_outputs(output_names, bar_on_all_displays, primary_output)
        .into_iter()
        .collect::<HashSet<_>>();
    output_names
        .iter()
        .flat_map(|output| {
            [SurfaceRole::Desktop, SurfaceRole::Panel, SurfaceRole::Lock]
                .into_iter()
                .filter(|role| {
                    (*role != SurfaceRole::Desktop || create_desktops)
                        && (*role != SurfaceRole::Panel || panel_outputs.contains(output))
                })
                .map(|role| (output.clone(), role))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SurfaceId(WindowId);

#[derive(Debug)]
pub enum ShellUserEvent {
    GlobalShortcut(crate::platform::GlobalShortcut),
    #[cfg(target_os = "linux")]
    TestControl(crate::platform::ShellTestRequest),
}

/// Cross-thread sender for events consumed by the shell runtime.
///
/// Windows uses a Nickel-owned queue and a checked thread-message wake. Winit's
/// Windows proxy queues the payload internally but discards the result of its
/// `PostMessageW` wake, which can report success while leaving the payload
/// stranded under compatibility runtimes. The queue keeps payload ownership
/// and wake delivery in one transaction.
#[derive(Clone)]
pub struct ShellEventSender {
    #[cfg(not(target_os = "windows"))]
    proxy: EventLoopProxy<ShellUserEvent>,
    #[cfg(target_os = "windows")]
    queue: Arc<Mutex<VecDeque<ShellUserEvent>>>,
    #[cfg(target_os = "windows")]
    event_thread: u32,
}

impl ShellEventSender {
    pub fn send_event(&self, event: ShellUserEvent) -> Result<(), ShellUserEvent> {
        #[cfg(not(target_os = "windows"))]
        {
            self.proxy.send_event(event).map_err(|error| error.0)
        }
        #[cfg(target_os = "windows")]
        {
            let Ok(mut queue) = self.queue.lock() else {
                return Err(event);
            };
            queue.push_back(event);
            // SAFETY: `event_thread` is captured while constructing the winit loop on
            // its owning thread. Winit has already created that thread's message queue.
            if unsafe {
                PostThreadMessageW(
                    self.event_thread,
                    WM_APP + 0x4e,
                    WPARAM::default(),
                    LPARAM::default(),
                )
            }
            .is_err()
            {
                return Err(queue.pop_back().expect("just queued shell event"));
            }
            Ok(())
        }
    }
}

pub trait WinitWindowCompat {
    fn size(&self) -> (u32, u32);
    fn has_input_focus(&self) -> bool;
}

impl WinitWindowCompat for Window {
    fn size(&self) -> (u32, u32) {
        let size = self.inner_size().to_logical::<u32>(self.scale_factor());
        (size.width, size.height)
    }

    fn has_input_focus(&self) -> bool {
        self.has_focus()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShellMemoryDiagnostics {
    /// Cache-owned bytes reported by every currently instantiated surface presenter.
    pub presenter_caches: AggregatePresenterCacheDiagnostics,
    /// Allocator/process-visible resident bytes from the operating system.
    /// This is intentionally independent of `presenter_caches.live_bytes`.
    pub process_rss_bytes: Option<usize>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShellRuntimeDiagnostics {
    /// Completed presents after the presenter was initialized, in microseconds.
    pub warm_present_us: Vec<u64>,
    /// Input receipt through the first synchronous present it caused, in microseconds.
    pub input_to_present_us: Vec<u64>,
    /// Process-wide allocation operations observed during each warm present.
    pub warm_present_allocations: Vec<u64>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SurfaceRole {
    Desktop,
    Panel,
    Launcher,
    ControlCenter,
    Notification,
    VolumeOsd,
    WindowPreview,
    WindowContextMenu,
    CodexProjectMenu,
    Lock,
    Screenshot,
    CodexChat,
}

#[cfg(target_os = "linux")]
fn surface_is_ephemeral(role: SurfaceRole) -> bool {
    matches!(
        role,
        SurfaceRole::Notification
            | SurfaceRole::VolumeOsd
            | SurfaceRole::WindowPreview
            | SurfaceRole::WindowContextMenu
            | SurfaceRole::CodexProjectMenu
            | SurfaceRole::Screenshot
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShellEvent {
    GlobalShortcut(crate::platform::GlobalShortcut),
    #[cfg(target_os = "linux")]
    TestControl(crate::platform::ShellTestRequest),
    Quit,
    Input {
        surface: SurfaceId,
        event: InputEvent,
    },
    FileDrop {
        surface: SurfaceId,
        path: std::path::PathBuf,
    },
    Shown(SurfaceId),
    Hidden(SurfaceId),
    CloseRequested(SurfaceId),
    FocusChanged {
        surface: SurfaceId,
        focused: bool,
    },
    PointerEntered {
        surface: SurfaceId,
        entered: bool,
    },
    LogicalResize {
        surface: SurfaceId,
        width: u32,
        height: u32,
    },
    PixelResize {
        surface: SurfaceId,
        width: u32,
        height: u32,
        scale: f32,
    },
    DisplayTopologyChanged,
    Redraw(SurfaceId),
}

pub struct ShellSurface {
    id: SurfaceId,
    role: SurfaceRole,
    application_id: String,
    display_index: usize,
    output_name: String,
    display_connected: bool,
    initial_exposed: bool,
    presenter: Option<SoftbufferPresenter>,
    last_host_change_token: Option<HostChangeToken>,
    visible: bool,
    window: Window,
}

impl ShellSurface {
    pub fn id(&self) -> SurfaceId {
        self.id
    }

    pub fn role(&self) -> SurfaceRole {
        self.role
    }

    pub fn display_index(&self) -> usize {
        self.display_index
    }

    pub fn output_name(&self) -> &str {
        &self.output_name
    }

    pub fn window(&self) -> &Window {
        &self.window
    }
}

pub struct WinitShell {
    // Presenters borrow native window handles and must drop before the event loop.
    surfaces: Vec<ShellSurface>,
    graphics: Option<SharedGraphics>,
    surface_indices: HashMap<WindowId, usize>,
    native_surface_indices: HashMap<WindowId, usize>,
    events: EventLoop<ShellUserEvent>,
    #[cfg(target_os = "windows")]
    external_events: Arc<Mutex<VecDeque<ShellUserEvent>>>,
    #[cfg(target_os = "windows")]
    event_thread: u32,
    pending_events: VecDeque<ShellEvent>,
    displays: Vec<(DisplayGeometry, String)>,
    input_adapters: HashMap<WindowId, nickel_input::winit::Adapter>,
    devices: nickel_input::winit::DeviceRegistry,
    warm_present_us: VecDeque<u64>,
    input_to_present_us: VecDeque<u64>,
    warm_present_allocations: VecDeque<u64>,
    presenter_cache_peak_bytes: Cell<usize>,
    output_retirements: OutputRetirementTracker,
    output_creation_retry: OutputCreationRetry,
    pending_input_started: Option<Instant>,
    clipboard: Option<std::cell::RefCell<arboard::Clipboard>>,
    started: Instant,
    options: ShellOptions,
    primary_output_name: Option<String>,
    active_output_name: Option<String>,
}

impl WinitShell {
    pub fn new(started: Instant) -> Result<Self, String> {
        Self::new_with_options(started, ShellOptions::default())
    }

    pub fn new_with_options(started: Instant, options: ShellOptions) -> Result<Self, String> {
        let events = EventLoop::<ShellUserEvent>::with_user_event()
            .build()
            .map_err(|error| error.to_string())?;
        #[cfg(target_os = "windows")]
        let external_events = Arc::new(Mutex::new(VecDeque::new()));
        tracing::info!(
            elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0,
            "winit event loop initialized"
        );
        Ok(Self {
            surfaces: Vec::new(),
            graphics: None,
            surface_indices: HashMap::new(),
            native_surface_indices: HashMap::new(),
            events,
            #[cfg(target_os = "windows")]
            external_events,
            #[cfg(target_os = "windows")]
            // SAFETY: querying the identifier of the current thread has no preconditions.
            event_thread: unsafe { GetCurrentThreadId() },
            pending_events: VecDeque::new(),
            displays: Vec::new(),
            input_adapters: HashMap::new(),
            devices: nickel_input::winit::DeviceRegistry::default(),
            warm_present_us: VecDeque::with_capacity(RUNTIME_SAMPLE_CAPACITY),
            input_to_present_us: VecDeque::with_capacity(RUNTIME_SAMPLE_CAPACITY),
            warm_present_allocations: VecDeque::with_capacity(RUNTIME_SAMPLE_CAPACITY),
            presenter_cache_peak_bytes: Cell::new(0),
            output_retirements: OutputRetirementTracker::default(),
            output_creation_retry: OutputCreationRetry::default(),
            pending_input_started: None,
            clipboard: arboard::Clipboard::new().ok().map(std::cell::RefCell::new),
            started,
            options,
            primary_output_name: None,
            active_output_name: None,
        })
    }

    pub fn event_sender(&self) -> ShellEventSender {
        ShellEventSender {
            #[cfg(not(target_os = "windows"))]
            proxy: self.events.create_proxy(),
            #[cfg(target_os = "windows")]
            queue: Arc::clone(&self.external_events),
            #[cfg(target_os = "windows")]
            event_thread: self.event_thread,
        }
    }

    pub fn create_shell_surfaces(&mut self) -> Result<(), String> {
        self.surfaces.clear();
        self.surface_indices.clear();
        self.native_surface_indices.clear();
        self.output_retirements = OutputRetirementTracker::default();
        self.output_creation_retry = OutputCreationRetry::default();
        let displays = require_displays(self.display_geometries()?)?;
        let output_names = self.display_names()?;
        let create_desktops =
            self.options.create_desktop_surfaces && crate::platform::renders_desktop_background();
        let desired = desired_output_surfaces(
            &output_names,
            create_desktops,
            self.options.bar_on_all_displays,
            self.primary_output_name.as_deref(),
        );
        let mut output_creation_failed = false;
        for (display_index, geometry) in displays.iter().copied().enumerate() {
            let output_name = output_names.get(display_index).ok_or_else(|| {
                "winit output identity count changed during shell startup".to_string()
            })?;
            for role in [SurfaceRole::Desktop, SurfaceRole::Panel, SurfaceRole::Lock] {
                if !desired.contains(&(output_name.clone(), role)) {
                    continue;
                }
                if let Err(error) = self.create_surface(role, display_index, geometry, output_name)
                {
                    output_creation_failed = true;
                    tracing::warn!(
                        output = output_name,
                        ?role,
                        %error,
                        "failed to create startup output-owned shell surface; retry scheduled"
                    );
                }
            }
        }
        if output_creation_failed {
            self.output_creation_retry.failed(Instant::now());
        }
        let primary = displays[0];
        let primary_name = output_names.first().ok_or_else(|| {
            "winit reported no output identity for the primary display".to_string()
        })?;
        self.create_surface(SurfaceRole::Launcher, 0, primary, primary_name)?;
        self.create_surface(SurfaceRole::ControlCenter, 0, primary, primary_name)?;
        self.create_surface(SurfaceRole::Notification, 0, primary, primary_name)?;
        self.create_surface(SurfaceRole::VolumeOsd, 0, primary, primary_name)?;
        self.create_surface(SurfaceRole::WindowPreview, 0, primary, primary_name)?;
        self.create_surface(SurfaceRole::WindowContextMenu, 0, primary, primary_name)?;
        self.create_surface(SurfaceRole::CodexProjectMenu, 0, primary, primary_name)?;
        self.create_surface(SurfaceRole::Screenshot, 0, primary, primary_name)?;
        tracing::info!(
            elapsed_ms = self.started.elapsed().as_secs_f64() * 1_000.0,
            surface_count = self.surfaces.len(),
            "winit shell windows created"
        );
        Ok(())
    }

    pub fn sync_display_geometry(&mut self) -> Result<(), String> {
        let displays = self.display_geometries()?;
        let output_names = self.display_names()?;
        self.retire_settled_output_surfaces(&output_names, Instant::now());
        if displays.is_empty() {
            for surface in &mut self.surfaces {
                surface.display_connected = false;
                surface.presenter = None;
                surface.window.set_visible(false);
            }
            self.rebuild_surface_indices();
            self.output_creation_retry.succeeded();
            tracing::info!("winit shell is dormant while no displays are available");
            return Ok(());
        }
        let create_desktops =
            self.options.create_desktop_surfaces && crate::platform::renders_desktop_background();
        let desired = desired_output_surfaces(
            &output_names,
            create_desktops,
            self.options.bar_on_all_displays,
            self.primary_output_name.as_deref(),
        );
        // A settings policy change is authoritative immediately. Missing outputs remain
        // dormant for the retirement grace period so a transient topology snapshot or a
        // quick reconnect can preserve their stable surface identities.
        self.surfaces.retain(|surface| {
            surface.role != SurfaceRole::Panel
                || desired.contains(&(surface.output_name.clone(), SurfaceRole::Panel))
        });
        for surface in &mut self.surfaces {
            if output_role(surface.role)
                && !desired.contains(&(surface.output_name.clone(), surface.role))
            {
                surface.display_connected = false;
                surface.presenter = None;
                surface.window.set_visible(false);
            }
        }
        self.rebuild_surface_indices();
        let mut creation_failed = false;
        for (display_index, geometry) in displays.iter().copied().enumerate() {
            let output_name = output_names.get(display_index).ok_or_else(|| {
                "winit output identity count changed during shell sync".to_string()
            })?;
            for role in [SurfaceRole::Desktop, SurfaceRole::Panel, SurfaceRole::Lock] {
                if !desired.contains(&(output_name.clone(), role)) {
                    continue;
                }
                if self.surfaces.iter().any(|surface| {
                    surface.display_connected
                        && surface.role == role
                        && surface.output_name == *output_name
                }) {
                    continue;
                }
                if let Some(surface) = self.surfaces.iter_mut().find(|surface| {
                    !surface.display_connected
                        && surface.role == role
                        && surface.output_name == *output_name
                }) {
                    surface.display_index = display_index;
                    surface.display_connected = true;
                    let (_, x, y, width, height, _) =
                        surface_geometry(role, geometry, self.options.panel_edge);
                    surface
                        .window
                        .set_outer_position(LogicalPosition::new(x, y));
                    let _ = surface
                        .window
                        .request_inner_size(LogicalSize::new(width, height));
                    surface.window.set_visible(true);
                } else {
                    if let Err(error) =
                        self.create_surface(role, display_index, geometry, output_name)
                    {
                        creation_failed = true;
                        tracing::warn!(
                            output = output_name,
                            ?role,
                            %error,
                            "failed to create output-owned shell surface; retry scheduled"
                        );
                    }
                }
            }
        }
        self.rebuild_surface_indices();
        if creation_failed {
            self.output_creation_retry.failed(Instant::now());
        } else {
            self.output_creation_retry.succeeded();
        }

        let primary = displays[0];
        let primary_name = &output_names[0];
        for surface in &mut self.surfaces {
            if surface.display_connected
                || matches!(
                    surface.role,
                    SurfaceRole::Desktop | SurfaceRole::Panel | SurfaceRole::Lock
                )
            {
                continue;
            }
            surface.display_index = 0;
            surface.output_name.clone_from(primary_name);
            surface.display_connected = true;
            let (_, x, y, width, height, _) =
                surface_geometry(surface.role, primary, self.options.panel_edge);
            surface
                .window
                .set_outer_position(LogicalPosition::new(x, y));
            let _ = surface
                .window
                .request_inner_size(LogicalSize::new(width, height));
        }
        self.rebuild_surface_indices();

        for surface in &mut self.surfaces {
            if !surface.display_connected {
                continue;
            }
            if matches!(
                surface.role,
                SurfaceRole::CodexChat
                    | SurfaceRole::WindowPreview
                    | SurfaceRole::WindowContextMenu
            ) {
                continue;
            }
            let Some(display_index) = output_names
                .iter()
                .position(|name| name == &surface.output_name)
            else {
                continue;
            };
            surface.display_index = display_index;
            let Some(display) = displays.get(display_index).copied() else {
                continue;
            };
            let (_, x, y, width, height, _) =
                surface_geometry(surface.role, display, self.options.panel_edge);
            surface
                .window
                .set_outer_position(LogicalPosition::new(x, y));
            let _ = surface
                .window
                .request_inner_size(LogicalSize::new(width, height));
        }
        Ok(())
    }

    pub fn set_bar_on_all_displays(&mut self, enabled: bool) -> Result<bool, String> {
        if self.options.bar_on_all_displays == enabled {
            return Ok(false);
        }
        self.options.bar_on_all_displays = enabled;
        self.sync_display_geometry()?;
        Ok(true)
    }

    pub fn set_primary_output_name(&mut self, output: Option<String>) -> Result<bool, String> {
        if self.primary_output_name == output {
            return Ok(false);
        }
        self.primary_output_name = output;
        if !self.options.bar_on_all_displays {
            self.sync_display_geometry()?;
        }
        Ok(true)
    }

    /// Records a genuine interaction point in desktop-physical coordinates.
    /// Monitor enumeration and repaint never call this, so they cannot steal
    /// the output used by the next global launcher invocation.
    pub fn set_active_output_at(&mut self, point: (i32, i32)) -> bool {
        let Some(name) = output_name_at(&self.displays, point) else {
            return false;
        };
        if self.active_output_name.as_deref() == Some(name) {
            return false;
        }
        self.active_output_name = Some(name.to_owned());
        true
    }

    pub fn set_active_output_from_surface(&mut self, id: SurfaceId) -> bool {
        let Some(name) = self.surface(id).map(|surface| surface.output_name.clone()) else {
            return false;
        };
        if name.is_empty() || self.active_output_name.as_ref() == Some(&name) {
            return false;
        }
        self.active_output_name = Some(name);
        true
    }

    fn active_output_index(&self) -> Option<usize> {
        preferred_output_index(
            &self.displays,
            self.active_output_name.as_deref(),
            self.primary_output_name.as_deref(),
        )
    }

    fn relocate_to_active_output(&mut self, index: usize) {
        let Some(display_index) = self.active_output_index() else {
            return;
        };
        let Some((geometry, output_name)) = self.displays.get(display_index).cloned() else {
            return;
        };
        let role = self.surfaces[index].role;
        if role != SurfaceRole::Launcher {
            return;
        }
        let (_, x, y, width, height, _) = surface_geometry(role, geometry, self.options.panel_edge);
        let surface = &mut self.surfaces[index];
        surface.display_index = display_index;
        surface.output_name = output_name;
        surface
            .window
            .set_outer_position(LogicalPosition::new(x, y));
        let _ = surface
            .window
            .request_inner_size(LogicalSize::new(width, height));
    }

    fn retire_settled_output_surfaces(&mut self, output_names: &[String], now: Instant) {
        let owned_outputs = self
            .surfaces
            .iter()
            .filter(|surface| output_role(surface.role))
            .map(|surface| surface.output_name.as_str())
            .collect::<Vec<_>>();
        let retired = self
            .output_retirements
            .observe(now, output_names, owned_outputs);
        if retired.is_empty() {
            return;
        }

        // Preserve the durable high-water mark before dropping the last
        // presenter diagnostics for a disconnected output.
        let _ = self.memory_diagnostics();
        self.surfaces.retain(|surface| {
            !output_role_is_retired(surface.role, &surface.output_name, &retired)
        });
        for output in retired {
            self.output_retirements.missing_since.remove(&output);
        }
        self.rebuild_surface_indices();
    }

    pub fn next_output_retirement_deadline(&self) -> Option<Instant> {
        match (
            self.output_retirements.next_deadline(),
            self.output_creation_retry.deadline,
        ) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (left, right) => left.or(right),
        }
    }

    fn rebuild_surface_indices(&mut self) {
        self.surface_indices.clear();
        self.native_surface_indices.clear();
        for (index, surface) in self
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, surface)| surface.display_connected)
        {
            self.surface_indices.insert(surface.id.0, index);
            self.native_surface_indices
                .insert(surface.window.id(), index);
        }
    }

    pub fn surfaces(&self) -> impl Iterator<Item = &ShellSurface> {
        self.surfaces
            .iter()
            .filter(|surface| surface.display_connected)
    }

    pub fn surface(&self, id: SurfaceId) -> Option<&ShellSurface> {
        self.surface_indices
            .get(&id.0)
            .and_then(|index| self.surfaces.get(*index))
    }

    pub fn surface_display_geometry(&self, id: SurfaceId) -> Option<DisplayGeometry> {
        let display_index = self.surface(id)?.display_index();
        self.display_geometries().ok()?.get(display_index).copied()
    }

    pub fn panel_edge(&self) -> PanelEdge {
        self.options.panel_edge
    }

    pub fn surface_mut(&mut self, id: SurfaceId) -> Option<&mut ShellSurface> {
        let index = *self.surface_indices.get(&id.0)?;
        self.surfaces.get_mut(index)
    }

    pub fn mark_initial_exposed(&mut self, id: SurfaceId) -> bool {
        let Some(surface) = self.surface_mut(id) else {
            return false;
        };
        if surface.initial_exposed {
            false
        } else {
            surface.initial_exposed = true;
            true
        }
    }

    pub fn create_codex_chat_surface(
        &mut self,
        title: &str,
        application_id: &str,
    ) -> Result<SurfaceId, String> {
        let geometry = require_displays(self.display_geometries()?)?[0];
        let attributes = Window::default_attributes()
            .with_title(title)
            .with_inner_size(LogicalSize::new(
                1120.min(geometry.width),
                760.min(geometry.height),
            ))
            .with_resizable(true);
        #[cfg(target_os = "linux")]
        let attributes = attributes.with_name(application_id, application_id);
        #[allow(deprecated)]
        let window = self
            .events
            .create_window(attributes)
            .map_err(|error| error.to_string())?;
        window.set_ime_allowed(true);
        let id = SurfaceId(window.id());
        let index = self.surfaces.len();
        self.surface_indices.insert(id.0, index);
        self.native_surface_indices.insert(window.id(), index);
        self.surfaces.push(ShellSurface {
            id,
            role: SurfaceRole::CodexChat,
            application_id: application_id.to_owned(),
            display_index: 0,
            output_name: String::new(),
            display_connected: true,
            initial_exposed: false,
            presenter: None,
            last_host_change_token: None,
            visible: true,
            window,
        });
        Ok(id)
    }

    pub fn destroy_surface(&mut self, id: SurfaceId) {
        let Some(index) = self.surface_indices.remove(&id.0) else {
            return;
        };
        // Observe the process peak before dropping the presenter's last
        // diagnostics. A closed surface must release live bytes without
        // erasing the process-wide high-water mark.
        let _ = self.memory_diagnostics();
        self.surfaces.remove(index);
        self.rebuild_surface_indices();
        let diagnostics = self.memory_diagnostics();
        tracing::debug!(
            presenters = diagnostics.presenter_caches.presenters,
            cache_live_bytes = diagnostics.presenter_caches.live_bytes,
            process_rss_bytes = diagnostics.process_rss_bytes,
            "shell surface closed and presenter accounting refreshed"
        );
    }

    pub fn memory_diagnostics(&self) -> ShellMemoryDiagnostics {
        let mut presenter_caches = AggregatePresenterCacheDiagnostics::from_presenters(
            self.graphics
                .as_ref()
                .map(SharedGraphics::cache_diagnostics),
        );
        let process_peak =
            durable_presenter_peak(self.presenter_cache_peak_bytes.get(), &presenter_caches);
        self.presenter_cache_peak_bytes.set(process_peak);
        presenter_caches.peak_cache_bytes = process_peak;
        ShellMemoryDiagnostics {
            presenter_caches,
            process_rss_bytes: process_rss_bytes(),
        }
    }

    pub fn presenter_roles(&self) -> Vec<SurfaceRole> {
        self.surfaces
            .iter()
            .filter_map(|surface| surface.presenter.as_ref().map(|_| surface.role))
            .collect()
    }

    pub fn runtime_diagnostics(&self) -> ShellRuntimeDiagnostics {
        ShellRuntimeDiagnostics {
            warm_present_us: self.warm_present_us.iter().copied().collect(),
            input_to_present_us: self.input_to_present_us.iter().copied().collect(),
            warm_present_allocations: self.warm_present_allocations.iter().copied().collect(),
        }
    }

    /// Starts a bounded input-to-visible observation. Call `finish_input_observation`
    /// after routing the input so inputs that do not paint cannot contaminate a later sample.
    pub fn begin_input_observation(&mut self, now: Instant) {
        self.pending_input_started = Some(now);
    }

    pub fn finish_input_observation(&mut self) {
        self.pending_input_started = None;
    }

    pub fn clipboard_text(&self) -> Option<String> {
        self.clipboard.as_ref()?.borrow_mut().get_text().ok()
    }

    pub fn clipboard_image(&self) -> Option<(u32, u32, Vec<u8>)> {
        let image = self.clipboard.as_ref()?.borrow_mut().get_image().ok()?;
        Some((
            u32::try_from(image.width).ok()?,
            u32::try_from(image.height).ok()?,
            image.bytes.into_owned(),
        ))
    }

    pub fn set_clipboard_text(&self, text: &str) {
        if let Some(clipboard) = &self.clipboard {
            let _ = clipboard.borrow_mut().set_text(text);
        }
    }

    pub fn present(
        &mut self,
        id: SurfaceId,
        commands: &[PaintCommand],
    ) -> Result<DamageRegion, String> {
        let index = *self
            .surface_indices
            .get(&id.0)
            .ok_or_else(|| "unknown winit shell surface".to_owned())?;
        if self.graphics.is_none() {
            let display = self.surfaces[index]
                .window()
                .display_handle()
                .map_err(|error| error.to_string())?;
            // SAFETY: `WinitShell` drops all surfaces and shared graphics before
            // its winit event loop, which owns this display connection.
            self.graphics = Some(unsafe { SharedGraphics::new(display) }?);
        }
        let graphics = self.graphics.as_ref().expect("shared renderer initialized");
        let entry = &mut self.surfaces[index];
        let warm = entry.presenter.is_some();
        if entry.presenter.is_none() {
            let window = entry
                .window
                .window_handle()
                .map_err(|error| error.to_string())?;
            // SAFETY: `ShellSurface` declares its presenter before its window,
            // so Rust drops the presenter while this native window is valid.
            entry.presenter = Some(unsafe { SoftbufferPresenter::new(window, graphics) }?);
        }
        let started = Instant::now();
        let allocations_before = crate::allocation_counter::allocation_operations();
        let physical = entry.window.inner_size();
        let logical = physical.to_logical::<u32>(entry.window.scale_factor());
        let geometry = PresentationGeometry {
            pixel_width: physical.width,
            pixel_height: physical.height,
            logical_width: logical.width,
            logical_height: logical.height,
        };
        let damage = entry
            .presenter
            .as_mut()
            .expect("shell presenter initialized")
            .present(geometry, graphics, commands)?;
        let elapsed_us = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        if warm {
            push_bounded(&mut self.warm_present_us, elapsed_us);
            if let (Some(before), Some(after)) = (
                allocations_before,
                crate::allocation_counter::allocation_operations(),
            ) {
                push_bounded(
                    &mut self.warm_present_allocations,
                    after.saturating_sub(before),
                );
            }
        }
        if let Some(input_started) = self.pending_input_started.take() {
            let input_us = input_started
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64;
            push_bounded(&mut self.input_to_present_us, input_us);
        }
        Ok(damage)
    }

    /// Presents a canonical UI host frame only when its semantic or paint generation changed.
    pub fn present_host_frame(
        &mut self,
        id: SurfaceId,
        token: HostChangeToken,
        commands: &[PaintCommand],
    ) -> Result<Option<DamageRegion>, String> {
        let index = *self
            .surface_indices
            .get(&id.0)
            .ok_or_else(|| "unknown winit shell surface".to_owned())?;
        if self.surfaces[index].last_host_change_token == Some(token) {
            return Ok(None);
        }
        let damage = self.present(id, commands)?;
        self.surfaces[index].last_host_change_token = Some(token);
        Ok(Some(damage))
    }

    pub fn show(&mut self, id: SurfaceId) -> bool {
        let Some(index) = self.surface_indices.get(&id.0).copied() else {
            return false;
        };
        if self.surfaces[index].role == SurfaceRole::Launcher {
            self.relocate_to_active_output(index);
        }
        let shown = self.surfaces.get_mut(index).is_some_and(|surface| {
            if surface.visible {
                return false;
            }
            surface.visible = true;
            surface.initial_exposed = false;
            surface.last_host_change_token = None;
            surface.window.set_visible(true);
            surface.window.request_redraw();
            true
        });
        if shown {
            self.pending_events.push_back(ShellEvent::Shown(id));
        }
        shown
    }

    pub fn hide(&mut self, id: SurfaceId) -> bool {
        let Some(index) = self.surface_indices.get(&id.0).copied() else {
            return false;
        };
        let hidden = {
            let surface = &mut self.surfaces[index];
            if !surface.visible {
                return false;
            }
            surface.visible = false;
            // Repeated reconciliation must
            // still release a lightweight presentation surface populated while
            // the native window was already hidden (for example by prewarm).
            // Drop native presentation borrows before winit tears down the
            // Wayland surface so the null-buffer unmap can complete.
            surface.presenter = None;
            surface.window.set_visible(false);
            true
        };
        #[cfg(target_os = "linux")]
        if hidden
            && surface_is_ephemeral(self.surfaces[index].role)
            && let Err(error) = self.recreate_hidden_wayland_surface(index)
        {
            tracing::warn!(
                role = ?self.surfaces[index].role,
                %error,
                "failed to recreate hidden ephemeral Wayland surface"
            );
        }
        if hidden {
            self.pending_events.push_back(ShellEvent::Hidden(id));
        }
        hidden
    }

    #[cfg(target_os = "linux")]
    fn recreate_hidden_wayland_surface(&mut self, index: usize) -> Result<(), String> {
        let surface = &self.surfaces[index];
        let geometry = self
            .displays
            .get(surface.display_index)
            .map(|(geometry, _)| *geometry)
            .or_else(|| self.displays.first().map(|(geometry, _)| *geometry))
            .ok_or_else(|| "cannot recreate a shell surface without an output".to_owned())?;
        let (base_title, x, y, width, height, _) =
            surface_geometry(surface.role, geometry, self.options.panel_edge);
        let title = shell_surface_title(surface.role, base_title, &surface.output_name);
        let attributes = Window::default_attributes()
            .with_title(title)
            .with_position(LogicalPosition::new(x, y))
            .with_inner_size(LogicalSize::new(width, height))
            .with_decorations(!surface_is_borderless(surface.role))
            .with_resizable(matches!(
                surface.role,
                SurfaceRole::WindowPreview
                    | SurfaceRole::WindowContextMenu
                    | SurfaceRole::Screenshot
            ))
            .with_visible(true)
            .with_name(&surface.application_id, &surface.application_id);
        #[allow(deprecated)]
        let replacement = self
            .events
            .create_window(attributes)
            .map_err(|error| error.to_string())?;
        if matches!(
            surface.role,
            SurfaceRole::CodexProjectMenu | SurfaceRole::Screenshot
        ) {
            replacement.set_ime_allowed(true);
        }
        if surface.role == SurfaceRole::Screenshot {
            replacement.set_min_inner_size(Some(LogicalSize::new(720, 480)));
        }
        let retired_native_id = surface.window.id();
        self.native_surface_indices.remove(&retired_native_id);
        self.input_adapters.remove(&retired_native_id);
        let surface = &mut self.surfaces[index];
        surface.window = replacement;
        surface.initial_exposed = false;
        surface.last_host_change_token = None;
        self.native_surface_indices
            .insert(surface.window.id(), index);
        Ok(())
    }

    pub fn raise(&mut self, id: SurfaceId) -> bool {
        self.surface_mut(id).is_some_and(|surface| {
            surface.window.focus_window();
            true
        })
    }

    pub fn raise_role(&mut self, role: SurfaceRole) -> bool {
        let ids = self
            .surfaces
            .iter()
            .filter(|surface| surface.role() == role)
            .map(ShellSurface::id)
            .collect::<Vec<_>>();
        let mut raised = false;
        for id in ids {
            if let Some(surface) = self.surface_mut(id) {
                surface.window.focus_window();
                raised = true;
            }
        }
        raised
    }

    pub fn start_text_input(&self, id: SurfaceId) -> bool {
        self.surface(id).is_some_and(|surface| {
            surface.window().set_ime_allowed(true);
            true
        })
    }

    pub fn stop_text_input(&self, id: SurfaceId) -> bool {
        self.surface(id).is_some_and(|surface| {
            surface.window().set_ime_allowed(false);
            true
        })
    }

    pub fn poll_events(&mut self) -> Vec<ShellEvent> {
        self.drain_external_events();
        self.pump_events(Some(Duration::ZERO));
        self.drain_external_events();
        self.pending_events.drain(..).collect()
    }

    pub fn wait_event(&mut self) -> Option<ShellEvent> {
        self.wait_event_timeout(Duration::from_secs(24 * 60 * 60))
    }

    pub fn wait_event_timeout(&mut self, timeout: Duration) -> Option<ShellEvent> {
        self.drain_external_events();
        if let Some(event) = self.pending_events.pop_front() {
            return Some(event);
        }
        #[cfg(target_os = "windows")]
        {
            // Drain anything winit can translate before arming the native wait.
            // This also establishes the queue's "seen" state so the wait below
            // responds to messages arriving after this point.
            self.pump_events(Some(Duration::ZERO));
            self.drain_external_events();
            if let Some(event) = self.pending_events.pop_front() {
                return Some(event);
            }
            // Winit's pump timeout returns immediately under Wine and some Windows
            // compatibility environments. Wait on the owning thread's message queue
            // directly so native input and Nickel's PostThreadMessageW wake remain
            // interruptible, then let winit translate everything without blocking.
            // Do not use MWMO_INPUTAVAILABLE: winit may leave a previously inspected
            // message queued, and that flag would make the wait return forever for
            // the old message instead of waiting for new queue activity.
            // SAFETY: this thread owns the event loop and therefore its message queue;
            // there are no object handles, and all flag values are Win32 constants.
            unsafe {
                MsgWaitForMultipleObjectsEx(
                    None,
                    windows_wait_timeout_millis(timeout),
                    QS_ALLINPUT,
                    Default::default(),
                )
            };
            self.pump_events(Some(Duration::ZERO));
        }
        #[cfg(not(target_os = "windows"))]
        self.pump_events(Some(timeout));
        self.drain_external_events();
        self.pending_events.pop_front()
    }

    fn drain_external_events(&mut self) {
        #[cfg(target_os = "windows")]
        {
            let events = {
                let mut queue = self
                    .external_events
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                queue.drain(..).collect::<Vec<_>>()
            };
            for event in events {
                self.push_user_event(event);
            }
        }
    }

    fn push_user_event(&mut self, event: ShellUserEvent) {
        match event {
            ShellUserEvent::GlobalShortcut(shortcut) => self
                .pending_events
                .push_back(ShellEvent::GlobalShortcut(shortcut)),
            #[cfg(target_os = "linux")]
            ShellUserEvent::TestControl(request) => {
                self.pending_events
                    .push_back(ShellEvent::TestControl(request));
            }
        }
    }

    fn pump_events(&mut self, timeout: Option<Duration>) {
        let indices = &self.native_surface_indices;
        let surfaces = &self.surfaces;
        let adapters = &mut self.input_adapters;
        let devices = &mut self.devices;
        let pending = &mut self.pending_events;
        let displays = &mut self.displays;
        self.events.set_control_flow(ControlFlow::Wait);
        #[allow(deprecated)]
        self.events
            .pump_events(timeout, |event, active| match event {
                Event::Resumed | Event::AboutToWait => {
                    *displays = active
                        .available_monitors()
                        .enumerate()
                        .map(|(index, display)| {
                            let position = display.position();
                            let size = display.size();
                            (
                                DisplayGeometry {
                                    x: position.x,
                                    y: position.y,
                                    width: size.width.max(1),
                                    height: size.height.max(1),
                                    scale: display.scale_factor().max(1.0) as f32,
                                },
                                display.name().unwrap_or_else(|| format!("display-{index}")),
                            )
                        })
                        .collect();
                }
                Event::UserEvent(ShellUserEvent::GlobalShortcut(shortcut)) => {
                    pending.push_back(ShellEvent::GlobalShortcut(shortcut));
                }
                #[cfg(target_os = "linux")]
                Event::UserEvent(ShellUserEvent::TestControl(request)) => {
                    pending.push_back(ShellEvent::TestControl(request));
                }
                Event::WindowEvent { window_id, event } => {
                    let Some(&index) = indices.get(&window_id) else {
                        return;
                    };
                    let surface = surfaces[index].id;
                    let scale = surfaces[index].window.scale_factor();
                    let native_device = window_event_device(&event);
                    // Winit omits a device on lifecycle and IME events. Its documented dummy
                    // identity is appropriate for this event-loop-local synthetic stream.
                    let native_device = native_device.unwrap_or_else(winit::event::DeviceId::dummy);
                    let device = devices.get_or_insert(native_device);
                    let adapter = adapters.entry(window_id).or_default();
                    for input in adapter.normalize_at_scale(device, scale, &event) {
                        pending.push_back(ShellEvent::Input {
                            surface,
                            event: input,
                        });
                    }
                    if let Some(event) = translate_window_event(surface, scale as f32, &event) {
                        pending.push_back(event);
                    }
                }
                Event::DeviceEvent {
                    device_id,
                    event: winit::event::DeviceEvent::Removed,
                } => {
                    if let Some(device) = devices.remove(device_id) {
                        for (&window_id, adapter) in adapters.iter_mut() {
                            if indices.contains_key(&window_id) {
                                pending.push_back(ShellEvent::Input {
                                    surface: SurfaceId(window_id),
                                    event: adapter.device_removed(device),
                                });
                            }
                        }
                    }
                }
                _ => {}
            });
    }

    pub fn display_geometries(&self) -> Result<Vec<DisplayGeometry>, String> {
        Ok(self
            .displays
            .iter()
            .map(|(geometry, _)| *geometry)
            .collect())
    }

    fn display_names(&self) -> Result<Vec<String>, String> {
        Ok(self.displays.iter().map(|(_, name)| name.clone()).collect())
    }

    fn create_surface(
        &mut self,
        role: SurfaceRole,
        display_index: usize,
        geometry: DisplayGeometry,
        output_name: &str,
    ) -> Result<(), String> {
        let (base_title, x, y, width, height, hidden) =
            surface_geometry(role, geometry, self.options.panel_edge);
        let title = shell_surface_title(role, base_title, output_name);
        let application_id = match role {
            SurfaceRole::Desktop => SessionShellRole::Desktop.application_id(),
            SurfaceRole::Panel => SessionShellRole::Panel.application_id(),
            SurfaceRole::Launcher => SessionShellRole::Launcher.application_id(),
            SurfaceRole::ControlCenter => SessionShellRole::ControlCenter.application_id(),
            SurfaceRole::Notification => SessionShellRole::Notification.application_id(),
            SurfaceRole::VolumeOsd => SessionShellRole::VolumeOsd.application_id(),
            SurfaceRole::WindowPreview => SessionShellRole::Preview.application_id(),
            SurfaceRole::WindowContextMenu => SessionShellRole::ContextMenu.application_id(),
            SurfaceRole::CodexProjectMenu => SessionShellRole::ProjectMenu.application_id(),
            SurfaceRole::Lock => SessionShellRole::Lock.application_id(),
            SurfaceRole::Screenshot => SessionShellRole::Screenshot.application_id(),
            SurfaceRole::CodexChat => unreachable!("chat surfaces are dynamic"),
        };
        let attributes = Window::default_attributes()
            .with_title(title)
            .with_position(LogicalPosition::new(x, y))
            .with_inner_size(LogicalSize::new(width, height))
            .with_decorations(!surface_is_borderless(role))
            .with_resizable(matches!(
                role,
                SurfaceRole::WindowPreview
                    | SurfaceRole::WindowContextMenu
                    | SurfaceRole::Screenshot
            ))
            .with_visible(!hidden || cfg!(target_os = "linux"));
        #[cfg(target_os = "linux")]
        let attributes = attributes.with_name(application_id, application_id);
        #[allow(deprecated)]
        let window = self
            .events
            .create_window(attributes)
            .map_err(|error| error.to_string())?;
        #[cfg(target_os = "windows")]
        match role {
            SurfaceRole::Desktop => {
                if !crate::platform::configure_desktop_window(
                    &window,
                    (geometry.x, geometry.y),
                    (geometry.width, geometry.height),
                ) {
                    tracing::warn!(?role, "failed to configure Windows shell window");
                }
            }
            SurfaceRole::Panel => {
                if !crate::platform::configure_panel_window(&window) {
                    tracing::warn!(?role, "failed to configure Windows shell window");
                }
            }
            SurfaceRole::Launcher => {
                if !crate::platform::configure_launcher_window(&window) {
                    tracing::warn!(?role, "failed to configure Windows shell window");
                }
            }
            SurfaceRole::WindowPreview => {
                if !crate::platform::configure_preview_window(&window) {
                    tracing::warn!(?role, "failed to configure Windows shell window");
                }
            }
            SurfaceRole::WindowContextMenu => {
                if !crate::platform::configure_context_menu_window(&window) {
                    tracing::warn!(?role, "failed to configure Windows shell window");
                }
            }
            SurfaceRole::VolumeOsd => {
                let configured = crate::platform::configure_volume_osd_window(&window);
                if !configured {
                    tracing::warn!(?role, "failed to configure Windows shell window");
                }
            }
            _ => {}
        }
        if role == SurfaceRole::Screenshot {
            window.set_min_inner_size(Some(LogicalSize::new(720, 480)));
            #[cfg(target_os = "windows")]
            if !crate::platform::configure_screenshot_window(&window) {
                tracing::warn!("failed to configure Nickel screenshot utility window");
            }
        }
        if role == SurfaceRole::CodexProjectMenu {
            window.set_ime_allowed(true);
        }
        if role == SurfaceRole::Launcher {
            window.set_ime_allowed(true);
        }
        if role == SurfaceRole::Lock {
            window.set_ime_allowed(true);
        }
        let id = SurfaceId(window.id());
        let index = self.surfaces.len();
        self.surface_indices.insert(id.0, index);
        self.native_surface_indices.insert(window.id(), index);
        self.surfaces.push(ShellSurface {
            id,
            role,
            application_id: application_id.to_owned(),
            display_index,
            output_name: output_name.to_owned(),
            display_connected: true,
            initial_exposed: false,
            presenter: None,
            last_host_change_token: None,
            // Start reconciled as visible even when the native window was
            // requested hidden so the first policy pass performs the real
            // platform hide transition. This matters on Wayland, where the
            // winit visibility hint itself is intentionally a no-op.
            visible: true,
            window,
        });
        Ok(())
    }
}

fn output_name_at(displays: &[(DisplayGeometry, String)], point: (i32, i32)) -> Option<&str> {
    displays
        .iter()
        .find(|(geometry, _)| {
            let right = i64::from(geometry.x) + i64::from(geometry.width);
            let bottom = i64::from(geometry.y) + i64::from(geometry.height);
            i64::from(point.0) >= i64::from(geometry.x)
                && i64::from(point.0) < right
                && i64::from(point.1) >= i64::from(geometry.y)
                && i64::from(point.1) < bottom
        })
        .map(|(_, name)| name.as_str())
}

fn preferred_output_index(
    displays: &[(DisplayGeometry, String)],
    active: Option<&str>,
    primary: Option<&str>,
) -> Option<usize> {
    active
        .and_then(|name| displays.iter().position(|(_, candidate)| candidate == name))
        .or_else(|| {
            primary.and_then(|name| displays.iter().position(|(_, candidate)| candidate == name))
        })
        .or_else(|| {
            displays
                .iter()
                .enumerate()
                .min_by(|(_, (_, left)), (_, (_, right))| left.cmp(right))
                .map(|(index, _)| index)
        })
}

fn panel_outputs(
    output_names: &[String],
    all_displays: bool,
    primary_output: Option<&str>,
) -> Vec<String> {
    if all_displays {
        output_names.to_vec()
    } else {
        primary_output
            .filter(|primary| output_names.iter().any(|output| output == primary))
            .map(str::to_owned)
            .or_else(|| output_names.iter().min().cloned())
            .into_iter()
            .collect()
    }
}

fn window_event_device(event: &WindowEvent) -> Option<winit::event::DeviceId> {
    match event {
        WindowEvent::KeyboardInput { device_id, .. }
        | WindowEvent::CursorMoved { device_id, .. }
        | WindowEvent::CursorEntered { device_id }
        | WindowEvent::CursorLeft { device_id }
        | WindowEvent::MouseWheel { device_id, .. }
        | WindowEvent::MouseInput { device_id, .. }
        | WindowEvent::TouchpadPressure { device_id, .. }
        | WindowEvent::AxisMotion { device_id, .. } => Some(*device_id),
        WindowEvent::Touch(touch) => Some(touch.device_id),
        _ => None,
    }
}

fn translate_window_event(
    surface: SurfaceId,
    scale: f32,
    event: &WindowEvent,
) -> Option<ShellEvent> {
    match event {
        WindowEvent::CloseRequested => Some(ShellEvent::CloseRequested(surface)),
        WindowEvent::Focused(focused) => Some(ShellEvent::FocusChanged {
            surface,
            focused: *focused,
        }),
        WindowEvent::CursorEntered { .. } => Some(ShellEvent::PointerEntered {
            surface,
            entered: true,
        }),
        WindowEvent::CursorLeft { .. } => Some(ShellEvent::PointerEntered {
            surface,
            entered: false,
        }),
        WindowEvent::Resized(size) => Some(ShellEvent::PixelResize {
            surface,
            width: size.width,
            height: size.height,
            scale,
        }),
        WindowEvent::ScaleFactorChanged { .. } => Some(ShellEvent::DisplayTopologyChanged),
        WindowEvent::RedrawRequested => Some(ShellEvent::Redraw(surface)),
        WindowEvent::Destroyed => Some(ShellEvent::Hidden(surface)),
        WindowEvent::DroppedFile(path) => Some(ShellEvent::FileDrop {
            surface,
            path: path.clone(),
        }),
        _ => None,
    }
}

fn shell_surface_title(role: SurfaceRole, title: &str, output_name: &str) -> String {
    if matches!(
        role,
        SurfaceRole::Desktop | SurfaceRole::Panel | SurfaceRole::Lock
    ) {
        let output_name = output_name
            .chars()
            .filter(|character| !character.is_control() && *character != ']')
            .collect::<String>();
        return format!("{title} [output={output_name}]");
    }
    title.to_owned()
}

fn surface_geometry(
    role: SurfaceRole,
    geometry: DisplayGeometry,
    panel_edge: PanelEdge,
) -> (&'static str, i32, i32, u32, u32, bool) {
    match role {
        SurfaceRole::Desktop => (
            DESKTOP_TITLE,
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            false,
        ),
        SurfaceRole::Panel => (
            PANEL_TITLE,
            geometry.x,
            match panel_edge {
                PanelEdge::Top => geometry.y,
                PanelEdge::Bottom => {
                    geometry.y + geometry.height.saturating_sub(PANEL_HEIGHT) as i32
                }
            },
            geometry.width,
            PANEL_HEIGHT,
            false,
        ),
        SurfaceRole::Launcher => (
            LAUNCHER_TITLE,
            geometry.x + 18,
            geometry.y + geometry.height.saturating_sub(744) as i32,
            920.min(geometry.width),
            680.min(geometry.height.saturating_sub(PANEL_HEIGHT + 8)),
            cfg!(not(target_os = "linux")),
        ),
        SurfaceRole::ControlCenter => (
            CONTROL_CENTER_TITLE,
            geometry.x + geometry.width.saturating_sub(438) as i32,
            geometry.y + geometry.height.saturating_sub(672) as i32,
            420.min(geometry.width),
            600.min(geometry.height),
            true,
        ),
        SurfaceRole::Notification => (
            NOTIFICATION_TITLE,
            geometry.x + geometry.width.saturating_sub(438) as i32,
            geometry.y + 24,
            420.min(geometry.width),
            180.min(geometry.height),
            true,
        ),
        SurfaceRole::VolumeOsd => (
            VOLUME_OSD_TITLE,
            geometry.x + (geometry.width.saturating_sub(320) / 2) as i32,
            geometry.y + geometry.height.saturating_sub(170) as i32,
            320.min(geometry.width),
            88.min(geometry.height),
            true,
        ),
        SurfaceRole::WindowPreview => (
            WINDOW_PREVIEW_TITLE,
            geometry.x + geometry.width.saturating_sub(1160.min(geometry.width)) as i32 / 2,
            geometry.y + geometry.height.saturating_sub(220.min(geometry.height)) as i32 / 2,
            1160.min(geometry.width),
            220.min(geometry.height),
            true,
        ),
        SurfaceRole::WindowContextMenu => (
            WINDOW_CONTEXT_MENU_TITLE,
            geometry.x,
            geometry.y,
            220.min(geometry.width),
            156.min(geometry.height),
            true,
        ),
        SurfaceRole::CodexProjectMenu => (
            CODEX_PROJECT_MENU_TITLE,
            geometry.x + geometry.width.saturating_sub(464) as i32,
            geometry.y + geometry.height.saturating_sub(476) as i32,
            360.min(geometry.width),
            420.min(geometry.height.saturating_sub(PANEL_HEIGHT)),
            true,
        ),
        SurfaceRole::Lock => (
            LOCK_TITLE,
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            true,
        ),
        SurfaceRole::Screenshot => (
            SCREENSHOT_TITLE,
            geometry.x + (geometry.width.saturating_sub(1200) / 2) as i32,
            geometry.y + (geometry.height.saturating_sub(760) / 2) as i32,
            1200.min(geometry.width),
            760.min(geometry.height),
            true,
        ),
        SurfaceRole::CodexChat => unreachable!("chat surfaces are created dynamically"),
    }
}

fn require_displays(displays: Vec<DisplayGeometry>) -> Result<Vec<DisplayGeometry>, String> {
    if displays.is_empty() {
        Err("winit reported no displays; refusing to start a headless Nickel shell".into())
    } else {
        Ok(displays)
    }
}

fn output_role(role: SurfaceRole) -> bool {
    matches!(
        role,
        SurfaceRole::Desktop | SurfaceRole::Panel | SurfaceRole::Lock
    )
}

fn output_role_is_retired(role: SurfaceRole, output_name: &str, retired: &[String]) -> bool {
    output_role(role) && retired.iter().any(|output| output == output_name)
}

#[cfg(test)]
mod runtime_diagnostics_tests {
    use super::{RUNTIME_SAMPLE_CAPACITY, push_bounded};
    use std::collections::VecDeque;

    #[test]
    fn runtime_samples_are_bounded_and_keep_the_newest_observations() {
        let mut samples = VecDeque::new();
        for sample in 0..(RUNTIME_SAMPLE_CAPACITY as u64 + 7) {
            push_bounded(&mut samples, sample);
        }
        assert_eq!(samples.len(), RUNTIME_SAMPLE_CAPACITY);
        assert_eq!(samples.front(), Some(&7));
        assert_eq!(samples.back(), Some(&(RUNTIME_SAMPLE_CAPACITY as u64 + 6)));
    }
}

fn surface_is_borderless(role: SurfaceRole) -> bool {
    // Linux shell roles are compositor-owned chrome. In particular, allowing the runtime to decorate the
    // screenshot utility adds client-side shadow/titlebar extents to its Wayland geometry, so the
    // compositor can no longer translate renderer-owned local input targets correctly. Windows
    // intentionally keeps the screenshot utility as a conventional decorated tool window.
    role != SurfaceRole::Screenshot || cfg!(target_os = "linux")
}

#[cfg(target_os = "linux")]
fn process_rss_bytes() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_proc_status_rss(&status)
}

#[cfg(not(target_os = "linux"))]
fn process_rss_bytes() -> Option<usize> {
    None
}

pub(crate) fn parse_proc_status_rss(status: &str) -> Option<usize> {
    let kibibytes = status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?.trim();
        let number = value.strip_suffix("kB")?.trim().parse::<usize>().ok()?;
        Some(number)
    })?;
    kibibytes.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use super::surface_is_ephemeral;
    use super::{
        DESKTOP_TITLE, DisplayGeometry, LAUNCHER_TITLE, OUTPUT_CREATION_RETRY_MAX,
        OUTPUT_CREATION_RETRY_MIN, OUTPUT_RETIREMENT_SETTLE, OutputCreationRetry,
        OutputRetirementTracker, PANEL_TITLE, PanelEdge, SurfaceRole, desired_output_surfaces,
        durable_presenter_peak, output_name_at, output_role_is_retired, panel_outputs,
        parse_proc_status_rss, preferred_output_index, require_displays, shell_surface_title,
        surface_geometry, surface_is_borderless,
    };

    use nickel_ui::AggregatePresenterCacheDiagnostics;
    use std::collections::HashSet;
    use std::time::{Duration, Instant};

    #[test]
    fn active_output_selection_uses_interaction_then_primary_then_stable_identity() {
        let displays = vec![
            (
                DisplayGeometry {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                    scale: 1.0,
                },
                "z-primary".to_owned(),
            ),
            (
                DisplayGeometry {
                    x: -2560,
                    y: -200,
                    width: 2560,
                    height: 1440,
                    scale: 1.5,
                },
                "a-wide".to_owned(),
            ),
        ];
        assert_eq!(output_name_at(&displays, (-200, 400)), Some("a-wide"));
        assert_eq!(output_name_at(&displays, (0, 0)), Some("z-primary"));
        assert_eq!(output_name_at(&displays, (1920, 100)), None);
        assert_eq!(
            preferred_output_index(&displays, Some("a-wide"), Some("z-primary")),
            Some(1)
        );
        assert_eq!(
            preferred_output_index(&displays, Some("missing"), Some("z-primary")),
            Some(0),
            "a stale interaction output falls through to the configured primary"
        );
        assert_eq!(preferred_output_index(&displays, None, None), Some(1));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn wayland_surface_lifecycle_keeps_only_high_frequency_chrome_warm() {
        for role in [
            SurfaceRole::Notification,
            SurfaceRole::VolumeOsd,
            SurfaceRole::WindowPreview,
            SurfaceRole::WindowContextMenu,
            SurfaceRole::CodexProjectMenu,
            SurfaceRole::Screenshot,
        ] {
            assert!(surface_is_ephemeral(role), "{role:?}");
        }
        for role in [
            SurfaceRole::Desktop,
            SurfaceRole::Panel,
            SurfaceRole::Lock,
            SurfaceRole::Launcher,
            SurfaceRole::ControlCenter,
            SurfaceRole::CodexChat,
        ] {
            assert!(!surface_is_ephemeral(role), "{role:?}");
        }
    }

    #[test]
    fn process_presenter_peak_survives_destroyed_presenters() {
        let live = AggregatePresenterCacheDiagnostics {
            presenters: 2,
            live_bytes: 220,
            peak_cache_bytes: 320,
            ..AggregatePresenterCacheDiagnostics::default()
        };
        let peak = durable_presenter_peak(0, &live);
        assert_eq!(
            durable_presenter_peak(peak, &AggregatePresenterCacheDiagnostics::default()),
            320
        );

        let reopened = AggregatePresenterCacheDiagnostics {
            presenters: 1,
            live_bytes: 80,
            peak_cache_bytes: 140,
            ..AggregatePresenterCacheDiagnostics::default()
        };
        assert_eq!(durable_presenter_peak(peak, &reopened), 320);
    }

    #[test]
    fn linux_screenshot_shell_surface_has_no_client_decoration_extents() {
        if cfg!(target_os = "linux") {
            assert!(surface_is_borderless(SurfaceRole::Screenshot));
        }
    }

    #[test]
    fn rejects_a_headless_shell_startup() {
        assert_eq!(
            require_displays(Vec::new()).unwrap_err(),
            "winit reported no displays; refusing to start a headless Nickel shell"
        );
    }

    #[test]
    fn accepts_visible_displays() {
        let display = DisplayGeometry {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
        };
        assert_eq!(require_displays(vec![display]).unwrap(), vec![display]);
    }

    #[test]
    fn panel_scope_reconciles_primary_and_all_display_topologies() {
        let outputs = vec!["DP-1".to_owned(), "HDMI-A-1".to_owned()];
        assert_eq!(
            panel_outputs(&outputs, false, Some("HDMI-A-1")),
            vec!["HDMI-A-1"]
        );
        assert_eq!(
            panel_outputs(&outputs, false, Some("missing")),
            vec!["DP-1"]
        );
        assert_eq!(panel_outputs(&outputs, true, Some("HDMI-A-1")), outputs);
        assert_eq!(panel_outputs(&[], false, None), Vec::<String>::new());
    }

    #[test]
    fn primary_panel_policy_is_stable_across_reorder_hotplug_and_fallback() {
        let original = vec!["DP-1".to_owned(), "HDMI-A-1".to_owned()];
        let reordered = vec!["HDMI-A-1".to_owned(), "DP-1".to_owned()];
        let expanded = vec![
            "USB-C-1".to_owned(),
            "HDMI-A-1".to_owned(),
            "DP-1".to_owned(),
        ];
        for outputs in [&original, &reordered, &expanded] {
            assert_eq!(
                panel_outputs(outputs, false, Some("HDMI-A-1")),
                vec!["HDMI-A-1"]
            );
        }
        assert_eq!(
            panel_outputs(&["USB-C-1".into(), "DP-1".into()], false, Some("HDMI-A-1")),
            vec!["DP-1"]
        );
        assert_eq!(
            panel_outputs(&reordered, false, Some("HDMI-A-1")),
            vec!["HDMI-A-1"]
        );
    }

    #[test]
    fn every_enabled_output_requires_its_own_wallpaper_bar_and_lock() {
        let outputs = vec!["DP-1".to_owned(), "HDMI-A-1".to_owned()];
        let desired = desired_output_surfaces(&outputs, true, true, None);
        assert_eq!(desired.len(), 6);
        for output in outputs {
            for role in [SurfaceRole::Desktop, SurfaceRole::Panel, SurfaceRole::Lock] {
                assert!(
                    desired.contains(&(output.clone(), role)),
                    "{output} {role:?}"
                );
            }
        }
    }

    #[test]
    fn hotplug_requires_output_chrome_even_when_the_existing_panel_is_healthy() {
        let before = desired_output_surfaces(&["DP-1".to_owned()], true, false, None);
        let after = desired_output_surfaces(
            &["DP-1".to_owned(), "HDMI-A-1".to_owned()],
            true,
            false,
            None,
        );
        let added = after.difference(&before).cloned().collect::<HashSet<_>>();
        assert_eq!(
            added,
            HashSet::from([
                ("HDMI-A-1".to_owned(), SurfaceRole::Desktop),
                ("HDMI-A-1".to_owned(), SurfaceRole::Lock),
            ])
        );
        assert!(after.contains(&("DP-1".to_owned(), SurfaceRole::Panel)));
        assert!(!after.contains(&("HDMI-A-1".to_owned(), SurfaceRole::Panel)));
    }

    #[test]
    fn output_reordering_does_not_change_stable_surface_ownership() {
        let forward = desired_output_surfaces(
            &["DP-1".to_owned(), "HDMI-A-1".to_owned()],
            true,
            true,
            None,
        );
        let reversed = desired_output_surfaces(
            &["HDMI-A-1".to_owned(), "DP-1".to_owned()],
            true,
            true,
            None,
        );
        assert_eq!(forward, reversed);
    }

    #[test]
    fn desktop_policy_does_not_suppress_per_output_bars() {
        let desired = desired_output_surfaces(
            &["DP-1".to_owned(), "HDMI-A-1".to_owned()],
            false,
            true,
            None,
        );
        assert_eq!(desired.len(), 4);
        assert!(
            desired
                .iter()
                .all(|(_, role)| *role == SurfaceRole::Panel || *role == SurfaceRole::Lock)
        );
    }

    #[test]
    fn failed_output_surface_creation_retries_with_a_bounded_backoff() {
        let started = Instant::now();
        let mut retry = OutputCreationRetry::default();
        retry.failed(started);
        assert_eq!(retry.deadline, Some(started + OUTPUT_CREATION_RETRY_MIN));
        for step in 1..12 {
            retry.failed(started + Duration::from_secs(step));
        }
        assert_eq!(
            retry.deadline,
            Some(started + Duration::from_secs(11) + OUTPUT_CREATION_RETRY_MAX)
        );
        retry.succeeded();
        assert_eq!(retry.deadline, None);
        assert_eq!(retry.failures, 0);
    }

    #[test]
    fn proc_rss_is_allocator_visible_and_parsed_independently() {
        assert_eq!(
            parse_proc_status_rss("Name:\tnickel\nVmRSS:\t   12345 kB\nThreads:\t1\n"),
            Some(12_641_280)
        );
        assert_eq!(parse_proc_status_rss("VmRSS:\tunknown kB\n"), None);
        assert_eq!(parse_proc_status_rss("VmSize:\t123 kB\n"), None);
    }

    #[test]
    fn shell_surfaces_follow_updated_display_geometry() {
        let display = DisplayGeometry {
            x: 40,
            y: 20,
            width: 1920,
            height: 1006,
            scale: 1.5,
        };
        assert_eq!(
            surface_geometry(SurfaceRole::Desktop, display, PanelEdge::Bottom),
            ("Nickel Desktop", 40, 20, 1920, 1006, false)
        );
        assert_eq!(
            surface_geometry(SurfaceRole::Panel, display, PanelEdge::Bottom),
            ("Nickel Panel", 40, 970, 1920, 56, false)
        );
        assert_eq!(
            surface_geometry(SurfaceRole::Panel, display, PanelEdge::Top),
            ("Nickel Panel", 40, 20, 1920, 56, false)
        );
    }

    #[test]
    fn per_output_shell_titles_carry_sanitized_output_identity() {
        assert_eq!(
            shell_surface_title(SurfaceRole::Desktop, DESKTOP_TITLE, "DP-1"),
            "Nickel Desktop [output=DP-1]"
        );
        assert_eq!(
            shell_surface_title(SurfaceRole::Panel, PANEL_TITLE, "HDMI A/1"),
            "Nickel Panel [output=HDMI A/1]"
        );
        assert_eq!(
            shell_surface_title(SurfaceRole::Launcher, LAUNCHER_TITLE, "DP-1"),
            LAUNCHER_TITLE
        );
    }

    #[test]
    fn unchanged_output_surface_ids_survive_transient_missing_snapshots() {
        let started = Instant::now();
        let mut tracker = OutputRetirementTracker::default();
        let mut surfaces = vec![
            (SurfaceRole::Desktop, "winit".to_owned(), 11_u32),
            (SurfaceRole::Panel, "winit".to_owned(), 12),
            (SurfaceRole::Lock, "winit".to_owned(), 13),
        ];

        assert!(
            tracker
                .observe(started, &[], ["winit", "winit", "winit"])
                .is_empty()
        );
        assert!(
            tracker
                .observe(
                    started + Duration::from_millis(100),
                    &["winit".to_owned()],
                    ["winit", "winit", "winit"],
                )
                .is_empty()
        );
        surfaces.retain(|(role, output, _)| !output_role_is_retired(*role, output, &[]));
        assert_eq!(
            surfaces.iter().map(|(_, _, id)| *id).collect::<Vec<_>>(),
            vec![11, 12, 13],
            "transient snapshots must not replace unchanged-output surfaces"
        );
        assert_eq!(tracker.next_deadline(), None);
    }

    #[test]
    fn removed_output_roles_retire_only_after_settled_confirmation() {
        let started = Instant::now();
        let mut tracker = OutputRetirementTracker::default();
        assert!(tracker.observe(started, &[], ["memory-a"]).is_empty());
        assert!(
            tracker
                .observe(
                    started + OUTPUT_RETIREMENT_SETTLE - Duration::from_millis(1),
                    &[],
                    ["memory-a"],
                )
                .is_empty()
        );
        let retired = tracker.observe(started + OUTPUT_RETIREMENT_SETTLE, &[], ["memory-a"]);
        assert_eq!(retired, vec!["memory-a".to_owned()]);
        let mut surfaces = vec![
            (SurfaceRole::Desktop, "winit".to_owned(), 11_u32),
            (SurfaceRole::Desktop, "memory-a".to_owned(), 21),
            (SurfaceRole::Panel, "memory-a".to_owned(), 22),
            (SurfaceRole::Lock, "memory-a".to_owned(), 23),
        ];
        surfaces.retain(|(role, output, _)| !output_role_is_retired(*role, output, &retired));
        assert_eq!(
            surfaces,
            vec![(SurfaceRole::Desktop, "winit".to_owned(), 11)]
        );
    }

    #[test]
    fn reconnect_after_retirement_starts_a_fresh_settlement_generation() {
        let started = Instant::now();
        let mut tracker = OutputRetirementTracker::default();
        assert!(tracker.observe(started, &[], ["memory-a"]).is_empty());
        let retired = tracker.observe(started + OUTPUT_RETIREMENT_SETTLE, &[], ["memory-a"]);
        assert_eq!(retired, vec!["memory-a".to_owned()]);
        let retired_ids = [21_u32, 22, 23];

        let reconnected = started + OUTPUT_RETIREMENT_SETTLE + Duration::from_millis(1);
        assert!(
            tracker
                .observe(reconnected, &["memory-a".to_owned()], ["memory-a"])
                .is_empty()
        );
        assert!(
            tracker
                .observe(reconnected + Duration::from_millis(1), &[], ["memory-a"])
                .is_empty()
        );
        assert_eq!(
            tracker.next_deadline(),
            Some(reconnected + Duration::from_millis(1) + OUTPUT_RETIREMENT_SETTLE)
        );
        let fresh_ids = [31_u32, 32, 33];
        assert_ne!(retired_ids, fresh_ids);
    }

    #[test]
    fn unique_output_name_churn_has_one_bounded_pending_generation() {
        let started = Instant::now();
        let mut tracker = OutputRetirementTracker::default();
        let mut now = started;

        for generation in 0..512 {
            let historical = format!("memory-{generation}");
            assert!(
                tracker
                    .observe(now, &["winit".to_owned()], ["winit", &historical])
                    .is_empty()
            );
            now += Duration::from_millis(1);
            assert!(
                tracker
                    .observe(now, &["winit".to_owned()], ["winit", &historical])
                    .is_empty()
            );
            now += OUTPUT_RETIREMENT_SETTLE;
            assert_eq!(
                tracker.observe(now, &["winit".to_owned()], ["winit", &historical]),
                vec![historical]
            );
            tracker.missing_since.clear();
            assert_eq!(tracker.missing_since.len(), 0);
            now += Duration::from_millis(1);
        }
    }

    #[test]
    fn windows_message_wait_rounds_sub_millisecond_deadlines_up() {
        assert_eq!(super::windows_wait_timeout_millis(Duration::ZERO), 0);
        assert_eq!(
            super::windows_wait_timeout_millis(Duration::from_nanos(1)),
            1
        );
        assert_eq!(
            super::windows_wait_timeout_millis(Duration::from_millis(15)),
            15
        );
        assert_eq!(
            super::windows_wait_timeout_millis(Duration::from_millis(15) + Duration::from_nanos(1)),
            16
        );
    }

    #[test]
    fn windows_message_wait_keeps_infinite_sentinel_reserved() {
        assert_eq!(
            super::windows_wait_timeout_millis(Duration::from_secs(u64::MAX)),
            u32::MAX - 1
        );
    }
}
