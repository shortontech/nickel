//! Scrubbed printer/removable diagnostics and guarded printer controls.
use super::*;
use nickel_remote_control::peripheral_controls::{Outcome, Snapshot, Transaction};
use nickel_remote_control::semantics::SurfaceSemanticCompletion as Completion;
use std::sync::Arc;

pub(super) struct Prepared {
    pub native: crate::remote_peripheral_controls::Prepared,
    pub held: Arc<nickel_remote_control::HeldInput>,
    pub deadline: Instant,
}

impl NickelSession {
    fn peripheral_authority(
        &mut self,
        permit: &DesktopPermit,
        input: bool,
    ) -> Result<bool, String> {
        let protected = self.locked
            || self.shell_recovery_visible()
            || self
                .internal_ui
                .focused()
                .is_some_and(|surface| self.internal_ui.remote_access_protected(surface));
        permit.with_debug(protected, || Ok(()))?;
        if input {
            let busy = self.poll_remote_controller_ownership();
            self.remote_semantic_input_idle(busy)?;
        }
        Ok(protected)
    }

    pub(super) fn remote_read_peripheral_controls(
        &mut self,
        permit: &DesktopPermit,
        observed: nickel_platform::PeripheralSnapshot,
    ) -> Result<Snapshot, String> {
        self.peripheral_authority(permit, false)?;
        let snapshot = self.remote_peripherals.observe(
            observed,
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
            true,
        )?;
        self.peripheral_authority(permit, false)?;
        Ok(snapshot)
    }

    pub(super) fn remote_begin_peripheral_control(
        &mut self,
        permit: &DesktopPermit,
        transaction: Transaction,
        deadline: Instant,
    ) -> Result<Prepared, String> {
        let protected = self.peripheral_authority(permit, true)?;
        if Instant::now() >= deadline {
            return Err("peripheral control expired before admission".into());
        }
        let native = self.remote_peripherals.prepare(transaction)?;
        let held = permit.begin_debug_input(protected, || Ok(()))?;
        Ok(Prepared {
            native,
            held: Arc::new(held),
            deadline,
        })
    }

    pub(super) fn remote_finish_peripheral_control(
        &mut self,
        permit: &DesktopPermit,
        prepared: Prepared,
        outcome: nickel_platform::PeripheralOutcome,
        refreshed: Option<nickel_platform::PeripheralSnapshot>,
    ) -> Result<Outcome, String> {
        let protected = match self.peripheral_authority(permit, false) {
            Ok(protected) => protected,
            Err(_) => {
                return Ok(Outcome {
                    completion: Completion::Uncertain,
                });
            }
        };
        if Instant::now() >= prepared.deadline
            || permit
                .continue_debug_input(&prepared.held, protected, || Ok(()))
                .is_err()
        {
            return Ok(Outcome {
                completion: Completion::Uncertain,
            });
        }
        Ok(Outcome {
            completion: prepared.native.completion(outcome, refreshed.as_ref()),
        })
    }
}
