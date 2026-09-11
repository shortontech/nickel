//! Owned shell continuations stay inside the original blocking desktop request.
use super::*;
use nickel_remote_control::semantics::{
    SurfaceSemanticActionOutcome as ActionOutcome, SurfaceSemanticCompletion as Completion,
};
use nickel_remote_control::{DesktopPermit, leases::ResourceId};

pub(super) enum ShellActionStep {
    Command(crate::platform::ShellCommand),
    Device {
        action: crate::control_view::ControlAction,
        registration: u64,
        ticket: crate::platform::GuardedControlOrigin,
    },
    Launch {
        application: crate::model::Application,
        catalog_generation: u64,
    },
}

pub(super) struct ShellActionPlan {
    pub origin: ResourceId,
    pub output: ResourceId,
    pub changed: bool,
    pub tree_generation: u64,
    pub steps: Vec<ShellActionStep>,
}

pub(super) enum PreparedShellStep {
    Command(crate::platform::ShellCommand),
    Launch(Box<remote_launch::PreparedLaunch>),
    DeviceResult {
        registration: u64,
        outcome: crate::platform::GuardedControlOutcome,
    },
}

pub(super) struct PendingDeviceOrigin {
    origin: ResourceId,
    output: ResourceId,
    expires: Instant,
    owner: crate::platform::GuardedControlOriginOwner,
    standing: bool,
}

impl RemoteDesktopBridge {
    pub(super) fn finish_shell_action(
        &self,
        permit: DesktopPermit,
        lease_id: u64,
        plan: ShellActionPlan,
    ) -> Result<ActionOutcome, String> {
        let mut changed = plan.changed;
        let mut committed = false;
        let mut completion = Completion::UiUpdated;
        if plan.steps.len() > 8 {
            return Err("shell effect budget exceeded; do not retry".into());
        }
        for step in plan.steps {
            let prepared = match step {
                ShellActionStep::Command(command) => PreparedShellStep::Command(command),
                ShellActionStep::Device {
                    action,
                    registration,
                    ticket,
                } => {
                    // The existing blocking desktop adapter waits; the compositor never does.
                    let outcome = match crate::platform::submit_guarded_control(
                        action,
                        permit.clone(),
                        ticket,
                    ) {
                        Ok(response) => response
                            .recv_timeout(Duration::from_secs(2))
                            .unwrap_or(crate::platform::GuardedControlOutcome::Uncertain),
                        Err(_) => crate::platform::GuardedControlOutcome::Unavailable,
                    };
                    PreparedShellStep::DeviceResult {
                        registration,
                        outcome,
                    }
                }
                ShellActionStep::Launch {
                    application,
                    catalog_generation,
                } => {
                    match remote_launch::PreparedLaunch::prepare_selected(
                        &permit,
                        lease_id,
                        &application,
                        catalog_generation,
                        &plan.output,
                    ) {
                        Ok(prepared) => PreparedShellStep::Launch(Box::new(prepared)),
                        Err(error) if !committed => return Err(error),
                        Err(_) => {
                            return Ok(incomplete(changed, Completion::Unavailable, committed));
                        }
                    }
                }
            };
            let (reply, response) = std::sync::mpsc::sync_channel(1);
            if self
                .sender
                .try_send(RemoteDesktopRequest::ShellSemanticStep {
                    permit: permit.clone(),
                    origin: plan.origin.clone(),
                    output: plan.output.clone(),
                    tree_generation: plan.tree_generation,
                    prepared,
                    reply,
                })
                .is_err()
            {
                return Ok(incomplete(changed, Completion::Uncertain, committed));
            }
            let step_completion = match response.recv_timeout(Duration::from_secs(2)) {
                Ok(Ok(completion)) => completion,
                _ => return Ok(incomplete(changed, Completion::Uncertain, committed)),
            };
            match step_completion {
                Completion::Confirmed => completion = Completion::Confirmed,
                Completion::UiUpdated => {}
                other => return Ok(incomplete(changed, other, committed)),
            }
            committed = true;
            changed = true;
        }
        Ok(ActionOutcome {
            changed,
            completion,
            partial: false,
        })
    }
}

