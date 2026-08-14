use crate::contract::{BlinkPhase, TrackingState};
use std::{collections::VecDeque, time::Duration};

pub const COLUMNS: usize = 7;
pub const ROWS: usize = 4;
type GazePoint = (f32, f32);
type OpenSample = (GazePoint, GazePoint, GazePoint);
type CursorSample = (Duration, GazePoint, (GazePoint, GazePoint));
type VisibleEyeCursors = (Option<GazePoint>, Option<GazePoint>);
const DISPLAY_AVERAGE_WINDOW: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridLayout {
    pub bounds: GridRect,
    pub cell: f32,
}

impl GridLayout {
    pub fn fit(width: f32, height: f32, header: f32) -> Self {
        let available_height = (height - header).max(0.0);
        let cell = (width / COLUMNS as f32)
            .min(available_height / ROWS as f32)
            .max(0.0);
        let grid_width = cell * COLUMNS as f32;
        let grid_height = cell * ROWS as f32;
        Self {
            bounds: GridRect {
                x: (width - grid_width) * 0.5,
                y: header + (available_height - grid_height) * 0.5,
                width: grid_width,
                height: grid_height,
            },
            cell,
        }
    }

    pub fn cell_at(&self, normalized: (f32, f32)) -> (usize, usize) {
        let column = (normalized.0.clamp(0.0, 0.999_999) * COLUMNS as f32) as usize;
        let row = (normalized.1.clamp(0.0, 0.999_999) * ROWS as f32) as usize;
        (column, row)
    }

    pub fn cursor(&self, normalized: (f32, f32)) -> (f32, f32) {
        (
            self.bounds.x + normalized.0.clamp(0.0, 1.0) * self.bounds.width,
            self.bounds.y + normalized.1.clamp(0.0, 1.0) * self.bounds.height,
        )
    }

