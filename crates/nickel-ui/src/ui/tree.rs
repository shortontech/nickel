use super::*;
use std::{
    collections::BTreeSet,
    hash::{DefaultHasher, Hash, Hasher},
};

#[derive(Clone, Debug)]
struct HitRegion<Message> {
    id: UiId,
    rect: Rect,
    target_bounds: Rect,
    message: Option<Message>,
    message_mapper: Option<fn(f32) -> Message>,
    drag_mapper: Option<fn(Message, DragGesture) -> Message>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ControllerDirection {
    Up,
    Down,
    Left,
    Right,
}

impl From<ControllerDirection> for NavigationDirection {
    fn from(direction: ControllerDirection) -> Self {
        match direction {
            ControllerDirection::Up => Self::Up,
            ControllerDirection::Down => Self::Down,
            ControllerDirection::Left => Self::Left,
            ControllerDirection::Right => Self::Right,
        }
    }
}

#[derive(Clone, Debug)]
struct MessageRegion<Message> {
    id: UiId,
    navigation_owner: Option<UiId>,
    rect: Rect,
    message: Message,
    message_mapper: Option<fn(f32) -> Message>,
}

struct OverlayMenuLevel<'a, Message> {
    id: &'a UiId,
    node_index: usize,
    rect: Rect,
    items: &'a [crate::OverlayMenuItem<Message>],
    depth: usize,
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
    secure: bool,
}

