use super::{NickelSession, RemoteDisplayRecovery};
use nickel_remote_control::display_layout::{
    Layout, Placement, Recovery, RecoveryState, Snapshot, Transaction,
};
use std::time::{Duration, Instant};

const RECOVERY_DURATION: Duration = Duration::from_secs(15);

impl NickelSession {
    fn current_remote_display_layout(&mut self) -> Result<Layout, String> {
        self.refresh_output_topology_generation();
        self.refresh_remote_output_identities();
        let outputs = self.protocol_outputs();
        let expected = self.space.outputs().count();
        #[cfg(feature = "backend-udev")]
        let expected = self
            .native
            .as_ref()
            .map_or(expected, |_| self.native_output_inventory().len());
        if outputs.is_empty()
            || outputs.len() != expected
            || outputs.len() > nickel_remote_control::display_layout::MAX_LAYOUT_OUTPUTS
        {
            return Err("complete display layout exceeds the observation bound".into());
        }
        let mut placements = outputs
            .iter()
            .map(|output| {
                let identity = self
                    .remote_output_identity(output.name.clone())
                    .ok_or("display output identity is unavailable")?;
                Ok(Placement {
                    output: identity,
                    x: output.geometry.x,
                    y: output.geometry.y,
                    enabled: output.enabled,
                    scale_120: output.scale_120,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        placements.sort_by(|left, right| left.output.id.cmp(&right.output.id));
        let primary_name = outputs
            .iter()
            .find(|output| output.primary && output.enabled)
            .map(|output| output.name.clone())
            .ok_or("display layout has no enabled primary output")?;
        let layout = Layout {
            primary: self
                .remote_output_identity(primary_name)
                .ok_or("primary display identity is unavailable")?,
            outputs: placements,
        };
        layout
            .valid_representation()
            .then_some(layout)
            .ok_or_else(|| "production display layout is invalid".into())
    }

    fn remote_display_snapshot(&mut self) -> Result<Snapshot, String> {
        let requested = self.current_remote_display_layout()?;
        let now = Instant::now();
        let observed_at_us = self
            .start_time
            .elapsed()
            .as_micros()
            .min(u128::from(u64::MAX)) as u64;
        let (confirmed, recovery) = self.remote_display_recovery.as_ref().map_or_else(
            || {
                (
                    requested.clone(),
                    Recovery {
                        state: RecoveryState::Confirmed,
                        generation: 0,
                        deadline_uptime_us: None,
                    },
                )
            },
            |pending| {
                let deadline_uptime_us = pending.deadline.map(|deadline| {
                    observed_at_us.saturating_add(
                        deadline
                            .saturating_duration_since(now)
                            .as_micros()
                            .min(u128::from(u64::MAX)) as u64,
                    )
                });
                (
                    pending.confirmed.clone(),
                    Recovery {
                        state: if pending.revert_failed {
                            RecoveryState::RevertFailed
                        } else {
                            RecoveryState::AwaitingConfirmation
                        },
                        generation: pending.generation,
                        deadline_uptime_us,
                    },
                )
            },
        );
        self.remote_observation_generation = self.remote_observation_generation.saturating_add(1);
        Ok(Snapshot {
            observation_generation: self.remote_observation_generation,
            observed_at_us,
            topology_generation: self.output_topology_generation,
            transaction_supported: true,
            transaction_unavailable_reason: None,
            requested,
            confirmed,
            recovery,
        })
    }

    fn remote_display_input_busy(&mut self) -> bool {
        self.poll_remote_controller_ownership()
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
    }

    fn session_layout(layout: &Layout) -> nickel_session_protocol::OutputLayout {
        nickel_session_protocol::OutputLayout {
            primary: layout.primary.id.clone(),
            placements: layout
                .outputs
                .iter()
                .map(|placement| nickel_session_protocol::OutputPlacement {
                    name: placement.output.id.clone(),
                    x: placement.x,
                    y: placement.y,
                    enabled: placement.enabled,
                    scale_120: placement.scale_120,
                    mode: None,
                })
                .collect(),
        }
    }

    pub(super) fn remote_read_display_layout(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
    ) -> Result<Snapshot, String> {
        permit.with_debug(self.locked || self.shell_recovery_visible(), || Ok(()))?;
        let snapshot = self.remote_display_snapshot()?;
        permit.check_live()?;
        Ok(snapshot)
    }

    pub(super) fn remote_display_layout_transaction(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
        transaction: Transaction,
    ) -> Result<Snapshot, String> {
        self.refresh_output_topology_generation();
        let protected = self.locked || self.shell_recovery_visible();
        match transaction {
            Transaction::Apply {
                topology_generation,
                prior,
                requested,
            } => {
                permit.with_debug_input_deadline(protected, |boundary| {
                    if self.remote_display_input_busy() {
                        return Err("shared input is busy".into());
                    }
                    if topology_generation != self.output_topology_generation
                        || self.remote_display_recovery.is_some()
                    {
                        return Err("display layout transaction has stale owner state".into());
                    }
                    let current = self.current_remote_display_layout()?;
                    if prior != current
                        || !requested.valid_representation()
                        || !requested.same_output_incarnations(&current)
                    {
                        return Err(
                            "display layout transaction has stale or incomplete output evidence"
                                .into(),
                        );
                    }
                    permit.check_commit_boundary(boundary)?;
                    self.apply_output_layout(Self::session_layout(&requested))
                        .map_err(str::to_owned)?;
                    self.refresh_output_topology_generation();
                    self.refresh_remote_output_identities();
                    let generation = self
                        .remote_display_recovery_generation
                        .checked_add(1)
                        .ok_or("display recovery generations exhausted")?;
                    self.remote_display_recovery_generation = generation;
                    self.remote_display_recovery = Some(RemoteDisplayRecovery {
                        owner: permit.clone(),
                        generation,
                        confirmed: current,
                        deadline: Some(Instant::now() + RECOVERY_DURATION),
                        revert_failed: false,
                    });
                    Ok(())
                })?;
            }
            Transaction::Keep {
                topology_generation,
                recovery_generation,
            } => {
                permit.with_debug(protected, || {
                    if topology_generation != self.output_topology_generation {
                        return Err("display layout transaction has stale topology".into());
                    }
                    let pending = self
                        .remote_display_recovery
                        .as_ref()
                        .ok_or("no display recovery is pending")?;
                    if pending.generation != recovery_generation
                        || !pending.owner.same_lease_as(permit)
                    {
                        return Err(
                            "display recovery is owned by another lease or generation".into()
                        );
                    }
                    self.remote_display_recovery = None;
                    Ok(())
                })?;
            }
            Transaction::Revert {
                topology_generation,
                recovery_generation,
            } => {
                permit.with_debug_input_deadline(protected, |boundary| {
                    if self.remote_display_input_busy() {
                        return Err("shared input is busy".into());
                    }
                    if topology_generation != self.output_topology_generation {
                        return Err("display layout transaction has stale topology".into());
                    }
                    let pending = self
                        .remote_display_recovery
                        .as_ref()
                        .ok_or("no display recovery is pending")?;
                    if pending.generation != recovery_generation
                        || !pending.owner.same_lease_as(permit)
                    {
                        return Err(
                            "display recovery is owned by another lease or generation".into()
                        );
                    }
                    let confirmed = pending.confirmed.clone();
                    permit.check_commit_boundary(boundary)?;
                    self.apply_output_layout(Self::session_layout(&confirmed))
                        .map_err(str::to_owned)?;
                    self.remote_display_recovery = None;
                    self.refresh_output_topology_generation();
                    self.refresh_remote_output_identities();
                    Ok(())
                })?;
            }
        }
        self.remote_display_snapshot()
    }

    pub(super) fn expire_remote_display_recovery(&mut self) {
        let Some(confirmed) = self
            .remote_display_recovery
            .as_ref()
            .filter(|pending| {
                pending
                    .deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
            })
            .map(|pending| pending.confirmed.clone())
        else {
            return;
        };
        if self
            .apply_output_layout(Self::session_layout(&confirmed))
            .is_ok()
        {
            self.remote_display_recovery = None;
            self.refresh_output_topology_generation();
            self.refresh_remote_output_identities();
        } else if let Some(pending) = self.remote_display_recovery.as_mut() {
            pending.deadline = None;
            pending.revert_failed = true;
        }
    }
}
