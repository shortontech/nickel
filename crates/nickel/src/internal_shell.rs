//! Event-loop-free ownership of the built-in shell surfaces.
//!
//! This is the seam used by a compositor that wants to host Nickel's shell UI
//! directly.  It deliberately knows nothing about winit windows, Wayland
//! application ids, or the session control socket.  The compositor supplies a
//! typed [`SessionHost`], output geometry, input and presentation.

use std::{
    collections::HashMap,
    sync::{Arc, mpsc},
    time::Instant,
};

use nickel_ui::{
    AnyView, Application, HostBatch, InternalSurfaceId, InternalSurfaceSet, Text, ViewContext,
    backend::PaintCommand,
};

use crate::{
    file_window_host::internal_file_window_channel,
    live_shell::LiveShell,
    session_host::SessionHost,
    winit_shell::{PANEL_HEIGHT, PanelEdge, SurfaceRole},
};

/// Geometry of an output supplied by the compositor-native host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InternalOutput {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

/// Placement and identity of one compositor-owned shell surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InternalShellSurface {
    pub id: InternalSurfaceId,
    pub role: SurfaceRole,
    pub output: Option<String>,
    pub size: (u32, u32),
}

/// A presentation slot in [`InternalSurfaceSet`].
///
/// Shell state remains coordinated by `LiveShell` while it is being sliced
/// into independently hosted applications.  Registering every slot in the
/// shared internal-surface collection gives the compositor opaque identities
/// now, without creating a native window or a second event loop.
struct ShellSurfaceSlot {
    title: String,
}

impl Application for ShellSurfaceSlot {
    type Message = ();

    fn update(&mut self, (): Self::Message) {}

    fn view(&self, _context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        AnyView::new(Text::new(""))
    }

    fn title(&self) -> &str {
        &self.title
    }
}

/// Owns reusable shell state and compositor-local surface identities.
///
/// Construction performs no winit initialization and never registers a
/// socket-backed shell surface. Session mutations go through the injected
/// typed host.
pub(crate) struct InternalShellCoordinator {
    shell: LiveShell,
    surfaces: InternalSurfaceSet,
    entries: Vec<InternalShellSurface>,
    indices: HashMap<(SurfaceRole, Option<String>), usize>,
    panel_edge: PanelEdge,
    file_windows: nickel_file::FileWindowCoordinator,
    file_requests: mpsc::Receiver<nickel_file::FileWindowRequest>,
    file_actions: Vec<nickel_file::FileWindowAction>,
}

impl InternalShellCoordinator {
    pub fn new(session_host: Arc<dyn SessionHost>, panel_edge: PanelEdge) -> Result<Self, String> {
        let (file_window_host, file_requests) = internal_file_window_channel();
        Ok(Self {
            shell: LiveShell::new_with_internal_hosts(session_host, file_window_host)?,
            surfaces: InternalSurfaceSet::new(),
            entries: Vec::new(),
            indices: HashMap::new(),
            panel_edge,
            file_windows: nickel_file::FileWindowCoordinator::new(),
            file_requests,
            file_actions: Vec::new(),
        })
    }

    pub fn semantic_theme(&self) -> nickel_ui::SemanticTheme {
        self.shell.semantic_theme()
    }

    pub fn codex_project_menu_visible(&self) -> bool {
        self.shell.surface_visible(SurfaceRole::CodexProjectMenu)
    }

    pub fn apply_codex_projection(
        &mut self,
        projection: nickel_core::optional_features::CodexAvailabilityProjection,
    ) -> bool {
        self.shell.apply_codex_projection(projection)
    }

    pub fn take_requested_codex_project(&mut self) -> Option<String> {
        self.shell.take_requested_codex_project()
    }

    #[cfg(test)]
    fn codex_available(&self) -> bool {
        self.shell.codex_available()
    }

