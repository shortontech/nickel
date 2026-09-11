use super::{NickelSession, WindowId};
use crate::session::focus::{KeyboardFocusTarget, PointerFocusTarget};
use nickel_remote_control::diagnostics::{
    InputDeviceDiagnostic, InputDiagnostic, InternalApplicationDiagnostic,
};
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::seat::WaylandFocus;

impl NickelSession {
    pub(super) fn remote_pending_effects_diagnostic(
        &self,
        observation_generation: u64,
        observed_at_us: u64,
    ) -> nickel_remote_control::diagnostics::PendingEffectsDiagnostic {
        use nickel_remote_control::diagnostics::PendingEffectsDiagnostic;

        PendingEffectsDiagnostic {
            observation_generation,
            observed_at_us,
            desktop_scene_updates: self.pending_desktop_scenes.len() as u64,
            image_copy_frames: self.pending_image_copy_frames.len() as u64,
            launch_observations: self.pending_launch_observations.len() as u64,
            output_retirements: self.pending_output_global_retirements.len() as u64,
            shell_focus_pending: self.pending_shell_focus_role.is_some(),
        }
    }

    #[cfg(any(feature = "backend-winit", feature = "backend-udev"))]
    pub(super) fn remote_frame_trace_category(
        &self,
    ) -> Option<nickel_remote_control::frame_trace::FrameTraceCategory> {
        use nickel_remote_control::frame_trace::FrameTraceCategory;
        #[cfg(feature = "backend-udev")]
        if self.native.is_some() {
            return Some(FrameTraceCategory::DrmFrameDispatch);
        }
        #[cfg(feature = "backend-winit")]
        if self.winit_redraw_window.is_some() {
            return Some(FrameTraceCategory::NestedFrameDispatch);
        }
        None
    }

    pub(crate) fn revalidate_remote_frame_trace(&mut self) {
        let protected = self.locked || self.shell_recovery_visible();
        if self
            .remote_frame_trace
            .as_mut()
            .is_some_and(|trace| !trace.revalidate(protected))
        {
            self.remote_frame_trace = None;
        }
    }

    #[cfg(any(feature = "backend-winit", feature = "backend-udev"))]
    pub(crate) fn record_remote_frame_dispatch(
        &mut self,
        output: &str,
        elapsed: std::time::Duration,
    ) {
        let protected = self.locked || self.shell_recovery_visible();
        let generation = self
            .remote_output_generations
            .get(output)
            .map(|(_, generation)| *generation);
        if let (Some(trace), Some(generation)) = (self.remote_frame_trace.as_mut(), generation)
            && !trace.record(protected, generation, elapsed)
        {
            self.remote_frame_trace = None;
        }
    }

    pub(super) fn remote_diagnostic_logs(
        &self,
    ) -> Option<nickel_remote_control::diagnostics::DiagnosticLogSnapshot> {
        use nickel_remote_control::diagnostics::{DiagnosticLogRecord, DiagnosticLogSnapshot};
        let snapshot = nickel_logging::diagnostics::snapshot()?;
        Some(DiagnosticLogSnapshot {
            collecting: snapshot.collecting,
            generation: snapshot.generation,
            observed_at_us: snapshot.observed_at_us,
            evicted: snapshot.evicted,
            contention_drops: snapshot.contention_drops,
            records: snapshot
                .records
                .into_iter()
                .filter_map(|record| {
                    DiagnosticLogRecord::from_static_metadata(
                        record.generation,
                        record.observed_at_us,
                        record.level,
                        record.target,
                        record.file,
                        record.line,
                    )
                })
                .collect(),
        })
    }

    pub(super) fn remote_trace_lifecycle(
        &self,
        permit: &nickel_remote_control::DesktopPermit,
    ) -> Option<nickel_remote_control::diagnostics::TraceLifecycleSnapshot> {
        nickel_remote_control::diagnostics::trace_lifecycle_snapshot(permit, self.start_time)
    }

    pub(crate) fn record_remote_focus_event(&mut self, focused: Option<&KeyboardFocusTarget>) {
        let id = match focused {
            Some(KeyboardFocusTarget::X11(surface)) => {
                self.x11_windows.get(&surface.window_id()).copied()
            }
            Some(KeyboardFocusTarget::Wayland(surface)) => {
                self.surface_windows.get(&surface.id()).copied()
            }
            None => None,
        };
        let state = id
            .filter(|id| !self.remote_window_is_protected(*id))
            .map_or(super::RemoteFocusEventState::Cleared, |id| {
                super::RemoteFocusEventState::Window(id.0)
            });
        self.record_remote_focus_state(state);
    }

    pub(crate) fn record_remote_internal_focus_event(
        &mut self,
        runtime: nickel_ui::InternalSurfaceId,
    ) {
        let state = self
            .internal_shell_surfaces
            .iter()
            .find_map(|(owner, current)| (*current == runtime).then_some(*owner))
            .and_then(|owner| {
                let role = self
                    .internal_shell
                    .as_ref()?
                    .surfaces()
                    .iter()
                    .find(|surface| surface.id == owner)
                    .and_then(|surface| super::remote_shell_event_role(surface.role))?;
                (!self.internal_ui.remote_access_protected(runtime)).then_some(
                    super::RemoteFocusEventState::Shell(runtime.snapshot_token(), role),
                )
            })
            .unwrap_or(super::RemoteFocusEventState::Cleared);
        self.record_remote_focus_state(state);
    }

    pub(crate) fn record_remote_focus_cleared(&mut self) {
        self.record_remote_focus_state(super::RemoteFocusEventState::Cleared);
    }

    fn record_remote_focus_state(&mut self, state: super::RemoteFocusEventState) {
        use nickel_remote_control::desktop_events::DesktopEventKind;
        if self.remote_focus_event_state == Some(state) {
            return;
        }
        self.remote_focus_event_state = Some(state);
        let event = match state {
            super::RemoteFocusEventState::Window(window_id) => {
                DesktopEventKind::KeyboardFocusChanged { window_id }
            }
            super::RemoteFocusEventState::Shell(surface_generation, role) => {
                DesktopEventKind::ShellKeyboardFocusChanged {
                    surface_generation,
                    role,
                }
            }
            super::RemoteFocusEventState::Cleared => DesktopEventKind::KeyboardFocusCleared,
        };
        self.remote_desktop_events.record(
            event,
            self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
        );
    }

    pub(super) fn remote_list_outputs(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
    ) -> Result<nickel_remote_control::diagnostics::OutputInventory, String> {
        use nickel_remote_control::diagnostics::{
            MAX_DIAGNOSTIC_OUTPUTS, OutputDiagnostic, OutputInventory,
        };
        self.refresh_output_topology_generation();
        permit.check_live()?;
        let mut outputs = Vec::new();
        let mut truncated = false;
        for output in self.protocol_outputs() {
            let Some(identity) = self.remote_output_identity(output.name.clone()) else {
                continue;
            };
            let evidence = nickel_remote_control::leases::ResourceEvidence {
                window: None,
                surface: None,
                verified_application: None,
                output: Some(&identity),
                authorized_surface_ancestors: &[],
                protected: self.locked || self.shell_recovery_visible(),
            };
            // Count only authorized omissions; unrelated outputs must not leak
            // through inventory bounds or truncation metadata.
            if permit.with_resource(&evidence, || Ok(())).is_err() {
                continue;
            }
            if outputs.len() == MAX_DIAGNOSTIC_OUTPUTS || output.name.len() > 512 {
                truncated = true;
                continue;
            }
            let geometry = |value: nickel_session_protocol::Geometry| {
                [value.x, value.y, value.width, value.height]
            };
            outputs.push(OutputDiagnostic {
                name: output.name,
                generation: identity.generation,
                geometry: geometry(output.geometry),
                work_area: geometry(output.work_area),
                scale_120: output.scale_120,
                primary: output.primary,
                enabled: output.enabled,
            });
        }
        // An expired/revoked lease must not turn into a successful empty or
        // partially filtered observation after per-output authorization.
        permit.check_live()?;
        self.remote_observation_generation = self.remote_observation_generation.saturating_add(1);
        Ok(OutputInventory {
            observation_generation: self.remote_observation_generation,
            observed_at_us: self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
            topology_generation: self.output_topology_generation,
            outputs,
            truncated,
        })
    }

