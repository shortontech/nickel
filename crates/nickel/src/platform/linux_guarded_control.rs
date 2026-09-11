//! Bounded owned device work. The reply reports native completion, never queue admission.
use crate::control_view::ControlAction;
use nickel_remote_control::DesktopPermit;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardedControlOutcome {
    Confirmed,
    DiscoveryStarted,
    Requested,
    Cancelled,
    Unavailable,
    Uncertain,
}

/// Compositor-owned cancellation for material changes to an accepted origin.
/// Dropping the owner invalidates every queued/in-flight ticket; never rearmed.
struct OriginState {
    live: AtomicBool,
    standing: AtomicBool,
    expected: Option<super::linux_device_settings::GuardedDeviceObservation>,
}
pub struct GuardedControlOriginOwner(Arc<OriginState>);

#[derive(Clone)]
pub struct GuardedControlOrigin(Arc<OriginState>);

impl Default for GuardedControlOriginOwner {
    fn default() -> Self {
        Self::new()
    }
}
impl GuardedControlOriginOwner {
    pub fn new() -> Self {
        Self(Arc::new(OriginState {
            live: AtomicBool::new(true),
            standing: AtomicBool::new(false),
            expected: None,
        }))
    }
    pub fn for_device(expected: super::linux_device_settings::GuardedDeviceObservation) -> Self {
        Self(Arc::new(OriginState {
            live: AtomicBool::new(true),
            standing: AtomicBool::new(false),
            expected: Some(expected),
        }))
    }
    pub fn ticket(&self) -> GuardedControlOrigin {
        GuardedControlOrigin(Arc::clone(&self.0))
    }
    pub fn has_standing_session(&self) -> bool {
        self.0.live.load(Ordering::Acquire) && self.0.standing.load(Ordering::Acquire)
    }
    pub fn invalidate(&self) {
        self.0.live.store(false, Ordering::Release);
    }
}
impl Drop for GuardedControlOriginOwner {
    fn drop(&mut self) {
        self.invalidate();
    }
}
impl GuardedControlOrigin {
    pub(super) fn validate_observation(
        &self,
        current: &super::linux_device_settings::GuardedDeviceObservation,
    ) -> Result<(), String> {
        self.check()?;
        if self
            .0
            .expected
            .as_ref()
            .is_some_and(|expected| expected != current)
        {
            return Err("device observation changed".into());
        }
        Ok(())
    }
    pub(super) fn validate_bus_target(&self, owner: &str, path: &str) -> Result<(), String> {
        if let Some(expected) = &self.0.expected
            && !matches!(&expected.identity,super::linux_device_settings::NativeIdentity::Bus{owner:current,path:target} if current==owner && target==path)
        {
            return Err("device target incarnation changed".into());
        }
        Ok(())
    }
    pub(super) fn expects_device(&self) -> bool {
        self.0.expected.is_some()
    }
    pub(super) fn with_boundary<T>(
        &self,
        permit: &DesktopPermit,
        evidence: &nickel_remote_control::leases::ResourceEvidence<'_>,
        action: impl FnOnce(&nickel_remote_control::CommitBoundary) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.expects_device() {
            permit.with_debug_input_deadline(false, action)
        } else {
            permit.with_input_boundary(evidence, action)
        }
    }
    pub(super) fn set_standing(&self, active: bool) {
        self.0.standing.store(active, Ordering::Release);
    }
    pub(super) fn check(&self) -> Result<(), String> {
        if self.0.live.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err("remote control origin was invalidated".into())
        }
    }
}

struct Job {
    action: ControlAction,
    permit: DesktopPermit,
    origin: GuardedControlOrigin,
    reply: mpsc::SyncSender<GuardedControlOutcome>,
}
const CAPACITY: usize = 16;
static WORKER: OnceLock<mpsc::SyncSender<Job>> = OnceLock::new();

pub fn submit_guarded_control(
    action: ControlAction,
    permit: DesktopPermit,
    origin: GuardedControlOrigin,
) -> Result<mpsc::Receiver<GuardedControlOutcome>, &'static str> {
    match &action {
        ControlAction::SetAudioVolume(value) if *value <= 100 => {}
        ControlAction::SetAudioMuted(_) => {}
        ControlAction::SelectAudioDevice { id }
            if !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control) => {}
        ControlAction::SetWifiEnabled(_)
        | ControlAction::SetBluetoothPowered(_)
        | ControlAction::SetBluetoothDiscovery(_) => {}
        ControlAction::ActivateWifi { id }
            if !id.is_empty() && id.len() <= 512 && id.split('\t').count() == 2 => {}
        ControlAction::ToggleBluetoothDevice { id }
            if !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control) => {}
        _ => return Err("guarded device operation unavailable"),
    }
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<Job>(CAPACITY);
        let _ = std::thread::Builder::new()
            .name("nickel-guarded-devices".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    let result = match job.action {
                        ControlAction::SetAudioVolume(_)
                        | ControlAction::SetAudioMuted(_)
                        | ControlAction::SelectAudioDevice { .. } => {
                            super::linux_audio::execute_guarded(job.action, job.permit, job.origin)
                        }
                        _ => super::linux_control::execute_guarded(
                            job.action, job.permit, job.origin,
                        ),
                    };
                    let _ = job.reply.try_send(result);
                }
            });
        tx
    });
    let (reply, receiver) = mpsc::sync_channel(1);
    worker
        .try_send(Job {
            action,
            permit,
            origin,
            reply,
        })
        .map_err(|_| "guarded device queue unavailable or full")?;
    Ok(receiver)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn material_origin_invalidation_and_owner_drop_cannot_rearm_old_tickets() {
        let owner = GuardedControlOriginOwner::new();
        let first = owner.ticket();
        let clone = first.clone();
        assert!(first.check().is_ok());
        assert!(!owner.has_standing_session());
        first.set_standing(true);
        assert!(owner.has_standing_session());
        owner.invalidate();
        assert!(!owner.has_standing_session());
        assert!(first.check().is_err());
        assert!(clone.check().is_err());
        assert!(owner.ticket().check().is_err());
        let replacement = GuardedControlOriginOwner::new();
        let fresh = replacement.ticket();
        assert!(fresh.check().is_ok());
        drop(replacement);
        assert!(fresh.check().is_err());
        assert!(first.check().is_err());
    }
}