impl NickelSession {
    fn record_remote_production_effect(
        &mut self,
        effect: nickel_remote_control::desktop_events::ProductionEffectKind,
        completion: Completion,
    ) {
        use nickel_remote_control::desktop_events::{DesktopEventKind, ProductionEffectOutcome};
        let outcome = match completion {
            Completion::Confirmed => ProductionEffectOutcome::Confirmed,
            Completion::UiUpdated => ProductionEffectOutcome::UiUpdated,
            Completion::Requested => ProductionEffectOutcome::Requested,
            Completion::Cancelled => ProductionEffectOutcome::Cancelled,
            Completion::Unavailable => ProductionEffectOutcome::Unavailable,
            Completion::Uncertain => ProductionEffectOutcome::Uncertain,
        };
        self.remote_desktop_events.record(
            DesktopEventKind::ProductionEffectCompleted { effect, outcome },
            self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64,
        );
    }

    pub(super) fn commit_shell_semantic_step(
        &mut self,
        permit: &DesktopPermit,
        origin: &ResourceId,
        output: &ResourceId,
        tree_generation: u64,
        prepared: PreparedShellStep,
    ) -> Result<Completion, String> {
        if let PreparedShellStep::DeviceResult {
            registration,
            outcome,
        } = &prepared
        {
            let mut validate = || -> Result<Completion, String> {
                let mut registered = self
                    .remote_shell_origins
                    .remove(registration)
                    .ok_or("device origin was invalidated")?;
                if &registered.origin != origin
                    || &registered.output != output
                    || Instant::now() >= registered.expires
                {
                    return Err("device origin changed or expired".into());
                }
                self.with_surface_capture_authority(origin, permit, || Ok(()))?;
                let (_, current_output) = self.surface_capture_evidence(origin)?;
                if current_output.as_ref() != Some(output) {
                    return Err("device origin output changed".into());
                }
                // Accepted typed device actions survive incidental tree refresh.
                let controller_busy = self.poll_remote_controller_ownership();
                self.remote_semantic_input_idle(controller_busy)?;
                let completion = permit.with_input_boundary(&device_evidence(), |_| {
                    Ok(match outcome {
                        crate::platform::GuardedControlOutcome::DiscoveryStarted => {
                            Completion::Confirmed
                        }
                        crate::platform::GuardedControlOutcome::Confirmed => Completion::Confirmed,
                        crate::platform::GuardedControlOutcome::Requested => Completion::Requested,
                        crate::platform::GuardedControlOutcome::Cancelled => Completion::Cancelled,
                        crate::platform::GuardedControlOutcome::Unavailable => {
                            Completion::Unavailable
                        }
                        crate::platform::GuardedControlOutcome::Uncertain => Completion::Uncertain,
                    })
                })?;
                if matches!(
                    outcome,
                    crate::platform::GuardedControlOutcome::DiscoveryStarted
                ) {
                    if !registered.owner.has_standing_session() {
                        return Err("discovery session already retired".into());
                    }
                    registered.standing = true;
                    self.remote_shell_origins.insert(*registration, registered);
                }
                Ok(completion)
            };
            let completion = validate().unwrap_or(match outcome {
                crate::platform::GuardedControlOutcome::Cancelled
                | crate::platform::GuardedControlOutcome::Unavailable => Completion::Cancelled,
                _ => Completion::Uncertain,
            });
            self.record_remote_production_effect(
                nickel_remote_control::desktop_events::ProductionEffectKind::DeviceControl,
                completion,
            );
            return Ok(completion);
        }
        // Preparation and queueing cannot preserve stale visibility or membership.
        let (_, current_output) = self.surface_capture_evidence(origin)?;
        if current_output.as_ref() != Some(output) {
            return Err("shell output changed during effect preparation; do not retry".into());
        }
        let current = self.remote_surface_semantics(permit, &origin.id, origin.generation)?;
        if current.tree_generation != tree_generation {
            return Err("shell tree changed during effect preparation; do not retry".into());
        }
        let (effect, result) = match prepared {
            PreparedShellStep::Command(command) => self
                .remote_replay_shell_command(permit, origin, command)
                .map(|_| Completion::UiUpdated)
                .map(|completion| {
                    (
                        nickel_remote_control::desktop_events::ProductionEffectKind::ShellCommand,
                        completion,
                    )
                }),
            PreparedShellStep::DeviceResult { .. } => unreachable!("device result handled above"),
            PreparedShellStep::Launch(prepared) => self
                .remote_launch_installed_application(permit, *prepared)
                .map(|_| Completion::Confirmed)
                .map(|completion| {
                    (
                        nickel_remote_control::desktop_events::ProductionEffectKind::ApplicationLaunch,
                        completion,
                    )
                }),
        }?;
        self.record_remote_production_effect(effect, result);
        Ok(result)
    }
}