    pub(super) fn remote_workspace_action(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
        action: nickel_remote_control::diagnostics::WorkspaceAction,
    ) -> Result<nickel_remote_control::diagnostics::WorkspaceOutcome, String> {
        use nickel_core::workspaces::WorkspaceId;
        use nickel_remote_control::diagnostics::{
            MAX_DIAGNOSTIC_WINDOWS, WorkspaceAction, WorkspaceOutcome,
        };
        let evidence = nickel_remote_control::leases::ResourceEvidence {
            window: None,
            surface: None,
            verified_application: None,
            output: None,
            authorized_surface_ancestors: &[],
            protected: self.locked || self.shell_recovery_visible(),
        };
        // Empty resource evidence requires full-session authority, without
        // granting or requiring the separate diagnostic capability.
        permit.with_resource(&evidence, || Ok(()))?;
        let mut created_workspace = None;
        if action != WorkspaceAction::List {
            let controller_busy = self.poll_remote_controller_ownership();
            let transition = permit.with_input(&evidence, || {
                if controller_busy
                    || self.remote_held_keyboard.is_some()
                    || self.remote_held_pointer.is_some()
                    || !self.active_touch_slots.is_empty()
                    || self.internal_ui.pointer_interaction_active()
                    || self.internal_ui.desktop_keyboard_interaction_active()
                    || self
                        .internal_ui
                        .focused()
                        .is_some_and(|surface| self.internal_ui.remote_access_protected(surface))
                    || self.seat.get_keyboard().is_some_and(|keyboard| {
                        !keyboard.pressed_keys().is_empty() || keyboard.is_grabbed()
                    })
                    || self
                        .seat
                        .get_pointer()
                        .is_some_and(|pointer| pointer.is_grabbed())
                    || self
                        .internal_shell
                        .as_ref()
                        .is_some_and(|shell| shell.pointer_interaction_active())
                {
                    return Err("shared input is busy or unavailable".into());
                }
                if self
                    .workspaces
                    .ordered()
                    .iter()
                    .map(|workspace| workspace.windows.len())
                    .sum::<usize>()
                    > MAX_DIAGNOSTIC_WINDOWS
                {
                    return Err("workspace inventory exceeds the operation bound".into());
                }
                let mut next = self.workspaces.clone();
                let unavailable = |_| "workspace is unavailable".to_owned();
                let transition = match action {
                    WorkspaceAction::Create => {
                        created_workspace = Some(next.create().map_err(unavailable)?.0);
                        None
                    }
                    WorkspaceAction::Switch { workspace }
                        if WorkspaceId(workspace) == next.active() =>
                    {
                        None
                    }
                    WorkspaceAction::Switch { workspace } => Some(
                        next.switch_to(WorkspaceId(workspace), None)
                            .map_err(unavailable)?,
                    ),
                    WorkspaceAction::Remove { workspace } => {
                        Some(next.remove(WorkspaceId(workspace)).map_err(unavailable)?)
                    }
                    WorkspaceAction::List => unreachable!(),
                };
                // Check both visibility/focus effects and hidden membership
                // transfers before replacing the production workspace state.
                let affected = self
                    .workspaces
                    .ordered()
                    .iter()
                    .flat_map(|workspace| workspace.windows.iter())
                    .filter(|window| {
                        self.workspaces.workspace_for(window) != next.workspace_for(window)
                            || transition.as_ref().is_some_and(|transition| {
                                transition.hide.contains(window)
                                    || transition.show.contains(window)
                                    || transition.focus.as_ref() == Some(*window)
                            })
                    });
                if affected.into_iter().any(|window| {
                    !self.shell_owned_windows.contains(window)
                        && self.remote_window_is_protected(*window)
                }) {
                    return Err("workspace contains an unavailable resource".into());
                }
                self.workspaces = next;
                Ok(transition)
            })?;
            // Production reconciliation can consult authority, so it must run
            // outside the control lock on this same owner thread.
            if let Some(transition) = transition {
                self.apply_workspace_transition(transition);
            } else {
                self.notify_workspace_state();
            }
        }
        permit.with_resource(&evidence, || {
            let mut windows = self
                .remote_protocol_windows()
                .into_iter()
                .filter(|window| !self.remote_window_is_protected(WindowId(window.id.0)))
                .take(MAX_DIAGNOSTIC_WINDOWS + 1)
                .map(|window| self.remote_window_summary(window))
                .collect::<Vec<_>>();
            let truncated = windows.len() > MAX_DIAGNOSTIC_WINDOWS;
            windows.truncate(MAX_DIAGNOSTIC_WINDOWS);
            self.remote_observation_generation =
                self.remote_observation_generation.saturating_add(1);
            Ok(WorkspaceOutcome {
                requested: action,
                created_workspace,
                observation_generation: self.remote_observation_generation,
                observed_at_us: self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
                workspaces: self.remote_workspace_diagnostics(&windows),
                truncated,
            })
        })
    }

    pub(super) fn remote_move_window_to_workspace(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
        evidence: &nickel_remote_control::leases::ResourceEvidence<'_>,
        window: WindowId,
        workspace: u64,
    ) -> Result<nickel_remote_control::window_actions::WindowOutcome, String> {
        let controller_busy = self.poll_remote_controller_ownership();
        let transition = permit.with_input(evidence, || {
            if controller_busy
                || self.remote_held_keyboard.is_some()
                || self.remote_held_pointer.is_some()
                || !self.active_touch_slots.is_empty()
                || self.internal_ui.pointer_interaction_active()
                || self.internal_ui.desktop_keyboard_interaction_active()
                || self.seat.get_keyboard().is_some_and(|keyboard| {
                    !keyboard.pressed_keys().is_empty() || keyboard.is_grabbed()
                })
                || self
                    .seat
                    .get_pointer()
                    .is_some_and(|pointer| pointer.is_grabbed())
                || self
                    .internal_shell
                    .as_ref()
                    .is_some_and(|shell| shell.pointer_interaction_active())
            {
                return Err("shared input is busy or unavailable".into());
            }
            self.workspaces
                .move_window(&window, nickel_core::workspaces::WorkspaceId(workspace))
                .map_err(|_| "workspace or window is unavailable".to_owned())
        })?;
        // The membership change is committed. Reconcile it on the same owner
        // before another request, outside the non-reentrant authority lock.
        self.apply_workspace_transition(transition);
        let observed = self
            .remote_protocol_windows()
            .into_iter()
            .find(|entry| entry.id.0 == window.0)
            .map(|entry| self.remote_window_summary(entry));
        Ok(
            nickel_remote_control::window_actions::WindowOutcome::observed(
                nickel_remote_control::window_actions::WindowAction::MoveToWorkspace { workspace },
                observed,
            ),
        )
    }

    pub(super) fn remote_workspace_diagnostics(
        &self,
        windows: &[nickel_remote_control::WindowSummary],
    ) -> Vec<nickel_remote_control::diagnostics::WorkspaceDiagnostic> {
        use nickel_remote_control::diagnostics::{
            MAX_DIAGNOSTIC_WINDOWS, MAX_DIAGNOSTIC_WORKSPACES, WorkspaceDiagnostic,
        };
        let allowed = windows
            .iter()
            .take(MAX_DIAGNOSTIC_WINDOWS)
            .filter(|window| !self.remote_window_is_protected(WindowId(window.generation)))
            .collect::<Vec<_>>();
        self.workspaces
            .ordered()
            .iter()
            .take(MAX_DIAGNOSTIC_WORKSPACES)
            .map(|workspace| {
                let members = allowed
                    .iter()
                    .filter(|window| {
                        self.workspaces.workspace_for(&WindowId(window.generation))
                            == Some(workspace.id)
                    })
                    .map(|window| window.id.clone())
                    .collect::<Vec<_>>();
                let last_focused_window = workspace
                    .last_focused
                    .map(|window| window.0.to_string())
                    .filter(|window| members.contains(window));
                WorkspaceDiagnostic {
                    id: workspace.id.0,
                    active: workspace.id == self.workspaces.active(),
                    windows: members,
                    last_focused_window,
                }
            })
            .collect()
    }

