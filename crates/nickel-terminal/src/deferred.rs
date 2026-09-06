//! Platform-neutral deferred-window launch classification.

use std::time::Duration;

pub const DEFAULT_CLASSIFICATION_DELAY: Duration = Duration::from_millis(100);
pub const MAX_CLASSIFICATION_DELAY: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LaunchGeneration(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchClass {
    KnownGraphical,
    KnownTerminal,
    Observe,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowClass {
    Application,
    Popup,
    Subsurface,
    LayerShell,
    Tray,
    Notification,
    Utility,
    Hidden,
}

impl WindowClass {
    pub const fn qualifies(self) -> bool {
        matches!(self, Self::Application)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowObservation {
    pub generation: LaunchGeneration,
    /// Monotonic time since the launch deadline was captured, recorded by the attribution owner.
    pub at: Duration,
    pub class: WindowClass,
    pub attributed: bool,
    pub descendant: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisibilityReason {
    ExplicitGraphicalMetadata,
    ExplicitTerminalMetadata,
    AttributedApplicationWindow,
    ClassificationDeadline,
    AttributionUnavailable,
    SpawnFailure,
    SessionFailure,
    CancelledBeforeSpawn,
    CancelledAfterSpawn,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisibilityDecision {
    Suppress(VisibilityReason),
    Show(VisibilityReason),
    Cancel(VisibilityReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchEvent {
    Window(WindowObservation),
    ChildExited { at: Duration, code: Option<i32> },
    SpawnFailed { at: Duration },
    SessionFailed { at: Duration },
    Cancelled { at: Duration, child_spawned: bool },
    AttributionUnavailable { at: Duration },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeferredLaunchTimeline {
    generation: LaunchGeneration,
    class: LaunchClass,
    deadline: Duration,
    observations: Vec<WindowObservation>,
    child_exit: Option<(Duration, Option<i32>)>,
    terminal_failure: Option<(Duration, VisibilityReason)>,
    cancellation: Option<(Duration, bool)>,
    attribution_unavailable: bool,
    decision: Option<VisibilityDecision>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum TimelineError {
    #[error("deferred launch delay exceeds the bounded maximum")]
    DelayTooLarge,
    #[error("event belongs to another launch generation")]
    WrongGeneration,
}

impl DeferredLaunchTimeline {
    pub fn new(
        generation: LaunchGeneration,
        class: LaunchClass,
        delay: Duration,
    ) -> Result<Self, TimelineError> {
        if delay > MAX_CLASSIFICATION_DELAY {
            return Err(TimelineError::DelayTooLarge);
        }
        let decision = match class {
            LaunchClass::KnownGraphical => Some(VisibilityDecision::Suppress(
                VisibilityReason::ExplicitGraphicalMetadata,
            )),
            LaunchClass::KnownTerminal => Some(VisibilityDecision::Show(
                VisibilityReason::ExplicitTerminalMetadata,
            )),
            LaunchClass::Observe if delay.is_zero() => Some(VisibilityDecision::Show(
                VisibilityReason::ClassificationDeadline,
            )),
            LaunchClass::Observe => None,
        };
        Ok(Self {
            generation,
            class,
            deadline: delay,
            observations: Vec::new(),
            child_exit: None,
            terminal_failure: None,
            cancellation: None,
            attribution_unavailable: false,
            decision,
        })
    }

    pub fn record(&mut self, event: LaunchEvent) -> Result<(), TimelineError> {
        if self.decision.is_some() {
            return Ok(());
        }
        match event {
            LaunchEvent::Window(observation) => {
                if observation.generation != self.generation {
                    return Err(TimelineError::WrongGeneration);
                }
                if observation.at <= self.deadline
                    && observation.attributed
                    && observation.class.qualifies()
                {
                    self.observations.push(observation);
                }
            }
            LaunchEvent::ChildExited { at, code } => self.child_exit = Some((at, code)),
            LaunchEvent::SpawnFailed { at } => {
                self.terminal_failure = Some((at, VisibilityReason::SpawnFailure));
            }
            LaunchEvent::SessionFailed { at } => {
                self.terminal_failure = Some((at, VisibilityReason::SessionFailure));
            }
            LaunchEvent::Cancelled { at, child_spawned } => {
                self.cancellation = Some((at, child_spawned));
            }
            LaunchEvent::AttributionUnavailable { .. } => self.attribution_unavailable = true,
        }
        Ok(())
    }

    /// Finalize after the ordered platform event source has published a watermark at or beyond the
    /// deadline. A timer callback alone is not a watermark; this distinction prevents callback
    /// scheduling from beating an earlier, already-recorded compositor observation.
    pub fn settle(&mut self, observed_through: Duration) -> Option<VisibilityDecision> {
        if let Some(decision) = self.decision {
            return Some(decision);
        }
        if self.class != LaunchClass::Observe || observed_through < self.deadline {
            return None;
        }
        let earliest = self
            .observations
            .iter()
            .min_by_key(|observation| observation.at)
            .copied();
        let decision = if self.cancellation.is_some_and(|(at, _)| at <= self.deadline) {
            let child_spawned = self.cancellation.unwrap().1;
            VisibilityDecision::Cancel(if child_spawned {
                VisibilityReason::CancelledAfterSpawn
            } else {
                VisibilityReason::CancelledBeforeSpawn
            })
        } else if let Some((at, reason)) = self.terminal_failure
            && at <= self.deadline
        {
            VisibilityDecision::Show(reason)
        } else if earliest.is_some() {
            VisibilityDecision::Suppress(VisibilityReason::AttributedApplicationWindow)
        } else if self.attribution_unavailable {
            VisibilityDecision::Show(VisibilityReason::AttributionUnavailable)
        } else {
            // Child exit never discards already captured output and therefore does not suppress UI.
            let _ = self.child_exit;
            VisibilityDecision::Show(VisibilityReason::ClassificationDeadline)
        };
        self.decision = Some(decision);
        Some(decision)
    }

    pub const fn decision(&self) -> Option<VisibilityDecision> {
        self.decision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed() -> DeferredLaunchTimeline {
        DeferredLaunchTimeline::new(
            LaunchGeneration(7),
            LaunchClass::Observe,
            DEFAULT_CLASSIFICATION_DELAY,
        )
        .unwrap()
    }

    #[test]
    fn exact_boundary_matrix_is_timestamp_owned() {
        for (milliseconds, suppressed) in [(0, true), (99, true), (100, true), (101, false)] {
            let mut timeline = observed();
            timeline
                .record(LaunchEvent::Window(WindowObservation {
                    generation: LaunchGeneration(7),
                    at: Duration::from_millis(milliseconds),
                    class: WindowClass::Application,
                    attributed: true,
                    descendant: false,
                }))
                .unwrap();
            assert_eq!(
                timeline.settle(DEFAULT_CLASSIFICATION_DELAY),
                Some(if suppressed {
                    VisibilityDecision::Suppress(VisibilityReason::AttributedApplicationWindow)
                } else {
                    VisibilityDecision::Show(VisibilityReason::ClassificationDeadline)
                }),
                "boundary {milliseconds} ms"
            );
        }
    }

    #[test]
    fn delivery_after_timer_does_not_beat_an_earlier_recorded_timestamp() {
        let mut timeline = observed();
        assert_eq!(timeline.settle(Duration::from_millis(99)), None);
        timeline
            .record(LaunchEvent::Window(WindowObservation {
                generation: LaunchGeneration(7),
                at: Duration::from_millis(50),
                class: WindowClass::Application,
                attributed: true,
                descendant: true,
            }))
            .unwrap();
        assert_eq!(
            timeline.settle(Duration::from_millis(100)),
            Some(VisibilityDecision::Suppress(
                VisibilityReason::AttributedApplicationWindow
            ))
        );
    }

    #[test]
    fn unrelated_and_non_application_surfaces_never_suppress_output() {
        for (class, attributed) in [
            (WindowClass::Application, false),
            (WindowClass::Popup, true),
            (WindowClass::Utility, true),
            (WindowClass::Hidden, true),
            (WindowClass::LayerShell, true),
        ] {
            let mut timeline = observed();
            timeline
                .record(LaunchEvent::Window(WindowObservation {
                    generation: LaunchGeneration(7),
                    at: Duration::ZERO,
                    class,
                    attributed,
                    descendant: false,
                }))
                .unwrap();
            assert_eq!(
                timeline.settle(DEFAULT_CLASSIFICATION_DELAY),
                Some(VisibilityDecision::Show(
                    VisibilityReason::ClassificationDeadline
                ))
            );
        }
    }

    #[test]
    fn fast_exit_and_failure_remain_visible_but_cancellation_is_terminal() {
        let mut exited = observed();
        exited
            .record(LaunchEvent::ChildExited {
                at: Duration::from_millis(1),
                code: Some(9),
            })
            .unwrap();
        assert!(matches!(
            exited.settle(DEFAULT_CLASSIFICATION_DELAY),
            Some(VisibilityDecision::Show(_))
        ));

        let mut failed = observed();
        failed
            .record(LaunchEvent::SpawnFailed { at: Duration::ZERO })
            .unwrap();
        assert_eq!(
            failed.settle(DEFAULT_CLASSIFICATION_DELAY),
            Some(VisibilityDecision::Show(VisibilityReason::SpawnFailure))
        );

        let mut cancelled = observed();
        cancelled
            .record(LaunchEvent::Cancelled {
                at: Duration::from_millis(10),
                child_spawned: true,
            })
            .unwrap();
        assert_eq!(
            cancelled.settle(DEFAULT_CLASSIFICATION_DELAY),
            Some(VisibilityDecision::Cancel(
                VisibilityReason::CancelledAfterSpawn
            ))
        );
    }

    #[test]
    fn metadata_and_unavailable_attribution_are_conservative() {
        assert_eq!(
            DeferredLaunchTimeline::new(
                LaunchGeneration(1),
                LaunchClass::KnownGraphical,
                DEFAULT_CLASSIFICATION_DELAY,
            )
            .unwrap()
            .decision(),
            Some(VisibilityDecision::Suppress(
                VisibilityReason::ExplicitGraphicalMetadata
            ))
        );
        let mut unavailable = observed();
        unavailable
            .record(LaunchEvent::AttributionUnavailable { at: Duration::ZERO })
            .unwrap();
        assert_eq!(
            unavailable.settle(DEFAULT_CLASSIFICATION_DELAY),
            Some(VisibilityDecision::Show(
                VisibilityReason::AttributionUnavailable
            ))
        );
    }

    #[test]
    fn generation_and_delay_bounds_fail_closed() {
        let mut timeline = observed();
        assert_eq!(
            timeline.record(LaunchEvent::Window(WindowObservation {
                generation: LaunchGeneration(8),
                at: Duration::ZERO,
                class: WindowClass::Application,
                attributed: true,
                descendant: false,
            })),
            Err(TimelineError::WrongGeneration)
        );
        assert_eq!(
            DeferredLaunchTimeline::new(
                LaunchGeneration(1),
                LaunchClass::Observe,
                MAX_CLASSIFICATION_DELAY + Duration::from_millis(1),
            ),
            Err(TimelineError::DelayTooLarge)
        );
    }
}
