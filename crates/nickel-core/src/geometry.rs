//! Platform-neutral geometry primitives shared by shell policy and adapters.

/// A rectangle in a logical, signed coordinate space.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LogicalRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl LogicalRect {
    /// Returns the overlapping area without overflowing on hostile coordinates.
    pub fn intersection_area(self, other: Self) -> u64 {
        let left = i64::from(self.x.max(other.x));
        let top = i64::from(self.y.max(other.y));
        let self_right = i64::from(self.x) + i64::from(self.width);
        let other_right = i64::from(other.x) + i64::from(other.width);
        let self_bottom = i64::from(self.y) + i64::from(self.height);
        let other_bottom = i64::from(other.y) + i64::from(other.height);
        let width = self_right.min(other_right).saturating_sub(left).max(0);
        let height = self_bottom.min(other_bottom).saturating_sub(top).max(0);
        u64::try_from(width.saturating_mul(height)).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::LogicalRect;

    #[test]
    fn intersection_is_symmetric_and_clamps_disjoint_rectangles() {
        let left = LogicalRect {
            x: -100,
            y: 20,
            width: 150,
            height: 80,
        };
        let right = LogicalRect {
            x: 0,
            y: 0,
            width: 100,
            height: 60,
        };
        assert_eq!(left.intersection_area(right), 2_000);
        assert_eq!(right.intersection_area(left), 2_000);
        assert_eq!(left.intersection_area(LogicalRect { x: 500, ..right }), 0);
    }

    #[test]
    fn intersection_does_not_overflow_extreme_coordinates() {
        let enormous = LogicalRect {
            x: i32::MIN,
            y: i32::MIN,
            width: i32::MAX,
            height: i32::MAX,
        };
        assert_eq!(
            enormous.intersection_area(enormous),
            (i32::MAX as u64) * (i32::MAX as u64)
        );
    }
}
