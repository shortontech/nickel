//! Read-only projection of the production indicator host for local assistive
//! technology. Only the actual semantic Stop button accepts an action. This
//! module has no remote-control protocol entry point.
use accesskit::{
    Action, ActionHandler, ActionRequest, Node, NodeId, Role, Tree, TreeId, TreeUpdate,
};
use nickel_ui::{AccessibilityNode, ActionKind, SemanticRole, UiId};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

const ROOT: NodeId = NodeId(1);
const MAX_NODES: usize = 2048;
const MAX_TEXT_BYTES: usize = 65_536;
const ACTION_LIFETIME: Duration = Duration::from_secs(1);

struct PendingStop {
    node: NodeId,
    queued_at: Instant,
}

/// Native accessibility threads retain only this bounded mailbox and an atomic
/// publication ID. They cannot read or mutate UiHost, surface ownership or
/// lease state.
pub(crate) struct LocalActionHandler {
    current: Arc<AtomicU64>,
    sender: SyncSender<PendingStop>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}
impl ActionHandler for LocalActionHandler {
    fn do_action(&mut self, request: ActionRequest) {
        if request.action != Action::Click
            || request.target_tree != TreeId::ROOT
            || request.data.is_some()
            || request.target_node.0 == 0
            || self.current.load(Ordering::Acquire) != request.target_node.0
        {
            return;
        }
        if self
            .sender
            .try_send(PendingStop {
                node: request.target_node,
                queued_at: Instant::now(),
            })
            .is_ok()
            && let Some(wake) = &self.wake
        {
            wake();
        }
    }
}

pub(crate) struct TrustedAccessibility {
    current: Arc<AtomicU64>,
    receiver: Receiver<PendingStop>,
    ids: BTreeMap<UiId, NodeId>,
    next_id: u64,
    stop: Option<(NodeId, UiId)>,
}
impl TrustedAccessibility {
    #[cfg(any(test, target_os = "windows"))]
    pub(crate) fn new() -> (Self, LocalActionHandler) {
        Self::new_with_wake(None)
    }

    fn new_with_wake(wake: Option<Arc<dyn Fn() + Send + Sync>>) -> (Self, LocalActionHandler) {
        let (sender, receiver) = mpsc::sync_channel(8);
        let current = Arc::new(AtomicU64::new(0));
        (
            Self {
                current: current.clone(),
                receiver,
                ids: BTreeMap::new(),
                next_id: 2,
                stop: None,
            },
            LocalActionHandler {
                current,
                sender,
                wake,
            },
        )
    }

