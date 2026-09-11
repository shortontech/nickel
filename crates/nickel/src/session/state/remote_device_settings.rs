//! Typed global device settings, admitted by the compositor and executed off-owner.
use super::*;
use crate::platform::{GuardedControlOriginOwner, GuardedControlOutcome, GuardedDeviceObservation};
use nickel_remote_control::device_settings::{Action, Outcome, Snapshot, Transaction};
use nickel_remote_control::semantics::SurfaceSemanticCompletion as Completion;

#[derive(Default)]
pub(super) struct DeviceState {
    next: u64,
    observations: std::collections::VecDeque<(u64, GuardedDeviceObservation)>,
    pending: std::collections::HashMap<u64, Pending>,
}
struct Pending {
    owner: GuardedControlOriginOwner,
    expires: Instant,
    standing: bool,
}
pub(super) struct Prepared {
    pub registration: u64,
    pub action: crate::control_view::ControlAction,
    pub ticket: crate::platform::GuardedControlOrigin,
}
impl DeviceState {
    pub fn invalidate(&mut self) {
        self.pending.clear();
        self.observations.clear();
    }
    pub fn expire(&mut self) {
        let now = Instant::now();
        self.pending.retain(|_, p| {
            if p.standing {
                p.owner.has_standing_session()
            } else {
                now < p.expires
            }
        });
    }
}
impl NickelSession {
    pub(super) fn remote_read_device_settings(
        &mut self,
        permit: &DesktopPermit,
        observed: GuardedDeviceObservation,
    ) -> Result<Snapshot, String> {
        self.device_settings_authority(permit, false)?;
        self.remote_devices.next = self
            .remote_devices
            .next
            .checked_add(1)
            .ok_or("device generation exhausted")?;
        let generation = self.remote_devices.next;
        if self.remote_devices.observations.len() >= 16 {
            self.remote_devices.observations.pop_front();
        }
        let values = observed.values.clone();
        self.remote_devices
            .observations
            .push_back((generation, observed));
        self.device_settings_authority(permit, false)?;
        Ok(Snapshot {
            generation,
            values,
            observed_at_micros: self
                .start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
        })
    }
    fn device_settings_authority(
        &mut self,
        permit: &DesktopPermit,
        input: bool,
    ) -> Result<Instant, String> {
        let protected = self.locked
            || self.shell_recovery_visible()
            || self
                .internal_ui
                .focused()
                .is_some_and(|s| self.internal_ui.remote_access_protected(s));
        let evidence = nickel_remote_control::leases::ResourceEvidence {
            window: None,
            surface: None,
            output: None,
            verified_application: None,
            authorized_surface_ancestors: &[],
            protected,
        };
        permit.with_resource(&evidence, || Ok(()))?;
        if input {
            let busy = self.poll_remote_controller_ownership();
            self.remote_semantic_input_idle(busy)?;
            permit.with_debug_input_deadline(protected, |b| Ok(b.deadline()))
        } else {
            permit.with_debug(protected, || Ok(Instant::now()))
        }
    }
    pub(super) fn remote_begin_device_settings(
        &mut self,
        permit: &DesktopPermit,
        transaction: Transaction,
    ) -> Result<Prepared, String> {
        if !transaction.valid() {
            return Err("invalid device transaction".into());
        }
        let expires = self.device_settings_authority(permit, true)?;
        self.remote_devices.expire();
        if self.remote_devices.pending.len() >= 16 {
            return Err("device origin capacity reached".into());
        }
        let index = self
            .remote_devices
            .observations
            .iter()
            .position(|(g, _)| *g == transaction.generation)
            .ok_or("device observation is stale; read again")?;
        let (_, observed) = self
            .remote_devices
            .observations
            .remove(index)
            .ok_or("device observation is stale")?;
        if observed.values != transaction.prior {
            return Err("device prior values changed".into());
        }
        let owner = GuardedControlOriginOwner::for_device(observed);
        let ticket = owner.ticket();
        self.remote_devices.pending.insert(
            transaction.generation,
            Pending {
                owner,
                expires,
                standing: false,
            },
        );
        use crate::control_view::ControlAction as C;
        let action = match transaction.action {
            Action::AudioVolume(v) => C::SetAudioVolume(v),
            Action::AudioMuted(v) => C::SetAudioMuted(v),
            Action::WifiPowered(v) => C::SetWifiEnabled(v),
            Action::BluetoothPowered(v) => C::SetBluetoothPowered(v),
            Action::BluetoothDiscovery(v) => C::SetBluetoothDiscovery(v),
        };
        Ok(Prepared {
            registration: transaction.generation,
            action,
            ticket,
        })
    }
    pub(super) fn remote_finish_device_settings(
        &mut self,
        permit: &DesktopPermit,
        registration: u64,
        native: GuardedControlOutcome,
    ) -> Result<Outcome, String> {
        let Some(mut pending) = self.remote_devices.pending.remove(&registration) else {
            return Ok(Outcome {
                completion: if matches!(
                    native,
                    GuardedControlOutcome::Cancelled | GuardedControlOutcome::Unavailable
                ) {
                    Completion::Cancelled
                } else {
                    Completion::Uncertain
                },
            });
        };
        if self.device_settings_authority(permit, true).is_err()
            || Instant::now() >= pending.expires
        {
            return Ok(Outcome {
                completion: Completion::Uncertain,
            });
        }
        let completion = match native {
            GuardedControlOutcome::DiscoveryStarted => {
                if !pending.owner.has_standing_session() {
                    return Ok(Outcome {
                        completion: Completion::Uncertain,
                    });
                }
                pending.standing = true;
                self.remote_devices.pending.insert(registration, pending);
                Completion::Confirmed
            }
            GuardedControlOutcome::Confirmed => Completion::Confirmed,
            GuardedControlOutcome::Requested => Completion::Requested,
            GuardedControlOutcome::Cancelled => Completion::Cancelled,
            GuardedControlOutcome::Unavailable => Completion::Unavailable,
            GuardedControlOutcome::Uncertain => Completion::Uncertain,
        };
        Ok(Outcome { completion })
    }
}
