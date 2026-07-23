use std::{collections::HashMap, path::Path, time::Duration};

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
            ImportAll, ImportDma, ImportMem,
            element::{
                Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                surface::WaylandSurfaceRenderElement,
            },
            gles::GlesRenderer,
            multigpu::{GpuManager, gbm::GbmGlesBackend},
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
            EventLoop, RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        drm::control::{ModeTypeFlags, connector, crtc},
        input::Libinput,
        rustix::fs::OFlags,
        wayland_server::backend::GlobalId,
    },
    utils::{DeviceFd, Transform},
};
use thiserror::Error;

use crate::{
    CalloopData, NickelSession,
    backend::{
        OutputLayout, SessionActivity,
        drm_scanner::{DrmScanEvent, DrmScanner},
    },
};

const FORMATS: &[Fourcc] = &[Fourcc::Abgr8888, Fourcc::Argb8888];

type RendererBackend = GbmGlesBackend<GlesRenderer, DrmDeviceFd>;
type NativeRenderer<'a> =
    smithay::backend::renderer::multigpu::MultiRenderer<'a, 'a, RendererBackend, RendererBackend>;
smithay::backend::renderer::element::render_elements! {
    NativeCustomElement<R> where R: ImportAll + ImportMem;
    Pointer=MemoryRenderBufferRenderElement<R>,
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
    render_path_logged: bool,
}

struct DeviceData {
    registration: RegistrationToken,
    manager: NativeOutputManager,
    scanner: DrmScanner,
    render_node: DrmNode,
    surfaces: HashMap<crtc::Handle, SurfaceData>,
}

pub struct UdevData {
    session: LibSeatSession,
    activity: SessionActivity,
    gpus: GpuManager<RendererBackend>,
    primary_gpu: DrmNode,
    devices: HashMap<DrmNode, DeviceData>,
    layout: OutputLayout,
    cursor: MemoryRenderBuffer,
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
    event_loop: &mut EventLoop<'static, CalloopData>,
    data: &mut CalloopData,
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
        cursor: arrow_cursor(),
    });

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

    let mut libinput =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput
        .udev_assign_seat(&seat_name)
        .map_err(|()| "libinput rejected the active seat")?;
    let input = LibinputInputBackend::new(libinput.clone());
    event_loop.handle().insert_source(input, |event, _, data| {
        if let Some(vt) = data.state.process_input_event(event)
            && let Some(native) = data.native.as_mut()
            && let Err(error) = native.session.change_vt(vt)
        {
            tracing::error!(vt, ?error, "failed to switch virtual terminal");
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
                        if let Err(error) = device.manager.activate(false) {
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
                    data.scan_connectors(node);
                }
            }
            UdevEvent::Removed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    data.remove_drm_device(node);
                }
            }
        })?;

    unsafe { std::env::set_var("WAYLAND_DISPLAY", &data.state.socket_name) };
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