    pub fn center(&self) -> (f32, f32) {
        (
            self.bounds.x + self.bounds.width * 0.5,
            self.bounds.y + self.bounds.height * 0.5,
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GridObservation {
    pub state: TrackingState,
    pub blink: BlinkPhase,
    pub horizontal: f32,
    pub vertical: f32,
    pub left_gaze: (f32, f32),
    pub right_gaze: (f32, f32),
    pub left_eye_openness: f32,
    pub right_eye_openness: f32,
    pub confidence: f32,
}

#[derive(Debug, Default)]
pub struct GridTracker {
    armed: bool,
    neutral: Option<(f32, f32)>,
    recent_open: VecDeque<OpenSample>,
    cursor: Option<(f32, f32)>,
    eye_neutral: Option<((f32, f32), (f32, f32))>,
    eye_cursors: Option<((f32, f32), (f32, f32))>,
    cursor_history: VecDeque<CursorSample>,
    eye_visible: (bool, bool),
    last_good: Option<Duration>,
}

impl GridTracker {
    pub fn arm(&mut self) {
        self.armed = true;
        self.neutral = None;
        self.cursor = None;
        self.eye_neutral = None;
        self.eye_cursors = None;
        self.cursor_history.clear();
        self.recent_open.clear();
    }

    pub fn armed(&self) -> bool {
        self.armed
    }

    pub fn neutral(&self) -> Option<(f32, f32)> {
        self.neutral
    }

    pub fn cursor(&self, now: Duration) -> Option<(f32, f32)> {
        if !self.eye_visible.0 || !self.eye_visible.1 {
            return None;
        }
        self.last_good
            .filter(|last| now.saturating_sub(*last) <= Duration::from_millis(350))?;
        average_points(
            self.cursor_history
                .iter()
                .filter(|sample| now.saturating_sub(sample.0) <= DISPLAY_AVERAGE_WINDOW)
                .map(|sample| sample.1),
        )
    }

    pub fn eye_cursors(&self, now: Duration) -> VisibleEyeCursors {
        if self
            .last_good
            .is_none_or(|last| now.saturating_sub(last) > Duration::from_millis(350))
        {
            return (None, None);
        }
        let recent = self
            .cursor_history
            .iter()
            .filter(|sample| now.saturating_sub(sample.0) <= DISPLAY_AVERAGE_WINDOW);
        let left = self
            .eye_visible
            .0
            .then(|| average_points(recent.clone().map(|sample| sample.2.0)))
            .flatten();
        let right = self
            .eye_visible
            .1
            .then(|| average_points(recent.map(|sample| sample.2.1)))
            .flatten();
        (left, right)
    }

    pub fn update(&mut self, observation: GridObservation, now: Duration) {
        let confident =
            observation.state == TrackingState::Tracking && observation.confidence >= 0.15;
        self.eye_visible = (
            observation.left_eye_openness > 0.30,
            observation.right_eye_openness > 0.30,
        );
        if !confident {
            self.cursor = None;
            self.eye_cursors = None;
            self.cursor_history.clear();
            if self.armed {
                self.recent_open.clear();
            }
            return;
        }

        if self.armed && observation.blink == BlinkPhase::Open {
            self.recent_open.push_back((
                (observation.horizontal, observation.vertical),
                observation.left_gaze,
                observation.right_gaze,
            ));
            while self.recent_open.len() > 12 {
                self.recent_open.pop_front();
            }
        }
        if self.armed && observation.blink == BlinkPhase::Complete && self.recent_open.len() >= 3 {
            let count = self.recent_open.len() as f32;
            let sums =
                self.recent_open
                    .iter()
                    .fold(((0.0, 0.0), (0.0, 0.0), (0.0, 0.0)), |sum, value| {
                        (
                            (sum.0.0 + value.0.0 / count, sum.0.1 + value.0.1 / count),
                            (sum.1.0 + value.1.0 / count, sum.1.1 + value.1.1 / count),
                            (sum.2.0 + value.2.0 / count, sum.2.1 + value.2.1 / count),
                        )
                    });
            self.neutral = Some(sums.0);
            self.eye_neutral = Some((sums.1, sums.2));
            self.armed = false;
        }
        if let Some(neutral) = self.neutral {
            self.cursor = Some(map_gaze(
                (observation.horizontal, observation.vertical),
                neutral,
            ));
            if let Some((left, right)) = self.eye_neutral {
                self.eye_cursors = Some((
                    map_gaze(observation.left_gaze, left),
                    map_gaze(observation.right_gaze, right),
                ));
            }
            if self.eye_visible.0
                && self.eye_visible.1
                && let (Some(cursor), Some(eyes)) = (self.cursor, self.eye_cursors)
            {
                self.cursor_history.push_back((now, cursor, eyes));
                while self
                    .cursor_history
                    .front()
                    .is_some_and(|sample| now.saturating_sub(sample.0) > DISPLAY_AVERAGE_WINDOW)
                {
                    self.cursor_history.pop_front();
                }
            }
            self.last_good = Some(now);
        }
    }
}

fn map_gaze(value: (f32, f32), neutral: (f32, f32)) -> (f32, f32) {
    (
        (0.5 - (value.0 - neutral.0) * 1.75).clamp(0.0, 1.0),
        (0.5 + (value.1 - neutral.1) * 1.75).clamp(0.0, 1.0),
    )
}

fn average_points(points: impl Iterator<Item = GazePoint>) -> Option<GazePoint> {
    let (sum, count) = points.fold(((0.0, 0.0), 0_u32), |(sum, count), point| {
        ((sum.0 + point.0, sum.1 + point.1), count + 1)
    });
    (count > 0).then(|| (sum.0 / count as f32, sum.1 / count as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(blink: BlinkPhase) -> GridObservation {
        GridObservation {
            state: TrackingState::Tracking,
            blink,
            horizontal: 0.1,
            vertical: -0.1,
            left_gaze: (0.1, -0.1),
            right_gaze: (0.1, -0.1),
            left_eye_openness: 0.9,
            right_eye_openness: 0.9,
            confidence: 0.9,
        }
    }

    #[test]
    fn grid_is_letterboxed_with_square_cells() {
        let layout = GridLayout::fit(1000.0, 800.0, 100.0);
        assert_eq!(layout.bounds.width, layout.cell * 7.0);
        assert_eq!(layout.bounds.height, layout.cell * 4.0);
        assert_eq!(layout.center().1, layout.bounds.y + layout.cell * 2.0);
    }

    #[test]
    fn hit_testing_clamps_to_twenty_eight_cells() {
        let layout = GridLayout::fit(700.0, 500.0, 100.0);
        assert_eq!(layout.cell_at((0.0, 0.0)), (0, 0));
        assert_eq!(layout.cell_at((1.0, 1.0)), (6, 3));
    }

    #[test]
    fn complete_blink_centers_from_surrounding_open_samples() {
        let mut tracker = GridTracker::default();
        tracker.arm();
        for index in 0..4 {
            tracker.update(observation(BlinkPhase::Open), Duration::from_millis(index));
        }
        tracker.update(observation(BlinkPhase::Complete), Duration::from_millis(5));
        assert_eq!(tracker.neutral(), Some((0.1, -0.1)));
        assert_eq!(tracker.cursor(Duration::from_millis(5)), Some((0.5, 0.5)));
    }

    #[test]
    fn low_confidence_and_staleness_clear_cursor() {
        let mut tracker = GridTracker::default();
        tracker.arm();
        for index in 0..3 {
            tracker.update(observation(BlinkPhase::Open), Duration::from_millis(index));
        }
        tracker.update(observation(BlinkPhase::Complete), Duration::from_millis(3));
        assert!(tracker.cursor(Duration::from_millis(400)).is_none());
        let mut lost = observation(BlinkPhase::Open);
        lost.state = TrackingState::FaceLost;
        tracker.update(lost, Duration::from_millis(10));
        assert!(tracker.cursor(Duration::from_millis(10)).is_none());
    }

    #[test]
    fn recentering_discards_previous_reference() {
        let mut tracker = GridTracker::default();
        tracker.arm();
        for index in 0..3 {
            tracker.update(observation(BlinkPhase::Open), Duration::from_millis(index));
        }
        tracker.update(observation(BlinkPhase::Complete), Duration::from_millis(3));
        tracker.arm();
        assert!(tracker.neutral().is_none());
        assert!(tracker.cursor(Duration::from_millis(3)).is_none());
    }

    #[test]
    fn camera_relative_horizontal_is_mirrored_to_screen_direction() {
        let mut tracker = GridTracker::default();
        tracker.arm();
        for index in 0..3 {
            tracker.update(observation(BlinkPhase::Open), Duration::from_millis(index));
        }
        tracker.update(observation(BlinkPhase::Complete), Duration::from_millis(3));
        let mut moved = observation(BlinkPhase::Open);
        moved.horizontal = 0.2;
        tracker.update(moved, Duration::from_millis(4));
        assert!(tracker.cursor(Duration::from_millis(4)).unwrap().0 < 0.5);
    }

    #[test]
    fn closed_eye_hides_its_cursor_and_the_combined_cursor() {
        let mut tracker = GridTracker::default();
        tracker.arm();
        for index in 0..3 {
            tracker.update(observation(BlinkPhase::Open), Duration::from_millis(index));
        }
        tracker.update(observation(BlinkPhase::Complete), Duration::from_millis(3));
        let mut right_closed = observation(BlinkPhase::Open);
        right_closed.right_eye_openness = 0.1;
        tracker.update(right_closed, Duration::from_millis(4));
        assert!(tracker.cursor(Duration::from_millis(4)).is_none());
        let (left, right) = tracker.eye_cursors(Duration::from_millis(4));
        assert!(left.is_some());
        assert!(right.is_none());
    }
}
