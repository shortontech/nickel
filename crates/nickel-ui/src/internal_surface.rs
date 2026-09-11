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
    AccessibilityNode, ControllerFamily, DamageRegion, HostBatch, HostEventOutcome, HostInspection,
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
    /// Custom hosts must explicitly supply their protection state before an
    /// adapter can admit them for remote access.
    fn remote_access_protected(&self) -> bool {
        true
    }
    /// None means this host does not expose a resolved-tree generation.
    fn resolved_frame_generation(&self) -> Option<u64> {
        None
    }
    /// Custom hosts must report local pointer ownership before remote input.
    fn pointer_interaction_active(&self) -> bool {
        true
    }
    fn title(&self) -> &str;
    fn logical_size(&self) -> (u32, u32);
    fn scale_factor(&self) -> f32;
    fn next_deadline(&self) -> Option<Instant>;
    fn step(&mut self, batch: HostBatch) -> HostEventOutcome;
    fn inspect(&self) -> HostInspection;
    fn semantic_nodes(&self) -> Vec<SemanticNodeSnapshot>;
    /// Local assistive-technology projection from the canonical resolved tree.
    /// Remote adapters remain on the separately bounded semantic interface.
    fn accessibility_nodes(&self) -> Vec<AccessibilityNode>;
    fn bounded_semantic_nodes(
        &self,
        _max_nodes: usize,
        _max_bytes: usize,
    ) -> Result<Vec<SemanticNodeSnapshot>, crate::BoundedSemanticError> {
        Err(crate::BoundedSemanticError::ProtectedSurface)
    }

    fn perform_bounded_semantic_action(
        &mut self,
        _expected_generation: u64,
        _ordinal: usize,
        _action: crate::SemanticAction,
        _max_nodes: usize,
        _max_bytes: usize,
        _clipboard_text_limit: Option<usize>,
    ) -> Result<HostEventOutcome, crate::BoundedSemanticActionError> {
        Err(crate::BoundedSemanticActionError::Snapshot(
            crate::BoundedSemanticError::ProtectedSurface,
        ))
    }

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
    fn remote_access_protected(&self) -> bool {
        self.host.remote_access_protected()
    }

    fn resolved_frame_generation(&self) -> Option<u64> {
        Some(self.host.resolved_frame_generation())
    }

    fn pointer_interaction_active(&self) -> bool {
        self.host.pointer_interaction_active()
    }

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

    fn accessibility_nodes(&self) -> Vec<AccessibilityNode> {
        self.host.accessibility_nodes().to_vec()
    }

    fn bounded_semantic_nodes(
        &self,
        max_nodes: usize,
        max_bytes: usize,
    ) -> Result<Vec<SemanticNodeSnapshot>, crate::BoundedSemanticError> {
        self.host.bounded_semantic_nodes(max_nodes, max_bytes)
    }

    fn perform_bounded_semantic_action(
        &mut self,
        expected_generation: u64,
        ordinal: usize,
        action: crate::SemanticAction,
        max_nodes: usize,
        max_bytes: usize,
        clipboard_text_limit: Option<usize>,
    ) -> Result<HostEventOutcome, crate::BoundedSemanticActionError> {
        self.host.perform_bounded_semantic_action(
            expected_generation,
            ordinal,
            action,
            max_nodes,
            max_bytes,
            clipboard_text_limit,
        )
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

    struct PrivacyApp(bool);
    impl crate::Application for PrivacyApp {
        type Message = ();
        fn update(&mut self, _: ()) {}
        fn remote_access_protected(&self) -> bool {
            self.0
        }
        fn view(&self, _: ViewContext) -> impl View<()> {
            Text::new("ordinary view")
        }
    }

    #[test]
    fn bounded_semantic_action_checks_generation_budget_target_and_protection() {
        use crate::{BoundedSemanticActionError as Error, BoundedSemanticError};
        let mut surface = HostedApplication::new(Counter { count: 0 }, 320, 200);
        let generation = surface.host().resolved_frame_generation();
        let invoke = || SemanticAction::Invoke(ActionKind::Activate);
        assert_eq!(
            surface
                .perform_bounded_semantic_action(
                    generation,
                    0,
                    SemanticAction::SetValue(crate::SemanticValueInput::Text("x".repeat(4097))),
                    64,
                    4096,
                    None
                )
                .err(),
            Some(Error::Snapshot(BoundedSemanticError::BudgetExceeded))
        );
        assert_eq!(
            surface
                .perform_bounded_semantic_action(
                    generation,
                    0,
                    SemanticAction::SetValue(crate::SemanticValueInput::Number(f64::NAN)),
                    64,
                    4096,
                    None
                )
                .err(),
            Some(Error::ActionUnavailable)
        );

        assert_eq!(
            surface
                .perform_bounded_semantic_action(generation, 0, invoke(), 0, 4096, None)
                .err(),
            Some(Error::Snapshot(BoundedSemanticError::BudgetExceeded))
        );
        assert_eq!(
            surface
                .perform_bounded_semantic_action(generation, usize::MAX, invoke(), 64, 4096, None)
                .err(),
            Some(Error::MissingTarget)
        );
        assert_eq!(
            surface
                .perform_bounded_semantic_action(
                    generation,
                    0,
                    SemanticAction::Invoke(ActionKind::Increment),
                    64,
                    4096,
                    None
                )
                .err(),
            Some(Error::ActionUnavailable)
        );
        assert_eq!(surface.host().application().count, 0);
        let result = surface
            .perform_bounded_semantic_action(generation, 0, invoke(), 64, 4096, None)
            .unwrap();
        assert!(result.changed);
        assert!(result.semantic_failures.is_empty());
        assert_eq!(surface.host().application().count, 1);
        assert_eq!(
            surface
                .perform_bounded_semantic_action(generation, 0, invoke(), 64, 4096, None)
                .err(),
            Some(Error::StaleGeneration)
        );
        assert_eq!(surface.host().application().count, 1);

        let mut protected = HostedApplication::new(PrivacyApp(true), 320, 200);
        let generation = protected.host().resolved_frame_generation();
        assert_eq!(
            protected
                .perform_bounded_semantic_action(generation, 0, invoke(), 64, 4096, None)
                .err(),
            Some(Error::Snapshot(BoundedSemanticError::ProtectedSurface))
        );
    }

    #[test]
    fn bounded_semantic_action_cannot_interrupt_a_local_pointer_press() {
        let mut surface = HostedApplication::new(Counter { count: 0 }, 320, 200);
        let bounds = surface.semantic_nodes()[0].bounds;
        let point = crate::Point {
            x: bounds.origin.x + bounds.size.width / 2.0,
            y: bounds.origin.y + bounds.size.height / 2.0,
        };
        assert!(!surface.pointer_interaction_active());
        surface.step(HostBatch {
            events: vec![HostEvent::Ui(crate::UiEvent::PointerPressed(point))],
            ..HostBatch::default()
        });
        assert!(surface.pointer_interaction_active());
        let fresh = surface.host().new_viewport(320, 200);
        let captured = surface.host_mut().replace_viewport(fresh);
        assert!(captured.pointer_interaction_active());
        assert!(!surface.pointer_interaction_active());
        surface.host_mut().replace_viewport(captured);
        assert!(surface.pointer_interaction_active());
        let before = surface.host().application().count;
        let generation = surface.host().resolved_frame_generation();
        assert_eq!(
            surface
                .perform_bounded_semantic_action(
                    generation,
                    0,
                    SemanticAction::Invoke(ActionKind::Activate),
                    64,
                    4096,
                    None
                )
                .err(),
            Some(crate::BoundedSemanticActionError::InputBusy)
        );
        assert_eq!(surface.host().application().count, before);
        surface.step(HostBatch {
            events: vec![HostEvent::Ui(crate::UiEvent::PointerCancelled)],
            ..HostBatch::default()
        });
        assert!(!surface.pointer_interaction_active());
        let before = surface.host().application().count;
        let generation = surface.host().resolved_frame_generation();
        let outcome = surface
            .perform_bounded_semantic_action(
                generation,
                0,
                SemanticAction::Invoke(ActionKind::Activate),
                64,
                4096,
                None,
            )
            .unwrap();
        assert!(outcome.semantic_failures.is_empty());
        assert_eq!(surface.host().application().count, before + 1);
    }

    #[test]
    fn bounded_semantic_value_uses_production_field_reducer() {
        struct InputApp(String);
        impl crate::Application for InputApp {
            type Message = String;
            fn update(&mut self, value: String) {
                self.0 = value;
            }
            fn view(&self, _: ViewContext) -> impl View<String> {
                crate::TextField::on_change(&self.0, |value| value)
            }
        }
        let mut host = UiHost::new(InputApp("before".into()), 320, 200);
        let generation = host.resolved_frame_generation();
        let outcome = host
            .perform_bounded_semantic_action(
                generation,
                0,
                SemanticAction::SetValue(crate::SemanticValueInput::Text("after".into())),
                64,
                4096,
                None,
            )
            .unwrap();
        assert!(outcome.semantic_failures.is_empty());
        assert!(outcome.changed);
        assert_eq!(host.application().0, "after");
        assert!(host.resolved_frame_generation() > generation);
    }

    #[test]
    fn live_application_protection_precedes_repaint_in_erased_host() {
        let mut surface = HostedApplication::new(PrivacyApp(false), 320, 200);
        assert!(!surface.remote_access_protected());
        surface
            .application_mut()
            .downcast_mut::<PrivacyApp>()
            .unwrap()
            .0 = true;
        assert!(surface.remote_access_protected());
        surface
            .application_mut()
            .downcast_mut::<PrivacyApp>()
            .unwrap()
            .0 = false;
        assert!(!surface.remote_access_protected());
    }

    #[test]
    fn protected_view_stays_protected_until_rebuilt_including_parked_viewports() {
        let mut surface = HostedApplication::new(PrivacyApp(true), 320, 200);
        let protected_viewport = surface.host().new_viewport(640, 480);
        surface.host_mut().application_mut().0 = false;
        assert!(surface.remote_access_protected());
        assert_eq!(
            surface.bounded_semantic_nodes(64, 4096),
            Err(crate::BoundedSemanticError::ProtectedSurface)
        );
        surface.step(HostBatch {
            application_changed: true,
            ..HostBatch::default()
        });
        assert!(!surface.remote_access_protected());

        let ordinary_viewport = surface.host_mut().replace_viewport(protected_viewport);
        assert!(surface.remote_access_protected());
        assert_eq!(
            surface.bounded_semantic_nodes(64, 4096),
            Err(crate::BoundedSemanticError::ProtectedSurface)
        );
        surface.host_mut().replace_viewport(ordinary_viewport);
        assert!(!surface.remote_access_protected());
    }

    #[test]
    fn masked_field_protects_host_without_application_override() {
        struct MaskedApp;
        impl crate::Application for MaskedApp {
            type Message = ();
            fn update(&mut self, _: ()) {}
            fn view(&self, _: ViewContext) -> impl View<()> {
                crate::TextField::on_change_masked("private fixture", '•', |_| ())
            }
        }
        let surface = HostedApplication::new(MaskedApp, 320, 200);
        assert!(surface.remote_access_protected());
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