    pub(crate) fn publish(
        &mut self,
        nodes: &[AccessibilityNode],
        scale: f64,
        renew_actions: bool,
    ) -> Result<TreeUpdate, String> {
        // Invalidate before any fallible work. An old queued action can never
        // become valid because a node ID or host instance was reused.
        self.current.store(0, Ordering::Release);
        if nodes.len() > MAX_NODES || !scale.is_finite() || scale <= 0.0 {
            return Err("trusted accessibility projection exceeds its bounds".to_owned());
        }
        let named: Vec<_> = nodes
            .iter()
            .filter(|node| node.label.as_ref().is_some_and(|label| !label.is_empty()))
            .collect();
        let mut text_bytes = 0usize;
        for node in &named {
            text_bytes = text_bytes.saturating_add(node.label.as_ref().map_or(0, String::len));
            if text_bytes > MAX_TEXT_BYTES
                || node.label.as_ref().is_some_and(|label| label.len() > 4096)
            {
                return Err("trusted accessibility labels exceed their bounds".to_owned());
            }
        }
        let mut stops = named.iter().filter(|node| {
            node.id.as_str().ends_with("remote-control-stop")
                && node.semantic_role == Some(SemanticRole::Button)
                && node.enabled
                && node.actions.contains(&ActionKind::Activate)
        });
        let stop_target = stops
            .next()
            .ok_or("trusted Stop semantic target is unavailable")?
            .id
            .clone();
        if stops.next().is_some() {
            return Err("trusted Stop semantic target is ambiguous".to_owned());
        }
        let live: BTreeSet<_> = named.iter().map(|node| node.id.clone()).collect();
        if live.len() != named.len() {
            return Err("duplicate trusted semantic identity".to_owned());
        }
        self.ids.retain(|id, _| live.contains(id));
        if renew_actions {
            self.ids.remove(&stop_target);
        }
        let mut update = TreeUpdate {
            nodes: Vec::with_capacity(named.len() + 1),
            tree: Some(Tree::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: ROOT,
        };
        let mut children = Vec::with_capacity(named.len());
        for source in named {
            let id = match self.ids.get(&source.id) {
                Some(id) => *id,
                None => {
                    let id = NodeId(self.next_id);
                    self.next_id = self
                        .next_id
                        .checked_add(1)
                        .ok_or("trusted accessibility generation exhausted")?;
                    self.ids.insert(source.id.clone(), id);
                    id
                }
            };
            let is_stop = source.id == stop_target;
            let mut node = Node::new(if is_stop { Role::Button } else { Role::Label });
            node.set_label(source.label.as_ref().expect("named node").clone());
            let bounds = source.rect;
            let rect = accesskit::Rect::new(
                f64::from(bounds.origin.x) * scale,
                f64::from(bounds.origin.y) * scale,
                f64::from(bounds.origin.x + bounds.size.width) * scale,
                f64::from(bounds.origin.y + bounds.size.height) * scale,
            );
            if ![rect.x0, rect.y0, rect.x1, rect.y1]
                .iter()
                .all(|value| value.is_finite())
            {
                return Err("trusted accessibility geometry is invalid".to_owned());
            }
            node.set_bounds(rect);
            if is_stop {
                node.add_action(Action::Click);
                self.stop = Some((id, source.id.clone()));
            } else {
                node.set_read_only();
            }
            if source.focused {
                update.focus = id;
            }
            children.push(id);
            update.nodes.push((id, node));
        }
        let mut root = Node::new(Role::Window);
        root.set_label("Nickel Remote AI Control");
        root.set_children(children);
        update.nodes.push((ROOT, root));
        self.current.store(
            self.stop.as_ref().expect("validated Stop").0.0,
            Ordering::Release,
        );
        Ok(update)
    }

    /// Owner-thread-only admission: stale, expired and retired actions are
    /// discarded before constructing the production semantic action.
    pub(crate) fn take_stop(&mut self) -> Option<UiId> {
        self.take_stop_with_clock(Instant::now)
    }

    fn take_stop_with_clock(&mut self, clock: impl Fn() -> Instant) -> Option<UiId> {
        for _ in 0..8 {
            let request = self.receiver.try_recv().ok()?;
            // Sample at admission, not at the beginning of a rendering frame:
            // COM callbacks can enqueue while accessibility events are raised.
            let now = clock();
            if self.current.load(Ordering::Acquire) == request.node.0
                && now
                    .checked_duration_since(request.queued_at)
                    .is_some_and(|age| age < ACTION_LIFETIME)
                && let Some((id, target)) = &self.stop
                && *id == request.node
            {
                return Some(target.clone());
            }
        }
        None
    }
}
impl Drop for TrustedAccessibility {
    fn drop(&mut self) {
        self.current.store(0, Ordering::Release);
    }
}

#[cfg(target_os = "linux")]
pub(crate) mod native {
    use super::*;
    use accesskit::{ActivationHandler, DeactivationHandler, Rect, TreeUpdate};
    use std::sync::Mutex;

    struct InitialTree(Arc<Mutex<TreeUpdate>>);
    impl ActivationHandler for InitialTree {
        fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
            Some(
                self.0
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone(),
            )
        }
    }

    struct Deactivation;
    impl DeactivationHandler for Deactivation {
        fn deactivate_accessibility(&mut self) {}
    }

    #[derive(Clone, Copy, Debug)]
    pub(crate) struct IndicatorGeometry {
        pub x: i32,
        pub y: i32,
        pub width: u32,
        pub height: u32,
        pub scale: f32,
    }

    pub(crate) struct IndicatorAccessibility {
        // Dropping the projection invalidates queued actions before the Unix
        // adapter unregisters its AT-SPI objects.
        projection: TrustedAccessibility,
        adapter: accesskit_unix::Adapter,
        snapshot: Arc<Mutex<TreeUpdate>>,
    }

