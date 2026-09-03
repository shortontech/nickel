use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        allocator::{
            Allocator, Fourcc, Modifier,
            dumb::{DumbAllocator, DumbBuffer},
            format::FormatSet,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, DrmSurface, PlaneConfig, PlaneState,
            compositor::{FrameFlags, PrimaryPlaneElement},
            dumb::{DumbFramebuffer, framebuffer_from_dumb_buffer},
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
        },
        egl::context::ContextPriority,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            Bind, Color32F, ExportMem, Frame, ImportAll, ImportDma, ImportMem, Offscreen, Renderer,
            TextureMapping,
            damage::OutputDamageTracker,
            element::{
                AsRenderElements, Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::{SolidColorBuffer, SolidColorRenderElement},
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
                utils::{ConstrainAlign, ConstrainScaleBehavior, constrain_render_elements},
            },
            gles::{GlesRenderer, GlesTexture},
            multigpu::{GpuManager, gbm::GbmGlesBackend},
            utils::draw_render_elements,
        },
        session::{
            Event as SessionEvent, Session,
            libseat::{self, LibSeatSession},
        },
        udev::{UdevBackend, UdevEvent, all_gpus, primary_gpu},
    },
    desktop::space::SpaceRenderElements,
    output::{Mode, Output, PhysicalProperties},
    reexports::{
        calloop::{
            EventLoop, RegistrationToken, channel,
            timer::{TimeoutAction, Timer},
        },
        drm::{
            buffer::Buffer as _,
            control::{Device as _, Mode as DrmMode, ModeTypeFlags, connector, crtc},
        },
        input::Libinput,
        rustix::fs::OFlags,
        wayland_server::{Resource, backend::GlobalId},
    },
    utils::{Buffer, DeviceFd, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
    wayland::seat::WaylandFocus,
};
use thiserror::Error;

use nickel_core::{
    resource_owner::DependencyOwnerToken,
    shell_settings::ShellSettings,
    theme::{Appearance, ThemePalette},
};

use crate::{
    NickelSession,
    backend::{
        OutputLayout, SessionActivity,
        drm_scanner::{DrmScanEvent, DrmScanner},
    },
    state::PreviewFrame,
};

const FORMATS: &[Fourcc] = &[Fourcc::Abgr8888, Fourcc::Argb8888];
const BOOTSTRAP_RENDER_TIMEOUT: Duration = Duration::from_secs(30);
// EVDI presentation includes a synchronous GPU readback, CPU copy into a dumb
// buffer, and an atomic commit. Unlike a normal DRM output it has no vblank
// completion event to provide natural pacing, so an eager client could keep
// the compositor event-loop thread in this path indefinitely.
const EVDI_MIN_RENDER_INTERVAL: Duration = Duration::from_millis(16);
const SWITCHER_MAX_CARDS: usize = 5;

fn output_model(connector_name: &str) -> String {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return connector_name.to_owned();
    };
    let suffix = format!("-{connector_name}");
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        if !file_name.to_string_lossy().ends_with(&suffix) {
            continue;
        }
        let Ok(edid) = std::fs::read(entry.path().join("edid")) else {
            continue;
        };
        for descriptor in edid.get(54..126).unwrap_or_default().chunks_exact(18) {
            if descriptor[..5] != [0, 0, 0, 0xfc, 0] {
                continue;
            }
            let model = String::from_utf8_lossy(&descriptor[5..18])
                .trim_matches(['\0', '\n', '\r', ' '])
                .to_owned();
            if !model.is_empty() {
                return model;
            }
        }
    }
    connector_name.to_owned()
}

fn connector_name(connector: &connector::Info) -> String {
    format!(
        "{}-{}",
        connector.interface().as_str(),
        connector.interface_id()
    )
}

type RendererBackend = GbmGlesBackend<GlesRenderer, DrmDeviceFd>;
type NativeRenderer<'a> =
    smithay::backend::renderer::multigpu::MultiRenderer<'a, 'a, RendererBackend, RendererBackend>;
smithay::backend::renderer::element::render_elements! {
    NativeCustomElement<R> where R: ImportAll + ImportMem;
    Surface=WaylandSurfaceRenderElement<R>,
    Pointer=MemoryRenderBufferRenderElement<R>,
    Solid=SolidColorRenderElement,
}
smithay::backend::renderer::element::render_elements! {
    NativeElement<R, E> where R: ImportAll + ImportMem;
    Space=SpaceRenderElements<R, E>,
    Custom=NativeCustomElement<R>,
}
type NativeDrmOutput =
    DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;
type NativeOutputManager = DrmOutputManager<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    (),
    DrmDeviceFd,
>;
enum OutputManager {
    Gbm(NativeOutputManager),
    Evdi(DrmDevice),
}

impl OutputManager {
    fn device(&self) -> &DrmDevice {
        match self {
            Self::Gbm(manager) => manager.device(),
            Self::Evdi(device) => device,
        }
    }

    fn pause(&mut self) {
        match self {
            Self::Gbm(manager) => manager.pause(),
            Self::Evdi(device) => device.pause(),
        }
    }

    fn activate(&mut self) -> Result<(), smithay::backend::drm::DrmError> {
        match self {
            Self::Gbm(manager) => manager.lock().activate(false),
            Self::Evdi(device) => device.activate(false),
        }
    }
}

enum OutputDrm {
    Gbm(NativeDrmOutput),
    Evdi(Box<EvdiOutput>),
}

impl OutputDrm {
    fn reset_buffer_ages(&mut self) {
        match self {
            Self::Gbm(output) => {
                output.with_compositor(|compositor| compositor.reset_buffer_ages())
            }
            Self::Evdi(output) => output.invalidate(),
        }
    }

    fn format(&self) -> Fourcc {
        match self {
            Self::Gbm(output) => output.format(),
            Self::Evdi(output) => output.format,
        }
    }

    fn frame_submitted(&mut self) -> Result<(), String> {
        match self {
            Self::Gbm(output) => output
                .frame_submitted()
                .map(|_| ())
                .map_err(|error| error.to_string()),
            Self::Evdi(output) => {
                output.frame_submitted();
                Ok(())
            }
        }
    }
}

struct EvdiScanoutBuffer {
    buffer: DumbBuffer,
    framebuffer: DumbFramebuffer,
}

struct EvdiOutput {
    surface: DrmSurface,
    buffers: [EvdiScanoutBuffer; 2],
    displayed: usize,
    format: Fourcc,
    size: Size<i32, Physical>,
    render_target: Option<GlesTexture>,
    damage_tracker: OutputDamageTracker,
    previous_damage: Option<Rectangle<i32, Buffer>>,
    diagnostics: EvdiCopyoutDiagnostics,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct EvdiCopyoutDiagnostics {
    rendered_bytes: u64,
    mapped_bytes: u64,
    copied_bytes: u64,
    submitted_bytes: u64,
    full_copies: u64,
    partial_copies: u64,
    unchanged_frames: u64,
    submissions: u64,
    retries: u64,
    failures: u64,
    total_copy_micros: u64,
    max_copy_micros: u64,
    total_present_micros: u64,
    max_present_micros: u64,
}

impl EvdiOutput {
    fn new(
        device: &mut DrmDevice,
        crtc: crtc::Handle,
        mode: DrmMode,
        connector: connector::Handle,
    ) -> Result<Self, String> {
        let size = Size::<i32, Physical>::from((mode.size().0 as i32, mode.size().1 as i32));
        let fd = device.device_fd().clone();
        let surface = device
            .create_surface(crtc, mode, &[connector])
            .map_err(|error| error.to_string())?;
        let mut allocator = DumbAllocator::new(fd.clone());
        let mut create = || -> Result<EvdiScanoutBuffer, String> {
            let buffer = allocator
                .create_buffer(
                    size.w as u32,
                    size.h as u32,
                    Fourcc::Abgr8888,
                    &[Modifier::Linear],
                )
                .map_err(|error| error.to_string())?;
            let framebuffer = framebuffer_from_dumb_buffer(&fd, &buffer, false)
                .map_err(|error| error.to_string())?;
            Ok(EvdiScanoutBuffer {
                buffer,
                framebuffer,
            })
        };
        let buffers = [create()?, create()?];
        let output = Self {
            surface,
            buffers,
            displayed: 0,
            format: Fourcc::Abgr8888,
            size,
            render_target: None,
            damage_tracker: OutputDamageTracker::new(size, Scale::from(1.0), Transform::Normal),
            previous_damage: None,
            diagnostics: EvdiCopyoutDiagnostics::default(),
        };
        output
            .surface
            .commit([output.plane_state(0)], false)
            .map_err(|error| error.to_string())?;
        Ok(output)
    }

    fn plane_state(&self, buffer: usize) -> PlaneState<'_> {
        PlaneState {
            handle: self.surface.plane(),
            config: Some(PlaneConfig {
                src: Rectangle::from_size(Size::<i32, Buffer>::from((self.size.w, self.size.h)))
                    .to_f64(),
                dst: Rectangle::from_size(self.size),
                transform: Transform::Normal,
                alpha: 1.0,
                damage_clips: None,
                fb: *self.buffers[buffer].framebuffer.as_ref(),
                fence: None,
            }),
        }
    }

    fn invalidate(&mut self) {
        self.damage_tracker =
            OutputDamageTracker::new(self.size, Scale::from(1.0), Transform::Normal);
        self.previous_damage = None;
    }

    fn frame_submitted(&mut self) {
        // EVDI does not reliably emit page-flip completion events. Its atomic
        // framebuffer update is committed synchronously without an event.
    }

    fn reactivate(&mut self) -> Result<(), String> {
        self.surface
            .reset_state()
            .map_err(|error| error.to_string())?;
        self.surface
            .commit([self.plane_state(self.displayed)], false)
            .map_err(|error| error.to_string())?;
        self.invalidate();
        Ok(())
    }

    fn retained_bytes(&self) -> usize {
        self.buffers
            .iter()
            .map(|buffer| buffer.buffer.handle().pitch() as usize * self.size.h as usize)
            .sum()
    }

    fn render_and_present<'a>(
        &mut self,
        renderer: &mut NativeRenderer<'a>,
        elements: &[NativeElement<
            NativeRenderer<'a>,
            WaylandSurfaceRenderElement<NativeRenderer<'a>>,
        >],
    ) -> Result<bool, String> {
        let started = Instant::now();
        if self.render_target.is_none() {
            self.render_target = Some(
                <NativeRenderer<'a> as Offscreen<GlesTexture>>::create_buffer(
                    renderer,
                    Fourcc::Abgr8888,
                    Size::<i32, Buffer>::from((self.size.w, self.size.h)),
                )
                .map_err(|error| error.to_string())?,
            );
        }
        let mut framebuffer = renderer
            .bind(
                self.render_target
                    .as_mut()
                    .expect("render target initialized"),
            )
            .map_err(|error| error.to_string())?;
        let rendered = self
            .damage_tracker
            .render_output(
                renderer,
                &mut framebuffer,
                1,
                elements,
                [0.1, 0.1, 0.1, 1.0],
            )
            .map_err(|error| error.to_string())?;
        let Some(current_damage) = rendered.damage.cloned() else {
            self.diagnostics.unchanged_frames = self.diagnostics.unchanged_frames.saturating_add(1);
            return Ok(false);
        };
        rendered.sync.wait().map_err(|error| error.to_string())?;
        self.diagnostics.rendered_bytes = self
            .diagnostics
            .rendered_bytes
            .saturating_add(damage_bytes(&current_damage));

        // The target dumb buffer was displayed two submissions ago. Include
        // the preceding frame's damage so alternating buffers converge on the
        // current persistent render target without a full-frame copy.
        let current_region = damage_bounding_box(&current_damage)
            .ok_or_else(|| "damaged EVDI frame had no copy region".to_owned())?;
        let copy_region = self.previous_damage.map_or(current_region, |previous| {
            union_rectangles(previous, current_region)
        });
        let mapping = renderer
            .copy_framebuffer(&framebuffer, copy_region, Fourcc::Abgr8888)
            .map_err(|error| error.to_string())?;
        let flipped = mapping.flipped();
        let mapped = renderer
            .map_texture(&mapping)
            .map_err(|error| error.to_string())?;
        let mapped_bytes = copy_region.size.w as u64 * copy_region.size.h as u64 * 4;
        self.diagnostics.mapped_bytes = self.diagnostics.mapped_bytes.saturating_add(mapped_bytes);
        drop(framebuffer);
        let result = self.present_region(mapped, flipped, copy_region);
        if result.is_ok() {
            self.previous_damage = Some(current_region);
        }
        let elapsed = elapsed_micros(started);
        self.diagnostics.total_present_micros = self
            .diagnostics
            .total_present_micros
            .saturating_add(elapsed);
        self.diagnostics.max_present_micros = self.diagnostics.max_present_micros.max(elapsed);
        match &result {
            Ok(true) => {}
            Ok(false) => {}
            Err(_) => {
                self.diagnostics.failures = self.diagnostics.failures.saturating_add(1);
                self.diagnostics.retries = self.diagnostics.retries.saturating_add(1);
            }
        }
        result
    }

    fn present_region(
        &mut self,
        mapped: &[u8],
        flipped: bool,
        region: Rectangle<i32, Buffer>,
    ) -> Result<bool, String> {
        let next = 1 - self.displayed;
        {
            let fd = self.surface.device_fd();
            let buffer = &mut self.buffers[next];
            let mut raw = *buffer.buffer.handle();
            let pitch = raw.pitch() as usize;
            let mut mapping = fd
                .map_dumb_buffer(&mut raw)
                .map_err(|error| error.to_string())?;
            let copy_started = Instant::now();
            let copied = copy_mapped_region_to_strided(
                &mut mapping,
                pitch,
                mapped,
                flipped,
                region,
                self.size,
            )?;
            let copy_micros = elapsed_micros(copy_started);
            self.diagnostics.total_copy_micros = self
                .diagnostics
                .total_copy_micros
                .saturating_add(copy_micros);
            self.diagnostics.max_copy_micros = self.diagnostics.max_copy_micros.max(copy_micros);
            self.diagnostics.copied_bytes =
                self.diagnostics.copied_bytes.saturating_add(copied as u64);
            if copied
                == (self.size.w as usize)
                    .saturating_mul(self.size.h as usize)
                    .saturating_mul(4)
            {
                self.diagnostics.full_copies = self.diagnostics.full_copies.saturating_add(1);
            } else if copied > 0 {
                self.diagnostics.partial_copies = self.diagnostics.partial_copies.saturating_add(1);
            }
        }
        self.surface
            .commit([self.plane_state(next)], false)
            .map_err(|error| error.to_string())?;
        self.displayed = next;
        self.diagnostics.submissions = self.diagnostics.submissions.saturating_add(1);
        self.diagnostics.submitted_bytes = self
            .diagnostics
            .submitted_bytes
            .saturating_add(region.size.w as u64 * region.size.h as u64 * 4);
        if self.diagnostics.submissions.is_multiple_of(300) {
            tracing::info!(
                diagnostics = ?self.diagnostics,
                "EVDI copyout runtime diagnostics"
            );
        }
        Ok(true)
    }
}

