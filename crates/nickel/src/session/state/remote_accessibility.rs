use super::*;
use crate::session::remote_accessibility::{Proof, association};
use nickel_remote_control::{DesktopPermit, native_semantics::NativeSemanticSnapshot};

impl NickelSession {
    pub(super) fn native_accessibility_proof(
        &self,
        permit: &DesktopPermit,
        id: &str,
        generation: u64,
        application_root: bool,
    ) -> Result<Proof, String> {
        let id = WindowId(id.parse().map_err(|_| "invalid native window identity")?);
        if id.0 != generation {
            return Err("native window generation has retired".into());
        }
        let scope = if application_root {
            Some(permit.resource_scope()?)
        } else {
            None
        };
        #[cfg(not(any(feature = "backend-winit", feature = "backend-udev")))]
        return Err("native accessibility is unavailable on this backend".into());
        #[cfg(any(feature = "backend-winit", feature = "backend-udev"))]
        self.with_capture_authority(id, permit, || {
            let window = self
                .registry_native_window(id)
                .ok_or("native window has retired")?;
            if window.x11_surface().is_some() {
                return Err("XWayland accessibility association is unavailable".into());
            }
            if self.space.element_location(&window).is_none() {
                return Err("native window is hidden".into());
            }
            let surface = window
                .wl_surface()
                .ok_or("native Wayland surface is unavailable")?;
            let binding = if application_root {
                use nickel_remote_control::leases::ResourceScope;
                match scope.as_ref().expect("application scope was resolved") {
                    ResourceScope::FullSession => {}
                    ResourceScope::Application(application)
                        if self.remote_verified_application(id).as_deref()
                            == Some(application.as_str()) => {}
                    _ => return Err(
                        "application-root observation requires application or full-session scope"
                            .into(),
                    ),
                }
                None
            } else {
                Some(
                    association(&surface)
                        .ok_or("exact native accessibility association is unavailable")?,
                )
            };
            let process = self
                .remote_window_identities
                .get(&id)
                .and_then(super::super::remote_identity::WindowIdentity::observation_process)
                .ok_or("native process identity is unavailable")?;
            let credentials = surface
                .client()
                .ok_or("native client has retired")?
                .get_credentials(&self.display_handle)
                .map_err(|_| "native peer credentials unavailable")?;
            if u32::try_from(credentials.pid).ok() != Some(process.pid()) {
                return Err("native process identity changed".into());
            }
            let geometry = self
                .remote_window_geometry(id)
                .ok_or("native geometry unavailable")?;
            Ok(Proof {
                id: id.0,
                binding,
                process,
                uid: credentials.uid,
                geometry: [geometry.x, geometry.y, geometry.width, geometry.height],
            })
        })
    }

    pub(super) fn validate_native_accessibility(
        &self,
        permit: &DesktopPermit,
        proof: &Proof,
    ) -> Result<(), String> {
        proof.check_live(permit)?;
        let current = self.native_accessibility_proof(
            permit,
            &proof.id.to_string(),
            proof.id,
            proof.binding.is_none(),
        )?;
        if current.binding.as_ref().map(|b| b.generation)
            != proof.binding.as_ref().map(|b| b.generation)
            || current.geometry != proof.geometry
            || current.process.pid() != proof.process.pid()
            || current.uid != proof.uid
        {
            return Err("native accessibility association changed".into());
        }
        Ok(())
    }

    pub(super) fn finish_native_accessibility(
        &mut self,
        permit: &DesktopPermit,
        proof: &Proof,
        observation: crate::session::remote_accessibility::Observation,
    ) -> Result<NativeSemanticSnapshot, String> {
        self.validate_native_accessibility(permit, proof)?;
        self.remote_observation_generation = self.remote_observation_generation.saturating_add(1);
        let [started, completed, validated] =
            crate::session::remote_accessibility::observation_times(
                self.start_time,
                observation.started,
                observation.completed,
                Instant::now(),
            );
        let mut result = observation.snapshot;
        result.observation_generation = self.remote_observation_generation;
        result.observation_started_at_us = started;
        result.observed_at_us = completed;
        result.owner_validated_at_us = validated;
        #[cfg(any(feature = "backend-winit", feature = "backend-udev"))]
        {
            let result = self.with_capture_authority(WindowId(proof.id), permit, || Ok(result))?;
            if let Some(operation_id) = permit.operation_id() {
                self.remote_external_accessibility = Some(
                    nickel_remote_control::diagnostics::ExternalAccessibilityDiagnostic {
                        operation_id,
                        scope: result.scope.clone(),
                        observation_started_at_us: result.observation_started_at_us,
                        observed_at_us: result.observed_at_us,
                        owner_validated_at_us: result.owner_validated_at_us,
                        nodes: result.nodes.len().min(u32::MAX as usize) as u32,
                        truncated: result.truncated,
                        stale: false,
                    },
                );
            }
            Ok(result)
        }
        #[cfg(not(any(feature = "backend-winit", feature = "backend-udev")))]
        Err("native accessibility unavailable".into())
    }
}

pub(super) struct RemoteGtkMenu {
    permit: DesktopPermit,
    window: WindowId,
    generation: u64,
}

