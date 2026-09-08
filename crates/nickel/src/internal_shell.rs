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
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InternalOutput {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

/// Placement and identity of one compositor-owned shell surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InternalShellSurface {
    pub id: InternalSurfaceId,
    pub role: SurfaceRole,
    pub output: Option<String>,
    pub size: (u32, u32),
    pub scene_generation: u64,
    pub commands_copied: u64,
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
    bar_on_all_displays: bool,
    file_windows: nickel_file::FileWindowCoordinator,
    file_requests: mpsc::Receiver<nickel_file::FileWindowRequest>,
    file_actions: Vec<nickel_file::FileWindowAction>,
}

impl InternalShellCoordinator {
    pub fn new(session_host: Arc<dyn SessionHost>, panel_edge: PanelEdge) -> Result<Self, String> {
        let (file_window_host, file_requests) = internal_file_window_channel();
        let bar_on_all_displays =
            nickel_core::shell_settings::ShellSettings::load_default().bar_on_all_displays;
        Ok(Self {
            shell: LiveShell::new_with_internal_hosts(session_host, file_window_host)?,
            surfaces: InternalSurfaceSet::new(),
            entries: Vec::new(),
            indices: HashMap::new(),
            panel_edge,
            bar_on_all_displays,
            file_windows: nickel_file::FileWindowCoordinator::new(),
            file_requests,
            file_actions: Vec::new(),
        })
    }

    pub fn semantic_theme(&self) -> nickel_ui::SemanticTheme {
        self.shell.semantic_theme()
    }