fn elapsed_micros(started: Instant) -> u64 {
    started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

#[derive(Debug, Eq, PartialEq)]
struct OutputId {
    device: DrmNode,
    crtc: crtc::Handle,
}

struct SurfaceData {
    global: Option<GlobalId>,
    output: Output,
    drm: OutputDrm,
    background: SolidColorBuffer,
    render_path_logged: bool,
    invalidate_pending: bool,
}

struct DisabledOutput<N = DrmNode, T = Output> {
    node: N,
    output: T,
    present: bool,
}

fn published_disabled_outputs<K, N, T>(
    outputs: &HashMap<K, DisabledOutput<N, T>>,
) -> impl Iterator<Item = &T> {
    outputs
        .values()
        .filter(|disabled| disabled.present)
        .map(|disabled| &disabled.output)
}

fn mark_disabled_outputs_absent<K, N: Eq, T>(
    outputs: &mut HashMap<K, DisabledOutput<N, T>>,
    node: &N,
) {
    for disabled in outputs
        .values_mut()
        .filter(|disabled| &disabled.node == node)
    {
        disabled.present = false;
    }
}

struct DeviceData {
    generation: u64,
    registration: RegistrationToken,
    manager: OutputManager,
    scanner: DrmScanner,
    render_node: DrmNode,
    owns_renderer: bool,
    is_evdi: bool,
    render_scheduled: bool,
    last_render_started: Option<Instant>,
    surfaces: HashMap<crtc::Handle, SurfaceData>,
}

fn release_device_data(
    native: &mut UdevData,
    event_loop_handle: &smithay::reexports::calloop::LoopHandle<'static, NickelSession>,
    node: DrmNode,
    device: DeviceData,
) {
    if device.owns_renderer {
        native.gpus.as_mut().remove_node(&device.render_node);
    }
    let session_fd = device.manager.device().device_fd().device_fd();
    event_loop_handle.remove(device.registration);
    drop(device);
    close_libseat_device(&mut native.session, node, session_fd);
    let retired = native.renderer_lifecycle.retire(node);
    debug_assert!(
        retired,
        "retired DRM resource must have a lifecycle generation"
    );
}

fn close_libseat_device(session: &mut LibSeatSession, node: DrmNode, fd: DeviceFd) {
    match fd.try_into() {
        Ok(fd) => {
            if let Err(error) = session.close(fd) {
                tracing::error!(%node, %error, "failed to close DRM device through libseat");
            }
        }
        Err(_) => {
            tracing::error!(%node, "retired DRM device still has live file-descriptor owners");
        }
    }
}

#[derive(Clone)]
struct DiscoveredDevice {
    path: PathBuf,
    is_evdi: bool,
    driver: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DrmRenderStrategy {
    Gbm,
    EvdiCpuCopyout,
    EvdiLlvmpipeFallback,
}

fn drm_render_strategy(is_evdi: bool, force_evdi_fallback: bool) -> DrmRenderStrategy {
    match (is_evdi, force_evdi_fallback) {
        (true, true) => DrmRenderStrategy::EvdiLlvmpipeFallback,
        (true, false) => DrmRenderStrategy::EvdiCpuCopyout,
        (false, _) => DrmRenderStrategy::Gbm,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RendererRetainedReason {
    ActiveSurfaces,
    PrimaryForCrossGpu,
    PendingDependentRecovery,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RendererDeviceDiagnostics {
    pub node: DrmNode,
    pub render_node: Option<DrmNode>,
    pub device_path: PathBuf,
    pub driver: String,
    pub active_surfaces: usize,
    pub reason: Option<RendererRetainedReason>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RendererLifecycleDiagnostics {
    pub discovered_devices: usize,
    pub live_gpu_nodes: usize,
    pub live_output_managers: usize,
    pub active_surfaces: usize,
    pub retained_primary_renderers: usize,
    pub activations: u64,
    pub retirements: u64,
    pub devices: Vec<RendererDeviceDiagnostics>,
}

#[derive(Debug)]
struct RendererLifecycleLedger<K> {
    live_generations: HashMap<K, u64>,
    next_generation: u64,
    activations: u64,
    retirements: u64,
}

impl<K> Default for RendererLifecycleLedger<K> {
    fn default() -> Self {
        Self {
            live_generations: HashMap::new(),
            next_generation: 1,
            activations: 0,
            retirements: 0,
        }
    }
}

impl<K: Copy + Eq + Hash> RendererLifecycleLedger<K> {
    fn activate(&mut self, key: K) -> u64 {
        if let Some(generation) = self.live_generations.get(&key) {
            return *generation;
        }
        let generation = self.next_generation;
        self.next_generation = self.next_generation.saturating_add(1);
        self.live_generations.insert(key, generation);
        self.activations = self.activations.saturating_add(1);
        generation
    }

    fn retire(&mut self, key: K) -> bool {
        if self.live_generations.remove(&key).is_none() {
            return false;
        }
        self.retirements = self.retirements.saturating_add(1);
        true
    }

    fn generation(&self, key: K) -> Option<u64> {
        self.live_generations.get(&key).copied()
    }
}

pub struct UdevData {
    _renderer_owner: DependencyOwnerToken,
    session: LibSeatSession,
    activity: SessionActivity,
    gpus: GpuManager<RendererBackend>,
    primary_gpu: DrmNode,
    discovered_devices: HashMap<DrmNode, DiscoveredDevice>,
    devices: HashMap<DrmNode, DeviceData>,
    renderer_lifecycle: RendererLifecycleLedger<DrmNode>,
    pending_primary_dependents: HashSet<DrmNode>,
    disabled_outputs: HashMap<String, DisabledOutput>,
    layout: OutputLayout,
    bootstrap_render_until: Instant,
    client_bootstrap_started: bool,
    cursors: HashMap<crate::window_frame::FrameCursor, CursorBuffer>,
    frame_icons: Option<crate::window_frame::FrameIcons>,
    identify_badges: IdentifyBadgeCache,
    task_switcher_cache: Option<TaskSwitcherBufferCache>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct IdentifyBadgeDiagnostics {
    pub(crate) live_bytes: usize,
    pub(crate) peak_bytes: usize,
    pub(crate) entries: usize,
    pub(crate) rasterizations: u64,
    pub(crate) avoided_rasterizations: u64,
    pub(crate) evictions: u64,
    /// Smithay owns renderer imports; their byte cost is not exposed by its API.
    pub(crate) renderer_bytes: Option<usize>,
}

#[derive(Default)]
struct IdentifyBadgeCache {
    entries: HashMap<usize, MemoryRenderBuffer>,
    peak_bytes: usize,
    rasterizations: u64,
    avoided_rasterizations: u64,
    evictions: u64,
}

impl IdentifyBadgeCache {
    fn get(&mut self, index: usize) -> MemoryRenderBuffer {
        if let Some(badge) = self.entries.get(&index) {
            self.avoided_rasterizations = self.avoided_rasterizations.saturating_add(1);
            return badge.clone();
        }
        let badge = identify_badge(index + 1);
        self.entries.insert(index, badge.clone());
        self.rasterizations = self.rasterizations.saturating_add(1);
        self.peak_bytes = self
            .peak_bytes
            .max(self.entries.len() * IDENTIFY_BADGE_BYTES);
        badge
    }

    fn retire(&mut self) {
        self.evictions = self.evictions.saturating_add(self.entries.len() as u64);
        self.entries.clear();
    }

    fn retain_output_count(&mut self, output_count: usize) {
        let before = self.entries.len();
        self.entries.retain(|index, _| *index < output_count);
        self.evictions = self
            .evictions
            .saturating_add(before.saturating_sub(self.entries.len()) as u64);
    }

    fn diagnostics(&self) -> IdentifyBadgeDiagnostics {
        IdentifyBadgeDiagnostics {
            live_bytes: self.entries.len() * IDENTIFY_BADGE_BYTES,
            peak_bytes: self.peak_bytes,
            entries: self.entries.len(),
            rasterizations: self.rasterizations,
            avoided_rasterizations: self.avoided_rasterizations,
            evictions: self.evictions,
            renderer_bytes: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TaskSwitcherBufferKey {
    candidates: Vec<crate::window_registry::WindowId>,
    selected: usize,
    output_size: (i32, i32),
    preview_generation: u64,
}

struct TaskSwitcherBufferCache {
    key: TaskSwitcherBufferKey,
    buffer: MemoryRenderBuffer,
    size: smithay::utils::Size<i32, Physical>,
}

impl UdevData {
    pub(crate) fn disabled_outputs(&self) -> impl Iterator<Item = &Output> {
        published_disabled_outputs(&self.disabled_outputs)
    }

    pub(crate) fn identify_badge_diagnostics(&self) -> IdentifyBadgeDiagnostics {
        self.identify_badges.diagnostics()
    }

    pub(crate) fn retire_identify_badges(&mut self) {
        self.identify_badges.retire();
    }

    pub(crate) fn reconcile_identify_badges(&mut self, output_count: usize) {
        self.identify_badges.retain_output_count(output_count);
    }

    pub(crate) fn renderer_lifecycle_diagnostics(&self) -> RendererLifecycleDiagnostics {
        let secondary_surfaces = self
            .devices
            .iter()
            .filter(|(node, _)| **node != self.primary_gpu)
            .map(|(_, device)| device.surfaces.len())
            .sum::<usize>();
        RendererLifecycleDiagnostics {
            discovered_devices: self.discovered_devices.len(),
            live_gpu_nodes: self
                .devices
                .values()
                .filter(|device| device.owns_renderer)
                .count(),
            live_output_managers: self.devices.len(),
            active_surfaces: self
                .devices
                .values()
                .map(|device| device.surfaces.len())
                .sum(),
            retained_primary_renderers: usize::from(self.devices.contains_key(&self.primary_gpu)),
            activations: self.renderer_lifecycle.activations,
            retirements: self.renderer_lifecycle.retirements,
            devices: self
                .discovered_devices
                .iter()
                .map(|(node, discovered)| {
                    let live = self.devices.get(node);
                    let active_surfaces = live.map_or(0, |device| device.surfaces.len());
                    RendererDeviceDiagnostics {
                        node: *node,
                        render_node: live.map(|device| device.render_node),
                        device_path: discovered.path.clone(),
                        driver: discovered.driver.clone(),
                        active_surfaces,
                        reason: renderer_retained_reason(
                            *node == self.primary_gpu,
                            active_surfaces,
                            secondary_surfaces > 0,
                            !self.pending_primary_dependents.is_empty(),
                        ),
                    }
                })
                .collect(),
        }
    }

    pub(crate) fn clear_task_switcher_cache(&mut self) {
        self.task_switcher_cache = None;
    }
}

fn renderer_retained_reason(
    is_primary: bool,
    active_surfaces: usize,
    secondary_active: bool,
    pending_dependent_recovery: bool,
) -> Option<RendererRetainedReason> {
    if active_surfaces > 0 {
        Some(RendererRetainedReason::ActiveSurfaces)
    } else if is_primary && secondary_active {
        Some(RendererRetainedReason::PrimaryForCrossGpu)
    } else if is_primary && pending_dependent_recovery {
        Some(RendererRetainedReason::PendingDependentRecovery)
    } else {
        None
    }
}

fn device_activation_priority<K: Eq>(node: K, primary_gpu: K) -> usize {
    usize::from(node != primary_gpu)
}

fn primary_dependency_to_activate<K: Copy + Eq>(
    node: K,
    primary_gpu: K,
    primary_discovered: bool,
    primary_live: bool,
) -> Option<K> {
    (node != primary_gpu && primary_discovered && !primary_live).then_some(primary_gpu)
}

fn render_primary_available<K: Eq>(
    node: K,
    primary_gpu: K,
    primary_live: bool,
    primary_will_activate: bool,
) -> bool {
    node == primary_gpu || primary_live || primary_will_activate
}

fn dependent_renderers_after_primary_removal<K: Copy + Eq>(
    removed: K,
    primary_gpu: K,
    live_nodes: impl IntoIterator<Item = K>,
) -> Vec<K> {
    if removed != primary_gpu {
        return Vec::new();
    }
    live_nodes
        .into_iter()
        .filter(|candidate| *candidate != primary_gpu)
        .collect()
}

fn pending_recovery_devices<K: Copy + Eq + Hash, V>(
    pending: &HashSet<K>,
    discovered: &HashMap<K, V>,
) -> Vec<K> {
    pending
        .iter()
        .filter(|node| discovered.contains_key(node))
        .copied()
        .collect()
}

fn consume_pending_dependent<K: Copy + Eq + Hash>(
    pending: &mut HashSet<K>,
    node: K,
    primary_gpu: K,
) -> bool {
    node != primary_gpu && pending.remove(&node)
}

#[derive(Clone)]
struct CursorBuffer {
    buffer: MemoryRenderBuffer,
    hotspot: Point<i32, Physical>,
}

#[derive(Debug, Error)]
enum DeviceError {
    #[error("failed to open DRM device through libseat: {0}")]
    Open(#[from] libseat::Error),
    #[error("failed to initialize DRM device: {0}")]
    Drm(#[from] smithay::backend::drm::DrmError),
    #[error("failed to initialize GBM device: {0}")]
    Gbm(#[from] std::io::Error),
    #[error("failed to initialize EGL renderer: {0}")]
    Egl(#[from] smithay::backend::egl::Error),
    #[error("failed to acquire renderer: {0}")]
    Renderer(String),
    #[error("failed to register DRM event source: {0}")]
    Registration(String),
    #[error("secondary DRM device cannot render while the primary GPU is unavailable")]
    MissingPrimary,
}

pub fn init_udev(
    event_loop: &mut EventLoop<'static, NickelSession>,
    data: &mut NickelSession,
) -> Result<(), Box<dyn std::error::Error>> {
    let renderer_owner = nickel_core::resource_owner::try_acquire_smithay_renderer_owner()?;
    let (session, notifier) = LibSeatSession::new()?;
    let seat_name = session.seat();
    let udev = UdevBackend::new(&seat_name)?;
    let primary_gpu = select_primary_gpu(&session)?;
    let gpus = GpuManager::new(GbmGlesBackend::with_context_priority(ContextPriority::High))?;

    data.native = Some(UdevData {
        _renderer_owner: renderer_owner,
        session: session.clone(),
        activity: SessionActivity::default(),
        gpus,
        primary_gpu,
        discovered_devices: HashMap::new(),
        devices: HashMap::new(),
        renderer_lifecycle: RendererLifecycleLedger::default(),
        pending_primary_dependents: HashSet::new(),
        disabled_outputs: HashMap::new(),
        layout: OutputLayout::default(),
        bootstrap_render_until: Instant::now() + BOOTSTRAP_RENDER_TIMEOUT,
        client_bootstrap_started: false,
        cursors: themed_cursors(),
        frame_icons: crate::window_frame::FrameIcons::load(),
        identify_badges: IdentifyBadgeCache::default(),
        task_switcher_cache: None,
    });
    let (buffer_commit_tx, buffer_commit_rx) = channel::channel();
    data.buffer_commit_tx = Some(buffer_commit_tx);
    event_loop
        .handle()
        .insert_source(buffer_commit_rx, |event, _, data| {
            let channel::Event::Msg(commit) = event else {
                return;
            };
            let surface = commit.surface;
            if let Some(native) = data.native.as_mut()
                && let Err(error) = native.gpus.early_import(native.primary_gpu, &surface)
            {
                tracing::warn!(?error, "failed to import client buffer on the primary GPU");
            }
            // Synchronized subsurface state is latched by a later ancestor
            // commit. Import its buffer now, but do not present the partial
            // surface-tree transaction.
            if !commit.render_visible {
                return;
            }
            if let Some(native) = data.native.as_mut()
                && !native.client_bootstrap_started
            {
                native.client_bootstrap_started = true;
                native.bootstrap_render_until = Instant::now() + BOOTSTRAP_RENDER_TIMEOUT;
            }
            let mut root = surface.clone();
            while let Some(parent) = smithay::wayland::compositor::get_parent(&root) {
                root = parent;
            }
            let affected_outputs = data
                .space
                .elements()
                .find(|window| {
                    window
                        .toplevel()
                        .is_some_and(|toplevel| toplevel.wl_surface() == &root)
                })
                .map(|window| {
                    data.space
                        .outputs_for_element(window)
                        .into_iter()
                        .map(|output| output.name())
                        .collect::<HashSet<_>>()
                })
                .unwrap_or_default();
            let client_bootstrapping = data.native.as_ref().is_some_and(|native| {
                native.client_bootstrap_started && Instant::now() < native.bootstrap_render_until
            });
            if let Some(native) = data.native.as_mut() {
                native
                    .devices
                    .values_mut()
                    .filter(|device| client_bootstrapping || device.is_evdi)
                    .flat_map(|device| device.surfaces.values_mut())
                    .filter(|surface| {
                        client_bootstrapping
                            || affected_outputs.is_empty()
                            || affected_outputs.contains(&surface.output.name())
                    })
                    .for_each(|surface| surface.invalidate_pending = true);
            }
            let nodes = data
                .native
                .as_ref()
                .map(|native| {
                    native
                        .devices
                        .iter()
                        .filter(|(_, device)| {
                            affected_outputs.is_empty()
                                || device.surfaces.values().any(|surface| {
                                    affected_outputs.contains(&surface.output.name())
                                })
                        })
                        .map(|(node, _)| *node)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for node in nodes {
                data.schedule_render(node, Duration::ZERO);
            }
        })?;

    let devices: Vec<_> = udev
        .device_list()
        .filter_map(|(id, path)| {
            DrmNode::from_dev_id(id)
                .ok()
                .map(|node| (node, path.to_owned()))
        })
        .collect();
    for (node, path) in &devices {
        data.discover_drm_device(*node, path);
    }
    let mut devices = devices;
    devices.sort_by_key(|(node, _)| device_activation_priority(*node, primary_gpu));
    for (node, path) in devices {
        if let Err(error) = data.add_drm_device(event_loop, node, &path) {
            tracing::warn!(%node, %error, "skipping DRM device");
        }
    }
    if data
        .native
        .as_ref()
        .is_none_or(|native| native.devices.is_empty())
    {
        return Err("no usable DRM device was found".into());
    }
    let usable_outputs = data
        .native
        .as_ref()
        .map(|native| {
            native
                .devices
                .values()
                .map(|device| device.surfaces.len())
                .sum::<usize>()
        })
        .unwrap_or_default();
    if usable_outputs == 0 {
        return Err("DRM devices were opened, but no connected output could be initialized".into());
    }

    let mut libinput =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput
        .udev_assign_seat(&seat_name)
        .map_err(|()| "libinput rejected the active seat")?;
    let input = LibinputInputBackend::new(libinput.clone());
    event_loop.handle().insert_source(input, |event, _, data| {
        if let Some(vt) = data.process_input_event(event)
            && let Some(native) = data.native.as_mut()
            && let Err(error) = native.session.change_vt(vt)
        {
            tracing::error!(vt, ?error, "failed to switch virtual terminal");
        }
        let nodes = data
            .native
            .as_ref()
            .map(|native| native.devices.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for node in nodes {
            data.schedule_render(node, Duration::ZERO);
        }
    })?;

    event_loop
        .handle()
        .insert_source(notifier, move |event, _, data| {
            if matches!(&event, SessionEvent::PauseSession) {
                data.fail_all_image_copy_frames(
                    smithay::wayland::image_copy_capture::CaptureFailureReason::Stopped,
                );
            }
            let Some(native) = data.native.as_mut() else {
                return;
            };
            match event {
                SessionEvent::PauseSession => {
                    native.activity.pause();
                    libinput.suspend();
                    for device in native.devices.values_mut() {
                        device.manager.pause();
                    }
                    tracing::info!("native session paused");
                }
                SessionEvent::ActivateSession => {
                    if let Err(error) = libinput.resume() {
                        tracing::error!(?error, "failed to resume libinput");
                    }
                    native.activity.activate();
                    for device in native.devices.values_mut() {
                        if let Err(error) = device.manager.activate() {
                            tracing::error!(?error, "failed to reactivate DRM device");
                        }
                        for surface in device.surfaces.values_mut() {
                            if let OutputDrm::Evdi(output) = &mut surface.drm
                                && let Err(error) = output.reactivate()
                            {
                                tracing::error!(%error, "failed to reactivate EVDI output");
                            }
                        }
                    }
                    let nodes: Vec<_> = native.devices.keys().copied().collect();
                    for node in nodes {
                        data.schedule_render(node, Duration::ZERO);
                    }
                    tracing::info!("native session resumed");
                }
            }
        })?;

    event_loop
        .handle()
        .insert_source(udev, |event, _, data| match event {
            UdevEvent::Added { device_id, path } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    let handle = data.event_loop_handle.clone();
                    if let Err(error) = data.add_drm_device_with_handle(&handle, node, &path) {
                        tracing::warn!(%node, %error, "failed to add hotplugged DRM device");
                    }
                }
            }
            UdevEvent::Changed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    let known = data
                        .native
                        .as_ref()
                        .is_some_and(|native| native.devices.contains_key(&node));
                    if known {
                        data.scan_connectors(node);
                    } else if let Some(path) = node.dev_path() {
                        let handle = data.event_loop_handle.clone();
                        if let Err(error) = data.add_drm_device_with_handle(&handle, node, &path) {
                            tracing::warn!(%node, %error, "failed to add changed DRM device");
                        }
                    }
                }
            }
            UdevEvent::Removed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    data.remove_drm_device(node);
                }
            }
        })?;

    unsafe { std::env::set_var("WAYLAND_DISPLAY", &data.socket_name) };
    tracing::info!(seat = %seat_name, gpu = %primary_gpu, "native backend initialized");
    Ok(())
}

fn select_primary_gpu(session: &LibSeatSession) -> Result<DrmNode, Box<dyn std::error::Error>> {
    if let Some(path) = std::env::var_os("NICKEL_DRM_DEVICE") {
        return Ok(DrmNode::from_path(path)?);
    }
    if let Some(path) = primary_gpu(session.seat())? {
        return Ok(DrmNode::from_path(path)?);
    }
    let path = all_gpus(session.seat())?
        .into_iter()
        .next()
        .ok_or("no DRM GPU exists on the active seat")?;
    Ok(DrmNode::from_path(path)?)
}

impl NickelSession {
    pub(crate) fn native_output_without_global(&self, identity: &str) -> Option<Output> {
        self.native.as_ref().and_then(|native| {
            native.devices.values().find_map(|device| {
                device
                    .surfaces
                    .values()
                    .find(|surface| surface.output.name() == identity && surface.global.is_none())
                    .map(|surface| surface.output.clone())
            })
        })
    }

    pub(crate) fn set_native_output_global(&mut self, identity: &str, global: GlobalId) -> bool {
        let Some(surface) = self.native.as_mut().and_then(|native| {
            native.devices.values_mut().find_map(|device| {
                device
                    .surfaces
                    .values_mut()
                    .find(|surface| surface.output.name() == identity)
            })
        }) else {
            return false;
        };
        surface.global = Some(global);
        true
    }

    fn discover_drm_device(&mut self, node: DrmNode, path: &Path) {
        let native = self.native.as_mut().expect("native backend should exist");
        let is_evdi = is_evdi_device(path);
        native.discovered_devices.insert(
            node,
            DiscoveredDevice {
                path: path.to_owned(),
                is_evdi,
                driver: drm_driver_name(path, is_evdi),
            },
        );
    }

    pub(crate) fn render_all_outputs_once(&mut self) {
        let wave = self.begin_preview_render_wave();
        let nodes = self
            .native
            .as_ref()
            .map(|native| native.devices.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for node in &nodes {
            self.render_node_in_wave(*node, wave);
        }
    }

    pub(crate) fn render_all_outputs(&mut self) {
        self.render_all_outputs_once();
        let nodes = self
            .native
            .as_ref()
            .map(|native| native.devices.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        let timer = Timer::from_duration(Duration::from_millis(3050));
        let _ = self
            .event_loop_handle
            .insert_source(timer, move |_, _, data| {
                let wave = data.begin_preview_render_wave();
                for node in &nodes {
                    data.render_node_in_wave(*node, wave);
                }
                TimeoutAction::Drop
            });
    }

    fn add_drm_device(
        &mut self,
        event_loop: &mut EventLoop<'static, Self>,
        node: DrmNode,
        path: &Path,
    ) -> Result<(), DeviceError> {
        self.event_loop_handle = event_loop.handle();
        let handle = self.event_loop_handle.clone();
        self.add_drm_device_with_handle(&handle, node, path)
    }

    fn add_drm_device_with_handle(
        &mut self,
        handle: &smithay::reexports::calloop::LoopHandle<'static, Self>,
        node: DrmNode,
        path: &Path,
    ) -> Result<(), DeviceError> {
        self.discover_drm_device(node, path);
        let native = self.native.as_mut().expect("native backend should exist");
        let is_evdi = native
            .discovered_devices
            .get(&node)
            .expect("DRM device was just discovered")
            .is_evdi;
        if native.devices.contains_key(&node) {
            let retry_pending =
                node == native.primary_gpu && !native.pending_primary_dependents.is_empty();
            if retry_pending {
                self.recover_primary_dependents(handle);
                self.retire_inactive_renderers();
            } else {
                self.complete_pending_dependent(node);
            }
            return Ok(());
        }
        if is_evdi
            && matches!(
                has_connected_drm_connector(Path::new("/sys/class/drm"), path),
                Some(false)
            )
        {
            mark_disabled_outputs_absent(&mut native.disabled_outputs, &node);
            tracing::debug!(%node, path = %path.display(), "deferring disconnected EVDI device");
            self.complete_pending_dependent(node);
            return Ok(());
        }
        let primary_to_scan = self.activate_drm_device_with_dependencies(handle, node)?;
        self.scan_connectors(node);
        if let Some(primary_gpu) = primary_to_scan {
            self.scan_connectors(primary_gpu);
        }
        let is_primary = self
            .native
            .as_ref()
            .is_some_and(|native| node == native.primary_gpu);
        if is_primary {
            self.recover_primary_dependents(handle);
        } else {
            self.complete_pending_dependent(node);
        }
        self.retire_inactive_renderers();
        Ok(())
    }

    fn recover_primary_dependents(
        &mut self,
        handle: &smithay::reexports::calloop::LoopHandle<'static, Self>,
    ) {
        let pending = self
            .native
            .as_ref()
            .map(|native| {
                pending_recovery_devices(
                    &native.pending_primary_dependents,
                    &native.discovered_devices,
                )
                .into_iter()
                .map(|node| (node, native.discovered_devices[&node].path.clone()))
                .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (node, path) in pending {
            match self.add_drm_device_with_handle(handle, node, &path) {
                Ok(()) => {
                    tracing::info!(%node, "recovered DRM device after primary returned");
                }
                Err(error) => {
                    tracing::warn!(%node, %error, "failed to recover DRM device after primary returned");
                }
            }
        }
    }

    fn complete_pending_dependent(&mut self, node: DrmNode) {
        if let Some(native) = self.native.as_mut() {
            consume_pending_dependent(
                &mut native.pending_primary_dependents,
                node,
                native.primary_gpu,
            );
        }
        self.retire_inactive_renderers();
    }

    fn activate_drm_device(
        &mut self,
        handle: &smithay::reexports::calloop::LoopHandle<'static, Self>,
        node: DrmNode,
    ) -> Result<(), DeviceError> {
        let native = self.native.as_mut().expect("native backend should exist");
        if native.devices.contains_key(&node) {
            return Ok(());
        }
        let discovered = native
            .discovered_devices
            .get(&node)
            .expect("only discovered DRM devices can be activated")
            .clone();
        let fd = native.session.open(
            &discovered.path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )?;
        let fd = DeviceFd::from(fd);
        let close_fd = fd.clone();
        let activation = (|| {
            let fd = DrmDeviceFd::new(fd);
            let (drm, notifier) = DrmDevice::new(fd.clone(), true)?;
            let strategy = drm_render_strategy(
                discovered.is_evdi,
                std::env::var_os("NICKEL_EVDI_LLVMPIPE_FALLBACK").is_some(),
            );
            if strategy == DrmRenderStrategy::EvdiLlvmpipeFallback {
                tracing::warn!(%node, "EVDI CPU copyout disabled; using temporary llvmpipe fallback");
            }
            let (manager, render_node, owns_renderer) =
                if strategy == DrmRenderStrategy::EvdiCpuCopyout {
                    (OutputManager::Evdi(drm), native.primary_gpu, false)
                } else {
                    let gbm = GbmDevice::new(fd)?;
                    native.gpus.as_mut().add_node(node, gbm.clone())?;
                    let render_node = node;
                    let renderer = match native.gpus.single_renderer(&render_node) {
                        Ok(renderer) => renderer,
                        Err(error) => {
                            native.gpus.as_mut().remove_node(&render_node);
                            return Err(DeviceError::Renderer(error.to_string()));
                        }
                    };
                    let formats = renderer
                        .dmabuf_formats()
                        .iter()
                        .copied()
                        .collect::<FormatSet>();
                    let manager = DrmOutputManager::new(
                        drm,
                        GbmAllocator::new(
                            gbm.clone(),
                            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
                        ),
                        GbmFramebufferExporter::new(gbm.clone(), Some(render_node).into()),
                        Some(gbm),
                        FORMATS.iter().copied(),
                        formats,
                    );
                    drop(renderer);
                    (OutputManager::Gbm(manager), render_node, true)
                };

            let registration =
                match handle.insert_source(notifier, move |event, _, data| match event {
                    DrmEvent::VBlank(crtc) => data.frame_submitted(node, crtc),
                    DrmEvent::Error(error) => {
                        tracing::error!(%node, ?error, "DRM event error")
                    }
                }) {
                    Ok(registration) => registration,
                    Err(error) => {
                        if owns_renderer {
                            native.gpus.as_mut().remove_node(&render_node);
                        }
                        return Err(DeviceError::Registration(format!("{error:?}")));
                    }
                };
            let generation = native.renderer_lifecycle.activate(node);
            native.devices.insert(
                node,
                DeviceData {
                    generation,
                    registration,
                    manager,
                    scanner: DrmScanner::new(),
                    render_node,
                    owns_renderer,
                    is_evdi: discovered.is_evdi,
                    render_scheduled: false,
                    last_render_started: None,
                    surfaces: HashMap::new(),
                },
            );
            let diagnostics = native.renderer_lifecycle_diagnostics();
            tracing::debug!(?diagnostics, "DRM renderer lifecycle state");
            tracing::info!(%node, path = %discovered.path.display(), "DRM renderer activated");
            Ok(())
        })();
        if activation.is_err() {
            close_libseat_device(&mut native.session, node, close_fd);
        }
        activation
    }

    fn activate_drm_device_with_dependencies(
        &mut self,
        handle: &smithay::reexports::calloop::LoopHandle<'static, Self>,
        node: DrmNode,
    ) -> Result<Option<DrmNode>, DeviceError> {
        let primary_to_activate = self.native.as_ref().and_then(|native| {
            primary_dependency_to_activate(
                node,
                native.primary_gpu,
                native.discovered_devices.contains_key(&native.primary_gpu),
                native.devices.contains_key(&native.primary_gpu),
            )
        });
        let primary_available = self.native.as_ref().is_some_and(|native| {
            render_primary_available(
                node,
                native.primary_gpu,
                native.devices.contains_key(&native.primary_gpu),
                primary_to_activate.is_some(),
            )
        });
        if !primary_available {
            return Err(DeviceError::MissingPrimary);
        }
        if let Some(primary_gpu) = primary_to_activate {
            self.activate_drm_device(handle, primary_gpu)?;
        }
        if let Err(error) = self.activate_drm_device(handle, node) {
            self.retire_inactive_renderers();
            return Err(error);
        }
        Ok(primary_to_activate)
    }

    fn retire_inactive_renderers(&mut self) {
        let Some(native) = self.native.as_mut() else {
            return;
        };
        let secondary_active = native
            .devices
            .iter()
            .any(|(node, device)| *node != native.primary_gpu && !device.surfaces.is_empty());
        let retired = native
            .devices
            .iter()
            .filter(|(node, device)| {
                renderer_retained_reason(
                    **node == native.primary_gpu,
                    device.surfaces.len(),
                    secondary_active,
                    !native.pending_primary_dependents.is_empty(),
                )
                .is_none()
            })
            .map(|(node, _)| *node)
            .collect::<Vec<_>>();
        for node in retired {
            let Some(device) = native.devices.remove(&node) else {
                continue;
            };
            release_device_data(native, &self.event_loop_handle, node, device);
            tracing::info!(%node, "inactive DRM renderer retired");
        }
        let diagnostics = native.renderer_lifecycle_diagnostics();
        tracing::debug!(?diagnostics, "DRM renderer lifecycle state");
    }

    fn scan_connectors(&mut self, node: DrmNode) {
        if let Err(error) = self.scan_connectors_inner(node, None) {
            tracing::warn!(%node, %error, "failed to reconcile DRM connectors");
        }
        self.retire_inactive_renderers();
    }

    fn scan_connectors_for_enable(&mut self, node: DrmNode, name: &str) -> Result<(), String> {
        let result = self.scan_connectors_inner(node, Some(name));
        self.retire_inactive_renderers();
        result
    }

    fn scan_connectors_inner(
        &mut self,
        node: DrmNode,
        enable_name: Option<&str>,
    ) -> Result<(), String> {
        let (events, connected) = {
            let Some(device) = self
                .native
                .as_mut()
                .and_then(|native| native.devices.get_mut(&node))
            else {
                return Err("DRM renderer is not active".to_owned());
            };
            let events = device
                .scanner
                .scan_connectors(device.manager.device())
                .map_err(|error| error.to_string())?;
            let connected = device
                .scanner
                .connected_connectors()
                .map(|(connector, crtc)| (connector.clone(), crtc))
                .collect::<Vec<_>>();
            (events, connected)
        };
        let connected_names = connected
            .iter()
            .map(|(connector, _)| connector_name(connector))
            .collect::<HashSet<_>>();
        if let Some(native) = self.native.as_mut() {
            for disabled in native
                .disabled_outputs
                .values_mut()
                .filter(|disabled| disabled.node == node)
            {
                disabled.present = connected_names.contains(&disabled.output.name());
            }
        }
        for event in events {
            match event {
                DrmScanEvent::Connected {
                    connector,
                    crtc: Some(crtc),
                } => {
                    let name = connector_name(&connector);
                    let administratively_disabled = self
                        .native
                        .as_ref()
                        .is_some_and(|native| native.disabled_outputs.contains_key(&name));
                    if !administratively_disabled {
                        let handle = connector.handle();
                        if let Err(error) = self.connect_output(node, connector, crtc) {
                            if let Some(device) = self
                                .native
                                .as_mut()
                                .and_then(|native| native.devices.get_mut(&node))
                            {
                                device.scanner.retry_connector(handle);
                            }
                            tracing::warn!(%node, %error, "failed to connect DRM output");
                        }
                    }
                }
                DrmScanEvent::Disconnected {
                    connector,
                    crtc: Some(crtc),
                } => self.disconnect_output(node, connector, crtc),
                _ => {}
            }
        }
        if let Some(name) = enable_name {
            let (connector, crtc) = connected
                .into_iter()
                .find_map(|(connector, crtc)| {
                    (connector_name(&connector) == name).then_some((connector, crtc?))
                })
                .ok_or_else(|| format!("output {name} is not physically connected"))?;
            self.connect_output(node, connector, crtc)?;
            self.native
                .as_mut()
                .expect("native backend exists")
                .disabled_outputs
                .remove(name);
        }
        Ok(())
    }

    fn connect_output(
        &mut self,
        node: DrmNode,
        connector: connector::Info,
        crtc: crtc::Handle,
    ) -> Result<(), String> {
        let name = connector_name(&connector);
        if self.native.as_ref().is_some_and(|native| {
            native.devices.values().any(|device| {
                device
                    .surfaces
                    .values()
                    .any(|surface| surface.output.name() == name)
            })
        }) {
            return Err(format!("output {name} is already active"));
        }
        if !self.output_global_admission_available() {
            return Err(format!(
                "output {name} is waiting for retired Wayland globals to drain"
            ));
        }
        let Some(mode) = connector
            .modes()
            .iter()
            .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
            .copied()
            .or_else(|| connector.modes().first().copied())
        else {
            return Err(format!("output {name} has no modes"));
        };
        let wl_mode = Mode::from(mode);
        let (width, height) = connector.size().unwrap_or_default();
        let model = output_model(&name);
        let output = Output::new(
            name.clone(),
            PhysicalProperties {
                size: (width as i32, height as i32).into(),
                subpixel: connector.subpixel().into(),
                make: "Unknown".into(),
                model,
                serial_number: String::new(),
            },
        );
        output.set_preferred(wl_mode);
        output.change_current_state(
            Some(wl_mode),
            Some(Transform::Normal),
            None,
            Some((0, 0).into()),
        );
        let publish_global = self.output_global_identity_available(&name);
        let native = self.native.as_mut().expect("native backend should exist");
        let device = native
            .devices
            .get_mut(&node)
            .ok_or_else(|| format!("DRM renderer for {name} retired during connection"))?;
        let is_primary = node == native.primary_gpu;
        let is_evdi = device.is_evdi;
        let drm = match &mut device.manager {
            OutputManager::Gbm(manager) => {
                let mut renderer = native
                    .gpus
                    .single_renderer(&device.render_node)
                    .map_err(|error| format!("renderer for {name} is unavailable: {error}"))?;
                let empty = DrmOutputRenderElements::<
                    NativeRenderer<'_>,
                    NativeElement<
                        NativeRenderer<'_>,
                        WaylandSurfaceRenderElement<NativeRenderer<'_>>,
                    >,
                >::default();
                let drm = manager
                    .lock()
                    .initialize_output(
                        crtc,
                        mode,
                        &[connector.handle()],
                        &output,
                        None,
                        &mut renderer,
                        &empty,
                    )
                    .map_err(|error| format!("failed to initialize output {name}: {error}"))?;
                OutputDrm::Gbm(drm)
            }
            OutputManager::Evdi(manager) => OutputDrm::Evdi(Box::new(
                EvdiOutput::new(manager, crtc, mode, connector.handle())
                    .map_err(|error| format!("failed to initialize EVDI output {name}: {error}"))?,
            )),
        };

        let positions = native.layout.connect(
            name.clone(),
            wl_mode.size.w,
            wl_mode.size.h,
            u8::from(!is_evdi),
        );
        let position = positions
            .iter()
            .find(|position| position.name == name)
            .expect("connected output should be in layout");
        let location = (position.x, position.y).into();
        output.change_current_state(None, None, None, Some(location));
        let global =
            publish_global.then(|| output.create_global::<NickelSession>(&self.display_handle));
        self.space.map_output(&output, location);
        for position in &positions {
            let mapped = {
                self.space
                    .outputs()
                    .find(|mapped| mapped.name() == position.name)
                    .cloned()
            };
            if let Some(mapped) = mapped {
                let location = (position.x, position.y).into();
                mapped.change_current_state(None, None, None, Some(location));
                self.space.map_output(&mapped, location);
            }
        }
        if is_primary {
            self.primary_output_name = Some(name.clone());
        }
        output
            .user_data()
            .insert_if_missing(|| OutputId { device: node, crtc });

        device.surfaces.insert(
            crtc,
            SurfaceData {
                global,
                output: output.clone(),
                drm,
                background: SolidColorBuffer::new(
                    wl_mode.size.to_logical(1),
                    [0.055, 0.065, 0.085, 1.0],
                ),
                render_path_logged: false,
                invalidate_pending: is_evdi,
            },
        );
        self.restore_output_windows(&output);
        self.relayout_shell_surfaces();
        self.schedule_render(node, Duration::ZERO);
        tracing::info!(output = %name, "DRM output connected");
        Ok(())
    }

    fn disconnect_output(&mut self, node: DrmNode, connector: connector::Info, crtc: crtc::Handle) {
        let name = format!(
            "{}-{}",
            connector.interface().as_str(),
            connector.interface_id()
        );
        let Some(native) = self.native.as_mut() else {
            return;
        };
        if let Some(disabled) = native.disabled_outputs.get_mut(&name) {
            disabled.present = false;
        }
        let surface = native
            .devices
            .get_mut(&node)
            .and_then(|device| device.surfaces.remove(&crtc));
        let positions = native.layout.disconnect(&name);
        if let Some(mut surface) = surface {
            self.stage_output_removal(&surface.output);
            self.space.unmap_output(&surface.output);
            surface.output.leave_all();
            if let Some(global) = surface.global.take() {
                self.defer_output_global_retirement(name.clone(), global);
            }
        }
        self.reconcile_output_removal(&name);
        for position in positions {
            let output = self
                .space
                .outputs()
                .find(|output| output.name() == position.name)
                .cloned();
            if let Some(output) = output {
                let location = (position.x, position.y).into();
                output.change_current_state(None, None, None, Some(location));
                self.space.map_output(&output, location);
            }
        }
        self.reflow_windows_to_connected_outputs();
        self.relayout_maximized_windows();
        self.relayout_fullscreen_windows();
        self.relayout_shell_surfaces();
        tracing::info!(output = %name, "DRM output disconnected");
    }

    pub(crate) fn native_output_inventory(&self) -> Vec<(String, Size<i32, Logical>)> {
        let Some(native) = self.native.as_ref() else {
            return Vec::new();
        };
        let mut outputs = self
            .space
            .outputs()
            .filter_map(|output| {
                self.space
                    .output_geometry(output)
                    .map(|geometry| (output.name(), geometry.size))
            })
            .collect::<Vec<_>>();
        outputs.extend(native.disabled_outputs.values().filter_map(|disabled| {
            if !disabled.present {
                return None;
            }
            disabled
                .output
                .current_mode()
                .map(|mode| (disabled.output.name(), mode.size.to_logical(1)))
        }));
        outputs
    }

    pub(crate) fn set_native_output_enabled(
        &mut self,
        name: &str,
        enabled: bool,
    ) -> Result<(), &'static str> {
        if enabled {
            if self.space.outputs().any(|output| output.name() == name) {
                return Ok(());
            }
            let node = self
                .native
                .as_ref()
                .and_then(|native| native.disabled_outputs.get(name))
                .map(|disabled| disabled.node);
            let primary_to_scan = if let Some(node) = node {
                let handle = self.event_loop_handle.clone();
                match self.activate_drm_device_with_dependencies(&handle, node) {
                    Ok(primary_gpu) => primary_gpu,
                    Err(error) => {
                        tracing::error!(%node, ?error, "failed to reactivate disabled DRM output");
                        return Err("failed to reactivate output renderer");
                    }
                }
            } else {
                return Err("unknown output");
            };
            let node = node.expect("known disabled output has a DRM node");
            if let Err(error) = self.scan_connectors_for_enable(node, name) {
                tracing::error!(%node, output = %name, %error, "failed to enable DRM output");
                return Err("failed to enable output");
            }
            if let Some(primary_gpu) = primary_to_scan {
                self.scan_connectors(primary_gpu);
            }
            return Ok(());
        }
        let active_count = self.space.outputs().count();
        if active_count <= 1 {
            return Err("cannot disable the last active output");
        }
        let target = self.native.as_ref().and_then(|native| {
            native.devices.iter().find_map(|(node, device)| {
                device
                    .surfaces
                    .iter()
                    .find(|(_, surface)| surface.output.name() == name)
                    .map(|(crtc, _)| (*node, *crtc))
            })
        });
        let Some((node, crtc)) = target else {
            return self
                .native
                .as_ref()
                .is_some_and(|native| native.disabled_outputs.contains_key(name))
                .then_some(())
                .ok_or("unknown output");
        };
        let surface = self
            .native
            .as_mut()
            .and_then(|native| native.devices.get_mut(&node))
            .and_then(|device| device.surfaces.remove(&crtc))
            .ok_or("output disappeared while disabling")?;
        self.stage_output_removal(&surface.output);
        self.space.unmap_output(&surface.output);
        surface.output.leave_all();
        if let Some(global) = surface.global {
            self.defer_output_global_retirement(name.to_owned(), global);
        }
        let output = surface.output;
        drop(surface.drm);
        let native = self.native.as_mut().expect("native backend exists");
        native.layout.disconnect(name);
        native.disabled_outputs.insert(
            name.to_owned(),
            DisabledOutput {
                node,
                output,
                present: true,
            },
        );
        self.reconcile_output_removal(name);
        self.reflow_windows_to_connected_outputs();
        self.relayout_maximized_windows();
        self.relayout_fullscreen_windows();
        self.relayout_shell_surfaces();
        self.retire_inactive_renderers();
        tracing::info!(output = %name, "DRM output disabled by user");
        Ok(())
    }

    fn remove_drm_device(&mut self, node: DrmNode) {
        if let Some(native) = self.native.as_mut() {
            consume_pending_dependent(
                &mut native.pending_primary_dependents,
                node,
                native.primary_gpu,
            );
        }
        let dependent_nodes = self
            .native
            .as_ref()
            .map(|native| {
                dependent_renderers_after_primary_removal(
                    node,
                    native.primary_gpu,
                    native.devices.keys().copied(),
                )
            })
            .unwrap_or_default();
        if !dependent_nodes.is_empty()
            && let Some(native) = self.native.as_mut()
        {
            native
                .pending_primary_dependents
                .extend(dependent_nodes.iter().copied());
        }
        for dependent in dependent_nodes {
            tracing::warn!(
                primary = %node,
                dependent = %dependent,
                "retiring DRM outputs whose render primary was removed"
            );
            self.retire_live_drm_device(dependent, false);
        }
        self.retire_live_drm_device(node, true);
    }

    fn retire_live_drm_device(&mut self, node: DrmNode, forget_discovery: bool) {
        let Some(native) = self.native.as_mut() else {
            return;
        };
        if forget_discovery {
            native.discovered_devices.remove(&node);
            native
                .disabled_outputs
                .retain(|_, disabled| disabled.node != node);
        }
        let Some(mut device) = native.devices.remove(&node) else {
            self.retire_inactive_renderers();
            return;
        };
        let mut positions = Vec::new();
        let mut removed_names = Vec::new();
        let mut removed_surfaces = device
            .surfaces
            .drain()
            .map(|(_, surface)| surface)
            .collect::<Vec<_>>();
        for surface in &removed_surfaces {
            let name = surface.output.name();
            positions = native.layout.disconnect(&name);
            removed_names.push(name);
        }
        release_device_data(native, &self.event_loop_handle, node, device);
        for mut surface in removed_surfaces.drain(..) {
            let name = surface.output.name();
            self.stage_output_removal(&surface.output);
            self.space.unmap_output(&surface.output);
            surface.output.leave_all();
            if let Some(global) = surface.global.take() {
                self.defer_output_global_retirement(name.clone(), global);
            }
        }
        for name in removed_names {
            self.reconcile_output_removal(&name);
        }
        for position in positions {
            let output = self
                .space
                .outputs()
                .find(|output| output.name() == position.name)
                .cloned();
            if let Some(output) = output {
                let location = (position.x, position.y).into();
                output.change_current_state(None, None, None, Some(location));
                self.space.map_output(&output, location);
            }
        }
        self.reflow_windows_to_connected_outputs();
        self.relayout_maximized_windows();
        self.relayout_fullscreen_windows();
        self.relayout_shell_surfaces();
        self.retire_inactive_renderers();
        tracing::info!(%node, forget_discovery, "DRM device resources retired");
    }

    fn schedule_render(&mut self, node: DrmNode, delay: Duration) {
        let Some(device) = self
            .native
            .as_mut()
            .and_then(|native| native.devices.get_mut(&node))
        else {
            return;
        };
        if device.render_scheduled {
            return;
        }
        let delay = paced_render_delay(
            device.is_evdi,
            delay,
            device.last_render_started.map(|started| started.elapsed()),
        );
        device.render_scheduled = true;
        let generation = device.generation;
        let timer = Timer::from_duration(delay);
        let _ = self
            .event_loop_handle
            .insert_source(timer, move |_, _, data| {
                let current_generation = data
                    .native
                    .as_ref()
                    .and_then(|native| native.renderer_lifecycle.generation(node));
                if current_generation != Some(generation) {
                    return TimeoutAction::Drop;
                }
                if let Some(device) = data
                    .native
                    .as_mut()
                    .and_then(|native| native.devices.get_mut(&node))
                {
                    device.render_scheduled = false;
                    device.last_render_started = Some(Instant::now());
                }
                data.render_node(node);
                TimeoutAction::Drop
            });
    }

    pub(crate) fn invalidate_native_outputs(&mut self) {
        let nodes = self
            .native
            .as_mut()
            .map(|native| {
                for surface in native
                    .devices
                    .values_mut()
                    .flat_map(|device| device.surfaces.values_mut())
                {
                    surface.invalidate_pending = true;
                }
                native.devices.keys().copied().collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for node in nodes {
            self.schedule_render(node, Duration::ZERO);
        }
    }

    fn reflow_windows_to_connected_outputs(&mut self) {
        let output_geometries: Vec<_> = self
            .space
            .outputs()
            .filter_map(|output| self.space.output_geometry(output))
            .collect();
        let Some(fallback) = output_geometries.first().copied() else {
            return;
        };
        let stranded: Vec<_> = self
            .space
            .elements()
            .filter(|window| !self.is_shell_owned_window(window))
            .filter(|window| {
                self.space.element_bbox(window).is_some_and(|bounds| {
                    !output_geometries
                        .iter()
                        .any(|output| output.overlaps(bounds))
                })
            })
            .cloned()
            .collect();
        for window in stranded {
            self.space.map_element(window, fallback.loc, false);
        }
    }

    fn render_node(&mut self, node: DrmNode) {
        let wave = self.begin_preview_render_wave();
        self.render_node_in_wave(node, wave);
    }

    fn render_node_in_wave(&mut self, node: DrmNode, wave: u64) {
        let crtcs: Vec<_> = self
            .native
            .as_ref()
            .and_then(|native| native.devices.get(&node))
            .map(|device| device.surfaces.keys().copied().collect())
            .unwrap_or_default();
        for crtc in crtcs {
            self.render_output(node, crtc, wave);
        }
    }

    fn render_output(&mut self, node: DrmNode, crtc: crtc::Handle, wave: u64) {
        let shell_bootstrapping = self.launcher_window.is_none();
        let mut identified_outputs = self.space.outputs().cloned().collect::<Vec<_>>();
        identified_outputs.sort_by_key(|output| {
            self.space
                .output_geometry(output)
                .map(|geometry| (geometry.loc.x, geometry.loc.y))
                .unwrap_or_default()
        });
        let identify_output_count = identified_outputs.len();
        let identify_index = self
            .identify_outputs_until
            .filter(|deadline| *deadline > std::time::Instant::now())
            .and_then(|_| {
                let outputs = &identified_outputs;
                outputs.iter().position(|output| {
                    self.native
                        .as_ref()
                        .and_then(|native| native.devices.get(&node))
                        .and_then(|device| device.surfaces.get(&crtc))
                        .is_some_and(|surface| surface.output == *output)
                })
            });
        let Some(mut native) = self.native.take() else {
            return;
        };
        let rendered = (|| {
            native.reconcile_identify_badges(identify_output_count);
            if identify_index.is_none() && !native.identify_badges.entries.is_empty() {
                native.retire_identify_badges();
                tracing::trace!(
                    diagnostics = ?native.identify_badges.diagnostics(),
                    "output-identification rasters retired"
                );
            }
            if !native.activity.is_active() {
                return None;
            }
            let device = native.devices.get_mut(&node)?;
            let surface = device.surfaces.get_mut(&crtc)?;
            let output = surface.output.clone();
            let cursor = native
                .cursors
                .get(&self.frame_cursor)
                .or_else(|| native.cursors.get(&crate::window_frame::FrameCursor::Arrow))
                .cloned()
                .unwrap_or_else(fallback_arrow_cursor);
            let frame_icons = native.frame_icons.clone();
            let background = surface.background.clone();
            let identify_badge = identify_index.map(|index| native.identify_badges.get(index));
            let preview_windows = if self.locked {
                Vec::new()
            } else {
                self.preview_capture_candidates(wave)
            };
            if surface.invalidate_pending {
                surface.drm.reset_buffer_ages();
                surface.invalidate_pending = false;
            }
            if !surface.render_path_logged {
                let cpu_copyout = matches!(&surface.drm, OutputDrm::Evdi(_));
                let copyout_bytes = output.current_mode().map_or(0, |mode| {
                    (mode.size.w as usize)
                        .saturating_mul(mode.size.h as usize)
                        .saturating_mul(4)
                });
                let retained_copyout_bytes = match &surface.drm {
                    OutputDrm::Evdi(output) => output.retained_bytes(),
                    OutputDrm::Gbm(_) => 0,
                };
                tracing::info!(
                    output = %output.name(),
                    render_gpu = %native.primary_gpu,
                    target_gpu = %device.render_node,
                    format = ?surface.drm.format(),
                    cross_gpu = native.primary_gpu != device.render_node,
                    cpu_copyout,
                    copyout_bytes,
                    dumb_scanout_buffers = if cpu_copyout { 2 } else { 0 },
                    retained_copyout_bytes,
                    "DRM output render path selected"
                );
                surface.render_path_logged = true;
            }
            let target_gpu = device.render_node;
            let bootstrapping =
                shell_bootstrapping && Instant::now() < native.bootstrap_render_until;
            // Keep application and video surfaces in one composited scene.
            // Assigning video to an overlay plane can leak stale plane content
            // through windows above it when occlusion changes. A hardware
            // cursor remains safe because it is always the topmost element.
            let frame_flags = FrameFlags::ALLOW_CURSOR_PLANE_SCANOUT;
            let renderer = if native.primary_gpu == target_gpu {
                native.gpus.single_renderer(&target_gpu)
            } else {
                native
                    .gpus
                    .renderer(&native.primary_gpu, &target_gpu, surface.drm.format())
            };
            let mut renderer = match renderer {
                Ok(renderer) => renderer,
                Err(error) => {
                    for (id, _) in &preview_windows {
                        self.preview_renderer_failed(*id);
                    }
                    tracing::error!(
                        ?error,
                        render = %native.primary_gpu,
                        target = %target_gpu,
                        "failed to acquire multi-GPU renderer"
                    );
                    return Some((output, true));
                }
            };
            let mut preview_retry = false;
            for (id, window) in preview_windows {
                let (rgba, had_frame) = self.take_preview_capture_buffer(id);
                let mut rgba = rgba;
                if capture_preview(&mut renderer, &window, &mut rgba) {
                    self.store_preview(
                        id,
                        PreviewFrame {
                            width: crate::state::PREVIEW_WIDTH as u16,
                            height: crate::state::PREVIEW_HEIGHT as u16,
                            rgba,
                        },
                    );
                } else {
                    self.preview_capture_failed(id, rgba, had_frame);
                    preview_retry = true;
                }
            }
            let mut elements: Vec<
                NativeElement<NativeRenderer<'_>, WaylandSurfaceRenderElement<NativeRenderer<'_>>>,
            > = Vec::new();
            let shell_surfaces = self
                .shell_windows()
                .filter_map(|window| window.toplevel().map(|surface| surface.wl_surface().id()))
                .collect::<Vec<_>>();
            let desktop_surfaces = self
                .desktop_windows
                .iter()
                .filter_map(|window| window.toplevel().map(|surface| surface.wl_surface().id()))
                .collect::<Vec<_>>();
            let frame_palette = ThemePalette::from_appearance(
                ShellSettings::load_default().resolve_appearance(Appearance::default()),
            );
            crate::window_frame::retain_titlebars_for_windows(
                self.surface_windows.values().map(|id| id.0),
            );
            if let Some(output_geometry) = self.space.output_geometry(&output) {
                // Space stores windows back-to-front. Build each window and its
                // frame together, front-to-back, so overlapping frames obey the
                // same stacking order as their client surfaces.
                for window in self.space.elements().rev() {
                    if self.locked && !self.lock_windows.contains(window) {
                        continue;
                    }
                    let Some(bounds) = self.space.element_bbox(window) else {
                        continue;
                    };
                    if !output_geometry.overlaps(bounds) {
                        continue;
                    }
                    let Some(location) = self.space.element_location(window) else {
                        continue;
                    };
                    let render_location = location - window.geometry().loc - output_geometry.loc;
                    let window_elements = window
                        .render_elements::<WaylandSurfaceRenderElement<NativeRenderer<'_>>>(
                            &mut renderer,
                            render_location.to_physical_precise_round(Scale::from(1.0)),
                            Scale::from(1.0),
                            1.0,
                        );
                    let has_content = !window_elements.is_empty();
                    elements.extend(
                        window_elements
                            .into_iter()
                            .map(|element| NativeElement::from(NativeCustomElement::from(element))),
                    );

                    let Some(surface) = window.wl_surface() else {
                        continue;
                    };
                    if !has_content
                        || shell_surfaces.contains(&surface.id())
                        || self.is_fullscreen_window(window)
                        || !self.is_server_decorated(window)
                    {
                        continue;
                    }
                    let registry_id = self.surface_windows.get(&surface.id()).copied();
                    let active = registry_id.is_some_and(|id| self.windows.is_active(id));
                    let title = registry_id
                        .and_then(|id| self.windows.title(id))
                        .unwrap_or_default();
                    let maximized = self.is_maximized_window(window);
                    let frame_index = elements.len();
                    // element_bbox includes popups. A transient popup must not stretch the
                    // owning window's server-side frame beyond its content geometry.
                    let Some(frame_bounds) = self.space.element_geometry(window) else {
                        continue;
                    };
                    let foreground = if active {
                        frame_palette.text
                    } else {
                        frame_palette.muted
                    };
                    let titlebar_geometry =
                        crate::window_frame::titlebar_geometry(crate::shell_layout::Geometry {
                            x: frame_bounds.loc.x,
                            y: frame_bounds.loc.y,
                            width: frame_bounds.size.w,
                            height: frame_bounds.size.h,
                        });
                    if let Some(titlebar) = crate::window_frame::render_titlebar_for(
                        registry_id.map(|id| id.0),
                        titlebar_geometry.width,
                        title,
                        frame_palette.panel,
                        foreground,
                    ) && let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                        &mut renderer,
                        (
                            f64::from(titlebar_geometry.x - output_geometry.loc.x),
                            f64::from(titlebar_geometry.y - output_geometry.loc.y),
                        ),
                        &titlebar,
                        None,
                        None,
                        Some((titlebar_geometry.width, titlebar_geometry.height).into()),
                        Kind::Unspecified,
                    ) {
                        elements.push(NativeCustomElement::from(element).into());
                    }
                    let frame_height = frame_bounds.size.h + crate::window_frame::TITLEBAR_HEIGHT;
                    for shadow in
                        crate::window_frame::shadow_layers(frame_bounds.size.w, frame_height)
                    {
                        elements.push(
                            NativeCustomElement::from(SolidColorRenderElement::from_buffer(
                                &shadow.buffer,
                                (
                                    frame_bounds.loc.x - output_geometry.loc.x + shadow.offset.0,
                                    frame_bounds.loc.y
                                        - output_geometry.loc.y
                                        - crate::window_frame::TITLEBAR_HEIGHT
                                        + shadow.offset.1,
                                ),
                                1.0,
                                1.0,
                                Kind::Unspecified,
                            ))
                            .into(),
                        );
                    }
                    if let Some(icons) = &frame_icons {
                        let icon_y = frame_bounds.loc.y
                            - output_geometry.loc.y
                            - crate::window_frame::TITLEBAR_HEIGHT
                            + 8;
                        let icon_x =
                            frame_bounds.loc.x - output_geometry.loc.x + frame_bounds.size.w;
                        for (buffer, offset) in [
                            (&icons.close, 35),
                            (
                                if maximized {
                                    &icons.restore
                                } else {
                                    &icons.maximize
                                },
                                81,
                            ),
                            (&icons.minimize, 127),
                        ] {
                            if let Ok(icon) = MemoryRenderBufferRenderElement::from_buffer(
                                &mut renderer,
                                ((icon_x - offset) as f64, icon_y as f64),
                                buffer,
                                None,
                                None,
                                None,
                                Kind::Unspecified,
                            ) {
                                elements
                                    .insert(frame_index, NativeCustomElement::from(icon).into());
                            }
                        }
                    }
                }
            }
            if !self.locked
                && let Some(highlighted) = self.preview_highlight.and_then(|highlight| {
                    self.space.elements().find(|window| {
                        window
                            .wl_surface()
                            .and_then(|surface| self.surface_windows.get(&surface.id()))
                            .copied()
                            == Some(highlight)
                    })
                })
                && let Some(output_geometry) = self.space.output_geometry(&output)
            {
                // Peek is a single front-to-back composition: shell overlays
                // remain legible, the selected client stays bright, the dim
                // layer covers everything else, and the ordinary scene remains
                // behind it. Multiple render passes are not equivalent on DRM
                // because plane assignment and damage history can expose stale
                // content between passes.
                let mut peek_elements = Vec::new();
                for shell in self.space.elements().rev().filter(|window| {
                    window.toplevel().is_some_and(|surface| {
                        shell_surfaces.contains(&surface.wl_surface().id())
                            && !desktop_surfaces.contains(&surface.wl_surface().id())
                    })
                }) {
                    let Some(location) = self.space.element_location(shell) else {
                        continue;
                    };
                    let render_location = location - shell.geometry().loc - output_geometry.loc;
                    peek_elements.extend(
                        shell
                            .render_elements::<WaylandSurfaceRenderElement<NativeRenderer<'_>>>(
                                &mut renderer,
                                render_location.to_physical_precise_round(Scale::from(1.0)),
                                Scale::from(1.0),
                                1.0,
                            )
                            .into_iter()
                            .map(|element| NativeElement::from(NativeCustomElement::from(element))),
                    );
                }
                if let Some(location) = self.space.element_location(highlighted) {
                    let render_location =
                        location - highlighted.geometry().loc - output_geometry.loc;
                    peek_elements.extend(
                        highlighted
                            .render_elements::<WaylandSurfaceRenderElement<NativeRenderer<'_>>>(
                                &mut renderer,
                                render_location.to_physical_precise_round(Scale::from(1.0)),
                                Scale::from(1.0),
                                1.0,
                            )
                            .into_iter()
                            .map(|element| NativeElement::from(NativeCustomElement::from(element))),
                    );
                }
                let dim = SolidColorBuffer::new(output_geometry.size, [0.0, 0.0, 0.0, 0.62]);
                peek_elements.push(
                    NativeCustomElement::from(SolidColorRenderElement::from_buffer(
                        &dim,
                        (0, 0),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    ))
                    .into(),
                );
                peek_elements.append(&mut elements);
                elements = peek_elements;
            }
            if !self.locked && self.shell_recovery_visible() {
                let recovery_size = output
                    .current_mode()
                    .map(|mode| mode.size)
                    .unwrap_or_else(|| (1, 1).into());
                let panel_geometry =
                    crate::recovery_ui::RecoveryUi::panel_geometry(crate::shell_layout::Geometry {
                        x: 0,
                        y: 0,
                        width: recovery_size.w,
                        height: recovery_size.h,
                    });
                let panel = self.recovery_ui.render_buffer();
                if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                    &mut renderer,
                    (f64::from(panel_geometry.x), f64::from(panel_geometry.y)),
                    &panel,
                    None,
                    None,
                    Some((panel_geometry.width, panel_geometry.height).into()),
                    Kind::Unspecified,
                ) {
                    elements.insert(0, NativeCustomElement::from(element).into());
                }
            } else {
                self.recovery_ui.release_raster();
            }
            if self.dimmed && !self.locked {
                let size = self
                    .space
                    .output_geometry(&output)
                    .map(|geometry| geometry.size)
                    .unwrap_or_else(|| (1, 1).into());
                let dim = SolidColorBuffer::new(size, [0.0, 0.0, 0.0, 0.48]);
                elements.insert(
                    0,
                    NativeCustomElement::from(SolidColorRenderElement::from_buffer(
                        &dim,
                        (0, 0),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    ))
                    .into(),
                );
            }
            elements.push(
                NativeCustomElement::from(SolidColorRenderElement::from_buffer(
                    &background,
                    (0, 0),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                ))
                .into(),
            );
            let is_primary = self
                .primary_output_name
                .as_deref()
                .is_none_or(|name| name == output.name());
            let mode_size = output.current_mode().map(|mode| mode.size);
            let switcher = (!self.locked && is_primary)
                .then_some(mode_size)
                .flatten()
                .and_then(|mode_size| {
                    let key = TaskSwitcherBufferKey {
                        candidates: self.task_switcher.candidates().to_vec(),
                        selected: self.task_switcher.selected_index(),
                        output_size: (mode_size.w, mode_size.h),
                        preview_generation: self.preview_generation(),
                    };
                    if key.candidates.len() < 2 {
                        native.task_switcher_cache = None;
                        return None;
                    }
                    let stale = native
                        .task_switcher_cache
                        .as_ref()
                        .is_none_or(|cached| cached.key != key);
                    if stale {
                        native.task_switcher_cache = task_switcher_buffer(self, mode_size)
                            .map(|(buffer, size)| TaskSwitcherBufferCache { key, buffer, size });
                    }
                    native
                        .task_switcher_cache
                        .as_ref()
                        .map(|cached| (cached.buffer.clone(), cached.size))
                });
            if let Some((switcher, switcher_size)) = switcher {
                let mode_size = mode_size.expect("switcher requires a current output mode");
                let location = (
                    (mode_size.w - switcher_size.w).max(0) / 2,
                    (mode_size.h - switcher_size.h).max(0) / 2,
                );
                match MemoryRenderBufferRenderElement::from_buffer(
                    &mut renderer,
                    (f64::from(location.0), f64::from(location.1)),
                    &switcher,
                    None,
                    None,
                    None,
                    Kind::Unspecified,
                ) {
                    Ok(element) => elements.insert(0, NativeCustomElement::from(element).into()),
                    Err(error) => tracing::warn!(?error, "failed to upload task switcher"),
                }
            }
            if let Some(badge) = identify_badge
                && let Some(mode) = output.current_mode()
            {
                let location = (
                    (mode.size.w - 180).max(0) / 2,
                    (mode.size.h - 180).max(0) / 2,
                );
                match MemoryRenderBufferRenderElement::from_buffer(
                    &mut renderer,
                    (f64::from(location.0), f64::from(location.1)),
                    &badge,
                    None,
                    None,
                    None,
                    Kind::Unspecified,
                ) {
                    Ok(element) => elements.insert(0, NativeCustomElement::from(element).into()),
                    Err(error) => tracing::warn!(?error, "failed to upload identify badge"),
                }
            }
            if !self.locked
                && let Some(icon) = self.dnd_icon.as_ref()
                && let Some(geometry) = self.space.output_geometry(&output)
            {
                let pointer = self.seat.get_pointer().unwrap().current_location();
                if let Some(location) = crate::state::drag_icon_location(pointer, geometry) {
                    let location = location.to_physical(1);
                    let icon_elements = render_elements_from_surface_tree::<
                        _,
                        WaylandSurfaceRenderElement<NativeRenderer<'_>>,
                    >(
                        &mut renderer,
                        icon,
                        location,
                        Scale::from(1.0),
                        1.0,
                        Kind::Cursor,
                    )
                    .into_iter()
                    .map(|element| NativeElement::from(NativeCustomElement::from(element)));
                    elements.splice(0..0, icon_elements);
                }
            }
            if let Some(geometry) = self.space.output_geometry(&output) {
                let pointer = self.seat.get_pointer().unwrap().current_location();
                if geometry.to_f64().contains(pointer) {
                    let location = (pointer - geometry.loc.to_f64())
                        .to_i32_round()
                        .to_physical(1)
                        - cursor.hotspot;
                    match MemoryRenderBufferRenderElement::from_buffer(
                        &mut renderer,
                        location.to_f64(),
                        &cursor.buffer,
                        None,
                        None,
                        None,
                        Kind::Cursor,
                    ) {
                        Ok(cursor) => elements.insert(0, NativeCustomElement::from(cursor).into()),
                        Err(error) => tracing::warn!(?error, "failed to upload cursor"),
                    }
                }
            }
            let capture_this_output = self
                .output_capture_name
                .as_deref()
                .map_or(is_primary, |name| name == output.name());
            let capture_path = if capture_this_output {
                self.output_capture_path.take()
            } else {
                None
            };
            if capture_path.is_some() {
                self.output_capture_name = None;
            }
            let portal_requested = self.has_pending_image_copy_frames(&output);
            if portal_requested || capture_path.is_some() {
                let capture_result = output
                    .current_mode()
                    .ok_or_else(|| "output has no active mode".to_owned())
                    .and_then(|mode| {
                        capture_composited_mapped(
                            &mut renderer,
                            &elements,
                            mode.size,
                            |mapped, flipped| {
                                if portal_requested {
                                    self.complete_image_copy_frames(
                                        &output,
                                        mapped,
                                        mode.size.w as usize,
                                        mode.size.h as usize,
                                        flipped,
                                    );
                                }
                                if let Some(path) = capture_path.as_ref() {
                                    save_mapped_capture(mapped, mode.size, flipped, path)?;
                                }
                                Ok(())
                            },
                        )
                    });
                if let Err(error) = &capture_result {
                    tracing::warn!(%error, output = %output.name(), "failed to capture composited frame");
                    if portal_requested {
                        self.fail_image_copy_frames(
                            &output,
                            smithay::wayland::image_copy_capture::CaptureFailureReason::Unknown,
                        );
                    }
                }
                if let Some(path) = capture_path {
                    let response = match capture_result {
                        Ok(()) => nickel_session_protocol::CaptureResult::Saved {
                            backend: nickel_session_protocol::CaptureBackend::Native,
                        },
                        Err(message) => nickel_session_protocol::CaptureResult::Failed { message },
                    };
                    self.complete_output_capture(&path, response);
                }
            }
            let retry = match &mut surface.drm {
                OutputDrm::Gbm(drm) => match drm.render_frame(
                    &mut renderer,
                    &elements,
                    [0.1, 0.1, 0.1, 1.0],
                    frame_flags,
                ) {
                    Ok(frame) if !frame.is_empty => {
                        let synchronized = if frame.needs_sync()
                            && let PrimaryPlaneElement::Swapchain(element) = frame.primary_element
                        {
                            element.sync.wait().map_err(|error| {
                                tracing::warn!(
                                    output = %output.name(),
                                    render_gpu = %native.primary_gpu,
                                    target_gpu = %target_gpu,
                                    ?error,
                                    "failed to synchronize rendered DRM frame"
                                );
                            })
                        } else {
                            Ok(())
                        };
                        if synchronized.is_err() {
                            true
                        } else if let Err(error) = drm.queue_frame(()) {
                            tracing::warn!(
                                output = %output.name(),
                                render_gpu = %native.primary_gpu,
                                target_gpu = %target_gpu,
                                ?error,
                                "failed to queue DRM frame"
                            );
                            true
                        } else {
                            false
                        }
                    }
                    Ok(_) => {
                        tracing::trace!(output = %output.name(), "DRM frame contained no damage");
                        bootstrapping
                    }
                    Err(error) => {
                        tracing::warn!(
                            output = %output.name(),
                            render_gpu = %native.primary_gpu,
                            target_gpu = %target_gpu,
                            ?error,
                            "failed to render DRM frame"
                        );
                        true
                    }
                },
                OutputDrm::Evdi(drm) => {
                    let presented = drm.render_and_present(&mut renderer, &elements);
                    if let Err(error) = presented {
                        // Damage tracking advances when primary-GPU rendering
                        // succeeds. Roll it back after any later readback or
                        // submission failure so the bounded retry redraws a
                        // complete frame while the last scanout stays visible.
                        drm.invalidate();
                        tracing::warn!(%error, output = %output.name(), "failed to copy EVDI frame from primary GPU");
                        true
                    } else {
                        match presented.expect("checked successful EVDI presentation") {
                            true => false,
                            false => bootstrapping,
                        }
                    }
                }
            };
            Some((output, retry || preview_retry))
        })();
        self.native = Some(native);
        self.schedule_preview_retry();
        let Some((output, retry)) = rendered else {
            return;
        };
        if retry {
            self.schedule_render(node, Duration::from_millis(16));
        }
        self.space.elements().for_each(|window| {
            window.send_frame(
                &output,
                self.start_time.elapsed(),
                Some(Duration::ZERO),
                |_, _| Some(output.clone()),
            );
        });
        if let Some(icon) = self.dnd_icon.as_ref()
            && self.space.output_geometry(&output).is_some_and(|geometry| {
                geometry
                    .to_f64()
                    .contains(self.seat.get_pointer().unwrap().current_location())
            })
        {
            smithay::desktop::utils::send_frames_surface_tree(
                icon,
                &output,
                self.start_time.elapsed(),
                Some(Duration::ZERO),
                |_, _| Some(output.clone()),
            );
        }
        self.space.refresh();
        self.popups.cleanup();
        let _ = self.display_handle.flush_clients();
    }

    fn frame_submitted(&mut self, node: DrmNode, crtc: crtc::Handle) {
        let Some(surface) = self
            .native
            .as_mut()
            .and_then(|native| native.devices.get_mut(&node))
            .and_then(|device| device.surfaces.get_mut(&crtc))
        else {
            return;
        };
        if let Err(error) = surface.drm.frame_submitted() {
            tracing::warn!(%node, ?crtc, ?error, "failed to complete DRM frame");
        }
        // Surface commits, input, output changes, and explicit capture requests schedule their own
        // renders. Scheduling unconditionally at every vblank turns a static desktop into a
        // permanent full-refresh loop, which is especially destructive on llvmpipe.
    }
}

fn paced_render_delay(
    is_evdi: bool,
    requested: Duration,
    since_last_render: Option<Duration>,
) -> Duration {
    if !is_evdi {
        return requested;
    }
    let pacing = since_last_render
        .map(|elapsed| EVDI_MIN_RENDER_INTERVAL.saturating_sub(elapsed))
        .unwrap_or(Duration::ZERO);
    requested.max(pacing)
}

fn is_evdi_device(path: &Path) -> bool {
    let Some(card) = path.file_name() else {
        return false;
    };
    std::fs::read_link(Path::new("/sys/class/drm").join(card).join("device/driver"))
        .ok()
        .and_then(|driver| driver.file_name().map(|name| name == "evdi"))
        .unwrap_or(false)
}

fn drm_driver_name(path: &Path, is_evdi: bool) -> String {
    if is_evdi {
        return "evdi".to_owned();
    }
    path.file_name()
        .and_then(|card| {
            std::fs::read_link(Path::new("/sys/class/drm").join(card).join("device/driver")).ok()
        })
        .and_then(|driver| {
            driver
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

fn has_connected_drm_connector(sysfs_drm: &Path, path: &Path) -> Option<bool> {
    let card = path.file_name()?.to_string_lossy();
    let connector_prefix = format!("{card}-");
    let entries = std::fs::read_dir(sysfs_drm).ok()?;
    let mut found_connector = false;
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(&connector_prefix)
        {
            continue;
        }
        found_connector = true;
        if std::fs::read_to_string(entry.path().join("status"))
            .ok()
            .is_some_and(|status| status.trim() == "connected")
        {
            return Some(true);
        }
    }
    found_connector.then_some(false)
}

#[cfg(test)]
mod evdi_tests {
    use super::has_connected_drm_connector;
    use std::{fs, path::Path};

    #[test]
    fn disconnected_virtual_cards_do_not_require_renderers_until_connected() {
        let root =
            std::env::temp_dir().join(format!("nickel-evdi-connectors-{}", std::process::id()));
        let connector = root.join("card4-DVI-I-4");
        fs::create_dir_all(&connector).unwrap();
        fs::write(connector.join("status"), "disconnected\n").unwrap();
        assert_eq!(
            has_connected_drm_connector(&root, Path::new("/dev/dri/card4")),
            Some(false)
        );
        fs::write(connector.join("status"), "connected\n").unwrap();
        assert_eq!(
            has_connected_drm_connector(&root, Path::new("/dev/dri/card4")),
            Some(true)
        );
        fs::remove_dir_all(root).unwrap();
    }
}

fn themed_arrow_cursor() -> CursorBuffer {
    themed_cursor(&["default", "left_ptr"]).unwrap_or_else(fallback_arrow_cursor)
}

fn themed_cursors() -> HashMap<crate::window_frame::FrameCursor, CursorBuffer> {
    use crate::window_frame::FrameCursor;

    let arrow = themed_arrow_cursor();
    let mut cursors = HashMap::from([(FrameCursor::Arrow, arrow.clone())]);
    for (kind, names) in [
        (FrameCursor::North, &["n-resize", "top_side"][..]),
        (
            FrameCursor::NorthEast,
            &["ne-resize", "top_right_corner"][..],
        ),
        (FrameCursor::East, &["e-resize", "right_side"][..]),
        (
            FrameCursor::SouthEast,
            &["se-resize", "bottom_right_corner"][..],
        ),
        (FrameCursor::South, &["s-resize", "bottom_side"][..]),
        (
            FrameCursor::SouthWest,
            &["sw-resize", "bottom_left_corner"][..],
        ),
        (FrameCursor::West, &["w-resize", "left_side"][..]),
        (
            FrameCursor::NorthWest,
            &["nw-resize", "top_left_corner"][..],
        ),
    ] {
        cursors.insert(kind, themed_cursor(names).unwrap_or_else(|| arrow.clone()));
    }
    cursors
}

fn themed_cursor(names: &[&str]) -> Option<CursorBuffer> {
    let kde_cursor_settings = kde_cursor_settings();
    let theme_name = std::env::var("XCURSOR_THEME")
        .ok()
        .filter(|theme| !theme.is_empty())
        .or_else(|| kde_cursor_settings.0.clone())
        .unwrap_or_else(|| "default".into());
    let requested_size = std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|size| size.parse::<u32>().ok())
        .filter(|size| *size > 0)
        .or(kde_cursor_settings.1)
        .unwrap_or(24);
    let theme = xcursor::CursorTheme::load(&theme_name);
    if let Some(path) = names.iter().find_map(|name| theme.load_icon(name))
        && let Ok(bytes) = std::fs::read(&path)
        && let Some(images) = xcursor::parser::parse_xcursor(&bytes)
        && let Some(image) = images
            .into_iter()
            .min_by_key(|image| image.size.abs_diff(requested_size))
    {
        tracing::info!(
            theme = %theme_name,
            size = image.size,
            path = %path.display(),
            cursor = ?names.first().copied().unwrap_or("default"),
            "loaded desktop cursor"
        );
        return Some(CursorBuffer {
            buffer: MemoryRenderBuffer::from_slice(
                &image.pixels_rgba,
                Fourcc::Abgr8888,
                (image.width as i32, image.height as i32),
                1,
                Transform::Normal,
                None,
            ),
            hotspot: Point::from((image.xhot as i32, image.yhot as i32)),
        });
    }
    tracing::warn!(theme = %theme_name, ?names, "could not load desktop cursor");
    None
}

fn kde_cursor_settings() -> (Option<String>, Option<u32>) {
    let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|home| home.join(".config"))
        })
    else {
        return (None, None);
    };
    let Ok(contents) = std::fs::read_to_string(config_home.join("kcminputrc")) else {
        return (None, None);
    };
    parse_kde_cursor_settings(&contents)
}

fn parse_kde_cursor_settings(contents: &str) -> (Option<String>, Option<u32>) {
    let mut in_mouse_group = false;
    let mut theme = None;
    let mut size = None;
    for line in contents.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            in_mouse_group = line == "[Mouse]";
            continue;
        }
        if !in_mouse_group {
            continue;
        }
        if let Some(value) = line.strip_prefix("cursorTheme=")
            && !value.is_empty()
        {
            theme = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("cursorSize=") {
            size = value.parse().ok().filter(|size| *size > 0);
        }
    }
    (theme, size)
}

fn fallback_arrow_cursor() -> CursorBuffer {
    const SCALE: usize = 2;
    const ROWS: &[&str] = &[
        "B...............",
        "BB..............",
        "BWB.............",
        "BWWB............",
        "BWWWB...........",
        "BWWWWB..........",
        "BWWWWWB.........",
        "BWWWWWWB........",
        "BWWWWWWWB.......",
        "BWWWWBBBBB......",
        "BWWB..B.........",
        "BWB...B.........",
        "BB....BB........",
        "B.....BB........",
        "......BB........",
        "......BB........",
        ".......BB.......",
        ".......BB.......",
        "................",
        "................",
        "................",
        "................",
        "................",
        "................",
    ];
    let width = ROWS[0].len() * SCALE;
    let height = ROWS.len() * SCALE;
    let mut rgba = vec![0_u8; width * height * 4];
    for (y, row) in ROWS.iter().enumerate() {
        for (x, pixel) in row.bytes().enumerate() {
            let color = match pixel {
                b'B' => [16, 18, 22, 255],
                b'W' => [245, 247, 252, 255],
                _ => [0, 0, 0, 0],
            };
            for offset_y in 0..SCALE {
                for offset_x in 0..SCALE {
                    let index = ((y * SCALE + offset_y) * width + x * SCALE + offset_x) * 4;
                    rgba[index..index + 4].copy_from_slice(&color);
                }
            }
        }
    }
    CursorBuffer {
        buffer: MemoryRenderBuffer::from_slice(
            &rgba,
            Fourcc::Abgr8888,
            (width as i32, height as i32),
            1,
            Transform::Normal,
            None,
        ),
        hotspot: Point::from((0, 0)),
    }
}

fn identify_badge(number: usize) -> MemoryRenderBuffer {
    const SIZE: usize = 180;
    const THICKNESS: usize = 18;
    let mut rgba = vec![0_u8; SIZE * SIZE * 4];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[38, 45, 59, 238]);
    }
    let segments = match number {
        1 => [false, true, true, false, false, false, false],
        2 => [true, true, false, true, true, false, true],
        3 => [true, true, true, true, false, false, true],
        4 => [false, true, true, false, false, true, true],
        5 => [true, false, true, true, false, true, true],
        6 => [true, false, true, true, true, true, true],
        7 => [true, true, true, false, false, false, false],
        8 => [true; 7],
        _ => [true, true, true, true, false, true, true],
    };
    let rectangles = [
        (55, 25, 70, THICKNESS),
        (120, 35, THICKNESS, 55),
        (120, 90, THICKNESS, 55),
        (55, 137, 70, THICKNESS),
        (42, 90, THICKNESS, 55),
        (42, 35, THICKNESS, 55),
        (55, 81, 70, THICKNESS),
    ];
    for ((x, y, width, height), enabled) in rectangles.into_iter().zip(segments) {
        if !enabled {
            continue;
        }
        for row in y..y + height {
            for column in x..x + width {
                let index = (row * SIZE + column) * 4;
                rgba[index..index + 4].copy_from_slice(&[245, 247, 252, 255]);
            }
        }
    }
    MemoryRenderBuffer::from_slice(
        &rgba,
        Fourcc::Abgr8888,
        (SIZE as i32, SIZE as i32),
        1,
        Transform::Normal,
        None,
    )
}

const IDENTIFY_BADGE_BYTES: usize = 180 * 180 * 4;

fn switcher_visible_range(count: usize, selected: usize) -> std::ops::Range<usize> {
    let visible = count.min(SWITCHER_MAX_CARDS);
    let start = selected
        .saturating_sub(visible / 2)
        .min(count.saturating_sub(visible));
    start..start + visible
}

fn task_switcher_buffer(
    state: &NickelSession,
    output_size: smithay::utils::Size<i32, Physical>,
) -> Option<(MemoryRenderBuffer, smithay::utils::Size<i32, Physical>)> {
    let candidates = state.task_switcher.candidates();
    let selected_index = state.task_switcher.selected_index();
    if candidates.len() < 2 {
        return None;
    }
    let range = switcher_visible_range(candidates.len(), selected_index);
    let count = range.len();
    let gap = 14_u32;
    let padding = 20_u32;
    let available_width = u32::try_from(output_size.w.saturating_sub(80))
        .unwrap_or_default()
        .min(1160);
    let card_width = ((available_width
        .saturating_sub(padding * 2 + gap * count.saturating_sub(1) as u32))
        / count as u32)
        .clamp(140, 220);
    let card_height = 180_u32;
    let width = padding * 2 + card_width * count as u32 + gap * count.saturating_sub(1) as u32;
    let height = card_height + padding * 2;
    let size = (width as i32, height as i32);
    let buffer = draw_memory_render_buffer(width, height, |pixels| {
        let mut image =
            image::ImageBuffer::<image::Rgba<u8>, &mut [u8]>::from_raw(width, height, pixels)
                .expect("memory render buffer has the requested RGBA dimensions");
        image.pixels_mut().for_each(|pixel| {
            *pixel = image::Rgba([17, 24, 39, 244]);
        });

        for (slot, index) in range.enumerate() {
            let x = padding + slot as u32 * (card_width + gap);
            let selected = index == selected_index;
            let border = if selected {
                image::Rgba([101, 184, 255, 255])
            } else {
                image::Rgba([66, 81, 108, 255])
            };
            fill_rgba_rect(&mut image, x, padding, card_width, card_height, border);
            fill_rgba_rect(
                &mut image,
                x + 4,
                padding + 4,
                card_width - 8,
                card_height - 8,
                image::Rgba([43, 56, 82, 255]),
            );
            let id = candidates[index];
            let Some(frame) = state.preview_frames.get(&id) else {
                continue;
            };
            let Some(source) = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(
                u32::from(frame.width),
                u32::from(frame.height),
                frame.rgba.as_slice(),
            ) else {
                continue;
            };
            let thumbnail = image::imageops::resize(
                &source,
                card_width - 16,
                card_height - 16,
                image::imageops::FilterType::Triangle,
            );
            image::imageops::overlay(
                &mut image,
                &thumbnail,
                i64::from(x + 8),
                i64::from(padding + 8),
            );
        }
    });
    Some((buffer, size.into()))
}

fn draw_memory_render_buffer(
    width: u32,
    height: u32,
    draw: impl FnOnce(&mut [u8]),
) -> MemoryRenderBuffer {
    let size = (width as i32, height as i32);
    let mut buffer = MemoryRenderBuffer::new(Fourcc::Abgr8888, size, 1, Transform::Normal, None);
    buffer
        .render()
        .draw(|pixels| {
            draw(pixels);
            Ok::<_, std::convert::Infallible>(vec![Rectangle::from_size(size.into())])
        })
        .expect("infallible task switcher drawing");
    buffer
}

fn fill_rgba_rect<I>(image: &mut I, x: u32, y: u32, width: u32, height: u32, color: image::Rgba<u8>)
where
    I: image::GenericImage<Pixel = image::Rgba<u8>>,
{
    for row in y..y.saturating_add(height).min(image.height()) {
        for column in x..x.saturating_add(width).min(image.width()) {
            image.put_pixel(column, row, color);
        }
    }
}

fn capture_preview(
    renderer: &mut NativeRenderer<'_>,
    window: &smithay::desktop::Window,
    rgba: &mut Vec<u8>,
) -> bool {
    (|| {
        const WIDTH: i32 = 240;
        const HEIGHT: i32 = 135;
        let geometry = window.geometry();
        if geometry.size.w <= 0 || geometry.size.h <= 0 {
            return None;
        }
        let mut texture = <NativeRenderer<'_> as Offscreen<GlesTexture>>::create_buffer(
            renderer,
            Fourcc::Abgr8888,
            (WIDTH, HEIGHT).into(),
        )
        .ok()?;
        let mut framebuffer = renderer.bind(&mut texture).ok()?;
        let elements = window.render_elements::<WaylandSurfaceRenderElement<NativeRenderer<'_>>>(
            renderer,
            (-geometry.loc.x, -geometry.loc.y).into(),
            Scale::from(1.0),
            1.0,
        );
        let damage = Rectangle::from_size((WIDTH, HEIGHT).into());
        let reference = Rectangle::from_size(geometry.size.to_physical(1));
        let elements = constrain_render_elements(
            elements,
            (0, 0),
            damage,
            reference,
            ConstrainScaleBehavior::Fit,
            ConstrainAlign::TOP
                | ConstrainAlign::BOTTOM
                | ConstrainAlign::LEFT
                | ConstrainAlign::RIGHT,
            1.0,
        )
        .collect::<Vec<_>>();
        let mut frame = renderer
            .render(&mut framebuffer, (WIDTH, HEIGHT).into(), Transform::Normal)
            .ok()?;
        frame
            .clear(Color32F::new(0.03, 0.04, 0.06, 1.0), &[damage])
            .ok()?;
        draw_render_elements(&mut frame, 1.0, &elements, &[damage]).ok()?;
        frame.finish().ok()?.wait().ok()?;
        let region = Rectangle::<i32, Buffer>::from_size((WIDTH, HEIGHT).into());
        let mapping = renderer
            .copy_framebuffer(&framebuffer, region, Fourcc::Abgr8888)
            .ok()?;
        let mapped = renderer.map_texture(&mapping).ok()?;
        if !crate::state::preview_mapping_has_exact_size(mapped) {
            return None;
        }
        let replacement = crate::state::reuse_preview_pixels(std::mem::take(rgba), mapped);
        *rgba = replacement;
        Some(())
    })()
    .is_some()
}

fn save_mapped_capture(
    mapped: &[u8],
    size: smithay::utils::Size<i32, Physical>,
    flipped: bool,
    path: &Path,
) -> Result<(), String> {
    let rgba = normalize_capture_rows(mapped, size.w as usize, size.h as usize, flipped)?;
    image::save_buffer(
        path,
        &rgba,
        size.w as u32,
        size.h as u32,
        image::ColorType::Rgba8,
    )
    .map_err(|error| error.to_string())
}

fn capture_composited_mapped<'a>(
    renderer: &mut NativeRenderer<'a>,
    elements: &[NativeElement<
        NativeRenderer<'a>,
        WaylandSurfaceRenderElement<NativeRenderer<'a>>,
    >],
    size: smithay::utils::Size<i32, Physical>,
    mut consume: impl FnMut(&[u8], bool) -> Result<(), String>,
) -> Result<(), String> {
    if size.w <= 0 || size.h <= 0 {
        return Err("output has no drawable size".into());
    }
    let buffer_size = smithay::utils::Size::<i32, Buffer>::from((size.w, size.h));
    let mut texture = <NativeRenderer<'a> as Offscreen<GlesTexture>>::create_buffer(
        renderer,
        Fourcc::Abgr8888,
        buffer_size,
    )
    .map_err(|error| error.to_string())?;
    let mut framebuffer = renderer
        .bind(&mut texture)
        .map_err(|error| error.to_string())?;
    let damage = Rectangle::from_size(size);
    let mut frame = renderer
        .render(&mut framebuffer, size, Transform::Normal)
        .map_err(|error| error.to_string())?;
    frame
        .clear(Color32F::new(0.1, 0.1, 0.1, 1.0), &[damage])
        .map_err(|error| error.to_string())?;
    draw_render_elements(&mut frame, 1.0, elements, &[damage])
        .map_err(|error| error.to_string())?;
    frame
        .finish()
        .map_err(|error| error.to_string())?
        .wait()
        .map_err(|error| error.to_string())?;
    let region = Rectangle::<i32, Buffer>::from_size(buffer_size);
    let mapping = renderer
        .copy_framebuffer(&framebuffer, region, Fourcc::Abgr8888)
        .map_err(|error| error.to_string())?;
    let flipped = mapping.flipped();
    let mapped = renderer
        .map_texture(&mapping)
        .map_err(|error| error.to_string())?;
    consume(mapped, flipped)
}

fn normalize_capture_rows(
    mapped: &[u8],
    width: usize,
    height: usize,
    flipped: bool,
) -> Result<Vec<u8>, String> {
    let row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| "capture row size overflowed".to_owned())?;
    let expected = row_bytes
        .checked_mul(height)
        .ok_or_else(|| "capture buffer size overflowed".to_owned())?;
    let mut rgba = vec![0; expected];
    copy_capture_rows(&mut rgba, mapped, width, height, flipped)?;
    Ok(rgba)
}

fn copy_capture_rows(
    destination: &mut [u8],
    mapped: &[u8],
    width: usize,
    height: usize,
    flipped: bool,
) -> Result<(), String> {
    let row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| "capture row size overflowed".to_owned())?;
    let expected = row_bytes
        .checked_mul(height)
        .ok_or_else(|| "capture buffer size overflowed".to_owned())?;
    if mapped.len() < expected {
        return Err(format!(
            "renderer returned {} bytes for a {} byte output",
            mapped.len(),
            expected
        ));
    }
    if destination.len() < expected {
        return Err(format!(
            "copyout destination has {} bytes for a {} byte output",
            destination.len(),
            expected
        ));
    }
    for destination_y in 0..height {
        let source_y = if flipped {
            destination_y
        } else {
            height - 1 - destination_y
        };
        destination[destination_y * row_bytes..(destination_y + 1) * row_bytes]
            .copy_from_slice(&mapped[source_y * row_bytes..(source_y + 1) * row_bytes]);
    }
    Ok(())
}

