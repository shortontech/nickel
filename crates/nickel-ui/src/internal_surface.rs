//! Event-loop-free hosting for compositor-owned Nickel UI surfaces.
//!
//! [`UiHost`] remains the canonical application reducer. This module only
//! erases its application and message types so a compositor can own unrelated
//! applications in one collection without giving any of them a native window
//! or a `winit` event loop.

use std::{
    any::Any,
    collections::BTreeMap,
    fmt,
    time::{Duration, Instant},
};

use crate::{
    ControllerFamily, DamageRegion, HostBatch, HostEventOutcome, HostInspection,
    SemanticNodeSnapshot, SoftwareRenderer, UiHost,
};

/// Stable, process-local identity assigned by an [`InternalSurfaceSet`].
///
/// The id is deliberately opaque: application code cannot choose or infer a
/// compositor surface identity from a title, role, or native protocol object.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InternalSurfaceId(u64);

impl InternalSurfaceId {
    /// Read-only identity for cross-crate snapshots, not a native window handle.
    /// There is deliberately no inverse constructor: only the owning surface
    /// set allocates identities, and consumers cannot use this token to mint one.
    pub fn snapshot_token(self) -> u64 {
        self.0
    }
}

impl fmt::Display for InternalSurfaceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A type-erased, compositor-owned Nickel UI application.
///
/// This interface contains no native window handles and does not create or
/// pump an event loop. The owner supplies input, time, size, and scale through
/// [`HostBatch`] and chooses when and where to present the resulting pixels.
pub trait InternalUiSurface {
    fn title(&self) -> &str;
    fn logical_size(&self) -> (u32, u32);
    fn scale_factor(&self) -> f32;
    fn next_deadline(&self) -> Option<Instant>;
    fn step(&mut self, batch: HostBatch) -> HostEventOutcome;
    fn inspect(&self) -> HostInspection;
    fn semantic_nodes(&self) -> Vec<SemanticNodeSnapshot>;
    fn render_frame(&self) -> crate::backend::RenderFrame<'_>;
    fn render_software(&self, renderer: &mut SoftwareRenderer) -> DamageRegion;
    fn paste_clipboard_image(&mut self, width: u32, height: u32, rgba: &[u8]) -> bool;
    fn set_controller_family(&mut self, family: ControllerFamily) -> bool;
    fn application(&self) -> &dyn Any;
    fn application_mut(&mut self) -> &mut dyn Any;
}

/// Event-loop-free, type-erased wrapper around one [`crate::Application`].
pub struct HostedApplication<A: crate::Application + 'static> {
    host: UiHost<A>,
    logical_size: (u32, u32),
    scale_factor: f32,
}

impl<A: crate::Application + 'static> HostedApplication<A> {
    pub fn new(application: A, width: u32, height: u32) -> Self {
        Self {
            host: UiHost::new(application, width, height),
            logical_size: (width, height),
            scale_factor: 1.0,
        }
    }

    pub fn host(&self) -> &UiHost<A> {
        &self.host
    }

    pub fn host_mut(&mut self) -> &mut UiHost<A> {
        &mut self.host
    }
}

impl<A: crate::Application + 'static> InternalUiSurface for HostedApplication<A> {
    fn title(&self) -> &str {
        self.host.application().title()
    }

    fn logical_size(&self) -> (u32, u32) {
        self.logical_size
    }

    fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.host.next_deadline()
    }

    fn step(&mut self, batch: HostBatch) -> HostEventOutcome {
        if let Some(size) = batch.surface_size {
            self.logical_size = size;
        }
        if let Some(scale) = batch.scale_factor
            && scale.is_finite()
            && scale > 0.0
        {
            self.scale_factor = scale;
        }
        self.host.step(batch)
    }

    fn inspect(&self) -> HostInspection {
        self.host.inspect()
    }

    fn semantic_nodes(&self) -> Vec<SemanticNodeSnapshot> {
        self.host.semantic_nodes()
    }

    fn render_frame(&self) -> crate::backend::RenderFrame<'_> {
        self.host.render_frame()
    }

    fn render_software(&self, renderer: &mut SoftwareRenderer) -> DamageRegion {
        self.host.render_software(renderer)
    }

    fn paste_clipboard_image(&mut self, width: u32, height: u32, rgba: &[u8]) -> bool {
        self.host.paste_clipboard_image(width, height, rgba)
    }

    fn set_controller_family(&mut self, family: ControllerFamily) -> bool {
        self.host.set_controller_family(family)
    }

    fn application(&self) -> &dyn Any {
        self.host.application()
    }

    fn application_mut(&mut self) -> &mut dyn Any {
        self.host.application_mut()
    }
}

/// Heterogeneous collection of applications owned by one compositor runtime.
///
/// The collection is intentionally not `Send`: the compositor decides which
/// thread owns UI state, while individual applications remain free to contain
/// thread-affine resources.
#[derive(Default)]
pub struct InternalSurfaceSet {
    next_id: u64,
    surfaces: BTreeMap<InternalSurfaceId, Box<dyn InternalUiSurface>>,
}