impl NickelSession {
    pub(in crate::session) fn dispatch_gtk_titlebar_gesture(
        &mut self,
        id: WindowId,
        button: u32,
        origin: crate::session::remote_accessibility::PressOrigin,
    ) {
        use crate::session::remote_accessibility::PressOrigin;
        match origin {
            PressOrigin::Local => {
                self.take_over_remote_gtk_menu();
                self.apply_gtk_titlebar_gesture(id, button);
            }
            PressOrigin::Denied => {}
            PressOrigin::Remote(permit, original, epoch)
                if original == id && epoch == self.remote_gtk_epoch && epoch < u64::MAX =>
            {
                #[cfg(any(feature = "backend-winit", feature = "backend-udev"))]
                {
                    if self
                        .registry_native_window(id)
                        .is_none_or(|window| self.space.element_location(&window).is_none())
                    {
                        return;
                    }
                    if self.remote_held_keyboard.is_some() || self.remote_held_pointer.is_some() {
                        return;
                    }
                    if self.with_capture_authority(id, &permit, || Ok(())).is_err() {
                        return;
                    }
                    let identity = nickel_remote_control::leases::ResourceId {
                        id: id.0.to_string(),
                        generation: id.0,
                    };
                    let output = self.remote_window_output(id);
                    let application = self.remote_verified_application(id);
                    let evidence = nickel_remote_control::leases::ResourceEvidence {
                        window: Some(&identity),
                        surface: None,
                        output: output.as_ref(),
                        verified_application: application.as_deref(),
                        authorized_surface_ancestors: &[],
                        protected: self.locked || self.remote_window_is_protected(id),
                    };
                    let previous_menu = self
                        .internal_shell
                        .as_ref()
                        .and_then(|shell| shell.window_menu_generation());
                    let delivered = permit.with_input(&evidence, || {
                        self.apply_gtk_titlebar_gesture(id, button);
                        Ok(())
                    });
                    if delivered.is_ok()
                        && button == 273
                        && let Some(generation) = self
                            .internal_shell
                            .as_ref()
                            .and_then(|shell| shell.window_menu_generation())
                        && Some(generation) != previous_menu
                    {
                        self.remote_gtk_menu = Some(RemoteGtkMenu {
                            permit,
                            window: id,
                            generation,
                        });
                        self.schedule_remote_gtk_menu_check();
                    }
                }
            }
            PressOrigin::Remote(_, _, _) => {}
        }
    }

    fn apply_gtk_titlebar_gesture(&mut self, id: WindowId, button: u32) {
        let Some(window) = self.registry_native_window(id) else {
            return;
        };
        match button {
            272 => self.maximize_window(id),
            273 => {
                self.activate_window(id);
                let point = self
                    .seat
                    .get_pointer()
                    .map(|pointer| pointer.current_location());
                if let Some(point) = point
                    && let Some(shell) = self.internal_shell.as_mut()
                {
                    shell.open_window_menu_at(id.0, point.x.round() as i32, point.y.round() as i32);
                    self.sync_internal_shell();
                    self.schedule_internal_ui_frame();
                    self.wake_internal_shell();
                }
            }
            274 => {
                self.space.lower_element(&window);
                let next = self
                    .space
                    .elements()
                    .rev()
                    .find(|candidate| !self.is_shell_owned_window(candidate))
                    .and_then(|candidate| candidate.wl_surface())
                    .and_then(|surface| self.surface_windows.get(&surface.id()).copied());
                if let Some(next) = next {
                    self.activate_window(next);
                }
                self.request_output_redraw();
            }
            _ => {}
        }
    }

    /// Physical takeover transfers ownership before its legitimate menu input.
    pub(crate) fn take_over_remote_gtk_menu(&mut self) {
        self.remote_gtk_epoch = self.remote_gtk_epoch.saturating_add(1);
        self.remote_gtk_menu = None;
    }

    fn schedule_remote_gtk_menu_check(&mut self) {
        if self.remote_gtk_menu_timer_armed {
            return;
        }
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        self.event_loop_handle
            .insert_source(
                Timer::from_duration(Duration::from_millis(50)),
                |_, _, state| {
                    state.revalidate_remote_gtk_menu();
                    if state.remote_gtk_menu.is_some() {
                        TimeoutAction::ToDuration(Duration::from_millis(50))
                    } else {
                        state.remote_gtk_menu_timer_armed = false;
                        TimeoutAction::Drop
                    }
                },
            )
            .expect("failed to register remote GTK menu lifetime timer");
        self.remote_gtk_menu_timer_armed = true;
    }

    fn revalidate_remote_gtk_menu(&mut self) {
        let Some(origin) = self.remote_gtk_menu.take() else {
            return;
        };
        if self
            .internal_shell
            .as_ref()
            .and_then(|shell| shell.window_menu_generation())
            != Some(origin.generation)
        {
            return; // A replacement or local-owned menu is not this continuation.
        }
        #[cfg(any(feature = "backend-winit", feature = "backend-udev"))]
        if !self.locked
            && self
                .registry_native_window(origin.window)
                .is_some_and(|window| self.space.element_location(&window).is_some())
            && self
                .with_capture_authority(origin.window, &origin.permit, || Ok(()))
                .is_ok()
        {
            self.remote_gtk_menu = Some(origin);
            return;
        }
        if let Some(shell) = self.internal_shell.as_mut() {
            shell.retire_window_menu(origin.generation);
        }
        self.sync_internal_shell();
        self.schedule_internal_ui_frame();
        self.wake_internal_shell();
    }
}