#[cfg(test)]
fn copy_capture_damage(
    destination: &mut [u8],
    mapped: &[u8],
    width: usize,
    height: usize,
    flipped: bool,
) -> Result<Vec<Rectangle<i32, Buffer>>, String> {
    let row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| "capture row size overflowed".to_owned())?;
    let expected = row_bytes
        .checked_mul(height)
        .ok_or_else(|| "capture buffer size overflowed".to_owned())?;
    if mapped.len() < expected || destination.len() < expected {
        return Err(format!(
            "copyout buffers are {} and {} bytes for a {} byte output",
            mapped.len(),
            destination.len(),
            expected
        ));
    }

    let mut damage = Vec::new();
    let mut first_changed_row = None;
    for destination_y in 0..height {
        let source_y = if flipped {
            destination_y
        } else {
            height - 1 - destination_y
        };
        let destination_range = destination_y * row_bytes..(destination_y + 1) * row_bytes;
        let source = &mapped[source_y * row_bytes..(source_y + 1) * row_bytes];
        if destination[destination_range.clone()] == *source {
            if let Some(first) = first_changed_row.take() {
                damage.push(Rectangle::new(
                    (0, first as i32).into(),
                    (width as i32, (destination_y - first) as i32).into(),
                ));
            }
            continue;
        }
        destination[destination_range].copy_from_slice(source);
        first_changed_row.get_or_insert(destination_y);
    }
    if let Some(first) = first_changed_row {
        damage.push(Rectangle::new(
            (0, first as i32).into(),
            (width as i32, (height - first) as i32).into(),
        ));
    }
    Ok(damage)
}

