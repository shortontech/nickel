//! Compositor ownership for Nickel UI applications which do not have a Wayland surface.

use std::collections::BTreeMap;

use nickel_ui::{Application, HostBatch, InternalSurfaceId, InternalSurfaceSet, SoftwareRenderer};
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
}

/// Session-owned applications and their compositor presentation state.
#[derive(Default)]
pub struct InternalUiRuntime {
    surfaces: InternalSurfaceSet,
    presentation: BTreeMap<InternalSurfaceId, PresentedSurface>,
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
            },
        );
        id
    }

    pub fn remove(&mut self, id: InternalSurfaceId) -> bool {
        let removed = self.surfaces.remove(id).is_some();
        self.presentation.remove(&id);
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
            let surface = self.surfaces.get(id)?;
            let damage = surface.render_software(&mut presentation.renderer);
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
                    (
                        f64::from(placement.geometry.0 - output_origin.x),
                        f64::from(placement.geometry.1 - output_origin.y),
                    ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_ui::{Text, View, ViewContext};

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
}
