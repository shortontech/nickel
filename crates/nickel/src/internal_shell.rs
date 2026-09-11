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
    /// Canonical compositor-space logical origin; never divide by scale again.
    pub x: i32,
    pub y: i32,
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
    clipboard_result: Option<Result<String, String>>,
    preview_generation: Option<u64>,
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
            clipboard_result: None,
            preview_generation: None,
        })
    }

    pub fn semantic_theme(&self) -> nickel_ui::SemanticTheme {
        self.shell.semantic_theme()
    }

    pub fn image_cache_diagnostics(&self) -> crate::live_shell::ShellImageCacheDiagnostics {
        self.shell.image_cache_diagnostics()
    }

    pub(crate) fn image_cache_diagnostics_for_previews(
        &self,
        allow: impl Fn(crate::model::WindowId) -> bool,
    ) -> crate::live_shell::ShellImageCacheDiagnostics {
        self.shell.image_cache_diagnostics_for_previews(allow)
    }

    pub fn codex_project_menu_visible(&self) -> bool {
        self.shell.surface_visible(SurfaceRole::CodexProjectMenu)
    }

    pub(crate) fn dismiss_ephemeral_on_focus_loss(&mut self, role: SurfaceRole) -> bool {
        self.shell.dismiss_ephemeral_on_focus_loss(role)
    }

    pub fn apply_codex_projection(
        &mut self,
        projection: nickel_core::optional_features::CodexAvailabilityProjection,
    ) -> bool {
        self.shell.apply_codex_projection(projection)
    }

    pub(crate) fn codex_projection(
        &self,
    ) -> Option<&nickel_core::optional_features::CodexAvailabilityProjection> {
        self.shell.codex_projection()
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
        // Runtime surfaces are recreated on topology reconciliation. A release
        // from their former geometry must not activate the replacement keyboard.
        self.shell.cancel_keyboard_gestures();
        self.shell.retain_panel_outputs(outputs);
        // Reconcile file placement before any surface can render. Creating a desktop
        // slot alone leaves newly enumerated files without a live output assignment.
        self.shell.set_desktop_outputs(
            outputs
                .iter()
                .enumerate()
                .map(|(index, output)| {
                    // Match panel slot ownership below; an output without a panel
                    // must retain its full usable desktop height.
                    let reservation = if self.bar_on_all_displays || index == 0 {
                        PANEL_HEIGHT.min(output.height)
                    } else {
                        0
                    };
                    nickel_file::desktop::DesktopOutput {
                        id: output.name.clone(),
                        primary: index == 0,
                        work_area: nickel_file::desktop::Rect {
                            x: output.x as f32,
                            y: output.y as f32
                                + if self.panel_edge == PanelEdge::Top {
                                    reservation as f32
                                } else {
                                    0.0
                                },
                            width: output.width as f32,
                            height: output.height.saturating_sub(reservation) as f32,
                        },
                        scale: output.scale,
                    }
                })
                .collect(),
        );
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

    pub(crate) fn remote_access_protected(&self, id: InternalSurfaceId) -> bool {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .is_none_or(|entry| {
                !self.shell.surface_visible(entry.role)
                    || self.shell.surface_remote_access_protected(entry.role)
            })
    }

    pub(crate) fn bounded_shell_semantics(
        &self,
        id: InternalSurfaceId,
    ) -> Result<(u64, Vec<nickel_ui::SemanticNodeSnapshot>), String> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or("shell surface has retired")?;
        self.shell
            .bounded_shell_semantics(entry.role, entry.output.as_deref())
    }

    pub(crate) fn perform_bounded_shell_action(
        &mut self,
        id: InternalSurfaceId,
        generation: u64,
        node: usize,
        action: nickel_ui::SemanticAction,
        clipboard_limit: usize,
    ) -> Result<crate::live_shell::remote_semantics::RemoteShellOutcome, String> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or("shell surface has retired")?;
        let outcome = self.shell.perform_bounded_shell_action(
            entry.role,
            entry.output.as_deref(),
            generation,
            node,
            action,
            clipboard_limit,
        )?;
        Ok(outcome)
    }

    pub(crate) fn resolve_remote_installed_launch(
        &mut self,
        effect: &crate::live_shell::remote_semantics::RemoteShellEffect,
    ) -> Result<Option<crate::model::Application>, String> {
        self.shell.resolve_remote_installed_launch(effect)
    }

    pub(crate) fn stage_remote_shell_effect(
        &mut self,
        effect: crate::live_shell::remote_semantics::RemoteShellEffect,
    ) -> Result<Vec<crate::platform::ShellCommand>, String> {
        self.shell.stage_remote_shell_effect(effect)
    }

    pub(crate) fn apply_control_visibility(&mut self, visible: bool) {
        self.shell.apply_control_visibility(visible);
    }

    pub fn surfaces(&self) -> &[InternalShellSurface] {
        &self.entries
    }

    /// Keep scene layout and normalized input in the same compositor-owned
    /// logical size. Resizing does not replace the slot or its gesture identity.
    pub fn set_surface_size(&mut self, id: InternalSurfaceId, size: (u32, u32)) -> bool {
        let Some(surface) = self.entries.iter_mut().find(|surface| surface.id == id) else {
            return false;
        };
        if surface.size == size {
            return false;
        }
        surface.size = size;
        if let Some(host) = self.surfaces.get_mut(id) {
            host.step(HostBatch {
                surface_size: Some(size),
                ..Default::default()
            });
        }
        true
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

    #[cfg(any(target_os = "linux", test))]
    pub fn resolve_semantic_target(
        &self,
        target: &nickel_session_protocol::ShellSemanticTarget,
    ) -> Option<nickel_session_protocol::ResolvedShellTarget> {
        self.shell.resolve_semantic_target(target)
    }

    /// Bind a desktop viewport for either rendering or normalized input. Keeping
    /// this projection in one place prevents input from using the last drawn output.
    fn select_desktop_viewport(&mut self, id: InternalSurfaceId) -> Option<()> {
        let entry = self.entries.iter().find(|surface| surface.id == id)?;
        if entry.role == SurfaceRole::Desktop {
            let output = entry.output.as_deref()?;
            let (origin, scale) = self.shell.desktop_output_projection(output)?;
            // Layout reports the usable area's origin, but this surface covers the
            // whole output. Undo only the top reservation so it stays visible in
            // local icon coordinates rather than being subtracted a second time.
            let top_reservation = if self.panel_edge == PanelEdge::Top
                && self.surface(SurfaceRole::Panel, Some(output)).is_some()
            {
                PANEL_HEIGHT.min(entry.size.1) as f32
            } else {
                0.0
            };
            // Select the retained viewport on every dispatch, independently of pointer
            // focus and output enumeration order. The desktop model itself is shared.
            self.shell.set_desktop_output(
                output.to_owned(),
                origin.x,
                origin.y - top_reservation,
                scale,
            );
        }
        Some(())
    }

    pub fn scene(&mut self, id: InternalSurfaceId) -> Option<Vec<PaintCommand>> {
        self.select_desktop_viewport(id)?;
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

    pub(crate) fn native_preview_windows(&mut self) -> Vec<crate::model::WindowId> {
        self.shell.native_preview_windows()
    }

    pub(crate) fn sync_native_preview_pixels<'a>(
        &mut self,
        generation: u64,
        interest_changed: bool,
        frame_for: impl FnMut(crate::model::WindowId) -> Option<(u16, u16, &'a [u8])>,
    ) -> bool {
        // Ordinary pointer/focus scene sync must not compare every thumbnail.
        // The session revision changes only when completed pixels arrive/retire.
        if self.preview_generation == Some(generation)
            && !interest_changed
            && !self.shell.native_preview_cache_empty()
        {
            return false;
        }
        self.preview_generation = Some(generation);
        self.shell.sync_native_preview_pixels(frame_for)
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

    pub(crate) fn launcher_preferences_busy(&self) -> bool {
        self.shell.launcher_preferences_busy()
    }

    pub(crate) fn launcher_favorites_match(
        &self,
        preferences: &nickel_core::launcher_preferences::LauncherPreferences,
    ) -> bool {
        self.shell.launcher_favorites_match(preferences)
    }

    pub(crate) fn apply_committed_launcher_preferences(
        &mut self,
        preferences: nickel_core::launcher_preferences::LauncherPreferences,
    ) -> Result<Vec<InternalSurfaceId>, String> {
        self.shell
            .apply_committed_launcher_preferences(preferences)?;
        Ok(self
            .entries
            .iter()
            .filter(|entry| matches!(entry.role, SurfaceRole::Launcher | SurfaceRole::Panel))
            .map(|entry| entry.id)
            .collect())
    }

    pub(crate) fn apply_application_discovery(
        &mut self,
        discovery: crate::model::ApplicationDiscovery,
    ) -> (Vec<InternalSurfaceId>, usize, bool) {
        let (applications, partial) = self.shell.apply_application_discovery(discovery);
        let changed = self
            .entries
            .iter()
            .filter(|entry| matches!(entry.role, SurfaceRole::Launcher | SurfaceRole::Panel))
            .map(|entry| entry.id)
            .collect();
        (changed, applications, partial)
    }

    /// Apply an already-authorized settings snapshot without rereading its file.
    pub(crate) fn apply_prepared_shell_settings(
        &mut self,
        settings: nickel_core::shell_settings::ShellSettings,
    ) -> Vec<InternalSurfaceId> {
        if !self.shell.apply_shell_settings(settings) {
            return Vec::new();
        }
        self.entries.iter().map(|surface| surface.id).collect()
    }

    pub fn apply_system_status_update(
        &mut self,
        update: crate::platform::SystemStatusUpdate,
    ) -> Vec<InternalSurfaceId> {
        let roles: Option<&[SurfaceRole]> = match &update {
            crate::platform::SystemStatusUpdate::Audio(_)
            | crate::platform::SystemStatusUpdate::AudioWithActivity { .. } => {
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

    /// Install the seat snapshot on the target viewport before its pointer batch.
    /// This also handles modifiers pressed before the desktop receives focus.
    pub fn set_desktop_input_modifiers(
        &mut self,
        id: InternalSurfaceId,
        modifiers: &nickel_input::ModifierState,
    ) {
        if self
            .entries
            .iter()
            .any(|entry| entry.id == id && entry.role == SurfaceRole::Desktop)
            && self.select_desktop_viewport(id).is_some()
        {
            self.shell.set_desktop_input_modifiers(modifiers);
        }
    }

    pub fn set_file_clipboard_available(&mut self, available: bool) {
        self.shell.set_file_clipboard_available(available);
    }

    pub fn step_slot_changes(
        &mut self,
        id: InternalSurfaceId,
        batch: HostBatch,
    ) -> Vec<InternalSurfaceId> {
        if self.select_desktop_viewport(id).is_none() {
            return Vec::new();
        }
        let Some(entry) = self.entries.iter().find(|surface| surface.id == id) else {
            return Vec::new();
        };
        let visibility = self
            .entries
            .iter()
            .map(|surface| self.shell.surface_visible(surface.role))
            .collect::<Vec<_>>();
        let mut changed = false;
        if let Some(focused) = batch.window_focused
            && !matches!(
                entry.role,
                SurfaceRole::Desktop | SurfaceRole::OnScreenKeyboard
            )
        {
            changed |= self.shell.shell_role_host_ui(
                entry.role,
                if focused {
                    nickel_ui::UiEvent::FocusGained
                } else {
                    nickel_ui::UiEvent::FocusLost
                },
                entry.size.0,
                entry.size.1,
            );
        }
        if entry.role == SurfaceRole::Desktop && batch.window_focused == Some(false) {
            // Host focus changes are lifecycle notifications, not device events;
            // they still must cancel the production desktop transaction and keys.
            changed |= self
                .shell
                .desktop_input(nickel_input::InputEvent::FocusLost {
                    order: nickel_input::EventOrder(0),
                });
        }
        let mut dependent_roles = Vec::new();
        if entry.role == SurfaceRole::OnScreenKeyboard && batch.window_focused == Some(false) {
            changed |= self.shell.keyboard_host_input(
                nickel_input::InputEvent::FocusLost {
                    order: nickel_input::EventOrder(0),
                },
                entry.size.0,
                entry.size.1,
            );
        }
        for event in batch.events {
            if entry.role == SurfaceRole::Screenshot {
                changed |= match event {
                    nickel_ui::HostEvent::Controller(action) => {
                        self.shell.screenshot_controller(action)
                    }
                    event => self
                        .shell
                        .screenshot_host_event(event, entry.size.0, entry.size.1),
                };
                continue;
            }
            if matches!(
                entry.role,
                SurfaceRole::Launcher | SurfaceRole::ControlCenter
            ) && let nickel_ui::HostEvent::Normalized {
                input,
                clipboard_text,
            } = event
            {
                let mut outcome = if entry.role == SurfaceRole::Launcher {
                    self.shell.launcher_host_event_with_clipboard_limit(
                        nickel_ui::HostEvent::Normalized {
                            input,
                            clipboard_text,
                        },
                        entry.size.0,
                        entry.size.1,
                        batch.clipboard_text_limit,
                    )
                } else {
                    dependent_roles.extend([
                        SurfaceRole::Panel,
                        SurfaceRole::VolumeOsd,
                        SurfaceRole::OnScreenKeyboard,
                    ]);
                    self.shell.control_host_event(
                        nickel_ui::HostEvent::Normalized {
                            input,
                            clipboard_text,
                        },
                        entry.size,
                        batch.clipboard_text_limit,
                    )
                };
                changed |= outcome.changed;
                crate::session_host::record_clipboard_outcome(
                    &mut self.clipboard_result,
                    &mut outcome,
                );
                continue;
            }
            if entry.role == SurfaceRole::OnScreenKeyboard
                && let nickel_ui::HostEvent::Normalized { input, .. } = event
            {
                // Preserve the press-time recipient lease through native release.
                changed |= self
                    .shell
                    .keyboard_host_input(input, entry.size.0, entry.size.1);
                continue;
            }
            // Desktop reducers need the original button, key edge, modifier snapshot,
            // and contact identity. Do not fabricate them from lossy UiEvent actions.
            if entry.role == SurfaceRole::Desktop {
                if let nickel_ui::HostEvent::Normalized { input, .. } = event {
                    changed |= self.shell.desktop_input(input);
                }
                continue;
            }
            if let nickel_ui::HostEvent::Normalized { input, .. } = event {
                match entry.role {
                    SurfaceRole::WindowPreview => {
                        dependent_roles
                            .extend([SurfaceRole::Panel, SurfaceRole::WindowContextMenu]);
                        changed |= self.shell.preview_host_input(input).changed;
                    }
                    SurfaceRole::WindowContextMenu => {
                        dependent_roles.extend([SurfaceRole::Panel, SurfaceRole::WindowPreview]);
                        changed |=
                            self.shell
                                .window_menu_host_input(input, entry.size.0, entry.size.1);
                    }
                    _ => {}
                }
                continue;
            }
            if let nickel_ui::HostEvent::Shortcut(shortcut) = event {
                changed |= self.shell.shell_role_host_shortcut(
                    entry.role,
                    shortcut,
                    entry.size.0,
                    entry.size.1,
                );
                continue;
            }
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
            // Semantic commands and context-menu activation need the same
            // pre-edit admission and ownership transport as normalized keys.
            let outcome = match entry.role {
                SurfaceRole::Launcher => Some(self.shell.launcher_host_event_with_clipboard_limit(
                    nickel_ui::HostEvent::Ui(event),
                    entry.size.0,
                    entry.size.1,
                    batch.clipboard_text_limit,
                )),
                SurfaceRole::ControlCenter => Some(self.shell.control_host_event(
                    nickel_ui::HostEvent::Ui(event),
                    entry.size,
                    batch.clipboard_text_limit,
                )),
                _ => {
                    changed |= self.shell.shell_role_host_ui(
                        entry.role,
                        event,
                        entry.size.0,
                        entry.size.1,
                    );
                    None
                }
            };
            if let Some(mut outcome) = outcome {
                changed |= outcome.changed;
                crate::session_host::record_clipboard_outcome(
                    &mut self.clipboard_result,
                    &mut outcome,
                );
            }
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

    /// Apply a compositor-owned transition without sending a second IPC request.
    pub(crate) fn apply_launcher_visibility(&mut self, visible: bool) {
        if self.launcher_visible() != visible {
            self.shell.apply_session_launcher_visibility(visible);
        }
    }

    pub fn toggle_launcher(&mut self) -> bool {
        self.shell.request_launcher_toggle()
    }

    pub(crate) fn take_clipboard_result(&mut self) -> Option<Result<String, String>> {
        self.clipboard_result.take()
    }

    pub(crate) fn focused_field_lease(
        &self,
        id: InternalSurfaceId,
    ) -> Option<(nickel_ui::UiId, u64)> {
        let role = self.entries.iter().find(|entry| entry.id == id)?.role;
        self.shell.shell_field_lease(role)
    }

    pub(crate) fn pointer_interaction_active(&self) -> bool {
        self.shell.pointer_interaction_active()
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

    /// Keep the compositor-owned lock scene synchronized with the session's
    /// authoritative security state.
    pub fn set_lock_state(&mut self, locked: bool) -> bool {
        self.shell
            .global_shortcut(crate::platform::GlobalShortcut::LockState { locked })
    }

    /// Keep capture and presentation on the output selected at invocation.
    pub(crate) fn set_screenshot_output(&mut self, output: Option<String>) {
        self.shell.screenshot_output = output;
    }

    pub(crate) fn screenshot_output(&self) -> Option<&str> {
        self.shell.screenshot_output.as_deref()
    }

    pub(crate) fn window_menu_generation(&self) -> Option<u64> {
        self.shell.window_menu_generation()
    }

    pub(crate) fn retire_window_menu(&mut self, generation: u64) -> bool {
        self.shell.retire_window_menu(generation)
    }

    pub(crate) fn window_menu_geometry(&self) -> Option<(i32, i32, u32, u32)> {
        self.shell.window_menu_geometry()
    }

    pub(crate) fn open_window_menu_at(&mut self, id: u64, x: i32, y: i32) -> bool {
        self.shell.open_window_menu_at(id, x, y)
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
    pub(crate) fn shell_mut(&mut self) -> &mut LiveShell {
        &mut self.shell
    }
}

pub(crate) fn launcher_size(width: u32, height: u32) -> (u32, u32) {
    (width.min(960), height.saturating_sub(PANEL_HEIGHT).min(720))
}

pub(crate) fn control_center_size(width: u32, height: u32) -> (u32, u32) {
    (420.min(width), height.saturating_sub(PANEL_HEIGHT))
}

fn role_size(role: SurfaceRole, width: u32, height: u32, panel_edge: PanelEdge) -> (u32, u32) {
    let _ = panel_edge;
    match role {
        SurfaceRole::Desktop | SurfaceRole::Lock => (width, height),
        SurfaceRole::Panel => (width, PANEL_HEIGHT),
        SurfaceRole::Launcher => launcher_size(width, height),
        SurfaceRole::ControlCenter => control_center_size(width, height),
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

    #[test]
    fn ephemeral_focus_loss_hides_control_center_without_requesting_focus_restoration() {
        #[derive(Default)]
        struct RecordingHost(std::sync::Mutex<Vec<ShellCommand>>);
        impl SessionHost for RecordingHost {
            fn dispatch(&self, command: ShellCommand) -> Result<(), SessionRequestError> {
                self.0.lock().unwrap().push(command);
                Ok(())
            }
        }
        let host = Arc::new(RecordingHost::default());
        let mut coordinator =
            InternalShellCoordinator::new(host.clone(), PanelEdge::Bottom).unwrap();
        coordinator.set_outputs(&[InternalOutput {
            x: 0,
            y: 0,
            name: "test".into(),
            width: 1280,
            height: 720,
            scale: 1.0,
        }]);
        coordinator.global_shortcut(nickel_session_protocol::ShortcutAction::ShowControlCenter);
        host.0.lock().unwrap().clear();
        assert!(coordinator.dismiss_ephemeral_on_focus_loss(SurfaceRole::ControlCenter));
        let control = coordinator
            .surface(SurfaceRole::ControlCenter, None)
            .unwrap()
            .id;
        assert!(!coordinator.visible(control));
        assert!(
            host.0.lock().unwrap().is_empty(),
            "focus loss must not issue a restore command"
        );
    }

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

        fn capture_desktop(
            &self,
            _output: Option<&str>,
        ) -> crate::session_host::DesktopCapturePoll {
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
    fn native_keyboard_normalized_gesture_uses_press_epoch_and_blur_cancels_release() {
        use nickel_input::{
            DeviceId, EventOrder, InputEvent, KeyEdge, PointerButton, PointerEvent,
        };
        use nickel_session_protocol::{
            OnScreenKeyboardInput, OnScreenKeyboardSnapshot, ShellSemanticTarget, WindowId,
        };
        struct KeyboardHost(std::sync::Mutex<Vec<(u64, OnScreenKeyboardInput)>>);
        impl SessionHost for KeyboardHost {
            fn dispatch(&self, _: ShellCommand) -> Result<(), SessionRequestError> {
                Ok(())
            }
            fn keyboard_snapshot(&self) -> Result<OnScreenKeyboardSnapshot, SessionRequestError> {
                Ok(OnScreenKeyboardSnapshot {
                    enabled: true,
                    visible: true,
                    epoch: 19,
                    generation: 1,
                    recipient: Some(WindowId(7)),
                    ..Default::default()
                })
            }
            fn configure_keyboard(
                &self,
                _: bool,
                _: bool,
                _: u64,
                _: bool,
                _: bool,
                _: u32,
            ) -> Result<(), SessionRequestError> {
                Ok(())
            }
            fn keyboard_input(
                &self,
                epoch: u64,
                input: OnScreenKeyboardInput,
            ) -> Result<(), SessionRequestError> {
                self.0.lock().unwrap().push((epoch, input));
                Ok(())
            }
        }
        let host = Arc::new(KeyboardHost(std::sync::Mutex::new(Vec::new())));
        let mut coordinator =
            InternalShellCoordinator::new(host.clone(), PanelEdge::Bottom).unwrap();
        coordinator.set_outputs(&[InternalOutput {
            name: "test".into(),
            x: 0,
            y: 0,
            width: 1280,
            height: 1104,
            scale: 1.0,
        }]);
        coordinator.poll(Instant::now());
        let id = coordinator
            .surface(SurfaceRole::OnScreenKeyboard, None)
            .unwrap()
            .id;
        // The session supplies authority geometry before rendering. Resolve a
        // production key after resize so both its hit target and input dispatch
        // use the new dimensions, not the output-derived default role height.
        assert!(coordinator.set_surface_size(id, (1280, 280)));
        assert!(!coordinator.set_surface_size(id, (1280, 280)));
        assert_eq!(
            coordinator.surfaces.get(id).unwrap().logical_size(),
            (1280, 280)
        );
        coordinator.scene(id);
        let target = coordinator
            .shell
            .resolve_semantic_target(&ShellSemanticTarget::OnScreenKeyboard {
                key: "osk-char-97".into(),
            })
            .unwrap();
        let event = |edge| HostBatch {
            events: vec![nickel_ui::HostEvent::Normalized {
                input: InputEvent::Pointer(PointerEvent::Button {
                    device: DeviceId(1),
                    order: EventOrder(1),
                    button: PointerButton::Primary,
                    edge,
                    position: Some(nickel_input::Point {
                        x: f64::from(target.x),
                        y: f64::from(target.y),
                    }),
                }),
                clipboard_text: None,
            }],
            ..Default::default()
        };
        coordinator.step_slot_changes(id, event(KeyEdge::Pressed));
        coordinator.step_slot_changes(id, event(KeyEdge::Released));
        assert_eq!(
            *host.0.lock().unwrap(),
            vec![(19, OnScreenKeyboardInput::Text { text: "a".into() })]
        );
        coordinator.step_slot_changes(id, event(KeyEdge::Pressed));
        coordinator.step_slot_changes(
            id,
            HostBatch {
                window_focused: Some(false),
                ..Default::default()
            },
        );
        coordinator.step_slot_changes(id, event(KeyEdge::Released));
        assert_eq!(host.0.lock().unwrap().len(), 1);
        let touch = |started| HostBatch {
            events: vec![nickel_ui::HostEvent::Normalized {
                input: InputEvent::Touch(if started {
                    nickel_input::TouchEvent::Started {
                        device: DeviceId(2),
                        contact: nickel_input::TouchId(1),
                        order: EventOrder(2),
                        position: nickel_input::Point {
                            x: f64::from(target.x),
                            y: f64::from(target.y),
                        },
                    }
                } else {
                    nickel_input::TouchEvent::Ended {
                        device: DeviceId(2),
                        contact: nickel_input::TouchId(1),
                        order: EventOrder(3),
                        position: nickel_input::Point {
                            x: f64::from(target.x),
                            y: f64::from(target.y),
                        },
                    }
                }),
                clipboard_text: None,
            }],
            ..Default::default()
        };
        coordinator.step_slot_changes(id, touch(true));
        coordinator.step_slot_changes(id, touch(false));
        assert_eq!(host.0.lock().unwrap().len(), 2);
        assert_eq!(host.0.lock().unwrap()[1].0, 19);
        coordinator.step_slot_changes(id, touch(true));
        coordinator.step_slot_changes(
            id,
            HostBatch {
                events: vec![nickel_ui::HostEvent::Normalized {
                    input: InputEvent::Touch(nickel_input::TouchEvent::Cancelled {
                        device: DeviceId(2),
                        contact: nickel_input::TouchId(1),
                        order: EventOrder(4),
                    }),
                    clipboard_text: None,
                }],
                ..Default::default()
            },
        );
        coordinator.step_slot_changes(id, touch(false));
        assert_eq!(host.0.lock().unwrap().len(), 2);
    }

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
            x: 0,
            y: 0,
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
            x: 0,
            y: 0,
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
                x: 0,
                y: 0,
                name: "one".into(),
                width: 1920,
                height: 1080,
                scale: 1.0,
            },
            InternalOutput {
                x: 0,
                y: 0,
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
                x: 0,
                y: 0,
                name: "primary".into(),
                width: 1920,
                height: 1080,
                scale: 1.5,
            },
            InternalOutput {
                x: 0,
                y: 0,
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
                x: 0,
                y: 0,
                name: "primary".into(),
                width: 1920,
                height: 1080,
                scale: 1.5,
            },
            InternalOutput {
                x: 0,
                y: 0,
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
                x: 0,
                y: 0,
                name: "left".into(),
                width: 1280,
                height: 720,
                scale: 1.0,
            },
            InternalOutput {
                x: 0,
                y: 0,
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
                x: 0,
                y: 0,
                name: "right".into(),
                width: 1600,
                height: 900,
                scale: 1.0,
            },
            InternalOutput {
                x: 0,
                y: 0,
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
            x: 0,
            y: 0,
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
    fn native_launcher_focus_gain_restores_search_after_scene_creation() {
        let mut coordinator = coordinator();
        coordinator.set_outputs(&[InternalOutput {
            x: 0,
            y: 0,
            name: "nested".into(),
            width: 1280,
            height: 720,
            scale: 1.0,
        }]);
        let launcher = coordinator.surface(SurfaceRole::Launcher, None).unwrap().id;
        coordinator.scene(launcher).unwrap();
        assert!(coordinator.focused_field_lease(launcher).is_none());
        coordinator.step_slot(
            launcher,
            HostBatch {
                window_focused: Some(true),
                ..HostBatch::default()
            },
        );
        assert!(coordinator.focused_field_lease(launcher).is_some());
        coordinator.step_slot(
            launcher,
            HostBatch {
                events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::TextInput(
                    "konsole".into(),
                ))],
                ..HostBatch::default()
            },
        );
        assert!(
            coordinator
                .scene(launcher)
                .unwrap()
                .iter()
                .any(|command| matches!(
                    command, PaintCommand::Text { text, .. } if text == "konsole"
                ))
        );
    }

    #[test]
    fn meta_launcher_toggle_changes_internal_visibility_without_session_transport() {
        let mut coordinator = coordinator();
        coordinator.set_outputs(&[InternalOutput {
            x: 0,
            y: 0,
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
    fn remote_accessibility_menu_retirement_cannot_hide_a_replacement() {
        let mut coordinator = coordinator();
        coordinator.set_outputs(&[InternalOutput {
            x: 0,
            y: 0,
            name: "nested".into(),
            width: 800,
            height: 600,
            scale: 1.0,
        }]);
        coordinator.apply_session_snapshot(nickel_session_protocol::Snapshot {
            windows: vec![nickel_session_protocol::WindowSnapshot {
                id: nickel_session_protocol::WindowId(41),
                application_id: "owned-test".into(),
                title: "Owned test".into(),
                active: true,
                minimized: false,
                maximized: false,
                fullscreen: false,
                geometry: None,
                workspace: nickel_session_protocol::WorkspaceId(1),
            }],
            ..Default::default()
        });
        assert!(coordinator.open_window_menu_at(41, 20, 30));
        let old = coordinator.window_menu_generation().unwrap();
        assert!(coordinator.open_window_menu_at(41, 70, 80));
        let replacement = coordinator.window_menu_generation().unwrap();
        assert!(replacement > old);
        assert!(!coordinator.retire_window_menu(old));
        assert_eq!(coordinator.window_menu_generation(), Some(replacement));
        let menu = coordinator
            .surface(SurfaceRole::WindowContextMenu, None)
            .unwrap()
            .id;
        coordinator.step_slot_changes(
            menu,
            HostBatch {
                events: vec![nickel_ui::HostEvent::Normalized {
                    input: nickel_input::InputEvent::Key(nickel_input::KeyEvent {
                        device: nickel_input::DeviceId(1),
                        order: nickel_input::EventOrder(1),
                        physical: nickel_input::PhysicalKey::Code(nickel_input::KeyCode::Escape),
                        logical: nickel_input::LogicalKey::Named(nickel_input::NamedKey::Escape),
                        location: nickel_input::KeyLocation::Standard,
                        edge: nickel_input::KeyEdge::Pressed,
                        repeat: false,
                        modifiers: nickel_input::ModifierState::default(),
                    }),
                    clipboard_text: None,
                }],
                ..HostBatch::default()
            },
        );
        assert_eq!(coordinator.window_menu_generation(), None);
        assert!(coordinator.open_window_menu_at(41, 70, 80));
        coordinator.step_slot_changes(
            menu,
            HostBatch {
                events: vec![nickel_ui::HostEvent::Ui(
                    nickel_ui::UiEvent::KeyboardNavigateBack,
                )],
                ..HostBatch::default()
            },
        );
        assert_eq!(coordinator.window_menu_generation(), None);
        assert!(coordinator.open_window_menu_at(41, 70, 80));
        let replacement = coordinator.window_menu_generation().unwrap();
        assert!(coordinator.retire_window_menu(replacement));
        assert_eq!(coordinator.window_menu_generation(), None);
        assert!(coordinator.open_window_menu_at(41, 20, 30));
        let menu = coordinator
            .surface(SurfaceRole::WindowContextMenu, None)
            .unwrap()
            .id;
        coordinator.scene(menu).unwrap();
        coordinator.step_slot(
            menu,
            HostBatch {
                events: vec![nickel_ui::HostEvent::Shortcut(nickel_ui::Shortcut::Escape)],
                ..Default::default()
            },
        );
        assert_eq!(coordinator.window_menu_generation(), None);
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
            x: 0,
            y: 0,
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
                x: 0,
                y: 0,
                name: "left".into(),
                width: 1000,
                height: 800,
                scale: 1.0,
            },
            InternalOutput {
                x: 0,
                y: 0,
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
        coordinator.step_slot_changes(
            launcher,
            HostBatch {
                clipboard_text_limit: Some(8),
                events: vec![
                    nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::TextSelectAll),
                    nickel_ui::HostEvent::Normalized {
                        input: nickel_input::InputEvent::Key(nickel_input::KeyEvent {
                            device: nickel_input::DeviceId(1),
                            order: nickel_input::EventOrder(9),
                            physical: nickel_input::PhysicalKey::Code(nickel_input::KeyCode::KeyC),
                            logical: nickel_input::LogicalKey::Character("c".into()),
                            location: nickel_input::KeyLocation::Standard,
                            edge: nickel_input::KeyEdge::Pressed,
                            repeat: false,
                            modifiers: nickel_input::ModifierState::from_sides([
                                nickel_input::Modifier::ControlLeft,
                            ]),
                        }),
                        clipboard_text: None,
                    },
                ],
                ..Default::default()
            },
        );
        assert_eq!(
            coordinator.take_clipboard_result(),
            Some(Ok("terminal".into()))
        );
        coordinator.step_slot_changes(
            launcher,
            HostBatch {
                clipboard_text_limit: Some(4),
                events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::TextCut)],
                ..Default::default()
            },
        );
        assert!(matches!(coordinator.take_clipboard_result(), Some(Err(_))));
        coordinator.step_slot_changes(
            launcher,
            HostBatch {
                clipboard_text_limit: Some(8),
                events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::TextCut)],
                ..Default::default()
            },
        );
        assert_eq!(
            coordinator.take_clipboard_result(),
            Some(Ok("terminal".into())),
            "rejected semantic Cut must preserve the selected text for the accepted Cut"
        );
        coordinator.step_slot_changes(
            launcher,
            HostBatch {
                clipboard_text_limit: Some(8),
                events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::TextInput(
                    "terminal".into(),
                ))],
                ..Default::default()
            },
        );
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
            x: 0,
            y: 0,
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
            x: 0,
            y: 0,
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

    fn opened_screenshot() -> (InternalShellCoordinator, InternalSurfaceId) {
        let mut coordinator = coordinator();
        coordinator.set_outputs(&[InternalOutput {
            x: 0,
            y: 0,
            name: "nested".into(),
            width: 800,
            height: 600,
            scale: 1.0,
        }]);
        coordinator.global_shortcut(nickel_session_protocol::ShortcutAction::ShowScreenshotTool);
        coordinator.poll(Instant::now() + std::time::Duration::from_millis(100));
        let id = coordinator
            .surface(SurfaceRole::Screenshot, None)
            .unwrap()
            .id;
        assert!(coordinator.visible(id));
        coordinator.step_slot_changes(
            id,
            HostBatch {
                window_focused: Some(true),
                ..HostBatch::default()
            },
        );
        (coordinator, id)
    }

    #[test]
    fn native_screenshot_accepts_escape_and_controller_cancel() {
        use nickel_input::{
            DeviceId, EventOrder, InputEvent, KeyEvent, KeyLocation, LogicalKey, ModifierState,
            NamedKey, PhysicalKey,
        };
        for event in [
            nickel_ui::HostEvent::Normalized {
                input: InputEvent::Key(KeyEvent {
                    device: DeviceId(1),
                    order: EventOrder(1),
                    physical: PhysicalKey::Code(nickel_input::KeyCode::Escape),
                    logical: LogicalKey::Named(NamedKey::Escape),
                    location: KeyLocation::Standard,
                    edge: nickel_input::KeyEdge::Pressed,
                    repeat: false,
                    modifiers: ModifierState::default(),
                }),
                clipboard_text: None,
            },
            nickel_ui::HostEvent::Shortcut(nickel_ui::Shortcut::Escape),
            nickel_ui::HostEvent::Controller(nickel_ui::ControllerAction::Cancel),
        ] {
            let (mut coordinator, id) = opened_screenshot();
            coordinator.step_slot_changes(
                id,
                HostBatch {
                    events: vec![event],
                    ..HostBatch::default()
                },
            );
            assert!(!coordinator.visible(id));
        }
    }

    #[test]
    fn native_screenshot_drag_confirmation_and_cancel_use_normalized_pointer_input() {
        use nickel_input::{
            DeviceId, EventOrder, InputEvent, KeyEdge, PointerButton, PointerEvent,
        };
        let (mut coordinator, id) = opened_screenshot();
        let image = coordinator
            .scene(id)
            .unwrap()
            .iter()
            .find_map(|command| match command {
                PaintCommand::Image { bounds, .. } => Some(*bounds),
                _ => None,
            })
            .unwrap();
        let point = |fraction: f32| nickel_input::Point {
            x: f64::from(image.origin.x + image.size.width * fraction),
            y: f64::from(image.origin.y + image.size.height * fraction),
        };
        let send = |coordinator: &mut InternalShellCoordinator, position, edge, order| {
            coordinator.step_slot_changes(
                id,
                HostBatch {
                    events: vec![nickel_ui::HostEvent::Normalized {
                        input: InputEvent::Pointer(PointerEvent::Button {
                            device: DeviceId(1),
                            order: EventOrder(order),
                            position: Some(position),
                            button: PointerButton::Primary,
                            edge,
                        }),
                        clipboard_text: None,
                    }],
                    ..HostBatch::default()
                },
            );
        };
        send(&mut coordinator, point(0.25), KeyEdge::Pressed, 1);
        send(&mut coordinator, point(0.75), KeyEdge::Released, 2);
        for order in [3, 5] {
            send(&mut coordinator, point(0.5), KeyEdge::Pressed, order);
            send(&mut coordinator, point(0.5), KeyEdge::Released, order + 1);
        }
        let scene = coordinator.scene(id).unwrap();
        assert!(scene.iter().any(|command| matches!(command,
            PaintCommand::Text { text, .. } if text == "SELECTION CONFIRMED")));
        let cancel = scene
            .iter()
            .find_map(|command| match command {
                PaintCommand::Text { text, bounds, .. } if text == "Cancel" => Some(*bounds),
                _ => None,
            })
            .expect("confirmed selection exposes Cancel");
        let cancel = nickel_input::Point {
            x: f64::from(cancel.origin.x + cancel.size.width / 2.0),
            y: f64::from(cancel.origin.y + cancel.size.height / 2.0),
        };
        send(&mut coordinator, cancel, KeyEdge::Pressed, 7);
        assert!(coordinator.visible(id));
        send(&mut coordinator, cancel, KeyEdge::Released, 8);
        assert!(!coordinator.visible(id));
    }

    #[test]
    fn production_print_screen_reducer_requests_internal_capture_surface() {
        let mut coordinator = coordinator();
        coordinator.set_outputs(&[InternalOutput {
            x: 0,
            y: 0,
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
