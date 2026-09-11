//! Preparation owns its admission until dispatch or cancellation, including HTTP timeout.
use super::NickelSession;
use super::remote_worker::{StagingAdmission, WorkerStaging};
use nickel_remote_control::{
    DesktopPermit,
    diagnostics::{LaunchApplicationOutcome, LaunchApplicationRequest},
    leases::{ResourceEvidence, ResourceId, ResourceScope},
};
use std::sync::OnceLock;

const MAX_LAUNCHED_CHILDREN: usize = 128;

fn preparation() -> &'static WorkerStaging {
    static PREPARATION: OnceLock<WorkerStaging> = OnceLock::new();
    PREPARATION.get_or_init(WorkerStaging::default)
}

struct Admission {
    _guard: StagingAdmission<'static>,
}
impl Admission {
    fn acquire() -> Result<Self, String> {
        Ok(Self {
            _guard: preparation().acquire()?,
        })
    }
}

pub(super) struct PreparedLaunch {
    application: crate::model::Application,
    generation: u64,
    command: std::process::Command,
    _admission: Admission,
    executable: Option<crate::session::remote_identity::LaunchExecutable>,
    output: Option<ResourceId>,
}

fn session_evidence(protected: bool, application: Option<&str>) -> ResourceEvidence<'_> {
    ResourceEvidence {
        window: None,
        surface: None,
        verified_application: application,
        output: None,
        authorized_surface_ancestors: &[],
        protected,
    }
}

impl PreparedLaunch {
    pub(super) fn prepare_selected(
        permit: &DesktopPermit,
        lease_id: u64,
        selected: &crate::model::Application,
        catalog_generation: u64,
        output: &ResourceId,
    ) -> Result<Self, String> {
        let mut prepared = Self::prepare(
            permit,
            &LaunchApplicationRequest {
                lease_id,
                application_id: selected.id().to_owned(),
                catalog_generation,
            },
        )?;
        if &prepared.application != selected {
            return Err("selected shell application changed; inspect again".into());
        }
        if prepared
            .output
            .as_ref()
            .is_some_and(|scoped| scoped != output)
        {
            return Err("selected shell output is outside the launch lease".into());
        }
        prepared.output = Some(output.clone());
        Ok(prepared)
    }
    pub fn prepare(
        permit: &DesktopPermit,
        request: &LaunchApplicationRequest,
    ) -> Result<Self, String> {
        let scope = permit.resource_scope()?;
        let output = match &scope {
            ResourceScope::Output(output) => Some(output.clone()),
            _ => None,
        };
        let expected_application = match scope {
            nickel_remote_control::leases::ResourceScope::FullSession => None,
            nickel_remote_control::leases::ResourceScope::Application(application) => {
                Some(application)
            }
            ResourceScope::Output(_) => None,
            _ => return Err("launch requires an application, output or full-session lease".into()),
        };
        if request.application_id.is_empty()
            || request.application_id.len() > 512
            || request.catalog_generation == 0
        {
            return Err("invalid application launch target".into());
        }
        let admission = Admission::acquire()?;
        let (generation, catalog, _) = crate::platform::installed_application_signatures();
        if generation != request.catalog_generation {
            return Err("application catalog changed; enumerate it again".into());
        }
        let application = catalog
            .iter()
            .find(|application| application.id() == request.application_id)
            .cloned()
            .ok_or_else(|| "installed application is unavailable".to_owned())?;
        // Ordinary production preparation strips trusted session capabilities
        // and applies the same terminal, cwd and scaling policies as local launch.
        let mut command = application
            .process()
            .map_err(|_| "application launch preparation failed".to_owned())?;
        if crate::session::remote_identity::protected_launch_target(&command) {
            return Err("application launch target is protected".into());
        }
        let executable = if let Some(expected) = &expected_application {
            let (executable, pinned) =
                crate::session::remote_identity::LaunchExecutable::prepare(&command)
                    .ok_or_else(|| "application launch identity is unavailable".to_owned())?;
            if executable.application.as_ref() != Some(expected) {
                return Err("installed launch target is outside the application lease".into());
            }
            command = pinned;
            Some(executable)
        } else if let Some((executable, pinned)) =
            crate::session::remote_identity::LaunchExecutable::prepare_full_session(&command)?
        {
            command = pinned;
            Some(executable)
        } else {
            return Err("application executable could not be pinned".into());
        };
        let mut evidence = session_evidence(
            false,
            executable
                .as_ref()
                .and_then(|value| value.application.as_deref()),
        );
        evidence.output = output.as_ref();
        permit.with_resource(&evidence, || Ok(()))?;
        Ok(Self {
            application,
            generation,
            command,
            _admission: admission,
            executable,
            output,
        })
    }
}

