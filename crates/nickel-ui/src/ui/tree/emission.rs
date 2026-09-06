use super::*;

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

fn translate_custom_command(mut command: PaintCommand, origin: Point) -> PaintCommand {
    let translate = |rect: &mut Rect| {
        rect.origin.x += origin.x;
        rect.origin.y += origin.y;
    };
    match &mut command {
        PaintCommand::Fill { rect, .. }
        | PaintCommand::TopRoundedFill { rect, .. }
        | PaintCommand::RoundedFill { rect, .. }
        | PaintCommand::Gradient { rect, .. }
        | PaintCommand::Stroke { rect, .. } => translate(rect),
        PaintCommand::Text { bounds, .. }
        | PaintCommand::StyledText { bounds, .. }
        | PaintCommand::Image { bounds, .. } => translate(bounds),
        PaintCommand::OverlayFill { .. }
        | PaintCommand::OverlayStroke { .. }
        | PaintCommand::PushClip(_)
        | PaintCommand::PopClip => {}
    }
    command
}

pub(super) fn emit_element<Message: Clone>(
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
        Kind::CustomPaintCommands { commands } => {
            tree.commands.push(PaintCommand::PushClip(rect));
            tree.commands
                .extend(commands.iter().cloned().filter_map(|command| {
                    let command = translate_custom_command(command, rect.origin);
                    custom_paint_bounds(&command)
                        .is_some_and(|bounds| rect_contains(rect, bounds))
                        .then_some(command)
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
                font_size: None,
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
