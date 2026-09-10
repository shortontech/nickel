//! Read-only external accessibility, bound to native surface and process incarnations.
mod gtk_shell;

pub(super) use gtk_shell::install;
pub(in crate::session) use gtk_shell::{PressOrigin, record_pointer_press};

pub(in crate::session) use gtk_shell::association;
mod observe;
pub(in crate::session) use observe::observe;

#[derive(Clone)]
pub(in crate::session) struct Proof {
    pub id: u64,
    pub binding: Option<gtk_shell::Association>,
    pub process: super::remote_identity::ProcessIdentity,
    pub uid: u32,
    pub geometry: [i32; 4],
}
impl Proof {
    pub fn check_live(&self, permit: &nickel_remote_control::DesktopPermit) -> Result<(), String> {
        permit.check_live()?;
        if self
            .binding
            .as_ref()
            .is_some_and(|binding| !binding.current.load(std::sync::atomic::Ordering::Acquire))
            || !self.process.is_current()
        {
            return Err("native accessibility identity has retired".into());
        }
        Ok(())
    }
}

/// Held on the blocking worker until it actually finishes, including cancellation.
pub(in crate::session) struct Admission;
static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
impl Admission {
    pub fn acquire() -> Result<Self, String> {
        BUSY.compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .map_err(|_| "native accessibility observation is busy")?;
        Ok(Self)
    }
}
impl Drop for Admission {
    fn drop(&mut self) {
        BUSY.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// Provider-owned interval survives owner queue delay without refreshing data age.
pub(in crate::session) struct Observation {
    pub snapshot: nickel_remote_control::native_semantics::NativeSemanticSnapshot,
    pub started: std::time::Instant,
    pub completed: std::time::Instant,
}

pub(in crate::session) fn observation_times(
    session_start: std::time::Instant,
    started: std::time::Instant,
    completed: std::time::Instant,
    validated: std::time::Instant,
) -> [u64; 3] {
    [started, completed, validated].map(|time| {
        time.saturating_duration_since(session_start)
            .as_micros()
            .min(u64::MAX as u128) as u64
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn delayed_owner_validation_preserves_provider_observation_age() {
        use std::time::{Duration, Instant};
        let session = Instant::now();
        let started = session + Duration::from_secs(2);
        let completed = started + Duration::from_millis(400);
        let immediate = super::observation_times(session, started, completed, completed);
        let delayed = super::observation_times(
            session,
            started,
            completed,
            completed + Duration::from_secs(3),
        );
        assert_eq!(immediate, [2_000_000, 2_400_000, 2_400_000]);
        assert_eq!(delayed, [2_000_000, 2_400_000, 5_400_000]);
    }
}