pub(super) struct PreparedCatalog {
    expected: Option<String>,
    output: Option<ResourceId>,
    generation: u64,
    observed_at: std::time::Instant,
    applications: Vec<nickel_remote_control::diagnostics::InstalledApplication>,
    truncated: bool,
    _admission: Admission,
}

impl PreparedCatalog {
    pub fn prepare(permit: &DesktopPermit) -> Result<Self, String> {
        use nickel_remote_control::{
            diagnostics::{InstalledApplication, MAX_INSTALLED_APPLICATIONS},
            leases::ResourceScope,
        };
        let scope = permit.resource_scope()?;
        let output = match &scope {
            ResourceScope::Output(output) => Some(output.clone()),
            _ => None,
        };
        let expected = match scope {
            ResourceScope::FullSession | ResourceScope::Output(_) => None,
            ResourceScope::Application(application) => Some(application),
            _ => return Err(
                "installed application enumeration requires application or full-session authority"
                    .into(),
            ),
        };
        let admission = Admission::acquire()?;
        let (generation, catalog, mut truncated) =
            crate::platform::installed_application_signatures();
        // One settings observation for the whole inventory. Loading it for each
        // entry repeats filesystem work and can mix policies within a snapshot.
        let scale_settings =
            nickel_core::dpi::ApplicationScaleSettings::load_default().unwrap_or_default();
        let mut applications = Vec::new();
        for application in catalog.iter() {
            permit.check_live()?;
            if applications.len() == MAX_INSTALLED_APPLICATIONS {
                truncated = true;
                break;
            }
            if application.id().len() > 512 || application.name().len() > 512 {
                truncated = true;
                continue;
            }
            let verified_application = application
                .process_with_scale_settings(&scale_settings)
                .ok()
                .and_then(|command| {
                    crate::session::remote_identity::LaunchExecutable::prepare(&command)
                        .and_then(|(proof, _)| proof.application)
                });
            if expected
                .as_ref()
                .is_some_and(|expected| verified_application.as_ref() != Some(expected))
            {
                continue;
            }
            applications.push(InstalledApplication {
                id: application.id().to_owned(),
                name: application.name().to_owned(),
                verified_application,
            });
        }
        permit.check_live()?;
        Ok(Self {
            expected,
            output,
            generation,
            observed_at: std::time::Instant::now(),
            applications,
            truncated,
            _admission: admission,
        })
    }
}

impl NickelSession {
    pub(super) fn remote_application_launch_diagnostic(
        &self,
    ) -> nickel_remote_control::diagnostics::ApplicationLaunchDiagnostic {
        nickel_remote_control::diagnostics::ApplicationLaunchDiagnostic {
            preparation: preparation().snapshot(),
            tracked_children: self.remote_launched_children.len(),
            child_capacity: MAX_LAUNCHED_CHILDREN,
        }
    }

    pub(super) fn remote_scoped_application_inventory(
        &mut self,
        permit: &DesktopPermit,
        prepared: PreparedCatalog,
    ) -> Result<nickel_remote_control::diagnostics::ApplicationInventory, String> {
        let protected = self.locked || self.shell_recovery_visible();
        self.validate_launch_output(prepared.output.as_ref())?;
        let mut evidence = session_evidence(protected, prepared.expected.as_deref());
        evidence.output = prepared.output.as_ref();
        permit.with_resource(&evidence, || {
            let (generation, _, _) = crate::platform::installed_application_signatures();
            if generation != prepared.generation {
                return Err("application catalog changed; enumerate it again".into());
            }
            self.remote_observation_generation =
                self.remote_observation_generation.saturating_add(1);
            Ok(nickel_remote_control::diagnostics::ApplicationInventory {
                observation_generation: self.remote_observation_generation,
                observed_at_us: self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
                catalog_observed_at_us: prepared
                    .observed_at
                    .saturating_duration_since(self.start_time)
                    .as_micros()
                    .min(u64::MAX as u128) as u64,
                catalog_generation: generation,
                available: generation != 0,
                applications: prepared.applications,
                truncated: prepared.truncated,
            })
        })
    }

