//! Compositor ownership for Nickel UI applications which do not have a Wayland surface.

use std::collections::BTreeMap;

use nickel_ui::{
    Application, DamageRegion, GradientAxis, HostBatch, HostEvent, InternalSurfaceId,
    InternalSurfaceSet, LinearGradient, Point as UiPoint, SoftwareRenderer, Text, UiEvent, View,
    ViewContext,
    backend::{FrameRenderer, PaintCommand, RenderFrame},
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Color32F, ImportMem, Renderer,
            element::{
                Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::{SolidColorBuffer, SolidColorRenderElement},
            },
        },
    },
    utils::{Logical, Point, Transform},
};

smithay::backend::renderer::element::render_elements! {
    /// Smithay elements emitted by the compositor-owned Nickel UI presenter.
    pub InternalUiRenderElement<R> where R: Renderer + ImportMem;
    Memory=MemoryRenderBufferRenderElement<R>,
    Solid=SolidColorRenderElement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternalSurfaceRole {
    Desktop,
    Panel,
    Overlay,
    Application,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InternalSurfacePlacement {
    pub role: InternalSurfaceRole,
    pub geometry: (i32, i32, u32, u32),
    pub output: Option<String>,
}

struct PresentedSurface {
    placement: InternalSurfacePlacement,
    renderer: SmithayFrameRenderer,
    dirty: bool,
    external_scene: Option<Vec<PaintCommand>>,
    scale_factor: f32,
}

struct SceneSlot;

impl Application for SceneSlot {
    type Message = ();
    fn update(&mut self, (): ()) {}
    fn view(&self, _: ViewContext) -> impl View<Self::Message> {
        Text::new("")
    }
}

fn output_local_location(
    geometry: (i32, i32, u32, u32),
    output_origin: Point<i32, Logical>,
) -> (f64, f64) {
    (
        f64::from(geometry.0 - output_origin.x),
        f64::from(geometry.1 - output_origin.y),
    )
}

/// How the most recently prepared UI frame will reach Smithay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternalUiPresentationMode {
    /// Every primitive maps to GPU-native Smithay solid elements.
    GpuSolid,
    /// The display list contains a primitive not yet handled natively and is
    /// rasterized once into an importable texture to preserve exact ordering.
    RasterFallback,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InternalUiRendererDiagnostics {
    pub gpu_frames: u64,
    pub fallback_frames: u64,
    pub fallback_primitive_count: usize,
    pub fallback_text_count: usize,
    pub fallback_image_count: usize,
}

/// Nickel display-list adapter for Smithay's renderer element API.
///
/// Geometry and gradients become solid render elements and never enter the
/// software rasterizer. Text and images use one bounded full-surface upload
/// until dedicated texture primitives are added.
pub struct SmithayFrameRenderer {
    software: SoftwareRenderer,
    solids: Vec<(nickel_ui::Rect, SolidColorBuffer)>,
    raster: Option<MemoryRenderBuffer>,
    mode: InternalUiPresentationMode,
    diagnostics: InternalUiRendererDiagnostics,
}

impl SmithayFrameRenderer {
    fn new(width: u32, height: u32, scale: f32) -> Self {
        Self {
            software: SoftwareRenderer::new(width, height, scale),
            solids: Vec::new(),
            raster: None,
            mode: InternalUiPresentationMode::RasterFallback,
            diagnostics: InternalUiRendererDiagnostics::default(),
        }
    }

    pub fn mode(&self) -> InternalUiPresentationMode {
        self.mode
    }

    pub fn diagnostics(&self) -> InternalUiRendererDiagnostics {
        self.diagnostics
    }

    fn supports_gpu(commands: &[PaintCommand]) -> bool {
        commands.iter().all(|command| {
            matches!(
                command,
                PaintCommand::Fill { .. }
                    | PaintCommand::OverlayFill { .. }
                    | PaintCommand::TopRoundedFill { .. }
                    | PaintCommand::RoundedFill { .. }
                    | PaintCommand::Gradient { .. }
                    | PaintCommand::Stroke { .. }
                    | PaintCommand::OverlayStroke { .. }
                    | PaintCommand::PushClip(_)
                    | PaintCommand::PopClip
            )
        })
    }

    fn push_solid(&mut self, rect: nickel_ui::Rect, color: u32, clip: nickel_ui::Rect) {
        let Some(rect) = intersect(rect, clip) else {
            return;
        };
        let size = (
            rect.size.width.ceil().max(1.0) as i32,
            rect.size.height.ceil().max(1.0) as i32,
        );
        self.solids
            .push((rect, SolidColorBuffer::new(size, color32f(color))));
    }

    fn push_rounded_solid(
        &mut self,
        rect: nickel_ui::Rect,
        color: u32,
        radius: f32,
        top_only: bool,
        clip: nickel_ui::Rect,
    ) {
        let radius = radius
            .max(0.0)
            .min(rect.size.width / 2.0)
            .min(rect.size.height / 2.0);
        if radius < 0.5 {
            self.push_solid(rect, color, clip);
            return;
        }
        // One logical-pixel strip per row gives the same pixel-centre circle
        // rule as SoftwareRenderer while keeping the result renderer-native.
        let rows = rect.size.height.ceil().max(1.0) as u32;
        for row in 0..rows {
            let y = row as f32;
            let height = (rect.size.height - y).clamp(0.0, 1.0);
            if height == 0.0 {
                continue;
            }
            let sample_y = y + height / 2.0;
            let corner_y = if sample_y < radius {
                Some(radius - sample_y)
            } else if !top_only && sample_y > rect.size.height - radius {
                Some(sample_y - (rect.size.height - radius))
            } else {
                None
            };
            let inset = corner_y
                .map(|dy| radius - (radius * radius - dy * dy).max(0.0).sqrt())
                .unwrap_or(0.0);
            self.push_solid(
                nickel_ui::Rect::new(
                    rect.origin.x + inset,
                    rect.origin.y + y,
                    (rect.size.width - inset * 2.0).max(0.0),
                    height,
                ),
                color,
                clip,
            );
        }
    }

    fn push_gradient(
        &mut self,
        rect: nickel_ui::Rect,
        gradient: LinearGradient,
        clip: nickel_ui::Rect,
    ) {
        let (steps, horizontal) = match gradient.axis {
            GradientAxis::Horizontal => (rect.size.width.ceil().max(1.0) as u32, true),
            GradientAxis::Vertical => (rect.size.height.ceil().max(1.0) as u32, false),
        };
        for step in 0..steps {
            let extent = if horizontal {
                rect.size.width
            } else {
                rect.size.height
            };
            let progress = ((step as f32 + 0.5) / extent.max(1.0)).clamp(0.0, 1.0);
            let color = interpolate_color(gradient.start, gradient.end, progress);
            let strip = if horizontal {
                nickel_ui::Rect::new(
                    rect.origin.x + step as f32,
                    rect.origin.y,
                    (rect.size.width - step as f32).min(1.0),
                    rect.size.height,
                )
            } else {
                nickel_ui::Rect::new(
                    rect.origin.x,
                    rect.origin.y + step as f32,
                    rect.size.width,
                    (rect.size.height - step as f32).min(1.0),
                )
            };
            self.push_solid(strip, color, clip);
        }
    }

    fn prepare_gpu(&mut self, frame: RenderFrame<'_>) {
        self.solids.clear();
        let viewport = nickel_ui::Rect::new(
            0.0,
            0.0,
            frame.logical_size.0 as f32,
            frame.logical_size.1 as f32,
        );
        let mut clips = vec![viewport];
        for command in frame.commands {
            let clip = *clips.last().unwrap_or(&viewport);
            match command {
                PaintCommand::Fill { rect, color } | PaintCommand::OverlayFill { rect, color } => {
                    self.push_solid(*rect, *color, clip)
                }
                PaintCommand::TopRoundedFill {
                    rect,
                    color,
                    radius,
                } => self.push_rounded_solid(*rect, *color, *radius, true, clip),
                PaintCommand::RoundedFill {
                    rect,
                    color,
                    radius,
                } => self.push_rounded_solid(*rect, *color, *radius, false, clip),
                PaintCommand::Gradient { rect, gradient } => {
                    self.push_gradient(*rect, *gradient, clip)
                }
                PaintCommand::Stroke { rect, color, width }
                | PaintCommand::OverlayStroke { rect, color, width } => {
                    let width = width.max(0.0).min(rect.size.width).min(rect.size.height);
                    self.push_solid(
                        nickel_ui::Rect::new(rect.origin.x, rect.origin.y, rect.size.width, width),
                        *color,
                        clip,
                    );
                    self.push_solid(
                        nickel_ui::Rect::new(
                            rect.origin.x,
                            rect.origin.y + rect.size.height - width,
                            rect.size.width,
                            width,
                        ),
                        *color,
                        clip,
                    );
                    self.push_solid(
                        nickel_ui::Rect::new(
                            rect.origin.x,
                            rect.origin.y + width,
                            width,
                            (rect.size.height - width * 2.0).max(0.0),
                        ),
                        *color,
                        clip,
                    );
                    self.push_solid(
                        nickel_ui::Rect::new(
                            rect.origin.x + rect.size.width - width,
                            rect.origin.y + width,
                            width,
                            (rect.size.height - width * 2.0).max(0.0),
                        ),
                        *color,
                        clip,
                    );
                }
                PaintCommand::PushClip(rect) => clips.push(
                    intersect(clip, *rect)
                        .unwrap_or_else(|| nickel_ui::Rect::new(0.0, 0.0, 0.0, 0.0)),
                ),
                PaintCommand::PopClip => {
                    if clips.len() > 1 {
                        clips.pop();
                    }
                }
                _ => unreachable!("GPU support checked before translation"),
            }
        }
        self.raster = None;
    }

    fn prepare_fallback(&mut self, frame: RenderFrame<'_>) -> DamageRegion {
        let damage = self.software.render(frame.commands);
        let mut bytes = Vec::with_capacity(self.software.pixels().len() * 4);
        for pixel in self.software.pixels() {
            bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
        }
        let (width, height) = self.software.size();
        self.raster = Some(MemoryRenderBuffer::from_slice(
            &bytes,
            Fourcc::Abgr8888,
            (width as i32, height as i32),
            1,
            Transform::Normal,
            None,
        ));
        self.solids.clear();
        damage
    }

    fn elements<R: Renderer + ImportMem>(
        &self,
        renderer: &mut R,
        location: Point<i32, Logical>,
        logical_size: (u32, u32),
    ) -> Vec<InternalUiRenderElement<R>>
    where
        R::TextureId: Send + Clone + 'static,
    {
        match self.mode {
            InternalUiPresentationMode::GpuSolid => self
                .solids
                .iter()
                .rev()
                .map(|(rect, buffer)| {
                    SolidColorRenderElement::from_buffer(
                        buffer,
                        (
                            location.x + rect.origin.x.round() as i32,
                            location.y + rect.origin.y.round() as i32,
                        ),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    )
                    .into()
                })
                .collect(),
            InternalUiPresentationMode::RasterFallback => self
                .raster
                .as_ref()
                .and_then(|buffer| {
                    MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        (f64::from(location.x), f64::from(location.y)),
                        buffer,
                        None,
                        None,
                        Some((logical_size.0 as i32, logical_size.1 as i32).into()),
                        Kind::Unspecified,
                    )
                    .ok()
                })
                .map(|element| vec![element.into()])
                .unwrap_or_default(),
        }
    }
}

impl FrameRenderer for SmithayFrameRenderer {
    type Error = std::convert::Infallible;

    fn render_frame(&mut self, frame: RenderFrame<'_>) -> Result<DamageRegion, Self::Error> {
        let damage = if Self::supports_gpu(frame.commands) {
            self.mode = InternalUiPresentationMode::GpuSolid;
            self.diagnostics.gpu_frames += 1;
            self.diagnostics.fallback_primitive_count = 0;
            self.diagnostics.fallback_text_count = 0;
            self.diagnostics.fallback_image_count = 0;
            self.prepare_gpu(frame);
            DamageRegion {
                rects: [nickel_ui::Rect::new(
                    0.0,
                    0.0,
                    frame.logical_size.0 as f32,
                    frame.logical_size.1 as f32,
                )]
                .into_iter()
                .collect(),
            }
        } else {
            self.mode = InternalUiPresentationMode::RasterFallback;
            self.diagnostics.fallback_frames += 1;
            self.diagnostics.fallback_text_count = frame
                .commands
                .iter()
                .filter(|command| {
                    matches!(
                        command,
                        PaintCommand::Text { .. } | PaintCommand::StyledText { .. }
                    )
                })
                .count();
            self.diagnostics.fallback_image_count = frame
                .commands
                .iter()
                .filter(|command| matches!(command, PaintCommand::Image { .. }))
                .count();
            self.diagnostics.fallback_primitive_count =
                self.diagnostics.fallback_text_count + self.diagnostics.fallback_image_count;
            self.prepare_fallback(frame)
        };
        Ok(damage)
    }
}

fn intersect(left: nickel_ui::Rect, right: nickel_ui::Rect) -> Option<nickel_ui::Rect> {
    let x = left.origin.x.max(right.origin.x);
    let y = left.origin.y.max(right.origin.y);
    let right_edge = (left.origin.x + left.size.width).min(right.origin.x + right.size.width);
    let bottom = (left.origin.y + left.size.height).min(right.origin.y + right.size.height);
    (right_edge > x && bottom > y).then(|| nickel_ui::Rect::new(x, y, right_edge - x, bottom - y))
}

fn color32f(color: u32) -> Color32F {
    let alpha = if color <= 0x00ff_ffff {
        0xff
    } else {
        (color >> 24) & 0xff
    };
    Color32F::new(
        ((color >> 16) & 0xff) as f32 / 255.0,
        ((color >> 8) & 0xff) as f32 / 255.0,
        (color & 0xff) as f32 / 255.0,
        alpha as f32 / 255.0,
    )
}

fn interpolate_color(start: u32, end: u32, progress: f32) -> u32 {
    let channels = |color: u32| {
        let alpha = if color <= 0x00ff_ffff {
            0xff
        } else {
            (color >> 24) & 0xff
        };
        [
            alpha,
            (color >> 16) & 0xff,
            (color >> 8) & 0xff,
            color & 0xff,
        ]
    };
    let start = channels(start);
    let end = channels(end);
    let mut result = 0;
    for (shift, channel) in [24, 16, 8, 0].into_iter().zip(0..4) {
        let value =
            start[channel] as f32 + (end[channel] as f32 - start[channel] as f32) * progress;
        result |= (value.round() as u32) << shift;
    }
    result
}

/// Session-owned applications and their compositor presentation state.
#[derive(Default)]
pub struct InternalUiRuntime {
    surfaces: InternalSurfaceSet,
    presentation: BTreeMap<InternalSurfaceId, PresentedSurface>,
    focused: Option<InternalSurfaceId>,
    hovered: Option<InternalSurfaceId>,
    touches: BTreeMap<u64, (InternalSurfaceId, UiPoint)>,
    routed_events: Vec<(InternalSurfaceId, UiEvent)>,
}

impl InternalUiRuntime {
    pub fn insert_boxed(
        &mut self,
        surface: Box<dyn nickel_ui::InternalUiSurface>,
        placement: InternalSurfacePlacement,
        scale: f32,
    ) -> InternalSurfaceId {
        let (_, _, width, height) = placement.geometry;
        let id = self.surfaces.insert_boxed(surface);
        self.presentation.insert(
            id,
            PresentedSurface {
                placement,
                renderer: SmithayFrameRenderer::new(width, height, scale),
                dirty: true,
                external_scene: None,
                scale_factor: scale,
            },
        );
        id
    }

    pub fn application<T: 'static>(&self, id: InternalSurfaceId) -> Option<&T> {
        self.surfaces.get(id)?.application().downcast_ref()
    }

    pub fn update_scene(&mut self, id: InternalSurfaceId, commands: Vec<PaintCommand>) -> bool {
        let Some(surface) = self.presentation.get_mut(&id) else {
            return false;
        };
        surface.external_scene = Some(commands);
        surface.dirty = true;
        true
    }

    pub fn focus_surface(&mut self, id: InternalSurfaceId) -> bool {
        if !self.presentation.contains_key(&id) {
            return false;
        }
        if self.focused != Some(id) {
            if let Some(previous) = self.focused {
                self.step(
                    previous,
                    HostBatch {
                        window_focused: Some(false),
                        ..Default::default()
                    },
                );
            }
            self.focused = Some(id);
            self.step(
                id,
                HostBatch {
                    window_focused: Some(true),
                    ..Default::default()
                },
            );
        }
        true
    }
    pub fn insert<A: Application + 'static>(
        &mut self,
        application: A,
        placement: InternalSurfacePlacement,
        scale: f32,
    ) -> InternalSurfaceId {
        let (_, _, width, height) = placement.geometry;
        let id = self.surfaces.insert(application, width, height);
        let physical_width = ((width as f32) * scale).round().max(1.0) as u32;
        let physical_height = ((height as f32) * scale).round().max(1.0) as u32;
        self.presentation.insert(
            id,
            PresentedSurface {
                placement,
                renderer: SmithayFrameRenderer::new(physical_width, physical_height, scale),
                dirty: true,
                external_scene: None,
                scale_factor: scale,
            },
        );
        id
    }

    /// Insert a renderer-neutral scene produced by compositor-owned shell state.
    pub fn insert_scene(
        &mut self,
        commands: Vec<PaintCommand>,
        placement: InternalSurfacePlacement,
        scale: f32,
    ) -> InternalSurfaceId {
        let id = self.insert(SceneSlot, placement, scale);
        self.presentation.get_mut(&id).unwrap().external_scene = Some(commands);
        id
    }

    pub fn remove(&mut self, id: InternalSurfaceId) -> bool {
        let removed = self.surfaces.remove(id).is_some();
        self.presentation.remove(&id);
        if self.focused == Some(id) {
            self.focused = None;
        }
        if self.hovered == Some(id) {
            self.hovered = None;
        }
        self.touches.retain(|_, (target, _)| *target != id);
        removed
    }

    pub fn placement(&self, id: InternalSurfaceId) -> Option<&InternalSurfacePlacement> {
        self.presentation.get(&id).map(|surface| &surface.placement)
    }

    pub fn step(&mut self, id: InternalSurfaceId, batch: HostBatch) -> bool {
        let Some(surface) = self.surfaces.get_mut(id) else {
            return false;
        };
        let changed = surface.step(batch).changed;
        if changed && let Some(presentation) = self.presentation.get_mut(&id) {
            presentation.dirty = true;
        }
        changed
    }

    pub fn update_scene(&mut self, id: InternalSurfaceId, commands: Vec<PaintCommand>) -> bool {
        let Some(surface) = self.presentation.get_mut(&id) else {
            return false;
        };
        surface.external_scene = Some(commands);
        surface.dirty = true;
        true
    }

    pub fn drain_routed_events(&mut self) -> Vec<(InternalSurfaceId, UiEvent)> {
        std::mem::take(&mut self.routed_events)
    }

    pub fn mark_dirty(&mut self, id: InternalSurfaceId) {
        if let Some(surface) = self.presentation.get_mut(&id) {
            surface.dirty = true;
        }
    }

    pub fn presentation_mode(&self, id: InternalSurfaceId) -> Option<InternalUiPresentationMode> {
        self.presentation
            .get(&id)
            .map(|surface| surface.renderer.mode())
    }

    pub fn renderer_diagnostics(
        &self,
        id: InternalSurfaceId,
    ) -> Option<InternalUiRendererDiagnostics> {
        self.presentation
            .get(&id)
            .map(|surface| surface.renderer.diagnostics())
    }

    pub fn has_damage(&self) -> bool {
        self.presentation.values().any(|surface| surface.dirty)
    }

    pub fn focused(&self) -> Option<InternalSurfaceId> {
        self.focused
    }

    fn role_order(role: InternalSurfaceRole) -> u8 {
        match role {
            InternalSurfaceRole::Desktop => 0,
            InternalSurfaceRole::Application => 1,
            InternalSurfaceRole::Panel => 2,
            InternalSurfaceRole::Overlay => 3,
        }
    }

    pub fn surface_at(&self, point: (f64, f64)) -> Option<(InternalSurfaceId, UiPoint)> {
        self.presentation
            .iter()
            .filter_map(|(id, surface)| {
                let (x, y, width, height) = surface.placement.geometry;
                (point.0 >= f64::from(x)
                    && point.1 >= f64::from(y)
                    && point.0 < f64::from(x) + f64::from(width)
                    && point.1 < f64::from(y) + f64::from(height))
                .then_some((
                    Self::role_order(surface.placement.role),
                    *id,
                    UiPoint {
                        x: (point.0 - f64::from(x)) as f32,
                        y: (point.1 - f64::from(y)) as f32,
                    },
                ))
            })
            .max_by_key(|(role, id, _)| (*role, *id))
            .map(|(_, id, local)| (id, local))
    }

    fn dispatch_ui(&mut self, id: InternalSurfaceId, event: UiEvent) -> bool {
        self.routed_events.push((id, event.clone()));
        if self
            .presentation
            .get(&id)
            .is_some_and(|surface| surface.external_scene.is_some())
        {
            // Renderer-neutral scenes are owned by the shell coordinator. It
            // consumes this event after routing and supplies the next scene;
            // the identity-only SceneSlot must never reduce input itself.
            return true;
        }
        self.step(
            id,
            HostBatch {
                events: vec![HostEvent::Ui(event)],
                ..Default::default()
            },
        )
    }

    pub fn pointer_motion(&mut self, point: (f64, f64)) -> bool {
        let target = self.surface_at(point);
        if self.hovered != target.map(|(id, _)| id) {
            if let Some(previous) = self.hovered {
                self.dispatch_ui(previous, UiEvent::PointerCancelled);
            }
            self.hovered = target.map(|(id, _)| id);
        }
        if let Some((id, local)) = target {
            self.dispatch_ui(id, UiEvent::PointerMoved(local));
            true
        } else {
            false
        }
    }

    pub fn pointer_button(&mut self, point: (f64, f64), pressed: bool) -> bool {
        let Some((id, local)) = self.surface_at(point) else {
            if pressed && let Some(previous) = self.focused.take() {
                self.step(
                    previous,
                    HostBatch {
                        window_focused: Some(false),
                        ..Default::default()
                    },
                );
            }
            return false;
        };
        if pressed {
            if self.focused != Some(id) {
                if let Some(previous) = self.focused {
                    self.step(
                        previous,
                        HostBatch {
                            window_focused: Some(false),
                            ..Default::default()
                        },
                    );
                }
                self.focused = Some(id);
                self.step(
                    id,
                    HostBatch {
                        window_focused: Some(true),
                        ..Default::default()
                    },
                );
            }
            self.dispatch_ui(id, UiEvent::PointerPressed(local));
        } else {
            self.dispatch_ui(id, UiEvent::PointerReleased(local));
        }
        true
    }

    pub fn scroll(&mut self, point: (f64, f64), horizontal: f32, vertical: f32) -> bool {
        let Some((id, local)) = self.surface_at(point) else {
            return false;
        };
        if horizontal != 0.0 {
            self.dispatch_ui(
                id,
                UiEvent::ScrollHorizontal {
                    point: local,
                    delta_x: horizontal,
                },
            );
        }
        if vertical != 0.0 {
            self.dispatch_ui(
                id,
                UiEvent::Scroll {
                    point: local,
                    delta_y: vertical,
                },
            );
        }
        true
    }

    pub fn touch(&mut self, contact: u64, point: (f64, f64), phase: TouchPhase) -> bool {
        match phase {
            TouchPhase::Started => {
                let Some((id, local)) = self.surface_at(point) else {
                    return false;
                };
                self.touches.insert(contact, (id, local));
                if self.focused != Some(id) {
                    if let Some(previous) = self.focused {
                        self.step(
                            previous,
                            HostBatch {
                                window_focused: Some(false),
                                ..Default::default()
                            },
                        );
                    }
                    self.focused = Some(id);
                    self.step(
                        id,
                        HostBatch {
                            window_focused: Some(true),
                            ..Default::default()
                        },
                    );
                }
                self.dispatch_ui(id, UiEvent::PointerPressed(local));
                true
            }
            TouchPhase::Moved => self.touches.get(&contact).copied().is_some_and(|(id, _)| {
                let Some(surface) = self.presentation.get(&id) else {
                    return false;
                };
                let (x, y, _, _) = surface.placement.geometry;
                let local = UiPoint {
                    x: (point.0 - f64::from(x)) as f32,
                    y: (point.1 - f64::from(y)) as f32,
                };
                self.touches.insert(contact, (id, local));
                self.dispatch_ui(id, UiEvent::PointerMoved(local));
                true
            }),
            TouchPhase::Ended => self.touches.remove(&contact).is_some_and(|(id, local)| {
                self.dispatch_ui(id, UiEvent::PointerReleased(local));
                true
            }),
            TouchPhase::Cancelled => self
                .touches
                .remove(&contact)
                .is_some_and(|(id, _)| self.dispatch_ui(id, UiEvent::PointerCancelled)),
        }
    }

    pub fn keyboard(&mut self, event: UiEvent) -> bool {
        self.focused.is_some_and(|id| {
            self.dispatch_ui(id, event);
            true
        })
    }

    pub fn cancel_touches(&mut self) -> bool {
        let targets = std::mem::take(&mut self.touches);
        let mut handled = false;
        for (_, (id, _)) in targets {
            handled |= self.dispatch_ui(id, UiEvent::PointerCancelled);
        }
        handled
    }

    pub fn ids_for_output<'a>(
        &'a self,
        output: &'a str,
    ) -> impl Iterator<Item = InternalSurfaceId> + 'a {
        self.presentation.iter().filter_map(move |(id, surface)| {
            surface
                .placement
                .output
                .as_deref()
                .is_none_or(|name| name == output)
                .then_some(*id)
        })
    }

    /// Prepare a dirty surface and return its Smithay-importable fallback buffer.
    ///
    /// `None` after preparation means the frame is represented by GPU-native
    /// solid elements and should be obtained through [`Self::render_elements`].
    pub fn render_buffer(&mut self, id: InternalSurfaceId) -> Option<MemoryRenderBuffer> {
        let presentation = self.presentation.get_mut(&id)?;
        if presentation.dirty {
            if let Some(commands) = &presentation.external_scene {
                let (_, _, width, height) = presentation.placement.geometry;
                let _ = presentation.renderer.render_frame(RenderFrame {
                    commands,
                    logical_size: (width, height),
                    scale_factor: presentation.scale_factor,
                    generation: 0,
                });
            } else {
                let surface = self.surfaces.get(id)?;
                let _ = presentation.renderer.render_frame(surface.render_frame());
            }
            presentation.dirty = false;
        }
        presentation.renderer.raster.clone()
    }

    /// Build render elements in output-local coordinates for the backend's current renderer.
    pub fn render_elements<R: Renderer + ImportMem>(
        &mut self,
        renderer: &mut R,
        output: &str,
        output_origin: Point<i32, Logical>,
    ) -> Vec<InternalUiRenderElement<R>>
    where
        R::TextureId: Send + Clone + 'static,
    {
        let ids = self.ids_for_output(output).collect::<Vec<_>>();
        ids.into_iter()
            .filter_map(|id| {
                let placement = self.presentation.get(&id)?.placement.clone();
                if self.presentation.get(&id)?.dirty {
                    let presentation = self.presentation.get_mut(&id)?;
                    if let Some(commands) = &presentation.external_scene {
                        let (_, _, width, height) = placement.geometry;
                        let _ = presentation.renderer.render_frame(RenderFrame {
                            commands,
                            logical_size: (width, height),
                            scale_factor: presentation.scale_factor,
                            generation: 0,
                        });
                    } else {
                        let surface = self.surfaces.get(id)?;
                        let _ = presentation.renderer.render_frame(surface.render_frame());
                    }
                    presentation.dirty = false;
                }
                let presentation = self.presentation.get(&id)?;
                let local = output_local_location(placement.geometry, output_origin);
                Some(presentation.renderer.elements(
                    renderer,
                    (local.0.round() as i32, local.1.round() as i32).into(),
                    (placement.geometry.2, placement.geometry.3),
                ))
            })
            .flatten()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.presentation.len()
    }
    pub fn is_empty(&self) -> bool {
        self.presentation.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TouchPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_ui::{Button, Text, View, ViewContext};

    struct Label;
    impl Application for Label {
        type Message = ();
        fn update(&mut self, (): ()) {}
        fn view(&self, _: ViewContext) -> impl View<Self::Message> {
            Text::new("owned")
        }
    }

    fn placement(output: Option<&str>) -> InternalSurfacePlacement {
        InternalSurfacePlacement {
            role: InternalSurfaceRole::Panel,
            geometry: (10, 20, 120, 32),
            output: output.map(str::to_owned),
        }
    }

    #[test]
    fn lifecycle_keeps_application_and_presentation_identity_together() {
        let mut runtime = InternalUiRuntime::default();
        let id = runtime.insert(Label, placement(Some("DP-1")), 1.0);
        assert_eq!(runtime.ids_for_output("DP-1").collect::<Vec<_>>(), vec![id]);
        assert!(runtime.ids_for_output("HDMI-A-1").next().is_none());
        assert!(runtime.has_damage());
        assert!(runtime.render_buffer(id).is_some());
        assert!(!runtime.has_damage());
        assert!(runtime.remove(id));
        assert!(runtime.is_empty());
    }

    #[test]
    fn output_agnostic_surfaces_are_visible_on_every_output() {
        let mut runtime = InternalUiRuntime::default();
        let id = runtime.insert(Label, placement(None), 1.0);
        assert_eq!(runtime.ids_for_output("DP-1").next(), Some(id));
        assert_eq!(runtime.ids_for_output("HDMI-A-1").next(), Some(id));
    }

    #[test]
    fn global_placement_is_translated_to_output_local_logical_coordinates() {
        let origin = Point::<i32, Logical>::from((1920, -120));

        assert_eq!(
            output_local_location((1936, -88, 480, 64), origin),
            (16.0, 32.0)
        );
    }

    struct Counter(usize);
    impl Application for Counter {
        type Message = ();
        fn update(&mut self, (): ()) {
            self.0 += 1;
        }
        fn view(&self, _: ViewContext) -> impl View<Self::Message> {
            Button::new((), "count")
        }
    }

    #[test]
    fn hit_testing_respects_system_role_stack_and_focus_routes_keyboard() {
        let mut runtime = InternalUiRuntime::default();
        let application = runtime.insert(
            Counter(0),
            InternalSurfacePlacement {
                role: InternalSurfaceRole::Application,
                geometry: (0, 0, 100, 100),
                output: None,
            },
            1.0,
        );
        let panel = runtime.insert(
            Counter(0),
            InternalSurfacePlacement {
                role: InternalSurfaceRole::Panel,
                geometry: (0, 0, 100, 40),
                output: None,
            },
            1.0,
        );

        assert_eq!(
            runtime.surface_at((10.0, 10.0)).map(|hit| hit.0),
            Some(panel)
        );
        assert!(runtime.pointer_button((10.0, 60.0), true));
        assert_eq!(runtime.focused(), Some(application));
        assert!(runtime.keyboard(UiEvent::KeyboardActivate));
        assert_eq!(
            runtime
                .surfaces
                .get(application)
                .unwrap()
                .application()
                .downcast_ref::<Counter>()
                .unwrap()
                .0,
            1
        );
        assert_eq!(
            runtime
                .surfaces
                .get(panel)
                .unwrap()
                .application()
                .downcast_ref::<Counter>()
                .unwrap()
                .0,
            0
        );
    }

    #[test]
    fn renderer_neutral_scene_uses_the_same_presentation_lifecycle() {
        let mut runtime = InternalUiRuntime::default();
        let id = runtime.insert_scene(Vec::new(), placement(Some("nested")), 1.0);
        assert!(runtime.has_damage());
        assert!(runtime.render_buffer(id).is_none());
        assert_eq!(
            runtime.presentation_mode(id),
            Some(InternalUiPresentationMode::GpuSolid)
        );
        assert!(!runtime.has_damage());
    }

    #[test]
    fn input_outside_internal_surfaces_is_left_for_wayland_clients() {
        let mut runtime = InternalUiRuntime::default();
        runtime.insert(Label, placement(Some("DP-1")), 1.0);
        assert!(!runtime.pointer_motion((500.0, 500.0)));
        assert!(!runtime.pointer_button((500.0, 500.0), true));
        assert!(!runtime.scroll((500.0, 500.0), 0.0, 1.0));
    }

    #[test]
    fn rectangular_display_list_selects_gpu_solids_and_honors_clip() {
        let commands = [
            PaintCommand::PushClip(nickel_ui::Rect::new(5.0, 4.0, 20.0, 10.0)),
            PaintCommand::Fill {
                rect: nickel_ui::Rect::new(0.0, 0.0, 40.0, 30.0),
                color: 0xff336699,
            },
            PaintCommand::PopClip,
        ];
        let mut renderer = SmithayFrameRenderer::new(40, 30, 1.0);

        renderer
            .render_frame(RenderFrame {
                commands: &commands,
                logical_size: (40, 30),
                scale_factor: 1.0,
                generation: 1,
            })
            .unwrap();

        assert_eq!(renderer.mode(), InternalUiPresentationMode::GpuSolid);
        assert_eq!(renderer.solids.len(), 1);
        assert_eq!(
            renderer.solids[0].0,
            nickel_ui::Rect::new(5.0, 4.0, 20.0, 10.0)
        );
        assert!(renderer.raster.is_none());
        assert_eq!(renderer.diagnostics().gpu_frames, 1);
    }

    #[test]
    fn rounded_fill_stays_on_gpu_as_scanline_solids() {
        let commands = [PaintCommand::RoundedFill {
            rect: nickel_ui::Rect::new(0.0, 0.0, 20.0, 12.0),
            color: 0x336699,
            radius: 4.0,
        }];
        let mut renderer = SmithayFrameRenderer::new(20, 12, 1.0);

        renderer
            .render_frame(RenderFrame {
                commands: &commands,
                logical_size: (20, 12),
                scale_factor: 1.0,
                generation: 1,
            })
            .unwrap();

        assert_eq!(renderer.mode(), InternalUiPresentationMode::GpuSolid);
        assert!(renderer.raster.is_none());
        assert_eq!(renderer.solids.len(), 12);
        assert!(renderer.solids[0].0.size.width < 20.0);
        assert_eq!(renderer.solids[6].0.size.width, 20.0);
    }

    #[test]
    fn text_fallback_reports_the_remaining_primitive_cause() {
        let commands = [PaintCommand::Text {
            bounds: nickel_ui::Rect::new(0.0, 0.0, 20.0, 12.0),
            text: "Nickel".into(),
            scale: 1.0,
            color: 0x336699,
            align: nickel_ui::TextAlign::Start,
            bold: false,
            wrap: false,
        }];
        let mut renderer = SmithayFrameRenderer::new(20, 12, 1.0);

        renderer
            .render_frame(RenderFrame {
                commands: &commands,
                logical_size: (20, 12),
                scale_factor: 1.0,
                generation: 1,
            })
            .unwrap();

        assert_eq!(renderer.mode(), InternalUiPresentationMode::RasterFallback);
        assert_eq!(renderer.diagnostics().fallback_text_count, 1);
        assert_eq!(renderer.diagnostics().fallback_image_count, 0);
    }
}