fn incomplete(changed: bool, completion: Completion, partial: bool) -> ActionOutcome {
    ActionOutcome {
        changed,
        completion,
        partial,
    }
}

impl NickelSession {
    pub(super) fn prepare_shell_device_action(
        &mut self,
        permit: &DesktopPermit,
        origin: &ResourceId,
        output: &ResourceId,
        action: crate::control_view::ControlAction,
    ) -> Result<ShellActionStep, String> {
        self.expire_remote_shell_origins();
        if self.remote_shell_origins.len() >= 16 {
            return Err("device origin capacity reached".into());
        }
        // Device state is global authority, independent of the originating panel grant.
        let expires =
            permit.with_input_boundary(&device_evidence(), |boundary| Ok(boundary.deadline()))?;
        self.remote_shell_origin_generation = self
            .remote_shell_origin_generation
            .checked_add(1)
            .ok_or("device origin generation exhausted")?;
        let registration = self.remote_shell_origin_generation;
        let owner = crate::platform::GuardedControlOriginOwner::new();
        let ticket = owner.ticket();
        self.remote_shell_origins.insert(
            registration,
            PendingDeviceOrigin {
                origin: origin.clone(),
                output: output.clone(),
                expires,
                owner,
                standing: false,
            },
        );
        Ok(ShellActionStep::Device {
            action,
            registration,
            ticket,
        })
    }

    pub(crate) fn invalidate_remote_shell_actions(&mut self) {
        self.remote_shell_origins.clear();
    }

    pub(super) fn expire_remote_shell_origins(&mut self) {
        let now = Instant::now();
        self.remote_shell_origins.retain(|_, entry| {
            if entry.standing {
                entry.owner.has_standing_session()
            } else {
                now < entry.expires
            }
        });
    }

    pub(super) fn invalidate_remote_shell_surface(
        &mut self,
        runtime: nickel_ui::InternalSurfaceId,
    ) {
        self.remote_shell_origins
            .retain(|_, entry| entry.origin.generation != runtime.snapshot_token());
    }
}

fn device_evidence() -> nickel_remote_control::leases::ResourceEvidence<'static> {
    nickel_remote_control::leases::ResourceEvidence {
        window: None,
        surface: None,
        output: None,
        verified_application: None,
        authorized_surface_ancestors: &[],
        protected: false,
    }
}

impl NickelSession {
    pub(super) fn invalidate_departed_shell_outputs(&mut self) {
        let outputs = self.space.outputs().cloned().collect::<Vec<_>>();
        self.remote_shell_origins.retain(|_, entry| {
            self.remote_output_generations
                .get(&entry.output.id)
                .is_some_and(|(native, generation)| {
                    *generation == entry.output.generation && outputs.contains(native)
                })
        });
    }
}
