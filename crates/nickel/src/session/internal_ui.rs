//! Compositor ownership for Nickel UI applications which do not have a Wayland surface.

use std::collections::BTreeMap;

use nickel_ui::{
    Application, HostBatch, HostEvent, InternalSurfaceId, InternalSurfaceSet, Point as UiPoint,
    SoftwareRenderer, Text, UiEvent, View, ViewContext, backend::PaintCommand,
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ImportMem, Renderer,
            element::{Kind, memory::MemoryRenderBuffer, memory::MemoryRenderBufferRenderElement},
        },
    },
    utils::{Logical, Point, Transform},
};

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
    renderer: SoftwareRenderer,
    buffer: Option<MemoryRenderBuffer>,
    dirty: bool,
    external_scene: Option<Vec<PaintCommand>>,
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

/// Session-owned applications and their compositor presentation state.
#[derive(Default)]
pub struct InternalUiRuntime {
    surfaces: InternalSurfaceSet,
    presentation: BTreeMap<InternalSurfaceId, PresentedSurface>,
    focused: Option<InternalSurfaceId>,
    hovered: Option<InternalSurfaceId>,
    touches: BTreeMap<u64, (InternalSurfaceId, UiPoint)>,
}

impl InternalUiRuntime {
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
                renderer: SoftwareRenderer::new(physical_width, physical_height, scale),
                buffer: None,
                dirty: true,
                external_scene: None,
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

    pub fn mark_dirty(&mut self, id: InternalSurfaceId) {
        if let Some(surface) = self.presentation.get_mut(&id) {
            surface.dirty = true;
        }
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

    /// Rasterize a dirty surface and expose it as a Smithay-importable memory buffer.
    pub fn render_buffer(&mut self, id: InternalSurfaceId) -> Option<MemoryRenderBuffer> {
        let presentation = self.presentation.get_mut(&id)?;
        if presentation.dirty {
            let damage = if let Some(commands) = &presentation.external_scene {
                presentation.renderer.render(commands)
            } else {
                self.surfaces
                    .get(id)?
                    .render_software(&mut presentation.renderer)
            };
            if !damage.is_empty() || presentation.buffer.is_none() {
                let mut bytes = Vec::with_capacity(presentation.renderer.pixels().len() * 4);
                for pixel in presentation.renderer.pixels() {
                    bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
                }
                let (width, height) = presentation.renderer.size();
                presentation.buffer = Some(MemoryRenderBuffer::from_slice(
                    &bytes,
                    Fourcc::Abgr8888,
                    (width as i32, height as i32),
                    1,
                    Transform::Normal,
                    None,
                ));
            }
            presentation.dirty = false;
        }
        presentation.buffer.clone()
    }

    /// Build render elements in output-local coordinates for the backend's current renderer.
    pub fn render_elements<R: Renderer + ImportMem>(
        &mut self,
        renderer: &mut R,
        output: &str,
        output_origin: Point<i32, Logical>,
    ) -> Vec<MemoryRenderBufferRenderElement<R>>
    where
        R::TextureId: Send + Clone + 'static,
    {
        let ids = self.ids_for_output(output).collect::<Vec<_>>();
        ids.into_iter()
            .filter_map(|id| {
                let placement = self.presentation.get(&id)?.placement.clone();
                let buffer = self.render_buffer(id)?;
                MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    output_local_location(placement.geometry, output_origin),
                    &buffer,
                    None,
                    None,
                    Some((placement.geometry.2 as i32, placement.geometry.3 as i32).into()),
                    Kind::Unspecified,
                )
                .ok()
            })
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
        assert!(runtime.render_buffer(id).is_some());
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
}
