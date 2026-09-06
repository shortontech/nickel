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
    pub(super) rect: Rect,
    pub(super) clip: Rect,
    pub(super) extent: ScrollExtent,
    pub(super) scrollbar: crate::ScrollbarPalette,
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
    let (viewport, content, offset, track) = match axis {
        ScrollbarAxis::Horizontal => (
            scroll.extent.viewport.width,
            scroll.extent.content.width,
            scroll.extent.offset_x,
            Rect::new(
                scroll.rect.origin.x + SCROLLBAR_INSET,
                scroll.rect.origin.y + scroll.rect.size.height
                    - SCROLLBAR_THICKNESS
                    - SCROLLBAR_INSET,
                (scroll.rect.size.width - SCROLLBAR_INSET * 2.0).max(0.0),
                SCROLLBAR_THICKNESS,
            ),
        ),
        ScrollbarAxis::Vertical => (
            scroll.extent.viewport.height,
            scroll.extent.content.height,
            scroll.extent.offset,
            Rect::new(
                scroll.rect.origin.x + scroll.rect.size.width
                    - SCROLLBAR_THICKNESS
                    - SCROLLBAR_INSET,
                scroll.rect.origin.y + SCROLLBAR_INSET,
                SCROLLBAR_THICKNESS,
                (scroll.rect.size.height - SCROLLBAR_INSET * 2.0).max(0.0),
            ),
        ),
    };
    if content <= viewport || viewport <= 0.0 {
        return None;
    }
    let track_length = match axis {
        ScrollbarAxis::Horizontal => track.size.width,
        ScrollbarAxis::Vertical => track.size.height,
    };
    let thumb_length = (track_length * viewport / content)
        .clamp(SCROLLBAR_MIN_THUMB.min(track_length), track_length);
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
    let hit = match axis {
        ScrollbarAxis::Horizontal => Rect::new(
            track.origin.x,
            scroll.rect.origin.y + scroll.rect.size.height - SCROLLBAR_HIT_THICKNESS,
            track.size.width,
            SCROLLBAR_HIT_THICKNESS,
        ),
        ScrollbarAxis::Vertical => Rect::new(
            scroll.rect.origin.x + scroll.rect.size.width - SCROLLBAR_HIT_THICKNESS,
            track.origin.y,
            SCROLLBAR_HIT_THICKNESS,
            track.size.height,
        ),
    };
    intersection(hit, scroll.rect).and_then(|hit| intersection(hit, scroll.clip))
}

pub(super) fn scrollbar_thumb_hit_rect<Message>(
    scroll: &ScrollRegion<Message>,
    axis: ScrollbarAxis,
) -> Option<Rect> {
    let (_, thumb) = scrollbar_geometry(scroll, axis)?;
    let hit = match axis {
        ScrollbarAxis::Horizontal => Rect::new(
            thumb.origin.x,
            scroll.rect.origin.y + scroll.rect.size.height - SCROLLBAR_HIT_THICKNESS,
            thumb.size.width,
            SCROLLBAR_HIT_THICKNESS,
        ),
        ScrollbarAxis::Vertical => Rect::new(
            scroll.rect.origin.x + scroll.rect.size.width - SCROLLBAR_HIT_THICKNESS,
            thumb.origin.y,
            SCROLLBAR_HIT_THICKNESS,
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