    pub fn set_outputs(&mut self, outputs: &[InternalOutput]) {
        let mut desired = Vec::new();
        for output in outputs {
            for role in [SurfaceRole::Desktop, SurfaceRole::Panel, SurfaceRole::Lock] {
                let size = role_size(role, output.width, output.height, self.panel_edge);
                desired.push((role, Some(output.name.clone()), size));
            }
        }
        if let Some(primary) = outputs.first() {
            for role in [
                SurfaceRole::Launcher,
                SurfaceRole::ControlCenter,
                SurfaceRole::Notification,
                SurfaceRole::VolumeOsd,
                SurfaceRole::WindowPreview,
                SurfaceRole::WindowContextMenu,
                SurfaceRole::CodexProjectMenu,
                SurfaceRole::Screenshot,
                SurfaceRole::OnScreenKeyboard,
            ] {
                let size = role_size(role, primary.width, primary.height, self.panel_edge);
                desired.push((role, None, size));
            }
        }

        let mut existing = std::mem::take(&mut self.entries)
            .into_iter()
            .map(|surface| ((surface.role, surface.output.clone()), surface))
            .collect::<HashMap<_, _>>();
        self.indices.clear();
        for (role, output, size) in desired {
            let key = (role, output.clone());
            if let Some(mut surface) = existing.remove(&key) {
                surface.size = size;
                self.indices.insert(key, self.entries.len());
                self.entries.push(surface);
            } else {
                self.insert(role, output, size);
            }
        }
        for surface in existing.into_values() {
            self.surfaces.remove(surface.id);
        }
    }

    fn insert(&mut self, role: SurfaceRole, output: Option<String>, size: (u32, u32)) {
        let id = self.surfaces.insert(
            ShellSurfaceSlot {
                title: format!("Nickel {role:?}"),
            },
            size.0,
            size.1,
        );
        let index = self.entries.len();
        self.indices.insert((role, output.clone()), index);
        self.entries.push(InternalShellSurface {
            id,
            role,
            output,
            size,
        });
    }

    pub fn surfaces(&self) -> &[InternalShellSurface] {
        &self.entries
    }

    pub fn surface(
        &self,
        role: SurfaceRole,
        output: Option<&str>,
    ) -> Option<&InternalShellSurface> {
        let key = (role, output.map(str::to_owned));
        self.indices
            .get(&key)
            .and_then(|index| self.entries.get(*index))
    }

    pub fn visible(&self, id: InternalSurfaceId) -> bool {
        self.entries
            .iter()
            .find(|surface| surface.id == id)
            .is_some_and(|surface| self.shell.surface_visible(surface.role))
    }

