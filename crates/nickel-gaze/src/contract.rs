use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackingState {
    Unavailable,
    Acquiring,
    Tracking,
    LowConfidence,
    FaceLost,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlinkPhase {
    Open,
    Closing,
    Closed,
    Reopening,
    Complete,
}

#[derive(Clone, Debug, Serialize)]
pub struct GazeSample {
    pub sequence: u64,
    pub timestamp_ms: u128,
    pub state: TrackingState,
    pub face_confidence: f32,
    pub left_eye_openness: f32,
    pub right_eye_openness: f32,
    pub blink: BlinkPhase,
    pub horizontal: f32,
    pub vertical: f32,
    pub gaze_confidence: f32,
    pub frame_age_ms: u128,
    pub inference_ms: f64,
}

#[derive(Debug)]
pub struct BlinkDetector {
    phase: BlinkPhase,
    closed_frames: u8,
}

impl Default for BlinkDetector {
    fn default() -> Self {
        Self {
            phase: BlinkPhase::Open,
            closed_frames: 0,
        }
    }
}

impl BlinkDetector {
    pub fn update(&mut self, left: f32, right: f32) -> BlinkPhase {
        let both_below = |threshold: f32| left < threshold && right < threshold;
        let both_above = |threshold: f32| left > threshold && right > threshold;
        self.phase = match self.phase {
            BlinkPhase::Open | BlinkPhase::Complete if both_below(0.45) => {
                self.closed_frames = 1;
                BlinkPhase::Closing
            }
            BlinkPhase::Open => BlinkPhase::Open,
            BlinkPhase::Closing if both_below(0.28) => {
                self.closed_frames = self.closed_frames.saturating_add(1);
                BlinkPhase::Closed
            }
            BlinkPhase::Closing => BlinkPhase::Open,
            BlinkPhase::Closed if both_above(0.43) => BlinkPhase::Reopening,
            BlinkPhase::Closed => {
                self.closed_frames = self.closed_frames.saturating_add(1);
                BlinkPhase::Closed
            }
            BlinkPhase::Reopening if both_above(0.43) && self.closed_frames >= 2 => {
                self.closed_frames = 0;
                BlinkPhase::Complete
            }
            BlinkPhase::Reopening if both_below(0.35) => BlinkPhase::Closed,
            BlinkPhase::Reopening => BlinkPhase::Reopening,
            BlinkPhase::Complete => BlinkPhase::Open,
        };
        self.phase
    }
}

#[cfg(test)]
mod tests {
    use super::{BlinkDetector, BlinkPhase};

    #[test]
    fn complete_blink_requires_close_and_reopen() {
        let mut detector = BlinkDetector::default();
        assert_eq!(detector.update(0.9, 0.9), BlinkPhase::Open);
        assert_eq!(detector.update(0.3, 0.3), BlinkPhase::Closing);
        assert_eq!(detector.update(0.1, 0.1), BlinkPhase::Closed);
        assert_eq!(detector.update(0.8, 0.8), BlinkPhase::Reopening);
        assert_eq!(detector.update(0.9, 0.9), BlinkPhase::Complete);
        assert_eq!(detector.update(0.9, 0.9), BlinkPhase::Open);
    }

    #[test]
    fn monocular_closure_does_not_complete_blink() {
        let mut detector = BlinkDetector::default();
        assert_eq!(detector.update(0.1, 0.9), BlinkPhase::Open);
        assert_eq!(detector.update(0.9, 0.9), BlinkPhase::Open);
    }
}
