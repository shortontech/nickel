pub const SIDEBAR_WIDTH: f32 = 220.0;
pub const CONTENT_LEFT: f32 = 244.0;
pub const CONTENT_RIGHT_INSET: f32 = 28.0;
pub const LIST_TOP: f32 = 100.0;
pub const GRID_COLUMNS: usize = 5;
const ROW_GAP: f32 = 6.0;
const COLUMN_GAP: f32 = 6.0;
const LIST_BOTTOM_INSET: f32 = 80.0;
const TILE_PADDING: f32 = 10.0;
const ICON_SIZE: f32 = 44.0;
const SCROLLBAR_RIGHT_INSET: f32 = 18.0;
const SCROLLBAR_WIDTH: f32 = 6.0;
const MIN_THUMB_HEIGHT: f32 = 32.0;

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
        let cell_width = grid_cell_size(available_width);
        let column = index % GRID_COLUMNS;
        let grid_row = index / GRID_COLUMNS;
        let outer = Rect {
            x: CONTENT_LEFT + column as f32 * (cell_width + COLUMN_GAP),
            y: LIST_TOP + grid_row as f32 * (cell_width + ROW_GAP),
            width: cell_width,
            height: cell_width,
        };
        let icon = Rect {
            x: outer.x + (outer.width - ICON_SIZE) / 2.0,
            y: outer.y + TILE_PADDING,
            width: ICON_SIZE,
            height: ICON_SIZE,
        };
        let label = Rect {
            x: outer.x + TILE_PADDING,
            y: icon.bottom() + 6.0,
            width: (outer.width - TILE_PADDING * 2.0).max(0.0),
            height: (outer.bottom() - icon.bottom() - 12.0).max(0.0),
        };
        Self { outer, icon, label }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scrollbar {
    pub track: Rect,
    pub thumb: Rect,
}

pub fn visible_capacity(available_width: u32, available_height: u32) -> usize {
    let height = (available_height as f32 - LIST_TOP - LIST_BOTTOM_INSET).max(0.0);
    let cell_size = grid_cell_size(available_width);
    ((height + ROW_GAP) / (cell_size + ROW_GAP)).floor() as usize * GRID_COLUMNS
}

fn grid_cell_size(available_width: u32) -> f32 {
    let available = (available_width as f32 - CONTENT_LEFT - CONTENT_RIGHT_INSET).max(0.0);
    (available - COLUMN_GAP * GRID_COLUMNS.saturating_sub(1) as f32) / GRID_COLUMNS as f32
}

pub fn max_scroll_offset(total: usize, capacity: usize) -> usize {
    total.saturating_sub(capacity)
}

pub fn scrollbar(
    available_width: u32,
    available_height: u32,
    total: usize,
    capacity: usize,
    offset: usize,
) -> Option<Scrollbar> {
    if capacity == 0 || total <= capacity {
        return None;
    }
    let track = Rect {
        x: available_width as f32 - SCROLLBAR_RIGHT_INSET - SCROLLBAR_WIDTH,
        y: LIST_TOP,
        width: SCROLLBAR_WIDTH,
        height: (available_height as f32 - LIST_TOP - LIST_BOTTOM_INSET).max(0.0),
    };
    let thumb_height = (track.height * capacity as f32 / total as f32)
        .max(MIN_THUMB_HEIGHT)
        .min(track.height);
    let travel = track.height - thumb_height;
    let max_offset = max_scroll_offset(total, capacity);
    let thumb_y = track.y + travel * offset.min(max_offset) as f32 / max_offset as f32;
    Some(Scrollbar {
        track,
        thumb: Rect {
            x: track.x,
            y: thumb_y,
            width: track.width,
            height: thumb_height,
        },
    })
}

pub fn offset_from_thumb_y(
    thumb_y: f64,
    scrollbar: Scrollbar,
    total: usize,
    capacity: usize,
) -> usize {
    let travel = scrollbar.track.height - scrollbar.thumb.height;
    if travel <= 0.0 {
        return 0;
    }
    let fraction = ((thumb_y as f32 - scrollbar.track.y) / travel).clamp(0.0, 1.0);
    (fraction * max_scroll_offset(total, capacity) as f32).round() as usize
}

pub fn rect_contains(rect: Rect, x: f64, y: f64) -> bool {
    rect.contains(x, y)
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
        assert_eq!(row.outer.y, 100.0);
        assert_eq!(row.icon.y, 110.0);
        assert!(row.icon.x > row.outer.x);
        assert_eq!(row.label.x, row.outer.x + 10.0);
        assert_eq!(row.outer.width, row.outer.height);
        assert!(row.label.width > row.icon.width);
    }

    #[test]
    fn hit_testing_excludes_grid_gap_and_unallocated_rows() {
        assert_eq!(hit_test_result(260.0, 140.0, 960, 3), Some(0));
        assert_eq!(hit_test_result(379.0, 140.0, 960, 3), None);
        assert_eq!(hit_test_result(260.0, 300.0, 960, 3), None);
    }

    #[test]
    fn scrollbar_thumb_tracks_visible_fraction_and_offset() {
        let top = super::scrollbar(960, 640, 100, 20, 0).expect("overflow scrollbar");
        let bottom = super::scrollbar(960, 640, 100, 20, 80).expect("overflow scrollbar");
        assert_eq!(top.thumb.y, top.track.y);
        assert_eq!(bottom.thumb.bottom(), bottom.track.bottom());
        assert_eq!(
            super::offset_from_thumb_y(bottom.thumb.y.into(), bottom, 100, 20),
            80
        );
    }
}