impl InternalSurfaceSet {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            surfaces: BTreeMap::new(),
        }
    }

    pub fn insert<A: crate::Application + 'static>(
        &mut self,
        application: A,
        width: u32,
        height: u32,
    ) -> InternalSurfaceId {
        self.insert_hosted(HostedApplication::new(application, width, height))
    }

    pub fn insert_hosted(
        &mut self,
        surface: impl InternalUiSurface + 'static,
    ) -> InternalSurfaceId {
        let id = InternalSurfaceId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("internal surface ids exhausted");
        self.surfaces.insert(id, Box::new(surface));
        id
    }

    pub fn insert_boxed(&mut self, surface: Box<dyn InternalUiSurface>) -> InternalSurfaceId {
        let id = InternalSurfaceId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("internal surface ids exhausted");
        self.surfaces.insert(id, surface);
        id
    }

    pub fn remove(&mut self, id: InternalSurfaceId) -> Option<Box<dyn InternalUiSurface>> {
        self.surfaces.remove(&id)
    }

    pub fn get(&self, id: InternalSurfaceId) -> Option<&dyn InternalUiSurface> {
        self.surfaces.get(&id).map(Box::as_ref)
    }

    pub fn get_mut(&mut self, id: InternalSurfaceId) -> Option<&mut (dyn InternalUiSurface + '_)> {
        match self.surfaces.get_mut(&id) {
            Some(surface) => Some(surface.as_mut()),
            None => None,
        }
    }

    pub fn ids(&self) -> impl Iterator<Item = InternalSurfaceId> + '_ {
        self.surfaces.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.surfaces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }

    /// Earliest requested application or interaction wakeup across all surfaces.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.surfaces
            .values()
            .filter_map(|surface| surface.next_deadline())
            .min()
    }

    /// Produces a timeout suitable for an externally owned event loop.
    pub fn timeout_from(&self, now: Instant) -> Option<Duration> {
        self.next_deadline()
            .map(|deadline| deadline.saturating_duration_since(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionKind, Button, HostEvent, SemanticAction, Text, View, ViewContext};

    struct Counter {
        count: usize,
    }

    impl crate::Application for Counter {
        type Message = ();

        fn update(&mut self, (): ()) {
            self.count += 1;
        }

        fn view(&self, _context: ViewContext) -> impl View<Self::Message> {
            Button::new((), format!("Count {}", self.count))
        }

        fn title(&self) -> &str {
            "Counter"
        }
    }

    struct Caption(&'static str);

    impl crate::Application for Caption {
        type Message = bool;

        fn update(&mut self, message: bool) {
            if message {
                self.0 = "Changed";
            }
        }

        fn view(&self, _context: ViewContext) -> impl View<Self::Message> {
            Text::new(self.0)
        }
    }

    #[test]
    fn set_hosts_unrelated_application_types_without_an_event_loop() {
        let mut surfaces = InternalSurfaceSet::new();
        let counter = surfaces.insert(Counter { count: 0 }, 320, 200);
        let caption = surfaces.insert(Caption("Initial"), 640, 480);

        assert_ne!(counter, caption);
        assert_eq!(surfaces.ids().collect::<Vec<_>>(), vec![counter, caption]);
        assert_eq!(surfaces.get(counter).unwrap().title(), "Counter");
        assert_eq!(surfaces.get(caption).unwrap().logical_size(), (640, 480));

        let target = surfaces
            .get(counter)
            .unwrap()
            .semantic_nodes()
            .into_iter()
            .find(|node| node.name.as_deref() == Some("Count 0"))
            .unwrap()
            .id;
        let outcome = surfaces.get_mut(counter).unwrap().step(HostBatch {
            events: vec![HostEvent::Semantic {
                target,
                action: SemanticAction::Invoke(ActionKind::Activate),
            }],
            ..HostBatch::default()
        });

        assert!(outcome.changed);
        assert_eq!(
            surfaces
                .get(counter)
                .unwrap()
                .application()
                .downcast_ref::<Counter>()
                .unwrap()
                .count,
            1
        );
        assert!(surfaces.get(caption).unwrap().application().is::<Caption>());
    }

    #[test]
    fn compositor_controls_size_scale_and_rasterization() {
        let mut surface = HostedApplication::new(Caption("Internal"), 100, 50);
        let outcome = surface.step(HostBatch {
            surface_size: Some((200, 80)),
            scale_factor: Some(2.0),
            ..HostBatch::default()
        });
        let mut renderer = SoftwareRenderer::new(400, 160, 2.0);
        let damage = surface.render_software(&mut renderer);

        assert!(outcome.changed);
        assert_eq!(surface.logical_size(), (200, 80));
        assert_eq!(surface.scale_factor(), 2.0);
        assert!(!damage.is_empty());
    }

    #[test]
    fn collection_reports_earliest_deadline() {
        struct Polling;
        impl crate::Application for Polling {
            type Message = ();
            fn update(&mut self, (): ()) {}
            fn view(&self, _context: ViewContext) -> impl View<Self::Message> {
                Text::new("Polling")
            }
            fn poll_interval(&self) -> Option<Duration> {
                Some(Duration::from_millis(20))
            }
        }

        let mut surfaces = InternalSurfaceSet::new();
        surfaces.insert(Polling, 10, 10);
        let deadline = surfaces.next_deadline().unwrap();
        assert_eq!(surfaces.timeout_from(deadline), Some(Duration::ZERO));
    }
}
