use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        allocator::{
            Fourcc,
            format::FormatSet,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode,
            compositor::FrameFlags,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
        },
        egl::context::ContextPriority,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            Bind, Color32F, ExportMem, Frame, ImportAll, ImportDma, ImportMem, Offscreen, Renderer,
            TextureMapping,
            element::{
                AsRenderElements, Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::{SolidColorBuffer, SolidColorRenderElement},
                surface::WaylandSurfaceRenderElement,
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
        drm::control::{ModeTypeFlags, connector, crtc},
        input::Libinput,
        rustix::fs::OFlags,
        wayland_server::{Resource, backend::GlobalId},
    },
    utils::{Buffer, DeviceFd, Physical, Point, Rectangle, Scale, Transform},
    wayland::seat::WaylandFocus,
};
use thiserror::Error;

use nickel_core::{
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

#[derive(Debug, Eq, PartialEq)]
struct OutputId {
    device: DrmNode,
    crtc: crtc::Handle,
}

struct SurfaceData {
    global: Option<GlobalId>,
    output: Output,
    drm: NativeDrmOutput,
    background: SolidColorBuffer,
    render_path_logged: bool,
    invalidate_pending: bool,
}

struct DeviceData {
    registration: RegistrationToken,
    manager: NativeOutputManager,
    scanner: DrmScanner,
    render_node: DrmNode,
    is_evdi: bool,
    render_scheduled: bool,
    surfaces: HashMap<crtc::Handle, SurfaceData>,
}

pub struct UdevData {
    session: LibSeatSession,
    activity: SessionActivity,
    gpus: GpuManager<RendererBackend>,
    primary_gpu: DrmNode,
    devices: HashMap<DrmNode, DeviceData>,
    layout: OutputLayout,
    bootstrap_render_until: Instant,
    client_bootstrap_started: bool,
    cursors: HashMap<crate::window_frame::FrameCursor, CursorBuffer>,
    frame_icons: Option<crate::window_frame::FrameIcons>,
    identify_badges: Vec<MemoryRenderBuffer>,
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
}

pub fn init_udev(
    event_loop: &mut EventLoop<'static, NickelSession>,
    data: &mut NickelSession,
) -> Result<(), Box<dyn std::error::Error>> {
    let (session, notifier) = LibSeatSession::new()?;
    let seat_name = session.seat();
    let udev = UdevBackend::new(&seat_name)?;
    let primary_gpu = select_primary_gpu(&session)?;
    let gpus = GpuManager::new(GbmGlesBackend::with_context_priority(ContextPriority::High))?;

    data.native = Some(UdevData {
        session: session.clone(),
        activity: SessionActivity::default(),
        gpus,
        primary_gpu,
        devices: HashMap::new(),
        layout: OutputLayout::default(),
        bootstrap_render_until: Instant::now() + BOOTSTRAP_RENDER_TIMEOUT,
        client_bootstrap_started: false,
        cursors: themed_cursors(),
        frame_icons: crate::window_frame::FrameIcons::load(),
        identify_badges: (1..=9).map(identify_badge).collect(),
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
                        if let Err(error) = device.manager.lock().activate(false) {
                            tracing::error!(?error, "failed to reactivate DRM device");
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
    pub(crate) fn render_all_outputs(&mut self) {
        let nodes = self
            .native
            .as_ref()
            .map(|native| native.devices.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for node in &nodes {
            self.render_node(*node);
        }
        let timer = Timer::from_duration(Duration::from_millis(3050));
        let _ = self
            .event_loop_handle
            .insert_source(timer, move |_, _, data| {
                for node in &nodes {
                    data.render_node(*node);
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
        let native = self.native.as_mut().expect("native backend should exist");
        if native.devices.contains_key(&node) {
            return Ok(());
        }
        let is_evdi = is_evdi_device(path);
        if is_evdi
            && matches!(
                has_connected_drm_connector(Path::new("/sys/class/drm"), path),
                Some(false)
            )
        {
            tracing::debug!(%node, path = %path.display(), "deferring disconnected EVDI device");
            return Ok(());
        }
        let fd = native.session.open(
            path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )?;
        let fd = DrmDeviceFd::new(DeviceFd::from(fd));
        let (drm, notifier) = DrmDevice::new(fd.clone(), true)?;
        let gbm = GbmDevice::new(fd)?;
        native.gpus.as_mut().add_node(node, gbm.clone())?;
        let render_node = node;
        let renderer = native
            .gpus
            .single_renderer(&render_node)
            .map_err(|error| DeviceError::Renderer(error.to_string()))?;
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

        let registration = handle
            .insert_source(notifier, move |event, _, data| match event {
                DrmEvent::VBlank(crtc) => data.frame_submitted(node, crtc),
                DrmEvent::Error(error) => tracing::error!(%node, ?error, "DRM event error"),
            })
            .expect("DRM notifier registration should succeed");
        native.devices.insert(
            node,
            DeviceData {
                registration,
                manager,
                scanner: DrmScanner::new(),
                render_node,
                is_evdi,
                render_scheduled: false,
                surfaces: HashMap::new(),
            },
        );
        self.scan_connectors(node);
        Ok(())
    }

    fn scan_connectors(&mut self, node: DrmNode) {
        let events = {
            let Some(device) = self
                .native
                .as_mut()
                .and_then(|native| native.devices.get_mut(&node))
            else {
                return;
            };
            match device.scanner.scan_connectors(device.manager.device()) {
                Ok(events) => events,
                Err(error) => {
                    tracing::warn!(%node, ?error, "failed to scan DRM connectors");
                    return;
                }
            }
        };
        for event in events {
            match event {
                DrmScanEvent::Connected {
                    connector,
                    crtc: Some(crtc),
                } => self.connect_output(node, connector, crtc),
                DrmScanEvent::Disconnected {
                    connector,
                    crtc: Some(crtc),
                } => self.disconnect_output(node, connector, crtc),
                _ => {}
            }
        }
    }

    fn connect_output(&mut self, node: DrmNode, connector: connector::Info, crtc: crtc::Handle) {
        let name = format!(
            "{}-{}",
            connector.interface().as_str(),
            connector.interface_id()
        );
        let Some(mode) = connector
            .modes()
            .iter()
            .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
            .copied()
            .or_else(|| connector.modes().first().copied())
        else {
            tracing::warn!(output = %name, "connector has no modes");
            return;
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
        let native = self.native.as_mut().expect("native backend should exist");
        let device = native.devices.get(&node).expect("DRM device should exist");
        let is_primary = node == native.primary_gpu;
        let positions = native.layout.connect(
            name.clone(),
            wl_mode.size.w,
            wl_mode.size.h,
            u8::from(!device.is_evdi),
        );
        let location = positions
            .iter()
            .find(|position| position.name == name)
            .expect("connected output should be in layout")
            .to_owned();
        let location = (location.x, location.y).into();
        output.set_preferred(wl_mode);
        output.change_current_state(Some(wl_mode), Some(Transform::Normal), None, Some(location));
        let global = output.create_global::<NickelSession>(&self.display_handle);
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

        let device = native
            .devices
            .get_mut(&node)
            .expect("DRM device should exist");
        let mut renderer = native
            .gpus
            .single_renderer(&device.render_node)
            .expect("renderer should exist");
        let empty = DrmOutputRenderElements::<
            NativeRenderer<'_>,
            NativeElement<NativeRenderer<'_>, WaylandSurfaceRenderElement<NativeRenderer<'_>>>,
        >::default();
        let initialized = {
            let mut manager = device.manager.lock();
            manager.initialize_output(
                crtc,
                mode,
                &[connector.handle()],
                &output,
                None,
                &mut renderer,
                &empty,
            )
        };
        match initialized {
            Ok(drm) => {
                device.surfaces.insert(
                    crtc,
                    SurfaceData {
                        global: Some(global),
                        output: output.clone(),
                        drm,
                        background: SolidColorBuffer::new(
                            wl_mode.size.to_logical(1),
                            [0.055, 0.065, 0.085, 1.0],
                        ),
                        render_path_logged: false,
                        invalidate_pending: device.is_evdi,
                    },
                );
                self.restore_output_windows(&output);
                self.relayout_shell_surfaces();
                self.schedule_render(node, Duration::ZERO);
                tracing::info!(output = %name, "DRM output connected");
            }
            Err(error) => {
                self.space.unmap_output(&output);
                self.display_handle.remove_global::<NickelSession>(global);
                tracing::error!(output = %name, ?error, "failed to initialize DRM output");
            }
        }
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
                self.display_handle.remove_global::<NickelSession>(global);
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

    fn remove_drm_device(&mut self, node: DrmNode) {
        let Some(native) = self.native.as_mut() else {
            return;
        };
        let Some(mut device) = native.devices.remove(&node) else {
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
        native.gpus.as_mut().remove_node(&device.render_node);
        self.event_loop_handle.remove(device.registration);
        for mut surface in removed_surfaces.drain(..) {
            self.stage_output_removal(&surface.output);
            self.space.unmap_output(&surface.output);
            surface.output.leave_all();
            if let Some(global) = surface.global.take() {
                self.display_handle.remove_global::<NickelSession>(global);
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
        tracing::info!(%node, "DRM device removed");
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
        device.render_scheduled = true;
        let timer = Timer::from_duration(delay);
        let _ = self
            .event_loop_handle
            .insert_source(timer, move |_, _, data| {
                if let Some(device) = data
                    .native
                    .as_mut()
                    .and_then(|native| native.devices.get_mut(&node))
                {
                    device.render_scheduled = false;
                }
                data.render_node(node);
                TimeoutAction::Drop
            });
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
        let crtcs: Vec<_> = self
            .native
            .as_ref()
            .and_then(|native| native.devices.get(&node))
            .map(|device| device.surfaces.keys().copied().collect())
            .unwrap_or_default();
        for crtc in crtcs {
            self.render_output(node, crtc);
        }
    }

    fn render_output(&mut self, node: DrmNode, crtc: crtc::Handle) {
        let shell_bootstrapping = self.launcher_window.is_none();
        let identify_index = self
            .identify_outputs_until
            .filter(|deadline| *deadline > std::time::Instant::now())
            .and_then(|_| {
                let mut outputs = self.space.outputs().cloned().collect::<Vec<_>>();
                outputs.sort_by_key(|output| {
                    self.space
                        .output_geometry(output)
                        .map(|geometry| (geometry.loc.x, geometry.loc.y))
                        .unwrap_or_default()
                });
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
            let identify_badge = identify_index
                .and_then(|index| native.identify_badges.get(index))
                .cloned();
            let preview_windows = if self.locked {
                Vec::new()
            } else {
                self.space
                    .elements()
                    .filter_map(|window| {
                        let id = self.surface_windows.get(&window.wl_surface()?.id())?;
                        self.preview_requests
                            .contains(id)
                            .then(|| (*id, window.clone()))
                    })
                    .collect::<Vec<_>>()
            };
            if surface.invalidate_pending {
                surface
                    .drm
                    .with_compositor(|compositor| compositor.reset_buffer_ages());
                surface.invalidate_pending = false;
            }
            if !surface.render_path_logged {
                tracing::info!(
                    output = %output.name(),
                    render_gpu = %native.primary_gpu,
                    target_gpu = %device.render_node,
                    format = ?surface.drm.format(),
                    cross_gpu = native.primary_gpu != device.render_node,
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
                    tracing::error!(
                        ?error,
                        render = %native.primary_gpu,
                        target = %target_gpu,
                        "failed to acquire multi-GPU renderer"
                    );
                    return None;
                }
            };
            for (id, window) in preview_windows {
                if let Some(frame) = capture_preview(&mut renderer, &window) {
                    self.preview_frames.insert(id, frame);
                    self.notify_preview_frame(id);
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
                    let foreground = if active {
                        frame_palette.text
                    } else {
                        frame_palette.muted
                    };
                    if let Some(titlebar) = crate::window_frame::render_titlebar(
                        bounds.size.w,
                        title,
                        frame_palette.panel,
                        foreground,
                    ) && let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                        &mut renderer,
                        (
                            f64::from(bounds.loc.x - output_geometry.loc.x),
                            f64::from(
                                bounds.loc.y
                                    - output_geometry.loc.y
                                    - crate::window_frame::TITLEBAR_HEIGHT,
                            ),
                        ),
                        &titlebar,
                        None,
                        None,
                        Some((bounds.size.w, crate::window_frame::TITLEBAR_HEIGHT).into()),
                        Kind::Unspecified,
                    ) {
                        elements.push(NativeCustomElement::from(element).into());
                    }
                    let frame_height = bounds.size.h + crate::window_frame::TITLEBAR_HEIGHT;
                    for shadow in crate::window_frame::shadow_layers(bounds.size.w, frame_height) {
                        elements.push(
                            NativeCustomElement::from(SolidColorRenderElement::from_buffer(
                                &shadow.buffer,
                                (
                                    bounds.loc.x - output_geometry.loc.x + shadow.offset.0,
                                    bounds.loc.y
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
                        let icon_y = bounds.loc.y
                            - output_geometry.loc.y
                            - crate::window_frame::TITLEBAR_HEIGHT
                            + 8;
                        let icon_x = bounds.loc.x - output_geometry.loc.x + bounds.size.w;
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
                let banner_width = recovery_size.w.clamp(1, 560);
                let banner_height = recovery_size.h.clamp(1, 112);
                let banner =
                    SolidColorBuffer::new((banner_width, banner_height), [0.45, 0.06, 0.08, 1.0]);
                elements.push(
                    NativeCustomElement::from(SolidColorRenderElement::from_buffer(
                        &banner,
                        (
                            (recovery_size.w - banner_width) / 2,
                            (recovery_size.h - banner_height) / 2,
                        ),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    ))
                    .into(),
                );
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
            if !self.locked
                && is_primary
                && let Some(mode) = output.current_mode()
                && let Some((switcher, switcher_size)) = task_switcher_buffer(self, mode.size)
            {
                let location = (
                    (mode.size.w - switcher_size.w).max(0) / 2,
                    (mode.size.h - switcher_size.h).max(0) / 2,
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
            if is_primary && let Some(path) = self.output_capture_path.take() {
                let result = output
                    .current_mode()
                    .ok_or_else(|| "output has no active mode".to_owned())
                    .and_then(|mode| {
                        capture_composited_output(&mut renderer, &elements, mode.size, &path)
                    });
                let response = match result {
                    Ok(()) => nickel_session_protocol::CaptureResult::Saved {
                        backend: nickel_session_protocol::CaptureBackend::Native,
                    },
                    Err(error) => {
                        tracing::warn!(%error, path = %path.display(), "failed to capture output");
                        nickel_session_protocol::CaptureResult::Failed { message: error }
                    }
                };
                self.complete_output_capture(&path, response);
            }
            let retry = match surface.drm.render_frame(
                &mut renderer,
                &elements,
                [0.1, 0.1, 0.1, 1.0],
                frame_flags,
            ) {
                Ok(frame) if !frame.is_empty => {
                    if let Err(error) = surface.drm.queue_frame(()) {
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
            };
            Some((output, retry))
        })();
        self.native = Some(native);
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
        self.schedule_render(node, Duration::ZERO);
    }
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
    let mut image = image::RgbaImage::from_pixel(width, height, image::Rgba([17, 24, 39, 244]));

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
        let Some(source) = image::RgbaImage::from_raw(
            u32::from(frame.width),
            u32::from(frame.height),
            frame.rgba.clone(),
        ) else {
            continue;
        };
        let target_width = card_width - 16;
        let target_height = card_height - 16;
        let thumbnail = image::imageops::resize(
            &source,
            target_width,
            target_height,
            image::imageops::FilterType::Triangle,
        );
        image::imageops::overlay(
            &mut image,
            &thumbnail,
            i64::from(x + 8),
            i64::from(padding + 8),
        );
    }

    let size = (width as i32, height as i32);
    Some((
        MemoryRenderBuffer::from_slice(
            image.as_raw(),
            Fourcc::Abgr8888,
            size,
            1,
            Transform::Normal,
            None,
        ),
        size.into(),
    ))
}

fn fill_rgba_rect(
    image: &mut image::RgbaImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    color: image::Rgba<u8>,
) {
    for row in y..y.saturating_add(height).min(image.height()) {
        for column in x..x.saturating_add(width).min(image.width()) {
            image.put_pixel(column, row, color);
        }
    }
}

fn capture_preview(
    renderer: &mut NativeRenderer<'_>,
    window: &smithay::desktop::Window,
) -> Option<PreviewFrame> {
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
        ConstrainAlign::TOP | ConstrainAlign::BOTTOM | ConstrainAlign::LEFT | ConstrainAlign::RIGHT,
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
    let rgba = renderer.map_texture(&mapping).ok()?.to_vec();
    Some(PreviewFrame {
        width: WIDTH as u16,
        height: HEIGHT as u16,
        rgba,
    })
}

fn capture_composited_output<'a>(
    renderer: &mut NativeRenderer<'a>,
    elements: &[NativeElement<
        NativeRenderer<'a>,
        WaylandSurfaceRenderElement<NativeRenderer<'a>>,
    >],
    size: smithay::utils::Size<i32, Physical>,
    path: &Path,
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
    if mapped.len() < expected {
        return Err(format!(
            "renderer returned {} bytes for a {} byte output",
            mapped.len(),
            expected
        ));
    }
    let mut rgba = vec![0; expected];
    for destination_y in 0..height {
        let source_y = if flipped {
            destination_y
        } else {
            height - 1 - destination_y
        };
        rgba[destination_y * row_bytes..(destination_y + 1) * row_bytes]
            .copy_from_slice(&mapped[source_y * row_bytes..(source_y + 1) * row_bytes]);
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::{normalize_capture_rows, parse_kde_cursor_settings, switcher_visible_range};

    #[test]
    fn task_switcher_keeps_the_selection_in_a_centered_bounded_window() {
        assert_eq!(switcher_visible_range(3, 1), 0..3);
        assert_eq!(switcher_visible_range(9, 0), 0..5);
        assert_eq!(switcher_visible_range(9, 4), 2..7);
        assert_eq!(switcher_visible_range(9, 8), 4..9);
    }

    #[test]
    fn reads_cursor_preferences_from_kde_mouse_group() {
        let settings = parse_kde_cursor_settings(
            "[Keyboard]\nRepeatDelay=600\n[Mouse]\ncursorTheme=Oxygen_Black\ncursorSize=36\n",
        );
        assert_eq!(settings, (Some("Oxygen_Black".into()), Some(36)));
    }

    #[test]
    fn ignores_cursor_preferences_outside_kde_mouse_group() {
        let settings =
            parse_kde_cursor_settings("[Other]\ncursorTheme=Oxygen_Black\ncursorSize=36\n");
        assert_eq!(settings, (None, None));
    }

    #[test]
    fn preserves_rows_from_a_flipped_renderer_mapping() {
        let bottom = [9_u8; 8];
        let top = [3_u8; 8];
        let mapped = [bottom, top].concat();
        let normalized = normalize_capture_rows(&mapped, 2, 2, true).unwrap();
        assert_eq!(&normalized[..8], &bottom);
        assert_eq!(&normalized[8..], &top);
    }

    #[test]
    fn reverses_rows_from_an_unflipped_renderer_mapping() {
        let bottom = [9_u8; 8];
        let top = [3_u8; 8];
        let mapped = [bottom, top].concat();
        let normalized = normalize_capture_rows(&mapped, 2, 2, false).unwrap();
        assert_eq!(&normalized[..8], &top);
        assert_eq!(&normalized[8..], &bottom);
    }
}