    pub fn scene(&mut self, id: InternalSurfaceId) -> Option<Vec<PaintCommand>> {
        let surface = self.entries.iter().find(|surface| surface.id == id)?;
        Some(
            self.shell
                .scene(surface.role, surface.size.0, surface.size.1),
        )
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        [
            self.shell.next_host_deadline(),
            self.surfaces.next_deadline(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub fn poll(&mut self, now: Instant) -> Vec<InternalSurfaceId> {
        self.apply_file_requests();
        let outcome = self.shell.poll_deadlines(now);
        self.entries
            .iter()
            .filter(|surface| outcome.visibility_changed || outcome.redraw.contains(&surface.role))
            .map(|surface| surface.id)
            .collect()
    }

    /// Refresh state supplied by session services rather than UI deadlines.
    ///
    /// Secure-storage transitions originate on the login-services worker and
    /// are delivered to the compositor loop explicitly. They must not depend
    /// on an animation, clock, or keyboard deadline happening to poll first.
    pub fn refresh_system(&mut self) -> Vec<InternalSurfaceId> {
        if !self.shell.refresh_secure_storage() {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|surface| surface.role == SurfaceRole::Launcher)
            .map(|surface| surface.id)
            .collect()
    }

    pub fn apply_session_snapshot(&mut self, snapshot: nickel_session_protocol::Snapshot) {
        self.shell.apply_internal_session_snapshot(snapshot);
    }

    pub fn apply_system_status_update(
        &mut self,
        update: crate::platform::SystemStatusUpdate,
    ) -> bool {
        self.shell.apply_system_status_update(update)
    }

    pub fn file_windows(&self) -> &nickel_file::FileWindowCoordinator {
        &self.file_windows
    }

    pub fn file_windows_mut(&mut self) -> &mut nickel_file::FileWindowCoordinator {
        &mut self.file_windows
    }

    fn apply_file_requests(&mut self) -> Vec<nickel_file::FileWindowAction> {
        let actions = self
            .file_requests
            .try_iter()
            .map(|request| self.file_windows.handle(request))
            .collect::<Vec<_>>();
        self.file_actions.extend(actions.iter().copied());
        actions
    }

    pub fn drain_file_actions(&mut self) -> Vec<nickel_file::FileWindowAction> {
        std::mem::take(&mut self.file_actions)
    }

    pub fn step_slot(&mut self, id: InternalSurfaceId, batch: HostBatch) -> bool {
        let Some(entry) = self.entries.iter().find(|surface| surface.id == id) else {
            return false;
        };
        let mut changed = false;
        for event in batch.events {
            let nickel_ui::HostEvent::Ui(event) = event else {
                continue;
            };
            changed |= self
                .shell
                .shell_role_host_ui(entry.role, event, entry.size.0, entry.size.1);
        }
        changed
    }

    pub fn toggle_launcher(&mut self) -> bool {
        self.shell.request_launcher_toggle()
    }

    #[cfg(test)]
    fn shell_mut(&mut self) -> &mut LiveShell {
        &mut self.shell
    }
}

fn role_size(role: SurfaceRole, width: u32, height: u32, panel_edge: PanelEdge) -> (u32, u32) {
    let _ = panel_edge;
    match role {
        SurfaceRole::Desktop | SurfaceRole::Lock => (width, height),
        SurfaceRole::Panel => (width, PANEL_HEIGHT),
        SurfaceRole::Launcher => (width.min(960), height.saturating_sub(PANEL_HEIGHT).min(720)),
        SurfaceRole::ControlCenter => (420.min(width), height.saturating_sub(PANEL_HEIGHT)),
        SurfaceRole::Notification => (420.min(width), 180.min(height)),
        SurfaceRole::VolumeOsd => (420.min(width), 96.min(height)),
        SurfaceRole::WindowPreview => (760.min(width), 520.min(height)),
        SurfaceRole::WindowContextMenu | SurfaceRole::CodexProjectMenu => {
            (360.min(width), 480.min(height))
        }
        SurfaceRole::Screenshot => (width, height),
        SurfaceRole::OnScreenKeyboard => (width, (height / 3).max(240).min(height)),
        SurfaceRole::CodexChat => (width.min(1120), height.min(760)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{SessionRequestError, ShellCommand};
    use std::sync::atomic::{AtomicU8, Ordering};

    struct TestHost;

    impl SessionHost for TestHost {
        fn dispatch(&self, _command: ShellCommand) -> Result<(), SessionRequestError> {
            Ok(())
        }

        fn secure_storage_state(
            &self,
        ) -> Result<crate::platform::SecureStorageState, SessionRequestError> {
            Ok(crate::platform::SecureStorageState::Ready)
        }

        fn request_secure_storage_retry(&self) -> Result<(), SessionRequestError> {
            Ok(())
        }
    }

    fn coordinator() -> InternalShellCoordinator {
        InternalShellCoordinator::new(Arc::new(TestHost), PanelEdge::Bottom)
            .expect("headless shell coordinator")
    }

    struct StorageHost(Arc<AtomicU8>);

    impl SessionHost for StorageHost {
        fn dispatch(&self, _command: ShellCommand) -> Result<(), SessionRequestError> {
            Ok(())
        }

        fn secure_storage_state(
            &self,
        ) -> Result<crate::platform::SecureStorageState, SessionRequestError> {
            Ok(match self.0.load(Ordering::Acquire) {
                0 => crate::platform::SecureStorageState::Starting,
                _ => crate::platform::SecureStorageState::Ready,
            })
        }
    }

    #[test]
    fn session_service_transition_refreshes_deadline_driven_shell_immediately() {
        let state = Arc::new(AtomicU8::new(0));
        let mut coordinator = InternalShellCoordinator::new(
            Arc::new(StorageHost(Arc::clone(&state))),
            PanelEdge::Bottom,
        )
        .unwrap();
        coordinator.set_outputs(&[InternalOutput {
            name: "one".into(),
            width: 1920,
            height: 1080,
        }]);

        state.store(1, Ordering::Release);
        let changed = coordinator.refresh_system();

        assert_eq!(
            changed,
            [coordinator.surface(SurfaceRole::Launcher, None).unwrap().id]
        );
        assert!(coordinator.refresh_system().is_empty());
    }

    #[test]
    fn compositor_can_publish_codex_availability_to_shell_surfaces() {
        use nickel_core::optional_features::{
            CodexAvailabilityProjection, FeatureHealth, FeatureInstallation, FeatureSupport,
        };

        let mut coordinator = coordinator();
        assert!(!coordinator.codex_available());

        assert!(
            coordinator.apply_codex_projection(CodexAvailabilityProjection::new(
                FeatureSupport::Supported,
                FeatureInstallation::Installed,
                true,
                FeatureHealth::Loading,
                7,
                Some("Checking the selected Codex backend…".into()),
            ))
        );

        assert!(coordinator.codex_available());
    }

    #[test]
    fn output_inventory_uses_internal_ids_without_native_windows() {
        let mut coordinator = coordinator();
        coordinator.set_outputs(&[
            InternalOutput {
                name: "one".into(),
                width: 1920,
                height: 1080,
            },
            InternalOutput {
                name: "two".into(),
                width: 1280,
                height: 720,
            },
        ]);

        assert_eq!(coordinator.surfaces().len(), 15);
        let panel = coordinator
            .surface(SurfaceRole::Panel, Some("two"))
            .unwrap();
        assert_eq!(panel.size, (1280, PANEL_HEIGHT));
        assert!(coordinator.visible(panel.id));
        assert!(!coordinator.visible(coordinator.surface(SurfaceRole::Launcher, None).unwrap().id));
    }

    #[test]
    fn topology_reconciliation_preserves_surfaces_for_unchanged_outputs() {
        let mut coordinator = coordinator();
        coordinator.set_outputs(&[
            InternalOutput {
                name: "left".into(),
                width: 1280,
                height: 720,
            },
            InternalOutput {
                name: "right".into(),
                width: 1920,
                height: 1080,
            },
        ]);
        let left_panel = coordinator
            .surface(SurfaceRole::Panel, Some("left"))
            .unwrap()
            .id;
        let right_desktop = coordinator
            .surface(SurfaceRole::Desktop, Some("right"))
            .unwrap()
            .id;
        let launcher = coordinator.surface(SurfaceRole::Launcher, None).unwrap().id;

        coordinator.set_outputs(&[
            InternalOutput {
                name: "right".into(),
                width: 1600,
                height: 900,
            },
            InternalOutput {
                name: "new".into(),
                width: 1024,
                height: 768,
            },
        ]);

        assert!(
            coordinator
                .surface(SurfaceRole::Panel, Some("left"))
                .is_none()
        );
        assert_eq!(
            coordinator
                .surface(SurfaceRole::Desktop, Some("right"))
                .unwrap()
                .id,
            right_desktop
        );
        assert_eq!(
            coordinator.surface(SurfaceRole::Launcher, None).unwrap().id,
            launcher
        );
        assert_eq!(
            coordinator
                .surface(SurfaceRole::Panel, Some("right"))
                .unwrap()
                .size,
            (1600, PANEL_HEIGHT)
        );
        assert!(
            !coordinator
                .surfaces()
                .iter()
                .any(|surface| surface.id == left_panel)
        );
    }

    #[test]
    fn scenes_are_rendered_from_reusable_shell_state() {
        let mut coordinator = coordinator();
        coordinator.set_outputs(&[InternalOutput {
            name: "nested".into(),
            width: 800,
            height: 600,
        }]);
        let panel = coordinator
            .surface(SurfaceRole::Panel, Some("nested"))
            .unwrap()
            .id;
        assert!(!coordinator.scene(panel).unwrap().is_empty());
        assert!(coordinator.shell_mut().surface_visible(SurfaceRole::Panel));
    }

    #[test]
    fn meta_launcher_toggle_changes_internal_visibility_without_session_transport() {
        let mut coordinator = coordinator();
        coordinator.set_outputs(&[InternalOutput {
            name: "nested".into(),
            width: 800,
            height: 600,
        }]);
        let launcher = coordinator.surface(SurfaceRole::Launcher, None).unwrap().id;
        assert!(!coordinator.visible(launcher));
        assert!(coordinator.toggle_launcher());
        assert!(coordinator.visible(launcher));
    }

    #[test]
    fn panel_semantic_click_opens_the_internal_launcher() {
        let mut coordinator = coordinator();
        coordinator.set_outputs(&[InternalOutput {
            name: "nested".into(),
            width: 800,
            height: 600,
        }]);
        let panel = coordinator
            .surface(SurfaceRole::Panel, Some("nested"))
            .unwrap()
            .id;
        let launcher = coordinator.surface(SurfaceRole::Launcher, None).unwrap().id;
        for event in [
            nickel_ui::UiEvent::PointerPressed(nickel_ui::Point { x: 20.0, y: 28.0 }),
            nickel_ui::UiEvent::PointerReleased(nickel_ui::Point { x: 20.0, y: 28.0 }),
        ] {
            coordinator.step_slot(
                panel,
                HostBatch {
                    events: vec![nickel_ui::HostEvent::Ui(event)],
                    ..Default::default()
                },
            );
        }
        assert!(coordinator.visible(launcher));
        assert!(!coordinator.scene(launcher).unwrap().is_empty());
    }
}