#[cfg(test)]
fn mapped_damage_rows(
    existing: &[u8],
    existing_stride: usize,
    mapped: &[u8],
    width: usize,
    height: usize,
    flipped: bool,
) -> Result<Vec<Rectangle<i32, Buffer>>, String> {
    let row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| "copyout row size overflowed".to_owned())?;
    let mapped_bytes = row_bytes
        .checked_mul(height)
        .ok_or_else(|| "mapped copyout size overflowed".to_owned())?;
    let existing_bytes = existing_stride
        .checked_mul(height)
        .ok_or_else(|| "dumb buffer size overflowed".to_owned())?;
    if mapped.len() < mapped_bytes || existing.len() < existing_bytes {
        return Err(format!(
            "copyout buffers are {} and {} bytes for {} mapped and {} scanout bytes",
            mapped.len(),
            existing.len(),
            mapped_bytes,
            existing_bytes
        ));
    }

    let mut damage = Vec::new();
    let mut first_changed_row = None;
    for destination_y in 0..height {
        let source_y = if flipped {
            destination_y
        } else {
            height - 1 - destination_y
        };
        let source = &mapped[source_y * row_bytes..(source_y + 1) * row_bytes];
        let destination =
            &existing[destination_y * existing_stride..destination_y * existing_stride + row_bytes];
        if destination == source {
            if let Some(first) = first_changed_row.take() {
                damage.push(Rectangle::new(
                    (0, first as i32).into(),
                    (width as i32, (destination_y - first) as i32).into(),
                ));
            }
        } else {
            first_changed_row.get_or_insert(destination_y);
        }
    }
    if let Some(first) = first_changed_row {
        damage.push(Rectangle::new(
            (0, first as i32).into(),
            (width as i32, (height - first) as i32).into(),
        ));
    }
    Ok(damage)
}