    pub(super) fn remote_shell_behavior_transaction(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
        transaction: nickel_session_protocol::ShellBehaviorTransaction,
        prepared: super::remote_settings::PreparedShellBehavior,
    ) -> Result<nickel_remote_control::diagnostics::ShellBehaviorDiagnostic, String> {
        // Output housekeeping can consult control state, so do it before taking
        // the authority lock. The compare-and-set commit checks the resulting version.
        self.refresh_output_topology_generation();
        let controller_busy = self.poll_remote_controller_ownership();
        let protected = self.locked
            || self.shell_recovery_visible()
            || self
                .internal_ui
                .focused()
                .is_some_and(|surface| self.internal_ui.remote_access_protected(surface));
        let (effective, transitions) = permit.with_debug_input_deadline(protected, |deadline| {
            if controller_busy {
                return Err("local controller input is held, pending, or unavailable".into());
            }
            if self.remote_held_keyboard.is_some()
                || self.remote_held_pointer.is_some()
                || !self.active_touch_slots.is_empty()
                || self.internal_ui.pointer_interaction_active()
                || self.internal_ui.desktop_keyboard_interaction_active()
                || self.seat.get_keyboard().is_some_and(|keyboard| {
                    !keyboard.pressed_keys().is_empty() || keyboard.is_grabbed()
                })
                || self
                    .seat
                    .get_pointer()
                    .is_some_and(|pointer| pointer.is_grabbed())
                || self
                    .internal_shell
                    .as_ref()
                    .is_some_and(|shell| shell.pointer_interaction_active())
            {
                return Err("shared input is busy".into());
            }
            if transaction.topology_generation != self.output_topology_generation {
                return Err("settings transaction rejected: stale state or invalid value".into());
            }
            if self
                .workspaces
                .ordered()
                .iter()
                .map(|workspace| workspace.windows.len())
                .sum::<usize>()
                > nickel_remote_control::diagnostics::MAX_DIAGNOSTIC_WINDOWS
            {
                return Err("workspace state exceeds settings transaction budget".into());
            }
            // Prepare production workspace policy on a bounded copy first. A
            // failed file commit cannot partially mutate the live workspace owner.
            let mut workspaces = self.workspaces.clone();
            let transitions = workspaces
                .set_count(usize::from(prepared.requested.desktop_count))
                .map_err(|_| "workspace policy rejected settings transaction")?;
            let effective = nickel_session_protocol::ShellBehaviorSnapshot {
                bar_on_all_displays: prepared.requested.bar_on_all_displays,
                all_windows_on_every_bar: prepared.requested.all_windows_on_every_bar,
                desktop_count: prepared.requested.desktop_count,
                topology_generation: self.output_topology_generation,
            };
            prepared.commit(deadline.deadline(), || {
                permit.check_commit_boundary(deadline)
            })?;
            self.workspaces = workspaces;
            Ok((effective, transitions))
        })?;
        // Authority covered the configuration/workspace-policy commit. Reconcile
        // that committed state now, rather than queueing another remote mutation.
        // Shell/indicator polling must run outside the non-reentrant control lock.
        self.reconcile_shell_behavior_commit(&effective, transitions);
        self.remote_observation_generation = self.remote_observation_generation.saturating_add(1);
        Ok(
            nickel_remote_control::diagnostics::ShellBehaviorDiagnostic {
                observation_generation: self.remote_observation_generation,
                observed_at_us: self
                    .start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
                topology_generation: effective.topology_generation,
                bar_on_all_displays: effective.bar_on_all_displays,
                all_windows_on_every_bar: effective.all_windows_on_every_bar,
                configured_desktop_count: effective.desktop_count,
                runtime_desktop_count: self.workspaces.ordered().len(),
            },
        )
    }

    pub(super) fn remote_shell_behavior_diagnostic(
        &self,
        observation_generation: u64,
        observed_at_us: u64,
    ) -> nickel_remote_control::diagnostics::ShellBehaviorDiagnostic {
        let settings = self.protocol_shell_behavior();
        nickel_remote_control::diagnostics::ShellBehaviorDiagnostic {
            observation_generation,
            observed_at_us,
            topology_generation: settings.topology_generation,
            bar_on_all_displays: settings.bar_on_all_displays,
            all_windows_on_every_bar: settings.all_windows_on_every_bar,
            configured_desktop_count: settings.desktop_count,
            runtime_desktop_count: self.workspaces.ordered().len(),
        }
    }

    pub(super) fn remote_internal_renderer_diagnostics(
        &self,
        applications: &[InternalApplicationDiagnostic],
        observed_at_us: u64,
    ) -> Vec<nickel_remote_control::diagnostics::InternalRendererDiagnostic> {
        self.remote_renderer_diagnostics(
            applications
                .iter()
                .map(|surface| (surface.id.as_str(), surface.generation)),
            observed_at_us,
        )
    }

    pub(super) fn remote_shell_renderer_diagnostics(
        &self,
        observed_at_us: u64,
    ) -> Vec<nickel_remote_control::diagnostics::InternalRendererDiagnostic> {
        // Resolve the current production projection here, so callers cannot supply
        // stale visible records after lock, hiding, or a protection change.
        let (surfaces, _) = self.remote_shell_surface_diagnostics();
        self.remote_renderer_diagnostics(
            surfaces
                .iter()
                .map(|surface| (surface.id.as_str(), surface.generation)),
            observed_at_us,
        )
    }

    fn remote_renderer_diagnostics<'a>(
        &self,
        surfaces: impl Iterator<Item = (&'a str, u64)>,
        observed_at_us: u64,
    ) -> Vec<nickel_remote_control::diagnostics::InternalRendererDiagnostic> {
        use crate::session::backend::InternalUiRendererMode;
        use crate::session::internal_ui::{InternalUiFallbackReason, InternalUiPresentationMode};
        use nickel_remote_control::diagnostics::{
            InternalRendererDiagnostic, RendererFallbackReason, RendererPolicy,
        };
        surfaces
            .filter_map(|(id, generation)| {
                let surface = self.internal_ui.resolve_surface_identity(id, generation)?;
                if self.internal_ui.remote_access_protected(surface) {
                    return None;
                }
                let state = self.internal_ui.renderer_diagnostics(surface)?;
                let mode = if !self.internal_ui.is_visible(surface) {
                    "suspended"
                } else {
                    match self.internal_ui.presentation_mode(surface)? {
                        InternalUiPresentationMode::GpuSolid => "gpu_elements",
                        InternalUiPresentationMode::RasterFallback => "raster_fallback",
                    }
                };
                Some(InternalRendererDiagnostic {
                    surface: id.to_owned(),
                    surface_generation: generation,
                    observed_at_us,
                    mode: mode.into(),
                    configured_mode: match state.configured_mode {
                        InternalUiRendererMode::Gpu => RendererPolicy::Gpu,
                        InternalUiRendererMode::Software => RendererPolicy::Software,
                    },
                    fallback_reason: state.fallback_reason.map(|reason| match reason {
                        InternalUiFallbackReason::RequestedSoftware => {
                            RendererFallbackReason::RequestedSoftware
                        }
                        InternalUiFallbackReason::UnsupportedCommands => {
                            RendererFallbackReason::UnsupportedCommands
                        }
                        InternalUiFallbackReason::ElementBudget => {
                            RendererFallbackReason::ElementBudget
                        }
                        InternalUiFallbackReason::TextureImportFailure => {
                            RendererFallbackReason::TextureImportFailure
                        }
                    }),
                    gpu_frames: state.gpu_frames,
                    fallback_frames: state.fallback_frames,
                    software_frame_bytes: state.software_frame_bytes as u64,
                    fallback_raster_bytes: state.fallback_raster_bytes as u64,
                    fallback_buffer_creations: state.fallback_buffer_creations,
                    fallback_buffer_reuses: state.fallback_buffer_reuses,
                    fallback_upload_damage_bytes: state.fallback_upload_damage_bytes,
                    fallback_full_repaints: state.fallback_full_repaints,
                    fallback_partial_repaints: state.fallback_partial_repaints,
                    texture_import_failures: state.texture_import_failures,
                    fallback_import_failures: state.fallback_import_failures,
                    presentation_generation: None,
                    presentation_failures: None,
                })
            })
            .collect()
    }

