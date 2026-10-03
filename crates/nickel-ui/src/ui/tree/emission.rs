use super::*;

fn paint_bounds(commands: &[PaintCommand], fallback: Rect) -> Rect {
    commands
        .iter()
        .filter_map(crate::gpu::command_bounds)
        .reduce(crate::gpu::union_rect)
        .unwrap_or(fallback)
}

pub(super) fn paint_dropdown_part(
    commands: &mut Vec<PaintCommand>,
    rect: Rect,
    style: &DropdownPartStyle,
    text: &str,
    align: TextAlign,
) {
    let background = style.background.filter(|color| *color != 0);
    let border = style.border_color.filter(|color| *color != 0);
    let rounded_border =
        style.radius > 0.0 && style.border_width > 0.0 && background.is_some() && border.is_some();
    if rounded_border {
        commands.push(PaintCommand::RoundedFill {
            rect,
            color: border.unwrap(),
            radius: style.radius,
        });
        commands.push(PaintCommand::RoundedFill {
            rect: rect.inset(Insets::all(style.border_width)),
            color: background.unwrap(),
            radius: (style.radius - style.border_width).max(0.0),
        });
    } else {
        if let Some(color) = background {
            commands.push(if style.radius > 0.0 {
                PaintCommand::RoundedFill {
                    rect,
                    color,
                    radius: style.radius,
                }
            } else {
                PaintCommand::Fill { rect, color }
            });
        }
        if let Some(color) = border
            && style.border_width > 0.0
        {
            commands.push(if style.radius > 0.0 {
                PaintCommand::RoundedStroke {
                    rect,
                    color,
                    width: style.border_width,
                    radius: style.radius,
                }
            } else {
                PaintCommand::Stroke {
                    rect,
                    color,
                    width: style.border_width,
                }
            });
        }
    }
    if let Some(color) = style.foreground.filter(|color| *color != 0)
        && style.font_size > 0.0
    {
        let mut bounds = rect.inset(style.padding);
        if style.line_height > 0.0 {
            bounds.size.height = style.line_height.min(bounds.size.height);
        }
        commands.push(PaintCommand::Text {
            bounds,
            text: text.into(),
            scale: -style.font_size,
            color,
            align,
            bold: false,
            wrap: false,
        });
    }
}