    impl IndicatorAccessibility {
        pub(crate) fn new(
            nodes: &[AccessibilityNode],
            geometry: IndicatorGeometry,
            wake: impl Fn() + Send + Sync + 'static,
        ) -> Result<Self, String> {
            let (mut projection, actions) =
                TrustedAccessibility::new_with_wake(Some(Arc::new(wake)));
            let tree = projection.publish(nodes, f64::from(geometry.scale), true)?;
            let snapshot = Arc::new(Mutex::new(tree));
            let mut adapter = accesskit_unix::Adapter::new(
                InitialTree(Arc::clone(&snapshot)),
                actions,
                Deactivation,
            );
            set_bounds(&mut adapter, geometry)?;
            Ok(Self {
                projection,
                adapter,
                snapshot,
            })
        }

        pub(crate) fn update(
            &mut self,
            nodes: &[AccessibilityNode],
            geometry: IndicatorGeometry,
            renew_actions: bool,
        ) -> Result<(), String> {
            let update =
                self.projection
                    .publish(nodes, f64::from(geometry.scale), renew_actions)?;
            set_bounds(&mut self.adapter, geometry)?;
            *self
                .snapshot
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = update.clone();
            self.adapter.update_if_active(|| update);
            Ok(())
        }

        pub(crate) fn take_stop(&mut self) -> Option<UiId> {
            self.projection.take_stop()
        }
    }

    fn set_bounds(
        adapter: &mut accesskit_unix::Adapter,
        geometry: IndicatorGeometry,
    ) -> Result<(), String> {
        let scale = f64::from(geometry.scale);
        if !scale.is_finite() || scale <= 0.0 {
            return Err("trusted accessibility geometry is invalid".to_owned());
        }
        let rect = Rect::new(
            f64::from(geometry.x) * scale,
            f64::from(geometry.y) * scale,
            f64::from(geometry.x) * scale + f64::from(geometry.width) * scale,
            f64::from(geometry.y) * scale + f64::from(geometry.height) * scale,
        );
        if ![rect.x0, rect.y0, rect.x1, rect.y1]
            .iter()
            .all(|value| value.is_finite())
        {
            return Err("trusted accessibility geometry is invalid".to_owned());
        }
        adapter.set_root_window_bounds(rect, rect);
        Ok(())
    }
}

#[cfg(target_os = "windows")]
pub(crate) mod native {
    use super::*;
    use accesskit::{ActivationHandler, TreeUpdate};
    use accesskit_windows::SubclassingAdapter;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::{cell::RefCell, rc::Rc};
    use windows::Win32::Foundation::HWND;