    pub(super) fn remote_semantic_action(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
        request: nickel_remote_control::semantics::SemanticActionRequest,
    ) -> Result<bool, String> {
        use nickel_remote_control::leases::{ResourceEvidence, ResourceId};
        request.validate().map_err(str::to_owned)?;
        let controller_busy = self.poll_remote_controller_ownership();
        let window = WindowId(request.window_generation);
        let surface = self.internal_window_surfaces.get(&window).copied();
        let window_resource = ResourceId {
            id: request.window_id,
            generation: request.window_generation,
        };
        let surface_resource = surface.map(|id| ResourceId {
            id: format!("internal:{}", id.snapshot_token()),
            generation: id.snapshot_token(),
        });
        let output = self.remote_window_output(window);
        let application = self.remote_verified_application(window);
        let evidence = ResourceEvidence {
            window: Some(&window_resource),
            surface: surface_resource.as_ref(),
            output: output.as_ref(),
            verified_application: application.as_deref(),
            authorized_surface_ancestors: &[],
            protected: self.locked || self.remote_window_is_protected(window),
        };
        let result = permit.with_input(&evidence, || {
            let surface = surface.ok_or("window semantics are unavailable on this backend")?;
            if surface.snapshot_token() != request.surface_generation
                || surface_resource
                    .as_ref()
                    .is_none_or(|id| id.id != request.surface_id)
            {
                return Err("stale semantic surface".into());
            }
            if controller_busy {
                return Err("local controller input is held, pending, or unavailable".into());
            }
            if self.remote_held_keyboard.is_some()
                || self.remote_held_pointer.is_some()
                || !self.active_touch_slots.is_empty()
                || self.seat.get_keyboard().is_some_and(|keyboard| {
                    !keyboard.pressed_keys().is_empty() || keyboard.is_grabbed()
                })
                || self
                    .seat
                    .get_pointer()
                    .is_some_and(|pointer| pointer.is_grabbed())
                || self
                    .internal_shell
                    .as_ref()
                    .is_some_and(|shell| shell.pointer_interaction_active())
            {
                return Err("shared input is busy".into());
            }
            let action = semantic_mutation_action(request.action);
            self.internal_ui.perform_bounded_application_action(
                surface,
                request.tree_generation,
                request.node as usize,
                action,
            )
        });
        // The authority lock must be released before polling shell effects, which
        // can themselves consult or modify control state.
        self.wake_internal_shell();
        result
    }

    pub(super) fn remote_surface_semantic_action(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
        request: nickel_remote_control::semantics::SurfaceSemanticActionRequest,
    ) -> Result<super::remote_shell_actions::ShellActionPlan, String> {
        use nickel_remote_control::leases::{ResourceEvidence, ResourceId};
        request.validate().map_err(str::to_owned)?;
        let controller_busy = self.poll_remote_controller_ownership();
        let identity = ResourceId {
            id: request.surface_id,
            generation: request.surface_generation,
        };
        let (runtime, output) = self.surface_capture_evidence(&identity)?;
        if output.is_none() {
            return Err("shell output membership is unavailable".into());
        }
        let owner = self
            .internal_shell
            .as_ref()
            .ok_or("shell unavailable")?
            .surfaces()
            .iter()
            .find(|entry| self.internal_shell_surfaces.get(&entry.id) == Some(&runtime))
            .ok_or("shell surface has retired")?
            .id;
        let ancestors = self.remote_surface_ancestors(&identity);
        let evidence = ResourceEvidence {
            window: None,
            surface: Some(&identity),
            output: output.as_ref(),
            verified_application: None,
            authorized_surface_ancestors: &ancestors,
            protected: false,
        };
        let result = permit.with_input(&evidence, || {
            self.remote_semantic_input_idle(controller_busy)?;
            let limit = self.internal_ui.clipboard_limit();
            let shell = self.internal_shell.as_mut().ok_or("shell unavailable")?;
            let mut outcome = shell.perform_bounded_shell_action(
                owner,
                request.tree_generation,
                request.node as usize,
                semantic_mutation_action(request.action),
                limit,
            )?;
            if !outcome.host.semantic_failures.is_empty() {
                return Err("semantic action rejected by application".into());
            }
            if !outcome.host.failures.is_empty() || !outcome.host.completion_failures.is_empty() {
                return Err(
                    "semantic action completed with local effect failure; do not retry".into(),
                );
            }
            if let Some(text) = outcome.host.clipboard_text.take() {
                self.publish_native_text_selection(text)
                    .map_err(|_| "semantic clipboard effect failed; do not retry".to_owned())?;
            }
            Ok(outcome)
        });
        let result = result.and_then(|outcome| {
            use super::remote_shell_actions::{ShellActionPlan, ShellActionStep};
            let changed = outcome.host.changed;
            let mut steps = Vec::new();
            for effect in outcome.effects {
                use super::remote_launcher_favorites::SemanticFavoriteAction;
                let favorite = match &effect {
                    crate::live_shell::remote_semantics::RemoteShellEffect::Launcher(
                        crate::launcher_view::LauncherShellEffect::TogglePin(application),
                    )
                    | crate::live_shell::remote_semantics::RemoteShellEffect::Panel(
                        crate::live_shell::PanelAction::ToggleTaskPin(application),
                        _,
                    ) => Some(SemanticFavoriteAction::Toggle(application.clone())),
                    crate::live_shell::remote_semantics::RemoteShellEffect::Panel(
                        crate::live_shell::PanelAction::MoveTaskPinLeft(application),
                        _,
                    ) => Some(SemanticFavoriteAction::MoveLeft(application.clone())),
                    crate::live_shell::remote_semantics::RemoteShellEffect::Panel(
                        crate::live_shell::PanelAction::MoveTaskPinRight(application),
                        _,
                    ) => Some(SemanticFavoriteAction::MoveRight(application.clone())),
                    _ => None,
                };
                if let Some(action) = favorite {
                    steps.push(ShellActionStep::Favorite(action));
                    continue;
                }
                let (_, current_output) = self.surface_capture_evidence(&identity)?;
                let ancestors = self.remote_surface_ancestors(&identity);
                let resource = ResourceEvidence {
                    window: None,
                    surface: Some(&identity),
                    output: current_output.as_ref(),
                    verified_application: None,
                    authorized_surface_ancestors: &ancestors,
                    protected: false,
                };
                let selected = permit.with_input(&resource, || {
                    self.internal_shell
                        .as_mut()
                        .ok_or("shell unavailable")?
                        .resolve_remote_installed_launch(&effect)
                })?;
                if let Some(application) = selected {
                    let (catalog_generation, catalog, _) =
                        crate::platform::installed_application_signatures();
                    if !catalog.iter().any(|current| current == &application) {
                        return Err(
                            "selected shell application is not in the current installed catalog"
                                .into(),
                        );
                    }
                    steps.push(ShellActionStep::Launch {
                        application,
                        catalog_generation,
                    });
                    if matches!(
                        &effect,
                        crate::live_shell::remote_semantics::RemoteShellEffect::Launcher(_)
                    ) {
                        steps.push(ShellActionStep::Command(
                            crate::platform::ShellCommand::Hide,
                        ));
                    }
                    continue;
                }
                if let crate::live_shell::remote_semantics::RemoteShellEffect::Control(action) =
                    &effect
                    && matches!(
                        action,
                        crate::control_view::ControlAction::SetAudioVolume(_)
                            | crate::control_view::ControlAction::SelectAudioDevice { .. }
                            | crate::control_view::ControlAction::SetWifiEnabled(_)
                            | crate::control_view::ControlAction::SetBluetoothPowered(_)
                            | crate::control_view::ControlAction::SetBluetoothDiscovery(_)
                            | crate::control_view::ControlAction::ActivateWifi { .. }
                            | crate::control_view::ControlAction::ToggleBluetoothDevice { .. }
                    )
                {
                    steps.push(
                        self.prepare_shell_device_action(
                            permit,
                            &identity,
                            current_output
                                .as_ref()
                                .ok_or("shell output membership is unavailable")?,
                            action.clone(),
                        )?,
                    );
                    continue;
                }
                let commands = permit.with_input(&resource, || {
                    self.internal_shell
                        .as_mut()
                        .ok_or("shell unavailable")?
                        .stage_remote_shell_effect(effect)
                })?;
                for command in commands {
                    self.preflight_remote_shell_command(permit, &identity, &command)?;
                    steps.push(ShellActionStep::Command(command));
                }
            }
            if steps.len() > 8 {
                return Err("shell effect budget exceeded".into());
            }
            let tree_generation = self
                .remote_surface_semantics(permit, &identity.id, identity.generation)?
                .tree_generation;
            Ok(ShellActionPlan {
                origin: identity.clone(),
                output: output
                    .clone()
                    .ok_or("shell output membership is unavailable")?,
                changed,
                tree_generation,
                steps,
            })
        });
        self.wake_internal_shell();
        result
    }

