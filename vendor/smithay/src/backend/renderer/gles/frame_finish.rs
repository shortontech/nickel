//! Finalization policy, separate from GL calls so fallback behavior is testable.

use super::GlesError;

#[derive(Clone, Copy)]
pub(super) enum FinishMode {
    AllowBlocking,
    NoBlocking,
}

impl FinishMode {
    pub(super) fn complete<T>(
        self,
        exported: Option<T>,
        calibrate: impl FnOnce(),
        finish: impl FnOnce() -> T,
    ) -> Result<T, GlesError> {
        // Clock calibration is itself a potentially synchronous GL query, even
        // when fence export succeeded. Optional work must avoid that too.
        if matches!(self, Self::AllowBlocking) {
            calibrate();
        }
        match (exported, self) {
            (Some(sync), _) => Ok(sync),
            (None, Self::AllowBlocking) => Ok(finish()),
            (None, Self::NoBlocking) => Err(GlesError::SyncExportFailed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn nonblocking_finish_never_calibrates_or_waits_even_when_fence_export_fails() {
        for exported in [Some(7), None] {
            let result = FinishMode::NoBlocking.complete(
                exported,
                || panic!("GPU clock calibration can block"),
                || panic!("glFinish fallback must not run"),
            );
            match exported {
                Some(expected) => assert_eq!(result.unwrap(), expected),
                None => assert!(matches!(result, Err(GlesError::SyncExportFailed))),
            }
        }
    }

    #[test]
    fn ordinary_finish_keeps_calibration_and_only_waits_without_a_fence() {
        for exported in [Some(7), None] {
            let events = RefCell::new(Vec::new());
            let result = FinishMode::AllowBlocking.complete(
                exported,
                || events.borrow_mut().push("calibrate"),
                || {
                    events.borrow_mut().push("finish");
                    9
                },
            );
            assert_eq!(result.unwrap(), exported.unwrap_or(9));
            if exported.is_some() {
                assert_eq!(*events.borrow(), ["calibrate"]);
            } else {
                assert_eq!(*events.borrow(), ["calibrate", "finish"]);
            }
        }
    }
}