    struct InitialTree(Rc<RefCell<TreeUpdate>>);
    impl ActivationHandler for InitialTree {
        fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
            Some(self.0.borrow().clone())
        }
    }

    pub(crate) struct IndicatorAccessibility {
        // Publication is invalidated before the subclass and retained COM state
        // are released. The owner drops this object before destroying its HWND.
        projection: TrustedAccessibility,
        adapter: SubclassingAdapter,
        snapshot: Rc<RefCell<TreeUpdate>>,
    }
    impl IndicatorAccessibility {
        pub(crate) fn new(
            window: &winit::window::Window,
            nodes: &[AccessibilityNode],
        ) -> Result<Self, String> {
            let RawWindowHandle::Win32(handle) = window
                .window_handle()
                .map_err(|error| error.to_string())?
                .as_raw()
            else {
                return Err("trusted accessibility requires an owned Windows window".to_owned());
            };
            let (mut projection, actions) = TrustedAccessibility::new();
            let tree = projection.publish(nodes, window.scale_factor(), true)?;
            let snapshot = Rc::new(RefCell::new(tree));
            // Called by the winit owner before its dedicated exposure method.
            let adapter = SubclassingAdapter::new(
                HWND(handle.hwnd.get() as *mut _),
                InitialTree(snapshot.clone()),
                actions,
            );
            Ok(Self {
                projection,
                adapter,
                snapshot,
            })
        }
        pub(crate) fn update(
            &mut self,
            nodes: &[AccessibilityNode],
            scale: f64,
            renew_actions: bool,
        ) -> Result<(), String> {
            let update = self.projection.publish(nodes, scale, renew_actions)?;
            *self.snapshot.borrow_mut() = update.clone();
            if let Some(events) = self.adapter.update_if_active(|| update) {
                events.raise();
            }
            Ok(())
        }
        pub(crate) fn take_stop(&mut self) -> Option<UiId> {
            self.projection.take_stop()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_indicator::{IndicatorGrant, RemoteIndicator};
    use nickel_ui::{HostBatch, HostEvent, SemanticAction, UiHost};

    fn host() -> UiHost<RemoteIndicator> {
        UiHost::new_at(
            RemoteIndicator {
                theme: crate::window_preview::semantic_theme_from_palette(
                    nickel_core::theme::ThemePalette::from_appearance(Default::default()),
                ),
                transport: "HTTPS".to_owned(),
                grants: vec![IndicatorGrant {
                    suspended: false,
                    connected: true,
                    id: 1,
                    client: "Test client".to_owned(),
                    scope: "Application".to_owned(),
                    remaining: "60s".to_owned(),
                    peer: "127.0.0.1 TLS".to_owned(),
                }],
                stop_requested: false,
                stopped_confirmation: false,
            },
            420,
            260,
            Instant::now(),
        )
    }
    fn click(node: NodeId) -> ActionRequest {
        ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: node,
            data: None,
        }
    }

    #[test]
    fn production_host_projects_read_only_labels_and_owner_dispatched_stop() {
        let mut host = host();
        let (mut projection, mut actions) = TrustedAccessibility::new();
        let update = projection
            .publish(host.accessibility_nodes(), 2.0, true)
            .unwrap();
        let labels: Vec<_> = update
            .nodes
            .iter()
            .filter_map(|(_, node)| node.label())
            .collect();
        for expected in [
            "Client: Test client",
            "Scope: Application",
            "State: Active",
            "Peer: 127.0.0.1 TLS",
            "Time: 60s",
        ] {
            assert!(
                labels.contains(&expected),
                "missing production semantic label {expected}"
            );
        }
        let clickable: Vec<_> = update
            .nodes
            .iter()
            .filter(|(_, node)| node.supports_action(Action::Click))
            .collect();
        assert_eq!(clickable.len(), 1);
        let (stop, node) = clickable[0];
        assert_eq!(node.role(), Role::Button);
        assert_eq!(node.label(), Some("Stop"));
        let source = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.id == projection.stop.as_ref().unwrap().1)
            .unwrap();
        assert_eq!(
            node.bounds().unwrap().x0,
            f64::from(source.rect.origin.x) * 2.0
        );
        for (_, node) in &update.nodes {
            assert!(!node.supports_action(Action::SetValue));
            assert!(!node.supports_action(Action::Focus));
            if node.role() == Role::Label {
                assert!(node.is_read_only());
            }
        }
        // This is the same callback used by COM; it can only enqueue.
        actions.do_action(click(*stop));
        assert!(!host.application().stop_requested);
        let target = projection.take_stop().unwrap();
        let result = host.step(HostBatch {
            events: vec![HostEvent::Accessibility {
                target,
                action: SemanticAction::Invoke(ActionKind::Activate),
            }],
            ..Default::default()
        });
        assert!(result.semantic_failures.is_empty());
        assert!(host.application().stop_requested);
    }

    #[test]
    fn stop_arriving_after_the_frame_observation_uses_fresh_admission_time() {
        let host = host();
        let (mut projection, mut actions) = TrustedAccessibility::new();
        let frame_observed = Instant::now();
        projection
            .publish(host.accessibility_nodes(), 1.0, true)
            .unwrap();
        actions.do_action(click(projection.stop.as_ref().unwrap().0));
        let request = projection.receiver.try_recv().unwrap();
        assert!(request.queued_at > frame_observed);
        // Model a callback arriving during adapter.update_if_active/events.raise.
        // Passing frame_observed to the old admission API discarded this Stop.
        assert!(actions.sender.try_send(request).is_ok());
        assert!(projection.take_stop().is_some());
    }

    #[test]
    fn renewed_authority_expiry_and_retirement_reject_retained_actions() {
        let host = host();
        let (mut projection, mut actions) = TrustedAccessibility::new();
        projection
            .publish(host.accessibility_nodes(), 1.0, true)
            .unwrap();
        let old = projection.stop.as_ref().unwrap().0;
        actions.do_action(click(old));
        projection
            .publish(host.accessibility_nodes(), 1.0, true)
            .unwrap();
        let current = projection.stop.as_ref().unwrap().0;
        assert_ne!(old, current);
        assert!(projection.take_stop().is_none());
        actions.do_action(click(old));
        assert!(projection.take_stop().is_none());
        actions.do_action(click(current));
        assert!(
            projection
                .take_stop_with_clock(|| Instant::now() + ACTION_LIFETIME)
                .is_none()
        );
        actions.do_action(click(current));
        assert!(projection.take_stop().is_some());
        drop(projection);
        assert_eq!(actions.current.load(Ordering::Acquire), 0);
        actions.do_action(click(current));
        assert!(
            actions
                .sender
                .try_send(PendingStop {
                    node: current,
                    queued_at: Instant::now()
                })
                .is_err()
        );
    }

    #[test]
    fn unsupported_actions_and_floods_cannot_escape_the_bounded_stop_mailbox() {
        let host = host();
        let (mut projection, mut actions) = TrustedAccessibility::new();
        projection
            .publish(host.accessibility_nodes(), 1.0, true)
            .unwrap();
        let stop = projection.stop.as_ref().unwrap().0;
        let mut unsupported = click(stop);
        unsupported.action = Action::SetValue;
        actions.do_action(unsupported);
        let mut with_data = click(stop);
        with_data.data = Some(accesskit::ActionData::Value("ignored".into()));
        actions.do_action(with_data);
        actions.do_action(click(ROOT));
        assert!(projection.take_stop().is_none());
        for _ in 0..100 {
            actions.do_action(click(stop));
        }
        let mut count = 0;
        while projection.take_stop().is_some() {
            count += 1;
        }
        assert_eq!(count, 8);
    }

    #[test]
    fn accepted_native_actions_wake_the_owner_but_rejected_actions_do_not() {
        let host = host();
        let wakes = Arc::new(AtomicU64::new(0));
        let wake_counter = Arc::clone(&wakes);
        let (mut projection, mut actions) =
            TrustedAccessibility::new_with_wake(Some(Arc::new(move || {
                wake_counter.fetch_add(1, Ordering::Relaxed);
            })));
        projection
            .publish(host.accessibility_nodes(), 1.0, true)
            .unwrap();
        let stop = projection.stop.as_ref().unwrap().0;
        let mut rejected = click(stop);
        rejected.action = Action::SetValue;
        actions.do_action(rejected);
        assert_eq!(wakes.load(Ordering::Relaxed), 0);
        actions.do_action(click(stop));
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        assert!(projection.take_stop().is_some());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unix_adapter_accepts_the_production_indicator_tree_and_geometry() {
        let host = host();
        let geometry = native::IndicatorGeometry {
            x: 120,
            y: 24,
            width: 420,
            height: 260,
            scale: 1.5,
        };
        let mut adapter =
            native::IndicatorAccessibility::new(host.accessibility_nodes(), geometry, || {})
                .unwrap();
        adapter
            .update(host.accessibility_nodes(), geometry, false)
            .unwrap();
        assert!(adapter.take_stop().is_none());
    }

    #[test]
    fn metadata_refresh_preserves_stop_identity_but_failed_publication_invalidates_it() {
        let mut host = host();
        let (mut projection, mut actions) = TrustedAccessibility::new();
        projection
            .publish(host.accessibility_nodes(), 1.0, true)
            .unwrap();
        let stop = projection.stop.as_ref().unwrap().0;
        actions.do_action(click(stop));
        host.application_mut().grants[0].remaining = "59s".to_owned();
        host.step(HostBatch {
            application_changed: true,
            ..Default::default()
        });
        projection
            .publish(host.accessibility_nodes(), 1.0, false)
            .unwrap();
        assert_eq!(projection.stop.as_ref().unwrap().0, stop);
        assert!(projection.take_stop().is_some());
        actions.do_action(click(stop));
        let oversize = vec![host.accessibility_nodes()[0].clone(); MAX_NODES + 1];
        assert!(projection.publish(&oversize, 1.0, false).is_err());
        assert!(projection.take_stop().is_none());
        projection.next_id = u64::MAX;
        assert!(
            projection
                .publish(host.accessibility_nodes(), 1.0, true)
                .is_err()
        );
        assert_eq!(projection.current.load(Ordering::Acquire), 0);
    }
}