    pub(super) fn preflight_remote_shell_command(
        &self,
        permit: &nickel_remote_control::DesktopPermit,
        origin: &nickel_remote_control::leases::ResourceId,
        command: &crate::platform::ShellCommand,
    ) -> Result<nickel_remote_control::leases::ResourceId, String> {
        use crate::platform::ShellCommand;
        use nickel_remote_control::leases::{ResourceEvidence, ResourceId};
        let (_, output) = self.surface_capture_evidence(origin)?;
        let output = output.ok_or("shell output membership is unavailable")?;
        if !matches!(
            command,
            ShellCommand::Show
                | ShellCommand::Hide
                | ShellCommand::FocusControlCenter
                | ShellCommand::RestoreApplicationFocus
        ) {
            return Err("staged shell command is unavailable".into());
        }
        // A new top-level surface is not an implicit child of a panel-only grant.
        if matches!(
            command,
            ShellCommand::Show | ShellCommand::FocusControlCenter
        ) {
            permit.with_resource(
                &ResourceEvidence {
                    window: None,
                    surface: None,
                    output: Some(&output),
                    verified_application: None,
                    authorized_surface_ancestors: &[],
                    protected: false,
                },
                || Ok(()),
            )?;
        }
        // Visibility changes can retire another output's existing overlay. The
        // invoking panel is not evidence of authority over that overlay.
        if let Some(shell) = self.internal_shell.as_ref() {
            for entry in shell.surfaces() {
                use crate::winit_shell::SurfaceRole;
                let affected = match command {
                    ShellCommand::Show => matches!(
                        entry.role,
                        SurfaceRole::Launcher | SurfaceRole::ControlCenter
                    ),
                    ShellCommand::Hide => entry.role == SurfaceRole::Launcher,
                    ShellCommand::FocusControlCenter | ShellCommand::RestoreApplicationFocus => {
                        entry.role == SurfaceRole::ControlCenter
                    }
                    _ => false,
                };
                if !affected || !shell.visible(entry.id) {
                    continue;
                }
                let Some(runtime) = self.internal_shell_surfaces.get(&entry.id) else {
                    return Err("affected shell surface identity is unavailable".into());
                };
                if !self.internal_ui.is_visible(*runtime) {
                    continue;
                }
                let identity = ResourceId {
                    id: format!("internal:{}", runtime.snapshot_token()),
                    generation: runtime.snapshot_token(),
                };
                self.with_surface_capture_authority(&identity, permit, || Ok(()))?;
            }
        }
        let restore = match command {
            ShellCommand::Hide => self.launcher_restore_window,
            ShellCommand::RestoreApplicationFocus => self.shell_focus_restore_window,
            _ => None,
        };
        if let Some(window) = restore {
            let resource = ResourceId {
                id: window.0.to_string(),
                generation: window.0,
            };
            let target_output = self.remote_window_output(window);
            let application = self.remote_verified_application(window);
            permit.with_resource(
                &ResourceEvidence {
                    window: Some(&resource),
                    surface: None,
                    output: target_output.as_ref(),
                    verified_application: application.as_deref(),
                    authorized_surface_ancestors: &[],
                    protected: self.remote_window_is_protected(window),
                },
                || Ok(()),
            )?;
        }
        Ok(output)
    }

    pub(super) fn remote_replay_shell_command(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
        origin: &nickel_remote_control::leases::ResourceId,
        command: crate::platform::ShellCommand,
    ) -> Result<(), String> {
        use crate::platform::ShellCommand;
        use nickel_remote_control::leases::ResourceEvidence;
        let output = self.preflight_remote_shell_command(permit, origin, &command)?;
        let controller_busy = self.poll_remote_controller_ownership();
        let ancestors = self.remote_surface_ancestors(origin);
        let evidence = ResourceEvidence {
            window: None,
            surface: Some(origin),
            output: Some(&output),
            verified_application: None,
            authorized_surface_ancestors: &ancestors,
            protected: false,
        };
        permit.with_input(&evidence, || {
            self.remote_semantic_input_idle(controller_busy)?;
            match command {
                ShellCommand::Show => {
                    self.set_launcher_visible_on_output(true, Some(output.id.clone()))
                }
                ShellCommand::Hide => {
                    self.set_launcher_visible_on_output(false, Some(output.id.clone()))
                }
                ShellCommand::FocusControlCenter => {
                    self.internal_shell
                        .as_mut()
                        .ok_or("shell unavailable")?
                        .apply_control_visibility(true);
                    self.sync_internal_shell();
                }
                ShellCommand::RestoreApplicationFocus => {
                    self.internal_shell
                        .as_mut()
                        .ok_or("shell unavailable")?
                        .apply_control_visibility(false);
                    self.sync_internal_shell();
                    self.restore_application_focus();
                }
                _ => return Err("staged shell command is unavailable".into()),
            }
            Ok(())
        })
    }

    pub(super) fn remote_semantic_input_idle(&self, controller_busy: bool) -> Result<(), String> {
        if controller_busy {
            return Err("local controller input is held, pending, or unavailable".into());
        }
        if self.remote_held_keyboard.is_some()
            || self.remote_held_pointer.is_some()
            || !self.active_touch_slots.is_empty()
            || self.seat.get_keyboard().is_some_and(|keyboard| {
                !keyboard.pressed_keys().is_empty() || keyboard.is_grabbed()
            })
            || self
                .seat
                .get_pointer()
                .is_some_and(|pointer| pointer.is_grabbed())
            || self.internal_ui.pointer_interaction_active()
            || self.internal_ui.desktop_keyboard_interaction_active()
            || self
                .internal_shell
                .as_ref()
                .is_some_and(|shell| shell.pointer_interaction_active())
        {
            return Err("shared input is busy".into());
        }
        Ok(())
    }