#[cfg(test)]
fn copy_mapped_damage_to_strided(
    destination: &mut [u8],
    destination_stride: usize,
    mapped: &[u8],
    width: usize,
    height: usize,
    flipped: bool,
    damage: &[Rectangle<i32, Buffer>],
) -> Result<usize, String> {
    let source_stride = width
        .checked_mul(4)
        .ok_or_else(|| "copyout row size overflowed".to_owned())?;
    let mut copied = 0_usize;
    for rectangle in damage {
        if rectangle.loc.x < 0
            || rectangle.loc.y < 0
            || rectangle.size.w < 0
            || rectangle.size.h < 0
        {
            return Err("copyout damage lies outside the buffer".into());
        }
        let x = rectangle.loc.x as usize * 4;
        let copy_width = rectangle.size.w as usize * 4;
        for destination_y in rectangle.loc.y as usize..(rectangle.loc.y + rectangle.size.h) as usize
        {
            if destination_y >= height {
                return Err("copyout damage exceeds mapped buffer height".into());
            }
            let source_y = if flipped {
                destination_y
            } else {
                height - 1 - destination_y
            };
            let source_start = source_y
                .checked_mul(source_stride)
                .and_then(|offset| offset.checked_add(x))
                .ok_or_else(|| "copyout source offset overflowed".to_owned())?;
            let destination_start = destination_y
                .checked_mul(destination_stride)
                .and_then(|offset| offset.checked_add(x))
                .ok_or_else(|| "copyout destination offset overflowed".to_owned())?;
            let source_row = mapped
                .get(source_start..source_start + copy_width)
                .ok_or_else(|| "copyout damage exceeds source buffer".to_owned())?;
            let destination_row = destination
                .get_mut(destination_start..destination_start + copy_width)
                .ok_or_else(|| "copyout damage exceeds dumb buffer".to_owned())?;
            destination_row.copy_from_slice(source_row);
            copied = copied.saturating_add(copy_width);
        }
    }
    Ok(copied)
}

