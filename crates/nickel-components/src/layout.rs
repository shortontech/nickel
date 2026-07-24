#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            origin: Point { x, y },
            size: Size { width, height },
        }
    }

    pub fn inset(self, insets: Insets) -> Self {
        Self::new(
            self.origin.x + insets.left,
            self.origin.y + insets.top,
            (self.size.width - insets.left - insets.right).max(0.0),
            (self.size.height - insets.top - insets.bottom).max(0.0),
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Insets {
    pub const fn all(value: f32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Constraints {
    pub min: Size,
    pub max: Size,
}

impl Constraints {
    pub const fn new(min: Size, max: Size) -> Self {
        Self { min, max }
    }

    pub fn constrain(self, size: Size) -> Size {
        Size {
            width: size.width.clamp(self.min.width, self.max.width),
            height: size.height.clamp(self.min.height, self.max.height),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlexItem {
    pub preferred: f32,
    pub min: f32,
    pub max: f32,
    pub grow: f32,
}

impl FlexItem {
    pub const fn fixed(size: f32) -> Self {
        Self {
            preferred: size,
            min: size,
            max: size,
            grow: 0.0,
        }
    }

    pub const fn flexible(preferred: f32, min: f32, max: f32, grow: f32) -> Self {
        Self {
            preferred,
            min,
            max,
            grow,
        }
    }
}

pub fn layout_flex(bounds: Rect, axis: Axis, gap: f32, items: &[FlexItem]) -> Vec<Rect> {
    if items.is_empty() {
        return Vec::new();
    }
    let available = match axis {
        Axis::Horizontal => bounds.size.width,
        Axis::Vertical => bounds.size.height,
    };
    let gap_total = gap * items.len().saturating_sub(1) as f32;
    let mut sizes: Vec<_> = items
        .iter()
        .map(|item| item.preferred.clamp(item.min, item.max))
        .collect();
    let occupied = sizes.iter().sum::<f32>() + gap_total;
    let extra = (available - occupied).max(0.0);
    let total_grow = items.iter().map(|item| item.grow.max(0.0)).sum::<f32>();
    if extra > 0.0 && total_grow > 0.0 {
        for (size, item) in sizes.iter_mut().zip(items) {
            *size = (*size + extra * item.grow.max(0.0) / total_grow).min(item.max);
        }
    }

    let mut cursor = match axis {
        Axis::Horizontal => bounds.origin.x,
        Axis::Vertical => bounds.origin.y,
    };
    sizes
        .into_iter()
        .map(|main| {
            let rect = match axis {
                Axis::Horizontal => Rect::new(cursor, bounds.origin.y, main, bounds.size.height),
                Axis::Vertical => Rect::new(bounds.origin.x, cursor, bounds.size.width, main),
            };
            cursor += main + gap;
            rect
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Axis, FlexItem, Insets, Rect, layout_flex};

    #[test]
    fn row_distributes_remaining_space_to_flexible_children() {
        let layout = layout_flex(
            Rect::new(0.0, 0.0, 300.0, 40.0),
            Axis::Horizontal,
            10.0,
            &[
                FlexItem::fixed(50.0),
                FlexItem::flexible(50.0, 20.0, 500.0, 1.0),
                FlexItem::flexible(50.0, 20.0, 500.0, 1.0),
            ],
        );
        assert_eq!(layout[0], Rect::new(0.0, 0.0, 50.0, 40.0));
        assert_eq!(layout[1], Rect::new(60.0, 0.0, 115.0, 40.0));
        assert_eq!(layout[2], Rect::new(185.0, 0.0, 115.0, 40.0));
    }

    #[test]
    fn insets_never_produce_negative_content_size() {
        assert_eq!(
            Rect::new(0.0, 0.0, 10.0, 10.0).inset(Insets::all(8.0)),
            Rect::new(8.0, 8.0, 0.0, 0.0)
        );
    }
}