impl CalloopData {
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
            GbmFramebufferExporter::new(gbm.clone(), Some(render_node)),
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
        let output = Output::new(
            name.clone(),
            PhysicalProperties {
                size: (width as i32, height as i32).into(),
                subpixel: connector.subpixel().into(),
                make: "Unknown".into(),
                model: name.clone(),
            },
        );
        let native = self.native.as_mut().expect("native backend should exist");
        let location = native
            .layout
            .connect(name.clone(), wl_mode.size.to_logical(1));
        output.set_preferred(wl_mode);
        output.change_current_state(Some(wl_mode), Some(Transform::Normal), None, Some(location));
        let global = output.create_global::<NickelSession>(&self.display_handle);
        self.state.space.map_output(&output, location);
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
        match device.manager.initialize_output(
            crtc,
            mode,
            &[connector.handle()],
            &output,
            None,
            &mut renderer,
            &empty,
        ) {
            Ok(drm) => {
                device.surfaces.insert(
                    crtc,
                    SurfaceData {
                        global: Some(global),
                        output,
                        drm,
                        render_path_logged: false,
                    },
                );
                self.state.relayout_shell_surfaces();
                self.schedule_render(node, Duration::ZERO);
                tracing::info!(output = %name, "DRM output connected");
            }
            Err(error) => {
                self.state.space.unmap_output(&output);
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
        if let Some(mut surface) = native
            .devices
            .get_mut(&node)
            .and_then(|device| device.surfaces.remove(&crtc))
        {
            self.state.space.unmap_output(&surface.output);
            if let Some(global) = surface.global.take() {
                self.display_handle.remove_global::<NickelSession>(global);
            }
        }
        let positions = native.layout.disconnect(&name);
        for position in positions {
            let output = self
                .state
                .space
                .outputs()
                .find(|output| output.name() == position.name)
                .cloned();
            if let Some(output) = output {
                output.change_current_state(None, None, None, Some(position.location));
                self.state.space.map_output(&output, position.location);
            }
        }
        self.reflow_windows_to_connected_outputs();
        self.state.relayout_shell_surfaces();
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
        for (_, mut surface) in device.surfaces.drain() {
            positions = native.layout.disconnect(&surface.output.name());
            self.state.space.unmap_output(&surface.output);
            if let Some(global) = surface.global.take() {
                self.display_handle.remove_global::<NickelSession>(global);
            }
        }
        native.gpus.as_mut().remove_node(&device.render_node);
        self.event_loop_handle.remove(device.registration);
        for position in positions {
            let output = self
                .state
                .space
                .outputs()
                .find(|output| output.name() == position.name)
                .cloned();
            if let Some(output) = output {
                output.change_current_state(None, None, None, Some(position.location));
                self.state.space.map_output(&output, position.location);
            }
        }
        self.reflow_windows_to_connected_outputs();
        self.state.relayout_shell_surfaces();
        tracing::info!(%node, "DRM device removed");
    }

    fn schedule_render(&self, node: DrmNode, delay: Duration) {
        let timer = Timer::from_duration(delay);
        let _ = self
            .event_loop_handle
            .insert_source(timer, move |_, _, data| {
                data.render_node(node);
                TimeoutAction::Drop
            });
    }

    fn reflow_windows_to_connected_outputs(&mut self) {
        let output_geometries: Vec<_> = self
            .state
            .space
            .outputs()
            .filter_map(|output| self.state.space.output_geometry(output))
            .collect();
        let Some(fallback) = output_geometries.first().copied() else {
            return;
        };
        let stranded: Vec<_> = self
            .state
            .space
            .elements()
            .filter(|window| {
                self.state.space.element_bbox(window).is_some_and(|bounds| {
                    !output_geometries
                        .iter()
                        .any(|output| output.overlaps(bounds))
                })
            })
            .cloned()
            .collect();
        for window in stranded {
            self.state.space.map_element(window, fallback.loc, false);
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
        let (output, retry) = {
            let Some(native) = self.native.as_mut() else {
                return;
            };
            if !native.activity.is_active() {
                return;
            }
            let Some(device) = native.devices.get_mut(&node) else {
                return;
            };
            let Some(surface) = device.surfaces.get_mut(&crtc) else {
                return;
            };
            let output = surface.output.clone();
            let cursor = native.cursor.clone();
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
                    return;
                }
            };
            let mut elements: Vec<
                NativeElement<NativeRenderer<'_>, WaylandSurfaceRenderElement<NativeRenderer<'_>>>,
            > = match self
                .state
                .space
                .render_elements_for_output(&mut renderer, &output, 1.0)
            {
                Ok(elements) => elements.into_iter().map(NativeElement::from).collect(),
                Err(error) => {
                    tracing::warn!(?error, "failed to build output elements");
                    return;
                }
            };
            if let Some(geometry) = self.state.space.output_geometry(&output) {
                let pointer = self.state.seat.get_pointer().unwrap().current_location();
                if geometry.to_f64().contains(pointer) {
                    let location = (pointer - geometry.loc.to_f64())
                        .to_i32_round()
                        .to_physical(1);
                    match MemoryRenderBufferRenderElement::from_buffer(
                        &mut renderer,
                        location.to_f64(),
                        &cursor,
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
            let retry = match surface.drm.render_frame(
                &mut renderer,
                &elements,
                [0.1, 0.1, 0.1, 1.0],
                FrameFlags::DEFAULT,
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
                    true
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
            (output, retry)
        };
        if retry {
            self.schedule_render(node, Duration::from_millis(16));
        }
        self.state.space.elements().for_each(|window| {
            window.send_frame(
                &output,
                self.state.start_time.elapsed(),
                Some(Duration::ZERO),
                |_, _| Some(output.clone()),
            );
        });
        self.state.space.refresh();
        self.state.popups.cleanup();
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

fn arrow_cursor() -> MemoryRenderBuffer {
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
    MemoryRenderBuffer::from_slice(
        &rgba,
        Fourcc::Abgr8888,
        (width as i32, height as i32),
        1,
        Transform::Normal,
        None,
    )
}