    pub(super) fn remote_surface_semantics(
        &self,
        permit: &nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<nickel_remote_control::semantics::SurfaceSemanticSnapshot, String> {
        let identity = nickel_remote_control::leases::ResourceId {
            id: id.to_owned(),
            generation,
        };
        let (runtime, _) = self.surface_capture_evidence(&identity)?;
        self.with_surface_capture_authority(&identity, permit, || {
            let shell = self.internal_shell.as_ref().ok_or("shell unavailable")?;
            let entry = shell
                .surfaces()
                .iter()
                .find(|entry| self.internal_shell_surfaces.get(&entry.id) == Some(&runtime))
                .ok_or("shell surface has retired")?;
            let (tree_generation, projection) = shell.bounded_shell_semantics(entry.id)?;
            Ok(nickel_remote_control::semantics::SurfaceSemanticSnapshot {
                surface: identity.id.clone(),
                surface_generation: generation,
                tree_generation,
                observed_at_us: self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
                nodes: semantic_projection(projection)?,
            })
        })
    }

    pub(super) fn remote_window_semantics(
        &self,
        permit: &nickel_remote_control::DesktopPermit,
        id: &str,
        generation: u64,
    ) -> Result<nickel_remote_control::semantics::SemanticSnapshot, String> {
        use nickel_remote_control::{
            leases::{ResourceEvidence, ResourceId},
            semantics::*,
        };
        let numeric = id.parse::<u64>().map_err(|_| "invalid window identity")?;
        if numeric != generation || numeric.to_string() != id {
            return Err("stale window identity".into());
        }
        let window = WindowId(numeric);
        let surface = self.internal_window_surfaces.get(&window).copied();
        let window_resource = ResourceId {
            id: id.to_owned(),
            generation,
        };
        let surface_resource = surface.map(|surface| ResourceId {
            id: format!("internal:{}", surface.snapshot_token()),
            generation: surface.snapshot_token(),
        });
        let output = self.remote_window_output(window);
        let application = self.remote_verified_application(window);
        let evidence = ResourceEvidence {
            window: Some(&window_resource),
            surface: surface_resource.as_ref(),
            output: output.as_ref(),
            verified_application: application.as_deref(),
            authorized_surface_ancestors: &[],
            protected: self.locked || self.remote_window_is_protected(window),
        };
        permit.with_resource(&evidence, || {
            let surface = surface.ok_or("window semantics are unavailable on this backend")?;
            let surface_resource = surface_resource.as_ref().ok_or("surface unavailable")?;
            let (tree_generation, projection) =
                self.internal_ui.bounded_application_semantics(surface)?;
            let nodes = semantic_projection(projection)?;
            Ok(SemanticSnapshot {
                window: id.to_owned(),
                window_generation: generation,
                surface: surface_resource.id.clone(),
                surface_generation: surface_resource.generation,
                tree_generation,
                observed_at_us: self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
                nodes,
            })
        })
    }

    pub(super) fn remote_shell_surface_diagnostics(
        &self,
    ) -> (
        Vec<nickel_remote_control::diagnostics::ShellSurfaceDiagnostic>,
        bool,
    ) {
        use crate::winit_shell::SurfaceRole;
        use nickel_remote_control::diagnostics::{
            MAX_DIAGNOSTIC_SHELL_SURFACES, ShellDiagnosticRole, ShellSurfaceDiagnostic,
        };
        let Some(shell) = self.internal_shell.as_ref().filter(|_| !self.locked) else {
            return (Vec::new(), false);
        };
        let mut records = shell.surfaces().iter().filter_map(|entry| {
            // Transients which can contain authentication or protected-window content
            // need their own production protection evidence before being projected.
            let role = match entry.role {
                SurfaceRole::Desktop => ShellDiagnosticRole::Desktop,
                SurfaceRole::Panel => ShellDiagnosticRole::Panel,
                SurfaceRole::Launcher => ShellDiagnosticRole::Launcher,
                SurfaceRole::ControlCenter => ShellDiagnosticRole::ControlCenter,
                SurfaceRole::Notification => ShellDiagnosticRole::Notification,
                SurfaceRole::VolumeOsd => ShellDiagnosticRole::VolumeOsd,
                SurfaceRole::WindowPreview => ShellDiagnosticRole::WindowPreview,
                SurfaceRole::WindowContextMenu => ShellDiagnosticRole::WindowContextMenu,
                SurfaceRole::Screenshot => ShellDiagnosticRole::Screenshot,
                SurfaceRole::OnScreenKeyboard => ShellDiagnosticRole::OnScreenKeyboard,
                SurfaceRole::Lock | SurfaceRole::CodexProjectMenu | SurfaceRole::CodexChat => {
                    return None;
                }
            };
            let runtime = *self.internal_shell_surfaces.get(&entry.id)?;
            if shell.remote_access_protected(entry.id)
                || !self.internal_ui.is_visible(runtime)
                || self.internal_ui.remote_access_protected(runtime)
            {
                return None;
            }
            let placement = self.internal_ui.placement(runtime)?;
            let (x, y, width, height) = placement.geometry;
            Some(ShellSurfaceDiagnostic {
                id: format!("internal:{}", runtime.snapshot_token()),
                generation: runtime.snapshot_token(),
                role,
                geometry: [
                    i64::from(x),
                    i64::from(y),
                    i64::from(width),
                    i64::from(height),
                ],
                output: placement
                    .output
                    .as_ref()
                    .map(|name| name.chars().take(128).collect()),
                scene_generation: entry.scene_generation,
                scale_factor: self.internal_ui.scale_factor(runtime)?,
                redraw_pending: self.internal_ui.redraw_pending(runtime)?,
                keyboard_focused: self.internal_ui.focused() == Some(runtime),
            })
        });
        let result = records
            .by_ref()
            .take(MAX_DIAGNOSTIC_SHELL_SURFACES)
            .collect();
        let truncated = records.next().is_some();
        (result, truncated)
    }

    pub(super) fn remote_surface_ancestors(
        &self,
        identity: &nickel_remote_control::leases::ResourceId,
    ) -> Vec<nickel_remote_control::leases::ResourceId> {
        let (surfaces, truncated) = self.remote_shell_surface_diagnostics();
        if truncated {
            return Vec::new();
        }
        crate::remote_surface_authority::SurfaceAuthority::from_shell_surfaces(&surfaces)
            .map(|authority| authority.ancestors(identity))
            .unwrap_or_default()
    }

    pub(super) fn remote_internal_application_diagnostics(
        &self,
        windows: &[nickel_remote_control::WindowSummary],
    ) -> Vec<nickel_remote_control::diagnostics::InternalApplicationDiagnostic> {
        windows
            .iter()
            .filter_map(|window| {
                let id = WindowId(window.generation);
                if self.remote_window_is_protected(id) {
                    return None;
                }
                let surface = self.internal_window_surfaces.get(&id)?;
                self.internal_ui
                    .remote_application_diagnostic(*surface, &window.id)
            })
            .collect()
    }

    pub(super) fn remote_codex_feature_diagnostic(
        &self,
        observation_generation: u64,
        observed_at_us: u64,
    ) -> Option<nickel_remote_control::diagnostics::CodexFeatureDiagnostic> {
        use nickel_core::optional_features::{FeatureHealth, FeatureInstallation, FeatureSupport};
        use nickel_remote_control::diagnostics::{
            CodexFeatureDiagnostic, FeatureHealthDiagnostic, FeatureInstallationDiagnostic,
        };
        let projection = self.internal_shell.as_ref()?.codex_projection()?;
        let installation = match projection.installation {
            FeatureInstallation::Installed => FeatureInstallationDiagnostic::Installed,
            FeatureInstallation::Missing => FeatureInstallationDiagnostic::Missing,
            FeatureInstallation::Incompatible => FeatureInstallationDiagnostic::Incompatible,
        };
        let health = match projection.health {
            FeatureHealth::Unknown => FeatureHealthDiagnostic::Unknown,
            FeatureHealth::Loading => FeatureHealthDiagnostic::Loading,
            FeatureHealth::SignedOut => FeatureHealthDiagnostic::SignedOut,
            FeatureHealth::Ready => FeatureHealthDiagnostic::Ready,
            FeatureHealth::Failed => FeatureHealthDiagnostic::Failed,
        };
        Some(CodexFeatureDiagnostic {
            observation_generation,
            observed_at_us,
            supported: projection.support == FeatureSupport::Supported,
            installation,
            enabled: projection.enabled,
            health,
            configuration_generation: projection.generation,
        })
    }

    pub(super) fn remote_platform_diagnostic(
        &self,
        observation_generation: u64,
        observed_at_us: u64,
    ) -> nickel_remote_control::diagnostics::PlatformDiagnostic {
        let backend = None;
        #[cfg(feature = "backend-winit")]
        let backend = self
            .winit_redraw_window
            .map(|_| nickel_remote_control::diagnostics::CompositorBackend::Winit)
            .or(backend);
        #[cfg(feature = "backend-udev")]
        let backend = self
            .native
            .as_ref()
            .map(|_| nickel_remote_control::diagnostics::CompositorBackend::Udev)
            .or(backend);
        nickel_remote_control::diagnostics::PlatformDiagnostic {
            observation_generation,
            observed_at_us,
            backend,
            keyboard_present: self.seat.get_keyboard().is_some(),
            pointer_present: self.seat.get_pointer().is_some(),
            touch_present: self.seat.get_touch().is_some(),
            xwayland_connected: self.xwm.is_some(),
            xwayland_restart_pending: self.xwayland_restart_pending,
            isolated_x11_keyboard_initialized: self
                .xwm
                .as_ref()
                .is_some_and(|(_, xwm)| xwm.isolated_keyboard_initialized()),
            native_keyboard_worker_initialized: self.remote_native_key_worker.is_some(),
        }
    }

    pub(super) fn remote_input_diagnostic(
        &self,
        windows: &[nickel_remote_control::WindowSummary],
        internal_applications: &[InternalApplicationDiagnostic],
        observation_generation: u64,
        observed_at_us: u64,
    ) -> InputDiagnostic {
        if self.locked || self.shell_recovery_visible() {
            return InputDiagnostic {
                observation_generation,
                observed_at_us,
                keyboard: None,
                pointer: None,
                pointer_hit_test: None,
            };
        }
        let ordinary_shell = |surface: nickel_ui::InternalSurfaceId| {
            let identity = nickel_remote_control::leases::ResourceId {
                id: format!("internal:{}", surface.snapshot_token()),
                generation: surface.snapshot_token(),
            };
            self.surface_capture_evidence(&identity)
                .ok()
                .map(|_| identity)
        };
        let pointer_window = |focus: &PointerFocusTarget| {
            windows.iter().find_map(|summary| {
                if self.remote_window_is_protected(WindowId(summary.generation)) {
                    return None;
                }
                let window = self.window_for_registry_id(WindowId(summary.generation))?;
                let matches = match focus {
                    PointerFocusTarget::X11(surface) => window.x11_surface() == Some(surface),
                    PointerFocusTarget::Wayland(surface) => window
                        .wl_surface()
                        .is_some_and(|native| *native == *surface),
                };
                matches.then(|| summary.id.clone())
            })
        };
        let pointer_hit_test = self.seat.get_pointer().and_then(|pointer| {
            let point = pointer.current_location();
            let frame_hit = self.internal_ui.internal_frame_target((point.x, point.y));
            let internal_hit = frame_hit
                .map(|(surface, part)| (surface, Some(part)))
                .or_else(|| {
                    self.internal_ui
                        .surface_at(
                            (point.x, point.y),
                            self.client_scene_under(point)
                                && !self.internal_applications_are_foremost(),
                        )
                        .map(|(surface, _)| (surface, None))
                });
            if let Some((surface, frame_part)) = internal_hit {
                if self.internal_ui.remote_access_protected(surface) {
                    return None;
                }
                let placement = self.internal_ui.placement(surface)?;
                let hit_x = point.x as f32 - placement.geometry.0 as f32;
                let hit_y = point.y as f32 - placement.geometry.1 as f32;
                let semantic_hit =
                    if let Some(shell) = self.internal_shell.as_ref() {
                        if let Some(entry) = shell.surfaces().iter().find(|entry| {
                            self.internal_shell_surfaces.get(&entry.id) == Some(&surface)
                        }) {
                            shell.bounded_shell_semantics(entry.id).ok()
                        } else {
                            self.internal_ui.bounded_application_semantics(surface).ok()
                        }
                    } else {
                        self.internal_ui.bounded_application_semantics(surface).ok()
                    }
                    .and_then(|(generation, nodes)| {
                        nodes
                            .iter()
                            .enumerate()
                            .filter(|(_, node)| {
                                let left = node.bounds.origin.x;
                                let top = node.bounds.origin.y;
                                hit_x >= left
                                    && hit_y >= top
                                    && hit_x < left + node.bounds.size.width
                                    && hit_y < top + node.bounds.size.height
                            })
                            .min_by(|(_, left), (_, right)| {
                                let left_area = left.bounds.size.width * left.bounds.size.height;
                                let right_area = right.bounds.size.width * right.bounds.size.height;
                                left_area.total_cmp(&right_area)
                            })
                            .map(|(ordinal, _)| (generation, ordinal as u64))
                    });
                if let Some(identity) = ordinary_shell(surface) {
                    return Some(
                        nickel_remote_control::diagnostics::PointerHitTestDiagnostic {
                            window: None,
                            surface: Some(identity),
                            semantic_tree_generation: semantic_hit.map(|hit| hit.0),
                            semantic_node: semantic_hit.map(|hit| hit.1),
                            decoration: None,
                        },
                    );
                }
                let application = internal_applications
                    .iter()
                    .find(|application| application.generation == surface.snapshot_token())?;
                return Some(
                    nickel_remote_control::diagnostics::PointerHitTestDiagnostic {
                        window: Some(application.window.clone()),
                        surface: None,
                        semantic_tree_generation: semantic_hit.map(|hit| hit.0),
                        semantic_node: semantic_hit.map(|hit| hit.1),
                        decoration: frame_part.map(internal_decoration_hit),
                    },
                );
            }
            let window = match self.pointer_surface_under(point) {
                Some((focus, _)) => Some(pointer_window(&focus)?),
                None => None,
            };
            Some(
                nickel_remote_control::diagnostics::PointerHitTestDiagnostic {
                    window,
                    surface: None,
                    semantic_tree_generation: None,
                    semantic_node: None,
                    decoration: None,
                },
            )
        });
        let keyboard = self.seat.get_keyboard().and_then(|keyboard| {
            if let Some(surface) = self.internal_ui.focused() {
                if self.internal_ui.remote_access_protected(surface) {
                    return None;
                }
                if let Some(identity) = ordinary_shell(surface) {
                    return Some(InputDeviceDiagnostic {
                        focused_window: None,
                        focused_surface: Some(identity),
                        compositor_grabbed: keyboard.is_grabbed(),
                        remote_hold_active: self.remote_held_keyboard.is_some(),
                    });
                }
                let application = internal_applications
                    .iter()
                    .find(|application| application.generation == surface.snapshot_token())?;
                return Some(InputDeviceDiagnostic {
                    focused_window: Some(application.window.clone()),
                    focused_surface: None,
                    compositor_grabbed: keyboard.is_grabbed(),
                    remote_hold_active: self.remote_held_keyboard.is_some(),
                });
            }
            let focused_window = if let Some(focus) = keyboard.current_focus() {
                Some(windows.iter().find_map(|summary| {
                    if self.remote_window_is_protected(WindowId(summary.generation)) {
                        return None;
                    }
                    let window = self.window_for_registry_id(WindowId(summary.generation))?;
                    let matches = match &focus {
                        KeyboardFocusTarget::X11(surface) => {
                            surface.has_observed_keyboard_focus()
                                && window.x11_surface() == Some(surface)
                        }
                        KeyboardFocusTarget::Wayland(surface) => window
                            .wl_surface()
                            .is_some_and(|native| *native == *surface),
                    };
                    matches.then(|| summary.id.clone())
                })?)
            } else {
                None
            };
            Some(InputDeviceDiagnostic {
                focused_window,
                focused_surface: None,
                compositor_grabbed: keyboard.is_grabbed(),
                remote_hold_active: self.remote_held_keyboard.is_some(),
            })
        });
        let pointer = self.seat.get_pointer().and_then(|pointer| {
            // A hosted capture can outlive hit testing over its starting surface.
            // Do not misidentify a native recipient underneath that gesture.
            if self.internal_ui.pointer_interaction_active()
                || self
                    .internal_shell
                    .as_ref()
                    .is_some_and(|shell| shell.pointer_interaction_active())
            {
                return None;
            }
            let point = pointer.current_location();
            if self
                .internal_ui
                .surface_at(
                    (point.x, point.y),
                    self.client_scene_under(point) && !self.internal_applications_are_foremost(),
                )
                .is_some()
            {
                return None;
            }
            let focused_window = if let Some(focus) = pointer.current_focus() {
                Some(pointer_window(&focus)?)
            } else {
                None
            };
            Some(InputDeviceDiagnostic {
                focused_window,
                focused_surface: None,
                compositor_grabbed: pointer.is_grabbed(),
                remote_hold_active: self.remote_held_pointer.is_some(),
            })
        });
        InputDiagnostic {
            observation_generation,
            observed_at_us,
            keyboard,
            pointer,
            pointer_hit_test,
        }
    }
}

fn internal_decoration_hit(
    part: crate::session::window_frame::FramePart,
) -> nickel_remote_control::diagnostics::InternalDecorationHit {
    use crate::session::window_frame::FramePart;
    use nickel_remote_control::diagnostics::InternalDecorationHit;
    match part {
        FramePart::Titlebar => InternalDecorationHit::Titlebar,
        FramePart::Minimize => InternalDecorationHit::Minimize,
        FramePart::Maximize => InternalDecorationHit::Maximize,
        FramePart::Close => InternalDecorationHit::Close,
        FramePart::ResizeNorth => InternalDecorationHit::ResizeNorth,
        FramePart::ResizeNorthEast => InternalDecorationHit::ResizeNorthEast,
        FramePart::ResizeEast => InternalDecorationHit::ResizeEast,
        FramePart::ResizeSouthEast => InternalDecorationHit::ResizeSouthEast,
        FramePart::ResizeSouth => InternalDecorationHit::ResizeSouth,
        FramePart::ResizeSouthWest => InternalDecorationHit::ResizeSouthWest,
        FramePart::ResizeWest => InternalDecorationHit::ResizeWest,
        FramePart::ResizeNorthWest => InternalDecorationHit::ResizeNorthWest,
    }
}

fn semantic_projection(
    projection: Vec<nickel_ui::SemanticNodeSnapshot>,
) -> Result<Vec<nickel_remote_control::semantics::SemanticNode>, String> {
    use nickel_remote_control::semantics::*;
    projection
        .into_iter()
        .enumerate()
        .map(|(index, node)| {
            let bounds = [
                node.bounds.origin.x,
                node.bounds.origin.y,
                node.bounds.size.width,
                node.bounds.size.height,
            ];
            if !bounds.iter().all(|value| value.is_finite()) {
                return Err("semantic geometry unavailable".to_owned());
            }
            let value = match node.value {
                Some(nickel_ui::SemanticValueSnapshot::Boolean(value)) => {
                    Some(SemanticValue::Boolean(value))
                }
                Some(nickel_ui::SemanticValueSnapshot::Text(value)) => {
                    Some(SemanticValue::Text(value))
                }
                Some(nickel_ui::SemanticValueSnapshot::Number {
                    value,
                    minimum,
                    maximum,
                    step,
                }) => {
                    if ![value, minimum, maximum, step]
                        .iter()
                        .all(|value| value.is_finite())
                    {
                        return Err("semantic value unavailable".to_owned());
                    }
                    Some(SemanticValue::Number {
                        value,
                        minimum,
                        maximum,
                        step,
                    })
                }
                Some(nickel_ui::SemanticValueSnapshot::ProtectedText { .. }) => {
                    return Err("protected surface".into());
                }
                None => None,
            };
            Ok(SemanticNode {
                id: index as u32,
                role: node.role.map(|role| format!("{role:?}")),
                bounds,
                name: node.name,
                description: node.description,
                enabled: node.enabled,
                focused: node.focused,
                actions: node
                    .actions
                    .into_iter()
                    .map(|action| format!("{action:?}"))
                    .collect(),
                value,
            })
        })
        .collect::<Result<Vec<_>, String>>()
}

fn semantic_mutation_action(
    action: nickel_remote_control::semantics::SemanticMutation,
) -> nickel_ui::SemanticAction {
    use nickel_remote_control::semantics::*;
    match action {
        SemanticMutation::SetBoolean(value) => {
            nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Boolean(value))
        }
        SemanticMutation::SetNumber(value) => {
            nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Number(value))
        }
        SemanticMutation::SetText(value) => {
            nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Text(value))
        }
        SemanticMutation::Invoke(value) => nickel_ui::SemanticAction::Invoke(match value {
            SemanticInvocation::Activate => nickel_ui::ActionKind::Activate,
            SemanticInvocation::Cancel => nickel_ui::ActionKind::Cancel,
            SemanticInvocation::ContextMenu => nickel_ui::ActionKind::ContextMenu,
            SemanticInvocation::Increment => nickel_ui::ActionKind::Increment,
            SemanticInvocation::Decrement => nickel_ui::ActionKind::Decrement,
            SemanticInvocation::Expand => nickel_ui::ActionKind::Expand,
            SemanticInvocation::Collapse => nickel_ui::ActionKind::Collapse,
            SemanticInvocation::Select => nickel_ui::ActionKind::Select,
            SemanticInvocation::Dismiss => nickel_ui::ActionKind::Dismiss,
            SemanticInvocation::Scroll => nickel_ui::ActionKind::Scroll,
            SemanticInvocation::EnterNavigation => nickel_ui::ActionKind::EnterNavigation,
            SemanticInvocation::ExitNavigation => nickel_ui::ActionKind::ExitNavigation,
        }),
    }
}