fn custom_paint_bounds(command: &PaintCommand) -> Option<Rect> {
    match command {
        PaintCommand::BackdropBlur { rect, .. }
        | PaintCommand::Fill { rect, .. }
        | PaintCommand::TopRoundedFill { rect, .. }
        | PaintCommand::RoundedFill { rect, .. }
        | PaintCommand::Gradient { rect, .. }
        | PaintCommand::RoundedStroke { rect, .. }
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
        PaintCommand::BackdropBlur { rect, .. }
        | PaintCommand::Fill { rect, .. }
        | PaintCommand::TopRoundedFill { rect, .. }
        | PaintCommand::RoundedFill { rect, .. }
        | PaintCommand::Gradient { rect, .. }
        | PaintCommand::RoundedStroke { rect, .. }
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

fn slider_track_rect(content: Rect, style: &DropdownPartStyle) -> Rect {
    let available = content.inset(style.margin);
    Rect::new(
        available.origin.x,
        available.origin.y + (available.size.height - style.height) / 2.0,
        if style.width > 0.0 {
            style.width.min(available.size.width)
        } else {
            available.size.width
        },
        style.height,
    )
}

pub(super) fn emit_element<Message: Clone>(
    element: &Element<Message>,
    node_index: usize,
    inherited_foreground: Option<Color>,
    tree: &mut UiFrame<Message>,
    previous: Option<&UiFrame<Message>>,
) {
    let node = tree.resolved.nodes[node_index].clone();
    let fragment_start = tree.commands.len();
    let rect = node.allocated;
    let value_bounds = match &element.kind {
        Kind::Slider {
            presentation: Some(parts),
            ..
        } if element.style.css_paint => {
            let track = slider_track_rect(node.content, &parts[0]).inset(parts[0].padding);
            Rect::new(
                track.origin.x,
                node.content.origin.y,
                track.size.width,
                node.content.size.height,
            )
        }
        _ => rect,
    };
    if element.focus_message.is_some() || element.blur_message.is_some() {
        tree.focus_messages.push((
            node.id.clone(),
            element.focus_message.clone(),
            element.blur_message.clone(),
        ));
    }
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
                rect: value_bounds,
                message: message.clone(),
                message_mapper: element.message_mapper,
                seeded_value_mapper: element.seeded_value_mapper,
            });
        }
        if let Some(message) = &element.context_message {
            tree.context_messages.push(MessageRegion {
                id: node.id.clone(),
                navigation_owner: None,
                rect: value_bounds,
                message: message.clone(),
                message_mapper: None,
                seeded_value_mapper: None,
            });
        }
        let foreground = element.style.foreground.or(inherited_foreground);
        if matches!(
            element.kind,
            Kind::Flex(_) | Kind::Grid { .. } | Kind::Layer
        ) {
            for (&child_index, child) in node.children.iter().zip(&element.children) {
                tree.emit_incremental(child, child_index, foreground, previous);
            }
        }
        tree.paint_fragments.push(PaintFragment {
            id: node.id,
            commands: fragment_start..tree.commands.len(),
            bounds: paint_bounds(&tree.commands[fragment_start..], node.border_box),
        });
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
    if let Some(blur) = element.style.backdrop_blur
        && blur > 0.0
    {
        tree.commands.push(PaintCommand::BackdropBlur {
            rect,
            radius: element.style.corner_radius,
            blur,
        });
    }
    if let Some(shadow) = element.style.box_shadow
        && shadow.color != 0
    {
        let layers = shadow.blur.ceil().clamp(1.0, 12.0) as u32;
        let source_alpha = (shadow.color >> 24) & 0xff;
        for layer in (1..=layers).rev() {
            let blur = shadow.blur * layer as f32 / layers as f32;
            let expansion = (shadow.spread + blur).max(0.0);
            let alpha = ((source_alpha as f32 / layers as f32) * 1.8)
                .round()
                .clamp(1.0, 255.0) as u32;
            let color = (shadow.color & 0x00ff_ffff) | (alpha << 24);
            tree.commands.push(PaintCommand::RoundedFill {
                rect: Rect::new(
                    rect.origin.x + shadow.offset_x - expansion,
                    rect.origin.y + shadow.offset_y - expansion,
                    rect.size.width + expansion * 2.0,
                    rect.size.height + expansion * 2.0,
                ),
                color,
                radius: element.style.corner_radius + expansion,
            });
        }
    }
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
            tree.commands.push(if element.style.corner_radius > 0.0 {
                PaintCommand::RoundedStroke {
                    rect,
                    color,
                    width: element.style.border_width,
                    radius: element.style.corner_radius,
                }
            } else {
                PaintCommand::Stroke {
                    rect,
                    color,
                    width: element.style.border_width,
                }
            });
        }
    }
    if let Some(message) = &element.message {
        tree.messages.push(MessageRegion {
            id: node.id.clone(),
            navigation_owner: None,
            rect: value_bounds,
            message: message.clone(),
            message_mapper: element.message_mapper,
            seeded_value_mapper: element.seeded_value_mapper,
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
                value_bounds: (value_bounds != rect).then_some(value_bounds),
                message: Some(message.clone()),
                message_mapper: element.message_mapper,
                seeded_value_mapper: element.seeded_value_mapper,
                drag_seed: element.drag_seed.clone(),
                drag_mapper: element.drag_mapper,
                drop_message: element.drop_message.clone(),
                drop_mapper: element.drop_mapper,
            });
        }
    }
    if element.message.is_none()
        && element.context_message.is_none()
        && element.drop_mapper.is_some()
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
            value_bounds: None,
            message: None,
            message_mapper: None,
            seeded_value_mapper: None,
            drag_seed: None,
            drag_mapper: None,
            drop_message: element.drop_message.clone(),
            drop_mapper: element.drop_mapper,
        });
    }
    if let Some(message) = &element.context_message {
        tree.context_messages.push(MessageRegion {
            id: node.id.clone(),
            navigation_owner: None,
            rect: value_bounds,
            message: message.clone(),
            message_mapper: None,
            seeded_value_mapper: None,
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
                value_bounds: (value_bounds != rect).then_some(value_bounds),
                message: None,
                message_mapper: None,
                seeded_value_mapper: None,
                drag_seed: element.drag_seed.clone(),
                drag_mapper: element.drag_mapper,
                drop_message: element.drop_message.clone(),
                drop_mapper: element.drop_mapper,
            });
        }
    }
    if element.message.is_none()
        && element.context_message.is_none()
        && element.drag_seed.is_some()
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
            value_bounds: (value_bounds != rect).then_some(value_bounds),
            message: None,
            message_mapper: None,
            seeded_value_mapper: None,
            drag_seed: element.drag_seed.clone(),
            drag_mapper: element.drag_mapper,
            drop_message: element.drop_message.clone(),
            drop_mapper: element.drop_mapper,
        });
    }
    if let Some(map) = &element.text_mapper
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
            content: node.content,
            scale,
            bold,
            line_height,
            initial: input_value.clone().unwrap_or_else(|| value.clone()),
            map: map.clone(),
            secure,
            context_menu_presentation: element.style.editing_menu.clone(),
            context_menu_style: crate::OverlayStyle {
                background: match element.style.background {
                    Some(Background::Solid(color)) => color,
                    _ => crate::theme::FALLBACK_FOCUS_SURFACE,
                },
                foreground: element.style.foreground.unwrap_or(0xe8edf4),
                border: element
                    .style
                    .focus_background_tint
                    .unwrap_or(crate::theme::FALLBACK_KEYBOARD_FOCUS_CUE),
                selected: element
                    .style
                    .controller_focus_background_tint
                    .or(element.style.focus_background_tint)
                    .unwrap_or(crate::theme::FALLBACK_CONTROLLER_FOCUS_CUE),
                radius: 7,
            },
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
                value_bounds: (value_bounds != rect).then_some(value_bounds),
                message: None,
                message_mapper: None,
                seeded_value_mapper: None,
                drag_seed: None,
                drag_mapper: None,
                drop_message: None,
                drop_mapper: None,
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
    let text_rect = if element.text_mapper.is_some() {
        node.content
    } else {
        rect
    };
    if let Kind::Text {
        scale,
        line_height,
        selection_x: Some((start, end)),
        ..
    } = &element.kind
    {
        let color = element
            .style
            .editing_parts
            .as_ref()
            .map_or(Some(0x315a8f), |parts| parts[1].background);
        if let Some(color) = color.filter(|color| *color != 0) {
            let radius = element
                .style
                .editing_parts
                .as_ref()
                .map_or(0.0, |parts| parts[1].radius);
            let selection_rect = Rect::new(
                text_rect.origin.x + *start,
                text_rect.origin.y,
                (*end - *start).max(1.0),
                line_height.unwrap_or_else(|| text_font_size(*scale) * 1.3),
            );
            tree.commands.push(if radius > 0.0 {
                PaintCommand::RoundedFill {
                    rect: selection_rect,
                    color,
                    radius,
                }
            } else {
                PaintCommand::Fill {
                    rect: selection_rect,
                    color,
                }
            });
        }
    }
    if let Kind::Text {
        scale,
        line_height,
        caret_position: Some(caret_position),
        ..
    } = &element.kind
    {
        let parts = element.style.editing_parts.as_ref();
        let color = parts.map_or_else(
            || Some(foreground.unwrap_or(0x00ff_ffff)),
            |parts| parts[0].background,
        );
        if let Some(color) = color.filter(|color| *color != 0) {
            let width = parts.map_or(1.5, |parts| parts[0].width);
            let radius = parts.map_or(0.0, |parts| parts[0].radius);
            let height = parts.filter(|parts| parts[0].height > 0.0).map_or_else(
                || line_height.unwrap_or_else(|| text_font_size(*scale) * 1.3),
                |parts| parts[0].height,
            );
            let caret_rect = Rect::new(
                text_rect.origin.x + caret_position.x,
                text_rect.origin.y + caret_position.y,
                width,
                height,
            );
            tree.commands.push(if radius > 0.0 {
                PaintCommand::RoundedFill {
                    rect: caret_rect,
                    color,
                    radius,
                }
            } else {
                PaintCommand::Fill {
                    rect: caret_rect,
                    color,
                }
            });
        }
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
            let text = text_for_bounds(value, *scale, *bold, *ellipsis, text_rect.size.width);
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
                            text_rect.origin.x + x * *width,
                            text_rect.origin.y + y * *width,
                            text_rect.size.width,
                            text_rect.size.height,
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
            if !element.style.css_paint || foreground.is_some_and(|color| color != 0) {
                tree.commands.push(PaintCommand::Text {
                    bounds: text_rect,
                    text,
                    scale: *scale,
                    color: foreground.unwrap_or(0x00ff_ffff),
                    align: element.style.text_align,
                    bold: *bold,
                    wrap: *wrap,
                });
            }
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
                            seeded_value_mapper: None,
                        });
                        tree.hits.push(HitRegion {
                            id: link_id.clone(),
                            rect: glyph.rect,
                            target_bounds: glyph.rect,
                            value_bounds: None,
                            message: Some(message.clone()),
                            message_mapper: None,
                            seeded_value_mapper: None,
                            drag_seed: None,
                            drag_mapper: None,
                            drop_message: None,
                            drop_mapper: None,
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
            thumb_border,
            geometry,
            presentation,
        } => {
            if let Some(parts) = presentation {
                let track_style = &parts[0];
                let track_rect = slider_track_rect(node.content, track_style);
                paint_dropdown_part(
                    &mut tree.commands,
                    track_rect,
                    track_style,
                    "",
                    TextAlign::Start,
                );
                let content = track_rect.inset(track_style.padding);
                let fill_style = &parts[1];
                let fill_available = content.inset(fill_style.margin);
                let fill_rect = Rect::new(
                    fill_available.origin.x,
                    fill_available.origin.y,
                    fill_available.size.width * value.clamp(0.0, 1.0),
                    if fill_style.height > 0.0 {
                        fill_style.height
                    } else {
                        fill_available.size.height
                    },
                );
                paint_dropdown_part(
                    &mut tree.commands,
                    fill_rect,
                    fill_style,
                    "",
                    TextAlign::Start,
                );
                let thumb_style = &parts[2];
                let thumb_available = node.content.inset(thumb_style.margin);
                let width = thumb_style.width.min(thumb_available.size.width);
                let height = thumb_style.height.min(thumb_available.size.height);
                let x = (content.origin.x + content.size.width * value.clamp(0.0, 1.0)
                    - width / 2.0)
                    .clamp(
                        thumb_available.origin.x,
                        thumb_available.origin.x + thumb_available.size.width - width,
                    );
                let thumb_rect = Rect::new(
                    x,
                    thumb_available.origin.y + (thumb_available.size.height - height) / 2.0,
                    width,
                    height,
                );
                paint_dropdown_part(
                    &mut tree.commands,
                    thumb_rect,
                    thumb_style,
                    "",
                    TextAlign::Start,
                );
            } else {
                let [
                    track_height,
                    track_radius,
                    thumb_width,
                    thumb_height,
                    thumb_radius,
                    border_width,
                ] = *geometry;
                let thumb_width = thumb_width.min(rect.size.width);
                let thumb_height = thumb_height.min(rect.size.height);
                let border_width = border_width.min(thumb_width.min(thumb_height) / 2.0);
                let track_rect = Rect::new(
                    rect.origin.x,
                    rect.origin.y + rect.size.height / 2.0 - track_height / 2.0,
                    rect.size.width,
                    track_height,
                );
                let fill_width = track_rect.size.width * value.clamp(0.0, 1.0);
                if *track != 0 {
                    tree.commands.push(PaintCommand::RoundedFill {
                        rect: track_rect,
                        color: *track,
                        radius: track_radius,
                    });
                }
                if *fill != 0 {
                    tree.commands.push(PaintCommand::RoundedFill {
                        rect: Rect::new(
                            track_rect.origin.x,
                            track_rect.origin.y,
                            fill_width,
                            track_rect.size.height,
                        ),
                        color: *fill,
                        radius: track_radius,
                    });
                }
                let thumb_rect = Rect::new(
                    (track_rect.origin.x + fill_width - thumb_width / 2.0)
                        .clamp(rect.origin.x, rect.origin.x + rect.size.width - thumb_width),
                    rect.origin.y + rect.size.height / 2.0 - thumb_height / 2.0,
                    thumb_width,
                    thumb_height,
                );
                if *thumb_border != 0 {
                    tree.commands.push(PaintCommand::RoundedFill {
                        rect: thumb_rect,
                        color: *thumb_border,
                        radius: thumb_radius,
                    });
                }
                if *thumb != 0 {
                    tree.commands.push(PaintCommand::RoundedFill {
                        rect: Rect::new(
                            thumb_rect.origin.x + border_width,
                            thumb_rect.origin.y + border_width,
                            thumb_rect.size.width - 2.0 * border_width,
                            thumb_rect.size.height - 2.0 * border_width,
                        ),
                        color: *thumb,
                        radius: (thumb_radius - border_width).max(0.0),
                    });
                }
            }
        }
        Kind::Dropdown {
            selected,
            options,
            expanded,
            overlay,
            background,
            option_background,
            foreground,
            presentation,
            option_presentations,
            resolved_options,
            ..
        } => {
            let rect = node.content;
            let header_height = presentation
                .as_ref()
                .map_or(if *overlay { 30.0 } else { 42.0 }, |parts| {
                    parts[0].height + parts[0].margin.height()
                });
            let option_height = presentation
                .as_ref()
                .map_or(if *overlay { 34.0 } else { 36.0 }, |parts| {
                    parts[1].height + parts[1].margin.height()
                });
            let option_part = |index: usize| {
                resolved_options
                    .get(index)
                    .or_else(|| option_presentations.get(index))
            };
            let row_height = |index: usize| {
                option_part(index).map_or(option_height, |part| part.height + part.margin.height())
            };
            let options_height = (0..options.len()).map(row_height).sum::<f32>();
            let options_origin_y = if *overlay
                && rect.origin.y + header_height + options_height
                    > tree.viewport.origin.y + tree.viewport.size.height
                && rect.origin.y - options_height >= tree.viewport.origin.y
            {
                rect.origin.y - options_height
            } else {
                rect.origin.y + header_height
            };
            let option_bounds = |index: usize| {
                let y = options_origin_y + (0..index).map(row_height).sum::<f32>();
                let bounds = Rect::new(rect.origin.x, y, rect.size.width, row_height(index));
                if let Some(part) = option_part(index) {
                    let mut bounds = bounds.inset(part.margin);
                    if part.width > 0.0 {
                        bounds.size.width = bounds.size.width.min(part.width);
                    }
                    bounds
                } else {
                    bounds
                }
            };
            let header = Rect::new(rect.origin.x, rect.origin.y, rect.size.width, header_height);
            let header = presentation.as_ref().map_or(header, |parts| {
                let mut header = header.inset(parts[0].margin);
                if parts[0].width > 0.0 {
                    header.size.width = header.size.width.min(parts[0].width);
                }
                header
            });
            if let Some(parts) = presentation {
                paint_dropdown_part(
                    &mut tree.commands,
                    header,
                    &parts[0],
                    selected,
                    TextAlign::Start,
                );
                let indicator = &parts[2];
                let indicator_available = header.inset(indicator.margin);
                let indicator_width = indicator.width.min(indicator_available.size.width);
                let indicator_height = if indicator.height > 0.0 {
                    indicator.height.min(indicator_available.size.height)
                } else {
                    indicator_available.size.height
                };
                let indicator_rect = Rect::new(
                    indicator_available.origin.x + indicator_available.size.width - indicator_width,
                    indicator_available.origin.y
                        + (indicator_available.size.height - indicator_height) / 2.0,
                    indicator_width,
                    indicator_height,
                );
                paint_dropdown_part(
                    &mut tree.commands,
                    indicator_rect,
                    indicator,
                    if *expanded { "▲" } else { "▼" },
                    TextAlign::Center,
                );
            } else {
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
            }
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
                            rect: option_bounds(index),
                            message: message.clone(),
                            message_mapper: None,
                            seeded_value_mapper: None,
                        });
                    }
                }
            }
            if *expanded {
                for (index, option) in options.iter().enumerate() {
                    let option_rect = option_bounds(index);
                    let commands = if *overlay {
                        &mut tree.overlay_commands
                    } else {
                        &mut tree.commands
                    };
                    if let Some(parts) = presentation {
                        paint_dropdown_part(
                            commands,
                            option_rect,
                            option_part(index).unwrap_or(&parts[1]),
                            option,
                            TextAlign::Start,
                        );
                    } else {
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
                    }
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
                        content: option_rect.inset(
                            option_part(index).map_or(Insets::all(8.0), |part| part.padding),
                        ),
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
                        auto_focus: false,
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
                            seeded_value_mapper: None,
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
                            value_bounds: None,
                            message,
                            message_mapper: None,
                            seeded_value_mapper: None,
                            drag_seed: None,
                            drag_mapper: None,
                            drop_message: None,
                            drop_mapper: None,
                        });
                    }
                }
            }
        }
        Kind::VerticalScroll { .. } => {
            let content_clip = node
                .children
                .first()
                .and_then(|index| tree.resolved.nodes.get(*index))
                .and_then(|child| child.clip)
                .unwrap_or(node.content);
            tree.commands.push(PaintCommand::PushClip(content_clip));
            for (&child_index, child) in node.children.iter().zip(&element.children) {
                tree.emit_incremental(child, child_index, foreground, previous);
            }
            tree.commands.push(PaintCommand::PopClip);
        }
        Kind::Flex(_) | Kind::Grid { .. } | Kind::Layer => {
            for (&child_index, child) in node.children.iter().zip(&element.children) {
                tree.emit_incremental(child, child_index, foreground, previous);
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
    tree.paint_fragments.push(PaintFragment {
        id: node.id,
        commands: fragment_start..tree.commands.len(),
        bounds: paint_bounds(&tree.commands[fragment_start..], node.border_box),
    });
}
