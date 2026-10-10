use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollExtent {
    pub viewport: Size,
    pub content: Size,
    pub offset_x: f32,
    pub offset: f32,
}

impl ScrollExtent {
    pub fn can_scroll(self) -> bool {
        self.content.width > self.viewport.width || self.content.height > self.viewport.height
    }
}

#[derive(Clone, Debug)]
pub(super) struct ScrollRegion<Message> {
    pub(super) id: UiId,
    pub(super) message: Option<Message>,
    pub(super) offset_mapper: Option<fn(f32) -> Message>,
    pub(super) extent_mapper: Option<fn(ScrollExtent) -> Message>,
    pub(super) rect: Rect,
    pub(super) clip: Rect,
    pub(super) extent: ScrollExtent,
    pub(super) scrollbar: crate::ScrollbarPalette,
    pub(super) parts: Option<Box<[DropdownPartStyle; 2]>>,
}

impl<Message> ScrollRegion<Message> {
    pub(super) fn mapped_message(&self, offset: f32) -> Option<Message> {
        self.extent_mapper
            .map(|map| {
                map(ScrollExtent {
                    offset,
                    ..self.extent
                })
            })
            .or_else(|| self.offset_mapper.map(|map| map(offset)))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScrollbarAxis {
    Horizontal,
    Vertical,
}

// Keep painted chrome discoverable while giving it a larger, edge-aligned
// acquisition target than its visual footprint.
pub(super) const SCROLLBAR_THICKNESS: f32 = 10.0;
pub(super) const SCROLLBAR_HIT_THICKNESS: f32 = 20.0;
pub(super) const SCROLLBAR_INSET: f32 = 3.0;
const SCROLLBAR_MIN_THUMB: f32 = 32.0;
pub(super) const SCROLLBAR_GUTTER: f32 = SCROLLBAR_THICKNESS + SCROLLBAR_INSET * 2.0;

pub(super) fn scrollbar_id(id: &UiId, axis: ScrollbarAxis) -> UiId {
    id.scoped(match axis {
        ScrollbarAxis::Horizontal => "$scrollbar-x",
        ScrollbarAxis::Vertical => "$scrollbar-y",
    })
}

pub(super) fn scrollbar_geometry<Message>(
    scroll: &ScrollRegion<Message>,
    axis: ScrollbarAxis,
) -> Option<(Rect, Rect)> {
    let thickness = scroll
        .parts
        .as_ref()
        .map_or(SCROLLBAR_THICKNESS, |parts| parts[0].width);
    let inset = scroll
        .parts
        .as_ref()
        .map_or(SCROLLBAR_INSET, |parts| parts[0].margin.right);
    let minimum = scroll
        .parts
        .as_ref()
        .map_or(SCROLLBAR_MIN_THUMB, |parts| parts[1].height);
    let (viewport, content, offset, track) = match axis {
        ScrollbarAxis::Horizontal => (
            scroll.extent.viewport.width,
            scroll.extent.content.width,
            scroll.extent.offset_x,
            Rect::new(
                scroll.rect.origin.x + inset,
                scroll.rect.origin.y + scroll.rect.size.height - thickness - inset,
                (scroll.rect.size.width - inset * 2.0).max(0.0),
                thickness,
            ),
        ),
        ScrollbarAxis::Vertical => (
            scroll.extent.viewport.height,
            scroll.extent.content.height,
            scroll.extent.offset,
            Rect::new(
                scroll.rect.origin.x + scroll.rect.size.width - thickness - inset,
                scroll.rect.origin.y + inset,
                thickness,
                (scroll.rect.size.height - inset * 2.0).max(0.0),
            ),
        ),
    };
    if content <= viewport || viewport <= 0.0 || thickness <= 0.0 {
        return None;
    }
    let track_length = match axis {
        ScrollbarAxis::Horizontal => track.size.width,
        ScrollbarAxis::Vertical => track.size.height,
    };
    let thumb_length =
        (track_length * viewport / content).clamp(minimum.min(track_length), track_length);
    let travel = (track_length - thumb_length).max(0.0);
    let maximum = content - viewport;
    let position = if maximum > 0.0 {
        travel * (offset / maximum).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let thumb = match axis {
        ScrollbarAxis::Horizontal => Rect::new(
            track.origin.x + position,
            track.origin.y,
            thumb_length,
            track.size.height,
        ),
        ScrollbarAxis::Vertical => Rect::new(
            track.origin.x,
            track.origin.y + position,
            track.size.width,
            thumb_length,
        ),
    };
    Some((track, thumb))
}

pub(super) fn scrollbar_hit_rect<Message>(
    scroll: &ScrollRegion<Message>,
    axis: ScrollbarAxis,
) -> Option<Rect> {
    let (track, _) = scrollbar_geometry(scroll, axis)?;
    let hit_thickness = SCROLLBAR_HIT_THICKNESS.max(match axis {
        ScrollbarAxis::Horizontal => {
            scroll.rect.origin.y + scroll.rect.size.height - track.origin.y
        }
        ScrollbarAxis::Vertical => scroll.rect.origin.x + scroll.rect.size.width - track.origin.x,
    });
    let hit = match axis {
        ScrollbarAxis::Horizontal => Rect::new(
            track.origin.x,
            scroll.rect.origin.y + scroll.rect.size.height - hit_thickness,
            track.size.width,
            hit_thickness,
        ),
        ScrollbarAxis::Vertical => Rect::new(
            scroll.rect.origin.x + scroll.rect.size.width - hit_thickness,
            track.origin.y,
            hit_thickness,
            track.size.height,
        ),
    };
    intersection(hit, scroll.rect).and_then(|hit| intersection(hit, scroll.clip))
}

pub(super) fn scrollbar_thumb_hit_rect<Message>(
    scroll: &ScrollRegion<Message>,
    axis: ScrollbarAxis,
) -> Option<Rect> {
    let (track, thumb) = scrollbar_geometry(scroll, axis)?;
    let hit_thickness = SCROLLBAR_HIT_THICKNESS.max(match axis {
        ScrollbarAxis::Horizontal => {
            scroll.rect.origin.y + scroll.rect.size.height - track.origin.y
        }
        ScrollbarAxis::Vertical => scroll.rect.origin.x + scroll.rect.size.width - track.origin.x,
    });
    let hit = match axis {
        ScrollbarAxis::Horizontal => Rect::new(
            thumb.origin.x,
            scroll.rect.origin.y + scroll.rect.size.height - hit_thickness,
            thumb.size.width,
            hit_thickness,
        ),
        ScrollbarAxis::Vertical => Rect::new(
            scroll.rect.origin.x + scroll.rect.size.width - hit_thickness,
            thumb.origin.y,
            hit_thickness,
            thumb.size.height,
        ),
    };
    intersection(hit, scroll.rect).and_then(|hit| intersection(hit, scroll.clip))
}

pub(super) fn configure_scroll_semantics(node: &mut ResolvedNode, extent: ScrollExtent) {
    let vertical_maximum = (extent.content.height - extent.viewport.height).max(0.0);
    let horizontal_maximum = (extent.content.width - extent.viewport.width).max(0.0);
    let (value, maximum) = if vertical_maximum > 0.0 {
        (extent.offset, vertical_maximum)
    } else {
        (extent.offset_x, horizontal_maximum)
    };
    if maximum <= 0.0 {
        return;
    }
    node.interaction.interactive = true;
    node.semantic_role.get_or_insert(SemanticRole::ScrollBar);
    node.accessibility_hidden = false;
    node.accessibility_label
        .get_or_insert_with(|| "Scrollable content".to_owned());
    let mut actions = vec![ActionKind::Scroll];
    if value < maximum {
        actions.push(ActionKind::Increment);
    }
    if value > 0.0 {
        actions.push(ActionKind::Decrement);
    }
    for action in actions {
        if !node.semantic_actions.contains(&action) {
            node.semantic_actions.push(action);
        }
    }
    node.semantic_value = Some(SemanticValueSnapshot::Number {
        value: f64::from(value),
        minimum: 0.0,
        maximum: f64::from(maximum),
        step: f64::from((maximum * 0.1).max(1.0)),
    });
}

/// CSS width is scrollbar thickness; right margin is the edge inset.
pub(super) fn scrollbar_gutter(style: &Style) -> f32 {
    style
        .scrollbar_parts
        .as_ref()
        .map_or(SCROLLBAR_GUTTER, |parts| {
            parts[0].width + parts[0].margin.right * 2.0
        })
}