    pub(super) fn reap_remote_launched_children(&mut self) {
        // Bounded nonblocking waitpid checks. These children belong to the
        // session, not to a lease: revocation must not close local applications.
        self.remote_launched_children
            .retain_mut(|child| match child.try_wait() {
                Ok(Some(_)) => false,
                Err(error) if error.raw_os_error() == Some(nix::libc::ECHILD) => false,
                Ok(None) | Err(_) => true,
            });
    }

    pub(super) fn remote_launch_installed_application(
        &mut self,
        permit: &DesktopPermit,
        mut prepared: PreparedLaunch,
    ) -> Result<LaunchApplicationOutcome, String> {
        let protected = self.locked || self.shell_recovery_visible();
        let application_identity = prepared
            .executable
            .as_ref()
            .and_then(|value| value.application.clone());
        self.validate_launch_output(prepared.output.as_ref())?;
        let mut evidence = session_evidence(protected, application_identity.as_deref());
        evidence.output = prepared.output.as_ref();
        permit.with_resource(&evidence, || Ok(()))?;
        self.reap_remote_launched_children();
        let controller_busy = self.poll_remote_controller_ownership();
        permit.with_input(&evidence, || {
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
            let (generation, catalog, _) = crate::platform::installed_application_signatures();
            if generation != prepared.generation
                || !catalog
                    .iter()
                    .any(|application| application == &prepared.application)
            {
                return Err("application catalog changed; enumerate it again".into());
            }
            if crate::platform::application_requires_secure_storage(&prepared.application)
                && self.secure_storage_state()
                    != crate::session::login_services::SecureStorageState::Ready
            {
                return Err("application launch requires local secure-storage readiness".into());
            }
            // The compositor's host DISPLAY may name a different desktop when
            // nested. Applications belong to this session's own display servers.
            prepared.command.env("WAYLAND_DISPLAY", &self.socket_name);
            if let Some(display) = self.xwayland_display {
                prepared.command.env("DISPLAY", format!(":{display}"));
            } else {
                prepared.command.env_remove("DISPLAY");
            }
            if self.remote_launched_children.len() >= MAX_LAUNCHED_CHILDREN
                || self.remote_launch_placements.len() >= MAX_LAUNCHED_CHILDREN
            {
                return Err("application launch process capacity reached".into());
            }
            let child = prepared
                .command
                .spawn()
                .map_err(|_| "installed application could not be started".to_owned())?;
            let process_id = child.id();
            self.remote_launched_children.push(child);
            if let Some(output) = prepared.output.clone() {
                let root_start_time =
                    super::linux_process_start_time(process_id).ok_or_else(|| {
                        "launched process exited before placement could be associated".to_owned()
                    })?;
                self.remote_launch_placements.push(LaunchPlacement {
                    permit: permit.clone(),
                    output,
                    root_pid: process_id,
                    root_start_time,
                    deadline: std::time::Instant::now() + std::time::Duration::from_secs(30),
                });
            }
            Ok(LaunchApplicationOutcome {
                catalog_generation: generation,
                application_id: prepared.application.id().to_owned(),
                requested: true,
                process_spawn_confirmed: true,
                process_id: Some(process_id),
                output_requested: prepared.output.clone(),
                // Placement is retained until a verified descendant maps and
                // the production first-map effect runs under live authority.
                output_confirmed: false,
            })
        })
    }
}

/// Placement provenance is temporary; neither it nor its lease owns the child.
/// Every mapped window still undergoes ordinary resource authorization.
pub(super) struct LaunchPlacement {
    permit: DesktopPermit,
    output: ResourceId,
    root_pid: u32,
    root_start_time: u64,
    deadline: std::time::Instant,
}

pub(crate) enum DeferredLaunchMap {
    Wayland(smithay::reexports::wayland_server::protocol::wl_surface::WlSurface),
    X11(Box<smithay::xwayland::X11Surface>),
}

impl NickelSession {
    pub(super) fn validate_launch_output(&self, output: Option<&ResourceId>) -> Result<(), String> {
        if output.is_some_and(|output| {
            self.remote_output_generations
                .get(&output.id)
                .is_none_or(|(native, generation)| {
                    *generation != output.generation
                        || !self.space.outputs().any(|current| current == native)
                })
        }) {
            return Err("launch output has retired".into());
        }
        Ok(())
    }

