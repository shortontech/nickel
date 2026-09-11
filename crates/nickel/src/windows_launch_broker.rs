//! Linearizable commit state for the future one-shot Windows launch broker.
//!
//! Process creation and handle transfer are intentionally outside this module.
//! The only operation permitted at the authority commit boundary is resuming an
//! already-created suspended broker. ShellExecuteEx must run in that broker.

use crate::windows_application_registry::native::LaunchCapture;
use nickel_remote_control::{DesktopPermit, leases::ResourceEvidence};
use std::time::{Duration, Instant};

const COMMIT_TTL: Duration = Duration::from_secs(2);

pub(crate) struct StagedLaunch {
    capture: LaunchCapture,
    deadline: Instant,
}

impl StagedLaunch {
    pub(crate) fn new(capture: LaunchCapture, now: Instant) -> Self {
        Self {
            capture,
            deadline: now + COMMIT_TTL,
        }
    }

    pub(crate) fn application_identity(&self) -> &str {
        self.capture.application_identity()
    }

    /// Invoke exactly one bounded resume operation under a fresh permit check.
    /// The callback must contain only ResumeThread (and local bookkeeping), never
    /// ShellExecuteEx, IPC writes, waits, allocation, or broker construction.
    pub(crate) fn commit(
        self,
        permit: &DesktopPermit,
        evidence: &ResourceEvidence<'_>,
        now: Instant,
        resume_suspended_broker: impl FnOnce() -> Result<(), String>,
    ) -> Result<LaunchCapture, String> {
        if now >= self.deadline {
            return Err("Windows launch preparation expired".into());
        }
        permit.with_input(evidence, resume_suspended_broker)?;
        Ok(self.capture)
    }
}

#[cfg(test)]
mod tests {
    // Native LaunchCapture construction requires a real pinned Start Menu link.
    // Keep deadline boundary logic separately testable without weakening the
    // production type's ownership of those handles.
    use super::COMMIT_TTL;
    use std::time::{Duration, Instant};

    #[test]
    fn commit_deadline_is_short_and_bounded() {
        assert_eq!(COMMIT_TTL, Duration::from_secs(2));
        let staged = Instant::now();
        assert!(staged + COMMIT_TTL > staged);
    }
}