/// Project the actual compositor registration table, not a default shortcut list.
/// This seam accepts no keyboard device, event stream, or emergency-control state.
pub(super) fn shortcut_diagnostic(
    adapter: Option<&nickel_core::hotkeys::CompositorShortcutAdapter>,
    observation_generation: u64,
    observed_at_us: u64,
) -> nickel_remote_control::diagnostics::ShortcutDiagnostic {
    use nickel_input::{PhysicalKey, ShortcutKey};
    use nickel_remote_control::diagnostics::{
        MAX_DIAGNOSTIC_SHORTCUTS, ShortcutDiagnostic, ShortcutDiagnosticCapability,
        ShortcutRegistrationDiagnostic,
    };
    let mut snapshot = ShortcutDiagnostic {
        observation_generation,
        observed_at_us,
        registration_revision: adapter.and_then(|adapter| adapter.registration_revision()),
        capability: ShortcutDiagnosticCapability::BackendUnavailable,
        registrations: Vec::new(),
        unprojected_bindings: 0,
        truncated: false,
    };
    let Some(adapter) = adapter else {
        return snapshot;
    };
    snapshot.capability = if snapshot.registration_revision.is_some() {
        ShortcutDiagnosticCapability::Available
    } else {
        ShortcutDiagnosticCapability::RevisionExhausted
    };
    for (index, (id, registration)) in adapter.registrations().enumerate() {
        if index == MAX_DIAGNOSTIC_SHORTCUTS {
            snapshot.truncated = true;
            break;
        }
        let ShortcutKey::Physical(PhysicalKey::Code(key)) = &registration.shortcut.key else {
            snapshot.unprojected_bindings += 1;
            continue;
        };
        snapshot.registrations.push(ShortcutRegistrationDiagnostic {
            registration_id: id.0,
            physical_key: format!("{key:?}"),
            action: format!("{:?}", registration.action),
            modifiers: registration
                .shortcut
                .modifiers
                .iter()
                .map(|modifier| format!("{modifier:?}"))
                .collect(),
            trigger: format!("{:?}", registration.shortcut.trigger),
        });
    }
    snapshot
}