fn damage_bytes(damage: &[Rectangle<i32, Physical>]) -> u64 {
    damage.iter().fold(0_u64, |total, rectangle| {
        total.saturating_add(rectangle.size.w.max(0) as u64 * rectangle.size.h.max(0) as u64 * 4)
    })
}

fn damage_bounding_box(damage: &[Rectangle<i32, Physical>]) -> Option<Rectangle<i32, Buffer>> {
    let first = damage.first()?;
    let mut x1 = first.loc.x;
    let mut y1 = first.loc.y;
    let mut x2 = first.loc.x.saturating_add(first.size.w);
    let mut y2 = first.loc.y.saturating_add(first.size.h);
    for rectangle in &damage[1..] {
        x1 = x1.min(rectangle.loc.x);
        y1 = y1.min(rectangle.loc.y);
        x2 = x2.max(rectangle.loc.x.saturating_add(rectangle.size.w));
        y2 = y2.max(rectangle.loc.y.saturating_add(rectangle.size.h));
    }
    Some(Rectangle::new((x1, y1).into(), (x2 - x1, y2 - y1).into()))
}

fn union_rectangles(
    first: Rectangle<i32, Buffer>,
    second: Rectangle<i32, Buffer>,
) -> Rectangle<i32, Buffer> {
    let x1 = first.loc.x.min(second.loc.x);
    let y1 = first.loc.y.min(second.loc.y);
    let x2 = first
        .loc
        .x
        .saturating_add(first.size.w)
        .max(second.loc.x.saturating_add(second.size.w));
    let y2 = first
        .loc
        .y
        .saturating_add(first.size.h)
        .max(second.loc.y.saturating_add(second.size.h));
    Rectangle::new((x1, y1).into(), (x2 - x1, y2 - y1).into())
}

