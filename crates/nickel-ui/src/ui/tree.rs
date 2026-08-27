use super::*;

#[derive(Clone, Debug)]
struct HitRegion<Message> {
    id: UiId,
    rect: Rect,
    message: Option<Message>,
    message_mapper: Option<fn(f32) -> Message>,
}

#[derive(Clone, Debug)]
struct MessageRegion<Message> {
    id: UiId,
    rect: Rect,
    message: Message,
}

#[derive(Clone, Debug)]
struct TextInputRegion<Message> {
    id: UiId,
    rect: Rect,
    scale: f32,
    bold: bool,
    line_height: f32,
    initial: String,
    map: fn(String) -> Message,
}

#[derive(Clone, Debug)]
struct SelectionGlyph {
    rect: Rect,
    start: usize,
    end: usize,
    rtl: bool,
}

#[derive(Clone, Debug)]
struct SelectionRunGeometry {
    node_index: usize,
    run_id: String,
    glyphs: Vec<SelectionGlyph>,
}

#[derive(Clone, Debug)]
struct SelectionRegionLayout {
    id: UiId,
    rect: Rect,
    document: Arc<SelectionDocument>,
    runs: Vec<SelectionRunGeometry>,
}

impl SelectionRegionLayout {
    fn endpoint_at(&self, point: Point, nearest: bool) -> Option<SelectionEndpoint> {
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

struct SelectionRegionBuilder {
    id: UiId,
    rect: Rect,
    supplied: Option<Arc<SelectionDocument>>,
    runs: Vec<SelectionRunGeometry>,
    logical_runs: Vec<SelectionRun>,
}

fn text_offset_at<Message>(input: &TextInputRegion<Message>, point: Point) -> usize {
    let target_line = ((point.y - input.rect.origin.y) / input.line_height)
        .floor()
        .max(0.0) as usize;
    let mut line_start = 0;
    let mut line = input.initial.as_str();
    for (index, part) in input.initial.split_inclusive('\n').enumerate() {
        if index == target_line {
            line = part.strip_suffix('\n').unwrap_or(part);
            break;
        }
        line_start += part.len();
    }
    if target_line >= input.initial.lines().count().max(1) {
        return input.initial.len();
    }
    if point.x <= input.rect.origin.x {
        return line_start;
    }
    let target = (point.x - input.rect.origin.x).max(0.0);
    let mut previous = (0, 0.0);
    for (index, _) in line.grapheme_indices(true) {
        let width = measure_text(
            &line[..index],
            input.scale,
            input.bold,
            false,
            None,
            Some(1),
            f32::INFINITY,
        )
        .width;
        if target < (previous.1 + width) * 0.5 {
            return line_start + previous.0;
        }
        previous = (index, width);
    }
    line_start + line.len()
}

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
struct ScrollRegion<Message> {
    id: UiId,
    message: Option<Message>,
    offset_mapper: Option<fn(f32) -> Message>,
    rect: Rect,
    clip: Rect,
    extent: ScrollExtent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScrollbarAxis {
    Horizontal,
    Vertical,
}

const SCROLLBAR_THICKNESS: f32 = 8.0;
const SCROLLBAR_INSET: f32 = 3.0;
const SCROLLBAR_MIN_THUMB: f32 = 24.0;
const SCROLLBAR_GUTTER: f32 = SCROLLBAR_THICKNESS + SCROLLBAR_INSET * 2.0;

fn scrollbar_id(id: &UiId, axis: ScrollbarAxis) -> UiId {
    id.scoped(match axis {
        ScrollbarAxis::Horizontal => "$scrollbar-x",
        ScrollbarAxis::Vertical => "$scrollbar-y",
    })
}

fn scrollbar_geometry<Message>(
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedGrid {
    pub rect: Rect,
    pub columns: usize,
}

/// Backend-neutral geometry exposed to accessibility adapters.
#[derive(Clone, Debug, PartialEq)]
pub struct AccessibilityNode {
    pub id: UiId,
    pub component: &'static str,
    pub rect: Rect,
    pub interactive: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DiagnosticKind {
    ContradictoryConstraints,
    InvalidGeometry,
    FlexOverflow,
    IndefinitePercentage,
    DuplicateIdentity,
    ClippedInteraction,
    ScrollOffsetClamped,
    UnsatisfiedContent,
    MissingAsset,
    UnbalancedClip,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutDiagnostic {
    pub kind: DiagnosticKind,
    pub id: UiId,
    pub detail: String,
    pub source: Option<SourceLocation>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InteractionState {
    pub interactive: bool,
    pub focused: bool,
    pub hovered: bool,
    pub pressed: bool,
    pub captured: bool,
    pub controller_selected: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedNode {
    pub component: &'static str,
    pub id: UiId,
    pub source: Option<SourceLocation>,
    pub allocated: Rect,
    pub padding_box: Rect,
    pub border_box: Rect,
    pub content: Rect,
    pub constraints: Constraints,
    pub preferred: Size,
    pub flex_basis: Length,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub clip: Option<Rect>,
    pub scroll: Option<ScrollExtent>,
    pub grid_tracks: Vec<f32>,
    pub hit_stack: Option<usize>,
    pub interaction: InteractionState,
    pub children: Vec<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedLayout {
    nodes: Vec<ResolvedNode>,
}

impl ResolvedLayout {
    pub fn nodes(&self) -> &[ResolvedNode] {
        &self.nodes
    }

    pub fn find(&self, id: &UiId) -> Option<&ResolvedNode> {
        self.nodes.iter().find(|node| &node.id == id)
    }

    pub fn deterministic_snapshot(&self) -> String {
        let mut output = String::new();
        for node in &self.nodes {
            let _ = writeln!(
                output,
                "{} {} source={} allocated={:.2},{:.2},{:.2},{:.2} content={:.2},{:.2},{:.2},{:.2} min={:.2},{:.2} preferred={:.2},{:.2} max={:.2},{:.2} flex={:?},{:.2},{:.2} grid={:?} scroll={:?} hit={:?} clip={} children={:?}",
                node.component,
                node.id.as_str(),
                node.source.map_or_else(
                    || "none".into(),
                    |source| format!("{}:{}:{}", source.file, source.line, source.column)
                ),
                node.allocated.origin.x,
                node.allocated.origin.y,
                node.allocated.size.width,
                node.allocated.size.height,
                node.content.origin.x,
                node.content.origin.y,
                node.content.size.width,
                node.content.size.height,
                node.constraints.min.width,
                node.constraints.min.height,
                node.preferred.width,
                node.preferred.height,
                node.constraints.max.width,
                node.constraints.max.height,
                node.flex_basis,
                node.flex_grow,
                node.flex_shrink,
                node.grid_tracks,
                node.scroll,
                node.hit_stack,
                node.clip.map_or_else(
                    || "none".into(),
                    |clip| format!(
                        "{:.2},{:.2},{:.2},{:.2}",
                        clip.origin.x, clip.origin.y, clip.size.width, clip.size.height
                    )
                ),
                node.children,
            );
        }
        output
    }
}

#[derive(Clone, Debug)]
pub struct UiTree<Message = String> {
    commands: Vec<PaintCommand>,
    overlay_commands: Vec<PaintCommand>,
    hits: Vec<HitRegion<Message>>,
    overlay_hits: Vec<HitRegion<Message>>,
    messages: Vec<MessageRegion<Message>>,
    text_inputs: Vec<TextInputRegion<Message>>,
    selection_regions: Vec<SelectionRegionLayout>,
    selection_paints: HashMap<usize, Vec<Rect>>,
    scrolls: Vec<ScrollRegion<Message>>,
    grids: Vec<ResolvedGrid>,
    accessibility: Vec<AccessibilityNode>,
    resolved: ResolvedLayout,
    diagnostics: Vec<LayoutDiagnostic>,
    diagnostic_keys: HashSet<(DiagnosticKind, UiId)>,
    seen_ids: HashSet<UiId>,
    diagnostics_enabled: bool,
}

impl<Message> Default for UiTree<Message> {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
            overlay_commands: Vec::new(),
            hits: Vec::new(),
            overlay_hits: Vec::new(),
            messages: Vec::new(),
            text_inputs: Vec::new(),
            selection_regions: Vec::new(),
            selection_paints: HashMap::new(),
            scrolls: Vec::new(),
            grids: Vec::new(),
            accessibility: Vec::new(),
            resolved: ResolvedLayout::default(),
            diagnostics: Vec::new(),
            diagnostic_keys: HashSet::new(),
            seen_ids: HashSet::new(),
            diagnostics_enabled: false,
        }
    }
}

impl<Message: Clone> UiTree<Message> {
    fn selection_hit_at(
        &self,
        point: Point,
    ) -> Option<(&SelectionRegionLayout, SelectionEndpoint)> {
        self.selection_regions
            .iter()
            .rev()
            .filter(|region| contains(region.rect, point))
            .find_map(|region| {
                region
                    .endpoint_at(point, false)
                    .map(|endpoint| (region, endpoint))
            })
    }

    fn selection_region(&self, id: &UiId) -> Option<&SelectionRegionLayout> {
        self.selection_regions
            .iter()
            .find(|region| &region.id == id)
    }

    pub fn selected_text(&self, state: &UiStateStore) -> Option<String> {
        if let Some(text) = self
            .focused_editor(state)
            .and_then(TextEditor::selected_text)
            .map(ToOwned::to_owned)
        {
            return Some(text);
        }
        let owner = state.selection_owner()?;
        let region = self
            .selection_regions
            .iter()
            .find(|region| &region.id == owner)?;
        region
            .document
            .selected_text(state.document_selection(owner)?)
    }

    pub fn selection_region_ids(&self) -> impl Iterator<Item = &UiId> {
        self.selection_regions.iter().map(|region| &region.id)
    }

    fn prepare_selection_paints(&mut self, state: &UiStateStore) {
        self.selection_paints.clear();
        for region in &self.selection_regions {
            let Some(selection) = state.document_selection(&region.id) else {
                continue;
            };
            for run in &region.runs {
                let Some(range) = region.document.selected_range_in(selection, &run.run_id) else {
                    continue;
                };
                for glyph in &run.glyphs {
                    if glyph.end > range.start && glyph.start < range.end {
                        self.selection_paints
                            .entry(run.node_index)
                            .or_default()
                            .push(glyph.rect);
                    }
                }
            }
        }
    }

    pub fn layout(root: impl Component<Message>, bounds: Rect) -> Self {
        Self::layout_internal(root, bounds, false)
    }

    pub fn layout_with_diagnostics(root: impl Component<Message>, bounds: Rect) -> Self {
        Self::layout_internal(root, bounds, true)
    }

    pub fn layout_with_state(
        root: impl Component<Message>,
        bounds: Rect,
        state: &mut UiStateStore,
    ) -> Self {
        Self::layout_with_state_and_diagnostics(root, bounds, state, false)
    }

    pub fn layout_with_state_and_diagnostics(
        root: impl Component<Message>,
        bounds: Rect,
        state: &mut UiStateStore,
        diagnostics: bool,
    ) -> Self {
        let mut root = root.into_element();
        let root_id = root.id.as_ref().map_or_else(
            || UiId::from("root"),
            |id| UiId::from("root").scoped(id.as_str()),
        );
        state.begin_frame();
        apply_transient_state(&mut root, &root_id, state);
        let mut tree = Self {
            diagnostics_enabled: diagnostics,
            ..Self::default()
        };
        layout_element(&root, &root_id, bounds, None, None, &mut tree);
        tree.selection_regions = collect_selection_regions(&root, &tree.resolved);
        for region in &tree.selection_regions {
            state.touch(region.id.clone());
            state.reconcile_document_selection(region.id.clone(), region.document.clone());
        }
        tree.prepare_selection_paints(state);
        tree.reset_emission();
        emit_element(&root, 0, None, &mut tree);
        tree.append_scrollbars();
        tree.commands.append(&mut tree.overlay_commands);
        tree.hits.append(&mut tree.overlay_hits);
        for hit in &tree.hits {
            state.touch(hit.id.clone());
        }
        tree.emit_accessibility_geometry();
        tree.validate_clip_commands();
        for scroll in &tree.scrolls {
            let transient = state.touch(scroll.id.clone());
            transient.scroll_offset_x = scroll.extent.offset_x;
            transient.scroll_offset = scroll.extent.offset;
            for axis in [ScrollbarAxis::Horizontal, ScrollbarAxis::Vertical] {
                if scrollbar_geometry(scroll, axis).is_some() {
                    state.touch(scrollbar_id(&scroll.id, axis));
                }
            }
        }
        state.end_frame();
        tree.apply_interaction_state(state);
        tree
    }

    fn layout_internal(root: impl Component<Message>, bounds: Rect, diagnostics: bool) -> Self {
        let mut tree = Self {
            diagnostics_enabled: diagnostics,
            ..Self::default()
        };
        let root = root.into_element();
        let root_id = root.id.as_ref().map_or_else(
            || UiId::from("root"),
            |id| UiId::from("root").scoped(id.as_str()),
        );
        layout_element(&root, &root_id, bounds, None, None, &mut tree);
        tree.selection_regions = collect_selection_regions(&root, &tree.resolved);
        tree.reset_emission();
        emit_element(&root, 0, None, &mut tree);
        tree.append_scrollbars();
        tree.commands.append(&mut tree.overlay_commands);
        tree.hits.append(&mut tree.overlay_hits);
        tree.emit_accessibility_geometry();
        tree.validate_clip_commands();
        tree
    }

    pub fn resolved_layout(&self) -> &ResolvedLayout {
        &self.resolved
    }

    pub fn diagnostics(&self) -> &[LayoutDiagnostic] {
        &self.diagnostics
    }

    pub fn commands(&self) -> &[PaintCommand] {
        &self.commands
    }

    pub fn accessibility_nodes(&self) -> &[AccessibilityNode] {
        &self.accessibility
    }

    pub fn message_at(&self, point: Point) -> Option<&Message> {
        self.hits
            .iter()
            .rev()
            .find(|hit| contains(hit.rect, point))
            .and_then(|hit| hit.message.as_ref())
    }

    pub fn id_at(&self, point: Point) -> Option<&UiId> {
        self.hits
            .iter()
            .rev()
            .find(|hit| contains(hit.rect, point))
            .map(|hit| &hit.id)
    }

    pub fn pointer_icon_at(&self, point: Point) -> PointerIcon {
        if self.scrollbar_at(point).is_some() || self.id_at(point).is_some() {
            PointerIcon::Hand
        } else if self.selection_hit_at(point).is_some() {
            PointerIcon::Text
        } else {
            PointerIcon::Default
        }
    }

    pub fn message_for_id(&self, id: &UiId) -> Option<&Message> {
        self.messages
            .iter()
            .rev()
            .find(|region| &region.id == id)
            .map(|region| &region.message)
    }

    pub fn id_for_message(&self, message: &Message) -> Option<&UiId>
    where
        Message: PartialEq,
    {
        self.messages
            .iter()
            .rev()
            .find(|region| &region.message == message)
            .map(|region| &region.id)
    }

    pub fn message_at_with_horizontal_fraction(&self, point: Point) -> Option<(&Message, f32)> {
        self.hits
            .iter()
            .rev()
            .find(|hit| contains(hit.rect, point))
            .and_then(|hit| {
                let fraction =
                    ((point.x - hit.rect.origin.x) / hit.rect.size.width.max(1.0)).clamp(0.0, 1.0);
                Some((hit.message.as_ref()?, fraction))
            })
    }

    pub fn message_at_owned(&self, point: Point) -> Option<Message> {
        self.hits
            .iter()
            .rev()
            .find(|hit| contains(hit.rect, point))
            .and_then(|hit| {
                let fraction =
                    ((point.x - hit.rect.origin.x) / hit.rect.size.width.max(1.0)).clamp(0.0, 1.0);
                hit.message_mapper
                    .map(|map| map(fraction))
                    .or_else(|| hit.message.clone())
            })
    }

    pub fn horizontal_fraction_for_message(&self, message: &Message, x: f32) -> Option<f32>
    where
        Message: PartialEq,
    {
        self.hits
            .iter()
            .rev()
            .find(|hit| hit.message.as_ref() == Some(message))
            .map(|hit| ((x - hit.rect.origin.x) / hit.rect.size.width.max(1.0)).clamp(0.0, 1.0))
    }

    pub fn horizontal_fraction_for_id(&self, id: &UiId, x: f32) -> Option<f32> {
        self.hits
            .iter()
            .rev()
            .find(|hit| &hit.id == id)
            .map(|hit| ((x - hit.rect.origin.x) / hit.rect.size.width.max(1.0)).clamp(0.0, 1.0))
    }

    pub fn horizontal_fraction_for_matching(
        &self,
        x: f32,
        predicate: impl Fn(&Message) -> bool,
    ) -> Option<f32> {
        self.hits
            .iter()
            .rev()
            .find(|hit| hit.message.as_ref().is_some_and(&predicate))
            .map(|hit| ((x - hit.rect.origin.x) / hit.rect.size.width.max(1.0)).clamp(0.0, 1.0))
    }

    pub fn messages_intersecting(&self, rect: Rect) -> Vec<&Message> {
        self.hits
            .iter()
            .filter(|hit| rects_intersect(hit.rect, rect))
            .filter_map(|hit| hit.message.as_ref())
            .collect()
    }

    pub fn scroll_extent(&self, message: &Message) -> Option<ScrollExtent>
    where
        Message: PartialEq,
    {
        self.scrolls
            .iter()
            .find(|scroll| scroll.message.as_ref() == Some(message))
            .map(|scroll| scroll.extent)
    }

    pub fn scroll_viewport(&self, message: &Message) -> Option<Rect>
    where
        Message: PartialEq,
    {
        self.scrolls
            .iter()
            .find(|scroll| scroll.message.as_ref() == Some(message))
            .map(|scroll| scroll.rect)
    }

    pub fn message_rect(&self, message: &Message) -> Option<Rect>
    where
        Message: PartialEq,
    {
        self.hits
            .iter()
            .rev()
            .find(|hit| hit.message.as_ref() == Some(message))
            .map(|hit| hit.rect)
    }

    pub fn message_layout_rect(&self, message: &Message) -> Option<Rect>
    where
        Message: PartialEq,
    {
        self.messages
            .iter()
            .rev()
            .find(|region| &region.message == message)
            .map(|region| region.rect)
    }

    pub fn resolved_grid_columns(&self) -> Option<usize> {
        self.grids.last().map(|grid| grid.columns)
    }

    pub fn resolved_grids(&self) -> &[ResolvedGrid] {
        &self.grids
    }

    pub fn reconcile_state(&self, state: &mut UiStateStore) {
        self.reconcile_state_with(state, std::iter::empty::<UiId>());
    }

    pub fn reconcile_state_with(
        &self,
        state: &mut UiStateStore,
        retained: impl IntoIterator<Item = UiId>,
    ) {
        state.begin_frame();
        for region in &self.messages {
            state.touch(region.id.clone());
        }
        for scroll in &self.scrolls {
            state.touch(scroll.id.clone());
        }
        for id in retained {
            state.touch(id);
        }
        state.end_frame();
    }

    fn append_scrollbars(&mut self) {
        for scroll in &self.scrolls {
            for axis in [ScrollbarAxis::Horizontal, ScrollbarAxis::Vertical] {
                let Some((track, thumb)) = scrollbar_geometry(scroll, axis) else {
                    continue;
                };
                if intersection(track, scroll.clip).is_none() {
                    continue;
                }
                self.commands.push(PaintCommand::PushClip(scroll.clip));
                self.commands.push(PaintCommand::RoundedFill {
                    rect: track,
                    color: 0x40343b48,
                    radius: SCROLLBAR_THICKNESS / 2.0,
                });
                self.commands.push(PaintCommand::RoundedFill {
                    rect: thumb,
                    color: 0xc08a96a8,
                    radius: SCROLLBAR_THICKNESS / 2.0,
                });
                self.commands.push(PaintCommand::PopClip);
            }
        }
    }

    fn scrollbar_at(&self, point: Point) -> Option<(&ScrollRegion<Message>, ScrollbarAxis)> {
        self.scrolls.iter().rev().find_map(|scroll| {
            [ScrollbarAxis::Vertical, ScrollbarAxis::Horizontal]
                .into_iter()
                .find(|axis| {
                    scrollbar_geometry(scroll, *axis).is_some_and(|(track, _)| {
                        contains(track, point) && contains(scroll.clip, point)
                    })
                })
                .map(|axis| (scroll, axis))
        })
    }

    fn captured_scrollbar(
        &self,
        captured: &UiId,
    ) -> Option<(&ScrollRegion<Message>, ScrollbarAxis)> {
        self.scrolls.iter().rev().find_map(|scroll| {
            [ScrollbarAxis::Vertical, ScrollbarAxis::Horizontal]
                .into_iter()
                .find(|axis| scrollbar_id(&scroll.id, *axis) == *captured)
                .map(|axis| (scroll, axis))
        })
    }

    fn move_scrollbar(
        scroll: &ScrollRegion<Message>,
        axis: ScrollbarAxis,
        point: Point,
        state: &mut UiStateStore,
        messages: &mut Vec<Message>,
    ) -> Invalidation {
        let Some((track, thumb)) = scrollbar_geometry(scroll, axis) else {
            return Invalidation::None;
        };
        let (pointer, track_start, track_length, thumb_length, maximum, fallback_current) =
            match axis {
                ScrollbarAxis::Horizontal => (
                    point.x,
                    track.origin.x,
                    track.size.width,
                    thumb.size.width,
                    (scroll.extent.content.width - scroll.extent.viewport.width).max(0.0),
                    scroll.extent.offset_x,
                ),
                ScrollbarAxis::Vertical => (
                    point.y,
                    track.origin.y,
                    track.size.height,
                    thumb.size.height,
                    (scroll.extent.content.height - scroll.extent.viewport.height).max(0.0),
                    scroll.extent.offset,
                ),
            };
        let travel = (track_length - thumb_length).max(0.0);
        let target = if travel > 0.0 {
            ((pointer - track_start - thumb_length / 2.0) / travel).clamp(0.0, 1.0) * maximum
        } else {
            0.0
        };
        let current = state
            .state(&scroll.id)
            .map_or(fallback_current, |entry| match axis {
                ScrollbarAxis::Horizontal => entry.scroll_offset_x,
                ScrollbarAxis::Vertical => entry.scroll_offset,
            });
        match axis {
            ScrollbarAxis::Horizontal => {
                state.scroll_by_x(scroll.id.clone(), target - current, maximum)
            }
            ScrollbarAxis::Vertical => {
                let invalidation = state.scroll_by(scroll.id.clone(), target - current, maximum);
                if invalidation != Invalidation::None
                    && let Some(map) = scroll.offset_mapper
                    && let Some(offset) = state.state(&scroll.id).map(|entry| entry.scroll_offset)
                {
                    messages.push(map(offset));
                }
                invalidation
            }
        }
    }

    pub fn handle_event(&self, state: &mut UiStateStore, event: UiEvent) -> EventOutcome<Message> {
        let mut outcome = EventOutcome::default();
        outcome.invalidation = match event {
            UiEvent::PointerMoved(point) => {
                if let Some(captured) = state.captured().cloned()
                    && let Some((scroll, axis)) = self.captured_scrollbar(&captured)
                {
                    return EventOutcome {
                        invalidation: Self::move_scrollbar(
                            scroll,
                            axis,
                            point,
                            state,
                            &mut outcome.messages,
                        ),
                        ..outcome
                    };
                }
                let mut invalidation = state.set_hovered(self.id_at(point).cloned());
                if let Some(region_id) = state.captured().cloned()
                    && let Some(region) = self.selection_region(&region_id)
                    && let Some(endpoint) = region.endpoint_at(point, true)
                {
                    state.document_selection_mut(region_id).focus = Some(endpoint);
                    let autoscroll = self
                        .scrolls
                        .iter()
                        .rev()
                        .find(|scroll| {
                            point.x >= scroll.rect.origin.x
                                && point.x <= scroll.rect.origin.x + scroll.rect.size.width
                                && rectangles_overlap(scroll.rect, region.rect)
                        })
                        .and_then(|scroll| {
                            let edge = 28.0_f32.min(scroll.rect.size.height * 0.25);
                            let top = scroll.rect.origin.y + edge;
                            let bottom = scroll.rect.origin.y + scroll.rect.size.height - edge;
                            let delta = if point.y < top {
                                -((top - point.y) / edge).clamp(0.25, 1.0) * 18.0
                            } else if point.y > bottom {
                                ((point.y - bottom) / edge).clamp(0.25, 1.0) * 18.0
                            } else {
                                return None;
                            };
                            let changed = state.scroll_by(
                                scroll.id.clone(),
                                delta,
                                (scroll.extent.content.height - scroll.extent.viewport.height)
                                    .max(0.0),
                            );
                            if changed != Invalidation::None
                                && let Some(map) = scroll.offset_mapper
                                && let Some(offset) =
                                    state.state(&scroll.id).map(|entry| entry.scroll_offset)
                            {
                                outcome.messages.push(map(offset));
                            }
                            Some(changed)
                        })
                        .unwrap_or(Invalidation::None);
                    return EventOutcome {
                        messages: outcome.messages,
                        clipboard_text: outcome.clipboard_text,
                        invalidation: invalidation.merge(autoscroll).merge(Invalidation::Paint),
                    };
                }
                if let Some(id) = state.captured().cloned()
                    && let Some(input) = self.text_inputs.iter().find(|input| input.id == id)
                {
                    let cursor = text_offset_at(input, point);
                    state.editor(id, &input.initial).extend_selection_to(cursor);
                    invalidation = invalidation
                        .merge(state.show_caret())
                        .merge(Invalidation::Paint);
                }
                invalidation
            }
            UiEvent::PointerPressed(point) => {
                if let Some((scroll, axis)) = self.scrollbar_at(point) {
                    let invalidation = state
                        .set_focus(None)
                        .merge(state.set_pressed(Some(scrollbar_id(&scroll.id, axis))))
                        .merge(state.set_capture(Some(scrollbar_id(&scroll.id, axis))))
                        .merge(Self::move_scrollbar(
                            scroll,
                            axis,
                            point,
                            state,
                            &mut outcome.messages,
                        ));
                    return EventOutcome {
                        invalidation,
                        ..outcome
                    };
                }
                let clicked = self.id_at(point).map(UiId::as_str);
                let mut dismissed = Invalidation::None;
                for node in self
                    .resolved_layout()
                    .nodes()
                    .iter()
                    .filter(|node| node.component == "Dropdown")
                {
                    if state
                        .state(&node.id)
                        .is_some_and(|entry| entry.dropdown_open)
                        && !clicked.is_some_and(|id| id.starts_with(node.id.as_str()))
                    {
                        dismissed =
                            dismissed.merge(state.set_dropdown_open(node.id.clone(), false));
                    }
                }
                if clicked.is_none()
                    && let Some((region, endpoint)) = self.selection_hit_at(point)
                {
                    let region_id = region.id.clone();
                    let selection = state.document_selection_mut(region_id.clone());
                    selection.anchor = Some(endpoint.clone());
                    selection.focus = Some(endpoint);
                    return EventOutcome {
                        messages: outcome.messages,
                        clipboard_text: outcome.clipboard_text,
                        invalidation: state
                            .set_focus(None)
                            .merge(state.set_selection_owner(Some(region_id.clone())))
                            .merge(state.set_pressed(Some(region_id.clone())))
                            .merge(state.set_capture(Some(region_id)))
                            .merge(Invalidation::Paint),
                    };
                }
                let id = self.id_at(point).cloned();
                let mut invalidation = state
                    .clear_document_selection()
                    .merge(state.set_focus(id.clone()))
                    .merge(state.set_pressed(id.clone()))
                    .merge(state.set_capture(id.clone()));
                if let Some(id) = id
                    && let Some(input) = self.text_inputs.iter().find(|input| input.id == id)
                {
                    let cursor = text_offset_at(input, point);
                    state.editor(id, &input.initial).place_cursor(cursor);
                    invalidation = invalidation.merge(state.show_caret());
                }
                invalidation.merge(dismissed)
            }
            UiEvent::PointerReleased(point) => {
                if state
                    .captured()
                    .is_some_and(|captured| self.captured_scrollbar(captured).is_some())
                {
                    return EventOutcome {
                        invalidation: state
                            .set_pressed(None)
                            .merge(state.set_capture(None))
                            .merge(Invalidation::Paint),
                        ..outcome
                    };
                }
                let released = self.id_at(point);
                let activates = state
                    .captured()
                    .is_some_and(|captured| released == Some(captured));
                if activates && let Some(message) = self.message_at_owned(point) {
                    outcome.messages.push(message);
                }
                let dropdown = state.captured().and_then(|id| {
                    self.resolved_layout()
                        .find(id)
                        .filter(|node| node.component == "Dropdown")
                        .map(|_| id.clone())
                });
                let dropdown_invalidation = dropdown.map_or(Invalidation::None, |id| {
                    let open = !state.state(&id).is_some_and(|entry| entry.dropdown_open);
                    state.set_dropdown_open(id, open)
                });
                let option_parent = state.captured().and_then(|id| {
                    id.as_str()
                        .rsplit_once("/option-")
                        .map(|(parent, _)| UiId::from(parent.to_owned()))
                });
                let option_invalidation = option_parent
                    .map(|id| state.set_dropdown_open(id, false))
                    .unwrap_or(Invalidation::None);
                state
                    .set_pressed(None)
                    .merge(state.set_capture(None))
                    .merge(dropdown_invalidation)
                    .merge(option_invalidation)
            }
            UiEvent::Scroll { point, delta_y } => {
                let Some(scroll) = self.scrolls.iter().rev().find(|scroll| {
                    if !contains(scroll.rect, point) {
                        return false;
                    }
                    let maximum =
                        (scroll.extent.content.height - scroll.extent.viewport.height).max(0.0);
                    maximum > 0.0
                        && if delta_y > 0.0 {
                            scroll.extent.offset < maximum
                        } else if delta_y < 0.0 {
                            scroll.extent.offset > 0.0
                        } else {
                            false
                        }
                }) else {
                    return outcome;
                };
                let invalidation = state.scroll_by(
                    scroll.id.clone(),
                    delta_y,
                    (scroll.extent.content.height - scroll.extent.viewport.height).max(0.0),
                );
                if invalidation != Invalidation::None
                    && let Some(map) = scroll.offset_mapper
                    && let Some(offset) = state.state(&scroll.id).map(|entry| entry.scroll_offset)
                {
                    outcome.messages.push(map(offset));
                }
                invalidation
            }
            UiEvent::ScrollHorizontal { point, delta_x } => self
                .scrolls
                .iter()
                .rev()
                .find(|scroll| {
                    if !contains(scroll.rect, point) {
                        return false;
                    }
                    let maximum =
                        (scroll.extent.content.width - scroll.extent.viewport.width).max(0.0);
                    maximum > 0.0
                        && if delta_x > 0.0 {
                            scroll.extent.offset_x < maximum
                        } else if delta_x < 0.0 {
                            scroll.extent.offset_x > 0.0
                        } else {
                            false
                        }
                })
                .map(|scroll| {
                    state.scroll_by_x(
                        scroll.id.clone(),
                        delta_x,
                        (scroll.extent.content.width - scroll.extent.viewport.width).max(0.0),
                    )
                })
                .unwrap_or(Invalidation::None),
            UiEvent::FocusNext => self.move_focus(state, 1),
            UiEvent::FocusPrevious => self.move_focus(state, -1),
            UiEvent::ControllerNext => self.move_controller(state, 1),
            UiEvent::ControllerPrevious => self.move_controller(state, -1),
            UiEvent::ActivateFocused | UiEvent::KeyboardActivate => {
                if let Some(message) = state
                    .focused()
                    .and_then(|id| self.message_for_id(id))
                    .cloned()
                {
                    outcome.messages.push(message);
                }
                Invalidation::None
            }
            UiEvent::ControllerActivate => {
                if let Some(message) = state
                    .controller_selected()
                    .or_else(|| state.focused())
                    .and_then(|id| self.message_for_id(id))
                    .cloned()
                {
                    outcome.messages.push(message);
                }
                Invalidation::None
            }
            UiEvent::AccessibilityActivate(id) => {
                if let Some(message) = self.message_for_id(&id).cloned() {
                    outcome.messages.push(message);
                }
                Invalidation::None
            }
            UiEvent::TextInput(text) => {
                if let Some(id) = state.focused().cloned() {
                    if let Some(input) = self.text_inputs.iter().find(|input| input.id == id) {
                        let editor = state.editor(id, &input.initial);
                        editor.insert(&text);
                        outcome.messages.push((input.map)(editor.text().to_owned()));
                        state.show_caret();
                    }
                    Invalidation::Layout
                } else {
                    Invalidation::None
                }
            }
            UiEvent::ImePreedit(text) => {
                if let Some(id) = state.focused().cloned() {
                    state.editor(id, "").set_preedit(text, None);
                    Invalidation::Paint
                } else {
                    Invalidation::None
                }
            }
            UiEvent::TextBackspace => self
                .edit_focused_text(state, TextEditor::backspace)
                .map(|message| {
                    outcome.messages.push(message);
                    Invalidation::Layout
                })
                .unwrap_or(Invalidation::None),
            UiEvent::TextBackspaceWord => self
                .edit_focused_text(state, TextEditor::backspace_word)
                .map(|message| {
                    outcome.messages.push(message);
                    Invalidation::Layout
                })
                .unwrap_or(Invalidation::None),
            UiEvent::TextDelete => self
                .edit_focused_text(state, TextEditor::delete)
                .map(|message| {
                    outcome.messages.push(message);
                    Invalidation::Layout
                })
                .unwrap_or(Invalidation::None),
            UiEvent::TextMoveLeft { extend_selection } => {
                if let Some(message) =
                    self.edit_focused_text(state, |editor| editor.move_left(extend_selection))
                {
                    outcome.messages.push(message);
                    Invalidation::Paint
                } else {
                    self.move_document_selection(state, -1, false, extend_selection)
                }
            }
            UiEvent::TextMoveRight { extend_selection } => {
                if let Some(message) =
                    self.edit_focused_text(state, |editor| editor.move_right(extend_selection))
                {
                    outcome.messages.push(message);
                    Invalidation::Paint
                } else {
                    self.move_document_selection(state, 1, false, extend_selection)
                }
            }
            UiEvent::TextMoveWordLeft { extend_selection } => {
                if let Some(message) =
                    self.edit_focused_text(state, |editor| editor.move_word_left(extend_selection))
                {
                    outcome.messages.push(message);
                    Invalidation::Paint
                } else {
                    self.move_document_selection(state, -1, true, extend_selection)
                }
            }
            UiEvent::TextMoveWordRight { extend_selection } => {
                if let Some(message) =
                    self.edit_focused_text(state, |editor| editor.move_word_right(extend_selection))
                {
                    outcome.messages.push(message);
                    Invalidation::Paint
                } else {
                    self.move_document_selection(state, 1, true, extend_selection)
                }
            }
            UiEvent::TextMoveHome { extend_selection } => {
                if let Some(message) =
                    self.edit_focused_text(state, |editor| editor.move_home(extend_selection))
                {
                    outcome.messages.push(message);
                    Invalidation::Paint
                } else {
                    self.move_document_boundary(state, false, false, extend_selection)
                }
            }
            UiEvent::TextMoveEnd { extend_selection } => {
                if let Some(message) =
                    self.edit_focused_text(state, |editor| editor.move_end(extend_selection))
                {
                    outcome.messages.push(message);
                    Invalidation::Paint
                } else {
                    self.move_document_boundary(state, true, false, extend_selection)
                }
            }
            UiEvent::TextMoveDocumentHome { extend_selection } => {
                self.move_document_boundary(state, false, true, extend_selection)
            }
            UiEvent::TextMoveDocumentEnd { extend_selection } => {
                self.move_document_boundary(state, true, true, extend_selection)
            }
            UiEvent::TextSelectAll => {
                if let Some(message) = self.edit_focused_text(state, TextEditor::select_all) {
                    outcome.messages.push(message);
                    Invalidation::Paint
                } else if let Some(owner) = state.selection_owner().cloned()
                    && let Some(region) = self.selection_region(&owner)
                {
                    *state.document_selection_mut(owner) = region.document.select_all();
                    Invalidation::Paint
                } else {
                    Invalidation::None
                }
            }
            UiEvent::TextCopy => {
                outcome.clipboard_text = self.selected_text(state);
                Invalidation::None
            }
            UiEvent::TextCut => {
                let Some(id) = state.focused().cloned() else {
                    return outcome;
                };
                let Some(input) = self.text_inputs.iter().find(|input| input.id == id) else {
                    return outcome;
                };
                let editor = state.editor(id, &input.initial);
                outcome.clipboard_text = editor.cut_selection();
                if outcome.clipboard_text.is_some() {
                    outcome.messages.push((input.map)(editor.text().to_owned()));
                    state.show_caret();
                    Invalidation::Layout
                } else {
                    Invalidation::None
                }
            }
            UiEvent::TextPaste(text) => self
                .edit_focused_text(state, |editor| {
                    let text = text.replace("\r\n", "\n").replace('\r', "\n");
                    editor.insert(&text);
                })
                .map(|message| {
                    outcome.messages.push(message);
                    Invalidation::Layout
                })
                .unwrap_or(Invalidation::None),
            UiEvent::SelectionClear => state.clear_document_selection(),
            UiEvent::Dismiss => self
                .resolved_layout()
                .nodes()
                .iter()
                .filter(|node| node.component == "Dropdown")
                .fold(Invalidation::None, |invalidation, node| {
                    invalidation.merge(state.set_dropdown_open(node.id.clone(), false))
                }),
            UiEvent::CaretBlink => state.toggle_caret(),
            UiEvent::FocusLost => state.focus_lost(),
            UiEvent::Suspended => state.suspended(),
            UiEvent::DeviceRemoved => state.device_removed(),
        };
        outcome
    }

    fn edit_focused_text(
        &self,
        state: &mut UiStateStore,
        edit: impl FnOnce(&mut TextEditor),
    ) -> Option<Message> {
        let id = state.focused()?.clone();
        let input = self.text_inputs.iter().find(|input| input.id == id)?;
        let editor = state.editor(id, &input.initial);
        edit(editor);
        let message = (input.map)(editor.text().to_owned());
        state.show_caret();
        Some(message)
    }

    fn focused_editor<'a>(&self, state: &'a UiStateStore) -> Option<&'a TextEditor> {
        let id = state.focused()?;
        state.state(id)?.editor.as_ref()
    }

    fn move_document_selection(
        &self,
        state: &mut UiStateStore,
        direction: isize,
        word: bool,
        extend: bool,
    ) -> Invalidation {
        let Some(owner) = state.selection_owner().cloned() else {
            return Invalidation::None;
        };
        let Some(region) = self.selection_region(&owner) else {
            return Invalidation::None;
        };
        let selection = state
            .document_selection(&owner)
            .cloned()
            .unwrap_or_default();
        let current = if !extend {
            region.document.normalized(&selection).map(|(start, end)| {
                if direction.is_negative() { start } else { end }
            })
        } else {
            selection.focus.clone()
        }
        .or_else(|| region.document.document_boundary(!direction.is_negative()));
        let Some(current) = current else {
            return Invalidation::None;
        };
        let next = if word {
            region.document.move_word(&current, direction)
        } else {
            region.document.move_grapheme(&current, direction)
        }
        .unwrap_or_else(|| current.clone());
        let selection = state.document_selection_mut(owner);
        if !extend {
            selection.anchor = Some(next.clone());
        } else if selection.anchor.is_none() {
            selection.anchor = Some(current);
        }
        selection.focus = Some(next);
        Invalidation::Paint
    }

    fn move_document_boundary(
        &self,
        state: &mut UiStateStore,
        end: bool,
        document: bool,
        extend: bool,
    ) -> Invalidation {
        let Some(owner) = state.selection_owner().cloned() else {
            return Invalidation::None;
        };
        let Some(region) = self.selection_region(&owner) else {
            return Invalidation::None;
        };
        let current = state
            .document_selection(&owner)
            .and_then(|selection| selection.focus.clone())
            .or_else(|| region.document.document_boundary(end));
        let Some(current) = current else {
            return Invalidation::None;
        };
        let next = if document {
            region.document.document_boundary(end)
        } else {
            region.document.block_boundary(&current, end)
        }
        .unwrap_or_else(|| current.clone());
        let selection = state.document_selection_mut(owner);
        if !extend {
            selection.anchor = Some(next.clone());
        } else if selection.anchor.is_none() {
            selection.anchor = Some(current);
        }
        selection.focus = Some(next);
        Invalidation::Paint
    }

    fn move_focus(&self, state: &mut UiStateStore, direction: isize) -> Invalidation {
        let mut ids = self.hits.iter().map(|hit| &hit.id).collect::<Vec<_>>();
        ids.dedup();
        if ids.is_empty() {
            return state.set_focus(None);
        }
        let current = state
            .focused()
            .and_then(|focused| ids.iter().position(|id| *id == focused));
        let next = match (current, direction.is_negative()) {
            (Some(index), false) => (index + 1) % ids.len(),
            (Some(0), true) | (None, true) => ids.len() - 1,
            (Some(index), true) => index - 1,
            (None, false) => 0,
        };
        state.set_focus(Some(ids[next].clone()))
    }

    fn move_controller(&self, state: &mut UiStateStore, direction: isize) -> Invalidation {
        let mut ids = self.hits.iter().map(|hit| &hit.id).collect::<Vec<_>>();
        ids.dedup();
        if ids.is_empty() {
            return state.set_controller_selected(None);
        }
        let current = state
            .controller_selected()
            .and_then(|selected| ids.iter().position(|id| *id == selected));
        let next = match (current, direction.is_negative()) {
            (Some(index), false) => (index + 1) % ids.len(),
            (Some(0), true) | (None, true) => ids.len() - 1,
            (Some(index), true) => index - 1,
            (None, false) => 0,
        };
        state.set_controller_selected(Some(ids[next].clone()))
    }

    pub fn push_overlay_command(&mut self, command: PaintCommand) {
        self.commands.push(command);
    }

    pub fn push_overlay_message(&mut self, rect: Rect, message: Message) {
        self.messages.push(MessageRegion {
            id: UiId::from("overlay"),
            rect,
            message: message.clone(),
        });
        self.hits.push(HitRegion {
            id: UiId::from("overlay"),
            rect,
            message: Some(message),
            message_mapper: None,
        });
    }

    pub fn apply_interaction_state(&mut self, state: &UiStateStore) {
        for node in &mut self.resolved.nodes {
            node.interaction.focused = state.focused() == Some(&node.id);
            node.interaction.hovered = state.hovered() == Some(&node.id);
            node.interaction.pressed = state.pressed() == Some(&node.id);
            node.interaction.captured = state.captured() == Some(&node.id);
            node.interaction.controller_selected = state.controller_selected() == Some(&node.id);
        }
    }

    pub fn enable_diagnostic_overlay(&mut self, state: Option<&UiStateStore>) {
        self.enable_diagnostic_overlay_with_damage(state, &[]);
    }

    pub fn enable_diagnostic_overlay_with_damage(
        &mut self,
        state: Option<&UiStateStore>,
        damage: &[Rect],
    ) {
        if let Some(state) = state {
            self.apply_interaction_state(state);
        }
        for (index, node) in self.resolved.nodes.iter().enumerate() {
            self.commands.push(PaintCommand::OverlayStroke {
                rect: node.allocated,
                color: if node.interaction.focused || node.interaction.controller_selected {
                    0xffd166
                } else if node.interaction.hovered || node.interaction.pressed {
                    0xff6b9d
                } else {
                    0x40c9ff
                },
                width: 1.0,
            });
            self.commands.push(PaintCommand::OverlayStroke {
                rect: node.border_box,
                color: 0xb388ff,
                width: 1.0,
            });
            self.commands.push(PaintCommand::OverlayStroke {
                rect: node.padding_box,
                color: 0xffc857,
                width: 1.0,
            });
            self.commands.push(PaintCommand::OverlayStroke {
                rect: node.content,
                color: 0x72e6a0,
                width: 1.0,
            });
            if let Some(clip) = node.clip {
                self.commands.push(PaintCommand::OverlayStroke {
                    rect: clip,
                    color: 0xff4d6d,
                    width: 1.0,
                });
            }
            if let Some(scroll) = node.scroll {
                self.commands.push(PaintCommand::OverlayStroke {
                    rect: Rect::new(
                        node.content.origin.x - scroll.offset_x,
                        node.content.origin.y - scroll.offset,
                        scroll.content.width,
                        scroll.content.height,
                    ),
                    color: 0x00d4aa,
                    width: 1.0,
                });
            }
            if index < 96 {
                let source = node.source.map_or_else(
                    || "none".into(),
                    |source| {
                        format!(
                            "{}:{}:{}",
                            source.file.rsplit('/').next().unwrap_or(source.file),
                            source.line,
                            source.column
                        )
                    },
                );
                let grid = node
                    .grid_tracks
                    .iter()
                    .take(6)
                    .map(|track| format!("{track:.0}"))
                    .collect::<Vec<_>>()
                    .join(",");
                let scroll = node.scroll.map_or_else(
                    || "none".into(),
                    |scroll| {
                        format!(
                            "{:.0}x{:.0}/{:.0}x{:.0} @ {:.0},{:.0} max {:.0},{:.0}",
                            scroll.viewport.width,
                            scroll.viewport.height,
                            scroll.content.width,
                            scroll.content.height,
                            scroll.offset_x,
                            scroll.offset,
                            (scroll.content.width - scroll.viewport.width).max(0.0),
                            (scroll.content.height - scroll.viewport.height).max(0.0),
                        )
                    },
                );
                self.commands.push(PaintCommand::Text {
                    bounds: Rect::new(
                        node.allocated.origin.x + 2.0,
                        node.allocated.origin.y + 2.0,
                        node.allocated.size.width.max(0.0),
                        52.0,
                    ),
                    text: format!(
                        "{} {}\n{} final {:.0}x{:.0} pref {:.0}x{:.0}\nmin {:.0}x{:.0} max {:.0}x{:.0} flex {:?}/{:.1}/{:.1}\ngrid [{}] scroll {}",
                        node.component,
                        node.id.as_str(),
                        source,
                        node.allocated.size.width,
                        node.allocated.size.height,
                        node.preferred.width,
                        node.preferred.height,
                        node.constraints.min.width,
                        node.constraints.min.height,
                        node.constraints.max.width,
                        node.constraints.max.height,
                        node.flex_basis,
                        node.flex_grow,
                        node.flex_shrink,
                        grid,
                        scroll,
                    ),
                    scale: 0.7,
                    color: 0xffffff,
                    align: TextAlign::Start,
                    bold: false,
                });
            }
        }
        for (stack, hit) in self.hits.iter().enumerate() {
            self.commands.push(PaintCommand::OverlayStroke {
                rect: hit.rect,
                color: 0xff9f43u32.saturating_add(stack.min(255) as u32),
                width: 1.0,
            });
        }
        for rect in damage {
            self.commands.push(PaintCommand::OverlayStroke {
                rect: *rect,
                color: 0xff2dff,
                width: 2.0,
            });
        }
    }

    fn diagnostic(&mut self, kind: DiagnosticKind, id: &UiId, detail: impl Into<String>) {
        if !self.diagnostics_enabled || self.diagnostics.len() >= 128 {
            return;
        }
        let key = (kind, id.clone());
        if self.diagnostic_keys.insert(key) {
            let source = self
                .resolved
                .nodes
                .iter()
                .rev()
                .find(|node| &node.id == id)
                .and_then(|node| node.source);
            self.diagnostics.push(LayoutDiagnostic {
                kind,
                id: id.clone(),
                detail: detail.into(),
                source,
            });
        }
    }

    fn emit_accessibility_geometry(&mut self) {
        self.accessibility = self
            .resolved
            .nodes
            .iter()
            .filter_map(|node| {
                let rect = node
                    .clip
                    .map(|clip| intersection(node.allocated, clip))
                    .unwrap_or(Some(node.allocated))?;
                Some(AccessibilityNode {
                    id: node.id.clone(),
                    component: node.component,
                    rect,
                    interactive: node.interaction.interactive,
                })
            })
            .collect();
    }

    fn reset_emission(&mut self) {
        self.commands.clear();
        self.overlay_commands.clear();
        self.hits.clear();
        self.overlay_hits.clear();
        self.messages.clear();
        self.text_inputs.clear();
        for node in &mut self.resolved.nodes {
            node.hit_stack = None;
        }
    }

    fn validate_clip_commands(&mut self) {
        if !self.diagnostics_enabled {
            return;
        }
        let mut depth = 0usize;
        let mut invalid = false;
        for command in &self.commands {
            match command {
                PaintCommand::PushClip(_) => depth += 1,
                PaintCommand::PopClip if depth == 0 => invalid = true,
                PaintCommand::PopClip => depth -= 1,
                _ => {}
            }
        }
        if invalid || depth != 0 {
            self.diagnostic(
                DiagnosticKind::UnbalancedClip,
                &UiId::from("root"),
                format!("paint clip stack ended at depth {depth}"),
            );
        }
    }
}

fn rectangles_overlap(left: Rect, right: Rect) -> bool {
    left.origin.x < right.origin.x + right.size.width
        && left.origin.x + left.size.width > right.origin.x
        && left.origin.y < right.origin.y + right.size.height
        && left.origin.y + left.size.height > right.origin.y
}

fn rects_intersect(left: Rect, right: Rect) -> bool {
    left.origin.x < right.origin.x + right.size.width
        && left.origin.x + left.size.width > right.origin.x
        && left.origin.y < right.origin.y + right.size.height
        && left.origin.y + left.size.height > right.origin.y
}

fn intersection(left: Rect, right: Rect) -> Option<Rect> {
    let x = left.origin.x.max(right.origin.x);
    let y = left.origin.y.max(right.origin.y);
    let right_edge = (left.origin.x + left.size.width).min(right.origin.x + right.size.width);
    let bottom_edge = (left.origin.y + left.size.height).min(right.origin.y + right.size.height);
    (right_edge > x && bottom_edge > y).then(|| Rect::new(x, y, right_edge - x, bottom_edge - y))
}

fn approximately_same_rect(left: Rect, right: Rect) -> bool {
    const EPSILON: f32 = 0.01;
    (left.origin.x - right.origin.x).abs() <= EPSILON
        && (left.origin.y - right.origin.y).abs() <= EPSILON
        && (left.size.width - right.size.width).abs() <= EPSILON
        && (left.size.height - right.size.height).abs() <= EPSILON
}

pub(super) fn measure_element<Message>(
    element: &Element<Message>,
    constraints: Constraints,
) -> Size {
    let horizontal_padding = element.style.padding.width();
    let vertical_padding = element.style.padding.height();
    let child_max = Size::new(
        (constraints.max.width - horizontal_padding).max(0.0),
        (constraints.max.height - vertical_padding).max(0.0),
    );
    let child_constraints = Constraints::loose(child_max);
    let content = match &element.kind {
        Kind::Text {
            value,
            scale,
            bold,
            wrap,
            line_height,
            max_lines,
            ..
        } => measure_text(
            value,
            *scale,
            *bold,
            *wrap,
            *line_height,
            *max_lines,
            child_max.width,
        ),
        Kind::StyledText {
            value,
            spans,
            scale,
            wrap,
            line_height,
        } => measure_styled_text(value, spans, *scale, *wrap, *line_height, child_max.width),
        Kind::Image { image, .. } => {
            let intrinsic = if image.width() == 0 || image.height() == 0 {
                Size::new(16.0, 16.0)
            } else {
                Size::new(image.width() as f32, image.height() as f32)
            };
            match (
                explicit_px(element.style.width),
                explicit_px(element.style.height),
            ) {
                (Some(width), None) if intrinsic.width > 0.0 => {
                    Size::new(width, width * intrinsic.height / intrinsic.width)
                }
                (None, Some(height)) if intrinsic.height > 0.0 => {
                    Size::new(height * intrinsic.width / intrinsic.height, height)
                }
                _ => intrinsic,
            }
        }
        Kind::Slider { .. } => Size::new(120.0, 24.0),
        Kind::Dropdown {
            selected,
            options,
            expanded,
            overlay,
            ..
        } => {
            let width = std::iter::once(selected)
                .chain(options)
                .map(|label| {
                    measure_text(label, 2.0, false, false, None, Some(1), f32::INFINITY).width
                })
                .fold(0.0_f32, f32::max)
                + 48.0;
            Size::new(
                width,
                if *overlay {
                    30.0
                } else {
                    42.0 + if *expanded {
                        options.len() as f32 * 36.0
                    } else {
                        0.0
                    }
                },
            )
        }
        Kind::Flex(axis) => {
            let sizes = element
                .children
                .iter()
                .map(|child| {
                    let max = match axis {
                        Axis::Horizontal => Size::new(f32::INFINITY, child_max.height),
                        Axis::Vertical => Size::new(child_max.width, f32::INFINITY),
                    };
                    measure_element(child, Constraints::loose(max))
                })
                .collect::<Vec<_>>();
            let gap = element.style.gap.max(0.0) * element.children.len().saturating_sub(1) as f32;
            match axis {
                Axis::Horizontal if child_max.width.is_finite() => {
                    let items = element
                        .children
                        .iter()
                        .zip(&sizes)
                        .map(|(child, measured)| {
                            let preferred = child.style.basis.resolve(
                                child_max.width,
                                child.style.width.resolve(child_max.width, measured.width),
                            );
                            FlexItem::flex(
                                preferred,
                                child.style.min_width.max(0.0),
                                child.style.max_width.max(child.style.min_width.max(0.0)),
                                child.style.grow,
                                child.style.shrink,
                            )
                        })
                        .collect::<Vec<_>>();
                    let rects = layout_flex(
                        Rect::new(0.0, 0.0, child_max.width, 0.0),
                        Axis::Horizontal,
                        element.style.gap.max(0.0),
                        &items,
                    );
                    let height = element
                        .children
                        .iter()
                        .zip(&rects)
                        .map(|(child, rect)| {
                            measure_element(
                                child,
                                Constraints::loose(Size::new(rect.size.width, f32::INFINITY)),
                            )
                            .height
                        })
                        .fold(0.0, f32::max);
                    Size::new(
                        rects.iter().map(|rect| rect.size.width).sum::<f32>() + gap,
                        height,
                    )
                }
                Axis::Horizontal => Size::new(
                    sizes.iter().map(|size| size.width).sum::<f32>() + gap,
                    sizes.iter().map(|size| size.height).fold(0.0, f32::max),
                ),
                Axis::Vertical => Size::new(
                    sizes.iter().map(|size| size.width).fold(0.0, f32::max),
                    sizes.iter().map(|size| size.height).sum::<f32>() + gap,
                ),
            }
        }
        Kind::Grid { columns } => {
            let contributions = element
                .children
                .iter()
                .map(|child| measure_element(child, child_constraints))
                .collect::<Vec<_>>();
            let widths = resolve_grid_columns(
                columns,
                child_max.width,
                element.style.gap,
                &contributions,
                element.children.len(),
            );
            let columns = widths.len().max(1);
            let sizes = element
                .children
                .iter()
                .enumerate()
                .map(|(index, child)| {
                    measure_element(
                        child,
                        Constraints::loose(Size::new(widths[index % columns], child_max.height)),
                    )
                })
                .collect::<Vec<_>>();
            let rows = sizes.len().div_ceil(columns);
            let row_heights = (0..rows)
                .map(|row| {
                    sizes[row * columns..sizes.len().min((row + 1) * columns)]
                        .iter()
                        .map(|size| size.height)
                        .fold(0.0, f32::max)
                })
                .collect::<Vec<_>>();
            Size::new(
                widths.iter().sum::<f32>()
                    + element.style.gap * widths.len().saturating_sub(1) as f32,
                row_heights.iter().sum::<f32>() + element.style.gap * rows.saturating_sub(1) as f32,
            )
        }
        Kind::VerticalScroll { .. } => element
            .children
            .first()
            .map(|child| {
                measure_element(
                    child,
                    Constraints::loose(Size::new(child_max.width, f32::INFINITY)),
                )
            })
            .unwrap_or_default(),
    };
    let intrinsic_width = match (&element.kind, element.style.width) {
        (
            Kind::Text {
                value, scale, bold, ..
            },
            Length::MinContent,
        ) => {
            value
                .split_whitespace()
                .map(|word| {
                    measure_text(word, *scale, *bold, false, None, Some(1), f32::INFINITY).width
                })
                .fold(0.0, f32::max)
                + horizontal_padding
        }
        (
            Kind::Text {
                value, scale, bold, ..
            },
            Length::MaxContent,
        ) => {
            measure_text(value, *scale, *bold, false, None, Some(1), f32::INFINITY).width
                + horizontal_padding
        }
        _ => content.width + horizontal_padding,
    };
    let horizontal_scrollbar_gutter = match element.style.overflow_x {
        Overflow::Scroll => SCROLLBAR_GUTTER,
        Overflow::Auto if content.width > child_max.width + 0.01 => SCROLLBAR_GUTTER,
        _ => 0.0,
    };
    let intrinsic = Size::new(
        intrinsic_width,
        content.height + vertical_padding + horizontal_scrollbar_gutter,
    );
    let min_width = element.style.min_width.max(0.0);
    let min_height = element.style.min_height.max(0.0);
    let max_width = element.style.max_width.max(min_width);
    let max_height = element.style.max_height.max(min_height);
    let resolved = Size::new(
        element
            .style
            .width
            .resolve(constraints.max.width, intrinsic.width),
        element
            .style
            .height
            .resolve(constraints.max.height, intrinsic.height),
    );
    constraints.constrain(Size::new(
        resolved.width.clamp(min_width, max_width),
        resolved.height.clamp(min_height, max_height),
    ))
}

fn is_scroll_container<Message>(element: &Element<Message>) -> bool {
    matches!(element.kind, Kind::VerticalScroll { .. })
        || matches!(element.style.overflow_x, Overflow::Scroll | Overflow::Auto)
        || matches!(element.style.overflow_y, Overflow::Scroll | Overflow::Auto)
}

fn collect_selection_regions<Message>(
    root: &Element<Message>,
    resolved: &ResolvedLayout,
) -> Vec<SelectionRegionLayout> {
    fn visit<Message>(
        element: &Element<Message>,
        node_index: usize,
        resolved: &ResolvedLayout,
        builders: &mut Vec<SelectionRegionBuilder>,
        active: Option<usize>,
        excluded: bool,
    ) {
        let node = &resolved.nodes[node_index];
        let active = if element.style.selection_region {
            builders.push(SelectionRegionBuilder {
                id: node.id.clone(),
                rect: node.content,
                supplied: element.style.selection_document.clone(),
                runs: Vec::new(),
                logical_runs: Vec::new(),
            });
            Some(builders.len() - 1)
        } else {
            active
        };
        let excluded = excluded
            || element.message.is_some() && !is_scroll_container(element)
            || element.text_mapper.is_some()
            || matches!(element.kind, Kind::Slider { .. } | Kind::Dropdown { .. });
        if let Some(region_index) = active
            && !excluded
            && element.style.selectable != Some(false)
            && let Kind::Text {
                value,
                scale,
                bold,
                wrap,
                line_height,
                max_lines,
                ..
            } = &element.kind
        {
            let run_id = element
                .style
                .selection_run_id
                .clone()
                .unwrap_or_else(|| node.id.as_str().to_owned());
            let text = value.clone();
            let glyphs = shape_selection_glyphs(
                &text,
                node.content,
                node.clip,
                *scale,
                *bold,
                *wrap,
                *line_height,
                *max_lines,
                element.style.text_align,
            );
            builders[region_index].logical_runs.push(SelectionRun {
                id: run_id.clone(),
                text: Arc::from(text.clone()),
                boundary_before: element.style.selection_boundary,
            });
            builders[region_index].runs.push(SelectionRunGeometry {
                node_index,
                run_id,
                glyphs,
            });
        }
        if let Some(region_index) = active
            && !excluded
            && element.style.selectable != Some(false)
            && let Kind::StyledText {
                value,
                scale,
                wrap,
                line_height,
                ..
            } = &element.kind
        {
            let run_id = element
                .style
                .selection_run_id
                .clone()
                .unwrap_or_else(|| node.id.as_str().to_owned());
            let text = value.clone();
            let glyphs = shape_selection_glyphs(
                &text,
                node.content,
                node.clip,
                *scale,
                false,
                *wrap,
                *line_height,
                None,
                element.style.text_align,
            );
            builders[region_index].logical_runs.push(SelectionRun {
                id: run_id.clone(),
                text: Arc::from(text),
                boundary_before: element.style.selection_boundary,
            });
            builders[region_index].runs.push(SelectionRunGeometry {
                node_index,
                run_id,
                glyphs,
            });
        }
        for (&child_index, child) in node.children.iter().zip(&element.children) {
            visit(child, child_index, resolved, builders, active, excluded);
        }
    }

    let mut builders = Vec::new();
    visit(root, 0, resolved, &mut builders, None, false);
    builders
        .into_iter()
        .map(|builder| SelectionRegionLayout {
            id: builder.id,
            rect: builder.rect,
            document: builder
                .supplied
                .unwrap_or_else(|| Arc::new(SelectionDocument::new(builder.logical_runs))),
            runs: builder.runs,
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn shape_selection_glyphs(
    text: &str,
    rect: Rect,
    clip: Option<Rect>,
    scale: f32,
    bold: bool,
    wrap: bool,
    line_height: Option<f32>,
    max_lines: Option<usize>,
    align: TextAlign,
) -> Vec<SelectionGlyph> {
    let font_size = text_font_size(scale);
    let line_height = line_height.unwrap_or(font_size * 1.3).max(1.0);
    let mut line_bases = Vec::new();
    let mut base = 0;
    for line in text.split_inclusive('\n') {
        line_bases.push(base);
        base += line.len();
    }
    if line_bases.is_empty() {
        line_bases.push(0);
    }
    TEXT_MEASURER.with(|measurer| {
        let mut measurer = measurer.borrow_mut();
        let font_system = &mut measurer.font_system;
        let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
        buffer.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
        buffer.set_size(wrap.then_some(rect.size.width.max(1.0)), None);
        let mut attrs = Attrs::new().family(Family::SansSerif);
        if bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);
        let mut glyphs = Vec::new();
        for run in buffer.layout_runs().take(max_lines.unwrap_or(usize::MAX)) {
            let align_offset = match align {
                TextAlign::Start => 0.0,
                TextAlign::Center => (rect.size.width - run.line_w).max(0.0) * 0.5,
                TextAlign::End => (rect.size.width - run.line_w).max(0.0),
            };
            let line_base = line_bases.get(run.line_i).copied().unwrap_or(0);
            for glyph in run.glyphs {
                let glyph_rect = Rect::new(
                    rect.origin.x + align_offset + glyph.x,
                    rect.origin.y + run.line_top,
                    glyph.w.max(1.0),
                    run.line_height,
                );
                let visible = clip
                    .and_then(|clip| intersection(glyph_rect, clip))
                    .or_else(|| clip.is_none().then_some(glyph_rect));
                if let Some(rect) = visible {
                    glyphs.push(SelectionGlyph {
                        rect,
                        start: line_base + glyph.start,
                        end: line_base + glyph.end,
                        rtl: glyph.level.is_rtl(),
                    });
                }
            }
        }
        glyphs
    })
}

fn emit_element<Message: Clone>(
    element: &Element<Message>,
    node_index: usize,
    inherited_foreground: Option<Color>,
    tree: &mut UiTree<Message>,
) {
    let node = tree.resolved.nodes[node_index].clone();
    let rect = node.allocated;
    if node
        .clip
        .is_some_and(|clip| intersection(rect, clip).is_none())
    {
        let foreground = element.style.foreground.or(inherited_foreground);
        if matches!(element.kind, Kind::Flex(_) | Kind::Grid { .. }) {
            for (&child_index, child) in node.children.iter().zip(&element.children) {
                emit_element(child, child_index, foreground, tree);
            }
        }
        return;
    }
    if let Some(background) = element.style.background {
        tree.commands.push(match background {
            Background::Solid(color) if element.style.corner_radius > 0.0 => {
                PaintCommand::RoundedFill {
                    rect,
                    color,
                    radius: element.style.corner_radius,
                }
            }
            Background::Solid(color) if element.style.top_corner_radius > 0.0 => {
                PaintCommand::TopRoundedFill {
                    rect,
                    color,
                    radius: element.style.top_corner_radius,
                }
            }
            Background::Solid(color) => PaintCommand::Fill { rect, color },
            Background::LinearGradient(gradient) => PaintCommand::Gradient { rect, gradient },
        });
    }
    if let Some(color) = element.style.border {
        tree.commands.push(PaintCommand::Stroke {
            rect,
            color,
            width: element.style.border_width,
        });
    }
    if let Some(message) = &element.message {
        tree.messages.push(MessageRegion {
            id: node.id.clone(),
            rect,
            message: message.clone(),
        });
        if !is_scroll_container(element)
            && let Some(hit_rect) = node
                .clip
                .map(|clip| intersection(rect, clip))
                .unwrap_or(Some(rect))
        {
            tree.resolved.nodes[node_index].hit_stack = Some(tree.hits.len());
            tree.hits.push(HitRegion {
                id: node.id.clone(),
                rect: hit_rect,
                message: Some(message.clone()),
                message_mapper: element.message_mapper,
            });
        }
    }
    if let Some(map) = element.text_mapper
        && let Kind::Text {
            value, input_value, ..
        } = &element.kind
    {
        let (scale, bold, line_height) = match &element.kind {
            Kind::Text {
                scale,
                bold,
                line_height,
                ..
            } => (
                *scale,
                *bold,
                line_height.unwrap_or_else(|| text_font_size(*scale) * 1.3),
            ),
            _ => unreachable!(),
        };
        tree.text_inputs.push(TextInputRegion {
            id: node.id.clone(),
            rect,
            scale,
            bold,
            line_height,
            initial: input_value.clone().unwrap_or_else(|| value.clone()),
            map,
        });
        if element.message.is_none()
            && let Some(hit_rect) = node
                .clip
                .map(|clip| intersection(rect, clip))
                .unwrap_or(Some(rect))
        {
            tree.resolved.nodes[node_index].hit_stack = Some(tree.hits.len());
            tree.hits.push(HitRegion {
                id: node.id.clone(),
                rect: hit_rect,
                message: None,
                message_mapper: None,
            });
        }
    }

    let foreground = element.style.foreground.or(inherited_foreground);
    let clips_descendants = element.style.overflow_x != Overflow::Visible
        || element.style.overflow_y != Overflow::Visible;
    if clips_descendants {
        tree.commands.push(PaintCommand::PushClip(rect));
    }
    if let Some(rects) = tree.selection_paints.get(&node_index) {
        tree.commands
            .extend(rects.iter().map(|rect| PaintCommand::Fill {
                rect: *rect,
                color: 0x315a8f,
            }));
    }
    if let Kind::Text {
        scale,
        selection_x: Some((start, end)),
        ..
    } = &element.kind
    {
        tree.commands.push(PaintCommand::Fill {
            rect: Rect::new(
                rect.origin.x + *start,
                rect.origin.y,
                (*end - *start).max(1.0),
                text_font_size(*scale) * 1.3,
            ),
            color: 0x315a8f,
        });
    }
    if let Kind::Text {
        scale,
        caret_position: Some(caret_position),
        ..
    } = &element.kind
    {
        tree.commands.push(PaintCommand::Fill {
            rect: Rect::new(
                rect.origin.x + caret_position.x,
                rect.origin.y + caret_position.y,
                1.5,
                text_font_size(*scale) * 1.3,
            ),
            color: foreground.unwrap_or(0x00ff_ffff),
        });
    }
    match &element.kind {
        Kind::Text {
            value,
            scale,
            bold,
            ellipsis,
            ..
        } => tree.commands.push(PaintCommand::Text {
            bounds: rect,
            text: text_for_bounds(value, *scale, *bold, *ellipsis, rect.size.width),
            scale: *scale,
            color: foreground.unwrap_or(0x00ff_ffff),
            align: element.style.text_align,
            bold: *bold,
        }),
        Kind::StyledText {
            value,
            spans,
            scale,
            wrap,
            line_height,
        } => {
            tree.commands.push(PaintCommand::StyledText {
                bounds: rect,
                text: value.clone(),
                spans: spans.clone(),
                scale: *scale,
                color: foreground.unwrap_or(0x00ff_ffff),
                align: element.style.text_align,
            });
            if !element.inline_messages.is_empty() {
                let glyphs = shape_selection_glyphs(
                    value,
                    rect,
                    node.clip,
                    *scale,
                    false,
                    *wrap,
                    *line_height,
                    None,
                    element.style.text_align,
                );
                for (index, (range, message)) in element.inline_messages.iter().enumerate() {
                    let link_id = node.id.scoped(format!("$inline-{index}"));
                    for glyph in &glyphs {
                        if glyph.end <= range.start || glyph.start >= range.end {
                            continue;
                        }
                        tree.messages.push(MessageRegion {
                            id: link_id.clone(),
                            rect: glyph.rect,
                            message: message.clone(),
                        });
                        tree.hits.push(HitRegion {
                            id: link_id.clone(),
                            rect: glyph.rect,
                            message: Some(message.clone()),
                            message_mapper: None,
                        });
                    }
                }
            }
        }
        Kind::Image {
            id,
            image,
            presentation,
        } => tree.commands.push(PaintCommand::Image {
            bounds: presentation
                .bounds(rect, Size::new(image.width() as f32, image.height() as f32)),
            id: *id,
            image: image.clone(),
        }),
        Kind::Slider {
            value,
            track,
            fill,
            thumb,
        } => {
            let track_rect = Rect::new(
                rect.origin.x,
                rect.origin.y + rect.size.height / 2.0 - 3.0,
                rect.size.width,
                6.0,
            );
            let fill_width = track_rect.size.width * value.clamp(0.0, 1.0);
            tree.commands.push(PaintCommand::Fill {
                rect: track_rect,
                color: *track,
            });
            tree.commands.push(PaintCommand::Fill {
                rect: Rect::new(
                    track_rect.origin.x,
                    track_rect.origin.y,
                    fill_width,
                    track_rect.size.height,
                ),
                color: *fill,
            });
            tree.commands.push(PaintCommand::Fill {
                rect: Rect::new(
                    track_rect.origin.x + fill_width - 7.0,
                    rect.origin.y + rect.size.height / 2.0 - 7.0,
                    14.0,
                    14.0,
                ),
                color: *thumb,
            });
        }
        Kind::Dropdown {
            selected,
            options,
            expanded,
            overlay,
            background,
            option_background,
            foreground,
        } => {
            let header_height = if *overlay { 30.0 } else { 42.0 };
            let option_height = if *overlay { 34.0 } else { 36.0 };
            let header = Rect::new(rect.origin.x, rect.origin.y, rect.size.width, header_height);
            tree.commands.push(PaintCommand::Fill {
                rect: header,
                color: *background,
            });
            tree.commands.push(PaintCommand::Text {
                bounds: header.inset(Insets {
                    top: if *overlay { 5.0 } else { 10.0 },
                    right: 36.0,
                    bottom: if *overlay { 4.0 } else { 8.0 },
                    left: 12.0,
                }),
                text: selected.clone(),
                scale: 2.0,
                color: *foreground,
                align: TextAlign::Start,
                bold: false,
            });
            tree.commands.push(PaintCommand::Text {
                bounds: Rect::new(
                    header.origin.x + header.size.width - 32.0,
                    header.origin.y + if *overlay { 5.0 } else { 10.0 },
                    20.0,
                    22.0,
                ),
                text: if *expanded { "▲" } else { "▼" }.into(),
                scale: 1.0,
                color: *foreground,
                align: TextAlign::Center,
                bold: false,
            });
            if *expanded {
                for (index, option) in options.iter().enumerate() {
                    let option_rect = Rect::new(
                        rect.origin.x,
                        rect.origin.y + header_height + index as f32 * option_height,
                        rect.size.width,
                        option_height,
                    );
                    let commands = if *overlay {
                        &mut tree.overlay_commands
                    } else {
                        &mut tree.commands
                    };
                    commands.push(PaintCommand::Fill {
                        rect: option_rect,
                        color: *option_background,
                    });
                    commands.push(PaintCommand::Text {
                        bounds: option_rect.inset(Insets {
                            top: 7.0,
                            right: 12.0,
                            bottom: 7.0,
                            left: 12.0,
                        }),
                        text: option.clone(),
                        scale: 2.0,
                        color: *foreground,
                        align: TextAlign::Start,
                        bold: false,
                    });
                    let option_id = node.id.scoped(format!("option-{index}"));
                    let message = element.option_messages.get(index).cloned().flatten();
                    if let Some(message) = &message {
                        tree.messages.push(MessageRegion {
                            id: option_id.clone(),
                            rect: option_rect,
                            message: message.clone(),
                        });
                    }
                    if let Some(hit_rect) = node
                        .clip
                        .map(|clip| intersection(option_rect, clip))
                        .unwrap_or(Some(option_rect))
                    {
                        let hits = if *overlay {
                            &mut tree.overlay_hits
                        } else {
                            &mut tree.hits
                        };
                        hits.push(HitRegion {
                            id: option_id,
                            rect: hit_rect,
                            message,
                            message_mapper: None,
                        });
                    }
                }
            }
        }
        Kind::VerticalScroll { .. } => {
            tree.commands.push(PaintCommand::PushClip(node.content));
            for (&child_index, child) in node.children.iter().zip(&element.children) {
                emit_element(child, child_index, foreground, tree);
            }
            tree.commands.push(PaintCommand::PopClip);
        }
        Kind::Flex(_) | Kind::Grid { .. } => {
            for (&child_index, child) in node.children.iter().zip(&element.children) {
                emit_element(child, child_index, foreground, tree);
            }
        }
    }
    if clips_descendants {
        tree.commands.push(PaintCommand::PopClip);
    }
}

fn layout_element<Message: Clone>(
    element: &Element<Message>,
    id: &UiId,
    bounds: Rect,
    inherited_foreground: Option<Color>,
    inherited_clip: Option<Rect>,
    tree: &mut UiTree<Message>,
) -> usize {
    let rect = bounds;
    let preferred = measure_element(element, Constraints::loose(bounds.size));
    let node_index = tree.resolved.nodes.len();
    let interaction = InteractionState {
        interactive: element.message.is_some() && !is_scroll_container(element)
            || element.text_mapper.is_some(),
        ..InteractionState::default()
    };
    tree.resolved.nodes.push(ResolvedNode {
        component: element.kind.name(),
        id: id.clone(),
        source: element.source,
        allocated: rect,
        padding_box: rect,
        border_box: rect,
        content: rect.inset(element.style.padding),
        constraints: Constraints::new(
            Size::new(element.style.min_width, element.style.min_height),
            Size::new(element.style.max_width, element.style.max_height),
        ),
        preferred,
        flex_basis: element.style.basis,
        flex_grow: element.style.grow,
        flex_shrink: element.style.shrink,
        clip: inherited_clip,
        scroll: None,
        grid_tracks: Vec::new(),
        hit_stack: None,
        interaction,
        children: Vec::new(),
    });
    if tree.diagnostics_enabled && !tree.seen_ids.insert(id.clone()) {
        tree.diagnostic(
            DiagnosticKind::DuplicateIdentity,
            id,
            "stable identity occurs more than once in this surface",
        );
    }
    if element.style.min_width > element.style.max_width
        || element.style.min_height > element.style.max_height
    {
        tree.diagnostic(
            DiagnosticKind::ContradictoryConstraints,
            id,
            format!(
                "min=({:.2},{:.2}) max=({:.2},{:.2})",
                element.style.min_width,
                element.style.min_height,
                element.style.max_width,
                element.style.max_height
            ),
        );
    }
    if !rect.origin.x.is_finite()
        || !rect.origin.y.is_finite()
        || !rect.size.width.is_finite()
        || !rect.size.height.is_finite()
        || rect.size.width < 0.0
        || rect.size.height < 0.0
    {
        tree.diagnostic(
            DiagnosticKind::InvalidGeometry,
            id,
            format!("allocated rectangle is {rect:?}"),
        );
    }
    if matches!(element.style.width, Length::Percent(_)) && !rect.size.width.is_finite()
        || matches!(element.style.height, Length::Percent(_)) && !rect.size.height.is_finite()
    {
        tree.diagnostic(
            DiagnosticKind::IndefinitePercentage,
            id,
            "percentage length resolved against indefinite parent space",
        );
    }
    if element.message.is_some()
        && !is_scroll_container(element)
        && inherited_clip.is_some_and(|clip| {
            intersection(rect, clip).is_none_or(|visible| !approximately_same_rect(visible, rect))
        })
    {
        tree.diagnostic(
            DiagnosticKind::ClippedInteraction,
            id,
            format!("interactive rectangle {rect:?} is clipped by {inherited_clip:?}"),
        );
    }
    let unconstrained_content = match &element.kind {
        Kind::Text {
            value,
            scale,
            bold,
            line_height,
            max_lines,
            ..
        } => Some(measure_text(
            value,
            *scale,
            *bold,
            false,
            *line_height,
            *max_lines,
            f32::INFINITY,
        )),
        Kind::Image { image, .. } => Some(Size::new(image.width() as f32, image.height() as f32)),
        _ => None,
    };
    if unconstrained_content.is_some_and(|content| {
        content.width > rect.size.width + 0.01 || content.height > rect.size.height + 0.01
    }) {
        tree.diagnostic(
            DiagnosticKind::UnsatisfiedContent,
            id,
            format!(
                "intrinsic {:?} exceeds allocated {:?}",
                unconstrained_content.unwrap_or_default(),
                rect.size
            ),
        );
    }
    if matches!(&element.kind, Kind::Image { image, .. } if image.width() == 0 || image.height() == 0)
    {
        tree.diagnostic(
            DiagnosticKind::MissingAsset,
            id,
            "image has no pixels; using a bounded 16x16 fallback measurement",
        );
    }
    let mut child_indices = Vec::new();
    let foreground = element.style.foreground.or(inherited_foreground);
    let clips_descendants = element.style.overflow_x != Overflow::Visible
        || element.style.overflow_y != Overflow::Visible;
    let descendant_clip = if clips_descendants {
        Some(inherited_clip.map_or(rect, |parent| {
            intersection(parent, rect).unwrap_or_else(|| {
                Rect::new(
                    rect.origin.x.max(parent.origin.x),
                    rect.origin.y.max(parent.origin.y),
                    0.0,
                    0.0,
                )
            })
        }))
    } else {
        inherited_clip
    };
    match &element.kind {
        Kind::Text { .. }
        | Kind::StyledText { .. }
        | Kind::Image { .. }
        | Kind::Slider { .. }
        | Kind::Dropdown { .. } => {}
        Kind::Flex(axis) => {
            let scroll_rect = rect.inset(element.style.padding);
            let scrollable_y = *axis == Axis::Vertical
                && matches!(element.style.overflow_y, Overflow::Scroll | Overflow::Auto);
            let scrollable_x = *axis == Axis::Horizontal
                && matches!(element.style.overflow_x, Overflow::Scroll | Overflow::Auto);
            let intrinsic_content_width = if scrollable_x {
                element
                    .children
                    .iter()
                    .map(|child| {
                        measure_element(
                            child,
                            Constraints::loose(Size::new(f32::INFINITY, scroll_rect.size.height)),
                        )
                        .width
                    })
                    .sum::<f32>()
                    + element.style.gap * element.children.len().saturating_sub(1) as f32
            } else {
                scroll_rect.size.width
            };
            let horizontal_scrollbar_gutter = match element.style.overflow_x {
                Overflow::Scroll => SCROLLBAR_GUTTER,
                Overflow::Auto if intrinsic_content_width > scroll_rect.size.width + 0.01 => {
                    SCROLLBAR_GUTTER
                }
                _ => 0.0,
            }
            .min(scroll_rect.size.height);
            let content = Rect::new(
                scroll_rect.origin.x,
                scroll_rect.origin.y,
                scroll_rect.size.width,
                (scroll_rect.size.height - horizontal_scrollbar_gutter).max(0.0),
            );
            let minimum_total = element
                .children
                .iter()
                .map(|child| match axis {
                    Axis::Horizontal => child.style.min_width,
                    Axis::Vertical => child.style.min_height,
                })
                .sum::<f32>()
                + element.style.gap * element.children.len().saturating_sub(1) as f32;
            let available = match axis {
                Axis::Horizontal => content.size.width,
                Axis::Vertical => content.size.height,
            };
            if minimum_total > available + 0.01 {
                tree.diagnostic(
                    DiagnosticKind::FlexOverflow,
                    id,
                    format!("minimum total {minimum_total:.2} exceeds {available:.2}"),
                );
            }
            let intrinsic_content_height = if scrollable_y {
                element
                    .children
                    .iter()
                    .map(|child| {
                        measure_element(
                            child,
                            Constraints::loose(Size::new(content.size.width, f32::INFINITY)),
                        )
                        .height
                    })
                    .sum::<f32>()
                    + element.style.gap * element.children.len().saturating_sub(1) as f32
            } else {
                content.size.height
            };
            let content_extent = Size::new(
                intrinsic_content_width.max(content.size.width),
                intrinsic_content_height.max(content.size.height),
            );
            let maximum_offset_x = (content_extent.width - content.size.width).max(0.0);
            let scroll_offset_x = if scrollable_x {
                element.style.scroll_offset_x.clamp(0.0, maximum_offset_x)
            } else {
                0.0
            };
            if scrollable_x
                && (element.style.scroll_offset_x - scroll_offset_x).abs() > f32::EPSILON
            {
                tree.diagnostic(
                    DiagnosticKind::ScrollOffsetClamped,
                    id,
                    format!(
                        "horizontal offset {:.2} clamped to {:.2}",
                        element.style.scroll_offset_x, scroll_offset_x
                    ),
                );
            }
            let maximum_offset = (content_extent.height - content.size.height).max(0.0);
            let scroll_offset = if scrollable_y {
                let clamped = element.style.scroll_offset.clamp(0.0, maximum_offset);
                if (element.style.scroll_offset - clamped).abs() > f32::EPSILON {
                    tree.diagnostic(
                        DiagnosticKind::ScrollOffsetClamped,
                        id,
                        format!(
                            "offset {:.2} clamped to {:.2}",
                            element.style.scroll_offset, clamped
                        ),
                    );
                }
                let extent = ScrollExtent {
                    viewport: content.size,
                    content: content_extent,
                    offset_x: scroll_offset_x,
                    offset: clamped,
                };
                tree.resolved.nodes[node_index].scroll = Some(extent);
                tree.scrolls.push(ScrollRegion {
                    id: id.clone(),
                    message: element.message.clone(),
                    offset_mapper: element.message_mapper,
                    rect: scroll_rect,
                    clip: descendant_clip.unwrap_or(scroll_rect),
                    extent,
                });
                clamped
            } else {
                0.0
            };
            if scrollable_x {
                let extent = ScrollExtent {
                    viewport: content.size,
                    content: content_extent,
                    offset_x: scroll_offset_x,
                    offset: scroll_offset,
                };
                tree.resolved.nodes[node_index].scroll = Some(extent);
                tree.scrolls.push(ScrollRegion {
                    id: id.clone(),
                    message: element.message.clone(),
                    offset_mapper: element.message_mapper,
                    rect: scroll_rect,
                    clip: descendant_clip.unwrap_or(scroll_rect),
                    extent,
                });
            }
            let layout_content = Rect::new(
                content.origin.x - scroll_offset_x,
                content.origin.y - scroll_offset,
                content_extent.width,
                content_extent.height,
            );
            let child_bounds = flex_bounds(
                layout_content,
                *axis,
                element.style.gap,
                element.style.align_items,
                element.style.justify_content,
                &element.children,
            );
            let child_clip = if scrollable_x || scrollable_y {
                Some(descendant_clip.map_or(content, |parent| {
                    intersection(parent, content)
                        .unwrap_or_else(|| Rect::new(content.origin.x, content.origin.y, 0.0, 0.0))
                }))
            } else {
                descendant_clip
            };
            for (index, (child, bounds)) in element.children.iter().zip(child_bounds).enumerate() {
                let child_id = resolved_child_id(id, child, index);
                child_indices.push(layout_element(
                    child, &child_id, bounds, foreground, child_clip, tree,
                ));
            }
        }
        Kind::VerticalScroll { offset, .. } => {
            let requested_offset = *offset;
            let viewport = rect.inset(element.style.padding);
            let clip = descendant_clip.map_or(viewport, |parent| {
                intersection(parent, viewport)
                    .unwrap_or_else(|| Rect::new(viewport.origin.x, viewport.origin.y, 0.0, 0.0))
            });
            if let Some(child) = element.children.first() {
                let content_size = measure_element(
                    child,
                    Constraints::loose(Size::new(viewport.size.width, f32::INFINITY)),
                );
                let content_height = content_size.height.max(viewport.size.height);
                let offset =
                    requested_offset.clamp(0.0, (content_height - viewport.size.height).max(0.0));
                if (requested_offset - offset).abs() > f32::EPSILON {
                    tree.diagnostic(
                        DiagnosticKind::ScrollOffsetClamped,
                        id,
                        format!("offset {requested_offset:.2} clamped to {offset:.2}"),
                    );
                }
                let extent = ScrollExtent {
                    viewport: viewport.size,
                    content: Size::new(content_size.width.max(viewport.size.width), content_height),
                    offset_x: 0.0,
                    offset,
                };
                tree.resolved.nodes[node_index].scroll = Some(extent);
                if let Some(message) = &element.message {
                    tree.scrolls.push(ScrollRegion {
                        id: id.clone(),
                        message: Some(message.clone()),
                        offset_mapper: element.message_mapper,
                        rect: viewport,
                        clip,
                        extent,
                    });
                }
                child_indices.push(layout_element(
                    child,
                    &resolved_child_id(id, child, 0),
                    Rect::new(
                        viewport.origin.x,
                        viewport.origin.y - offset,
                        viewport.size.width,
                        content_height,
                    ),
                    foreground,
                    Some(clip),
                    tree,
                ));
            }
        }
        Kind::Grid { columns } => {
            let scroll_rect = rect.inset(element.style.padding);
            let horizontal_scrollbar_gutter =
                if matches!(element.style.overflow_x, Overflow::Scroll | Overflow::Auto) {
                    SCROLLBAR_GUTTER.min(scroll_rect.size.height)
                } else {
                    0.0
                };
            let content = Rect::new(
                scroll_rect.origin.x,
                scroll_rect.origin.y,
                scroll_rect.size.width,
                (scroll_rect.size.height - horizontal_scrollbar_gutter).max(0.0),
            );
            let contributions = element
                .children
                .iter()
                .map(|child| measure_element(child, Constraints::loose(content.size)))
                .collect::<Vec<_>>();
            let widths = resolve_grid_columns(
                columns,
                content.size.width,
                element.style.gap,
                &contributions,
                element.children.len(),
            );
            let column_count = widths.len().max(1);
            let measured = element
                .children
                .iter()
                .enumerate()
                .map(|(index, child)| {
                    measure_element(
                        child,
                        Constraints::loose(Size::new(
                            widths[index % column_count],
                            content.size.height,
                        )),
                    )
                })
                .collect::<Vec<_>>();
            tree.resolved.nodes[node_index]
                .grid_tracks
                .clone_from(&widths);
            tree.grids.push(ResolvedGrid {
                rect: content,
                columns: column_count,
            });
            let rows = element.children.len().div_ceil(column_count);
            let row_heights = (0..rows)
                .map(|row| {
                    measured[row * column_count..measured.len().min((row + 1) * column_count)]
                        .iter()
                        .map(|size| size.height)
                        .fold(0.0, f32::max)
                })
                .collect::<Vec<_>>();
            let content_extent = Size::new(
                (widths.iter().sum::<f32>()
                    + element.style.gap * widths.len().saturating_sub(1) as f32)
                    .max(content.size.width),
                (row_heights.iter().sum::<f32>()
                    + element.style.gap * rows.saturating_sub(1) as f32)
                    .max(content.size.height),
            );
            let scrollable_x =
                matches!(element.style.overflow_x, Overflow::Scroll | Overflow::Auto);
            let scrollable_y =
                matches!(element.style.overflow_y, Overflow::Scroll | Overflow::Auto);
            let offset_x = if scrollable_x {
                element
                    .style
                    .scroll_offset_x
                    .clamp(0.0, (content_extent.width - content.size.width).max(0.0))
            } else {
                0.0
            };
            let offset_y = if scrollable_y {
                element
                    .style
                    .scroll_offset
                    .clamp(0.0, (content_extent.height - content.size.height).max(0.0))
            } else {
                0.0
            };
            if scrollable_x && (offset_x - element.style.scroll_offset_x).abs() > f32::EPSILON
                || scrollable_y && (offset_y - element.style.scroll_offset).abs() > f32::EPSILON
            {
                tree.diagnostic(
                    DiagnosticKind::ScrollOffsetClamped,
                    id,
                    format!(
                        "grid offset ({:.2},{:.2}) clamped to ({offset_x:.2},{offset_y:.2})",
                        element.style.scroll_offset_x, element.style.scroll_offset
                    ),
                );
            }
            if scrollable_x || scrollable_y {
                let extent = ScrollExtent {
                    viewport: content.size,
                    content: content_extent,
                    offset_x,
                    offset: offset_y,
                };
                tree.resolved.nodes[node_index].scroll = Some(extent);
                tree.scrolls.push(ScrollRegion {
                    id: id.clone(),
                    message: element.message.clone(),
                    offset_mapper: element.message_mapper,
                    rect: scroll_rect,
                    clip: descendant_clip.unwrap_or(scroll_rect),
                    extent,
                });
            }
            for (index, child) in element.children.iter().enumerate() {
                let column = index % column_count;
                let row = index / column_count;
                let x = content.origin.x - offset_x
                    + widths[..column].iter().sum::<f32>()
                    + element.style.gap * column as f32;
                let y = content.origin.y - offset_y
                    + row_heights[..row].iter().sum::<f32>()
                    + element.style.gap * row as f32;
                child_indices.push(layout_element(
                    child,
                    &resolved_child_id(id, child, index),
                    Rect::new(x, y, widths[column], row_heights[row]),
                    foreground,
                    descendant_clip,
                    tree,
                ));
            }
        }
    }
    tree.resolved.nodes[node_index].children = child_indices;
    node_index
}

fn resolved_child_id<Message>(parent: &UiId, child: &Element<Message>, index: usize) -> UiId {
    child.id.as_ref().map_or_else(
        || parent.scoped(format!("#{index}")),
        |id| parent.scoped(id.as_str()),
    )
}

fn apply_transient_state<Message>(
    element: &mut Element<Message>,
    id: &UiId,
    state: &mut UiStateStore,
) {
    let owns_state = element.message.is_some()
        || element.text_mapper.is_some()
        || matches!(element.style.overflow_x, Overflow::Scroll | Overflow::Auto)
        || matches!(element.style.overflow_y, Overflow::Scroll | Overflow::Auto)
        || matches!(
            element.kind,
            Kind::VerticalScroll { .. } | Kind::Dropdown { .. }
        );
    if owns_state {
        let (scroll_offset_x, scroll_offset, scroll_at_end, dropdown_open) = {
            let transient = state.touch(id.clone());
            (
                transient.scroll_offset_x,
                transient.scroll_offset,
                transient.scroll_at_end,
                transient.dropdown_open,
            )
        };
        match &mut element.kind {
            Kind::VerticalScroll {
                offset,
                controlled: false,
            } => *offset = scroll_offset.max(0.0),
            Kind::Dropdown {
                expanded,
                options,
                overlay,
                ..
            } => {
                *expanded = dropdown_open;
                element.style.height = Length::Px(if *overlay {
                    30.0
                } else if *expanded {
                    42.0 + options.len() as f32 * 36.0
                } else {
                    42.0
                });
            }
            _ => {}
        }
        if element.text_mapper.is_some()
            && let Kind::Text {
                value,
                scale,
                bold,
                selection_x,
                caret_position,
                input_value,
                line_height,
                ..
            } = &mut element.kind
        {
            let initial = value.clone();
            let focused = state.focused() == Some(id);
            let caret_visible = state.caret_visible();
            let editor = state.editor(id.clone(), &initial);
            if editor.text() != initial {
                editor.set_text(initial);
            }
            *input_value = Some(editor.text().to_owned());
            *selection_x = focused
                .then(|| editor.selection())
                .flatten()
                .map(|selection| {
                    let width = |end| {
                        measure_text(
                            &editor.text()[..end],
                            *scale,
                            *bold,
                            false,
                            None,
                            Some(1),
                            f32::INFINITY,
                        )
                        .width
                    };
                    (width(selection.start), width(selection.end))
                });
            *caret_position = (focused && caret_visible).then(|| {
                let prefix = editor.display_caret_prefix();
                let (line_index, line) =
                    prefix
                        .rsplit_once('\n')
                        .map_or((0, prefix.as_str()), |(before, line)| {
                            (
                                before.bytes().filter(|byte| *byte == b'\n').count() + 1,
                                line,
                            )
                        });
                let x =
                    measure_text(line, *scale, *bold, false, None, Some(1), f32::INFINITY).width;
                let height = line_height.unwrap_or_else(|| text_font_size(*scale) * 1.3);
                Point {
                    x,
                    y: line_index as f32 * height,
                }
            });
            *value = editor.display_text_with_caret("");
        }
        if matches!(element.style.overflow_y, Overflow::Scroll | Overflow::Auto) {
            element.style.scroll_offset = if element.style.follow_scroll_end && scroll_at_end {
                f32::MAX
            } else {
                scroll_offset.max(0.0)
            };
        }
        if matches!(element.style.overflow_x, Overflow::Scroll | Overflow::Auto) {
            element.style.scroll_offset_x = scroll_offset_x.max(0.0);
        }
    }
    for (index, child) in element.children.iter_mut().enumerate() {
        let child_id = child.id.as_ref().map_or_else(
            || id.scoped(format!("#{index}")),
            |child_id| id.scoped(child_id.as_str()),
        );
        apply_transient_state(child, &child_id, state);
    }
}

fn resolve_grid_columns(
    definition: &GridColumnSpec,
    available: f32,
    gap: f32,
    children: &[Size],
    child_count: usize,
) -> Vec<f32> {
    let tracks = match definition {
        GridColumnSpec::Count(count) => {
            let count = (*count).max(1);
            if available.is_finite() {
                let width = ((available - gap.max(0.0) * count.saturating_sub(1) as f32)
                    / count as f32)
                    .max(0.0);
                return vec![width; count];
            }
            let width = children.iter().map(|size| size.width).fold(0.0, f32::max);
            return vec![width; count];
        }
        GridColumnSpec::Tracks(tracks) => {
            if let [Track::AutoFit(track)] = tracks.as_slice() {
                let minimum = track_minimum(track).max(1.0);
                let count = if available.is_finite() {
                    (((available + gap.max(0.0)) / (minimum + gap.max(0.0))).floor() as usize)
                        .max(1)
                        .min(child_count.max(1))
                } else {
                    child_count.max(1)
                };
                vec![track.as_ref().clone(); count]
            } else {
                expand_tracks(tracks)
            }
        }
        GridColumnSpec::AutoFit(track) => {
            let minimum = track_minimum(track).max(1.0);
            let count = if available.is_finite() {
                (((available + gap.max(0.0)) / (minimum + gap.max(0.0))).floor() as usize)
                    .max(1)
                    .min(child_count.max(1))
            } else {
                child_count.max(1)
            };
            vec![track.clone(); count]
        }
    };
    if tracks.is_empty() {
        return vec![available.max(0.0)];
    }
    let mut widths = vec![0.0; tracks.len()];
    let mut fractions = vec![0.0; tracks.len()];
    for (index, track) in tracks.iter().enumerate() {
        let contribution = children
            .iter()
            .skip(index)
            .step_by(tracks.len())
            .map(|size| size.width)
            .fold(0.0, f32::max);
        let (base, fraction) = resolve_track(track, contribution);
        widths[index] = if available.is_finite() || fraction == 0.0 {
            base
        } else {
            base.max(contribution)
        };
        fractions[index] = fraction;
    }
    if available.is_finite() {
        let gap_total = gap.max(0.0) * tracks.len().saturating_sub(1) as f32;
        let free = (available - gap_total - widths.iter().sum::<f32>()).max(0.0);
        let fraction_total = fractions.iter().sum::<f32>();
        if fraction_total > 0.0 {
            for ((width, fraction), track) in widths.iter_mut().zip(&fractions).zip(&tracks) {
                if *fraction > 0.0 {
                    *width += free * *fraction / fraction_total;
                    if let Track::MinMax(_, max) = track
                        && let Track::Px(maximum) = max.as_ref()
                    {
                        *width = width.min(maximum.max(0.0));
                    }
                }
            }
        }
    }
    widths
}

fn expand_tracks(tracks: &[Track]) -> Vec<Track> {
    let mut expanded = Vec::new();
    for track in tracks {
        match track {
            Track::Repeat(count, track) => {
                expanded.extend(std::iter::repeat_n(track.as_ref().clone(), *count));
            }
            Track::AutoFit(track) => expanded.push(track.as_ref().clone()),
            _ => expanded.push(track.clone()),
        }
    }
    expanded
}

fn track_minimum(track: &Track) -> f32 {
    match track {
        Track::Px(value) => value.max(0.0),
        Track::MinMax(min, _) => track_minimum(min),
        Track::Repeat(_, track) | Track::AutoFit(track) => track_minimum(track),
        Track::Auto | Track::Fraction(_) => 0.0,
    }
}

fn resolve_track(track: &Track, contribution: f32) -> (f32, f32) {
    match track {
        Track::Px(value) => (value.max(0.0), 0.0),
        Track::Auto => (contribution.max(0.0), 0.0),
        Track::Fraction(fraction) => (0.0, fraction.max(0.0)),
        Track::MinMax(min, max) => {
            let minimum = match min.as_ref() {
                Track::Auto => contribution,
                other => track_minimum(other),
            };
            match max.as_ref() {
                Track::Fraction(fraction) => (minimum, fraction.max(0.0)),
                Track::Px(maximum) => (minimum.min(maximum.max(0.0)), 0.0),
                Track::Auto => (minimum.max(contribution), 0.0),
                other => (minimum.max(track_minimum(other)), 0.0),
            }
        }
        Track::Repeat(_, track) | Track::AutoFit(track) => resolve_track(track, contribution),
    }
}

fn flex_bounds<Message>(
    content: Rect,
    axis: Axis,
    gap: f32,
    align_items: Align,
    justify: Justify,
    children: &[Element<Message>],
) -> Vec<Rect> {
    if children.is_empty() {
        return Vec::new();
    }
    let measured = children
        .iter()
        .map(|child| {
            measure_element(
                child,
                Constraints::loose(match axis {
                    Axis::Horizontal => Size::new(f32::INFINITY, content.size.height),
                    Axis::Vertical => Size::new(content.size.width, f32::INFINITY),
                }),
            )
        })
        .collect::<Vec<_>>();
    let items = children
        .iter()
        .zip(&measured)
        .map(|(child, measured)| {
            let (intrinsic, min, max) = match axis {
                Axis::Horizontal => (measured.width, child.style.min_width, child.style.max_width),
                Axis::Vertical => (
                    measured.height,
                    child.style.min_height,
                    child.style.max_height,
                ),
            };
            let parent = match axis {
                Axis::Horizontal => content.size.width,
                Axis::Vertical => content.size.height,
            };
            let preferred = child.style.basis.resolve(
                parent,
                match axis {
                    Axis::Horizontal => child.style.width.resolve(parent, intrinsic),
                    Axis::Vertical => child.style.height.resolve(parent, intrinsic),
                },
            );
            FlexItem::flex(
                preferred,
                min.max(0.0),
                max.max(min.max(0.0)),
                child.style.grow,
                child.style.shrink,
            )
        })
        .collect::<Vec<_>>();
    let mut rects = layout_flex(content, axis, gap.max(0.0), &items);
    let resolved_measured = children
        .iter()
        .zip(&rects)
        .map(|(child, rect)| match axis {
            Axis::Horizontal => measure_element(
                child,
                Constraints::loose(Size::new(rect.size.width, f32::INFINITY)),
            ),
            Axis::Vertical => measure_element(
                child,
                Constraints::loose(Size::new(content.size.width, rect.size.height)),
            ),
        })
        .collect::<Vec<_>>();
    let occupied = rects
        .iter()
        .map(|rect| match axis {
            Axis::Horizontal => rect.size.width,
            Axis::Vertical => rect.size.height,
        })
        .sum::<f32>()
        + gap.max(0.0) * rects.len().saturating_sub(1) as f32;
    let available = match axis {
        Axis::Horizontal => content.size.width,
        Axis::Vertical => content.size.height,
    };
    let free = (available - occupied).max(0.0);
    let (leading, extra_gap) = match justify {
        Justify::Start => (0.0, 0.0),
        Justify::Center => (free / 2.0, 0.0),
        Justify::End => (free, 0.0),
        Justify::SpaceBetween if rects.len() > 1 => (0.0, free / (rects.len() - 1) as f32),
        Justify::SpaceAround => {
            let space = free / rects.len() as f32;
            (space / 2.0, space)
        }
        Justify::SpaceEvenly => {
            let space = free / (rects.len() + 1) as f32;
            (space, space)
        }
        _ => (0.0, 0.0),
    };
    let shared_baseline = if axis == Axis::Horizontal {
        children
            .iter()
            .zip(&resolved_measured)
            .filter(|(child, _)| child.style.align_self.unwrap_or(align_items) == Align::Baseline)
            .map(|(_, size)| size.height * 0.8)
            .fold(0.0, f32::max)
    } else {
        0.0
    };
    for (index, ((rect, child), measured)) in rects
        .iter_mut()
        .zip(children)
        .zip(&resolved_measured)
        .enumerate()
    {
        let main_offset = leading + index as f32 * extra_gap;
        let alignment = child.style.align_self.unwrap_or(align_items);
        match axis {
            Axis::Horizontal => {
                rect.origin.x += main_offset;
                let cross = measured.height.min(content.size.height);
                if alignment != Align::Stretch {
                    rect.size.height = cross;
                    rect.origin.y += match alignment {
                        Align::Center => (content.size.height - cross) / 2.0,
                        Align::End => content.size.height - cross,
                        Align::Baseline => shared_baseline - cross * 0.8,
                        _ => 0.0,
                    };
                }
            }
            Axis::Vertical => {
                rect.origin.y += main_offset;
                let cross = measured.width.min(content.size.width);
                if alignment != Align::Stretch {
                    rect.size.width = cross;
                    rect.origin.x += match alignment {
                        Align::Center => (content.size.width - cross) / 2.0,
                        Align::End => content.size.width - cross,
                        _ => 0.0,
                    };
                }
            }
        }
    }
    rects
}

fn explicit_px(length: Length) -> Option<f32> {
    match length {
        Length::Px(value) => Some(value.max(0.0)),
        _ => None,
    }
}

fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.origin.x
        && point.x < rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y < rect.origin.y + rect.size.height
}

#[cfg(test)]
mod tests;
