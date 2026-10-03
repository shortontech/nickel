use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use crate::{Rect, Size, UiId};

use super::{Element, Kind, Length, PaintCommand, Style};

/// Stable native identity assigned independently of a node's current arena slot.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct RetainedNodeId(u64);

/// Independent work classes dirtied by declarative reconciliation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DirtyPhases(u8);

impl DirtyPhases {
    pub(crate) const CHILDREN: Self = Self(1 << 0);
    pub(crate) const MEASURE: Self = Self(1 << 1);
    pub(crate) const PLACE: Self = Self(1 << 2);
    pub(crate) const PAINT: Self = Self(1 << 3);
    pub(crate) const INTERACTION: Self = Self(1 << 4);
    pub(crate) const SEMANTICS: Self = Self(1 << 5);
    pub(crate) const ALL: Self = Self((1 << 6) - 1);

    pub(crate) const fn contains(self, phase: Self) -> bool {
        self.0 & phase.0 == phase.0
    }

    const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    fn insert(&mut self, other: Self) {
        *self = self.union(other);
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RetainedPhaseData {
    pub(crate) measured: Option<Size>,
    measured_constraints: Option<[u32; 2]>,
    pub(crate) allocated: Option<Rect>,
    pub(crate) paint: Vec<PaintCommand>,
    pub(crate) interaction_revision: u64,
    pub(crate) semantics_revision: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ReconcileStats {
    pub(crate) visited: usize,
    pub(crate) reused: usize,
    pub(crate) created: usize,
    pub(crate) removed: usize,
    pub(crate) moved: usize,
    pub(crate) replaced: usize,
    pub(crate) clean: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct RetainedNode {
    id: RetainedNodeId,
    parent: Option<RetainedNodeId>,
    identity: ChildIdentity,
    ui_id: UiId,
    kind: KindTag,
    signatures: PhaseSignatures,
    dirty: DirtyPhases,
    children: Vec<RetainedNodeId>,
    pub(crate) phases: RetainedPhaseData,
}

impl RetainedNode {
    #[cfg(test)]
    pub(crate) const fn id(&self) -> RetainedNodeId {
        self.id
    }

    #[cfg(test)]
    pub(crate) const fn dirty(&self) -> DirtyPhases {
        self.dirty
    }

    pub(crate) const fn parent(&self) -> Option<RetainedNodeId> {
        self.parent
    }

    fn invalidate_dirty_phases(&mut self) {
        if self.dirty.contains(DirtyPhases::MEASURE) {
            self.phases.measured = None;
        }
        if self.dirty.contains(DirtyPhases::PLACE) {
            self.phases.allocated = None;
        }
        if self.dirty.contains(DirtyPhases::PAINT) {
            self.phases.paint.clear();
        }
        if self.dirty.contains(DirtyPhases::INTERACTION) {
            self.phases.interaction_revision = 0;
        }
        if self.dirty.contains(DirtyPhases::SEMANTICS) {
            self.phases.semantics_revision = 0;
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RetainedNodeArena {
    next_id: u64,
    root: Option<RetainedNodeId>,
    nodes: HashMap<RetainedNodeId, RetainedNode>,
    last: ReconcileStats,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum ChildIdentity {
    Root,
    Key(UiId),
    Position(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KindTag {
    Row,
    Column,
    Layer,
    Scroll,
    Grid,
    CustomPaint,
    Text,
    StyledText,
    Image,
    Slider,
    Dropdown,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PhaseSignatures {
    measure: u64,
    place: u64,
    paint: u64,
    interaction: u64,
    semantics: u64,
    content_revision: Option<u64>,
    measure_content_sensitive: bool,
    paint_content_sensitive: bool,
    interaction_content_sensitive: bool,
    semantics_content_sensitive: bool,
}

#[derive(Default)]
struct CompactHasher(std::collections::hash_map::DefaultHasher);

impl CompactHasher {
    fn add<T: Hash>(&mut self, value: &T) {
        value.hash(&mut self.0);
    }

    fn finish(self) -> u64 {
        self.0.finish()
    }
}

fn bits(value: f32) -> u32 {
    value.to_bits()
}

fn length(hasher: &mut CompactHasher, value: Length) {
    std::mem::discriminant(&value).hash(&mut hasher.0);
    match value {
        Length::Px(value) | Length::Percent(value) | Length::Fraction(value) => {
            hasher.add(&bits(value));
        }
        Length::Auto | Length::Fill | Length::MinContent | Length::MaxContent => {}
    }
}

fn hash_layout_style(style: &Style) -> (u64, u64) {
    let mut measure = CompactHasher::default();
    for value in [
        style.padding.top,
        style.padding.right,
        style.padding.bottom,
        style.padding.left,
    ] {
        measure.add(&bits(value));
    }
    measure.add(&bits(style.gap));
    length(&mut measure, style.width);
    length(&mut measure, style.height);
    length(&mut measure, style.basis);
    for value in [
        style.min_width,
        style.min_height,
        style.max_width,
        style.max_height,
    ] {
        measure.add(&bits(value));
    }
    measure.add(&bits(style.grow));
    measure.add(&bits(style.shrink));
    measure.add(&format!(
        "{:?}{:?}{:?}",
        style.align_self, style.align_items, style.justify_content
    ));

    let mut place = CompactHasher::default();
    place.add(&format!("{:?}{:?}", style.overflow_x, style.overflow_y));
    if let Some(position) = style.absolute_position {
        place.add(&bits(position.x));
        place.add(&bits(position.y));
    }
    place.add(&style.follow_scroll_end);
    (measure.finish(), place.finish())
}

fn kind_tag(kind: &Kind) -> KindTag {
    match kind {
        Kind::Flex(super::Axis::Horizontal) => KindTag::Row,
        Kind::Flex(super::Axis::Vertical) => KindTag::Column,
        Kind::Layer => KindTag::Layer,
        Kind::VerticalScroll { .. } => KindTag::Scroll,
        Kind::Grid { .. } => KindTag::Grid,
        Kind::CustomPaint { .. } | Kind::CustomPaintCommands { .. } => KindTag::CustomPaint,
        Kind::Text { .. } => KindTag::Text,
        Kind::StyledText { .. } => KindTag::StyledText,
        Kind::Image { .. } => KindTag::Image,
        Kind::Slider { .. } => KindTag::Slider,
        Kind::Dropdown { .. } => KindTag::Dropdown,
    }
}

fn signatures<Message>(element: &Element<Message>) -> PhaseSignatures {
    let (style_measure, place) = hash_layout_style(&element.style);
    let mut content_measure = CompactHasher::default();
    let mut paint = CompactHasher::default();
    let mut semantics = CompactHasher::default();
    let mut measure_content_sensitive = false;
    let mut paint_content_sensitive = false;
    let interaction_content_sensitive = element.message.is_some()
        || element.context_message.is_some()
        || element.focus_message.is_some()
        || element.blur_message.is_some()
        || element.drag_seed.is_some()
        || element.drop_message.is_some()
        || element.text_mapper.is_some()
        || !element.option_messages.is_empty()
        || !element.inline_messages.is_empty();
    let mut semantics_content_sensitive = element.style.accessibility_label.is_some()
        || element.style.accessibility_description.is_some()
        || element.style.accessibility_role.is_some()
        || element.style.accessibility_state.is_some()
        || element.style.accessibility_controls.is_some();
    std::mem::discriminant(&element.kind).hash(&mut content_measure.0);
    std::mem::discriminant(&element.kind).hash(&mut paint.0);

    match &element.kind {
        Kind::Text {
            value,
            scale,
            bold,
            wrap,
            line_height,
            max_lines,
            ellipsis,
            outline,
            input_value,
            input_placeholder,
            input_mask,
            ..
        } => {
            let _ = value;
            measure_content_sensitive = true;
            paint_content_sensitive = true;
            semantics_content_sensitive = true;
            content_measure.add(&bits(*scale));
            content_measure.add(bold);
            content_measure.add(wrap);
            content_measure.add(&line_height.map(bits));
            content_measure.add(max_lines);
            paint.add(ellipsis);
            paint.add(&format!("{outline:?}"));
            paint.add(&input_value.is_some());
            paint.add(&input_placeholder.is_some());
            paint.add(&input_mask);
        }
        Kind::StyledText {
            value,
            spans,
            scale,
            wrap,
            line_height,
        } => {
            let _ = (value, spans);
            measure_content_sensitive = true;
            paint_content_sensitive = true;
            semantics_content_sensitive = true;
            content_measure.add(&bits(*scale));
            content_measure.add(wrap);
            content_measure.add(&line_height.map(bits));
        }
        Kind::Image {
            id,
            generation,
            presentation,
            ..
        } => {
            content_measure.add(id);
            content_measure.add(generation);
            content_measure.add(&format!("{presentation:?}"));
            paint.add(id);
            paint.add(generation);
            paint.add(&format!("{presentation:?}"));
        }
        Kind::VerticalScroll { offset, controlled } => {
            paint.add(&bits(*offset));
            paint.add(controlled);
        }
        Kind::Grid { columns } => content_measure.add(&format!("{columns:?}")),
        Kind::CustomPaint { paint: callback } => {
            paint.add(&(*callback as usize));
            paint_content_sensitive = true;
        }
        Kind::CustomPaintCommands { commands } => {
            let _ = commands;
            paint_content_sensitive = true;
        }
        Kind::Slider {
            value,
            track,
            fill,
            thumb,
            thumb_border,
            geometry,
            presentation,
        } => {
            paint.add(&bits(*value));
            paint.add(&format!(
                "{track:?}{fill:?}{thumb:?}{thumb_border:?}{geometry:?}{presentation:?}"
            ));
        }
        Kind::Dropdown {
            selected,
            options,
            expanded,
            open_generation,
            overlay,
            background,
            option_background,
            foreground,
            presentation,
            option_presentations,
            resolved_options,
        } => {
            let _ = (selected, options, option_presentations, resolved_options);
            measure_content_sensitive = true;
            paint_content_sensitive = true;
            semantics_content_sensitive = true;
            paint.add(expanded);
            paint.add(open_generation);
            paint.add(overlay);
            paint.add(&format!(
                "{background:?}{option_background:?}{foreground:?}{presentation:?}"
            ));
        }
        Kind::Flex(_) | Kind::Layer => {}
    }

    let fixed_leaf = element.children.is_empty()
        && matches!(element.style.width, Length::Px(_))
        && matches!(element.style.height, Length::Px(_));
    let measure = if fixed_leaf {
        measure_content_sensitive = false;
        style_measure
    } else {
        style_measure ^ content_measure.finish()
    };
    paint.add(&format!(
        "{:?}{:?}{:?}{:?}{:?}{:?}{:?}",
        element.style.background,
        element.style.border,
        element.style.foreground,
        element.style.box_shadow,
        element.style.corner_radius,
        element.style.top_corner_radius,
        element.style.interaction_paints
    ));

    let mut interaction = CompactHasher::default();
    interaction.add(&element.message.is_some());
    interaction.add(&element.context_message.is_some());
    interaction.add(&element.focus_message.is_some());
    interaction.add(&element.blur_message.is_some());
    interaction.add(&element.message_mapper.map(|callback| callback as usize));
    interaction.add(
        &element
            .seeded_value_mapper
            .map(|callback| callback as usize),
    );
    interaction.add(
        &element
            .scroll_extent_mapper
            .map(|callback| callback as usize),
    );
    interaction.add(&element.drag_mapper.map(|callback| callback as usize));
    interaction.add(&element.drop_mapper.map(|callback| callback as usize));
    interaction.add(&element.text_mapper.is_some());
    interaction.add(&element.option_messages.len());
    interaction.add(&element.inline_messages.len());

    semantics.add(&element.style.accessibility_label.is_some());
    semantics.add(&element.style.accessibility_description.is_some());
    semantics.add(&element.style.accessibility_role.is_some());
    semantics.add(&element.style.accessibility_state.is_some());
    semantics.add(&element.style.accessibility_controls.is_some());
    semantics.add(&format!("{:?}", element.style.semantic_role));
    semantics.add(&element.style.accessibility_hidden);
    semantics.add(&element.style.semantic_decorative);

    PhaseSignatures {
        measure,
        place,
        paint: paint.finish(),
        interaction: interaction.finish(),
        semantics: semantics.finish(),
        content_revision: element.content_revision,
        measure_content_sensitive,
        paint_content_sensitive,
        interaction_content_sensitive,
        semantics_content_sensitive,
    }
}

fn phase_matches(
    current_metadata: u64,
    previous_metadata: u64,
    current_sensitive: bool,
    previous_sensitive: bool,
    current_revision: Option<u64>,
    previous_revision: Option<u64>,
) -> bool {
    current_metadata == previous_metadata
        && current_sensitive == previous_sensitive
        && (!current_sensitive
            || current_revision
                .zip(previous_revision)
                .is_some_and(|(current, previous)| current == previous))
}

impl RetainedNodeArena {
    pub(crate) fn phase_is_clean(&self, id: &UiId, phase: DirtyPhases) -> bool {
        self.nodes
            .values()
            .find(|node| &node.ui_id == id)
            .is_some_and(|node| !node.dirty.contains(phase))
    }

    pub(crate) fn subtree_phase_is_clean(&self, id: &UiId, phase: DirtyPhases) -> bool {
        let Some(node) = self.nodes.values().find(|node| &node.ui_id == id) else {
            return false;
        };
        self.subtree_phase_is_clean_from(node.id, phase)
    }

    fn subtree_phase_is_clean_from(&self, id: RetainedNodeId, phase: DirtyPhases) -> bool {
        let node = &self.nodes[&id];
        !node.dirty.contains(phase)
            && node
                .children
                .iter()
                .all(|child| self.subtree_phase_is_clean_from(*child, phase))
    }

    pub(crate) fn reconcile<Message>(&mut self, root: &Element<Message>) -> ReconcileStats {
        let old_nodes = std::mem::take(&mut self.nodes);
        let old_root = self.root.take();
        self.last = ReconcileStats::default();
        let mut consumed = HashSet::new();
        let root_id = self.reconcile_node(
            root,
            None,
            ChildIdentity::Root,
            root.id.as_ref().map_or_else(
                || UiId::from("root"),
                |id| UiId::from("root").scoped(id.as_str()),
            ),
            0,
            old_root,
            &old_nodes,
            &mut consumed,
        );
        self.root = Some(root_id);
        self.last.removed = old_nodes.len().saturating_sub(consumed.len());
        self.propagate_descendant_work(root_id);
        for node in self.nodes.values_mut() {
            node.invalidate_dirty_phases();
        }
        self.last.clean = self
            .nodes
            .values()
            .filter(|node| node.dirty.is_empty())
            .count();
        self.last.clone()
    }

    #[allow(clippy::too_many_arguments)]
    fn reconcile_node<Message>(
        &mut self,
        element: &Element<Message>,
        parent: Option<RetainedNodeId>,
        identity: ChildIdentity,
        ui_id: UiId,
        position: usize,
        candidate: Option<RetainedNodeId>,
        old_nodes: &HashMap<RetainedNodeId, RetainedNode>,
        consumed: &mut HashSet<RetainedNodeId>,
    ) -> RetainedNodeId {
        self.last.visited += 1;
        let kind = kind_tag(&element.kind);
        let old = candidate.and_then(|id| old_nodes.get(&id));
        let reusable = old.filter(|node| node.kind == kind && node.identity == identity);
        let (id, phases, mut dirty) = if let Some(old) = reusable {
            consumed.insert(old.id);
            self.last.reused += 1;
            let current = signatures(element);
            let mut dirty = DirtyPhases::default();
            if !phase_matches(
                current.measure,
                old.signatures.measure,
                current.measure_content_sensitive,
                old.signatures.measure_content_sensitive,
                current.content_revision,
                old.signatures.content_revision,
            ) {
                dirty.insert(DirtyPhases::MEASURE.union(DirtyPhases::PLACE));
            }
            if current.place != old.signatures.place {
                dirty.insert(DirtyPhases::PLACE);
            }
            if !phase_matches(
                current.paint,
                old.signatures.paint,
                current.paint_content_sensitive,
                old.signatures.paint_content_sensitive,
                current.content_revision,
                old.signatures.content_revision,
            ) {
                dirty.insert(DirtyPhases::PAINT);
            }
            if !phase_matches(
                current.interaction,
                old.signatures.interaction,
                current.interaction_content_sensitive,
                old.signatures.interaction_content_sensitive,
                current.content_revision,
                old.signatures.content_revision,
            ) {
                dirty.insert(DirtyPhases::INTERACTION);
            }
            if !phase_matches(
                current.semantics,
                old.signatures.semantics,
                current.semantics_content_sensitive,
                old.signatures.semantics_content_sensitive,
                current.content_revision,
                old.signatures.content_revision,
            ) {
                dirty.insert(DirtyPhases::SEMANTICS);
            }
            (old.id, old.phases.clone(), dirty)
        } else {
            if old.is_some() {
                self.last.replaced += 1;
            }
            self.next_id = self.next_id.max(1);
            let id = RetainedNodeId(self.next_id);
            self.next_id += 1;
            self.last.created += 1;
            (id, RetainedPhaseData::default(), DirtyPhases::ALL)
        };

        let old_children = reusable.map_or(&[][..], |node| node.children.as_slice());
        let mut keyed = HashMap::new();
        for child_id in old_children {
            let child = &old_nodes[child_id];
            keyed.insert(child.identity.clone(), *child_id);
        }
        let mut children = Vec::with_capacity(element.children.len());
        for (index, child) in element.children.iter().enumerate() {
            let child_identity = child
                .id
                .clone()
                .map_or(ChildIdentity::Position(index), ChildIdentity::Key);
            let child_ui_id = child.id.as_ref().map_or_else(
                || ui_id.scoped(format!("#{index}")),
                |id| ui_id.scoped(id.as_str()),
            );
            let old_child = keyed
                .get(&child_identity)
                .copied()
                .filter(|candidate| !consumed.contains(candidate));
            if let Some(old_child) = old_child
                && old_children.get(index).copied() != Some(old_child)
            {
                self.last.moved += 1;
            }
            children.push(self.reconcile_node(
                child,
                Some(id),
                child_identity,
                child_ui_id,
                index,
                old_child,
                old_nodes,
                consumed,
            ));
        }
        if reusable.is_some_and(|old| old.children != children) {
            dirty.insert(DirtyPhases::CHILDREN);
        }
        let node = RetainedNode {
            id,
            parent,
            identity,
            ui_id,
            kind,
            signatures: signatures(element),
            dirty,
            children,
            phases,
        };
        let _ = position;
        self.nodes.insert(id, node);
        id
    }

    fn propagate_descendant_work(&mut self, root: RetainedNodeId) -> DirtyPhases {
        let children = self.nodes[&root].children.clone();
        let mut descendants = DirtyPhases::default();
        for child in children {
            descendants.insert(self.propagate_descendant_work(child));
        }
        let node = self.nodes.get_mut(&root).expect("retained node exists");
        if node.dirty.contains(DirtyPhases::CHILDREN) || descendants.contains(DirtyPhases::MEASURE)
        {
            node.dirty.insert(
                DirtyPhases::MEASURE
                    .union(DirtyPhases::PLACE)
                    .union(DirtyPhases::PAINT)
                    .union(DirtyPhases::INTERACTION)
                    .union(DirtyPhases::SEMANTICS),
            );
        } else if descendants.contains(DirtyPhases::PLACE) {
            node.dirty.insert(
                DirtyPhases::PLACE
                    .union(DirtyPhases::PAINT)
                    .union(DirtyPhases::INTERACTION)
                    .union(DirtyPhases::SEMANTICS),
            );
        }
        node.dirty
    }

    #[cfg(test)]
    pub(crate) fn nodes(&self) -> impl Iterator<Item = &RetainedNode> {
        self.nodes.values()
    }

    pub(crate) fn capture_layout(&mut self, id: &UiId, measured: Size, allocated: Rect) {
        if let Some(node) = self.nodes.values_mut().find(|node| &node.ui_id == id) {
            node.phases.measured = Some(measured);
            node.phases.measured_constraints = Some([
                allocated.size.width.to_bits(),
                allocated.size.height.to_bits(),
            ]);
            node.phases.allocated = Some(allocated);
        }
    }

    pub(crate) fn measured_for(&self, id: &UiId, constraints: Size) -> Option<Size> {
        let node = self.nodes.values().find(|node| &node.ui_id == id)?;
        (!node.dirty.contains(DirtyPhases::MEASURE)
            && node.phases.measured_constraints
                == Some([constraints.width.to_bits(), constraints.height.to_bits()]))
        .then_some(node.phases.measured)
        .flatten()
    }

    /// Returns whether measurement and placement records for the complete
    /// declaration subtree remain valid. Callers may reuse geometry only when
    /// the incoming parent allocation also matches the recorded allocation.
    pub(crate) fn subtree_layout_is_clean(&self, id: &UiId) -> bool {
        let Some(node) = self.nodes.values().find(|node| &node.ui_id == id) else {
            return false;
        };
        self.subtree_layout_is_clean_from(node.id)
    }

    fn subtree_layout_is_clean_from(&self, id: RetainedNodeId) -> bool {
        let node = &self.nodes[&id];
        !node.dirty.contains(DirtyPhases::MEASURE)
            && !node.dirty.contains(DirtyPhases::PLACE)
            && node.phases.measured.is_some()
            && node.phases.allocated.is_some()
            && node
                .children
                .iter()
                .all(|child| self.subtree_layout_is_clean_from(*child))
    }

    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub(crate) fn last(&self) -> ReconcileStats {
        self.last.clone()
    }

    pub(crate) fn estimated_bytes(&self) -> usize {
        self.nodes.capacity()
            * (std::mem::size_of::<RetainedNodeId>() + std::mem::size_of::<RetainedNode>())
            + self
                .nodes
                .values()
                .map(|node| {
                    let _parent = node.parent();
                    node.children.capacity() * std::mem::size_of::<RetainedNodeId>()
                        + node.phases.paint.capacity() * std::mem::size_of::<PaintCommand>()
                })
                .sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Column, Component, ComponentBuilderExt, CustomPaint, FrameRequest, Rect, Row, StyledText,
        StyledTextSpan, Text, TextUnderlineStyle, UiFrame, UiStateStore,
    };
    use proptest::prelude::*;

    fn keyed_text(id: &str, value: &str) -> Element<()> {
        let mut revision = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut revision);
        Text::new(value)
            .id(id)
            .content_revision(revision.finish())
            .width(80.0)
            .height(20.0)
            .into_element()
    }

    fn ids(arena: &RetainedNodeArena) -> HashMap<UiId, RetainedNodeId> {
        arena
            .nodes()
            .filter_map(|node| match &node.identity {
                ChildIdentity::Key(key) => Some((key.clone(), node.id)),
                ChildIdentity::Root | ChildIdentity::Position(_) => None,
            })
            .collect()
    }

    fn dirty_for(arena: &RetainedNodeArena, id: &str) -> DirtyPhases {
        arena
            .nodes()
            .find(|node| node.ui_id.as_str().ends_with(id))
            .expect("retained node")
            .dirty()
    }

    #[test]
    fn equal_length_middle_text_mutation_without_revision_cannot_authorize_measurement_reuse() {
        let prefix = "p".repeat(96);
        let suffix = "s".repeat(96);
        let before = format!("{prefix}iiiiiiii{suffix}");
        let after = format!("{prefix}ＷＷＷＷＷＷＷＷ{suffix}");
        assert_eq!(before.chars().count(), after.chars().count());

        let view = |value: &str| {
            Column::<()>::new()
                .child(Text::<()>::new(value).id("value"))
                .into_element()
        };
        let bounds = Rect::new(0.0, 0.0, 800.0, 200.0);
        let mut retained_state = UiStateStore::default();
        let first = UiFrame::resolve(
            view(&before),
            FrameRequest::new(bounds, &mut retained_state),
        );
        let next = UiFrame::resolve_against(
            view(&after),
            FrameRequest::new(bounds, &mut retained_state),
            &first,
        );
        assert!(next.resource_diagnostics().nodes_measured > 0);

        let mut cold_state = UiStateStore::default();
        let cold = UiFrame::resolve(view(&after), FrameRequest::new(bounds, &mut cold_state));
        assert_eq!(next.resolved_layout(), cold.resolved_layout());
        assert_eq!(next.commands(), cold.commands());
        assert_eq!(next.semantic_nodes(), cold.semantic_nodes());
    }

    #[test]
    fn styled_payload_revision_invalidates_equal_cardinality_span_changes() {
        let span = |bold| StyledTextSpan {
            range: 0..8,
            bold,
            italic: false,
            monospace: false,
            font_family: None,
            strikethrough: false,
            underline: TextUnderlineStyle::None,
            color: None,
            background: None,
        };
        let view = |bold, revision| {
            StyledText::<()>::new("abcdefgh", vec![span(bold)])
                .id("styled")
                .content_revision(revision)
        };
        let mut arena = RetainedNodeArena::default();
        arena.reconcile(&view(false, 1));
        arena.reconcile(&view(true, 2));
        let dirty = dirty_for(&arena, "styled");
        assert!(dirty.contains(DirtyPhases::MEASURE));
        assert!(dirty.contains(DirtyPhases::PAINT));
    }

    #[test]
    fn custom_paint_revision_detects_equal_length_middle_command_mutation() {
        let commands = |middle| {
            vec![
                PaintCommand::Fill {
                    rect: Rect::new(0.0, 0.0, 2.0, 2.0),
                    color: 1,
                },
                PaintCommand::Fill {
                    rect: Rect::new(2.0, 0.0, 2.0, 2.0),
                    color: middle,
                },
                PaintCommand::Fill {
                    rect: Rect::new(4.0, 0.0, 2.0, 2.0),
                    color: 3,
                },
            ]
        };
        let view = |middle, revision| {
            CustomPaint::<()>::commands(commands(middle))
                .id("paint")
                .content_revision(revision)
        };
        let mut arena = RetainedNodeArena::default();
        arena.reconcile(&view(2, 10));
        arena.reconcile(&view(9, 11));
        let dirty = dirty_for(&arena, "paint");
        assert!(dirty.contains(DirtyPhases::PAINT));
        assert!(!dirty.contains(DirtyPhases::MEASURE));
    }

    #[test]
    fn unchanged_declaration_reuses_every_node_and_phase() {
        let view = Column::new()
            .child(keyed_text("a", "A"))
            .child(keyed_text("b", "B"))
            .into_element();
        let mut arena = RetainedNodeArena::default();
        assert_eq!(arena.reconcile(&view).created, 3);
        let before = ids(&arena);
        let stats = arena.reconcile(&view);
        assert_eq!(stats.reused, 3);
        assert_eq!(stats.clean, 3);
        assert_eq!(ids(&arena), before);
        assert!(arena.nodes().all(|node| node.dirty().is_empty()));
    }

    #[test]
    fn paint_only_change_does_not_dirty_layout_or_semantics() {
        let mut arena = RetainedNodeArena::default();
        arena.reconcile(
            &Text::<()>::new("same")
                .id("leaf")
                .content_revision(1)
                .foreground(0xff01_0203)
                .into_element(),
        );
        arena.reconcile(
            &Text::<()>::new("same")
                .id("leaf")
                .content_revision(1)
                .foreground(0xff03_0201)
                .into_element(),
        );
        let leaf = arena
            .nodes()
            .find(|node| matches!(node.identity, ChildIdentity::Root))
            .unwrap();
        assert!(leaf.dirty().contains(DirtyPhases::PAINT));
        assert!(!leaf.dirty().contains(DirtyPhases::MEASURE));
        assert!(!leaf.dirty().contains(DirtyPhases::PLACE));
        assert!(!leaf.dirty().contains(DirtyPhases::SEMANTICS));
    }

    #[test]
    fn fixed_size_leaf_content_reuses_measurement_and_placement() {
        let mut arena = RetainedNodeArena::default();
        arena.reconcile(&keyed_text("leaf", "short"));
        let cached_size = Size::new(80.0, 20.0);
        let cached_bounds = Rect::new(4.0, 8.0, 80.0, 20.0);
        let leaf = arena.nodes.values_mut().next().unwrap();
        leaf.phases.measured = Some(cached_size);
        leaf.phases.allocated = Some(cached_bounds);
        leaf.phases.paint.push(PaintCommand::Fill {
            rect: cached_bounds,
            color: 0xff12_3456,
        });
        arena.reconcile(&keyed_text("leaf", "a much longer label"));
        let leaf = arena.nodes().next().unwrap();
        assert!(!leaf.dirty().contains(DirtyPhases::MEASURE));
        assert!(!leaf.dirty().contains(DirtyPhases::PLACE));
        assert!(leaf.dirty().contains(DirtyPhases::PAINT));
        assert!(leaf.dirty().contains(DirtyPhases::SEMANTICS));
        assert_eq!(leaf.phases.measured, Some(cached_size));
        assert_eq!(leaf.phases.allocated, Some(cached_bounds));
        assert!(leaf.phases.paint.is_empty());
    }

    #[test]
    fn keyed_insertion_and_reorder_preserve_identity_and_cleanup_removed_nodes() {
        let mut arena = RetainedNodeArena::default();
        let first = Row::new()
            .children([
                keyed_text("a", "A"),
                keyed_text("b", "B"),
                keyed_text("c", "C"),
            ])
            .into_element();
        arena.reconcile(&first);
        let original = ids(&arena);
        let reordered = Row::new()
            .children([
                keyed_text("x", "X"),
                keyed_text("c", "C"),
                keyed_text("a", "A"),
            ])
            .into_element();
        let stats = arena.reconcile(&reordered);
        let current = ids(&arena);
        assert_eq!(current[&UiId::from("a")], original[&UiId::from("a")]);
        assert_eq!(current[&UiId::from("c")], original[&UiId::from("c")]);
        assert!(!current.contains_key(&UiId::from("b")));
        assert_eq!(stats.created, 1);
        assert_eq!(stats.removed, 1);
        assert!(stats.moved >= 2);
        assert_eq!(arena.nodes().count(), 4);
    }

    #[test]
    fn positional_nodes_reuse_only_their_documented_slot() {
        let mut arena = RetainedNodeArena::default();
        let first = Column::new()
            .child(Text::<()>::new("A"))
            .child(Text::<()>::new("B"))
            .into_element();
        arena.reconcile(&first);
        let before = arena.nodes().map(RetainedNode::id).collect::<HashSet<_>>();
        let second = Column::new()
            .child(Text::<()>::new("B"))
            .child(Text::<()>::new("A"))
            .into_element();
        arena.reconcile(&second);
        assert_eq!(
            arena.nodes().map(RetainedNode::id).collect::<HashSet<_>>(),
            before
        );
    }

    #[test]
    fn semantic_and_paint_only_change_executes_no_measurement_or_placement() {
        let bounds = Rect::new(0.0, 0.0, 320.0, 120.0);
        let make_view = |label: &str, color| {
            Column::new()
                .width(320.0)
                .height(120.0)
                .child(
                    Text::<()>::new(label)
                        .id("label")
                        .width(120.0)
                        .height(24.0)
                        .foreground(color),
                )
                .into_element()
        };
        let mut retained_state = UiStateStore::default();
        let first = UiFrame::resolve(
            make_view("before", 0xff11_2233),
            FrameRequest::new(bounds, &mut retained_state),
        );
        assert_eq!(first.resource_diagnostics().nodes_measured, 2);
        assert_eq!(first.resource_diagnostics().nodes_placed, 2);

        let next = UiFrame::resolve_against(
            make_view("after", 0xff33_2211),
            FrameRequest::new(bounds, &mut retained_state),
            &first,
        );
        let work = next.resource_diagnostics();
        assert_eq!(work.nodes_measured, 0);
        assert_eq!(work.nodes_placed, 0);

        let mut cold_state = UiStateStore::default();
        let cold = UiFrame::resolve(
            make_view("after", 0xff33_2211),
            FrameRequest::new(bounds, &mut cold_state),
        );
        assert_eq!(next.resolved_layout(), cold.resolved_layout());
        assert_eq!(next.commands(), cold.commands());
        assert_eq!(next.semantic_nodes(), cold.semantic_nodes());
    }

    #[test]
    fn changed_constraints_reject_geometry_and_measurement_reuse() {
        let view = || {
            Column::new()
                .child(keyed_text("a", "A"))
                .child(keyed_text("b", "B"))
                .into_element()
        };
        let mut state = UiStateStore::default();
        let first = UiFrame::resolve(
            view(),
            FrameRequest::new(Rect::new(0.0, 0.0, 240.0, 120.0), &mut state),
        );
        let next = UiFrame::resolve_against(
            view(),
            FrameRequest::new(Rect::new(0.0, 0.0, 280.0, 120.0), &mut state),
            &first,
        );
        let work = next.resource_diagnostics();
        assert!(work.nodes_measured > 0);
        assert!(work.nodes_placed > 0);
    }

    fn assert_frame_oracle<Message: Clone + PartialEq + std::fmt::Debug>(
        incremental: &UiFrame<Message>,
        cold: &UiFrame<Message>,
    ) {
        assert_eq!(incremental.resolved_layout(), cold.resolved_layout());
        assert_eq!(incremental.commands(), cold.commands());
        assert_eq!(incremental.semantic_nodes(), cold.semantic_nodes());
        assert_eq!(
            incremental.accessibility_nodes(),
            cold.accessibility_nodes()
        );
        assert_eq!(
            incremental.interaction_record_counts(),
            cold.interaction_record_counts()
        );
    }

    #[test]
    fn output_phases_reuse_independently_from_proven_clean_authority() {
        let bounds = Rect::new(0.0, 0.0, 240.0, 80.0);
        let paint_view = |color| {
            Column::new()
                .child(keyed_text("label", "stable").foreground(color))
                .child(keyed_text("sibling", "unchanged"))
                .into_element()
        };
        let mut state = UiStateStore::default();
        let first = UiFrame::resolve(
            paint_view(0xff11_2233),
            FrameRequest::new(bounds, &mut state),
        );
        let painted = UiFrame::resolve_against(
            paint_view(0xff33_2211),
            FrameRequest::new(bounds, &mut state),
            &first,
        );
        let work = painted.resource_diagnostics();
        assert_eq!(work.paint_nodes_executed, 2);
        assert_eq!(work.paint_nodes_reused, 1);
        assert_eq!(work.interaction_nodes_executed, 0);
        assert_eq!(work.interaction_nodes_reused, 3);
        assert_eq!(work.semantic_nodes_executed, 0);
        assert_eq!(work.semantic_nodes_reused, 3);
        let mut cold_state = UiStateStore::default();
        let cold = UiFrame::resolve(
            paint_view(0xff33_2211),
            FrameRequest::new(bounds, &mut cold_state),
        );
        assert_frame_oracle(&painted, &cold);

        let interaction_view = |interactive| {
            let label = keyed_text("label", "stable");
            Column::new()
                .child(if interactive {
                    label.message(())
                } else {
                    label
                })
                .child(keyed_text("sibling", "unchanged"))
                .into_element()
        };
        let first = UiFrame::resolve(
            interaction_view(false),
            FrameRequest::new(bounds, &mut state),
        );
        let interactive = UiFrame::resolve_against(
            interaction_view(true),
            FrameRequest::new(bounds, &mut state),
            &first,
        );
        let work = interactive.resource_diagnostics();
        assert_eq!(work.paint_nodes_executed, 0);
        assert_eq!(work.paint_nodes_reused, 3);
        assert_eq!(work.interaction_nodes_executed, 2);
        assert_eq!(work.interaction_nodes_reused, 1);
        assert!(work.semantic_nodes_executed > 0);
        assert_eq!(work.semantic_nodes_reused, 2);
        let mut cold_state = UiStateStore::default();
        let cold = UiFrame::resolve(
            interaction_view(true),
            FrameRequest::new(bounds, &mut cold_state),
        );
        assert_frame_oracle(&interactive, &cold);

        let semantic_view = |role| {
            let view = Column::new()
                .child(keyed_text("label", "stable"))
                .child(keyed_text("sibling", "unchanged"));
            if role {
                view.accessibility_role("group").into_element()
            } else {
                view.into_element()
            }
        };
        let first = UiFrame::resolve(semantic_view(false), FrameRequest::new(bounds, &mut state));
        let semantic = UiFrame::resolve_against(
            semantic_view(true),
            FrameRequest::new(bounds, &mut state),
            &first,
        );
        let work = semantic.resource_diagnostics();
        assert_eq!(work.paint_nodes_executed, 0);
        assert_eq!(work.interaction_nodes_executed, 0);
        assert!(work.semantic_nodes_executed > 0);
        assert_eq!(work.paint_nodes_reused, 3);
        assert_eq!(work.interaction_nodes_reused, 3);
        assert_eq!(work.semantic_nodes_reused, 2);
        let mut cold_state = UiStateStore::default();
        let cold = UiFrame::resolve(
            semantic_view(true),
            FrameRequest::new(bounds, &mut cold_state),
        );
        assert_frame_oracle(&semantic, &cold);
    }

    proptest! {
        #[test]
        fn incremental_reconciliation_matches_a_cold_frame(
            operations in prop::collection::vec((0u8..4, 0u8..12), 1..48)
        ) {
            let bounds = Rect::new(0.0, 0.0, 420.0, 320.0);
            let mut order = vec![0u8, 1, 2, 3];
            let mut values = (0u8..12).map(|key| (key, key as u16)).collect::<HashMap<_, _>>();
            let make_view = |order: &[u8], values: &HashMap<u8, u16>| {
                Column::new()
                    .children(order.iter().map(|key| {
                        keyed_text(
                            &format!("item-{key}"),
                            &format!("value-{}", values[key]),
                        )
                    }))
                    .into_element()
            };

            let mut retained_state = UiStateStore::default();
            let mut retained = UiFrame::resolve(
                make_view(&order, &values),
                FrameRequest::new(bounds, &mut retained_state),
            );
            for (operation, key) in operations {
                let order_before = order.clone();
                match operation {
                    0 if !order.contains(&key) => order.insert((key as usize) % (order.len() + 1), key),
                    1 if order.len() > 1 => order.retain(|candidate| *candidate != key),
                    2 if order.contains(&key) => {
                        order.retain(|candidate| *candidate != key);
                        order.insert((key as usize * 7) % (order.len() + 1), key);
                    }
                    _ => *values.entry(key).or_default() += 1,
                }
                if order.is_empty() {
                    order.push(key);
                }

                let mut next_state = retained_state.clone();
                let next = UiFrame::resolve_against(
                    make_view(&order, &values),
                    FrameRequest::new(bounds, &mut next_state),
                    &retained,
                );
                if order == order_before {
                    let work = next.resource_diagnostics();
                    prop_assert_eq!(work.nodes_measured, 0);
                    prop_assert_eq!(work.nodes_placed, 0);
                    prop_assert_eq!(work.interaction_nodes_executed, 0);
                    prop_assert_eq!(work.interaction_nodes_reused, order.len() + 1);
                }
                let mut cold_state = retained_state.clone();
                let cold = UiFrame::resolve(
                    make_view(&order, &values),
                    FrameRequest::new(bounds, &mut cold_state),
                );
                prop_assert_eq!(next.resolved_layout(), cold.resolved_layout());
                prop_assert_eq!(next.commands(), cold.commands());
                prop_assert_eq!(next.semantic_nodes(), cold.semantic_nodes());
                prop_assert_eq!(next.interaction_record_counts(), cold.interaction_record_counts());
                prop_assert!(next.resource_diagnostics().estimated_retained_bytes <= 512 * 1024);
                retained = next;
                retained_state = next_state;
            }
        }
    }

    #[cfg(not(debug_assertions))]
    #[test]
    #[ignore = "release-profile retained layout admission benchmark"]
    fn retained_layout_release_admission_skips_work_and_beats_cold_resolution() {
        use crate::release_admission::AdmissionReport;

        const NODES: usize = 2_000;
        const CHANGED_LABEL: usize = NODES / 2;
        const ITERATIONS: usize = 2;
        const SAMPLES: usize = 5;

        fn view(changed_label: Option<char>) -> Element<()> {
            Column::new()
                .children((0..NODES).map(|index| {
                    let suffix = if index == CHANGED_LABEL {
                        changed_label.unwrap_or('a')
                    } else {
                        'a'
                    };
                    keyed_text(
                        &format!("item-{index}"),
                        &format!("retained admission item {index:04} value {suffix}"),
                    )
                }))
                .into_element()
        }

        let bounds = Rect::new(0.0, 0.0, 900.0, 800.0);
        let mut retained_state = UiStateStore::default();
        let mut retained =
            UiFrame::resolve(view(None), FrameRequest::new(bounds, &mut retained_state));
        let mut unchanged_samples = Vec::with_capacity(SAMPLES * ITERATIONS);
        let mut one_label_samples = Vec::with_capacity(SAMPLES * ITERATIONS);
        let mut cold_samples = Vec::with_capacity(SAMPLES);
        let mut unchanged_work = None;
        let mut label_work = None;
        for _ in 0..SAMPLES {
            for _ in 0..ITERATIONS {
                let started = std::time::Instant::now();
                let next = UiFrame::resolve_against(
                    view(None),
                    FrameRequest::new(bounds, &mut retained_state),
                    &retained,
                );
                let work = next.resource_diagnostics();
                assert_eq!(work.nodes_measured, 0);
                assert_eq!(work.nodes_placed, 0);
                assert_eq!(work.paint_nodes_executed, 0);
                assert_eq!(work.interaction_nodes_executed, 0);
                assert_eq!(work.semantic_nodes_executed, 0);
                unchanged_work.get_or_insert(work);
                assert_eq!(unchanged_work, Some(work));
                retained = next;
                unchanged_samples.push(started.elapsed());
            }

            for iteration in 0..ITERATIONS {
                let suffix = if iteration % 2 == 0 { 'b' } else { 'a' };
                let started = std::time::Instant::now();
                let next = UiFrame::resolve_against(
                    view(Some(suffix)),
                    FrameRequest::new(bounds, &mut retained_state),
                    &retained,
                );
                let work = next.resource_diagnostics();
                // The equal-width label mutation is paint-only: cached intrinsic
                // geometry remains authoritative while its fragment changes.
                assert_eq!(work.nodes_measured, 0);
                assert_eq!(work.nodes_placed, 0);
                assert!(work.paint_nodes_executed > 0);
                label_work.get_or_insert(work);
                assert_eq!(label_work, Some(work));
                retained = next;
                one_label_samples.push(started.elapsed());
            }

            let mut state = UiStateStore::default();
            let started = std::time::Instant::now();
            let cold = UiFrame::resolve(view(None), FrameRequest::new(bounds, &mut state));
            cold_samples.push(started.elapsed());
            assert_eq!(cold.resource_diagnostics().nodes_measured, NODES + 1);
            std::hint::black_box(cold.commands());
        }

        let unchanged_work = unchanged_work.expect("unchanged work sample");
        let label_work = label_work.expect("label work sample");
        AdmissionReport::new("retained_layout", "2000_node_unchanged_and_one_label")
            .metadata("nodes", NODES)
            .metadata("iterations_per_sample", ITERATIONS)
            .metadata("samples", SAMPLES)
            .work("unchanged_nodes_measured", unchanged_work.nodes_measured)
            .work("unchanged_nodes_placed", unchanged_work.nodes_placed)
            .work(
                "unchanged_paint_nodes_executed",
                unchanged_work.paint_nodes_executed,
            )
            .work(
                "unchanged_interaction_nodes_executed",
                unchanged_work.interaction_nodes_executed,
            )
            .work(
                "unchanged_semantic_nodes_executed",
                unchanged_work.semantic_nodes_executed,
            )
            .work("one_label_nodes_measured", label_work.nodes_measured)
            .work("one_label_nodes_placed", label_work.nodes_placed)
            .work(
                "one_label_paint_nodes_executed",
                label_work.paint_nodes_executed,
            )
            .work(
                "one_label_interaction_nodes_executed",
                label_work.interaction_nodes_executed,
            )
            .work(
                "one_label_semantic_nodes_executed",
                label_work.semantic_nodes_executed,
            )
            .timings("unchanged", &unchanged_samples)
            .timings("one_label_same_size", &one_label_samples)
            .timings("cold", &cold_samples)
            .emit();

        let retained =
            crate::release_admission::DurationDistribution::from_samples(&unchanged_samples).p95;
        let cold = crate::release_admission::DurationDistribution::from_samples(&cold_samples).p95;
        assert!(
            retained.as_nanos().saturating_mul(5) <= cold.as_nanos().saturating_mul(4),
            "retained layout p95 must be at least 20% faster: retained={retained:?}, cold={cold:?}"
        );
    }
}
