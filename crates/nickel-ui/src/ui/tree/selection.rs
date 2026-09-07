use super::*;
use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Clone, Debug)]
pub(super) struct SelectionGlyph {
    pub(super) rect: Rect,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) rtl: bool,
}

#[derive(Clone, Debug)]
pub(super) struct SelectionRunGeometry {
    pub(super) node_index: usize,
    pub(super) run_id: String,
    pub(super) glyphs: Vec<SelectionGlyph>,
}

#[derive(Clone, Debug)]
pub(super) struct SelectionRegionLayout {
    pub(super) id: UiId,
    pub(super) rect: Rect,
    pub(super) document: Arc<SelectionDocument>,
    pub(super) runs: Vec<SelectionRunGeometry>,
}

impl SelectionRegionLayout {
    pub(super) fn endpoint_at(&self, point: Point, nearest: bool) -> Option<SelectionEndpoint> {
        let mut best: Option<(f32, SelectionEndpoint)> = None;
        for run in &self.runs {
            for glyph in &run.glyphs {
                let inside = contains(glyph.rect, point);
                let on_line = point.y >= glyph.rect.origin.y
                    && point.y <= glyph.rect.origin.y + glyph.rect.size.height;
                if !inside && !nearest && !on_line {
                    continue;
                }
                let midpoint = glyph.rect.origin.x + glyph.rect.size.width * 0.5;
                let before = point.x < midpoint;
                let offset = match (glyph.rtl, before) {
                    (false, true) | (true, false) => glyph.start,
                    _ => glyph.end,
                };
                let dx = if point.x < glyph.rect.origin.x {
                    glyph.rect.origin.x - point.x
                } else if point.x > glyph.rect.origin.x + glyph.rect.size.width {
                    point.x - (glyph.rect.origin.x + glyph.rect.size.width)
                } else {
                    0.0
                };
                let dy = if point.y < glyph.rect.origin.y {
                    glyph.rect.origin.y - point.y
                } else if point.y > glyph.rect.origin.y + glyph.rect.size.height {
                    point.y - (glyph.rect.origin.y + glyph.rect.size.height)
                } else {
                    0.0
                };
                let distance = dx * dx + dy * dy;
                if inside || best.as_ref().is_none_or(|(current, _)| distance < *current) {
                    best = Some((distance, SelectionEndpoint::new(run.run_id.clone(), offset)));
                    if inside {
                        return best.map(|(_, endpoint)| endpoint);
                    }
                }
            }
        }
        best.map(|(_, endpoint)| endpoint)
    }
}

pub(super) struct SelectionRegionBuilder {
    pub(super) id: UiId,
    pub(super) rect: Rect,
    pub(super) supplied: Option<Arc<SelectionDocument>>,
    pub(super) runs: Vec<SelectionRunGeometry>,
    pub(super) logical_runs: Vec<SelectionRun>,
}

pub(super) fn selection_document_generation(document: &SelectionDocument) -> u64 {
    document.generation()
}

pub(super) fn document_selection_generation(selection: &crate::DocumentSelection) -> u64 {
    let mut hasher = DefaultHasher::new();
    for endpoint in [&selection.anchor, &selection.focus] {
        match endpoint {
            Some(endpoint) => {
                true.hash(&mut hasher);
                endpoint.run_id.hash(&mut hasher);
                endpoint.offset.hash(&mut hasher);
                (endpoint.affinity as u8).hash(&mut hasher);
            }
            None => false.hash(&mut hasher),
        }
    }
    hasher.finish()
}
