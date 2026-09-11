//! Secret-free native device observations for typed settings controls.
use nickel_remote_control::{
    DesktopPermit,
    device_settings::{Domain, Values},
};
#[derive(Clone, PartialEq, Eq)]
pub struct GuardedDeviceObservation {
    pub values: Values,
    pub(super) identity: NativeIdentity,
}
#[derive(Clone, PartialEq, Eq)]
pub(super) enum NativeIdentity {
    Audio { cookie: u32, serial: u64 },
    Bus { owner: String, path: String },
}

pub fn read_guarded_device(
    domain: Domain,
    permit: &DesktopPermit,
) -> Result<GuardedDeviceObservation, String> {
    permit.with_debug(false, || Ok(()))?;
    let observation = match domain {
        Domain::Audio => super::linux_audio::observe_guarded(permit),
        _ => super::linux_control::observe_guarded_device(domain, permit),
    }
    .map_err(|_| "native device settings unavailable".to_owned())?;
    permit.with_debug(false, || Ok(()))?;
    Ok(observation)
}