#[derive(Clone, Debug)]
struct TextCommandRegion {
    id: UiId,
    editor: UiId,
    command: crate::TextEditCommand,
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

fn selection_document_generation(document: &SelectionDocument) -> u64 {
    let mut hasher = DefaultHasher::new();
    for run in document.runs() {
        run.id.hash(&mut hasher);
        run.text.hash(&mut hasher);
        (run.boundary_before as u8).hash(&mut hasher);
    }
    hasher.finish()
}

fn document_selection_generation(selection: &crate::DocumentSelection) -> u64 {
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
    scrollbar: crate::ScrollbarPalette,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScrollbarAxis {
    Horizontal,
    Vertical,
}

// Shared scrollbar geometry. Keep the painted chrome discoverable while giving
// it a larger edge-aligned acquisition target than its visual footprint.
const SCROLLBAR_THICKNESS: f32 = 10.0;
const SCROLLBAR_HIT_THICKNESS: f32 = 20.0;
const SCROLLBAR_INSET: f32 = 3.0;
const SCROLLBAR_MIN_THUMB: f32 = 32.0;
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

fn scrollbar_hit_rect<Message>(
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

fn scrollbar_thumb_hit_rect<Message>(
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

fn configure_scroll_semantics(node: &mut ResolvedNode, extent: ScrollExtent) {
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedGrid {
    pub rect: Rect,
    pub columns: usize,
}

/// Backend-neutral geometry exposed to accessibility adapters.
#[derive(Clone, Debug, PartialEq)]
pub struct AccessibilityNode {
    pub id: UiId,
    pub parent: Option<UiId>,
    pub component: &'static str,
    pub rect: Rect,
    pub interactive: bool,
    pub label: Option<String>,
    pub description: Option<String>,
    pub role: Option<String>,
    pub state: Option<String>,
    pub controls: Option<UiId>,
    pub semantic_role: Option<SemanticRole>,
    pub actions: Vec<ActionKind>,
    pub enabled: bool,
    pub focused: bool,
    pub controller_selected: bool,
    pub navigation_depth: usize,
    pub value: Option<SemanticValueSnapshot>,
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
    MissingAccessibleName,
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
    pub navigation_scope: Option<crate::NavigationScope>,
    pub adjustment_step: f32,
    pub controller_value: Option<f32>,
    pub accessibility_label: Option<String>,
    pub accessibility_description: Option<String>,
    pub accessibility_role: Option<String>,
    pub accessibility_state: Option<String>,
    pub accessibility_controls: Option<UiId>,
    pub accessibility_hidden: bool,
    pub semantic_role: Option<SemanticRole>,
    pub semantic_actions: Vec<ActionKind>,
    pub semantic_value: Option<SemanticValueSnapshot>,
    pub children: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticNodeSnapshot {
    pub id: UiId,
    pub parent: Option<UiId>,
    pub bounds: Rect,
    pub role: Option<SemanticRole>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled: bool,
    pub focused: bool,
    pub controller_selected: bool,
    /// Whether this semantic node is also an enterable controller-navigation scope.
    pub navigation_scope: bool,
    pub navigation_depth: usize,
    pub controls: Option<UiId>,
    pub actions: Vec<ActionKind>,
    pub value: Option<SemanticValueSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticActionError {
    MissingTarget,
    AmbiguousTarget,
    ActionUnavailable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveHitRoute {
    pub target: UiId,
    pub bounds: Rect,
    pub point: Point,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedLayout {
    nodes: Vec<ResolvedNode>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticTarget {
    pub id: UiId,
    pub bounds: Rect,
    pub role: Option<SemanticRole>,
    pub name: Option<String>,
    pub interactive: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticSelector {
    Id(UiId),
    Role(SemanticRole),
    Name(String),
    RoleAndName { role: SemanticRole, name: String },
    Action(ActionKind),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticQueryError {
    Missing,
    Ambiguous { matches: usize },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameResourceDiagnostics {
    pub node_count: usize,
    pub paint_primitive_count: usize,
    pub hit_target_count: usize,
    pub message_binding_count: usize,
    pub accessibility_node_count: usize,
    /// Lower-bound estimate covering owned vector storage and accessibility
    /// strings. Message payloads and shared image pixels belong to their model
    /// or resource cache and are intentionally not guessed here.
    pub estimated_retained_bytes: usize,
    pub retained_build_scratch_bytes: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DiagnosticMode {
    #[default]
    Disabled,
    Collect,
}

pub struct FrameRequest<'state> {
    pub viewport: Rect,
    pub state: &'state mut UiStateStore,
    pub diagnostics: DiagnosticMode,
}

impl<'state> FrameRequest<'state> {
    pub fn new(viewport: Rect, state: &'state mut UiStateStore) -> Self {
        Self {
            viewport,
            state,
            diagnostics: DiagnosticMode::Disabled,
        }
    }

    pub fn diagnostics(mut self, mode: DiagnosticMode) -> Self {
        self.diagnostics = mode;
        self
    }
}

impl ResolvedLayout {
    pub fn nodes(&self) -> &[ResolvedNode] {
        &self.nodes
    }

    pub fn find(&self, id: &UiId) -> Option<&ResolvedNode> {
        self.nodes.iter().find(|node| &node.id == id)
    }

    fn find_mut(&mut self, id: &UiId) -> Option<&mut ResolvedNode> {
        self.nodes.iter_mut().find(|node| &node.id == id)
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
pub struct UiFrame<Message = String> {
    commands: Vec<PaintCommand>,
    overlay_commands: Vec<PaintCommand>,
    hits: Vec<HitRegion<Message>>,
    overlay_hits: Vec<HitRegion<Message>>,
    messages: Vec<MessageRegion<Message>>,
    context_messages: Vec<MessageRegion<Message>>,
    text_inputs: Vec<TextInputRegion<Message>>,
    text_commands: Vec<TextCommandRegion>,
    selection_regions: Vec<SelectionRegionLayout>,
    selection_paints: HashMap<usize, Vec<Rect>>,
    scrolls: Vec<ScrollRegion<Message>>,
    grids: Vec<ResolvedGrid>,
    accessibility: Vec<AccessibilityNode>,
    semantic_role_name_index: HashMap<(SemanticRole, String), Vec<usize>>,
    semantic_parents: Vec<Option<usize>>,
    navigation_depths: Vec<usize>,
    resolved: ResolvedLayout,
    diagnostics: Vec<LayoutDiagnostic>,
    diagnostic_keys: HashSet<(DiagnosticKind, UiId)>,
    seen_ids: HashSet<UiId>,
    diagnostics_enabled: bool,
    viewport: Rect,
    overlay_invokers: Vec<(UiId, crate::OverlayId)>,
    active_overlay: Option<(crate::OverlayId, Rect)>,
    active_overlay_dismiss: Option<crate::DismissPolicy>,
}

impl<Message> Default for UiFrame<Message> {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
            overlay_commands: Vec::new(),
            hits: Vec::new(),
            overlay_hits: Vec::new(),
            messages: Vec::new(),
            context_messages: Vec::new(),
            text_inputs: Vec::new(),
            text_commands: Vec::new(),
            selection_regions: Vec::new(),
            selection_paints: HashMap::new(),
            scrolls: Vec::new(),
            grids: Vec::new(),
            accessibility: Vec::new(),
            semantic_role_name_index: HashMap::new(),
            semantic_parents: Vec::new(),
            navigation_depths: Vec::new(),
            resolved: ResolvedLayout::default(),
            diagnostics: Vec::new(),
            diagnostic_keys: HashSet::new(),
            seen_ids: HashSet::new(),
            diagnostics_enabled: false,
            viewport: Rect::new(0.0, 0.0, 0.0, 0.0),
            overlay_invokers: Vec::new(),
            active_overlay: None,
            active_overlay_dismiss: None,
        }
    }
}

impl<Message: Clone> UiFrame<Message> {
    fn drag_message(&self, id: &UiId, phase: DragPhase, position: Point) -> Option<Message> {
        let hit = self.hits.iter().rev().find(|hit| &hit.id == id)?;
        Some((hit.drag_mapper?)(
            hit.message.clone()?,
            DragGesture {
                phase,
                position,
                bounds: hit.target_bounds,
            },
        ))
    }

    fn cancelled_drag_message(&self, state: &UiStateStore) -> Option<Message> {
        let captured = state.captured()?;
        let bounds = self
            .hits
            .iter()
            .rev()
            .find(|hit| &hit.id == captured)?
            .target_bounds;
        self.drag_message(captured, DragPhase::Cancelled, bounds.origin)
    }

    pub(crate) fn resolve_stable_target(&self, requested: &UiId) -> Option<UiId> {
        let suffix = format!("/{}", requested.as_str());
        let mut matches = self
            .resolved
            .nodes
            .iter()
            .filter(|node| &node.id == requested || node.id.as_str().ends_with(&suffix))
            .map(|node| node.id.clone());
        let target = matches.next()?;
        matches.next().is_none().then_some(target)
    }

    pub(crate) fn contains_target(&self, id: &UiId) -> bool {
        self.resolved.find(id).is_some()
            || self.hits.iter().any(|region| &region.id == id)
            || self.messages.iter().any(|region| &region.id == id)
    }
    /// Adds the visual marquee for an application-owned image selection.
    ///
    /// Selection geometry remains application state, while display-list
    /// construction stays owned by the resolved frame. The marquee is
    /// intentionally non-interactive; callers route drag gestures through the
    /// semantic image/selection surface that produced the rectangle.
    pub fn selection_marquee(&mut self, rect: Rect, color: Color, width: f32) {
        self.selection_marquee_layer(rect, None, color, width);
    }

    pub fn selection_marquee_layer(
        &mut self,
        rect: Rect,
        fill: Option<Color>,
        stroke: Color,
        width: f32,
    ) {
        if rect.size.width <= 0.0 || rect.size.height <= 0.0 || width <= 0.0 {
            return;
        }
        if let Some(color) = fill {
            self.commands
                .push(PaintCommand::OverlayFill { rect, color });
        }
        self.commands.push(PaintCommand::OverlayStroke {
            rect,
            color: stroke,
            width,
        });
    }

    pub fn present_transient_surface(
        &mut self,
        state: &mut UiStateStore,
        mut surface: crate::TransientSurface,
    ) -> Result<(), SemanticActionError> {
        let requested = surface.anchor.id().clone();
        let suffix = format!("/{}", requested.as_str());
        let mut matches = self
            .resolved
            .nodes
            .iter()
            .filter(|node| node.id == requested || node.id.as_str().ends_with(&suffix))
            .map(|node| node.id.clone());
        let target = matches.next().ok_or(SemanticActionError::MissingTarget)?;
        if matches.next().is_some() {
            return Err(SemanticActionError::AmbiguousTarget);
        }
        surface.anchor = surface.anchor.with_resolved_target(target);
        self.overlay_invokers
            .push((surface.anchor.id().clone(), surface.id.clone()));
        if state.open_overlay_id() != Some(&surface.id) {
            return Ok(());
        }
        let anchor_node = self
            .resolved
            .find(surface.anchor.id())
            .ok_or(SemanticActionError::MissingTarget)?
            .allocated;
        let rect = crate::place_transient(
            surface.anchor.rect(anchor_node),
            surface.logical_size,
            self.viewport,
            surface.placement,
            surface.collision,
            surface.direction,
            surface.scale,
        );
        let role = Some(match surface.kind {
            crate::TransientKind::Dialog => SemanticRole::Dialog,
            crate::TransientKind::Popover => SemanticRole::Popover,
            crate::TransientKind::Tooltip => SemanticRole::Tooltip,
            crate::TransientKind::ContextMenu => SemanticRole::Menu,
        });
        self.commands.push(PaintCommand::OverlayFill {
            rect,
            color: surface.style.background,
        });
        self.commands.push(PaintCommand::OverlayStroke {
            rect,
            color: surface.style.border,
            width: 1.0,
        });
        let index = self.resolved.nodes.len();
        self.resolved.nodes.push(ResolvedNode {
            component: "TransientSurface",
            id: surface.id.as_ui_id().clone(),
            source: None,
            allocated: rect,
            padding_box: rect,
            border_box: rect,
            content: rect,
            constraints: Constraints::tight(rect.size),
            preferred: rect.size,
            flex_basis: Length::Auto,
            flex_grow: 0.0,
            flex_shrink: 0.0,
            clip: None,
            scroll: None,
            grid_tracks: Vec::new(),
            hit_stack: None,
            interaction: InteractionState::default(),
            navigation_scope: Some(crate::NavigationScope::group()),
            adjustment_step: 0.05,
            controller_value: None,
            accessibility_label: surface
                .accessible_name
                .clone()
                .or_else(|| Some(format!("{:?}", surface.kind))),
            accessibility_description: None,
            accessibility_role: role.map(|role| role.as_str().into()),
            accessibility_state: Some("expanded".into()),
            accessibility_controls: Some(surface.anchor.id().clone()),
            accessibility_hidden: false,
            semantic_role: role,
            semantic_actions: vec![ActionKind::Dismiss, ActionKind::Cancel],
            semantic_value: None,
            children: Vec::new(),
        });
        if let Some(root) = self.resolved.nodes.first_mut() {
            root.children.push(index);
        }
        self.active_overlay = Some((surface.id, rect));
        self.active_overlay_dismiss = Some(surface.dismiss);
        Ok(())
    }

    pub fn present_transient_content(
        &mut self,
        state: &mut UiStateStore,
        surface: crate::TransientSurface,
        mut content: Element<Message>,
    ) -> Result<(), SemanticActionError> {
        let overlay = surface.id.clone();
        self.present_transient_surface(state, surface)?;
        if self.active_overlay.as_ref().map(|(id, _)| id) != Some(&overlay) {
            return Ok(());
        }
        let Some((_, rect)) = self.active_overlay.clone() else {
            return Ok(());
        };
        let Some(parent) = self.node_index(overlay.as_ui_id()) else {
            return Err(SemanticActionError::MissingTarget);
        };
        let content_rect = Rect::new(
            rect.origin.x + 12.0,
            rect.origin.y + 12.0,
            (rect.size.width - 24.0).max(0.0),
            (rect.size.height - 24.0).max(0.0),
        );
        let content_id = overlay.as_ui_id().scoped("content");
        apply_transient_state(&mut content, &content_id, state);
        let index = layout_element(&content, &content_id, content_rect, None, Some(rect), self);
        self.resolved.nodes[parent].children.push(index);
        emit_element(&content, index, None, self);
        for node in &self.resolved.nodes[index..] {
            state.touch(node.id.clone());
        }
        Ok(())
    }

    pub(crate) fn finalize_transient_layers(&mut self, state: &UiStateStore) {
        self.apply_interaction_state(state);
        self.emit_accessibility_geometry();
        self.validate_clip_commands();
    }

    pub(crate) fn reconcile_transient_focus(&self, state: &mut UiStateStore) {
        let Some((overlay, _)) = &self.active_overlay else {
            return;
        };
        if let Some(selected) = state.navigation().controller_selected().cloned()
            && self.is_descendant_or_self(overlay.as_ui_id(), &selected)
        {
            state.set_focus(Some(selected));
        }
    }

    fn emit_overlay_menu_items(
        &mut self,
        state: &UiStateStore,
        menu: &crate::OverlayMenu<Message>,
        level: OverlayMenuLevel<'_, Message>,
    ) -> Option<Rect> {
        let OverlayMenuLevel {
            id: menu_id,
            node_index: menu_index,
            rect,
            items,
            depth,
        } = level;
        let belongs_to = |ancestor: &UiId, candidate: &UiId| {
            candidate == ancestor
                || candidate
                    .as_str()
                    .strip_prefix(ancestor.as_str())
                    .is_some_and(|suffix| suffix.starts_with('/'))
        };
        let hovered = state
            .hovered()
            .filter(|id| belongs_to(menu.id.as_ui_id(), id));
        let authority = hovered.or_else(|| {
            (state.input_modality() != InputModality::Pointer)
                .then(|| {
                    state
                        .navigation()
                        .controller_selected()
                        .or_else(|| state.focused())
                })
                .flatten()
        });
        let active_submenu = items.iter().enumerate().find(|(_, item)| {
            if item.children.is_empty() {
                return false;
            }
            let id = menu_id.scoped(item.id.as_str());
            authority.is_some_and(|selected| belongs_to(&id, selected))
        });

        let mut submenu = None;
        for (index, item) in items.iter().enumerate() {
            let item_rect = Rect::new(
                rect.origin.x + menu.padding.left,
                rect.origin.y + menu.padding.top + index as f32 * (menu.row_height + menu.row_gap),
                rect.size.width - menu.padding.left - menu.padding.right,
                menu.row_height,
            );
            let id = menu_id.scoped(item.id.as_str());
            let enabled =
                item.action.is_some() || item.text_command.is_some() || !item.children.is_empty();
            let interaction = InteractionState {
                interactive: enabled,
                focused: state.focused() == Some(&id),
                hovered: state.hovered() == Some(&id),
                pressed: state.pressed() == Some(&id),
                controller_selected: state.navigation().controller_selected() == Some(&id),
                ..InteractionState::default()
            };
            let interaction_color = if interaction.pressed {
                menu.item_pressed
            } else if interaction.hovered {
                menu.item_hover
            } else if interaction.focused || interaction.controller_selected {
                menu.item_selected
            } else {
                None
            };
            if let Some(color) = interaction_color {
                self.commands.push(PaintCommand::RoundedFill {
                    rect: item_rect,
                    color,
                    radius: menu.item_radius,
                });
            }
            let text_bounds = item_rect.inset(Insets::all(8.0));
            self.commands.push(PaintCommand::Text {
                bounds: text_bounds,
                text: item.label.clone(),
                scale: menu.text_scale,
                color: menu.foreground,
                align: menu.text_align,
                bold: false,
                wrap: false,
            });
            if !item.children.is_empty() {
                self.commands.push(PaintCommand::Text {
                    bounds: text_bounds,
                    text: match menu.direction {
                        crate::ReadingDirection::LeftToRight => "›",
                        crate::ReadingDirection::RightToLeft => "‹",
                    }
                    .into(),
                    scale: menu.text_scale,
                    color: menu.foreground,
                    align: match menu.direction {
                        crate::ReadingDirection::LeftToRight => TextAlign::End,
                        crate::ReadingDirection::RightToLeft => TextAlign::Start,
                    },
                    bold: false,
                    wrap: false,
                });
            }
            let item_index = self.resolved.nodes.len();
            self.resolved.nodes.push(ResolvedNode {
                component: "MenuItem",
                id: id.clone(),
                source: None,
                allocated: item_rect,
                padding_box: item_rect,
                border_box: item_rect,
                content: text_bounds,
                constraints: Constraints::tight(item_rect.size),
                preferred: item_rect.size,
                flex_basis: Length::Auto,
                flex_grow: 0.0,
                flex_shrink: 0.0,
                clip: Some(rect),
                scroll: None,
                grid_tracks: Vec::new(),
                hit_stack: Some(self.hits.len()),
                interaction,
                navigation_scope: (!item.children.is_empty()).then(|| {
                    crate::NavigationScope::group().traversal(crate::NavigationTraversal::Vertical)
                }),
                adjustment_step: 0.05,
                controller_value: None,
                accessibility_label: Some(item.label.clone()),
                accessibility_description: None,
                accessibility_role: Some(SemanticRole::MenuItem.as_str().into()),
                accessibility_state: None,
                accessibility_controls: (!item.children.is_empty()).then(|| id.scoped("submenu")),
                accessibility_hidden: false,
                semantic_role: Some(SemanticRole::MenuItem),
                semantic_actions: enabled
                    .then_some(ActionKind::Activate)
                    .into_iter()
                    .collect(),
                semantic_value: None,
                children: Vec::new(),
            });
            self.resolved.nodes[menu_index].children.push(item_index);
            self.hits.push(HitRegion {
                id: id.clone(),
                rect: item_rect,
                target_bounds: item_rect,
                message: item.action.clone(),
                message_mapper: None,
                drag_mapper: None,
            });
            if let Some(message) = item.action.clone() {
                self.messages.push(MessageRegion {
                    id: id.clone(),
                    navigation_owner: Some(menu_id.clone()),
                    rect: item_rect,
                    message,
                    message_mapper: None,
                });
            }
            if let Some(command) = item.text_command {
                self.text_commands.push(TextCommandRegion {
                    id: id.clone(),
                    editor: menu.anchor.id().clone(),
                    command,
                });
            }
            self.accessibility.push(AccessibilityNode {
                id: id.clone(),
                parent: Some(menu_id.clone()),
                component: "MenuItem",
                rect: item_rect,
                interactive: enabled,
                label: Some(item.label.clone()),
                description: None,
                role: Some("menuitem".into()),
                state: None,
                controls: (!item.children.is_empty()).then(|| id.scoped("submenu")),
                semantic_role: Some(SemanticRole::MenuItem),
                actions: enabled
                    .then_some(ActionKind::Activate)
                    .into_iter()
                    .collect(),
                enabled,
                focused: interaction.focused,
                controller_selected: interaction.controller_selected,
                navigation_depth: depth,
                value: None,
            });
            if active_submenu.is_some_and(|(active, _)| active == index) {
                submenu = Some((id, item_index, item_rect, item.children.as_slice()));
            }
        }

        let (parent_id, parent_index, anchor, children) = submenu?;
        let submenu_id = parent_id.scoped("submenu");
        let height = menu.padding.top
            + menu.row_height * children.len() as f32
            + menu.row_gap * children.len().saturating_sub(1) as f32
            + menu.padding.bottom;
        let submenu_rect = crate::place_transient(
            anchor,
            Size {
                width: menu.width,
                height,
            },
            self.viewport,
            crate::OverlayPlacement::After,
            menu.collision,
            menu.direction,
            1.0,
        );
        self.commands.push(PaintCommand::RoundedFill {
            rect: submenu_rect,
            color: menu.background,
            radius: menu.radius,
        });
        if menu.border_width > 0.0 {
            self.commands.push(PaintCommand::OverlayStroke {
                rect: submenu_rect.inset(Insets::all(menu.border_width / 2.0)),
                color: menu.border,
                width: menu.border_width,
            });
        }
        let submenu_index = self.resolved.nodes.len();
        self.resolved.nodes.push(ResolvedNode {
            component: "Menu",
            id: submenu_id.clone(),
            source: None,
            allocated: submenu_rect,
            padding_box: submenu_rect,
            border_box: submenu_rect,
            content: submenu_rect.inset(menu.padding),
            constraints: Constraints::tight(submenu_rect.size),
            preferred: submenu_rect.size,
            flex_basis: Length::Auto,
            flex_grow: 0.0,
            flex_shrink: 0.0,
            clip: None,
            scroll: None,
            grid_tracks: Vec::new(),
            hit_stack: None,
            interaction: InteractionState::default(),
            navigation_scope: Some(
                crate::NavigationScope::group().traversal(crate::NavigationTraversal::Vertical),
            ),
            adjustment_step: 0.05,
            controller_value: None,
            accessibility_label: None,
            accessibility_description: None,
            accessibility_role: Some(SemanticRole::Menu.as_str().into()),
            accessibility_state: None,
            accessibility_controls: None,
            accessibility_hidden: false,
            semantic_role: Some(SemanticRole::Menu),
            semantic_actions: Vec::new(),
            semantic_value: None,
            children: Vec::with_capacity(children.len()),
        });
        self.resolved.nodes[parent_index]
            .children
            .push(submenu_index);
        self.accessibility.push(AccessibilityNode {
            id: submenu_id.clone(),
            parent: Some(parent_id),
            component: "Menu",
            rect: submenu_rect,
            interactive: false,
            label: None,
            description: None,
            role: Some(SemanticRole::Menu.as_str().into()),
            state: None,
            controls: None,
            semantic_role: Some(SemanticRole::Menu),
            actions: Vec::new(),
            enabled: true,
            focused: false,
            controller_selected: false,
            navigation_depth: depth + 1,
            value: None,
        });
        let descendant = self.emit_overlay_menu_items(
            state,
            menu,
            OverlayMenuLevel {
                id: &submenu_id,
                node_index: submenu_index,
                rect: submenu_rect,
                items: children,
                depth: depth + 1,
            },
        );
        Some(descendant.map_or(submenu_rect, |child| {
            let left = submenu_rect.origin.x.min(child.origin.x);
            let top = submenu_rect.origin.y.min(child.origin.y);
            let right = (submenu_rect.origin.x + submenu_rect.size.width)
                .max(child.origin.x + child.size.width);
            let bottom = (submenu_rect.origin.y + submenu_rect.size.height)
                .max(child.origin.y + child.size.height);
            Rect::new(left, top, right - left, bottom - top)
        }))
    }

    /// Registers and, when open, emits a menu into this frame's authoritative
    /// overlay paint and hit stacks.
    pub fn present_menu(
        &mut self,
        state: &mut UiStateStore,
        mut menu: crate::OverlayMenu<Message>,
    ) -> Result<(), SemanticActionError> {
        let requested = menu.anchor.id().clone();
        let suffix = format!("/{}", requested.as_str());
        let mut matches = self
            .resolved
            .nodes
            .iter()
            .filter(|node| node.id == requested || node.id.as_str().ends_with(&suffix))
            .map(|node| node.id.clone());
        let Some(target) = matches.next() else {
            if state.open_overlay_id() == Some(&menu.id) {
                state.dismiss_overlay(crate::DismissReason::Cancel);
            }
            return Err(SemanticActionError::MissingTarget);
        };
        if matches.next().is_some() {
            if state.open_overlay_id() == Some(&menu.id) {
                state.dismiss_overlay(crate::DismissReason::Cancel);
            }
            return Err(SemanticActionError::AmbiguousTarget);
        }
        menu.anchor = menu.anchor.with_resolved_target(target);
        let anchor_node = self
            .resolved
            .find(menu.anchor.id())
            .ok_or(SemanticActionError::MissingTarget)?
            .allocated;
        let anchor = menu.anchor.rect(anchor_node);
        self.overlay_invokers
            .push((menu.anchor.id().clone(), menu.id.clone()));
        if let Some(anchor) = self.resolved.find_mut(menu.anchor.id())
            && !anchor.semantic_actions.contains(&ActionKind::ContextMenu)
        {
            anchor.semantic_actions.push(ActionKind::ContextMenu);
        }
        if state.open_overlay_id() != Some(&menu.id) {
            return Ok(());
        }
        if state.navigation().controller_selected().is_none()
            && let Some(item) = menu.initial_controller_item.as_ref()
        {
            state
                .navigation_mut()
                .set_controller_selected(Some(menu.id.item_id(item)));
        }
        let item_count = menu.items.len();
        let content_height = menu.row_height * item_count as f32
            + menu.row_gap * item_count.saturating_sub(1) as f32;
        let rect = crate::place_transient(
            anchor,
            Size {
                width: menu.width,
                height: menu.padding.top + content_height + menu.padding.bottom,
            },
            self.viewport,
            menu.placement,
            menu.collision,
            menu.direction,
            1.0,
        );
        self.commands.push(PaintCommand::RoundedFill {
            rect,
            color: menu.background,
            radius: menu.radius,
        });
        if menu.border_width > 0.0 {
            self.commands.push(PaintCommand::OverlayStroke {
                rect: rect.inset(Insets::all(menu.border_width / 2.0)),
                color: menu.border,
                width: menu.border_width,
            });
        }
        let menu_index = self.resolved.nodes.len();
        self.resolved.nodes.push(ResolvedNode {
            component: "Menu",
            id: menu.id.as_ui_id().clone(),
            source: None,
            allocated: rect,
            padding_box: rect,
            border_box: rect,
            content: rect.inset(menu.padding),
            constraints: Constraints::tight(rect.size),
            preferred: rect.size,
            flex_basis: Length::Auto,
            flex_grow: 0.0,
            flex_shrink: 0.0,
            clip: None,
            scroll: None,
            grid_tracks: Vec::new(),
            hit_stack: None,
            interaction: InteractionState::default(),
            navigation_scope: Some(crate::NavigationScope::group()),
            adjustment_step: 0.05,
            controller_value: None,
            accessibility_label: None,
            accessibility_description: None,
            accessibility_role: Some(SemanticRole::Menu.as_str().into()),
            accessibility_state: None,
            accessibility_controls: None,
            accessibility_hidden: false,
            semantic_role: Some(SemanticRole::Menu),
            semantic_actions: Vec::new(),
            semantic_value: None,
            children: Vec::with_capacity(item_count),
        });
        self.accessibility.push(AccessibilityNode {
            id: menu.id.as_ui_id().clone(),
            parent: None,
            component: "Menu",
            rect,
            interactive: false,
            label: None,
            description: None,
            role: Some(SemanticRole::Menu.as_str().into()),
            state: None,
            controls: None,
            semantic_role: Some(SemanticRole::Menu),
            actions: Vec::new(),
            enabled: true,
            focused: false,
            controller_selected: false,
            navigation_depth: 1,
            value: None,
        });
        if menu.focus == crate::OverlayFocusPolicy::FirstItem
            && let Some(item) = menu.items.iter().find(|item| {
                item.action.is_some() || item.text_command.is_some() || !item.children.is_empty()
            })
        {
            let id = menu.id.item_id(&item.id);
            let belongs_to_menu = |selected: Option<&UiId>| {
                selected.is_some_and(|selected| {
                    selected
                        .as_str()
                        .strip_prefix(menu.id.as_ui_id().as_str())
                        .is_some_and(|suffix| suffix.starts_with('/'))
                })
            };
            if !belongs_to_menu(state.focused()) {
                state.set_focus(Some(id.clone()));
            }
            if (state.navigation().controller_selected().is_some()
                || state.input_modality() != InputModality::Pointer)
                && !belongs_to_menu(state.navigation().controller_selected())
            {
                state.navigation_mut().set_controller_selected(Some(id));
            }
            // Controller selection is the menu's active focus authority. Keep
            // focus synchronized before emitting paint commands so a rebuild
            // cannot retain the former focused row's selected treatment while
            // semantics already report the newly selected row.
            if let Some(selected) = state
                .navigation()
                .controller_selected()
                .filter(|selected| belongs_to_menu(Some(selected)))
                .cloned()
            {
                state.set_focus(Some(selected));
            }
        }
        let submenu_rect = self.emit_overlay_menu_items(
            state,
            &menu,
            OverlayMenuLevel {
                id: menu.id.as_ui_id(),
                node_index: menu_index,
                rect,
                items: &menu.items,
                depth: 1,
            },
        );
        let active_rect = submenu_rect.map_or(rect, |child| {
            let left = rect.origin.x.min(child.origin.x);
            let top = rect.origin.y.min(child.origin.y);
            let right = (rect.origin.x + rect.size.width).max(child.origin.x + child.size.width);
            let bottom = (rect.origin.y + rect.size.height).max(child.origin.y + child.size.height);
            Rect::new(left, top, right - left, bottom - top)
        });
        self.active_overlay = Some((menu.id.clone(), active_rect));
        self.active_overlay_dismiss = Some(crate::DismissPolicy::default());
        let overlay_root = menu.id.as_ui_id();
        let overlay_accessibility = self
            .resolved
            .nodes
            .iter()
            .filter(|node| self.is_descendant_or_self(overlay_root, &node.id))
            .map(|node| node.id.clone())
            .collect::<BTreeSet<_>>();
        self.accessibility
            .retain(|node| overlay_accessibility.contains(&node.id));
        Ok(())
    }

    /// Presents a menu whose host surface is itself the open menu. This keeps
    /// overlay ownership and focus initialization inside the canonical frame
    /// transition instead of requiring consumers to mutate [`UiStateStore`].
    pub fn present_open_menu(
        &mut self,
        state: &mut UiStateStore,
        menu: crate::OverlayMenu<Message>,
    ) -> Result<EventOutcome<Message>, SemanticActionError> {
        let target = menu.anchor.id().clone();
        self.present_menu(state, menu.clone())?;
        let outcome = self.transition(
            state,
            InputSource::Programmatic,
            InteractionIntent::Invoke {
                target,
                action: SemanticAction::Invoke(ActionKind::ContextMenu),
            },
        )?;
        self.present_menu(state, menu)?;
        Ok(outcome)
    }
    /// Resolves a declarative view into the canonical retained frame.
    pub fn resolve(root: impl Component<Message>, request: FrameRequest<'_>) -> Self {
        Self::layout_with_state_and_diagnostics(
            root,
            request.viewport,
            request.state,
            request.diagnostics == DiagnosticMode::Collect,
        )
    }

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
            viewport: bounds,
            ..Self::default()
        };
        layout_element(&root, &root_id, bounds, None, None, &mut tree);
        tree.selection_regions = collect_selection_regions(&root, &tree.resolved);
        for region in &tree.selection_regions {
            state.touch(region.id.clone());
            state.reconcile_document_selection(region.id.clone(), region.document.clone());
        }
        for node in &tree.resolved.nodes {
            if node.navigation_scope.is_some() {
                state.touch(node.id.clone());
            }
        }
        tree.prepare_selection_paints(state);
        tree.reset_emission();
        emit_element(&root, 0, None, &mut tree);
        tree.present_text_context(state);
        tree.append_scrollbars(Some(state));
        tree.commands.append(&mut tree.overlay_commands);
        tree.hits.append(&mut tree.overlay_hits);
        for hit in &tree.hits {
            state.touch(hit.id.clone());
        }
        if state.navigation().controller_scope().is_some()
            && state.navigation().controller_selected().is_none()
        {
            tree.move_controller(state, 1, true);
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
        state.reconcile_live_targets();
        tree.apply_interaction_state(state);
        tree.release_build_scratch();
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
        tree.append_scrollbars(None);
        tree.commands.append(&mut tree.overlay_commands);
        tree.hits.append(&mut tree.overlay_hits);
        tree.emit_accessibility_geometry();
        tree.validate_clip_commands();
        crate::ui::assert_background_color_policy(root_id.as_str(), &tree.commands);
        tree.release_build_scratch();
        tree
    }

    fn present_text_context(&mut self, state: &mut UiStateStore) {
        let Some(session) = state.text_context().cloned() else {
            return;
        };
        if !session.editable {
            let Some(region) = self.selection_region(&session.editor) else {
                state.dismiss_overlay(crate::DismissReason::Cancel);
                return;
            };
            let Some(selection) = state.document_selection(&session.editor) else {
                state.dismiss_overlay(crate::DismissReason::Cancel);
                return;
            };
            if selection_document_generation(&region.document) != session.document_generation
                || document_selection_generation(selection) != session.selection_generation
            {
                state.dismiss_overlay(crate::DismissReason::Cancel);
                return;
            }
            let menu = crate::text_context_menu::internal_read_only_text_context_menu(
                session.editor.scoped("text-context-menu"),
                session.anchor,
                region.document.selected_text(selection).is_some(),
                region
                    .document
                    .runs()
                    .iter()
                    .any(|run| !run.text.is_empty()),
            );
            let _ = self.present_menu(state, menu);
            return;
        }
        let Some(input) = self
            .text_inputs
            .iter()
            .find(|input| input.id == session.editor && input.secure == session.secure)
        else {
            state.dismiss_overlay(crate::DismissReason::Cancel);
            return;
        };
        let Some(editor) = state
            .state(&session.editor)
            .and_then(|entry| entry.editor.as_ref())
            .cloned()
        else {
            state.dismiss_overlay(crate::DismissReason::Cancel);
            return;
        };
        if editor.document_generation() != session.document_generation
            || editor.selection_generation() != session.selection_generation
        {
            state.dismiss_overlay(crate::DismissReason::Cancel);
            return;
        }
        let policy = crate::TextContextPolicy {
            editable: true,
            secure: input.secure,
            clipboard_has_text: state.clipboard_text().is_some(),
        };
        let menu = crate::text_context_menu::internal_text_context_menu(
            session.editor.scoped("text-context-menu"),
            session.anchor,
            &editor,
            policy,
        );
        let _ = self.present_menu(state, menu);
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

    pub(crate) fn navigation_depth(&self, state: &UiStateStore) -> usize {
        let mut depth = 0;
        let mut scope = state.navigation().controller_scope().cloned();
        while let Some(current) = scope {
            depth += 1;
            scope = self
                .nearest_ancestor_where(&current, |node| node.navigation_scope.is_some())
                .map(|node| node.id.clone());
        }
        depth
    }

    pub(crate) fn available_semantic_actions(&self, state: &UiStateStore) -> Vec<ActionKind> {
        state
            .navigation()
            .controller_selected()
            .or_else(|| state.focused())
            .and_then(|id| self.resolved.find(id))
            .map(|node| node.semantic_actions.clone())
            .unwrap_or_default()
    }

    pub fn semantic_nodes(&self) -> Vec<SemanticNodeSnapshot> {
        let mut parents = vec![None; self.resolved.nodes.len()];
        for node in &self.resolved.nodes {
            for child in &node.children {
                parents[*child] = Some(node.id.clone());
            }
        }
        self.resolved
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                self.active_overlay.as_ref().is_none_or(|(overlay, _)| {
                    self.is_descendant_or_self(overlay.as_ui_id(), &node.id)
                })
            })
            .filter(|(_, node)| node.semantic_role.is_some() || !node.semantic_actions.is_empty())
            .map(|(index, node)| SemanticNodeSnapshot {
                id: node.id.clone(),
                parent: parents[index].clone(),
                bounds: node.border_box,
                role: node.semantic_role,
                name: node.accessibility_label.clone(),
                description: node.accessibility_description.clone(),
                enabled: !node.semantic_actions.is_empty(),
                focused: node.interaction.focused,
                controller_selected: node.interaction.controller_selected,
                navigation_scope: node.navigation_scope.is_some(),
                navigation_depth: self.navigation_depth_for_index(index),
                controls: node.accessibility_controls.clone(),
                actions: node.semantic_actions.clone(),
                value: node.semantic_value.clone(),
            })
            .collect()
    }

    fn semantic_snapshot(&self, index: usize) -> SemanticNodeSnapshot {
        let node = &self.resolved.nodes[index];
        let parent = self
            .semantic_parents
            .get(index)
            .copied()
            .flatten()
            .map(|parent| self.resolved.nodes[parent].id.clone());
        SemanticNodeSnapshot {
            id: node.id.clone(),
            parent,
            bounds: node.border_box,
            role: node.semantic_role,
            name: node.accessibility_label.clone(),
            description: node.accessibility_description.clone(),
            enabled: !node.semantic_actions.is_empty(),
            focused: node.interaction.focused,
            controller_selected: node.interaction.controller_selected,
            navigation_scope: node.navigation_scope.is_some(),
            navigation_depth: self.navigation_depth_for_index(index),
            controls: node.accessibility_controls.clone(),
            actions: node.semantic_actions.clone(),
            value: node.semantic_value.clone(),
        }
    }

    /// Applies one interaction through the canonical frame and state authority.
    ///
    /// Existing event-specific entry points remain temporarily available while consumers migrate,
    /// but hosts and development tooling must use this transition surface.
    pub fn transition(
        &self,
        state: &mut UiStateStore,
        source: InputSource,
        intent: InteractionIntent,
    ) -> Result<EventOutcome<Message>, SemanticActionError> {
        let modality = match source {
            InputSource::Keyboard => Some(InputModality::Keyboard),
            InputSource::Pointer => Some(InputModality::Pointer),
            InputSource::Controller => Some(InputModality::Controller),
            InputSource::Accessibility => Some(InputModality::Accessibility),
            InputSource::Programmatic | InputSource::System => None,
        };
        let mut modality_invalidation = modality
            .map(|modality| state.set_input_modality(modality))
            .unwrap_or(Invalidation::None);
        if matches!(
            source,
            InputSource::Keyboard | InputSource::Controller | InputSource::Accessibility
        ) {
            modality_invalidation = modality_invalidation.merge(state.set_hovered(None));
        }
        let dismiss_blurred_choices = matches!(
            &intent,
            InteractionIntent::Event(
                UiEvent::FocusNext
                    | UiEvent::FocusPrevious
                    | UiEvent::KeyboardNavigateUp
                    | UiEvent::KeyboardNavigateDown
                    | UiEvent::KeyboardNavigateLeft
                    | UiEvent::KeyboardNavigateRight
                    | UiEvent::ControllerUp
                    | UiEvent::ControllerDown
                    | UiEvent::ControllerLeft
                    | UiEvent::ControllerRight
                    | UiEvent::ControllerNext
                    | UiEvent::ControllerPrevious
                    | UiEvent::AccessibilityFocus(_)
            )
        );
        let mut outcome = match intent {
            InteractionIntent::Event(event) => self.reduce_event(state, event),
            InteractionIntent::Invoke { target, action } => {
                if action == SemanticAction::Invoke(ActionKind::ContextMenu)
                    && let Some((invocation_target, overlay)) = self
                        .overlay_invokers
                        .iter()
                        .find(|(invocation_target, _)| invocation_target == &target)
                        .cloned()
                {
                    EventOutcome {
                        invalidation: state.open_overlay(overlay, invocation_target),
                        ..EventOutcome::default()
                    }
                } else if action == SemanticAction::Invoke(ActionKind::ContextMenu)
                    && self.is_dropdown(&target)
                {
                    EventOutcome {
                        messages: self
                            .context_message_for_id(&target)
                            .cloned()
                            .into_iter()
                            .collect(),
                        invalidation: state.set_dropdown_open(target, true),
                        ..EventOutcome::default()
                    }
                } else {
                    let dismisses = matches!(action, SemanticAction::Invoke(ActionKind::Activate))
                        && self
                            .active_overlay_dismiss
                            .is_none_or(|policy| policy.action)
                        && self.active_overlay.as_ref().is_some_and(|(overlay, _)| {
                            self.is_descendant_or_self(overlay.as_ui_id(), &target)
                        });
                    let mut outcome = EventOutcome::default();
                    if matches!(
                        action,
                        SemanticAction::Invoke(
                            ActionKind::Increment | ActionKind::Decrement | ActionKind::Scroll
                        )
                    ) && self.scrolls.iter().any(|scroll| scroll.id == target)
                    {
                        outcome = self.perform_scroll_action(state, &target, action)?;
                    } else if matches!(action, SemanticAction::Invoke(ActionKind::Activate))
                        && let Some(invalidation) =
                            self.activate_text_command(state, &target, &mut outcome)
                    {
                        outcome.invalidation = invalidation;
                    } else {
                        outcome = self.perform_semantic_action(&target, action)?;
                    }
                    if dismisses {
                        outcome.invalidation = outcome
                            .invalidation
                            .merge(state.dismiss_overlay(crate::DismissReason::Action));
                    }
                    outcome
                }
            }
        };
        if dismiss_blurred_choices {
            outcome.invalidation = outcome
                .invalidation
                .merge(self.dismiss_blurred_dropdowns(state));
        }
        outcome.invalidation = outcome.invalidation.merge(modality_invalidation);
        Ok(outcome)
    }

    fn dismiss_blurred_dropdowns(&self, state: &mut UiStateStore) -> Invalidation {
        let focused = state.focused().cloned();
        let blurred = self
            .resolved
            .nodes()
            .iter()
            .filter(|node| node.component == "Dropdown")
            .filter(|node| {
                state
                    .state(&node.id)
                    .is_some_and(|entry| entry.dropdown_open)
            })
            .filter(|node| {
                focused
                    .as_ref()
                    .is_none_or(|focused| !self.is_descendant_or_self(&node.id, focused))
            })
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        blurred
            .into_iter()
            .fold(Invalidation::None, |invalidation, id| {
                invalidation.merge(state.set_dropdown_open(id, false))
            })
    }

    fn perform_scroll_action(
        &self,
        state: &mut UiStateStore,
        id: &UiId,
        action: SemanticAction,
    ) -> Result<EventOutcome<Message>, SemanticActionError> {
        let scroll = self
            .scrolls
            .iter()
            .rev()
            .find(|scroll| &scroll.id == id)
            .ok_or(SemanticActionError::MissingTarget)?;
        let direction = match action {
            SemanticAction::Invoke(ActionKind::Decrement) => -1.0,
            SemanticAction::Invoke(ActionKind::Increment | ActionKind::Scroll) => 1.0,
            _ => return Err(SemanticActionError::ActionUnavailable),
        };
        let vertical_maximum =
            (scroll.extent.content.height - scroll.extent.viewport.height).max(0.0);
        let horizontal_maximum =
            (scroll.extent.content.width - scroll.extent.viewport.width).max(0.0);
        let mut messages = Vec::new();
        let invalidation = if vertical_maximum > 0.0 {
            let step = (scroll.extent.viewport.height * 0.8).max(1.0) * direction;
            let invalidation = state.scroll_by(scroll.id.clone(), step, vertical_maximum);
            if invalidation != Invalidation::None
                && let Some(map) = scroll.offset_mapper
                && let Some(offset) = state.state(&scroll.id).map(|entry| entry.scroll_offset)
            {
                messages.push(map(offset));
            }
            invalidation
        } else if horizontal_maximum > 0.0 {
            let step = (scroll.extent.viewport.width * 0.8).max(1.0) * direction;
            state.scroll_by_x(scroll.id.clone(), step, horizontal_maximum)
        } else {
            return Err(SemanticActionError::ActionUnavailable);
        };
        Ok(EventOutcome {
            messages,
            clipboard_text: None,
            invalidation,
        })
    }

    fn operate_navigation_scroll(
        &self,
        state: &mut UiStateStore,
        direction: f32,
        messages: &mut Vec<Message>,
    ) -> Option<Invalidation> {
        let selected = state
            .navigation()
            .controller_selected()
            .or_else(|| state.focused())?;
        let scroll = self.scrolls.iter().rev().find(|scroll| {
            scroll.id == *selected || self.is_descendant_or_self(&scroll.id, selected)
        })?;
        let action = if direction < 0.0 {
            ActionKind::Decrement
        } else {
            ActionKind::Increment
        };
        let outcome = self
            .perform_scroll_action(state, &scroll.id, SemanticAction::Invoke(action))
            .ok()?;
        messages.extend(outcome.messages);
        Some(outcome.invalidation)
    }

    pub fn perform_semantic_action(
        &self,
        id: &UiId,
        action: SemanticAction,
    ) -> Result<EventOutcome<Message>, SemanticActionError> {
        let node = self
            .resolved
            .find(id)
            .ok_or(SemanticActionError::MissingTarget)?;
        let kind = match &action {
            SemanticAction::Invoke(kind) => *kind,
            SemanticAction::SetValue(_) => ActionKind::SetValue,
        };
        if !node.semantic_actions.contains(&kind) {
            return Err(SemanticActionError::ActionUnavailable);
        }
        let message = match action {
            SemanticAction::Invoke(ActionKind::Activate) => self.message_for_id(id).cloned(),
            SemanticAction::Invoke(ActionKind::ContextMenu) => {
                self.context_message_for_id(id).cloned()
            }
            SemanticAction::Invoke(ActionKind::Increment | ActionKind::Decrement) => {
                let direction = if kind == ActionKind::Increment {
                    1.0
                } else {
                    -1.0
                };
                self.messages
                    .iter()
                    .rev()
                    .find(|region| &region.id == id)
                    .and_then(|region| region.message_mapper)
                    .zip(node.controller_value)
                    .map(|(map, value)| {
                        map((value + direction * node.adjustment_step).clamp(0.0, 1.0))
                    })
            }
            SemanticAction::SetValue(SemanticValueInput::Number(value)) => self
                .messages
                .iter()
                .rev()
                .find(|region| &region.id == id)
                .and_then(|region| region.message_mapper)
                .map(|map| map((value as f32).clamp(0.0, 1.0))),
            SemanticAction::SetValue(SemanticValueInput::Text(value)) => self
                .text_inputs
                .iter()
                .rev()
                .find(|region| &region.id == id)
                .map(|region| (region.map)(value)),
            _ => None,
        }
        .ok_or(SemanticActionError::ActionUnavailable)?;
        Ok(EventOutcome {
            messages: vec![message],
            clipboard_text: None,
            invalidation: Invalidation::None,
        })
    }

    pub fn resolve_effective_target(
        &self,
        id: &UiId,
        action: ActionKind,
    ) -> Result<EffectiveHitRoute, SemanticActionError> {
        let node = self
            .resolved
            .find(id)
            .ok_or(SemanticActionError::MissingTarget)?;
        if !node.semantic_actions.contains(&action) {
            return Err(SemanticActionError::ActionUnavailable);
        }
        let hit = self
            .hits
            .iter()
            .rev()
            .find(|hit| &hit.id == id)
            .ok_or(SemanticActionError::ActionUnavailable)?;
        let bounds = node
            .clip
            .and_then(|clip| intersection(hit.rect, clip))
            .unwrap_or(hit.rect);
        let point = Point {
            x: bounds.origin.x + bounds.size.width / 2.0,
            y: bounds.origin.y + bounds.size.height / 2.0,
        };
        if self.id_at(point) != Some(id) {
            return Err(SemanticActionError::ActionUnavailable);
        }
        Ok(EffectiveHitRoute {
            target: id.clone(),
            bounds,
            point,
        })
    }

    pub fn resource_diagnostics(&self) -> FrameResourceDiagnostics {
        let vector_bytes = self.commands.capacity() * std::mem::size_of::<PaintCommand>()
            + self.hits.capacity() * std::mem::size_of::<HitRegion<Message>>()
            + self.messages.capacity() * std::mem::size_of::<MessageRegion<Message>>()
            + self.context_messages.capacity() * std::mem::size_of::<MessageRegion<Message>>()
            + self.text_inputs.capacity() * std::mem::size_of::<TextInputRegion<Message>>()
            + self.selection_regions.capacity() * std::mem::size_of::<SelectionRegionLayout>()
            + self.scrolls.capacity() * std::mem::size_of::<ScrollRegion<Message>>()
            + self.grids.capacity() * std::mem::size_of::<ResolvedGrid>()
            + self.accessibility.capacity() * std::mem::size_of::<AccessibilityNode>()
            + self.resolved.nodes.capacity() * std::mem::size_of::<ResolvedNode>()
            + self.diagnostics.capacity() * std::mem::size_of::<LayoutDiagnostic>();
        let node_children = self
            .resolved
            .nodes
            .iter()
            .map(|node| node.children.capacity() * std::mem::size_of::<usize>())
            .sum::<usize>();
        let accessibility_strings = self
            .accessibility
            .iter()
            .map(|node| {
                node.label.as_ref().map_or(0, String::capacity)
                    + node.description.as_ref().map_or(0, String::capacity)
                    + node.role.as_ref().map_or(0, String::capacity)
                    + node.state.as_ref().map_or(0, String::capacity)
            })
            .sum::<usize>();
        let retained_build_scratch_bytes = self.overlay_commands.capacity()
            * std::mem::size_of::<PaintCommand>()
            + self.overlay_hits.capacity() * std::mem::size_of::<HitRegion<Message>>()
            + self.selection_paints.capacity() * std::mem::size_of::<(usize, Vec<Rect>)>()
            + self.diagnostic_keys.capacity() * std::mem::size_of::<(DiagnosticKind, UiId)>()
            + self.seen_ids.capacity() * std::mem::size_of::<UiId>();
        FrameResourceDiagnostics {
            node_count: self.resolved.nodes.len(),
            paint_primitive_count: self.commands.len(),
            hit_target_count: self.hits.len(),
            message_binding_count: self.messages.len() + self.context_messages.len(),
            accessibility_node_count: self.accessibility.len(),
            estimated_retained_bytes: vector_bytes + node_children + accessibility_strings,
            retained_build_scratch_bytes,
        }
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

    /// Returns every semantic target that dispatches `message`. Duplicate
    /// messages remain visible so callers must choose targets semantically.
    pub fn semantic_targets_for_message(&self, message: &Message) -> Vec<SemanticTarget>
    where
        Message: PartialEq,
    {
        self.messages
            .iter()
            .filter(|region| &region.message == message)
            .map(|region| {
                let node = self.resolved.find(&region.id);
                SemanticTarget {
                    id: region.id.clone(),
                    bounds: region.rect,
                    role: node.and_then(|node| node.semantic_role),
                    name: node.and_then(|node| node.accessibility_label.clone()),
                    interactive: node.is_some_and(|node| node.interaction.interactive),
                }
            })
            .collect()
    }

    /// Queries backend-neutral semantics without exposing renderer traversal
    /// or silently collapsing duplicate matches.
    pub fn query(&self, selector: &SemanticSelector) -> Vec<SemanticNodeSnapshot> {
        if let SemanticSelector::RoleAndName { role, name } = selector {
            return self
                .semantic_role_name_index
                .get(&(*role, name.clone()))
                .into_iter()
                .flatten()
                .map(|index| self.semantic_snapshot(*index))
                .collect();
        }
        self.semantic_nodes()
            .into_iter()
            .filter(|node| match selector {
                SemanticSelector::Id(id) => &node.id == id,
                SemanticSelector::Role(role) => node.role == Some(*role),
                SemanticSelector::Name(name) => node.name.as_ref() == Some(name),
                SemanticSelector::RoleAndName { role, name } => {
                    node.role == Some(*role) && node.name.as_ref() == Some(name)
                }
                SemanticSelector::Action(action) => node.actions.contains(action),
            })
            .collect()
    }

    pub fn query_unique(
        &self,
        selector: &SemanticSelector,
    ) -> Result<SemanticNodeSnapshot, SemanticQueryError> {
        let mut matches = self.query(selector).into_iter();
        let Some(target) = matches.next() else {
            return Err(SemanticQueryError::Missing);
        };
        let additional = matches.count();
        if additional == 0 {
            Ok(target)
        } else {
            Err(SemanticQueryError::Ambiguous {
                matches: additional + 1,
            })
        }
    }

    /// Resolves one semantic target and rejects message reuse explicitly.
    pub fn unique_semantic_target_for_message(
        &self,
        message: &Message,
    ) -> Result<SemanticTarget, SemanticQueryError>
    where
        Message: PartialEq,
    {
        let mut matches = self.semantic_targets_for_message(message).into_iter();
        let Some(target) = matches.next() else {
            return Err(SemanticQueryError::Missing);
        };
        let additional = matches.count();
        if additional == 0 {
            Ok(target)
        } else {
            Err(SemanticQueryError::Ambiguous {
                matches: additional + 1,
            })
        }
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

    pub fn context_message_at(&self, point: Point) -> Option<&Message> {
        self.id_at(point)
            .and_then(|id| self.context_message_for_id(id))
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
        for node in &self.resolved.nodes {
            if node.navigation_scope.is_some() {
                state.touch(node.id.clone());
            }
        }
        for id in retained {
            state.touch(id);
        }
        state.end_frame();
    }

    fn append_scrollbars(&mut self, state: Option<&UiStateStore>) {
        for scroll in &self.scrolls {
            for axis in [ScrollbarAxis::Horizontal, ScrollbarAxis::Vertical] {
                let Some((track, thumb)) = scrollbar_geometry(scroll, axis) else {
                    continue;
                };
                if intersection(track, scroll.clip).is_none() {
                    continue;
                }
                let id = scrollbar_id(&scroll.id, axis);
                let colors = state.map_or(scroll.scrollbar.idle, |state| {
                    if state.pressed() == Some(&id) || state.captured() == Some(&id) {
                        scroll.scrollbar.pressed
                    } else if state.hovered() == Some(&id) {
                        scroll.scrollbar.hovered
                    } else if state.focused() == Some(&scroll.id)
                        || state.navigation().controller_selected() == Some(&scroll.id)
                    {
                        scroll.scrollbar.focused
                    } else {
                        scroll.scrollbar.idle
                    }
                });
                self.commands.push(PaintCommand::PushClip(scroll.clip));
                self.commands.push(PaintCommand::RoundedFill {
                    rect: track,
                    color: colors.track,
                    radius: SCROLLBAR_THICKNESS / 2.0,
                });
                self.commands.push(PaintCommand::RoundedFill {
                    rect: thumb,
                    color: colors.thumb,
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
                    scrollbar_hit_rect(scroll, *axis).is_some_and(|hit| contains(hit, point))
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
        let grabbed_offset = state.scrollbar_grab_offset().unwrap_or(thumb_length / 2.0);
        let target = if travel > 0.0 {
            ((pointer - track_start - grabbed_offset) / travel).clamp(0.0, 1.0) * maximum
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

    fn page_scrollbar(
        scroll: &ScrollRegion<Message>,
        axis: ScrollbarAxis,
        point: Point,
        state: &mut UiStateStore,
        messages: &mut Vec<Message>,
    ) -> Invalidation {
        let Some((_, thumb)) = scrollbar_geometry(scroll, axis) else {
            return Invalidation::None;
        };
        let (pointer, thumb_start, thumb_end, amount, maximum) = match axis {
            ScrollbarAxis::Horizontal => (
                point.x,
                thumb.origin.x,
                thumb.origin.x + thumb.size.width,
                scroll.extent.viewport.width,
                (scroll.extent.content.width - scroll.extent.viewport.width).max(0.0),
            ),
            ScrollbarAxis::Vertical => (
                point.y,
                thumb.origin.y,
                thumb.origin.y + thumb.size.height,
                scroll.extent.viewport.height,
                (scroll.extent.content.height - scroll.extent.viewport.height).max(0.0),
            ),
        };
        let delta = if pointer < thumb_start {
            -amount
        } else if pointer > thumb_end {
            amount
        } else {
            return Invalidation::None;
        };
        let invalidation = match axis {
            ScrollbarAxis::Horizontal => state.scroll_by_x(scroll.id.clone(), delta, maximum),
            ScrollbarAxis::Vertical => state.scroll_by(scroll.id.clone(), delta, maximum),
        };
        if invalidation != Invalidation::None
            && axis == ScrollbarAxis::Vertical
            && let Some(map) = scroll.offset_mapper
            && let Some(offset) = state.state(&scroll.id).map(|entry| entry.scroll_offset)
        {
            messages.push(map(offset));
        }
        invalidation
    }

    pub fn handle_event(&self, state: &mut UiStateStore, event: UiEvent) -> EventOutcome<Message> {
        let source = event.input_source();
        self.transition(state, source, InteractionIntent::Event(event))
            .expect("ordinary UI events cannot fail semantic resolution")
    }

    pub(crate) fn is_text_input(&self, id: &UiId) -> bool {
        self.text_inputs.iter().any(|input| &input.id == id)
    }

    fn reduce_event(&self, state: &mut UiStateStore, event: UiEvent) -> EventOutcome<Message> {
        let mut outcome = EventOutcome::default();
        let keyboard_tree_navigation = matches!(
            event,
            UiEvent::KeyboardNavigateUp
                | UiEvent::KeyboardNavigateDown
                | UiEvent::KeyboardNavigateLeft
                | UiEvent::KeyboardNavigateRight
                | UiEvent::KeyboardNavigateBack
                | UiEvent::KeyboardNavigateActivate
        );
        if matches!(
            event,
            UiEvent::ControllerUp
                | UiEvent::ControllerDown
                | UiEvent::ControllerLeft
                | UiEvent::ControllerRight
                | UiEvent::ControllerNext
                | UiEvent::ControllerPrevious
                | UiEvent::ControllerPreviousPane
                | UiEvent::ControllerNextPane
                | UiEvent::ControllerAdjust(_)
                | UiEvent::ControllerBack
                | UiEvent::ControllerActivate
                | UiEvent::ControllerContextMenu
        ) && !state.window_focused()
        {
            return outcome;
        }
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
                let hovered = self
                    .scrollbar_at(point)
                    .map(|(scroll, axis)| scrollbar_id(&scroll.id, axis))
                    .or_else(|| self.id_at(point).cloned());
                let mut invalidation = state.set_hovered(hovered);
                if let Some(captured) = state.captured()
                    && let Some(message) = self.drag_message(captured, DragPhase::Moved, point)
                {
                    outcome.messages.push(message);
                    invalidation = invalidation.merge(Invalidation::Paint);
                }
                if let Some(captured) = state.captured()
                    && let Some(hit) = self.hits.iter().rev().find(|hit| &hit.id == captured)
                    && let Some(map) = hit.message_mapper
                {
                    let fraction = ((point.x - hit.rect.origin.x) / hit.rect.size.width.max(1.0))
                        .clamp(0.0, 1.0);
                    outcome.messages.push(map(fraction));
                    invalidation = invalidation.merge(Invalidation::Paint);
                }
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
            UiEvent::PointerContext(point) | UiEvent::TouchLongPress(point) => {
                if let Some(target) = self.id_at(point).cloned()
                    && let Some(invalidation) = self.open_text_context(state, &target, Some(point))
                {
                    return EventOutcome {
                        invalidation,
                        ..outcome
                    };
                }
                if let Some(target) = self
                    .selection_hit_at(point)
                    .map(|(region, _)| region.id.clone())
                    && let Some(invalidation) =
                        self.open_read_only_text_context(state, &target, Some(point))
                {
                    return EventOutcome {
                        invalidation,
                        ..outcome
                    };
                }
                if let Some((target, overlay)) = self
                    .id_at(point)
                    .and_then(|id| {
                        self.overlay_invokers
                            .iter()
                            .find(|(target, _)| target == id)
                    })
                    .cloned()
                {
                    return EventOutcome {
                        invalidation: state.open_overlay(overlay, target),
                        ..outcome
                    };
                }
                let target = self.id_at(point).cloned();
                if let Some(message) = target
                    .as_ref()
                    .and_then(|id| self.context_message_for_id(id))
                    .cloned()
                {
                    outcome.messages.push(message);
                }
                target
                    .filter(|id| self.is_dropdown(id))
                    .map_or(Invalidation::None, |id| state.set_dropdown_open(id, true))
            }
            UiEvent::PointerPressed(point) => {
                if self
                    .active_overlay
                    .as_ref()
                    .is_some_and(|(_, rect)| !contains(*rect, point))
                    && self
                        .active_overlay_dismiss
                        .is_none_or(|policy| policy.outside_pointer)
                {
                    let invalidation = state.dismiss_overlay(crate::DismissReason::OutsidePointer);
                    return EventOutcome {
                        invalidation,
                        ..outcome
                    };
                }
                if let Some((scroll, axis)) = self.scrollbar_at(point) {
                    let id = scrollbar_id(&scroll.id, axis);
                    let (_, thumb) = scrollbar_geometry(scroll, axis)
                        .expect("a hit-tested scrollbar has geometry");
                    let invalidation = if scrollbar_thumb_hit_rect(scroll, axis)
                        .is_some_and(|hit| contains(hit, point))
                    {
                        let grab_offset = match axis {
                            ScrollbarAxis::Horizontal => point.x - thumb.origin.x,
                            ScrollbarAxis::Vertical => point.y - thumb.origin.y,
                        };
                        state.set_scrollbar_grab_offset(grab_offset);
                        state
                            .set_focus(Some(scroll.id.clone()))
                            .merge(state.set_pressed(Some(id.clone())))
                            .merge(state.set_capture(Some(id)))
                            .merge(Invalidation::Paint)
                    } else {
                        state
                            .set_focus(None)
                            .merge(state.set_pressed(Some(id)))
                            .merge(Self::page_scrollbar(
                                scroll,
                                axis,
                                point,
                                state,
                                &mut outcome.messages,
                            ))
                    };
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
                        && !clicked.is_some_and(|id| {
                            self.is_descendant_or_self(&node.id, &UiId::from(id.to_owned()))
                        })
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
                if let Some(id) = id.as_ref()
                    && let Some(message) = self.drag_message(id, DragPhase::Started, point)
                {
                    outcome.messages.push(message);
                    invalidation = invalidation.merge(Invalidation::Paint);
                }
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
                if let Some(captured) = state.captured()
                    && let Some(message) = self.drag_message(captured, DragPhase::Ended, point)
                {
                    outcome.messages.push(message);
                }
                let released = self.id_at(point);
                let activates = state
                    .captured()
                    .is_some_and(|captured| released == Some(captured));
                let text_command_invalidation = if activates {
                    released
                        .and_then(|target| self.activate_text_command(state, target, &mut outcome))
                        .unwrap_or(Invalidation::None)
                } else {
                    Invalidation::None
                };
                if activates
                    && text_command_invalidation == Invalidation::None
                    && let Some(message) = self.message_at_owned(point)
                {
                    outcome.messages.push(message);
                }
                let overlay_action = activates
                    && released.is_some_and(|item| {
                        self.message_for_id(item).is_some()
                            || self.text_commands.iter().any(|command| &command.id == item)
                    })
                    && self.active_overlay.as_ref().is_some_and(|(id, _)| {
                        released.is_some_and(|item| self.is_descendant_or_self(id.as_ui_id(), item))
                    });
                let dropdown = state.captured().and_then(|id| {
                    self.resolved_layout()
                        .find(id)
                        .filter(|node| node.component == "Dropdown")
                        .map(|_| id.clone())
                });
                let dropdown_invalidation = dropdown.as_ref().map_or(Invalidation::None, |id| {
                    let open = !state.state(id).is_some_and(|entry| entry.dropdown_open);
                    state.set_dropdown_open(id.clone(), open)
                });
                let option_parent = state
                    .captured()
                    .filter(|id| {
                        self.resolved
                            .find(id)
                            .is_none_or(|node| node.component != "Dropdown")
                    })
                    .and_then(|id| {
                        self.nearest_ancestor_where(id, |node| node.component == "Dropdown")
                            .map(|node| node.id.clone())
                    });
                let option_invalidation = option_parent
                    .as_ref()
                    .map(|id| state.set_dropdown_open(id.clone(), false))
                    .unwrap_or(Invalidation::None);
                let invalidation = state
                    .set_pressed(None)
                    .merge(state.set_capture(None))
                    .merge(dropdown_invalidation)
                    .merge(option_invalidation)
                    .merge(text_command_invalidation);
                if overlay_action {
                    invalidation.merge(state.dismiss_overlay(crate::DismissReason::Action))
                } else {
                    invalidation
                }
            }
            UiEvent::PointerCancelled => {
                if let Some(message) = self.cancelled_drag_message(state) {
                    outcome.messages.push(message);
                }
                state
                    .set_pressed(None)
                    .merge(state.set_capture(None))
                    .merge(Invalidation::Paint)
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
            UiEvent::ControllerUp | UiEvent::KeyboardNavigateUp => {
                self.move_controller_spatial(state, ControllerDirection::Up, &mut outcome.messages)
            }
            UiEvent::ControllerDown | UiEvent::KeyboardNavigateDown => self
                .move_controller_spatial(state, ControllerDirection::Down, &mut outcome.messages),
            UiEvent::KeyboardNavigatePageUp => {
                let moved = self.move_controller_page(state, -1);
                if moved == Invalidation::None {
                    self.operate_navigation_scroll(state, -1.0, &mut outcome.messages)
                        .unwrap_or(Invalidation::None)
                } else {
                    moved
                }
            }
            UiEvent::KeyboardNavigatePageDown => {
                let moved = self.move_controller_page(state, 1);
                if moved == Invalidation::None {
                    self.operate_navigation_scroll(state, 1.0, &mut outcome.messages)
                        .unwrap_or(Invalidation::None)
                } else {
                    moved
                }
            }
            UiEvent::KeyboardNavigateStart => self.move_controller_boundary(state, false),
            UiEvent::KeyboardNavigateEnd => self.move_controller_boundary(state, true),
            UiEvent::ControllerLeft | UiEvent::KeyboardNavigateLeft => {
                let vertical_scope = state
                    .navigation()
                    .controller_scope()
                    .and_then(|id| self.scope_policy(id))
                    .is_some_and(|scope| scope.traversal == crate::NavigationTraversal::Vertical);
                if state.navigation().controller_editing() {
                    self.move_controller_spatial(
                        state,
                        ControllerDirection::Left,
                        &mut outcome.messages,
                    )
                } else if vertical_scope {
                    let scope = state
                        .navigation()
                        .controller_scope()
                        .cloned()
                        .expect("vertical scope is active");
                    self.leave_controller_scope(state, &scope)
                } else if state.navigation().controller_scope().is_none()
                    && state.navigation().controller_pane().is_some()
                {
                    self.switch_controller_pane(state, -1)
                } else {
                    self.move_controller_spatial(
                        state,
                        ControllerDirection::Left,
                        &mut outcome.messages,
                    )
                }
            }
            UiEvent::ControllerRight | UiEvent::KeyboardNavigateRight => {
                if let Some(invalidation) =
                    self.enter_selected_controller_scope(state, &mut outcome.messages)
                {
                    invalidation
                } else if state.navigation().controller_scope().is_none()
                    && state.navigation().controller_pane().is_some()
                {
                    self.switch_controller_pane(state, 1)
                } else {
                    self.move_controller_spatial(
                        state,
                        ControllerDirection::Right,
                        &mut outcome.messages,
                    )
                }
            }
            UiEvent::ControllerNext => self.move_controller(state, 1, true),
            UiEvent::ControllerPrevious => self.move_controller(state, -1, true),
            UiEvent::ControllerPreviousPane => self.switch_controller_pane(state, -1),
            UiEvent::ControllerNextPane => self.switch_controller_pane(state, 1),
            UiEvent::ControllerAdjust(direction) => {
                if !state.navigation().controller_editing() {
                    Invalidation::None
                } else if let Some((value, step, map)) =
                    state.navigation().controller_selected().and_then(|id| {
                        let node = self.resolved.nodes.iter().find(|node| &node.id == id)?;
                        let map = self
                            .messages
                            .iter()
                            .find(|region| &region.id == id)?
                            .message_mapper?;
                        Some((node.controller_value?, node.adjustment_step, map))
                    })
                {
                    outcome
                        .messages
                        .push(map((value + direction.signum() * step).clamp(0.0, 1.0)));
                    Invalidation::Layout
                } else if let Some(invalidation) =
                    self.operate_navigation_scroll(state, direction, &mut outcome.messages)
                {
                    invalidation
                } else {
                    Invalidation::None
                }
            }
            UiEvent::ControllerBack | UiEvent::KeyboardNavigateBack => {
                if state.open_overlay_id().is_some()
                    && self
                        .active_overlay_dismiss
                        .is_none_or(|policy| policy.cancel)
                {
                    let dismissed = state.dismiss_overlay(crate::DismissReason::Cancel);
                    let restored = state.focused().cloned();
                    dismissed.merge(state.navigation_mut().set_controller_selected(restored))
                } else if state.navigation().controller_editing() {
                    state.navigation_mut().set_controller_editing(false)
                } else if let Some(scope) = state.navigation().controller_scope().cloned() {
                    let close_menu = if self
                        .resolved
                        .nodes
                        .iter()
                        .find(|node| node.id == scope)
                        .is_some_and(|node| node.component == "Dropdown")
                    {
                        state.set_dropdown_open(scope.clone(), false)
                    } else {
                        Invalidation::None
                    };
                    close_menu.merge(self.leave_controller_scope(state, &scope))
                } else {
                    state
                        .navigation_mut()
                        .set_controller_selected(None)
                        .merge(state.navigation_mut().set_controller_pane(None))
                }
            }
            UiEvent::ActivateFocused | UiEvent::KeyboardActivate => {
                if let Some(target) = state.focused().cloned()
                    && let Some(invalidation) =
                        self.activate_text_command(state, &target, &mut outcome)
                {
                    return EventOutcome {
                        invalidation: invalidation
                            .merge(state.dismiss_overlay(crate::DismissReason::Action)),
                        ..outcome
                    };
                }
                if let Some(message) = state
                    .focused()
                    .and_then(|id| self.message_for_id(id))
                    .cloned()
                {
                    outcome.messages.push(message);
                }
                Invalidation::None
            }
            UiEvent::ControllerActivate | UiEvent::KeyboardNavigateActivate => {
                let selected = state
                    .navigation()
                    .controller_selected()
                    .or_else(|| state.focused())
                    .cloned();
                if let Some(target) = selected.as_ref()
                    && let Some(invalidation) =
                        self.activate_text_command(state, target, &mut outcome)
                {
                    return EventOutcome {
                        invalidation: invalidation
                            .merge(state.dismiss_overlay(crate::DismissReason::Action)),
                        ..outcome
                    };
                }
                if let Some(node) = selected
                    .as_ref()
                    .and_then(|id| self.resolved.nodes.iter().find(|node| &node.id == id))
                {
                    if node.component == "Dropdown" {
                        let dropdown = node.id.clone();
                        if let Some(message) = self.message_for_id(&dropdown).cloned() {
                            outcome.messages.push(message);
                        }
                        let first_option = self
                            .messages
                            .iter()
                            .find(|region| region.navigation_owner.as_ref() == Some(&dropdown))
                            .map(|region| region.id.clone());
                        let invalidation = state
                            .navigation_mut()
                            .set_controller_scope(Some(dropdown.clone()))
                            .merge(state.navigation_mut().set_controller_selected(None))
                            .merge(state.set_dropdown_open(dropdown.clone(), true))
                            .merge(first_option.map_or(Invalidation::None, |option| {
                                self.select_controller_id(state, option)
                            }));
                        return EventOutcome {
                            messages: outcome.messages,
                            invalidation,
                            clipboard_text: None,
                        };
                    }
                    // A scroll owner can also be a navigation waypoint. Confirm
                    // must enter adjustment mode so its advertised value actions
                    // remain operable; directional navigation still enters the
                    // nested scope through `enter_selected_controller_scope`.
                    if (node.controller_value.is_some() && node.adjustment_step > 0.0)
                        || (self.scrolls.iter().any(|scroll| scroll.id == node.id)
                            && node.semantic_actions.iter().any(|action| {
                                matches!(action, ActionKind::Increment | ActionKind::Decrement)
                            }))
                    {
                        return EventOutcome {
                            messages: outcome.messages,
                            invalidation: state.navigation_mut().set_controller_editing(true),
                            clipboard_text: None,
                        };
                    }
                    if node.navigation_scope.is_some() {
                        let scope = node.id.clone();
                        state.navigation_mut().set_controller_scope(Some(scope));
                        state.navigation_mut().set_controller_selected(None);
                        let dropdown_invalidation = if node.component == "Dropdown" {
                            state.set_dropdown_open(node.id.clone(), true)
                        } else {
                            Invalidation::None
                        };
                        let invalidation = self.select_scope_entry(state, &node.id);
                        return EventOutcome {
                            messages: outcome.messages,
                            invalidation: invalidation.merge(dropdown_invalidation),
                            clipboard_text: None,
                        };
                    }
                }
                if let Some(message) = selected
                    .as_ref()
                    .and_then(|id| self.message_for_id(id))
                    .cloned()
                {
                    outcome.messages.push(message);
                }
                let dropdown = selected
                    .as_ref()
                    .and_then(|id| {
                        self.nearest_ancestor_where(id, |node| node.component == "Dropdown")
                    })
                    .map(|parent| {
                        let parent = parent.id.clone();
                        state
                            .set_dropdown_open(parent.clone(), false)
                            .merge(state.navigation_mut().set_controller_scope(None))
                            .merge(self.select_controller_id(state, parent))
                    })
                    .unwrap_or(Invalidation::None);
                let overlay_action = selected.as_ref().is_some_and(|selected| {
                    self.active_overlay.as_ref().is_some_and(|(overlay, _)| {
                        self.is_descendant_or_self(overlay.as_ui_id(), selected)
                    })
                });
                if overlay_action {
                    let dismissed = state.dismiss_overlay(crate::DismissReason::Action);
                    let restored = state.focused().cloned();
                    dropdown
                        .merge(dismissed)
                        .merge(state.navigation_mut().set_controller_selected(restored))
                } else {
                    dropdown
                }
            }
            UiEvent::ControllerContextMenu => {
                if let Some(target) = state
                    .navigation()
                    .controller_selected()
                    .or_else(|| state.focused())
                    .cloned()
                    && let Some(invalidation) = self.open_text_context(state, &target, None)
                {
                    return EventOutcome {
                        invalidation,
                        ..outcome
                    };
                }
                if let Some(target) = state.selection_owner().cloned()
                    && let Some(invalidation) =
                        self.open_read_only_text_context(state, &target, None)
                {
                    return EventOutcome {
                        invalidation,
                        ..outcome
                    };
                }
                if let Some((target, overlay)) = state
                    .navigation()
                    .controller_selected()
                    .or_else(|| state.focused())
                    .and_then(|id| {
                        self.overlay_invokers
                            .iter()
                            .find(|(target, _)| target == id)
                    })
                    .cloned()
                {
                    return EventOutcome {
                        invalidation: state.open_overlay(overlay, target),
                        ..outcome
                    };
                }
                let target = state
                    .navigation()
                    .controller_selected()
                    .or_else(|| state.focused())
                    .cloned();
                if let Some(message) = target
                    .as_ref()
                    .and_then(|id| self.context_message_for_id(id))
                    .cloned()
                {
                    outcome.messages.push(message);
                }
                target
                    .filter(|id| self.is_dropdown(id))
                    .map_or(Invalidation::None, |id| state.set_dropdown_open(id, true))
            }
            UiEvent::KeyboardContextMenu => {
                if let Some(target) = state.focused().cloned()
                    && let Some(invalidation) = self.open_text_context(state, &target, None)
                {
                    return EventOutcome {
                        invalidation,
                        ..outcome
                    };
                }
                if let Some(target) = state.selection_owner().cloned()
                    && let Some(invalidation) =
                        self.open_read_only_text_context(state, &target, None)
                {
                    return EventOutcome {
                        invalidation,
                        ..outcome
                    };
                }
                if let Some((target, overlay)) = state
                    .focused()
                    .and_then(|id| {
                        self.overlay_invokers
                            .iter()
                            .find(|(target, _)| target == id)
                    })
                    .cloned()
                {
                    return EventOutcome {
                        invalidation: state.open_overlay(overlay, target),
                        ..outcome
                    };
                }
                let target = state.focused().cloned();
                if let Some(message) = target
                    .as_ref()
                    .and_then(|id| self.context_message_for_id(id))
                    .cloned()
                {
                    outcome.messages.push(message);
                }
                target
                    .filter(|id| self.is_dropdown(id))
                    .map_or(Invalidation::None, |id| state.set_dropdown_open(id, true))
            }
            UiEvent::AccessibilityFocus(id) => {
                if self.hits.iter().any(|hit| hit.id == id) {
                    state.set_focus(Some(id))
                } else {
                    Invalidation::None
                }
            }
            UiEvent::AccessibilityActivate(id) => {
                if let Some(invalidation) = self.activate_text_command(state, &id, &mut outcome) {
                    return EventOutcome {
                        invalidation: invalidation
                            .merge(state.dismiss_overlay(crate::DismissReason::Action)),
                        ..outcome
                    };
                }
                if let Some(message) = self.message_for_id(&id).cloned() {
                    outcome.messages.push(message);
                }
                Invalidation::None
            }
            UiEvent::AccessibilityContextMenu(id) => {
                if let Some(invalidation) = self.open_text_context(state, &id, None) {
                    return EventOutcome {
                        invalidation,
                        ..outcome
                    };
                }
                if let Some(invalidation) = self.open_read_only_text_context(state, &id, None) {
                    return EventOutcome {
                        invalidation,
                        ..outcome
                    };
                }
                if let Some((target, overlay)) = self
                    .overlay_invokers
                    .iter()
                    .find(|(target, _)| target == &id)
                    .cloned()
                {
                    return EventOutcome {
                        invalidation: state.open_overlay(overlay, target),
                        ..outcome
                    };
                }
                if let Some(message) = self.context_message_for_id(&id).cloned() {
                    outcome.messages.push(message);
                }
                if self.is_dropdown(&id) {
                    state.set_dropdown_open(id, true)
                } else {
                    Invalidation::None
                }
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
            UiEvent::TextUndo => self
                .execute_focused_text_command(
                    state,
                    crate::TextEditCommand::Undo,
                    None,
                    &mut outcome,
                )
                .unwrap_or(Invalidation::None),
            UiEvent::TextRedo => self
                .execute_focused_text_command(
                    state,
                    crate::TextEditCommand::Redo,
                    None,
                    &mut outcome,
                )
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
                if let Some(invalidation) = self.execute_focused_text_command(
                    state,
                    crate::TextEditCommand::SelectAll,
                    None,
                    &mut outcome,
                ) {
                    invalidation
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
                if let Some(invalidation) = self.execute_focused_text_command(
                    state,
                    crate::TextEditCommand::Copy,
                    None,
                    &mut outcome,
                ) {
                    invalidation
                } else {
                    outcome.clipboard_text = self.selected_text(state);
                    Invalidation::None
                }
            }
            UiEvent::TextCut => self
                .execute_focused_text_command(
                    state,
                    crate::TextEditCommand::Cut,
                    None,
                    &mut outcome,
                )
                .unwrap_or(Invalidation::None),
            UiEvent::TextPaste(text) => self
                .execute_focused_text_command(
                    state,
                    crate::TextEditCommand::Paste,
                    Some(&text),
                    &mut outcome,
                )
                .unwrap_or(Invalidation::None),
            UiEvent::SelectionClear => state.clear_document_selection(),
            UiEvent::Dismiss => {
                let overlay = state.dismiss_overlay(crate::DismissReason::Cancel);
                self.resolved_layout()
                    .nodes()
                    .iter()
                    .filter(|node| node.component == "Dropdown")
                    .fold(overlay, |invalidation, node| {
                        invalidation.merge(state.set_dropdown_open(node.id.clone(), false))
                    })
            }
            UiEvent::CaretBlink => state.toggle_caret(),
            UiEvent::FocusGained => state.set_window_focused(true),
            UiEvent::FocusLost => {
                if let Some(message) = self.cancelled_drag_message(state) {
                    outcome.messages.push(message);
                }
                let dismissed = self
                    .resolved_layout()
                    .nodes()
                    .iter()
                    .filter(|node| node.component == "Dropdown")
                    .fold(Invalidation::None, |invalidation, node| {
                        invalidation.merge(state.set_dropdown_open(node.id.clone(), false))
                    });
                state
                    .set_window_focused(false)
                    .merge(state.focus_lost())
                    .merge(dismissed)
            }
            UiEvent::Suspended => {
                if let Some(message) = self.cancelled_drag_message(state) {
                    outcome.messages.push(message);
                }
                state.suspended()
            }
            UiEvent::DeviceRemoved => {
                if let Some(message) = self.cancelled_drag_message(state) {
                    outcome.messages.push(message);
                }
                state.device_removed()
            }
        };
        if keyboard_tree_navigation {
            outcome.invalidation = outcome.invalidation.merge(state.set_focus(None));
        }
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

    fn open_text_context(
        &self,
        state: &mut UiStateStore,
        target: &UiId,
        point: Option<Point>,
    ) -> Option<Invalidation> {
        let input = self.text_inputs.iter().find(|input| &input.id == target)?;
        let editor = state.editor(target.clone(), &input.initial);
        // Opening a menu cancels composition; it never commits preedit implicitly.
        editor.cancel_preedit();
        if let Some(point) = point {
            let offset = text_offset_at(input, point);
            let inside_selection = editor
                .selection()
                .is_some_and(|selection| selection.contains(&offset) || offset == selection.end);
            if !inside_selection {
                editor.place_cursor(offset);
            }
        }
        let document_generation = editor.document_generation();
        let selection_generation = editor.selection_generation();
        let secure = input.secure;
        let anchor = point.map_or_else(
            || crate::OverlayAnchor::InvocationTargetCenter(target.clone()),
            |point| crate::OverlayAnchor::Point {
                invocation_target: target.clone(),
                point,
            },
        );
        Some(
            state
                .set_focus(Some(target.clone()))
                .merge(state.open_text_context(
                    target.clone(),
                    document_generation,
                    selection_generation,
                    secure,
                    anchor,
                )),
        )
    }

    fn open_read_only_text_context(
        &self,
        state: &mut UiStateStore,
        target: &UiId,
        point: Option<Point>,
    ) -> Option<Invalidation> {
        let region = self.selection_region(target)?;
        let selection = state.document_selection(target)?;
        let anchor = point.map_or_else(
            || crate::OverlayAnchor::InvocationTargetCenter(target.clone()),
            |point| crate::OverlayAnchor::Point {
                invocation_target: target.clone(),
                point,
            },
        );
        Some(state.open_read_only_text_context(
            target.clone(),
            selection_document_generation(&region.document),
            document_selection_generation(selection),
            anchor,
        ))
    }

    fn activate_text_command(
        &self,
        state: &mut UiStateStore,
        target: &UiId,
        outcome: &mut EventOutcome<Message>,
    ) -> Option<Invalidation> {
        let command = self.text_commands.iter().find(|item| &item.id == target)?;
        let session = state.text_context()?.clone();
        if command.editor != session.editor {
            state.clear_text_context();
            return Some(Invalidation::Layout);
        }
        if !session.editable {
            let region = self.selection_region(&session.editor)?;
            let selection = state.document_selection(&session.editor)?;
            if selection_document_generation(&region.document) != session.document_generation
                || document_selection_generation(selection) != session.selection_generation
            {
                state.clear_text_context();
                return Some(Invalidation::Layout);
            }
            match command.command {
                crate::TextEditCommand::Copy => {
                    outcome.clipboard_text = region.document.selected_text(selection);
                    return Some(Invalidation::None);
                }
                crate::TextEditCommand::SelectAll => {
                    *state.document_selection_mut(session.editor) = region.document.select_all();
                    return Some(Invalidation::Paint);
                }
                _ => return Some(Invalidation::None),
            }
        }
        let input = self
            .text_inputs
            .iter()
            .find(|input| input.id == session.editor && input.secure == session.secure)?;
        let clipboard = state.clipboard_text().map(ToOwned::to_owned);
        let editor = state.editor(session.editor.clone(), &input.initial);
        if editor.document_generation() != session.document_generation
            || editor.selection_generation() != session.selection_generation
        {
            state.clear_text_context();
            return Some(Invalidation::Layout);
        }
        let effect = crate::execute_text_command(
            editor,
            crate::TextContextPolicy {
                editable: true,
                secure: session.secure,
                clipboard_has_text: clipboard.is_some(),
            },
            command.command,
            clipboard.as_deref(),
        );
        if effect.changed {
            outcome.messages.push((input.map)(editor.text().to_owned()));
        }
        outcome.clipboard_text = effect.clipboard_text;
        Some(match command.command {
            crate::TextEditCommand::Copy => Invalidation::None,
            crate::TextEditCommand::SelectAll => Invalidation::Paint,
            _ if effect.changed => Invalidation::Layout,
            _ => Invalidation::None,
        })
    }

    fn execute_focused_text_command(
        &self,
        state: &mut UiStateStore,
        command: crate::TextEditCommand,
        clipboard: Option<&str>,
        outcome: &mut EventOutcome<Message>,
    ) -> Option<Invalidation> {
        let id = state.focused()?.clone();
        let input = self.text_inputs.iter().find(|input| input.id == id)?;
        let editor = state.editor(id, &input.initial);
        let effect = crate::execute_text_command(
            editor,
            crate::TextContextPolicy {
                editable: true,
                secure: input.secure,
                clipboard_has_text: clipboard.is_some(),
            },
            command,
            clipboard,
        );
        if effect.changed {
            outcome.messages.push((input.map)(editor.text().to_owned()));
        }
        outcome.clipboard_text = effect.clipboard_text;
        Some(match command {
            crate::TextEditCommand::Copy => Invalidation::None,
            crate::TextEditCommand::SelectAll => Invalidation::Paint,
            _ if effect.changed => Invalidation::Layout,
            _ => Invalidation::None,
        })
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
        let mut ids = self
            .resolved
            .nodes
            .iter()
            .filter(|node| {
                node.semantic_actions.iter().any(|action| {
                    matches!(
                        action,
                        ActionKind::Activate
                            | ActionKind::ContextMenu
                            | ActionKind::SetValue
                            | ActionKind::Increment
                            | ActionKind::Decrement
                    )
                })
            })
            .filter(|node| {
                self.active_overlay.as_ref().is_none_or(|(overlay, _)| {
                    self.is_descendant_or_self(overlay.as_ui_id(), &node.id)
                })
            })
            .map(|node| &node.id)
            .collect::<Vec<_>>();
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
        let target = ids[next].clone();
        state
            .set_focus(Some(target.clone()))
            .merge(self.reveal_controller_target(state, &target))
    }

    fn move_controller(
        &self,
        state: &mut UiStateStore,
        direction: isize,
        continue_in_parent: bool,
    ) -> Invalidation {
        let ids = self.controller_targets_with_scrolls(state, continue_in_parent);
        let at_scope_edge = state
            .navigation()
            .controller_selected()
            .and_then(|selected| ids.iter().position(|id| id == selected))
            .is_some_and(|index| {
                if direction.is_negative() {
                    index == 0
                } else {
                    index + 1 == ids.len()
                }
            });
        if at_scope_edge && let Some(scope) = state.navigation().controller_scope().cloned() {
            return match self.scope_policy(&scope).map(|policy| policy.exit) {
                Some(crate::NavigationExit::Parent) => {
                    let mut invalidation = self.leave_controller_scope(state, &scope);
                    if continue_in_parent {
                        let parent_targets =
                            self.controller_targets_with_scrolls(state, continue_in_parent);
                        invalidation = invalidation.merge(self.select_controller_from(
                            state,
                            direction,
                            parent_targets,
                        ));
                    }
                    invalidation
                }
                Some(crate::NavigationExit::Dismiss) => self.leave_controller_scope(state, &scope),
                Some(crate::NavigationExit::Contain) | None => Invalidation::None,
            };
        }
        self.select_controller_from(state, direction, ids)
    }

    fn move_controller_boundary(&self, state: &mut UiStateStore, end: bool) -> Invalidation {
        let ids = self.controller_targets(state);
        let target = if end { ids.last() } else { ids.first() };
        target.cloned().map_or(Invalidation::None, |target| {
            self.select_controller_id(state, target)
        })
    }

    fn move_controller_page(&self, state: &mut UiStateStore, direction: isize) -> Invalidation {
        let ids = self.controller_targets(state);
        if ids.is_empty() {
            return Invalidation::None;
        }
        let Some(current) = state
            .navigation()
            .controller_selected()
            .filter(|selected| ids.contains(selected))
            .cloned()
        else {
            let target = if direction.is_negative() {
                ids.last().cloned()
            } else {
                ids.first().cloned()
            };
            return target.map_or(Invalidation::None, |target| {
                self.select_controller_id(state, target)
            });
        };
        let Some(current_index) = ids.iter().position(|id| id == &current) else {
            return Invalidation::None;
        };
        let Some(current_rect) = self.controller_rect(&current) else {
            return Invalidation::None;
        };
        let viewport_height = self
            .scrolls
            .iter()
            .rev()
            .find(|scroll| {
                current_rect.origin.x < scroll.rect.origin.x + scroll.rect.size.width
                    && current_rect.origin.x + current_rect.size.width > scroll.rect.origin.x
            })
            .map_or(current_rect.size.height.max(1.0), |scroll| {
                scroll.rect.size.height.max(current_rect.size.height)
            });
        let desired_y = current_rect.origin.y + direction as f32 * viewport_height;
        let candidates: Box<dyn Iterator<Item = (usize, &UiId)>> = if direction.is_negative() {
            Box::new(ids[..current_index].iter().enumerate())
        } else {
            Box::new(ids.iter().enumerate().skip(current_index + 1))
        };
        let target = candidates
            .filter_map(|(index, id)| {
                let rect = self.controller_rect(id)?;
                Some((index, id, (rect.origin.y - desired_y).abs()))
            })
            .min_by(|left, right| left.2.total_cmp(&right.2))
            .map(|(_, id, _)| id.clone())
            .unwrap_or_else(|| {
                if direction.is_negative() {
                    ids[0].clone()
                } else {
                    ids[ids.len() - 1].clone()
                }
            });
        self.select_controller_id(state, target)
    }

    fn context_message_for_id(&self, id: &UiId) -> Option<&Message> {
        self.context_messages
            .iter()
            .rev()
            .find(|region| &region.id == id)
            .map(|region| &region.message)
    }

    fn is_dropdown(&self, id: &UiId) -> bool {
        self.resolved
            .find(id)
            .is_some_and(|node| node.component == "Dropdown")
    }

    fn enter_selected_controller_scope(
        &self,
        state: &mut UiStateStore,
        messages: &mut Vec<Message>,
    ) -> Option<Invalidation> {
        let selected = state
            .navigation()
            .controller_selected()
            .or_else(|| state.focused())
            .cloned()?;
        let node = self
            .resolved
            .nodes
            .iter()
            .find(|node| node.id == selected)?;
        if node.component == "Dropdown" {
            if let Some(message) = self.message_for_id(&selected).cloned() {
                messages.push(message);
            }
            let first_option = self
                .messages
                .iter()
                .find(|region| region.navigation_owner.as_ref() == Some(&selected))
                .map(|region| region.id.clone());
            let invalidation = state
                .navigation_mut()
                .set_controller_scope(Some(selected.clone()))
                .merge(state.navigation_mut().set_controller_selected(None))
                .merge(state.set_dropdown_open(selected, true))
                .merge(first_option.map_or(Invalidation::None, |option| {
                    self.select_controller_id(state, option)
                }));
            return Some(invalidation);
        }
        node.navigation_scope.as_ref()?;
        state
            .navigation_mut()
            .set_controller_scope(Some(selected.clone()));
        state.navigation_mut().set_controller_selected(None);
        Some(self.select_scope_entry(state, &selected))
    }

    fn move_controller_spatial(
        &self,
        state: &mut UiStateStore,
        direction: ControllerDirection,
        messages: &mut Vec<Message>,
    ) -> Invalidation {
        if state.navigation().controller_editing() {
            return match direction {
                ControllerDirection::Left => self.adjust_controller_value(state, -1.0, messages),
                ControllerDirection::Right => self.adjust_controller_value(state, 1.0, messages),
                ControllerDirection::Up | ControllerDirection::Down => Invalidation::None,
            };
        }
        if let Some(scope) = state
            .navigation()
            .controller_scope()
            .or_else(|| state.navigation().controller_pane())
            .and_then(|id| self.scope_policy(id))
            && matches!(
                scope.traversal,
                crate::NavigationTraversal::Linear | crate::NavigationTraversal::Vertical
            )
        {
            let direction = match (scope.traversal, direction, scope.direction) {
                (crate::NavigationTraversal::Vertical, ControllerDirection::Up, _) => Some(-1),
                (crate::NavigationTraversal::Vertical, ControllerDirection::Down, _) => Some(1),
                (crate::NavigationTraversal::Vertical, _, _) => None,
                (_, ControllerDirection::Up, _) => Some(-1),
                (_, ControllerDirection::Down, _) => Some(1),
                (_, ControllerDirection::Left, crate::ReadingDirection::LeftToRight)
                | (_, ControllerDirection::Right, crate::ReadingDirection::RightToLeft) => Some(-1),
                (_, ControllerDirection::Right, crate::ReadingDirection::LeftToRight)
                | (_, ControllerDirection::Left, crate::ReadingDirection::RightToLeft) => Some(1),
            };
            return direction.map_or(Invalidation::None, |direction| {
                self.move_controller(state, direction, false)
            });
        }
        let ids = self.controller_targets(state);
        if ids.is_empty() {
            return state.navigation_mut().set_controller_selected(None);
        }
        let Some(current) = state
            .navigation()
            .controller_selected()
            .filter(|selected| ids.contains(selected))
            .cloned()
        else {
            return self.select_controller_id(state, ids[0].clone());
        };
        let Some(current_rect) = self.controller_rect(&current) else {
            return self.select_controller_id(state, ids[0].clone());
        };
        let declared_neighbor = self
            .resolved
            .find(&current)
            .filter(|node| node.navigation_scope.is_some())
            .or_else(|| {
                self.nearest_ancestor_where(&current, |node| node.navigation_scope.is_some())
            })
            .and_then(|scope| scope.navigation_scope.as_ref())
            .and_then(|scope| scope.neighbors.target(direction.into()))
            .filter(|target| ids.contains(target))
            .cloned();
        if let Some(target) = declared_neighbor {
            return self.select_controller_id(state, target);
        }
        let current_center = Point {
            x: current_rect.origin.x + current_rect.size.width / 2.0,
            y: current_rect.origin.y + current_rect.size.height / 2.0,
        };
        let mut candidates = ids
            .iter()
            .filter(|id| *id != &current)
            .cloned()
            .filter_map(|id| {
                let rect = self.controller_rect(&id)?;
                let center = Point {
                    x: rect.origin.x + rect.size.width / 2.0,
                    y: rect.origin.y + rect.size.height / 2.0,
                };
                let dx = center.x - current_center.x;
                let dy = center.y - current_center.y;
                let (primary, secondary) = match direction {
                    ControllerDirection::Up if dy < -0.5 => (-dy, dx.abs()),
                    ControllerDirection::Down if dy > 0.5 => (dy, dx.abs()),
                    ControllerDirection::Left if dx < -0.5 => (-dx, dy.abs()),
                    ControllerDirection::Right if dx > 0.5 => (dx, dy.abs()),
                    _ => return None,
                };
                let directional_score = primary + secondary * 4.0;
                Some((id, directional_score, primary, secondary, dx * dx + dy * dy))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            left.1
                .total_cmp(&right.1)
                .then_with(|| left.2.total_cmp(&right.2))
                .then_with(|| left.3.total_cmp(&right.3))
                .then_with(|| left.4.total_cmp(&right.4))
                .then_with(|| left.0.cmp(&right.0))
        });
        if let Some((id, _, _, _, _)) = candidates.into_iter().next() {
            return self.select_controller_id(state, id);
        }

        // Large nested scopes can geometrically contain the current target,
        // leaving their centers neither strictly ahead nor behind. Preserve
        // spatial edge behavior for ordinary controls, but allow a declared
        // scope waypoint to be entered in structural order.
        let current_index = ids.iter().position(|id| id == &current).unwrap_or(0);
        let structural_scope = match direction {
            ControllerDirection::Down | ControllerDirection::Right => {
                ids.iter().skip(current_index + 1).find(|id| {
                    self.resolved
                        .find(id)
                        .is_some_and(|node| node.navigation_scope.is_some())
                })
            }
            ControllerDirection::Up | ControllerDirection::Left => {
                ids[..current_index].iter().rev().find(|id| {
                    self.resolved
                        .find(id)
                        .is_some_and(|node| node.navigation_scope.is_some())
                })
            }
        };
        structural_scope
            .cloned()
            .map(|id| self.select_controller_id(state, id))
            .unwrap_or(Invalidation::None)
    }

    fn adjust_controller_value(
        &self,
        state: &UiStateStore,
        direction: f32,
        messages: &mut Vec<Message>,
    ) -> Invalidation {
        let Some((value, step, map)) = state.navigation().controller_selected().and_then(|id| {
            let node = self.resolved.nodes.iter().find(|node| &node.id == id)?;
            let map = self
                .messages
                .iter()
                .find(|region| &region.id == id)?
                .message_mapper?;
            Some((node.controller_value?, node.adjustment_step, map))
        }) else {
            return Invalidation::None;
        };
        messages.push(map((value + direction.signum() * step).clamp(0.0, 1.0)));
        Invalidation::Layout
    }

    fn node_has_controller_action(&self, index: usize) -> bool {
        let node = &self.resolved.nodes[index];
        self.messages.iter().any(|region| region.id == node.id)
            || node
                .children
                .iter()
                .any(|child| self.node_has_controller_action(*child))
    }

    fn node_index(&self, id: &UiId) -> Option<usize> {
        self.resolved.nodes.iter().position(|node| &node.id == id)
    }

    fn subtree_contains_id(&self, index: usize, id: &UiId) -> bool {
        let node = &self.resolved.nodes[index];
        node.id == *id
            || node
                .children
                .iter()
                .any(|child| self.subtree_contains_id(*child, id))
    }

    pub(crate) fn is_descendant_or_self(&self, ancestor: &UiId, candidate: &UiId) -> bool {
        self.node_index(ancestor)
            .is_some_and(|index| self.subtree_contains_id(index, candidate))
            || self.messages.iter().any(|region| {
                region.id == *candidate
                    && region.navigation_owner.as_ref().is_some_and(|owner| {
                        owner == ancestor || self.is_descendant_or_self(ancestor, owner)
                    })
            })
    }

    fn navigation_owner(&self, id: &UiId) -> Option<&UiId> {
        self.messages
            .iter()
            .find(|region| &region.id == id)
            .and_then(|region| region.navigation_owner.as_ref())
    }

    fn nearest_ancestor_where(
        &self,
        candidate: &UiId,
        include: impl Fn(&ResolvedNode) -> bool,
    ) -> Option<&ResolvedNode> {
        let direct_owner = self.navigation_owner(candidate);
        if let Some(owner) = direct_owner
            && let Some(node) = self.resolved.nodes.iter().find(|node| &node.id == owner)
            && include(node)
        {
            return Some(node);
        }
        let candidate = direct_owner.unwrap_or(candidate);
        self.resolved
            .nodes
            .iter()
            .filter(|node| node.id != *candidate && include(node))
            .filter(|node| self.is_descendant_or_self(&node.id, candidate))
            .min_by_key(|node| self.subtree_size(&node.id).unwrap_or(usize::MAX))
    }

    fn subtree_size(&self, id: &UiId) -> Option<usize> {
        fn count(nodes: &[ResolvedNode], index: usize) -> usize {
            1 + nodes[index]
                .children
                .iter()
                .map(|child| count(nodes, *child))
                .sum::<usize>()
        }
        self.node_index(id)
            .map(|index| count(&self.resolved.nodes, index))
    }

    fn collect_controller_targets(
        &self,
        index: usize,
        root: bool,
        include_scrolls: bool,
        targets: &mut Vec<UiId>,
    ) {
        let node = &self.resolved.nodes[index];
        // A nested scope is itself a controller waypoint even when its
        // container has no activation message. Confirm enters the scope; its
        // descendants remain hidden from the parent traversal until then.
        if !root && node.navigation_scope.is_some() && self.node_has_controller_action(index) {
            targets.push(node.id.clone());
            return;
        }
        let is_scroll_target = self.scrolls.iter().any(|scroll| scroll.id == node.id)
            && node
                .semantic_actions
                .iter()
                .any(|action| matches!(action, ActionKind::Increment | ActionKind::Decrement));
        // A scroll viewport is directly adjustable, but unlike a button it
        // must not hide the controls contained in its viewport from traversal.
        if include_scrolls && is_scroll_target {
            targets.push(node.id.clone());
        }
        if (!root || node.navigation_scope.is_none())
            && node.semantic_actions.iter().any(|action| {
                matches!(
                    action,
                    ActionKind::Activate
                        | ActionKind::ContextMenu
                        | ActionKind::Increment
                        | ActionKind::Decrement
                        | ActionKind::SetValue
                )
            })
            && !is_scroll_target
        {
            targets.push(node.id.clone());
            return;
        }
        for child in &node.children {
            self.collect_controller_targets(*child, false, include_scrolls, targets);
        }
    }

    fn controller_targets(&self, state: &UiStateStore) -> Vec<UiId> {
        self.controller_targets_with_scrolls(state, false)
    }

    fn controller_targets_with_scrolls(
        &self,
        state: &UiStateStore,
        include_scrolls: bool,
    ) -> Vec<UiId> {
        if let Some((overlay, _)) = &self.active_overlay {
            let mut targets = self
                .messages
                .iter()
                .filter(|region| self.is_descendant_or_self(overlay.as_ui_id(), &region.id))
                .map(|region| region.id.clone())
                .collect::<Vec<_>>();
            targets.dedup();
            return targets;
        }
        if state.navigation().controller_scope().is_none()
            && state.navigation().controller_pane().is_none()
            && self.resolved.nodes[0].navigation_scope.is_some()
            && self.node_has_controller_action(0)
        {
            return vec![self.resolved.nodes[0].id.clone()];
        }
        let root = state
            .navigation()
            .controller_scope()
            .or_else(|| state.navigation().controller_pane())
            .and_then(|id| self.resolved.nodes.iter().position(|node| &node.id == id))
            .unwrap_or(0);
        let mut targets = Vec::new();
        self.collect_controller_targets(root, true, include_scrolls, &mut targets);
        let root_id = &self.resolved.nodes[root].id;
        if state
            .state(root_id)
            .is_some_and(|entry| entry.dropdown_open)
        {
            targets.extend(
                self.messages
                    .iter()
                    .filter(|region| {
                        region.id != *root_id && self.is_descendant_or_self(root_id, &region.id)
                    })
                    .map(|region| region.id.clone()),
            );
        }
        targets.dedup();
        targets
    }

    fn scope_policy(&self, id: &UiId) -> Option<&crate::NavigationScope> {
        self.resolved.find(id)?.navigation_scope.as_ref()
    }

    fn remember_scope_selection(&self, state: &mut UiStateStore, scope: &UiId) {
        if self
            .scope_policy(scope)
            .is_some_and(|policy| policy.retain_focus)
        {
            if let Some(selected) = state.navigation().controller_selected().cloned() {
                state
                    .navigation_mut()
                    .retain_controller_focus(scope.clone(), selected);
            }
        } else {
            state.navigation_mut().forget_controller_focus(scope);
        }
    }

    fn select_scope_entry(&self, state: &mut UiStateStore, scope: &UiId) -> Invalidation {
        let targets = self.controller_targets(state);
        if targets.is_empty() {
            return state.navigation_mut().set_controller_selected(None);
        }
        let policy = self.scope_policy(scope);
        let retained = policy
            .filter(|policy| policy.retain_focus)
            .and_then(|_| state.navigation().retained_controller_focus(scope))
            .filter(|id| targets.contains(id))
            .cloned();
        let declared = policy.and_then(|policy| match &policy.entry {
            crate::NavigationEntry::First => targets.first().cloned(),
            crate::NavigationEntry::Last => targets.last().cloned(),
            crate::NavigationEntry::Target(id) => targets
                .contains(id)
                .then(|| id.clone())
                .or_else(|| targets.first().cloned()),
        });
        retained
            .or(declared)
            .or_else(|| targets.first().cloned())
            .map(|id| self.select_controller_id(state, id))
            .unwrap_or(Invalidation::None)
    }

    fn leave_controller_scope(&self, state: &mut UiStateStore, scope: &UiId) -> Invalidation {
        let exit = self
            .scope_policy(scope)
            .map_or(crate::NavigationExit::Parent, |policy| policy.exit);
        match exit {
            crate::NavigationExit::Contain => Invalidation::None,
            crate::NavigationExit::Dismiss => {
                self.remember_scope_selection(state, scope);
                state
                    .navigation_mut()
                    .set_controller_scope(None)
                    .merge(state.navigation_mut().set_controller_pane(None))
                    .merge(state.navigation_mut().set_controller_selected(None))
            }
            crate::NavigationExit::Parent => {
                self.remember_scope_selection(state, scope);
                let parent_scope = self
                    .nearest_ancestor_where(scope, |node| node.navigation_scope.is_some())
                    .map(|node| node.id.clone());
                state
                    .navigation_mut()
                    .set_controller_scope(parent_scope)
                    .merge(state.navigation_mut().set_controller_editing(false))
                    .merge(self.select_controller_id(state, scope.clone()))
            }
        }
    }

    fn controller_rect(&self, id: &UiId) -> Option<Rect> {
        self.resolved
            .nodes
            .iter()
            .find(|node| &node.id == id)
            .map(|node| node.allocated)
            .or_else(|| {
                self.messages
                    .iter()
                    .rev()
                    .find(|region| &region.id == id)
                    .map(|region| region.rect)
            })
    }

    fn select_controller_from(
        &self,
        state: &mut UiStateStore,
        direction: isize,
        ids: Vec<UiId>,
    ) -> Invalidation {
        if ids.is_empty() {
            return state.navigation_mut().set_controller_selected(None);
        }
        let current = state
            .navigation()
            .controller_selected()
            .and_then(|selected| ids.iter().position(|id| id == selected));
        let next = match (current, direction.is_negative()) {
            (Some(index), false) => (index + 1) % ids.len(),
            (Some(0), true) | (None, true) => ids.len() - 1,
            (Some(index), true) => index - 1,
            (None, false) => 0,
        };
        self.select_controller_id(state, ids[next].clone())
    }

    fn select_controller_id(&self, state: &mut UiStateStore, selected: UiId) -> Invalidation {
        let mut invalidation = state
            .navigation_mut()
            .set_controller_selected(Some(selected.clone()));
        if self
            .active_overlay
            .as_ref()
            .is_some_and(|(overlay, _)| self.is_descendant_or_self(overlay.as_ui_id(), &selected))
        {
            invalidation = invalidation.merge(state.set_focus(Some(selected.clone())));
        }
        invalidation = invalidation.merge(self.reveal_controller_target(state, &selected));
        invalidation
    }

    fn reveal_controller_target(&self, state: &mut UiStateStore, selected: &UiId) -> Invalidation {
        let Some(target) = self.controller_rect(selected) else {
            return Invalidation::None;
        };
        let declared_owner = state
            .navigation()
            .controller_scope()
            .or_else(|| state.navigation().controller_pane())
            .and_then(|scope| self.scope_policy(scope))
            .and_then(|scope| scope.scroll_owner.as_ref());
        for scroll in self.scrolls.iter().rev().filter(|scroll| {
            declared_owner.is_none_or(|owner| {
                &scroll.id == owner
                    || scroll
                        .id
                        .as_str()
                        .strip_suffix(owner.as_str())
                        .is_some_and(|prefix| prefix.ends_with('/'))
            })
        }) {
            let overlaps_horizontally = target.origin.x
                < scroll.rect.origin.x + scroll.rect.size.width
                && target.origin.x + target.size.width > scroll.rect.origin.x;
            if !overlaps_horizontally {
                continue;
            }
            let margin = 12.0;
            let delta = if target.origin.y < scroll.rect.origin.y + margin {
                target.origin.y - scroll.rect.origin.y - margin
            } else if target.origin.y + target.size.height
                > scroll.rect.origin.y + scroll.rect.size.height - margin
            {
                target.origin.y + target.size.height
                    - (scroll.rect.origin.y + scroll.rect.size.height)
                    + margin
            } else {
                0.0
            };
            if delta != 0.0 {
                return state.scroll_by(
                    scroll.id.clone(),
                    delta,
                    (scroll.extent.content.height - scroll.extent.viewport.height).max(0.0),
                );
            }
        }
        Invalidation::None
    }

    fn switch_controller_pane(&self, state: &mut UiStateStore, direction: isize) -> Invalidation {
        let current_parent = state
            .navigation()
            .controller_pane()
            .and_then(|pane| {
                self.nearest_ancestor_where(pane, |node| node.navigation_scope.is_some())
            })
            .map(|node| node.id.clone());
        let panes = self
            .resolved
            .nodes
            .iter()
            .filter(|node| {
                node.navigation_scope
                    .as_ref()
                    .is_some_and(|scope| scope.pane)
                    && self
                        .nearest_ancestor_where(&node.id, |candidate| {
                            candidate.navigation_scope.is_some()
                        })
                        .map(|parent| parent.id.clone())
                        == current_parent
            })
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        if panes.is_empty() {
            return Invalidation::None;
        }
        let current = state
            .navigation()
            .controller_pane()
            .and_then(|pane| panes.iter().position(|candidate| candidate == pane));
        let next = match (current, direction.is_negative()) {
            (Some(index), false) => (index + 1).min(panes.len() - 1),
            (Some(index), true) => index.saturating_sub(1),
            (None, false) => 0,
            (None, true) => panes.len() - 1,
        };
        if let (Some(pane), Some(selected)) = (
            state.navigation().controller_pane().cloned(),
            state.navigation().controller_selected().cloned(),
        ) {
            let _ = selected;
            self.remember_scope_selection(state, &pane);
        }
        let destination = panes[next].clone();
        let mut invalidation = state
            .navigation_mut()
            .set_controller_pane(Some(destination))
            .merge(state.navigation_mut().set_controller_scope(None))
            .merge(state.navigation_mut().set_controller_editing(false))
            .merge(state.navigation_mut().set_controller_selected(None));
        invalidation = invalidation.merge(self.select_scope_entry(state, &panes[next]));
        invalidation
    }

    /// Moves controller selection among messages accepted by `include`.
    pub fn move_controller_where(
        &self,
        state: &mut UiStateStore,
        direction: isize,
        include: impl Fn(&Message) -> bool,
    ) -> Invalidation {
        let mut ids = self
            .messages
            .iter()
            .filter(|region| {
                include(&region.message)
                    && !self.scrolls.iter().any(|scroll| scroll.id == region.id)
            })
            .map(|region| &region.id)
            .collect::<Vec<_>>();
        ids.dedup();
        if ids.is_empty() {
            return state.navigation_mut().set_controller_selected(None);
        }
        let current = state
            .navigation()
            .controller_selected()
            .and_then(|selected| ids.iter().position(|id| *id == selected));
        let next = match (current, direction.is_negative()) {
            (Some(index), false) => (index + 1) % ids.len(),
            (Some(0), true) | (None, true) => ids.len() - 1,
            (Some(index), true) => index - 1,
            (None, false) => 0,
        };
        let selected = ids[next].clone();
        let mut invalidation = state
            .navigation_mut()
            .set_controller_selected(Some(selected.clone()));
        if let Some(target) = self
            .messages
            .iter()
            .rev()
            .find(|region| region.id == selected)
            .map(|region| region.rect)
        {
            for scroll in self.scrolls.iter().rev() {
                let overlaps_horizontally = target.origin.x
                    < scroll.rect.origin.x + scroll.rect.size.width
                    && target.origin.x + target.size.width > scroll.rect.origin.x;
                if !overlaps_horizontally {
                    continue;
                }
                let margin = 12.0;
                let delta = if target.origin.y < scroll.rect.origin.y + margin {
                    target.origin.y - scroll.rect.origin.y - margin
                } else if target.origin.y + target.size.height
                    > scroll.rect.origin.y + scroll.rect.size.height - margin
                {
                    target.origin.y + target.size.height
                        - (scroll.rect.origin.y + scroll.rect.size.height)
                        + margin
                } else {
                    0.0
                };
                if delta != 0.0 {
                    invalidation = invalidation.merge(state.scroll_by(
                        scroll.id.clone(),
                        delta,
                        (scroll.extent.content.height - scroll.extent.viewport.height).max(0.0),
                    ));
                    break;
                }
            }
        }
        invalidation
    }

    pub fn apply_interaction_state(&mut self, state: &UiStateStore) {
        for node in &mut self.resolved.nodes {
            node.interaction.focused = state.focused() == Some(&node.id);
            node.interaction.hovered = state.hovered() == Some(&node.id);
            node.interaction.pressed = state.pressed() == Some(&node.id);
            node.interaction.captured = state.captured() == Some(&node.id);
            node.interaction.controller_selected =
                state.navigation().controller_selected() == Some(&node.id);
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
                    wrap: true,
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
        self.semantic_parents = vec![None; self.resolved.nodes.len()];
        for (index, node) in self.resolved.nodes.iter().enumerate() {
            for child in &node.children {
                self.semantic_parents[*child] = Some(index);
            }
        }
        self.navigation_depths = vec![0; self.resolved.nodes.len()];
        for index in 0..self.resolved.nodes.len() {
            let parent_depth = self.semantic_parents[index]
                .map(|parent| self.navigation_depths[parent])
                .unwrap_or(0);
            self.navigation_depths[index] =
                parent_depth + usize::from(self.resolved.nodes[index].navigation_scope.is_some());
        }
        self.accessibility = self
            .resolved
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                if node.accessibility_hidden {
                    return None;
                }
                let rect = node.clip.map_or(node.allocated, |clip| {
                    intersection(node.allocated, clip).unwrap_or(node.allocated)
                });
                Some(AccessibilityNode {
                    id: node.id.clone(),
                    parent: self.semantic_parents[index]
                        .map(|parent| self.resolved.nodes[parent].id.clone()),
                    component: node.component,
                    rect,
                    interactive: node.interaction.interactive,
                    label: node.accessibility_label.clone(),
                    description: node.accessibility_description.clone(),
                    role: node
                        .semantic_role
                        .map(|role| role.as_str().to_owned())
                        .or_else(|| node.accessibility_role.clone()),
                    state: node.accessibility_state.clone(),
                    controls: node.accessibility_controls.clone(),
                    semantic_role: node.semantic_role,
                    actions: node.semantic_actions.clone(),
                    enabled: !node.semantic_actions.is_empty(),
                    focused: node.interaction.focused,
                    controller_selected: node.interaction.controller_selected,
                    navigation_depth: self.navigation_depth_for_index(index),
                    value: node.semantic_value.clone(),
                })
            })
            .collect();
        self.semantic_role_name_index.clear();
        for (index, node) in self.resolved.nodes.iter().enumerate() {
            if let (Some(role), Some(name)) = (node.semantic_role, &node.accessibility_label) {
                self.semantic_role_name_index
                    .entry((role, name.clone()))
                    .or_default()
                    .push(index);
            }
        }
    }

    fn navigation_depth_for_index(&self, index: usize) -> usize {
        self.navigation_depths.get(index).copied().unwrap_or(0)
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

    fn release_build_scratch(&mut self) {
        self.overlay_commands = Vec::new();
        self.overlay_hits = Vec::new();
        self.selection_paints = HashMap::new();
        self.diagnostic_keys = HashSet::new();
        self.seen_ids = HashSet::new();
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
        Kind::CustomPaint { .. } => Size::default(),
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
        Kind::Layer => element
            .children
            .iter()
            .map(|child| {
                let size = measure_element(child, child_constraints);
                let position = child.style.absolute_position.unwrap_or_default();
                Size::new(position.x + size.width, position.y + size.height)
            })
            .fold(Size::default(), |measured, child| {
                Size::new(
                    measured.width.max(child.width),
                    measured.height.max(child.height),
                )
            }),
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
        let measurer = measurer.borrow_mut();
        let mut font_system = measurer.font_system.lock();
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(font_size, line_height));
        buffer.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
        buffer.set_size(wrap.then_some(rect.size.width.max(1.0)), None);
        let mut attrs = Attrs::new().family(Family::SansSerif);
        if bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut font_system, false);
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

fn custom_paint_bounds(command: &PaintCommand) -> Option<Rect> {
    match command {
        PaintCommand::Fill { rect, .. }
        | PaintCommand::TopRoundedFill { rect, .. }
        | PaintCommand::RoundedFill { rect, .. }
        | PaintCommand::Gradient { rect, .. }
        | PaintCommand::Stroke { rect, .. } => Some(*rect),
        PaintCommand::Text { bounds, .. }
        | PaintCommand::StyledText { bounds, .. }
        | PaintCommand::Image { bounds, .. } => Some(*bounds),
        PaintCommand::OverlayFill { .. }
        | PaintCommand::OverlayStroke { .. }
        | PaintCommand::PushClip(_)
        | PaintCommand::PopClip => None,
    }
}

fn rect_contains(outer: Rect, inner: Rect) -> bool {
    inner.origin.x >= outer.origin.x
        && inner.origin.y >= outer.origin.y
        && inner.origin.x + inner.size.width <= outer.origin.x + outer.size.width
        && inner.origin.y + inner.size.height <= outer.origin.y + outer.size.height
}

fn emit_element<Message: Clone>(
    element: &Element<Message>,
    node_index: usize,
    inherited_foreground: Option<Color>,
    tree: &mut UiFrame<Message>,
) {
    let node = tree.resolved.nodes[node_index].clone();
    let rect = node.allocated;
    if node
        .clip
        .is_some_and(|clip| intersection(rect, clip).is_none())
    {
        // Pointer hit regions are clipped, but semantic controller traversal
        // must retain off-screen actions so it can reveal them on selection.
        if let Some(message) = &element.message {
            tree.messages.push(MessageRegion {
                id: node.id.clone(),
                navigation_owner: None,
                rect,
                message: message.clone(),
                message_mapper: element.message_mapper,
            });
        }
        if let Some(message) = &element.context_message {
            tree.context_messages.push(MessageRegion {
                id: node.id.clone(),
                navigation_owner: None,
                rect,
                message: message.clone(),
                message_mapper: None,
            });
        }
        let foreground = element.style.foreground.or(inherited_foreground);
        if matches!(
            element.kind,
            Kind::Flex(_) | Kind::Grid { .. } | Kind::Layer
        ) {
            for (&child_index, child) in node.children.iter().zip(&element.children) {
                emit_element(child, child_index, foreground, tree);
            }
        }
        return;
    }
    let rounded_solid_border = match (element.style.background, element.style.border) {
        (Some(Background::Solid(background)), Some(border))
            if element.style.corner_radius > 0.0 && element.style.border_width > 0.0 =>
        {
            Some((background, border))
        }
        _ => None,
    };
    if let Some((background, border)) = rounded_solid_border {
        let width = element
            .style
            .border_width
            .min(rect.size.width / 2.0)
            .min(rect.size.height / 2.0);
        tree.commands.push(PaintCommand::RoundedFill {
            rect,
            color: border,
            radius: element.style.corner_radius,
        });
        tree.commands.push(PaintCommand::RoundedFill {
            rect: rect.inset(Insets::all(width)),
            color: background,
            radius: (element.style.corner_radius - width).max(0.0),
        });
    } else {
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
    }
    if let Some(message) = &element.message {
        tree.messages.push(MessageRegion {
            id: node.id.clone(),
            navigation_owner: None,
            rect,
            message: message.clone(),
            message_mapper: element.message_mapper,
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
                target_bounds: rect,
                message: Some(message.clone()),
                message_mapper: element.message_mapper,
                drag_mapper: element.drag_mapper,
            });
        }
    }
    if let Some(message) = &element.context_message {
        tree.context_messages.push(MessageRegion {
            id: node.id.clone(),
            navigation_owner: None,
            rect,
            message: message.clone(),
            message_mapper: None,
        });
        if element.message.is_none()
            && !is_scroll_container(element)
            && let Some(hit_rect) = node
                .clip
                .map(|clip| intersection(rect, clip))
                .unwrap_or(Some(rect))
        {
            tree.resolved.nodes[node_index].hit_stack = Some(tree.hits.len());
            tree.hits.push(HitRegion {
                id: node.id.clone(),
                rect: hit_rect,
                target_bounds: rect,
                message: None,
                message_mapper: None,
                drag_mapper: element.drag_mapper,
            });
        }
    }
    if let Some(map) = element.text_mapper
        && let Kind::Text {
            value, input_value, ..
        } = &element.kind
    {
        let (scale, bold, line_height, secure) = match &element.kind {
            Kind::Text {
                scale,
                bold,
                line_height,
                input_mask,
                ..
            } => (
                *scale,
                *bold,
                line_height.unwrap_or_else(|| text_font_size(*scale) * 1.3),
                input_mask.is_some(),
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
            secure,
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
                target_bounds: rect,
                message: None,
                message_mapper: None,
                drag_mapper: None,
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
        Kind::CustomPaint { paint } => {
            tree.commands.push(PaintCommand::PushClip(rect));
            tree.commands
                .extend((paint)(rect).into_iter().filter(|command| {
                    custom_paint_bounds(command).is_some_and(|bounds| rect_contains(rect, bounds))
                }));
            tree.commands.push(PaintCommand::PopClip);
        }
        Kind::Text {
            value,
            scale,
            bold,
            wrap,
            ellipsis,
            outline,
            ..
        } => {
            let text = text_for_bounds(value, *scale, *bold, *ellipsis, rect.size.width);
            if let Some((color, width)) = outline {
                for (x, y) in [
                    (-1.0, -1.0),
                    (0.0, -1.0),
                    (1.0, -1.0),
                    (-1.0, 0.0),
                    (1.0, 0.0),
                    (-1.0, 1.0),
                    (0.0, 1.0),
                    (1.0, 1.0),
                ] {
                    tree.commands.push(PaintCommand::Text {
                        bounds: Rect::new(
                            rect.origin.x + x * *width,
                            rect.origin.y + y * *width,
                            rect.size.width,
                            rect.size.height,
                        ),
                        text: text.clone(),
                        scale: *scale,
                        color: *color,
                        align: element.style.text_align,
                        bold: *bold,
                        wrap: *wrap,
                    });
                }
            }
            tree.commands.push(PaintCommand::Text {
                bounds: rect,
                text,
                scale: *scale,
                color: foreground.unwrap_or(0x00ff_ffff),
                align: element.style.text_align,
                bold: *bold,
                wrap: *wrap,
            });
        }
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
                            navigation_owner: Some(node.id.clone()),
                            rect: glyph.rect,
                            message: message.clone(),
                            message_mapper: None,
                        });
                        tree.hits.push(HitRegion {
                            id: link_id.clone(),
                            rect: glyph.rect,
                            target_bounds: glyph.rect,
                            message: Some(message.clone()),
                            message_mapper: None,
                            drag_mapper: None,
                        });
                    }
                }
            }
        }
        Kind::Image {
            id,
            generation,
            image,
            high_density,
            presentation,
        } => {
            let source = Size::new(image.width() as f32, image.height() as f32);
            let bounds = presentation.bounds(rect, source);
            if presentation.fit == ImageFit::Tile
                && bounds.size.width > 0.0
                && bounds.size.height > 0.0
            {
                let mut y = bounds.origin.y;
                while y > rect.origin.y {
                    y -= bounds.size.height;
                }
                let mut count = 0usize;
                while y < rect.origin.y + rect.size.height && count < 4096 {
                    let mut x = bounds.origin.x;
                    while x > rect.origin.x {
                        x -= bounds.size.width;
                    }
                    while x < rect.origin.x + rect.size.width && count < 4096 {
                        tree.commands.push(PaintCommand::Image {
                            bounds: Rect::new(x, y, bounds.size.width, bounds.size.height),
                            id: *id,
                            generation: *generation,
                            image: image.clone(),
                            high_density: high_density.clone(),
                        });
                        x += bounds.size.width;
                        count += 1;
                    }
                    y += bounds.size.height;
                }
            } else {
                tree.commands.push(PaintCommand::Image {
                    bounds,
                    id: *id,
                    generation: *generation,
                    image: image.clone(),
                    high_density: high_density.clone(),
                });
            }
        }
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
            tree.commands.push(PaintCommand::RoundedFill {
                rect: track_rect,
                color: *track,
                radius: 3.0,
            });
            tree.commands.push(PaintCommand::RoundedFill {
                rect: Rect::new(
                    track_rect.origin.x,
                    track_rect.origin.y,
                    fill_width,
                    track_rect.size.height,
                ),
                color: *fill,
                radius: 3.0,
            });
            let thumb_rect = Rect::new(
                (track_rect.origin.x + fill_width - 10.0)
                    .clamp(rect.origin.x, rect.origin.x + rect.size.width - 20.0),
                rect.origin.y + rect.size.height / 2.0 - 10.0,
                20.0,
                20.0,
            );
            tree.commands.push(PaintCommand::RoundedFill {
                rect: thumb_rect,
                color: *thumb,
                radius: 10.0,
            });
            tree.commands.push(PaintCommand::Stroke {
                rect: thumb_rect,
                color: *fill,
                width: 2.0,
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
            ..
        } => {
            let header_height = if *overlay { 30.0 } else { 42.0 };
            let option_height = if *overlay { 34.0 } else { 36.0 };
            let options_height = option_height * options.len() as f32;
            let options_origin_y = if *overlay
                && rect.origin.y + header_height + options_height
                    > tree.viewport.origin.y + tree.viewport.size.height
                && rect.origin.y - options_height >= tree.viewport.origin.y
            {
                rect.origin.y - options_height
            } else {
                rect.origin.y + header_height
            };
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
                wrap: false,
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
                wrap: false,
            });
            // Keep the typed option topology available while collapsed so the
            // opening transition can select its declared entry target in the
            // same event batch. Hidden options remain absent from hit testing,
            // painting, and accessibility until expanded.
            if !*expanded {
                for (index, message) in element.option_messages.iter().enumerate() {
                    if let Some(message) = message {
                        tree.messages.push(MessageRegion {
                            id: node.id.scoped(format!("option-{index}")),
                            navigation_owner: Some(node.id.clone()),
                            rect: Rect::new(
                                rect.origin.x,
                                options_origin_y + index as f32 * option_height,
                                rect.size.width,
                                option_height,
                            ),
                            message: message.clone(),
                            message_mapper: None,
                        });
                    }
                }
            }
            if *expanded {
                for (index, option) in options.iter().enumerate() {
                    let option_rect = Rect::new(
                        rect.origin.x,
                        options_origin_y + index as f32 * option_height,
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
                        wrap: false,
                    });
                    let option_id = node.id.scoped(format!("option-{index}"));
                    let message = element.option_messages.get(index).cloned().flatten();
                    let option_node = tree.resolved.nodes.len();
                    tree.resolved.nodes.push(ResolvedNode {
                        component: "MenuItem",
                        id: option_id.clone(),
                        source: node.source,
                        allocated: option_rect,
                        padding_box: option_rect,
                        border_box: option_rect,
                        content: option_rect.inset(Insets::all(8.0)),
                        constraints: Constraints::tight(option_rect.size),
                        preferred: option_rect.size,
                        flex_basis: Length::Auto,
                        flex_grow: 0.0,
                        flex_shrink: 0.0,
                        clip: node.clip,
                        scroll: None,
                        grid_tracks: Vec::new(),
                        hit_stack: None,
                        interaction: InteractionState::default(),
                        navigation_scope: None,
                        adjustment_step: 0.05,
                        controller_value: None,
                        accessibility_label: Some(option.clone()),
                        accessibility_description: None,
                        accessibility_role: Some(SemanticRole::MenuItem.as_str().into()),
                        accessibility_state: None,
                        accessibility_controls: None,
                        accessibility_hidden: false,
                        semantic_role: Some(SemanticRole::MenuItem),
                        semantic_actions: message
                            .is_some()
                            .then_some(ActionKind::Activate)
                            .into_iter()
                            .collect(),
                        semantic_value: None,
                        children: Vec::new(),
                    });
                    tree.resolved.nodes[node_index].children.push(option_node);
                    if let Some(message) = &message {
                        tree.messages.push(MessageRegion {
                            id: option_id.clone(),
                            navigation_owner: Some(node.id.clone()),
                            rect: option_rect,
                            message: message.clone(),
                            message_mapper: None,
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
                            target_bounds: option_rect,
                            message,
                            message_mapper: None,
                            drag_mapper: None,
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
        Kind::Flex(_) | Kind::Grid { .. } | Kind::Layer => {
            for (&child_index, child) in node.children.iter().zip(&element.children) {
                emit_element(child, child_index, foreground, tree);
            }
        }
    }
    // Sliders and dropdowns paint their native surface after the generic
    // style layer. Repeat an active/focused border in the foreground so their
    // own track or header cannot cover the selection indicator.
    if matches!(element.kind, Kind::Slider { .. } | Kind::Dropdown { .. })
        && let Some(color) = element.style.border
        && element.style.border_width > 0.0
    {
        tree.commands.push(PaintCommand::Stroke {
            rect,
            color,
            width: element.style.border_width,
        });
    }
    if clips_descendants {
        tree.commands.push(PaintCommand::PopClip);
    }
}

fn derive_accessible_name<Message>(element: &Element<Message>) -> Option<String> {
    if let Some(label) = element
        .style
        .accessibility_label
        .as_ref()
        .filter(|label| !label.is_empty())
    {
        return Some(label.clone());
    }
    if let Kind::Text { value, .. } = &element.kind
        && !value.is_empty()
    {
        return Some(value.clone());
    }
    if let Kind::Dropdown { selected, .. } = &element.kind
        && !selected.is_empty()
    {
        return Some(selected.clone());
    }
    element
        .style
        .semantic_role
        .and_then(|_| {
            element
                .children
                .iter()
                .find_map(derive_descendant_accessible_name)
        })
        .or_else(|| match element.style.semantic_role {
            // Value/editing controls have an intrinsic control-type name even
            // when an author supplies no surrounding label.
            // Product components should still provide the more specific name.
            Some(SemanticRole::Slider) => Some("Value".into()),
            Some(SemanticRole::TextField) => Some("Text input".into()),
            _ => None,
        })
}

fn derive_descendant_accessible_name<Message>(element: &Element<Message>) -> Option<String> {
    element
        .style
        .accessibility_label
        .as_ref()
        .filter(|label| !label.is_empty())
        .cloned()
        .or_else(|| match &element.kind {
            Kind::Text { value, .. } if !value.is_empty() => Some(value.clone()),
            Kind::Dropdown { selected, .. } if !selected.is_empty() => Some(selected.clone()),
            _ => None,
        })
        .or_else(|| {
            element
                .children
                .iter()
                .find_map(derive_descendant_accessible_name)
        })
}

fn layout_element<Message: Clone>(
    element: &Element<Message>,
    id: &UiId,
    bounds: Rect,
    inherited_foreground: Option<Color>,
    inherited_clip: Option<Rect>,
    tree: &mut UiFrame<Message>,
) -> usize {
    let rect = bounds;
    let preferred = measure_element(element, Constraints::loose(bounds.size));
    let node_index = tree.resolved.nodes.len();
    let interaction = InteractionState {
        interactive: (element.message.is_some() || element.context_message.is_some())
            && !is_scroll_container(element)
            || element.text_mapper.is_some(),
        ..InteractionState::default()
    };
    let derived_accessible_name = derive_accessible_name(element);
    let missing_accessible_name = element.style.semantic_role.is_some()
        && derived_accessible_name.as_deref().is_none_or(str::is_empty)
        && !element.style.semantic_decorative;
    let semantic_role = (!missing_accessible_name && !element.style.semantic_decorative)
        .then_some(element.style.semantic_role)
        .flatten();
    let semantic_hidden = element.style.accessibility_hidden
        || element.style.semantic_decorative
        || missing_accessible_name;
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
        navigation_scope: element.navigation_scope.clone(),
        adjustment_step: element.adjustment_step,
        controller_value: match &element.kind {
            Kind::Slider { value, .. } => Some(*value),
            _ => None,
        },
        accessibility_label: derived_accessible_name,
        accessibility_description: element.style.accessibility_description.clone(),
        accessibility_role: element.style.accessibility_role.clone(),
        accessibility_state: element.style.accessibility_state.clone(),
        accessibility_controls: element.style.accessibility_controls.clone(),
        accessibility_hidden: semantic_hidden,
        semantic_role,
        semantic_actions: {
            let mut actions = Vec::new();
            if missing_accessible_name {
                actions.clear();
                actions
            } else {
                if let Kind::Slider { value, .. } = &element.kind
                    && element.message_mapper.is_some()
                {
                    if *value < 1.0 {
                        actions.push(ActionKind::Increment);
                    }
                    if *value > 0.0 {
                        actions.push(ActionKind::Decrement);
                    }
                    actions.push(ActionKind::SetValue);
                } else if element.text_mapper.is_some() {
                    actions.push(ActionKind::SetValue);
                } else if element.message.is_some() && !is_scroll_container(element) {
                    actions.push(ActionKind::Activate);
                }
                if element.context_message.is_some() {
                    actions.push(ActionKind::ContextMenu);
                }
                actions
            }
        },
        semantic_value: match &element.kind {
            Kind::Slider { value, .. } => Some(SemanticValueSnapshot::Number {
                value: f64::from(*value),
                minimum: 0.0,
                maximum: 1.0,
                step: f64::from(element.adjustment_step),
            }),
            Kind::Text {
                input_value: Some(value),
                input_mask: Some(_),
                ..
            } => Some(SemanticValueSnapshot::ProtectedText {
                character_count: value.chars().count(),
            }),
            Kind::Text {
                input_value: Some(value),
                ..
            } => Some(SemanticValueSnapshot::Text(value.clone())),
            Kind::Dropdown { selected, .. } => Some(SemanticValueSnapshot::Text(selected.clone())),
            _ => None,
        },
        children: Vec::new(),
    });
    if missing_accessible_name {
        tree.diagnostic(
            DiagnosticKind::MissingAccessibleName,
            id,
            "semantic role requires an accessible name or an explicit decorative exemption",
        );
    }
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
            wrap,
            line_height,
            max_lines,
            ..
        } => Some(measure_text(
            value,
            *scale,
            *bold,
            *wrap,
            *line_height,
            *max_lines,
            if *wrap {
                rect.size.width.max(0.0)
            } else {
                f32::INFINITY
            },
        )),
        // Images deliberately map intrinsic pixels through their presentation
        // policy, so a different allocated size is not unsatisfied content.
        Kind::Image { .. } => None,
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
        | Kind::CustomPaint { .. }
        | Kind::Image { .. }
        | Kind::Slider { .. }
        | Kind::Dropdown { .. } => {}
        Kind::Layer => {
            let content = rect.inset(element.style.padding);
            for (index, child) in element.children.iter().enumerate() {
                let position = child.style.absolute_position.unwrap_or_default();
                let size = measure_element(child, Constraints::loose(content.size));
                let bounds = Rect::new(
                    content.origin.x + position.x,
                    content.origin.y + position.y,
                    size.width.min((content.size.width - position.x).max(0.0)),
                    size.height.min((content.size.height - position.y).max(0.0)),
                );
                child_indices.push(layout_element(
                    child,
                    &resolved_child_id(id, child, index),
                    bounds,
                    foreground,
                    descendant_clip,
                    tree,
                ));
            }
        }
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
                configure_scroll_semantics(&mut tree.resolved.nodes[node_index], extent);
                tree.scrolls.push(ScrollRegion {
                    id: id.clone(),
                    message: element.message.clone(),
                    offset_mapper: element.message_mapper,
                    rect: scroll_rect,
                    clip: descendant_clip.unwrap_or(scroll_rect),
                    extent,
                    scrollbar: element.style.scrollbar_palette,
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
                configure_scroll_semantics(&mut tree.resolved.nodes[node_index], extent);
                tree.scrolls.push(ScrollRegion {
                    id: id.clone(),
                    message: element.message.clone(),
                    offset_mapper: element.message_mapper,
                    rect: scroll_rect,
                    clip: descendant_clip.unwrap_or(scroll_rect),
                    extent,
                    scrollbar: element.style.scrollbar_palette,
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
                configure_scroll_semantics(&mut tree.resolved.nodes[node_index], extent);
                if let Some(message) = &element.message {
                    tree.scrolls.push(ScrollRegion {
                        id: id.clone(),
                        message: Some(message.clone()),
                        offset_mapper: element.message_mapper,
                        rect: viewport,
                        clip,
                        extent,
                        scrollbar: element.style.scrollbar_palette,
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
                    scrollbar: element.style.scrollbar_palette,
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

fn mask_text(value: &str, mask: char) -> String {
    value
        .chars()
        .map(|character| if character == '\n' { '\n' } else { mask })
        .collect()
}

fn apply_transient_state<Message>(
    element: &mut Element<Message>,
    id: &UiId,
    state: &mut UiStateStore,
) {
    let focus_foreground = element.style.foreground.or_else(|| {
        element
            .children
            .iter()
            .find_map(|child| focus_participating_foreground(child))
    });
    let owns_state = element.message.is_some()
        || element.navigation_scope.is_some()
        || element.text_mapper.is_some()
        || matches!(element.style.overflow_x, Overflow::Scroll | Overflow::Auto)
        || matches!(element.style.overflow_y, Overflow::Scroll | Overflow::Auto)
        || matches!(
            element.kind,
            Kind::VerticalScroll { .. } | Kind::Dropdown { .. }
        );
    if owns_state {
        let scope_background_active = state.window_focused()
            && state.input_modality() == InputModality::Controller
            && element.navigation_scope.is_some()
            && (state.navigation().controller_selected() == Some(id)
                || state.navigation().controller_scope() == Some(id)
                || (element
                    .navigation_scope
                    .as_ref()
                    .is_some_and(|scope| scope.pane)
                    && state.navigation().controller_pane() == Some(id)))
            && element.style.controller_scope_background.is_some();
        if scope_background_active
            && let Some(background) = element.style.controller_scope_background
        {
            element.style.background = Some(background);
        }
        if element
            .navigation_scope
            .as_ref()
            .is_some_and(|scope| scope.pane)
            && state.navigation().controller_pane().is_none()
            && element
                .navigation_scope
                .as_ref()
                .is_some_and(|scope| scope.default_pane)
        {
            state.navigation_mut().set_controller_pane(Some(id.clone()));
        }
        if state.pressed() == Some(id) {
            if let Some(background) = element.style.pressed_background {
                element.style.background = Some(background);
            }
        } else if state.hovered() == Some(id)
            && let Some(background) = element.style.hover_background
        {
            element.style.background = Some(background);
        }
        let active_focus_tint = if scope_background_active {
            None
        } else {
            if state.window_focused() && state.navigation().controller_selected() == Some(id) {
                element
                    .style
                    .controller_focus_background_tint
                    .or(element.style.focus_background_tint)
                    .or(Some(crate::theme::FALLBACK_CONTROLLER_FOCUS_CUE))
            } else if state.window_focused()
                && state.focused() == Some(id)
                && matches!(
                    state.input_modality(),
                    InputModality::Keyboard | InputModality::Accessibility
                )
            {
                element
                    .style
                    .focus_background_tint
                    .or(Some(crate::theme::FALLBACK_KEYBOARD_FOCUS_CUE))
            } else {
                None
            }
        };
        if let Some(tint) = active_focus_tint {
            let transform = |color| {
                focus_foreground.map_or_else(
                    || crate::focused_surface(color, tint),
                    |foreground| crate::focused_surface_with_foreground(color, tint, foreground),
                )
            };
            element.style.background = Some(match element.style.background {
                Some(Background::Solid(color)) => Background::Solid(transform(color)),
                Some(Background::LinearGradient(mut gradient)) => {
                    gradient.start = transform(gradient.start);
                    gradient.end = transform(gradient.end);
                    Background::LinearGradient(gradient)
                }
                None => Background::Solid(transform(crate::theme::FALLBACK_FOCUS_SURFACE)),
            });
        }
        let requested_open_generation = match &element.kind {
            Kind::Dropdown {
                open_generation, ..
            } => Some(*open_generation),
            _ => None,
        };
        let (scroll_offset_x, scroll_offset, scroll_at_end, dropdown_open) = {
            let transient = state.touch(id.clone());
            if requested_open_generation.is_some_and(|generation| {
                if generation == transient.dropdown_open_generation {
                    false
                } else {
                    transient.dropdown_open_generation = generation;
                    true
                }
            }) {
                transient.dropdown_open = true;
            }
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
                ..
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
                input_mask,
                line_height,
                ..
            } = &mut element.kind
        {
            // A text field may paint a placeholder while its editable value is
            // intentionally empty. Reconciliation must seed the editor from
            // that canonical input value, not from presentation text.
            let initial = input_value.clone().unwrap_or_else(|| value.clone());
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
                        let visible = input_mask.map_or_else(
                            || editor.text()[..end].to_owned(),
                            |mask| mask_text(&editor.text()[..end], mask),
                        );
                        measure_text(&visible, *scale, *bold, false, None, Some(1), f32::INFINITY)
                            .width
                    };
                    (width(selection.start), width(selection.end))
                });
            *caret_position = (focused && caret_visible).then(|| {
                let prefix = input_mask.map_or_else(
                    || editor.display_caret_prefix(),
                    |mask| mask_text(&editor.display_caret_prefix(), mask),
                );
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
            *value = if let Some(mask) = input_mask {
                mask_text(&editor.display_text_with_caret(""), *mask)
            } else {
                editor.display_text_with_caret("")
            };
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

fn focus_participating_foreground<Message>(element: &Element<Message>) -> Option<Color> {
    element.style.foreground.or_else(|| {
        element
            .children
            .iter()
            .find_map(focus_participating_foreground)
    })
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