#[cfg(test)]
mod shortcut_tests {
    use super::shortcut_diagnostic;
    use nickel_core::hotkeys::{CompositorShortcutAdapter, HotkeyAction};
    use nickel_input::global::{GlobalShortcutAdapter, Registration};
    use nickel_input::{
        AggregateModifier, KeyCode, KeyEdge, LogicalKey, PhysicalKey, Shortcut, ShortcutKey,
        ShortcutTrigger,
    };
    use nickel_remote_control::diagnostics::{
        MAX_DIAGNOSTIC_SHORTCUTS, ShortcutDiagnosticCapability,
    };

    #[test]
    fn shortcut_projection_tracks_live_registrations_without_input_payloads() {
        let mut adapter = CompositorShortcutAdapter::default();
        let before = shortcut_diagnostic(Some(&adapter), 7, 11);
        assert_eq!(before.observation_generation, 7);
        assert_eq!(before.observed_at_us, 11);
        assert_eq!(before.capability, ShortcutDiagnosticCapability::Available);
        assert_eq!(before.registrations.len(), adapter.registrations().count());
        adapter.handle(KeyCode::KeyA, KeyEdge::Pressed);
        adapter.handle(KeyCode::ControlLeft, KeyEdge::Pressed);
        assert_eq!(
            shortcut_diagnostic(Some(&adapter), 7, 11).registrations,
            before.registrations
        );
        let id = adapter
            .register(Registration {
                shortcut: Shortcut {
                    key: ShortcutKey::Logical(LogicalKey::Character("PRIVATE_PAYLOAD".into())),
                    modifiers: Default::default(),
                    trigger: ShortcutTrigger::Pressed,
                },
                action: HotkeyAction::ToggleLauncher,
            })
            .unwrap();
        let projected = shortcut_diagnostic(Some(&adapter), 8, 12);
        assert_eq!(projected.unprojected_bindings, 1);
        assert_eq!(projected.registrations, before.registrations);
        assert!(
            !serde_json::to_string(&projected)
                .unwrap()
                .contains("PRIVATE_PAYLOAD")
        );
        assert!(adapter.unregister(id));
        let physical = before.registrations[0].registration_id;
        assert!(adapter.unregister(nickel_input::global::RegistrationId(physical)));
        let after = shortcut_diagnostic(Some(&adapter), 9, 13);
        assert!(
            !after
                .registrations
                .iter()
                .any(|entry| entry.registration_id == physical)
        );
        assert_eq!(after.unprojected_bindings, 0);
        assert!(after.registration_revision > before.registration_revision);
    }

    #[test]
    fn shortcut_projection_is_bounded_and_missing_backend_is_explicit() {
        let unavailable = shortcut_diagnostic(None, 1, 2);
        assert_eq!(
            unavailable.capability,
            ShortcutDiagnosticCapability::BackendUnavailable
        );
        assert!(unavailable.registrations.is_empty());
        assert!(unavailable.registration_revision.is_none());
        let mut adapter = CompositorShortcutAdapter::default();
        for key in [
            KeyCode::KeyA,
            KeyCode::KeyB,
            KeyCode::KeyC,
            KeyCode::KeyD,
            KeyCode::KeyE,
            KeyCode::KeyF,
            KeyCode::KeyG,
            KeyCode::KeyH,
            KeyCode::KeyI,
        ] {
            for mask in 0..16 {
                let modifiers = [
                    AggregateModifier::Control,
                    AggregateModifier::Alt,
                    AggregateModifier::Shift,
                    AggregateModifier::Super,
                ]
                .into_iter()
                .enumerate()
                .filter_map(|(bit, modifier)| (mask & (1 << bit) != 0).then_some(modifier))
                .collect();
                let _ = adapter.register(Registration {
                    shortcut: Shortcut {
                        key: ShortcutKey::Physical(PhysicalKey::Code(key)),
                        modifiers,
                        trigger: ShortcutTrigger::Pressed,
                    },
                    action: HotkeyAction::ToggleLauncher,
                });
            }
        }
        let snapshot = shortcut_diagnostic(Some(&adapter), 1, 2);
        assert_eq!(snapshot.registrations.len(), MAX_DIAGNOSTIC_SHORTCUTS);
        assert!(snapshot.truncated);
    }
}
