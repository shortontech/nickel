const LIST_LEFT: f32 = 40.0;
const LIST_RIGHT_INSET: f32 = 40.0;
const LIST_TOP: f32 = 132.0;
const ROW_HEIGHT: f32 = 48.0;
const ROW_GAP: f32 = 4.0;
const ROW_PADDING: f32 = 16.0;
const COLUMN_GAP: f32 = 16.0;
const ICON_SIZE: f32 = 36.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn right(self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    fn contains(self, x: f64, y: f64) -> bool {
        x >= f64::from(self.x)
            && x < f64::from(self.right())
            && y >= f64::from(self.y)
            && y < f64::from(self.bottom())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResultRow {
    pub outer: Rect,
    pub icon: Rect,
    pub label: Rect,
}

impl ResultRow {
    pub fn allocate(index: usize, available_width: u32) -> Self {
        let outer = Rect {
            x: LIST_LEFT,
            y: LIST_TOP + index as f32 * (ROW_HEIGHT + ROW_GAP),
            width: (available_width as f32 - LIST_LEFT - LIST_RIGHT_INSET).max(0.0),
            height: ROW_HEIGHT,
        };
        let icon = Rect {
            x: outer.x + ROW_PADDING,
            y: outer.y + (outer.height - ICON_SIZE) / 2.0,
            width: ICON_SIZE,
            height: ICON_SIZE,
        };
        let label_x = icon.right() + COLUMN_GAP;
        let label = Rect {
            x: label_x,
            y: outer.y,
            width: (outer.right() - ROW_PADDING - label_x).max(0.0),
            height: outer.height,
        };
        Self { outer, icon, label }
    }
}

pub fn hit_test_result(x: f64, y: f64, available_width: u32, count: usize) -> Option<usize> {
    (0..count).find(|index| {
        ResultRow::allocate(*index, available_width)
            .outer
            .contains(x, y)
    })
}

#[cfg(test)]
mod tests {
    use super::{ResultRow, hit_test_result};

    #[test]
    fn row_allocates_centered_icon_and_flexible_label_column() {
        let row = ResultRow::allocate(0, 960);
        assert_eq!(row.outer.y, 132.0);
        assert_eq!(row.icon.y, 138.0);
        assert_eq!(row.icon.x, 56.0);
        assert_eq!(row.label.x, 108.0);
        assert_eq!(row.label.height, row.outer.height);
        assert!(row.label.width > row.icon.width);
    }

    #[test]
    fn hit_testing_excludes_grid_gap_and_unallocated_rows() {
        assert_eq!(hit_test_result(100.0, 140.0, 960, 3), Some(0));
        assert_eq!(hit_test_result(100.0, 181.0, 960, 3), None);
        assert_eq!(hit_test_result(100.0, 300.0, 960, 3), None);
    }
}