    pub fn image_cache_diagnostics(&self) -> crate::live_shell::ShellImageCacheDiagnostics {
        self.shell.image_cache_diagnostics()
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

    pub fn set_dashboard_projects(
        &mut self,
        projects: crate::launcher::DashboardSection<Vec<crate::launcher::DashboardProject>>,
    ) -> bool {
        self.shell.set_dashboard_projects(projects)
    }

    pub fn take_requested_codex_project(&mut self) -> Option<String> {
        self.shell.take_requested_codex_project()
    }

    #[cfg(test)]
    fn codex_available(&self) -> bool {
        self.shell.codex_available()
    }

    pub fn set_outputs(&mut self, outputs: &[InternalOutput]) {
        self.shell.retain_panel_outputs(outputs);
        let mut desired = Vec::new();
        for (index, output) in outputs.iter().enumerate() {
            for role in [SurfaceRole::Desktop, SurfaceRole::Lock] {
                let size = role_size(role, output.width, output.height, self.panel_edge);
                desired.push((role, Some(output.name.clone()), size));
            }
            if self.bar_on_all_displays || index == 0 {
                let role = SurfaceRole::Panel;
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

    pub fn set_bar_on_all_displays(&mut self, enabled: bool) -> bool {
        let changed = self.bar_on_all_displays != enabled;
        self.bar_on_all_displays = enabled;
        // The same persisted transaction also carries window-scope and
        // desktop-count behavior consumed by LiveShell.
        self.shell.refresh_system();
        changed
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
            scene_generation: 0,
            commands_copied: 0,
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
        let surface = self.entries.iter_mut().find(|surface| surface.id == id)?;
        let commands = if surface.role == SurfaceRole::Panel {
            self.shell.panel_scene_for_output(
                surface.output.as_deref(),
                surface.size.0,
                surface.size.1,
            )
        } else {
            self.shell
                .scene(surface.role, surface.size.0, surface.size.1)
        };
        surface.scene_generation = surface.scene_generation.saturating_add(1);
        surface.commands_copied = surface
            .commands_copied
            .saturating_add(commands.len() as u64);
        tracing::trace!(surface = ?id, generation = surface.scene_generation, commands_copied = surface.commands_copied, "internal shell scene counters");
        Some(commands)
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
        let visibility = self
            .entries
            .iter()
            .map(|surface| self.shell.surface_visible(surface.role))
            .collect::<Vec<_>>();
        let mut outcome = self.shell.poll_deadlines(now);
        if outcome.capture_screenshot && self.shell.capture_screenshot() {
            outcome.visibility_changed = true;
            outcome.redraw.push(SurfaceRole::Screenshot);
        }
        self.deadline_changes(&outcome, &visibility)
    }

    fn deadline_changes(
        &self,
        outcome: &crate::live_shell::ShellDeadlineOutcome,
        visibility: &[bool],
    ) -> Vec<InternalSurfaceId> {
        self.entries
            .iter()
            .zip(visibility.iter().copied())
            .filter(|(surface, was_visible)| {
                self.shell.surface_visible(surface.role) != *was_visible
                    || outcome.redraw.contains(&surface.role)
                    || (outcome.visibility_changed && surface.role == SurfaceRole::Panel)
            })
            .map(|(surface, _)| surface.id)
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

    pub fn apply_session_snapshot(&mut self, snapshot: nickel_session_protocol::Snapshot) -> bool {
        self.shell.apply_internal_session_snapshot(snapshot);
        self.shell.refresh_fast()
    }

    pub fn apply_session_snapshot_changes(
        &mut self,
        snapshot: nickel_session_protocol::Snapshot,
    ) -> Vec<InternalSurfaceId> {
        self.shell.apply_internal_session_snapshot(snapshot);
        // An external shell refreshes its feeds from the session socket. The
        // unified shell instead receives the canonical snapshot directly, so
        // consume it here before deciding which compositor-owned surfaces need
        // repainting. Merely storing it leaves panels on their pinned-only
        // startup projection until an unrelated full refresh happens.
        let roles = self.shell.refresh_fast_changes();
        self.entries
            .iter()
            .filter(|surface| roles.contains(&surface.role))
            .map(|surface| surface.id)
            .collect()
    }

    pub fn apply_system_status_update(
        &mut self,
        update: crate::platform::SystemStatusUpdate,
    ) -> Vec<InternalSurfaceId> {
        let roles: Option<&[SurfaceRole]> = match &update {
            crate::platform::SystemStatusUpdate::Audio(_) => {
                Some(&[SurfaceRole::ControlCenter, SurfaceRole::VolumeOsd])
            }
            crate::platform::SystemStatusUpdate::Network(_)
            | crate::platform::SystemStatusUpdate::Bluetooth(_) => {
                Some(&[SurfaceRole::ControlCenter])
            }
            crate::platform::SystemStatusUpdate::ShellSettingsChanged => None,
        };
        if !self.shell.apply_system_status_update(update) {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|surface| roles.is_none_or(|roles| roles.contains(&surface.role)))
            .map(|surface| surface.id)
            .collect()
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

    #[cfg(test)]
    pub fn step_slot(&mut self, id: InternalSurfaceId, batch: HostBatch) -> bool {
        !self.step_slot_changes(id, batch).is_empty()
    }

    pub fn step_slot_changes(
        &mut self,
        id: InternalSurfaceId,
        batch: HostBatch,
    ) -> Vec<InternalSurfaceId> {
        let Some(entry) = self.entries.iter().find(|surface| surface.id == id) else {
            return Vec::new();
        };
        let visibility = self
            .entries
            .iter()
            .map(|surface| self.shell.surface_visible(surface.role))
            .collect::<Vec<_>>();
        let mut changed = false;
        let mut dependent_roles = Vec::new();
        for event in batch.events {
            let nickel_ui::HostEvent::Ui(event) = event else {
                continue;
            };
            // Pointer-only panel/launcher navigation cannot change sibling
            // content. Actions and control drags can update an already-visible
            // popover or OSD even when its visibility stays unchanged.
            let action = matches!(
                event,
                nickel_ui::UiEvent::PointerPressed(_)
                    | nickel_ui::UiEvent::PointerReleased(_)
                    | nickel_ui::UiEvent::PointerContext(_)
                    | nickel_ui::UiEvent::TouchLongPress(_)
                    | nickel_ui::UiEvent::KeyboardActivate
                    | nickel_ui::UiEvent::KeyboardNavigateActivate
                    | nickel_ui::UiEvent::ActivateFocused
                    | nickel_ui::UiEvent::ControllerActivate
                    | nickel_ui::UiEvent::ControllerContextMenu
                    | nickel_ui::UiEvent::KeyboardContextMenu
                    | nickel_ui::UiEvent::ControllerBack
                    | nickel_ui::UiEvent::KeyboardNavigateBack
            );
            match entry.role {
                SurfaceRole::Panel if action => dependent_roles.extend([
                    SurfaceRole::Panel,
                    SurfaceRole::WindowPreview,
                    SurfaceRole::WindowContextMenu,
                ]),
                SurfaceRole::ControlCenter => dependent_roles.extend([
                    SurfaceRole::Panel,
                    SurfaceRole::VolumeOsd,
                    SurfaceRole::OnScreenKeyboard,
                ]),
                SurfaceRole::WindowPreview => {
                    dependent_roles.extend([SurfaceRole::Panel, SurfaceRole::WindowContextMenu])
                }
                SurfaceRole::WindowContextMenu => {
                    dependent_roles.extend([SurfaceRole::Panel, SurfaceRole::WindowPreview])
                }
                SurfaceRole::Launcher if action => dependent_roles.push(SurfaceRole::Panel),
                _ => {}
            }
            changed |= self
                .shell
                .shell_role_host_ui(entry.role, event, entry.size.0, entry.size.1);
        }
        let mut changes = Vec::new();
        if changed {
            changes.push(id);
        }
        let visibility_changed = self
            .entries
            .iter()
            .zip(&visibility)
            .any(|(surface, was_visible)| self.shell.surface_visible(surface.role) != *was_visible);
        for (surface, was_visible) in self.entries.iter().zip(visibility) {
            if self.shell.surface_visible(surface.role) != was_visible
                || (visibility_changed && surface.role == SurfaceRole::Panel)
                || (changed && dependent_roles.contains(&surface.role))
            {
                changes.push(surface.id);
            }
        }
        changes
    }

    pub fn toggle_launcher(&mut self) -> bool {
        self.shell.request_launcher_toggle()
    }

    pub fn launcher_visible(&self) -> bool {
        self.shell.surface_visible(SurfaceRole::Launcher)
    }

    /// Identify the concrete panel receiving an internal pointer event.
    ///
    /// Panel scenes are duplicated per output, while `LiveShell` owns one
    /// reusable panel host. Set its invocation context immediately before
    /// dispatch so popovers retain the clicked panel's output and origin.
    pub fn set_panel_context(&mut self, output: impl Into<String>, origin: (i32, i32)) {
        self.shell.set_panel_output(output);
        self.shell.set_panel_origin_x(origin.0);
        self.shell.set_panel_origin_y(origin.1);
    }

    pub fn popover_anchor(
        &self,
        preferred: nickel_session_protocol::AnchorSide,
    ) -> Option<(
        nickel_session_protocol::ShellRole,
        nickel_session_protocol::ShellPopoverAnchor,
    )> {
        self.shell.popover_anchor(preferred)
    }

    /// Deliver a compositor-owned shortcut directly to the in-process shell.
    ///
    /// The native input reducer already owns suppression and key-repeat
    /// semantics.  Keeping this final hop typed avoids depending on the legacy
    /// subscriber datagram, which does not exist in a unified session.
    pub fn consumer_control(&mut self, control: nickel_session_protocol::ConsumerControl) -> bool {
        self.shell
            .global_shortcut(crate::platform::GlobalShortcut::ConsumerControl(control))
    }

    /// Deliver a non-consumer compositor shortcut through the same typed shell owner.
    pub fn global_shortcut(&mut self, action: nickel_session_protocol::ShortcutAction) -> bool {
        use crate::platform::{GlobalShortcut, ScreenshotAction};
        use nickel_session_protocol::ShortcutAction;

        let shortcut = match action {
            ShortcutAction::ShowRun => GlobalShortcut::ShowRun,
            ShortcutAction::OpenFiles => GlobalShortcut::OpenFiles,
            ShortcutAction::OpenSettings => GlobalShortcut::OpenSettings,
            ShortcutAction::ShowControlCenter => GlobalShortcut::ShowControlCenter,
            ShortcutAction::ShowNotifications => GlobalShortcut::ShowNotifications,
            ShortcutAction::ShowDesktop => GlobalShortcut::ShowDesktop,
            ShortcutAction::ProjectDisplays => GlobalShortcut::ProjectDisplays,
            ShortcutAction::ShowWindowMenu => GlobalShortcut::ShowWindowMenu,
            ShortcutAction::ShowScreenshotTool => {
                GlobalShortcut::Screenshot(ScreenshotAction::InteractiveRegion)
            }
            ShortcutAction::CaptureActiveWindow => {
                GlobalShortcut::Screenshot(ScreenshotAction::ActiveWindow)
            }
            ShortcutAction::CaptureActiveWindowToFile => {
                GlobalShortcut::Screenshot(ScreenshotAction::ActiveWindowToFile)
            }
        };
        self.shell.global_shortcut(shortcut)
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
    use nickel_core::hotkeys::{CompositorShortcutAdapter, HotkeyAction, KeyCode, KeyEdge};
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

        fn capture_desktop(&self) -> crate::session_host::DesktopCapturePoll {
            crate::session_host::DesktopCapturePoll::Ready(Ok(crate::platform::DesktopCapture {
                image: image::RgbaImage::new(4, 4),
            }))
        }
    }

    fn coordinator() -> InternalShellCoordinator {
        InternalShellCoordinator::new(Arc::new(TestHost), PanelEdge::Bottom)
            .expect("headless shell coordinator")
    }

    struct StorageHost(Arc<AtomicU8>);

    #[test]
    fn native_consumer_controls_use_typed_host_and_only_show_confirmed_limit_values() {
        use nickel_session_protocol::ConsumerControl;
        struct MediaHost(std::sync::Mutex<Vec<ConsumerControl>>);
        impl SessionHost for MediaHost {
            fn dispatch(&self, _: ShellCommand) -> Result<(), SessionRequestError> {
                Ok(())
            }
            fn consumer_control(&self, control: ConsumerControl) -> bool {
                self.0.lock().unwrap().push(control);
                control != ConsumerControl::VolumeDown
            }
        }
        let host = Arc::new(MediaHost(std::sync::Mutex::new(Vec::new())));
        let mut shell = InternalShellCoordinator::new(host.clone(), PanelEdge::Bottom).unwrap();
        shell.set_outputs(&[InternalOutput {
            name: "test".into(),
            width: 800,
            height: 600,
            scale: 1.0,
        }]);
        let osd = shell.surface(SurfaceRole::VolumeOsd, None).unwrap().id;
        shell.apply_system_status_update(crate::platform::SystemStatusUpdate::Audio(
            crate::platform::AudioStatus {
                available: true,
                volume_percent: 100,
                muted: false,
                devices: Vec::new(),
            },
        ));
        assert!(!shell.visible(osd));
        assert!(shell.consumer_control(ConsumerControl::VolumeUp));
        assert!(shell.visible(osd));
        let changed = shell.poll(Instant::now() + std::time::Duration::from_secs(2));
        assert!(changed.contains(&osd));
        assert!(!shell.visible(osd));
        for control in [
            ConsumerControl::VolumeDown,
            ConsumerControl::VolumeMute,
            ConsumerControl::PlayPause,
            ConsumerControl::Next,
        ] {
            assert!(!shell.consumer_control(control));
            assert!(
                !shell.visible(osd),
                "accepted commands do not fabricate an audio result"
            );
        }
        assert_eq!(
            *host.0.lock().unwrap(),
            vec![
                ConsumerControl::VolumeUp,
                ConsumerControl::VolumeDown,
                ConsumerControl::VolumeMute,
                ConsumerControl::PlayPause,
                ConsumerControl::Next
            ]
        );
    }

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
            scale: 1.0,
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
                scale: 1.0,
            },
            InternalOutput {
                name: "two".into(),
                width: 1280,
                height: 720,
                scale: 1.0,
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
    fn primary_only_panel_policy_reconciles_two_outputs_without_removing_desktops() {
        let mut coordinator = coordinator();
        coordinator.set_bar_on_all_displays(false);
        coordinator.set_outputs(&[
            InternalOutput {
                name: "primary".into(),
                width: 1920,
                height: 1080,
                scale: 1.5,
            },
            InternalOutput {
                name: "secondary".into(),
                width: 1280,
                height: 720,
                scale: 1.0,
            },
        ]);

        assert!(
            coordinator
                .surface(SurfaceRole::Panel, Some("primary"))
                .is_some()
        );
        assert!(
            coordinator
                .surface(SurfaceRole::Panel, Some("secondary"))
                .is_none()
        );
        assert!(
            coordinator
                .surface(SurfaceRole::Desktop, Some("primary"))
                .is_some()
        );
        assert!(
            coordinator
                .surface(SurfaceRole::Desktop, Some("secondary"))
                .is_some()
        );

        assert!(coordinator.set_bar_on_all_displays(true));
        coordinator.set_outputs(&[
            InternalOutput {
                name: "primary".into(),
                width: 1920,
                height: 1080,
                scale: 1.5,
            },
            InternalOutput {
                name: "secondary".into(),
                width: 1280,
                height: 720,
                scale: 1.0,
            },
        ]);
        assert!(
            coordinator
                .surface(SurfaceRole::Panel, Some("secondary"))
                .is_some()
        );
    }

    #[test]
    fn topology_reconciliation_preserves_surfaces_for_unchanged_outputs() {
        let mut coordinator = coordinator();
        coordinator.set_outputs(&[
            InternalOutput {
                name: "left".into(),
                width: 1280,
                height: 720,
                scale: 1.0,
            },
            InternalOutput {
                name: "right".into(),
                width: 1920,
                height: 1080,
                scale: 1.0,
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
                scale: 1.0,
            },
            InternalOutput {
                name: "new".into(),
                width: 1024,
                height: 768,
                scale: 1.0,
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
            scale: 1.0,
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
            scale: 1.0,
        }]);
        let launcher = coordinator.surface(SurfaceRole::Launcher, None).unwrap().id;
        assert!(!coordinator.visible(launcher));
        assert!(coordinator.toggle_launcher());
        assert!(coordinator.visible(launcher));
    }

    #[test]
    fn applying_session_snapshot_immediately_refreshes_running_window_projection() {
        let mut coordinator = coordinator();
        let snapshot = nickel_session_protocol::Snapshot {
            windows: vec![nickel_session_protocol::WindowSnapshot {
                id: nickel_session_protocol::WindowId(41),
                application_id: "org.kde.konsole".into(),
                title: "Konsole".into(),
                active: true,
                minimized: false,
                maximized: false,
                fullscreen: false,
                geometry: None,
                workspace: nickel_session_protocol::WorkspaceId(1),
            }],
            ..Default::default()
        };

        assert!(coordinator.apply_session_snapshot(snapshot.clone()));
        assert!(
            coordinator
                .shell_mut()
                .taskbar_has_application("org.kde.konsole")
        );
        assert!(!coordinator.apply_session_snapshot(snapshot));
    }

    #[test]
    fn system_feed_changes_only_invalidate_their_visible_consumers() {
        let mut coordinator = coordinator();
        coordinator.set_outputs(&[InternalOutput {
            name: "nested".into(),
            width: 800,
            height: 600,
            scale: 1.0,
        }]);
        let desktop = coordinator
            .surface(SurfaceRole::Desktop, Some("nested"))
            .unwrap()
            .id;
        let panel = coordinator
            .surface(SurfaceRole::Panel, Some("nested"))
            .unwrap()
            .id;
        let control = coordinator
            .surface(SurfaceRole::ControlCenter, None)
            .unwrap()
            .id;
        let osd = coordinator
            .surface(SurfaceRole::VolumeOsd, None)
            .unwrap()
            .id;
        coordinator.scene(desktop);
        coordinator.scene(panel);
        coordinator.apply_system_status_update(crate::platform::SystemStatusUpdate::Audio(
            crate::platform::AudioStatus {
                available: false,
                devices: Vec::new(),
                volume_percent: 0,
                muted: false,
            },
        ));
        let audio = crate::platform::SystemStatusUpdate::Audio(crate::platform::AudioStatus {
            available: true,
            devices: Vec::new(),
            volume_percent: 73,
            muted: true,
        });
        let changes = coordinator.apply_system_status_update(audio.clone());
        assert_eq!(changes, vec![control, osd]);
        for id in changes {
            coordinator.scene(id);
        }
        assert!(coordinator.apply_system_status_update(audio).is_empty());
        let network =
            crate::platform::SystemStatusUpdate::Network(crate::platform::NetworkStatus {
                available: true,
                enabled: true,
                connected: true,
                name: "Audit Network".into(),
                signal_percent: 53,
                networks: Vec::new(),
            });
        assert_eq!(
            coordinator.apply_system_status_update(network.clone()),
            vec![control]
        );
        assert!(coordinator.apply_system_status_update(network).is_empty());
        assert_eq!(
            coordinator
                .surface(SurfaceRole::Desktop, Some("nested"))
                .unwrap()
                .scene_generation,
            1
        );
        assert_eq!(
            coordinator
                .surface(SurfaceRole::Panel, Some("nested"))
                .unwrap()
                .scene_generation,
            1
        );
    }

    #[test]
    fn local_panel_hover_and_launcher_typing_preserve_unrelated_scene_generations() {
        let mut coordinator = coordinator();
        coordinator.bar_on_all_displays = true;
        coordinator.set_outputs(&[
            InternalOutput {
                name: "left".into(),
                width: 1000,
                height: 800,
                scale: 1.0,
            },
            InternalOutput {
                name: "right".into(),
                width: 1000,
                height: 800,
                scale: 1.0,
            },
        ]);
        coordinator.apply_session_snapshot(nickel_session_protocol::Snapshot {
            windows: vec![nickel_session_protocol::WindowSnapshot {
                id: nickel_session_protocol::WindowId(991),
                application_id: "io.nickel.codex.audit".into(),
                title: "Audit task".into(),
                active: true,
                minimized: false,
                maximized: false,
                fullscreen: false,
                geometry: None,
                workspace: nickel_session_protocol::WorkspaceId(1),
            }],
            ..Default::default()
        });
        let desktop = coordinator
            .surface(SurfaceRole::Desktop, Some("left"))
            .unwrap()
            .id;
        let left_panel = coordinator
            .surface(SurfaceRole::Panel, Some("left"))
            .unwrap()
            .id;
        let right_panel = coordinator
            .surface(SurfaceRole::Panel, Some("right"))
            .unwrap()
            .id;
        for id in [desktop, right_panel, left_panel] {
            coordinator.scene(id);
        }
        let target = coordinator
            .shell
            .resolve_semantic_target(
                &nickel_session_protocol::ShellSemanticTarget::PanelApplication {
                    application_id: "io.nickel.codex.audit".into(),
                    output: Some("left".into()),
                    interaction: nickel_session_protocol::PointerInteraction::Hover,
                },
            )
            .unwrap();
        coordinator.set_panel_context("left", (0, 0));
        let changes = coordinator.step_slot_changes(
            left_panel,
            HostBatch {
                events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::PointerMoved(
                    nickel_ui::Point {
                        x: target.x as f32,
                        y: target.y as f32,
                    },
                ))],
                ..Default::default()
            },
        );
        assert_eq!(changes, vec![left_panel]);
        for id in changes {
            coordinator.scene(id);
        }
        assert_eq!(
            coordinator
                .surface(SurfaceRole::Panel, Some("right"))
                .unwrap()
                .scene_generation,
            1
        );
        assert_eq!(
            coordinator
                .surface(SurfaceRole::Desktop, Some("left"))
                .unwrap()
                .scene_generation,
            1
        );
        coordinator.toggle_launcher();
        let launcher = coordinator.surface(SurfaceRole::Launcher, None).unwrap().id;
        coordinator.scene(launcher);
        let changes = coordinator.step_slot_changes(
            launcher,
            HostBatch {
                events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::TextInput(
                    "terminal".into(),
                ))],
                ..Default::default()
            },
        );
        assert_eq!(changes, vec![launcher]);
        for id in changes {
            coordinator.scene(id);
        }
        assert_eq!(
            coordinator
                .surface(SurfaceRole::Panel, Some("left"))
                .unwrap()
                .scene_generation,
            2
        );
        assert_eq!(
            coordinator
                .surface(SurfaceRole::Desktop, Some("left"))
                .unwrap()
                .scene_generation,
            1
        );
        assert!(
            coordinator
                .surface(SurfaceRole::Launcher, None)
                .unwrap()
                .commands_copied
                > 0
        );
        // Exercise the production deadline-to-surface mapping separately from
        // native desktop directory/icon polls, whose independent 250 ms
        // deadlines can legitimately request a desktop repaint.
        let visibility = coordinator
            .entries
            .iter()
            .map(|surface| coordinator.visible(surface.id))
            .collect::<Vec<_>>();
        let changes = coordinator.deadline_changes(
            &crate::live_shell::ShellDeadlineOutcome {
                redraw: vec![SurfaceRole::Panel],
                visibility_changed: true,
                ..Default::default()
            },
            &visibility,
        );
        assert_eq!(changes, vec![left_panel, right_panel]);
    }

    #[test]
    fn panel_semantic_click_opens_the_internal_launcher() {
        let mut coordinator = coordinator();
        coordinator.set_outputs(&[InternalOutput {
            name: "nested".into(),
            width: 800,
            height: 600,
            scale: 1.0,
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

    #[test]
    fn production_meta_r_reducer_opens_internal_run_surface() {
        let mut coordinator = coordinator();
        coordinator.set_outputs(&[InternalOutput {
            name: "nested".into(),
            width: 800,
            height: 600,
            scale: 1.0,
        }]);
        let launcher = coordinator.surface(SurfaceRole::Launcher, None).unwrap().id;
        let mut hotkeys = CompositorShortcutAdapter::default();

        assert_eq!(
            hotkeys.handle(KeyCode::SuperLeft, KeyEdge::Pressed).action,
            None
        );
        assert_eq!(
            hotkeys.handle(KeyCode::KeyR, KeyEdge::Pressed).action,
            Some(HotkeyAction::ShowRun)
        );
        assert!(coordinator.global_shortcut(nickel_session_protocol::ShortcutAction::ShowRun));

        assert!(coordinator.visible(launcher));
        assert!(!coordinator.scene(launcher).unwrap().is_empty());
    }

    #[test]
    fn production_print_screen_reducer_requests_internal_capture_surface() {
        let mut coordinator = coordinator();
        coordinator.set_outputs(&[InternalOutput {
            name: "nested".into(),
            width: 800,
            height: 600,
            scale: 1.0,
        }]);
        let screenshot = coordinator
            .surface(SurfaceRole::Screenshot, None)
            .unwrap()
            .id;
        let mut hotkeys = CompositorShortcutAdapter::default();

        assert_eq!(
            hotkeys
                .handle(KeyCode::PrintScreen, KeyEdge::Pressed)
                .action,
            Some(HotkeyAction::ShowScreenshotTool)
        );
        assert!(
            coordinator
                .global_shortcut(nickel_session_protocol::ShortcutAction::ShowScreenshotTool)
        );

        assert!(!coordinator.visible(screenshot));
        coordinator.poll(Instant::now() + std::time::Duration::from_millis(100));
        assert!(coordinator.visible(screenshot));
        assert!(!coordinator.scene(screenshot).unwrap().is_empty());
    }
}