fn copy_mapped_region_to_strided(
    destination: &mut [u8],
    destination_stride: usize,
    mapped: &[u8],
    flipped: bool,
    region: Rectangle<i32, Buffer>,
    destination_size: Size<i32, Physical>,
) -> Result<usize, String> {
    if region.loc.x < 0
        || region.loc.y < 0
        || region.size.w <= 0
        || region.size.h <= 0
        || region.loc.x.saturating_add(region.size.w) > destination_size.w
        || region.loc.y.saturating_add(region.size.h) > destination_size.h
    {
        return Err("copyout region lies outside the EVDI buffer".into());
    }
    let width = region.size.w as usize;
    let height = region.size.h as usize;
    let row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| "copyout region row size overflowed".to_owned())?;
    let expected = row_bytes
        .checked_mul(height)
        .ok_or_else(|| "copyout region size overflowed".to_owned())?;
    if mapped.len() < expected {
        return Err(format!(
            "renderer returned {} bytes for a {} byte damaged region",
            mapped.len(),
            expected
        ));
    }
    let destination_x = region.loc.x as usize * 4;
    for local_y in 0..height {
        let source_y = if flipped {
            local_y
        } else {
            height - 1 - local_y
        };
        let destination_y = region.loc.y as usize + local_y;
        let source_start = source_y * row_bytes;
        let destination_start = destination_y
            .checked_mul(destination_stride)
            .and_then(|offset| offset.checked_add(destination_x))
            .ok_or_else(|| "copyout destination offset overflowed".to_owned())?;
        destination
            .get_mut(destination_start..destination_start + row_bytes)
            .ok_or_else(|| "copyout region exceeds the dumb buffer".to_owned())?
            .copy_from_slice(&mapped[source_start..source_start + row_bytes]);
    }
    Ok(expected)
}

#[cfg(test)]
mod tests;
