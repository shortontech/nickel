use super::*;

pub(super) fn layout_element<Message: Clone>(
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

pub(super) fn apply_transient_state<Message>(
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

pub(super) fn resolve_grid_columns(
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

pub(super) fn explicit_px(length: Length) -> Option<f32> {
    match length {
        Length::Px(value) => Some(value.max(0.0)),
        _ => None,
    }
}