    pub(crate) fn remote_launch_placement_pending(&self) -> bool {
        !self.remote_launch_placements.is_empty()
    }

    pub(crate) fn defer_remote_launch_map(
        &mut self,
        id: super::WindowId,
        map: DeferredLaunchMap,
    ) -> bool {
        if self.remote_launch_placement_pending()
            && matches!(
                self.remote_window_identities.get(&id),
                Some(crate::session::remote_identity::WindowIdentity::Pending)
            )
        {
            self.remote_launch_maps
                .entry(id)
                .or_insert((std::time::Instant::now(), map));
            return true;
        }
        false
    }

    /// Hold current authority through the actual first-map placement effect.
    pub(crate) fn with_remote_launch_placement<T>(
        &mut self,
        id: super::WindowId,
        effect: impl FnOnce(&mut Self, crate::session::shell_layout::Geometry) -> Option<T>,
    ) -> Option<T> {
        let pid = self
            .remote_window_identities
            .get(&id)?
            .current_process_id()?;
        if self.remote_window_is_protected(id) {
            return None;
        }
        let (permit, output) =
            self.remote_launch_placements
                .iter()
                .rev()
                .find_map(|placement| {
                    if std::time::Instant::now() >= placement.deadline
                        || self
                            .validate_launch_output(Some(&placement.output))
                            .is_err()
                        || !super::process_descends_from(
                            pid,
                            placement.root_pid,
                            placement.root_start_time,
                        )
                    {
                        return None;
                    }
                    Some((
                        placement.permit.continued_observation().ok()?,
                        placement.output.clone(),
                    ))
                })?;
        let work_area = self
            .placement_outputs()
            .into_iter()
            .find(|candidate| candidate.name == output.id)?
            .work_area;
        let mut evidence = session_evidence(self.locked || self.shell_recovery_visible(), None);
        evidence.output = Some(&output);
        permit
            .with_resource(&evidence, || {
                if self.remote_window_is_protected(id) {
                    return Ok(None);
                }
                Ok(effect(self, work_area))
            })
            .ok()
            .flatten()
    }

    pub(super) fn resume_remote_launch_maps(&mut self) {
        let now = std::time::Instant::now();
        let protected = self.locked || self.shell_recovery_visible();
        let outputs = &self.remote_output_generations;
        self.remote_launch_placements.retain(|placement| {
            now < placement.deadline
                && outputs
                    .get(&placement.output.id)
                    .is_some_and(|(_, generation)| *generation == placement.output.generation)
                && super::linux_process_start_time(placement.root_pid)
                    == Some(placement.root_start_time)
                && placement
                    .permit
                    .continued_observation()
                    .is_ok_and(|permit| {
                        let mut evidence = session_evidence(protected, None);
                        evidence.output = Some(&placement.output);
                        permit.with_resource(&evidence, || Ok(())).is_ok()
                    })
        });
        let ready = self
            .remote_launch_maps
            .iter()
            .filter_map(|(id, (started, _))| {
                (self.remote_launch_placements.is_empty()
                    || now.duration_since(*started) >= std::time::Duration::from_secs(2)
                    || !matches!(
                        self.remote_window_identities.get(id),
                        Some(crate::session::remote_identity::WindowIdentity::Pending)
                    ))
                .then_some(*id)
            })
            .collect::<Vec<_>>();
        for id in ready {
            let Some((_, map)) = self.remote_launch_maps.remove(&id) else {
                continue;
            };
            // A timed-out identity cannot keep a local application invisible.
            if matches!(
                self.remote_window_identities.get(&id),
                Some(crate::session::remote_identity::WindowIdentity::Pending)
            ) {
                self.remote_window_identities.insert(
                    id,
                    crate::session::remote_identity::WindowIdentity::Unavailable,
                );
            }
            if self.windows.title(id).is_none() {
                continue;
            }
            match map {
                DeferredLaunchMap::Wayland(surface) => {
                    if smithay::backend::renderer::utils::with_renderer_surface_state(
                        &surface,
                        |state| state.buffer().is_some(),
                    ) == Some(true)
                    {
                        self.map_xdg_toplevel(&surface);
                    }
                }
                DeferredLaunchMap::X11(surface) => self.map_x11_window(*surface, true),
            }
        }
    }
}
