use super::*;
use crate::SemanticTheme;

pub struct Layer<Message = String>(Element<Message>);

impl<Message> Default for Layer<Message> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Message> Layer<Message> {
    pub fn new() -> Self {
        Self(Element::layer())
    }
    pub fn child(mut self, child: impl Component<Message>) -> Self {
        self.0 = self.0.child(child);
        self
    }
    pub fn children(mut self, children: impl IntoIterator<Item = impl Component<Message>>) -> Self {
        self.0 = self.0.children(children);
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.0 = self.0.width(width);
        self
    }
    pub fn height(mut self, height: f32) -> Self {
        self.0 = self.0.height(height);
        self
    }
    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }
}

impl<Message> Component<Message> for Layer<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

pub struct Spacer<Message = String>(Element<Message>);

impl<Message> Default for Spacer<Message> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Message> Spacer<Message> {
    pub fn new() -> Self {
        Self::flex()
    }

    pub fn flex() -> Self {
        Self(Element::flex(Axis::Horizontal).grow(1.0))
    }

    pub fn fixed(size: f32) -> Self {
        Self(Element::flex(Axis::Horizontal).width(size).height(size))
    }

    pub fn vertical(height: f32) -> Self {
        Self(Element::flex(Axis::Horizontal).height(height).grow(0.0))
    }

    pub fn grow(mut self, grow: f32) -> Self {
        self.0 = self.0.grow(grow);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VirtualWindow {
    pub range: Range<usize>,
    pub leading: f32,
    pub trailing: f32,
    pub total: f32,
}

impl VirtualWindow {
    pub fn from_heights(
        heights: &[f32],
        gap: f32,
        offset: f32,
        viewport: f32,
        overscan: f32,
    ) -> Self {
        let gap = gap.max(0.0);
        let viewport = viewport.max(0.0);
        let overscan = overscan.max(0.0);
        let mut starts = Vec::with_capacity(heights.len());
        let mut ends = Vec::with_capacity(heights.len());
        let mut cursor = 0.0;
        for (index, height) in heights.iter().enumerate() {
            if index > 0 {
                cursor += gap;
            }
            starts.push(cursor);
            cursor += height.max(0.0);
            ends.push(cursor);
        }
        let total = cursor;
        let offset = offset.clamp(0.0, (total - viewport).max(0.0));
        let minimum = (offset - overscan).max(0.0);
        let maximum = (offset + viewport + overscan).min(total);
        let first = ends
            .iter()
            .position(|end| *end >= minimum)
            .unwrap_or(heights.len());
        let end = starts
            .iter()
            .rposition(|start| *start <= maximum)
            .map_or(first, |index| index + 1)
            .max(first);
        let leading = starts.get(first).copied().unwrap_or(total);
        let visible_end = end
            .checked_sub(1)
            .and_then(|index| ends.get(index))
            .copied()
            .unwrap_or(leading);
        Self {
            range: first..end,
            leading,
            trailing: (total - visible_end).max(0.0),
            total,
        }
    }
}

pub struct VirtualColumn<Message = String> {
    window: VirtualWindow,
    visible: Column<Message>,
}

impl<Message> VirtualColumn<Message> {
    pub fn new() -> Self {
        Self {
            window: VirtualWindow::from_heights(&[], 0.0, 0.0, 0.0, 0.0),
            visible: Column::new().fill_width(),
        }
    }

    pub fn window(mut self, window: VirtualWindow) -> Self {
        self.window = window;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.visible = self.visible.gap(gap);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Component<Message>>) -> Self {
        self.visible = self.visible.children(children);
        self
    }

    pub fn max_width(mut self, width: f32) -> Self {
        self.visible = self.visible.max_width(width);
        self
    }

    pub fn align_self(mut self, align: Align) -> Self {
        self.visible = self.visible.align_self(align);
        self
    }
}

impl<Message> Default for VirtualColumn<Message> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Message> Component<Message> for VirtualColumn<Message> {
    fn into_element(self) -> Element<Message> {
        Column::new()
            .fill_width()
            .child(Spacer::vertical(self.window.leading))
            .child(self.visible)
            .child(Spacer::vertical(self.window.trailing))
            .into_element()
    }
}

impl<Message> Component<Message> for Spacer<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

pub struct VerticalScroll<Message = String>(Element<Message>);

impl<Message> VerticalScroll<Message> {
    pub fn new(message: Message, offset: f32) -> Self {
        Self(Element {
            id: None,
            source: None,
            kind: Kind::VerticalScroll {
                offset: offset.max(0.0),
                controlled: false,
            },
            style: Style::default(),
            message: Some(message),
            context_message: None,
            message_mapper: None,
            drag_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
            navigation_scope: None,
            adjustment_step: 0.05,
        })
    }

    pub fn child(mut self, child: impl Component<Message>) -> Self {
        self.0 = self.0.child(child);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Component<Message>>) -> Self {
        self.0 = self.0.children(children);
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn navigation_scope(mut self, scope: crate::NavigationScope) -> Self {
        self.0 = self.0.navigation_scope(scope);
        self
    }

    pub fn on_scroll(mut self, map: fn(f32) -> Message) -> Self {
        self.0 = self.0.on_scroll(map);
        self
    }

    /// Keep the supplied offset authoritative across reconstruction.
    pub fn controlled(mut self, controlled: bool) -> Self {
        if let Kind::VerticalScroll {
            controlled: value, ..
        } = &mut self.0.kind
        {
            *value = controlled;
        }
        self
    }

    /// Paint this shared scrollbar from the application's live semantic theme.
    pub fn theme(mut self, theme: crate::SemanticTheme) -> Self {
        self.0.style.scrollbar_palette = theme.scrollbar_palette();
        self
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.0 = self.0.background(background);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.0 = self.0.height(height);
        self
    }

    pub fn grow(mut self, grow: f32) -> Self {
        self.0 = self.0.grow(grow);
        self
    }
}

impl<Message> Component<Message> for VerticalScroll<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

pub struct Grid<Message = String>(Element<Message>);

impl<Message> Grid<Message> {
    pub fn new() -> Self {
        Self(Element {
            id: None,
            source: None,
            kind: Kind::Grid {
                columns: GridColumnSpec::Count(2),
            },
            style: Style::default(),
            message: None,
            context_message: None,
            message_mapper: None,
            drag_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
            navigation_scope: None,
            adjustment_step: 0.05,
        })
    }

    pub fn fixed(columns: usize) -> Self {
        Self::new().columns(columns)
    }

    pub fn columns(mut self, columns: impl Into<GridColumnSpec>) -> Self {
        if let Kind::Grid {
            columns: definition,
        } = &mut self.0.kind
        {
            *definition = columns.into();
        }
        self
    }

    pub fn tracks(tracks: impl IntoIterator<Item = Track>) -> Self {
        let tracks = tracks.into_iter().collect::<Vec<_>>();
        Self(Element {
            id: None,
            source: None,
            kind: Kind::Grid {
                columns: GridColumnSpec::Tracks(tracks),
            },
            style: Style::default(),
            message: None,
            context_message: None,
            message_mapper: None,
            drag_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
            navigation_scope: None,
            adjustment_step: 0.05,
        })
    }

    pub fn auto_fit(track: Track) -> Self {
        Self(Element {
            id: None,
            source: None,
            kind: Kind::Grid {
                columns: GridColumnSpec::AutoFit(track),
            },
            style: Style::default(),
            message: None,
            context_message: None,
            message_mapper: None,
            drag_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
            navigation_scope: None,
            adjustment_step: 0.05,
        })
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Component<Message>>) -> Self {
        self.0 = self.0.children(children);
        self
    }

    pub fn direction(mut self, direction: ReadingDirection) -> Self {
        if direction == ReadingDirection::RightToLeft {
            self.0.children.reverse();
        }
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn semantic_role(mut self, role: SemanticRole) -> Self {
        self.0 = self.0.semantic_role(role);
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.0 = self.0.gap(gap);
        self
    }

    pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
        self.0 = self.0.padding(padding);
        self
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.0 = self.0.background(background);
        self
    }

    pub fn border_value(mut self, border: impl Into<Border>) -> Self {
        self.0 = self.0.border_value(border);
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.0 = self.0.radius(radius);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.0 = self.0.width(width);
        self
    }

    pub fn min_width(mut self, width: f32) -> Self {
        self.0 = self.0.min_width(width);
        self
    }

    pub fn max_width(mut self, width: f32) -> Self {
        self.0 = self.0.max_width(width);
        self
    }

    pub fn fill_width(mut self) -> Self {
        self.0 = self.0.fill_width();
        self
    }

    pub fn overflow_x(mut self, overflow: Overflow) -> Self {
        self.0 = self.0.overflow_x(overflow);
        self
    }

    pub fn overflow_y(mut self, overflow: Overflow) -> Self {
        self.0 = self.0.overflow_y(overflow);
        self
    }

    pub fn scrollbar_theme(mut self, theme: crate::SemanticTheme) -> Self {
        self.0 = self.0.scrollbar_theme(theme);
        self
    }

    pub fn grow(mut self, grow: f32) -> Self {
        self.0 = self.0.grow(grow);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.0 = self.0.height(height);
        self
    }
}

impl<Message> Default for Grid<Message> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Message> Component<Message> for Grid<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

pub struct FileGrid<Message = String> {
    grid: Grid<Message>,
}

impl<Message> FileGrid<Message> {
    pub fn columns(columns: usize) -> Self {
        let mut grid = Grid::fixed(columns);
        grid.0.navigation_scope =
            Some(crate::NavigationScope::group().traversal(crate::NavigationTraversal::Grid));
        Self { grid }
    }

    pub fn auto_fit(minimum_width: f32) -> Self {
        let mut grid = Grid::auto_fit(Track::minmax(
            Track::px(minimum_width.max(1.0)),
            Track::fr(1.0),
        ));
        grid.0.navigation_scope =
            Some(crate::NavigationScope::group().traversal(crate::NavigationTraversal::Grid));
        Self { grid }
    }

    pub fn items<C>(mut self, items: impl IntoIterator<Item = C>) -> Self
    where
        C: Component<Message>,
    {
        self.grid = self.grid.children(items);
        self
    }

    pub fn children<C>(self, items: impl IntoIterator<Item = C>) -> Self
    where
        C: Component<Message>,
    {
        self.items(items)
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.grid = self.grid.gap(gap);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.grid = self.grid.height(height);
        self
    }

    pub fn grow(mut self, grow: f32) -> Self {
        self.grid = self.grid.grow(grow);
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.grid.0.id = Some(id.into());
        self
    }

    pub fn accessibility_label(mut self, label: impl Into<String>) -> Self {
        self.grid.0 = self
            .grid
            .0
            .semantic_role(SemanticRole::Grid)
            .accessibility_label(label);
        self
    }

    pub fn scroll_owner(mut self, owner: impl Into<UiId>) -> Self {
        self.grid.0.navigation_scope = Some(
            self.grid
                .0
                .navigation_scope
                .take()
                .unwrap_or_else(crate::NavigationScope::group)
                .scroll_owner(owner.into()),
        );
        self
    }
}

impl<Message> Component<Message> for FileGrid<Message> {
    fn into_element(self) -> Element<Message> {
        self.grid.into_element()
    }
}

/// Complete shared entry surface for a file-like plane.
///
/// Hosts supply policy (messages, dimensions, colors, and selection state), while
/// this component owns the hit region, semantics, interaction visuals, icon
/// containment, and bounded label layout.
pub struct FilePlaneItem<Message = String> {
    container: Container<Message>,
}

impl<Message> FilePlaneItem<Message> {
    pub fn new(
        message: Message,
        label: impl Into<String>,
        icon_id: u16,
        icon: Arc<RgbaImage>,
    ) -> Self {
        Self::from_image(message, label, Image::new(icon_id, icon))
    }

    pub fn new_with_generation(
        message: Message,
        label: impl Into<String>,
        icon_id: u16,
        icon: Arc<RgbaImage>,
        generation: u64,
    ) -> Self {
        Self::from_image(
            message,
            label,
            Image::new_with_generation(icon_id, icon, generation),
        )
    }

    fn from_image(message: Message, label: impl Into<String>, image: Image<Message>) -> Self {
        let label = label.into();
        Self {
            container: Container::new()
                .padding(Insets {
                    top: 8.0,
                    right: 6.0,
                    bottom: 4.0,
                    left: 6.0,
                })
                .message(message)
                .semantic_role(SemanticRole::Button)
                .accessibility_label(label.clone())
                .child(
                    Column::new()
                        .fill_width()
                        .align_items(Align::Center)
                        .gap(5.0)
                        .child(image.height(62.0).fit(ImageFit::Contain))
                        .child(
                            Container::new()
                                .height(36.0)
                                .fill_width()
                                .overflow_x(Overflow::Clip)
                                .overflow_y(Overflow::Clip)
                                .child(
                                    Text::new(label)
                                        .height(36.0)
                                        .wrap(true)
                                        .max_lines(2)
                                        .ellipsis(true)
                                        .align(TextAlign::Center)
                                        .fill_width(),
                                ),
                        ),
                ),
        }
    }

    fn content_mut(&mut self) -> Option<&mut Element<Message>> {
        self.container.0.children.first_mut()
    }

    fn label_box_mut(&mut self) -> Option<&mut Element<Message>> {
        self.content_mut()?.children.get_mut(1)
    }

    pub fn icon_size(mut self, size: f32) -> Self {
        if let Some(icon) = self
            .content_mut()
            .and_then(|content| content.children.first_mut())
        {
            icon.style.height = Length::Px(size.max(1.0));
        }
        self
    }

    pub fn label_height(mut self, height: f32) -> Self {
        if let Some(label) = self.label_box_mut() {
            label.style.height = Length::Px(height.max(1.0));
            if let Some(text) = label.children.first_mut() {
                text.style.height = Length::Px(height.max(1.0));
            }
        }
        self
    }

    pub fn label_scale(mut self, scale: f32) -> Self {
        if let Some(label) = self.label_box_mut()
            && let Some(text) = label.children.first_mut()
            && let Kind::Text { scale: value, .. } = &mut text.kind
        {
            *value = scale.max(0.1);
        }
        self
    }

    pub fn foreground(mut self, color: Color) -> Self {
        if let Some(label) = self.label_box_mut()
            && let Some(text) = label.children.first_mut()
        {
            text.style.foreground = Some(color);
        }
        self
    }

    pub fn label_background(mut self, color: Color, radius: f32) -> Self {
        if let Some(label) = self.label_box_mut() {
            label.style.background = Some(Background::Solid(color));
            label.style.corner_radius = radius.max(0.0);
        }
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        if let Some(content) = self.content_mut() {
            content.style.gap = gap.max(0.0);
        }
        self
    }

    pub fn padding(mut self, padding: Insets) -> Self {
        self.container = self.container.padding(padding);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.container = self.container.width(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.container = self.container.height(height);
        self
    }

    pub fn min_height(mut self, height: f32) -> Self {
        self.container = self.container.min_height(height);
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.container = self.container.radius(radius);
        self
    }

    pub fn position(mut self, position: Point) -> Self {
        self.container = self.container.position(position);
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.container = self.container.id(id);
        self
    }

    pub fn context_message(mut self, message: Message) -> Self {
        self.container = self.container.context_message(message);
        self
    }

    pub fn focus_background_tint(mut self, color: Color) -> Self {
        self.container = self.container.focus_background_tint(color);
        self
    }

    pub fn controller_focus_background_tint(mut self, color: Color) -> Self {
        self.container = self.container.controller_focus_background_tint(color);
        self
    }

    pub fn interaction_backgrounds(mut self, hover: Color, pressed: Color) -> Self {
        self.container = self.container.interaction_backgrounds(hover, pressed);
        self
    }

    pub fn selected_background(mut self, selected: bool, color: Color) -> Self {
        if selected {
            self.container = self.container.background(color);
        }
        self
    }

    pub fn hovered_background(self, hovered: bool, color: Color) -> Self {
        self.selected_background(hovered, color)
    }

    pub fn border(mut self, color: Color, width: f32) -> Self {
        self.container = self.container.border(color, width);
        self
    }

    pub fn semantic_role(mut self, role: SemanticRole) -> Self {
        self.container = self.container.semantic_role(role);
        self
    }

    pub fn accessibility_label(mut self, label: impl Into<String>) -> Self {
        self.container = self.container.accessibility_label(label);
        self
    }

    // Compatibility policy adapters for existing declarative file-grid call sites.
    pub fn colors(self, background: Color, border: Color, foreground: Color) -> Self {
        self.selected_background(true, background)
            .border(border, 1.0)
            .foreground(foreground)
    }

    pub fn borderless_colors(self, background: Color, foreground: Color) -> Self {
        self.selected_background(true, background)
            .foreground(foreground)
    }

    pub fn borderless_palette(self, colors: (Color, Color)) -> Self {
        self.borderless_colors(colors.0, colors.1)
    }

    pub fn file_grid_defaults(self) -> Self {
        self.padding(Insets {
            top: 8.0,
            right: 6.0,
            bottom: 4.0,
            left: 6.0,
        })
    }
}

impl<Message> Component<Message> for FilePlaneItem<Message> {
    fn into_element(self) -> Element<Message> {
        self.container.into_element()
    }
}

/// Compatibility name for the shared file-plane authority.
pub type FileGridItem<Message = String> = FilePlaneItem<Message>;

pub struct StyledText<Message = String>(Element<Message>);

impl<Message> StyledText<Message> {
    pub fn new(value: impl Into<String>, spans: Vec<StyledTextSpan>) -> Self {
        let value = value.into();
        Self(Element {
            kind: Kind::StyledText {
                value: value.clone(),
                spans,
                scale: 2.0,
                wrap: false,
                line_height: None,
            },
            id: None,
            source: None,
            style: Style {
                accessibility_label: Some(value),
                ..Style::default()
            },
            message: None,
            context_message: None,
            message_mapper: None,
            drag_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
            navigation_scope: None,
            adjustment_step: 0.05,
        })
    }

    pub fn inline_message(mut self, range: Range<usize>, message: Message) -> Self {
        self.0.inline_messages.push((range, message));
        self
    }

    pub fn scale(mut self, value: f32) -> Self {
        if let Kind::StyledText { scale, .. } = &mut self.0.kind {
            *scale = value;
        }
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.0 = self.0.foreground(color);
        self
    }

    pub fn wrap(mut self, enabled: bool) -> Self {
        if let Kind::StyledText { wrap, .. } = &mut self.0.kind {
            *wrap = enabled;
        }
        self
    }

    pub fn width_length(mut self, width: Length) -> Self {
        self.0 = self.0.width_length(width);
        self
    }

    pub fn selection_run_id(mut self, id: impl Into<String>) -> Self {
        self.0.style.selection_run_id = Some(id.into());
        self
    }

    pub fn selection_boundary(mut self, boundary: TextBoundary) -> Self {
        self.0.style.selection_boundary = boundary;
        self
    }
}

impl<Message> Component<Message> for StyledText<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

pub struct Text<Message = String>(Element<Message>);

impl<Message> Text<Message> {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        Self(Element::text(value.clone(), 2.0).accessibility_label(value))
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        if let Kind::Text {
            scale: text_scale, ..
        } = &mut self.0.kind
        {
            *text_scale = scale;
        }
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.0 = self.0.foreground(color);
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        if let Some(color) = tone.color() {
            self.0 = self.0.foreground(color);
        }
        self
    }

    pub fn bold(mut self, bold: bool) -> Self {
        if let Kind::Text {
            bold: text_bold, ..
        } = &mut self.0.kind
        {
            *text_bold = bold;
        }
        self
    }

    pub fn wrap(mut self, wrap: bool) -> Self {
        if let Kind::Text { wrap: value, .. } = &mut self.0.kind {
            *value = wrap;
        }
        self
    }

    pub fn line_height(mut self, height: f32) -> Self {
        if let Kind::Text { line_height, .. } = &mut self.0.kind {
            *line_height = Some(height.max(1.0));
        }
        self
    }

    pub fn max_lines(mut self, lines: usize) -> Self {
        if let Kind::Text { max_lines, .. } = &mut self.0.kind {
            *max_lines = Some(lines.max(1));
        }
        self
    }

    pub fn ellipsis(mut self, ellipsis: bool) -> Self {
        if let Kind::Text {
            ellipsis: value, ..
        } = &mut self.0.kind
        {
            *value = ellipsis;
        }
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.0 = self.0.width(width);
        self
    }

    pub fn width_length(mut self, width: Length) -> Self {
        self.0 = self.0.width_length(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.0 = self.0.height(height);
        self
    }

    pub fn align(mut self, align: TextAlign) -> Self {
        self.0 = self.0.text_align(align);
        self
    }

    pub fn selectable(mut self, selectable: bool) -> Self {
        self.0.style.selectable = Some(selectable);
        self
    }

    pub fn selection_run_id(mut self, id: impl Into<String>) -> Self {
        self.0.style.selection_run_id = Some(id.into());
        self
    }

    pub fn selection_boundary(mut self, boundary: TextBoundary) -> Self {
        self.0.style.selection_boundary = boundary;
        self
    }
}

impl<Message> Component<Message> for Text<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

pub struct SelectionRegion<Message = String>(Element<Message>);

impl<Message> SelectionRegion<Message> {
    pub fn new(document: impl Into<Arc<SelectionDocument>>) -> Self {
        let mut element = Element::flex(Axis::Vertical);
        element.style.selection_region = true;
        element.style.selection_document = Some(document.into());
        Self(element)
    }

    pub fn automatic() -> Self {
        let mut element = Element::flex(Axis::Vertical);
        element.style.selection_region = true;
        Self(element)
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn child(mut self, child: impl Component<Message>) -> Self {
        self.0 = self.0.child(child);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Component<Message>>) -> Self {
        self.0 = self.0.children(children);
        self
    }

    pub fn fill_width(mut self) -> Self {
        self.0 = self.0.fill_width();
        self
    }

    pub fn fill_height(mut self) -> Self {
        self.0 = self.0.fill_height();
        self
    }

    pub fn grow(mut self, grow: f32) -> Self {
        self.0 = self.0.grow(grow);
        self
    }
}

impl<Message> Component<Message> for SelectionRegion<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

pub struct Image<Message = String>(Element<Message>);

/// A bounded custom painter for exceptional visuals that cannot be expressed
/// by semantic primitives. The callback receives only its allocated rectangle;
/// commands outside it and clip-stack commands are discarded by resolution.
pub struct CustomPaint<Message = String>(Element<Message>);

impl<Message> CustomPaint<Message> {
    pub fn new(paint: fn(Rect) -> Vec<PaintCommand>) -> Self {
        Self(Element {
            id: None,
            source: None,
            kind: Kind::CustomPaint { paint },
            style: Style::default(),
            message: None,
            context_message: None,
            message_mapper: None,
            drag_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
            navigation_scope: None,
            adjustment_step: 0.05,
        })
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.0 = self.0.width(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.0 = self.0.height(height);
        self
    }

    pub fn message(mut self, message: Message) -> Self {
        self.0 = self.0.message(message);
        self
    }

    pub fn on_drag(mut self, (seed, map): (Message, fn(Message, DragGesture) -> Message)) -> Self {
        self.0 = self.0.on_drag(seed, map);
        self
    }

    pub fn context_message(mut self, message: Message) -> Self {
        self.0 = self.0.context_message(message);
        self
    }

    pub fn semantic_role(mut self, role: SemanticRole) -> Self {
        self.0 = self.0.semantic_role(role);
        self
    }

    pub fn accessibility_label(mut self, label: impl Into<String>) -> Self {
        self.0 = self.0.accessibility_label(label);
        self
    }

    pub fn accessibility_description(mut self, description: impl Into<String>) -> Self {
        self.0 = self.0.accessibility_description(description);
        self
    }

    pub fn accessibility_state(mut self, state: impl Into<String>) -> Self {
        self.0 = self.0.accessibility_state(state);
        self
    }
}

impl<Message> Component<Message> for CustomPaint<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

impl<Message> Image<Message> {
    pub fn new(id: u16, image: Arc<RgbaImage>) -> Self {
        use std::hash::{Hash, Hasher};
        #[cfg(debug_assertions)]
        let profile_started = std::time::Instant::now();
        let mut fingerprint = std::collections::hash_map::DefaultHasher::new();
        image.width().hash(&mut fingerprint);
        image.height().hash(&mut fingerprint);
        image.as_raw().hash(&mut fingerprint);
        let generation = fingerprint.finish();
        #[cfg(debug_assertions)]
        crate::gpu::record_image_fingerprint(image.as_raw().len(), profile_started.elapsed());
        Self::new_with_generation(id, image, generation)
    }

    /// Creates an image with a source-owned content generation.
    ///
    /// The caller must change `generation` whenever the pixels associated with
    /// `id` change. This avoids hashing the complete image on repeated builds.
    pub fn new_with_generation(id: u16, image: Arc<RgbaImage>, generation: u64) -> Self {
        Self(Element {
            id: None,
            source: None,
            kind: Kind::Image {
                id,
                generation,
                image,
                high_density: None,
                presentation: ImagePresentation::default(),
            },
            style: Style {
                overflow_x: Overflow::Clip,
                overflow_y: Overflow::Clip,
                ..Style::default()
            },
            message: None,
            context_message: None,
            message_mapper: None,
            drag_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
            navigation_scope: None,
            adjustment_step: 0.05,
        })
    }

    /// Supplies the source-owned content generation used by presenter caches.
    /// Increment it whenever pixels for the stable image id change.
    pub fn generation(mut self, generation: u64) -> Self {
        if let Kind::Image {
            generation: value, ..
        } = &mut self.0.kind
        {
            *value = generation;
        }
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.0 = self.0.width(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.0 = self.0.height(height);
        self
    }

    pub fn fit(mut self, fit: ImageFit) -> Self {
        if let Kind::Image { presentation, .. } = &mut self.0.kind {
            presentation.fit = fit;
        }
        self
    }

    /// Supplies a 2x raster selected by the renderer on high-density outputs.
    pub fn high_density(mut self, image: Arc<RgbaImage>) -> Self {
        if let Kind::Image { high_density, .. } = &mut self.0.kind {
            *high_density = Some(image);
        }
        self
    }

    pub fn alignment(mut self, horizontal: ImageAlignment, vertical: ImageAlignment) -> Self {
        if let Kind::Image { presentation, .. } = &mut self.0.kind {
            presentation.horizontal = horizontal;
            presentation.vertical = vertical;
        }
        self
    }

    pub fn presentation(mut self, value: ImagePresentation) -> Self {
        if let Kind::Image { presentation, .. } = &mut self.0.kind {
            *presentation = value;
        }
        self
    }

    /// Marks an image as purely visual so it is omitted from semantic and
    /// accessibility output while remaining in the paint list.
    pub fn decorative(mut self) -> Self {
        self.0 = self.0.decorative();
        self
    }
}

impl<Message> Component<Message> for Image<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

/// A semantic-tint icon with explicit accessible or decorative semantics.
pub struct Icon<Message = String>(Image<Message>);

impl<Message> Icon<Message> {
    pub fn new(id: u16, source: Arc<RgbaImage>, tint: Color, size: f32) -> Self {
        let red = ((tint >> 16) & 0xff) as u8;
        let green = ((tint >> 8) & 0xff) as u8;
        let blue = (tint & 0xff) as u8;
        let encoded_alpha = ((tint >> 24) & 0xff) as u8;
        let tint_alpha = if tint <= 0x00ff_ffff {
            255
        } else {
            encoded_alpha
        };
        let mut image = (*source).clone();
        for pixel in image.pixels_mut() {
            pixel.0 = [
                red,
                green,
                blue,
                ((u16::from(pixel[3]) * u16::from(tint_alpha)) / 255) as u8,
            ];
        }
        Self(
            Image::new(id, Arc::new(image))
                .width(size.max(1.0))
                .height(size.max(1.0)),
        )
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.0.0 = self
            .0
            .0
            .accessibility_hidden(false)
            .accessibility_label(label);
        self
    }

    pub fn decorative(mut self) -> Self {
        self.0.0 = self.0.0.accessibility_hidden(true);
        self
    }
}

impl<Message> Component<Message> for Icon<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

pub struct Header<Message = String>(Text<Message>);

impl<Message> Header<Message> {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Text::new(value).scale(4.0))
    }

    pub fn color(mut self, color: Color) -> Self {
        self.0 = self.0.color(color);
        self
    }
}

pub struct TextField<Message = String> {
    text: Text<Message>,
    displayed: String,
    single_line_height: Option<f32>,
}

impl<Message> TextField<Message> {
    pub fn new(editor: &TextEditor) -> Self {
        let displayed = editor.display_text_with_caret("▏");
        Self {
            text: Text::new(&displayed),
            displayed,
            single_line_height: None,
        }
    }

    pub fn placeholder(editor: &TextEditor, placeholder: impl Into<String>) -> Self {
        if editor.text().is_empty() && editor.preedit().is_empty() {
            let displayed = placeholder.into();
            Self {
                text: Text::new(&displayed),
                displayed,
                single_line_height: None,
            }
        } else {
            Self::new(editor)
        }
    }

    pub fn on_change(value: &str, map: fn(String) -> Message) -> Self {
        let mut field = Self {
            text: Text::new(value),
            displayed: value.to_owned(),
            single_line_height: None,
        };
        if let Kind::Text { input_value, .. } = &mut field.text.0.kind {
            *input_value = Some(value.to_owned());
        }
        field.text.0.text_mapper = Some(map);
        field
    }

    pub fn on_change_with_placeholder(
        value: &str,
        placeholder: impl Into<String>,
        map: fn(String) -> Message,
    ) -> Self {
        let displayed = if value.is_empty() {
            placeholder.into()
        } else {
            value.to_owned()
        };
        let mut field = Self {
            text: Text::new(&displayed),
            displayed,
            single_line_height: None,
        };
        if let Kind::Text { input_value, .. } = &mut field.text.0.kind {
            *input_value = Some(value.to_owned());
        }
        field.text.0.text_mapper = Some(map);
        field
    }

    /// Creates an editable field whose painted value is masked while edits are
    /// still applied to the unmasked application-owned value.
    pub fn on_change_masked(value: &str, mask: char, map: fn(String) -> Message) -> Self {
        let displayed = std::iter::repeat_n(mask, value.chars().count()).collect::<String>();
        let mut field = Self {
            text: Text::new(&displayed),
            displayed,
            single_line_height: None,
        };
        if let Kind::Text {
            input_value,
            input_mask,
            ..
        } = &mut field.text.0.kind
        {
            *input_value = Some(value.to_owned());
            *input_mask = Some(mask);
        }
        field.text.0.text_mapper = Some(map);
        field
    }

    pub fn on_change_masked_with_placeholder(
        value: &str,
        placeholder: impl Into<String>,
        mask: char,
        map: fn(String) -> Message,
    ) -> Self {
        let displayed = if value.is_empty() {
            placeholder.into()
        } else {
            std::iter::repeat_n(mask, value.chars().count()).collect()
        };
        let mut field = Self {
            text: Text::new(&displayed),
            displayed,
            single_line_height: None,
        };
        if let Kind::Text {
            input_value,
            input_mask,
            ..
        } = &mut field.text.0.kind
        {
            *input_value = Some(value.to_owned());
            *input_mask = Some(mask);
        }
        field.text.0.text_mapper = Some(map);
        field
    }

    pub fn display_text(&self) -> &str {
        &self.displayed
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.text = self.text.id(id);
        self
    }

    pub fn accessibility_label(mut self, label: impl Into<String>) -> Self {
        self.text.0 = self.text.0.accessibility_label(label);
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.text = self.text.scale(scale);
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.text = self.text.color(color);
        self
    }

    pub fn wrap(mut self, wrap: bool) -> Self {
        self.text = self.text.wrap(wrap);
        self
    }

    pub fn grow(mut self, grow: f32) -> Self {
        self.text.0 = self.text.0.grow(grow);
        self
    }

    /// Keeps a single-line editor vertically centered in a taller field.
    ///
    /// The wrapper uses the measured text line rather than a font-specific
    /// pixel offset, so placeholder, masked, selected, caret, and IME paint
    /// all share the same origin. Multiline fields remain unaffected unless
    /// they explicitly opt into this single-line presentation.
    pub fn single_line_height(mut self, height: f32) -> Self {
        self.text = self.text.wrap(false).max_lines(1);
        self.single_line_height = Some(height.max(0.0));
        self
    }
}

impl<Message> Component<Message> for TextField<Message> {
    fn into_element(self) -> Element<Message> {
        let text = self
            .text
            .into_element()
            .semantic_role(SemanticRole::TextField);
        match self.single_line_height {
            Some(height) => Element::flex(Axis::Vertical)
                .height(height)
                .justify_content(Justify::Center)
                .child(text),
            None => text,
        }
    }
}

impl<Message> Component<Message> for Header<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

pub struct Container<Message = String>(Element<Message>);

impl<Message> Container<Message> {
    pub fn new() -> Self {
        Self(Element::flex(Axis::Vertical))
    }

    pub fn child(mut self, child: impl Component<Message>) -> Self {
        self.0 = self.0.child(child);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Component<Message>>) -> Self {
        self.0 = self.0.children(children);
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.0 = self.0.background(background);
        self
    }

    pub fn interaction_backgrounds(
        mut self,
        hover: impl Into<Background>,
        pressed: impl Into<Background>,
    ) -> Self {
        self.0 = self.0.interaction_backgrounds(hover, pressed);
        self
    }

    pub fn hover_background(mut self, background: impl Into<Background>) -> Self {
        self.0.style.hover_background = Some(background.into());
        self
    }

    pub fn pressed_background(mut self, background: impl Into<Background>) -> Self {
        self.0.style.pressed_background = Some(background.into());
        self
    }

    pub fn focus_background_tint(mut self, color: Color) -> Self {
        self.0 = self.0.focus_background_tint(color);
        self
    }

    pub fn controller_focus_background_tint(mut self, color: Color) -> Self {
        self.0 = self.0.controller_focus_background_tint(color);
        self
    }

    pub fn navigation_scope(mut self, scope: crate::NavigationScope) -> Self {
        self.0 = self.0.navigation_scope(scope);
        self
    }

    pub fn controller_scope_background(mut self, background: impl Into<Background>) -> Self {
        self.0 = self.0.controller_scope_background(background);
        self
    }

    pub fn border(mut self, color: Color, width: f32) -> Self {
        self.0 = self.0.border(color, width);
        self
    }

    pub fn border_value(mut self, border: impl Into<Border>) -> Self {
        self.0 = self.0.border_value(border);
        self
    }

    pub fn top_corner_radius(mut self, radius: f32) -> Self {
        self.0 = self.0.top_corner_radius(radius);
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.0 = self.0.radius(radius);
        self
    }

    pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
        self.0 = self.0.padding(padding);
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.0 = self.0.gap(gap);
        self
    }

    pub fn position(mut self, position: Point) -> Self {
        self.0 = self.0.position(position);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.0 = self.0.width(width);
        self
    }

    pub fn width_length(mut self, width: Length) -> Self {
        self.0 = self.0.width_length(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.0 = self.0.height(height);
        self
    }

    pub fn height_length(mut self, height: Length) -> Self {
        self.0 = self.0.height_length(height);
        self
    }

    pub fn grow(mut self, grow: f32) -> Self {
        self.0 = self.0.grow(grow);
        self
    }

    pub fn shrink(mut self, shrink: f32) -> Self {
        self.0 = self.0.shrink(shrink);
        self
    }

    pub fn min_width(mut self, width: f32) -> Self {
        self.0 = self.0.min_width(width);
        self
    }

    pub fn max_width(mut self, width: f32) -> Self {
        self.0 = self.0.max_width(width);
        self
    }

    pub fn min_height(mut self, height: f32) -> Self {
        self.0 = self.0.min_height(height);
        self
    }

    pub fn max_height(mut self, height: f32) -> Self {
        self.0 = self.0.max_height(height);
        self
    }

    pub fn fill_width(mut self) -> Self {
        self.0 = self.0.fill_width();
        self
    }

    pub fn fill_height(mut self) -> Self {
        self.0 = self.0.fill_height();
        self
    }

    pub fn basis(mut self, basis: impl Into<Length>) -> Self {
        self.0 = self.0.basis(basis);
        self
    }

    pub fn align_self(mut self, align: Align) -> Self {
        self.0 = self.0.align_self(align);
        self
    }

    pub fn align_items(mut self, align: Align) -> Self {
        self.0 = self.0.align_items(align);
        self
    }

    pub fn justify_content(mut self, justify: Justify) -> Self {
        self.0 = self.0.justify_content(justify);
        self
    }

    pub fn overflow(mut self, x: Overflow, y: Overflow) -> Self {
        self.0 = self.0.overflow(x, y);
        self
    }

    pub fn overflow_x(mut self, overflow: Overflow) -> Self {
        self.0 = self.0.overflow_x(overflow);
        self
    }

    pub fn overflow_y(mut self, overflow: Overflow) -> Self {
        self.0 = self.0.overflow_y(overflow);
        self
    }

    pub fn scrollbar_theme(mut self, theme: crate::SemanticTheme) -> Self {
        self.0 = self.0.scrollbar_theme(theme);
        self
    }

    pub fn message(mut self, message: Message) -> Self {
        self.0 = self.0.message(message);
        self
    }

    pub fn on_drag(mut self, (seed, map): (Message, fn(Message, DragGesture) -> Message)) -> Self {
        self.0 = self.0.on_drag(seed, map);
        self
    }

    pub fn context_message(mut self, message: Message) -> Self {
        self.0 = self.0.context_message(message);
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        if !enabled {
            self.0.message = None;
        }
        self
    }

    pub fn accessibility_label(mut self, label: impl Into<String>) -> Self {
        self.0 = self.0.accessibility_label(label);
        self
    }

    pub fn semantic_role(mut self, role: SemanticRole) -> Self {
        self.0 = self.0.semantic_role(role);
        self
    }

    pub fn accessibility_description(mut self, description: impl Into<String>) -> Self {
        self.0 = self.0.accessibility_description(description);
        self
    }

    pub fn accessibility_state(mut self, state: impl Into<String>) -> Self {
        self.0 = self.0.accessibility_state(state);
        self
    }
}

impl<Message> Default for Container<Message> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Message> Component<Message> for Container<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

pub struct Sidebar<Message = String>(Column<Message>);

impl<Message> Sidebar<Message> {
    pub fn new(width: f32) -> Self {
        Self(Column::new().width(width).shrink(0.0))
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.0 = self.0.background(background);
        self
    }

    pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
        self.0 = self.0.padding(padding);
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.0 = self.0.gap(gap);
        self
    }

    pub fn min_width(mut self, width: f32) -> Self {
        self.0 = self.0.min_width(width);
        self
    }

    pub fn max_width(mut self, width: f32) -> Self {
        self.0 = self.0.max_width(width);
        self
    }

    pub fn child(mut self, child: impl Component<Message>) -> Self {
        self.0 = self.0.child(child);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Component<Message>>) -> Self {
        self.0 = self.0.children(children);
        self
    }
}

impl<Message> Component<Message> for Sidebar<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

/// A thematic break with configurable space above and below its one-pixel line.
pub struct HorizontalRule<Message = String>(Container<Message>);

impl<Message> HorizontalRule<Message> {
    pub fn new(color: Color) -> Self {
        Self(
            Container::new()
                .height(17.0)
                .padding(Insets {
                    top: 8.0,
                    right: 0.0,
                    bottom: 8.0,
                    left: 0.0,
                })
                .child(Container::new().height(1.0).background(color)),
        )
    }

    pub fn spacing(mut self, top: f32, bottom: f32) -> Self {
        self.0 = Container(self.0.0.height(top + 1.0 + bottom).padding(Insets {
            top,
            right: 0.0,
            bottom,
            left: 0.0,
        }));
        self
    }

    pub fn spacing_pair(self, spacing: (f32, f32)) -> Self {
        self.spacing(spacing.0, spacing.1)
    }
}

impl<Message> Component<Message> for HorizontalRule<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

pub struct SidebarSection<Message = String>(Column<Message>);

impl<Message> SidebarSection<Message> {
    pub fn new(label: impl Into<String>, color: Color) -> Self {
        Self(
            Column::new()
                .gap(3.0)
                .child(Text::new(label).height(26.0).scale(0.95).color(color)),
        )
    }

    pub fn child(mut self, child: impl Component<Message>) -> Self {
        self.0 = self.0.child(child);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Component<Message>>) -> Self {
        self.0 = self.0.children(children);
        self
    }
}

impl<Message> Component<Message> for SidebarSection<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

pub struct SidebarItem<Message = String>(Container<Message>);

impl<Message> SidebarItem<Message> {
    pub fn new(message: Message, label: impl Into<String>, foreground: Color) -> Self {
        Self(
            Container::new()
                .height(36.0)
                .message(message)
                .padding(Insets {
                    top: 8.0,
                    right: 8.0,
                    bottom: 6.0,
                    left: 10.0,
                })
                .child(Text::new(label).scale(1.05).color(foreground)),
        )
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.0 = self.0.background(background);
        self
    }

    pub fn indent(mut self, depth: usize) -> Self {
        self.0.0.style.padding.left += depth as f32 * 16.0;
        self
    }
}

impl<Message> Component<Message> for SidebarItem<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

pub struct SidebarFolder<Message = String>(Container<Message>);

impl<Message> SidebarFolder<Message> {
    pub fn new(
        toggle_message: Message,
        open_message: Message,
        label: impl Into<String>,
        expanded: bool,
        foreground: Color,
    ) -> Self {
        let label = label.into();
        let toggle_label = format!("Toggle {label}");
        Self(
            Container::new().height(36.0).child(
                Row::new()
                    .child(
                        Container::new()
                            .width(28.0)
                            .height(36.0)
                            .message(toggle_message)
                            .accessibility_label(toggle_label)
                            .padding(Insets {
                                top: 8.0,
                                right: 3.0,
                                bottom: 6.0,
                                left: 8.0,
                            })
                            .child(
                                Text::new(if expanded { "⌄" } else { "›" })
                                    .scale(1.05)
                                    .color(foreground),
                            ),
                    )
                    .child(
                        Container::new()
                            .grow(1.0)
                            .height(36.0)
                            .message(open_message)
                            .accessibility_label(label.clone())
                            .padding(Insets {
                                top: 8.0,
                                right: 8.0,
                                bottom: 6.0,
                                left: 2.0,
                            })
                            .child(Text::new(label).scale(1.05).color(foreground)),
                    ),
            ),
        )
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.0 = self.0.background(background);
        self
    }

    /// Assigns a stable semantic identity to the folder-opening action. This
    /// lets native drag adapters resolve a drop destination without coupling
    /// provider paths to painted row geometry.
    pub fn open_id(mut self, id: impl Into<UiId>) -> Self {
        if let Some(open) = self
            .0
            .0
            .children
            .first_mut()
            .and_then(|row| row.children.get_mut(1))
        {
            open.id = Some(id.into());
        }
        self
    }

    pub fn indent(mut self, depth: usize) -> Self {
        self.0.0.style.padding.left = depth as f32 * 16.0;
        self
    }

    /// Applies modality-specific focus rings to both semantic actions in the
    /// folder row without turning the noninteractive wrapper into a target.
    pub fn focus_background_tints(mut self, colors: (Color, Color)) -> Self {
        if let Some(row) = self.0.0.children.first_mut() {
            for action in &mut row.children {
                action.style.focus_background_tint = Some(colors.0);
                action.style.controller_focus_background_tint = Some(colors.1);
            }
        }
        self
    }

    pub fn accessibility_labels<T: Into<String>, O: Into<String>>(
        mut self,
        labels: (T, O),
    ) -> Self {
        let (toggle, open) = labels;
        if let Some(row) = self.0.0.children.first_mut() {
            if let Some(action) = row.children.first_mut() {
                action.style.accessibility_label = Some(toggle.into());
            }
            if let Some(action) = row.children.get_mut(1) {
                action.style.accessibility_label = Some(open.into());
            }
        }
        self
    }

    /// Adds provider-resolved artwork to the open action while preserving the
    /// disclosure action as an independent semantic target.
    pub fn artwork(
        mut self,
        asset_id: u16,
        image: std::sync::Arc<image::RgbaImage>,
        generation: u64,
    ) -> Self {
        if let Some(open) = self
            .0
            .0
            .children
            .first_mut()
            .and_then(|row| row.children.get_mut(1))
        {
            let label = open.children.pop();
            let mut row = Row::new().gap(7.0).child(
                Image::new_with_generation(asset_id, image, generation)
                    .fit(ImageFit::Contain)
                    .width(18.0)
                    .height(18.0),
            );
            if let Some(label) = label {
                row = row.child(label);
            }
            open.children.push(row.into_element());
        }
        self
    }
}

impl<Message> Component<Message> for SidebarFolder<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

pub struct ContentPane<Message = String>(Container<Message>);

impl<Message> ContentPane<Message> {
    pub fn new(content: impl Component<Message>) -> Self {
        Self(Container::new().grow(1.0).child(content))
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.0 = self.0.background(background);
        self
    }

    pub fn min_width(mut self, width: f32) -> Self {
        self.0 = self.0.min_width(width);
        self
    }

    pub fn fill_width(mut self) -> Self {
        self.0 = self.0.fill_width();
        self
    }
}

impl<Message> Component<Message> for ContentPane<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

pub struct ShoulderHints<Message = String>(Row<Message>);

impl<Message> ShoulderHints<Message> {
    pub fn new(color: Color, muted: Color) -> Self {
        fn keycap<Message>(label: &str, color: Color, muted: Color) -> Container<Message> {
            Container::new()
                .width(34.0)
                .height(24.0)
                .border(muted, 1.0)
                .padding(Insets {
                    top: 2.0,
                    right: 5.0,
                    bottom: 2.0,
                    left: 5.0,
                })
                .child(
                    Text::new(label)
                        .scale(1.0)
                        .color(color)
                        .align(TextAlign::Center),
                )
        }
        Self(
            Row::new()
                .gap(8.0)
                .child(keycap("LB", color, muted))
                .child(keycap("RB", color, muted)),
        )
    }
}

impl<Message> Component<Message> for ShoulderHints<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

pub struct Button<Message = String>(Container<Message>);

/// The semantic visual role of a button.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ButtonPresentation {
    Primary,
    Secondary,
    Quiet,
    Destructive,
    Disabled,
}

impl<Message> Button<Message> {
    pub fn new(message: Message, label: impl Into<String>) -> Self {
        let label = label.into();
        let mut button = Self::with_label(message, ButtonLabel::new(&label));
        button.0 = button.0.accessibility_label(label);
        button
    }

    /// Creates a button whose appearance is resolved entirely from semantic
    /// theme roles. A disabled presentation deliberately has no activation
    /// message.
    pub fn semantic(
        theme: SemanticTheme,
        message: Message,
        label: impl Into<String>,
        presentation: ButtonPresentation,
    ) -> Self {
        Self::new(message, label).presentation(theme, presentation)
    }

    /// Applies a semantic presentation without changing the existing button
    /// construction API.
    pub fn presentation(mut self, theme: SemanticTheme, presentation: ButtonPresentation) -> Self {
        let (background, border, foreground) = match presentation {
            ButtonPresentation::Primary => (
                Some(theme.accent.ordinary),
                theme.accent.ordinary,
                theme.accent.on_accent,
            ),
            ButtonPresentation::Secondary => (
                Some(theme.surfaces.raised),
                theme.borders.ordinary,
                theme.text.primary,
            ),
            ButtonPresentation::Quiet => (None, theme.borders.subtle, theme.text.primary),
            ButtonPresentation::Destructive => (
                Some(theme.text.danger),
                theme.text.danger,
                theme.accent.on_accent,
            ),
            ButtonPresentation::Disabled => (
                Some(theme.surfaces.raised),
                theme.borders.subtle,
                theme.text.disabled,
            ),
        };
        self.0.0.style.background = background.map(Background::Solid);
        self.0.0.style.border = (presentation != ButtonPresentation::Quiet).then_some(border);
        self.0.0.style.border_width = theme.sizing.border;
        self.0.0.style.corner_radius = theme.radii.control;
        self.0.0.style.height = Length::Px(theme.sizing.control_height);
        self.0.0.style.padding = Insets {
            top: 8.0,
            right: 12.0,
            bottom: 7.0,
            left: 12.0,
        };
        self.0.0.style.hover_background = Some(Background::Solid(theme.surfaces.hover));
        self.0.0.style.pressed_background = Some(Background::Solid(theme.surfaces.pressed));
        self.0.0.style.focus_background_tint = Some(theme.borders.focus);
        self.0.0.style.controller_focus_background_tint = Some(theme.borders.controller_focus);
        if presentation == ButtonPresentation::Disabled {
            self.0.0.message = None;
        }
        if let Some(label) = self.0.0.children.first_mut() {
            label.style.foreground = Some(foreground);
        }
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn focus_background_tint(mut self, color: Color) -> Self {
        self.0 = self.0.focus_background_tint(color);
        self
    }

    pub fn controller_focus_background_tint(mut self, color: Color) -> Self {
        self.0 = self.0.controller_focus_background_tint(color);
        self
    }

    pub fn with_label(message: Message, label: ButtonLabel<Message>) -> Self {
        Self(
            Container::new()
                .padding(Insets {
                    top: 11.0,
                    right: 12.0,
                    bottom: 8.0,
                    left: 12.0,
                })
                .height(42.0)
                .semantic_role(SemanticRole::Button)
                .message(message)
                .child(label),
        )
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.0 = self.0.background(background);
        self
    }

    pub fn border(mut self, color: Color, width: f32) -> Self {
        self.0 = self.0.border(color, width);
        self
    }

    pub fn border_value(mut self, border: impl Into<Border>) -> Self {
        self.0 = self.0.border_value(border);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.0 = self.0.width(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.0 = self.0.height(height);
        self
    }

    /// Keeps a button visible while removing its activation route when unavailable.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.0 = self.0.enabled(enabled);
        self
    }

    /// Allows a button label to wrap while keeping the ordinary one-line
    /// button height as its minimum.
    pub fn max_lines(mut self, lines: usize) -> Self {
        self.0 = self.0.height_length(Length::Auto).min_height(42.0);
        if let Some(label) = self.0.0.children.first_mut()
            && let Kind::Text {
                wrap,
                max_lines,
                ellipsis,
                ..
            } = &mut label.kind
        {
            *wrap = true;
            *max_lines = Some(lines.max(1));
            *ellipsis = false;
        }
        self
    }

    pub fn label_align(mut self, align: TextAlign) -> Self {
        if let Some(label) = self.0.0.children.first_mut()
            && let Kind::Text { .. } = label.kind
        {
            label.style.text_align = align;
        }
        self
    }

    /// Keeps a button label on one line and truncates it to the available width.
    pub fn ellipsis(mut self, ellipsis: bool) -> Self {
        if let Some(label) = self.0.0.children.first_mut()
            && let Kind::Text {
                wrap,
                max_lines,
                ellipsis: value,
                ..
            } = &mut label.kind
        {
            *wrap = !ellipsis;
            *max_lines = ellipsis.then_some(1);
            *value = ellipsis;
        }
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.0 = self.0.radius(radius);
        self
    }

    pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
        self.0 = self.0.padding(padding);
        self
    }

    pub fn min_width(mut self, width: f32) -> Self {
        self.0 = self.0.min_width(width);
        self
    }

    pub fn max_width(mut self, width: f32) -> Self {
        self.0 = self.0.max_width(width);
        self
    }

    pub fn shrink(mut self, shrink: f32) -> Self {
        self.0 = self.0.shrink(shrink);
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        if let Some(label) = self.0.0.children.first_mut() {
            label.style.foreground = Some(color);
        }
        self
    }
}

impl<Message> Component<Message> for Button<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

pub struct ButtonLabel<Message = String>(Text<Message>);

impl<Message> ButtonLabel<Message> {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Text::new(value).wrap(false).align(TextAlign::Center))
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.0 = self.0.scale(scale);
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.0 = self.0.color(color);
        self
    }

    pub fn align(mut self, align: TextAlign) -> Self {
        self.0 = self.0.align(align);
        self
    }

    pub fn max_lines(mut self, lines: usize) -> Self {
        self.0 = self.0.wrap(true).max_lines(lines);
        self
    }
}

impl<Message> Component<Message> for ButtonLabel<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

pub struct RadioButton<Message = String>(Container<Message>);

/// A full-row, single-selection option suitable for devices and settings lists.
pub struct RadioOption<Message = String> {
    theme: SemanticTheme,
    message: Message,
    label: String,
    description: Option<String>,
    selected: bool,
    enabled: bool,
    id: Option<UiId>,
    leading: Option<Element<Message>>,
    trailing: Option<Element<Message>>,
}

impl<Message> RadioOption<Message> {
    pub fn new(
        theme: SemanticTheme,
        message: Message,
        label: impl Into<String>,
        selected: bool,
    ) -> Self {
        Self {
            theme,
            message,
            label: label.into(),
            description: None,
            selected,
            enabled: true,
            id: None,
            leading: None,
            trailing: None,
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn leading(mut self, leading: impl Component<Message>) -> Self {
        self.leading = Some(leading.into_element());
        self
    }

    pub fn trailing(mut self, trailing: impl Component<Message>) -> Self {
        self.trailing = Some(trailing.into_element());
        self
    }
}

impl<Message> Component<Message> for RadioOption<Message> {
    fn into_element(self) -> Element<Message> {
        let text_color = if self.enabled {
            self.theme.text.primary
        } else {
            self.theme.text.disabled
        };
        let mut labels = Column::new()
            .gap(self.theme.spacing.compact)
            .grow(1.0)
            .child(Text::new(self.label.clone()).color(text_color).wrap(true));
        if let Some(description) = self.description.as_ref() {
            labels = labels.child(
                Text::new(description)
                    .scale(0.9)
                    .color(if self.enabled {
                        self.theme.text.secondary
                    } else {
                        self.theme.text.disabled
                    })
                    .wrap(true),
            );
        }
        let mut row = Row::new()
            .gap(self.theme.spacing.control)
            .align_items(Align::Center)
            .child(SelectionIndicator::semantic(self.theme, self.selected));
        if let Some(leading) = self.leading {
            row = row.child(leading);
        }
        row = row.child(labels);
        if let Some(trailing) = self.trailing {
            row = row.child(trailing);
        }
        let state = match (self.enabled, self.selected) {
            (false, _) => "disabled",
            (true, true) => "selected",
            (true, false) => "unselected",
        };
        let mut option = Container::new()
            .min_height(58.0)
            .fill_width()
            .padding(Insets::all(self.theme.spacing.content))
            .radius(self.theme.radii.control)
            .background(self.theme.surfaces.card)
            .border(
                if self.selected {
                    self.theme.accent.ordinary
                } else {
                    self.theme.borders.subtle
                },
                if self.selected { 2.0 } else { 1.0 },
            )
            .interaction_backgrounds(self.theme.surfaces.hover, self.theme.surfaces.pressed)
            .focus_background_tint(self.theme.borders.focus)
            .controller_focus_background_tint(self.theme.borders.controller_focus)
            .message(self.message)
            .enabled(self.enabled)
            .semantic_role(SemanticRole::Radio)
            .accessibility_label(self.label)
            .accessibility_description(self.description.unwrap_or_default())
            .accessibility_state(state)
            .child(row);
        if let Some(id) = self.id {
            option = option.id(id);
        }
        option.into_element()
    }
}

/// A semantic group of full-row radio options.
pub struct RadioGroup<Message = String>(Container<Message>);

impl<Message> RadioGroup<Message> {
    pub fn new(options: impl IntoIterator<Item = RadioOption<Message>>) -> Self {
        Self(
            Container::new()
                .fill_width()
                .semantic_role(SemanticRole::RadioGroup)
                .navigation_scope(crate::NavigationScope::group())
                .child(Column::new().fill_width().gap(10.0).children(options)),
        )
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }
}

impl<Message> Component<Message> for RadioGroup<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

/// A renderer-owned radio/selection mark that never depends on a font glyph.
pub struct SelectionIndicator<Message = String>(Container<Message>);

impl<Message> SelectionIndicator<Message> {
    pub fn new(selected: bool, indicator: Color, background: Color) -> Self {
        let mut inner = Container::new()
            .width(14.0)
            .height(14.0)
            .radius(7.0)
            .background(background);
        if selected {
            inner = inner.padding(3.0).child(
                Container::new()
                    .width(8.0)
                    .height(8.0)
                    .radius(4.0)
                    .background(indicator),
            );
        }
        Self(
            Container::new()
                .width(18.0)
                .height(18.0)
                .radius(9.0)
                .background(indicator)
                .padding(2.0)
                .child(inner),
        )
    }

    pub fn semantic(theme: SemanticTheme, selected: bool) -> Self {
        Self::new(
            selected,
            if selected {
                theme.accent.ordinary
            } else {
                theme.borders.strong
            },
            theme.surfaces.card,
        )
    }
}

impl<Message> Component<Message> for SelectionIndicator<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

impl<Message> RadioButton<Message> {
    pub fn new(message: Message, label: impl Into<String>, selected: bool) -> Self {
        Self::with_colors(
            message,
            label,
            selected,
            if selected { 0x68b8ff } else { 0x8792a8 },
            0xf4f7ff,
            0x10151e,
            0x202936,
            0x293545,
            0x68b8ff,
            0x63d69a,
        )
    }

    pub fn semantic(
        theme: SemanticTheme,
        message: Message,
        label: impl Into<String>,
        selected: bool,
    ) -> Self {
        Self::with_colors(
            message,
            label,
            selected,
            if selected {
                theme.accent.ordinary
            } else {
                theme.borders.strong
            },
            theme.text.primary,
            theme.surfaces.card,
            theme.surfaces.hover,
            theme.surfaces.pressed,
            theme.borders.focus,
            theme.borders.controller_focus,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn with_colors(
        message: Message,
        label: impl Into<String>,
        selected: bool,
        indicator: Color,
        label_color: Color,
        background: Color,
        hover: Color,
        pressed: Color,
        focus: Color,
        controller_focus: Color,
    ) -> Self {
        Self(
            Container::new()
                .height(34.0)
                .message(message)
                .interaction_backgrounds(hover, pressed)
                .focus_background_tint(focus)
                .controller_focus_background_tint(controller_focus)
                .child(
                    Row::new()
                        .gap(10.0)
                        .align_items(Align::Center)
                        .child(SelectionIndicator::new(selected, indicator, background))
                        .child(Text::new(label).scale(1.15).color(label_color)),
                ),
        )
    }

    pub fn width(mut self, width: f32) -> Self {
        self.0 = self.0.width(width);
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn colors(mut self, indicator: Color, label: Color) -> Self {
        if let Some(row) = self.0.0.children.first_mut() {
            if let Some(indicator_element) = row.children.first_mut() {
                indicator_element.style.background = Some(Background::Solid(indicator));
                if let Some(inner) = indicator_element.children.first_mut()
                    && let Some(dot) = inner.children.first_mut()
                {
                    dot.style.background = Some(Background::Solid(indicator));
                }
            }
            if let Some(label_text) = row.children.get_mut(1) {
                label_text.style.foreground = Some(label);
            }
        }
        self
    }

    pub fn colors_pair(self, colors: (Color, Color)) -> Self {
        self.colors(colors.0, colors.1)
    }
}

impl<Message> Component<Message> for RadioButton<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

pub struct Slider<Message = String>(Element<Message>);

impl<Message> Slider<Message> {
    pub fn new(message: Message, value: f32) -> Self {
        let mut element = Element {
            id: None,
            source: None,
            kind: Kind::Slider {
                value: value.clamp(0.0, 1.0),
                track: 0x354158,
                fill: 0x68b8ff,
                thumb: 0xf4f7ff,
            },
            style: Style::default(),
            message: Some(message),
            context_message: None,
            message_mapper: None,
            drag_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
            navigation_scope: None,
            adjustment_step: 0.05,
        };
        element.style.height = Length::Px(24.0);
        element.style.semantic_role = Some(SemanticRole::Slider);
        Self(element)
    }

    pub fn on_change(map: fn(f32) -> Message, value: f32) -> Self {
        let mut slider = Self::new(map(value.clamp(0.0, 1.0)), value);
        slider.0.message_mapper = Some(map);
        slider
    }

    pub fn colors(mut self, track: Color, fill: Color, thumb: Color) -> Self {
        if let Kind::Slider {
            track: slider_track,
            fill: slider_fill,
            thumb: slider_thumb,
            ..
        } = &mut self.0.kind
        {
            *slider_track = track;
            *slider_fill = fill;
            *slider_thumb = thumb;
        }
        self
    }

    pub fn colors_triplet(self, colors: (Color, Color, Color)) -> Self {
        self.colors(colors.0, colors.1, colors.2)
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.0 = self.0.width(width);
        self
    }

    pub fn adjustment_step(mut self, step: f32) -> Self {
        self.0 = self.0.adjustment_step(step);
        self
    }

    pub fn focus_background_tint(mut self, color: Color) -> Self {
        self.0 = self.0.focus_background_tint(color);
        self
    }

    pub fn controller_focus_background_tint(mut self, color: Color) -> Self {
        self.0 = self.0.controller_focus_background_tint(color);
        self
    }
}

impl<Message> Component<Message> for Slider<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

pub struct Dropdown<Message = String>(Element<Message>);

impl<Message> Dropdown<Message> {
    pub fn new(
        toggle_message: Message,
        selected: impl Into<String>,
        options: impl IntoIterator<Item = (impl Into<String>, Message)>,
    ) -> Self {
        let (options, option_messages): (Vec<_>, Vec<_>) = options
            .into_iter()
            .map(|(label, message)| (label.into(), Some(message)))
            .unzip();
        let mut element = Element {
            id: None,
            source: None,
            kind: Kind::Dropdown {
                selected: selected.into(),
                options,
                expanded: false,
                open_generation: 0,
                overlay: false,
                background: 0x27344c,
                option_background: 0x34445f,
                foreground: 0xf4f7ff,
            },
            style: Style::default(),
            message: Some(toggle_message),
            context_message: None,
            message_mapper: None,
            drag_mapper: None,
            text_mapper: None,
            option_messages,
            inline_messages: Vec::new(),
            children: Vec::new(),
            navigation_scope: Some(
                crate::NavigationScope::group().traversal(crate::NavigationTraversal::Vertical),
            ),
            adjustment_step: 0.05,
        };
        element.style.height = Length::Px(42.0);
        Self(element)
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn accessibility_label(mut self, label: impl Into<String>) -> Self {
        self.0.style.accessibility_label = Some(label.into());
        self
    }

    pub fn semantic_role(mut self, role: SemanticRole) -> Self {
        self.0.style.semantic_role = Some(role);
        self
    }

    /// Paint options in the transient overlay layer instead of growing the
    /// surrounding layout when the choice list opens.
    pub fn overlay(mut self, overlay: bool) -> Self {
        if let Kind::Dropdown {
            overlay: is_overlay,
            ..
        } = &mut self.0.kind
        {
            *is_overlay = overlay;
            self.0.style.height = Length::Px(if overlay { 30.0 } else { 42.0 });
        }
        self
    }

    /// Requests one opening transition for a new application-owned generation.
    pub fn open_generation(mut self, generation: u64) -> Self {
        if let Kind::Dropdown {
            open_generation, ..
        } = &mut self.0.kind
        {
            *open_generation = generation;
        }
        self
    }

    pub fn expanded(mut self, expanded: bool) -> Self {
        if let Kind::Dropdown {
            expanded: is_expanded,
            options,
            ..
        } = &mut self.0.kind
        {
            *is_expanded = expanded;
            self.0.style.height = Length::Px(
                42.0 + if expanded {
                    options.len() as f32 * 36.0
                } else {
                    0.0
                },
            );
        }
        self
    }

    pub fn colors(
        mut self,
        background: Color,
        option_background: Color,
        foreground: Color,
    ) -> Self {
        if let Kind::Dropdown {
            background: dropdown_background,
            option_background: dropdown_option_background,
            foreground: dropdown_foreground,
            ..
        } = &mut self.0.kind
        {
            *dropdown_background = background;
            *dropdown_option_background = option_background;
            *dropdown_foreground = foreground;
        }
        self
    }

    pub fn colors_triplet(self, colors: (Color, Color, Color)) -> Self {
        self.colors(colors.0, colors.1, colors.2)
    }

    pub fn focus_background_tint(mut self, color: Color) -> Self {
        self.0.style.focus_background_tint = Some(color);
        self
    }

    pub fn controller_focus_background_tint(mut self, color: Color) -> Self {
        self.0.style.controller_focus_background_tint = Some(color);
        self
    }
}

impl<Message> Component<Message> for Dropdown<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

pub struct MenuItem<Message = String> {
    label: String,
    message: Option<Message>,
}

impl<Message> MenuItem<Message> {
    pub fn new(label: impl Into<String>, message: Message) -> Self {
        Self {
            label: label.into(),
            message: Some(message),
        }
    }

    pub fn disabled(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            message: None,
        }
    }
}

pub struct Menu<Message = String>(Element<Message>);

impl<Message: Clone> Menu<Message> {
    pub fn new(
        toggle_message: Message,
        label: impl Into<String>,
        items: impl IntoIterator<Item = MenuItem<Message>>,
    ) -> Self {
        let items = items.into_iter().collect::<Vec<_>>();
        let mut element = Element {
            id: None,
            source: None,
            kind: Kind::Dropdown {
                selected: label.into(),
                options: items.iter().map(|item| item.label.clone()).collect(),
                expanded: false,
                open_generation: 0,
                overlay: true,
                background: 0x171b22,
                option_background: 0x202630,
                foreground: 0xe8edf4,
            },
            style: Style::default(),
            message: Some(toggle_message.clone()),
            // A menu header is itself a valid context-menu affordance. Keep
            // secondary-click, Shift+F10, controller menu, and accessibility
            // invocation on the same typed transition as ordinary activation.
            context_message: Some(toggle_message),
            message_mapper: None,
            drag_mapper: None,
            text_mapper: None,
            option_messages: items.into_iter().map(|item| item.message).collect(),
            inline_messages: Vec::new(),
            children: Vec::new(),
            navigation_scope: Some(crate::NavigationScope::group()),
            adjustment_step: 0.05,
        };
        element.style.height = Length::Px(30.0);
        element.style.semantic_role = Some(SemanticRole::Menu);
        Self(element)
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.0 = self.0.width(width);
        self
    }

    pub fn accessibility_label(mut self, label: impl Into<String>) -> Self {
        self.0.style.accessibility_label = Some(label.into());
        self
    }

    pub fn expanded(mut self, expanded: bool) -> Self {
        if let Kind::Dropdown {
            expanded: is_expanded,
            options,
            ..
        } = &mut self.0.kind
        {
            *is_expanded = expanded;
            self.0.style.height = Length::Px(
                30.0 + if expanded {
                    options.len() as f32 * 36.0
                } else {
                    0.0
                },
            );
        }
        self
    }

    pub fn colors(
        mut self,
        background: Color,
        option_background: Color,
        foreground: Color,
    ) -> Self {
        if let Kind::Dropdown {
            background: header,
            option_background: options,
            foreground: text,
            ..
        } = &mut self.0.kind
        {
            *header = background;
            *options = option_background;
            *text = foreground;
        }
        self
    }
}

impl<Message> Component<Message> for Menu<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

pub struct MenuBar<Message = String>(Row<Message>);

impl<Message> MenuBar<Message> {
    pub fn new() -> Self {
        Self(Row::new().height(30.0).shrink(0.0).background(0x171b22))
    }
    pub fn child(mut self, child: impl Component<Message>) -> Self {
        self.0 = self.0.child(child);
        self
    }
    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }
    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.0 = self.0.background(background);
        self
    }
}

impl<Message> Default for MenuBar<Message> {
    fn default() -> Self {
        Self::new()
    }
}
impl<Message> Component<Message> for MenuBar<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

#[cfg(test)]
mod semantic_control_tests {
    use super::*;
    use crate::{UiEvent, UiFrame, UiStateStore};

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Message {
        Activate,
        Select,
        SelectOther,
    }

    fn theme() -> SemanticTheme {
        SemanticTheme::from_tokens(crate::SemanticTokenSet::standard(
            0x101010, 0x181818, 0x202020, 0x242424, 0x303030, 0xf0f0f0, 0xa0a0a0, 0x9050e0,
            0x402060, 0x50c080, 0x50c080,
        ))
    }

    fn light_theme() -> SemanticTheme {
        SemanticTheme::from_tokens(crate::SemanticTokenSet::standard(
            0xf4f5f7, 0xe7e9ed, 0xffffff, 0xdfe3e8, 0xd4d9e0, 0x17191d, 0x555b66, 0x7440bd,
            0xe5d8f7, 0x207a4b, 0x207a4b,
        ))
    }

    fn high_contrast_theme() -> SemanticTheme {
        SemanticTheme::resolve(
            light_theme().tokens(),
            theme().tokens(),
            crate::ResolvedThemePreferences {
                appearance: crate::ResolvedAppearance::Dark,
                high_contrast: true,
                reduced_transparency: false,
                reduced_motion: false,
            },
        )
    }

    fn state_sheet(theme: SemanticTheme) -> impl Component<Message> {
        Column::new()
            .id("semantic-state-sheet")
            .gap(theme.spacing.content)
            .padding(Insets::all(theme.spacing.section))
            .background(theme.surfaces.window)
            .child(
                Text::new("Semantic controls")
                    .scale(1.7)
                    .color(theme.text.primary),
            )
            .child(
                Row::new().gap(theme.spacing.control).children([
                    Button::semantic(
                        theme,
                        Message::Activate,
                        "Default",
                        ButtonPresentation::Primary,
                    )
                    .id("button-default"),
                    Button::semantic(
                        theme,
                        Message::Activate,
                        "Hovered",
                        ButtonPresentation::Secondary,
                    )
                    .id("button-hovered"),
                    Button::semantic(
                        theme,
                        Message::Activate,
                        "Pressed",
                        ButtonPresentation::Secondary,
                    )
                    .id("button-pressed"),
                    Button::semantic(
                        theme,
                        Message::Activate,
                        "Focused",
                        ButtonPresentation::Quiet,
                    )
                    .id("button-focused"),
                    Button::semantic(
                        theme,
                        Message::Activate,
                        "Disabled",
                        ButtonPresentation::Disabled,
                    )
                    .id("button-disabled"),
                ]),
            )
            .child(
                Row::new().gap(theme.spacing.control).children([
                    Button::semantic(
                        theme,
                        Message::Activate,
                        "Destructive",
                        ButtonPresentation::Destructive,
                    )
                    .id("button-destructive"),
                    Button::semantic(
                        theme,
                        Message::Activate,
                        "Controller focus",
                        ButtonPresentation::Secondary,
                    )
                    .id("button-controller"),
                ]),
            )
            .child(Row::new().gap(theme.spacing.section).children([
                RadioButton::semantic(theme, Message::Select, "Selected", true),
                RadioButton::semantic(theme, Message::Select, "Unselected", false),
            ]))
            .child(Row::new().gap(theme.spacing.section).children([
                crate::Switch::with_state(
                    crate::SwitchState::Off,
                    Some(|_| Message::Activate),
                    theme,
                ),
                crate::Switch::with_state(
                    crate::SwitchState::On,
                    Some(|_| Message::Activate),
                    theme,
                ),
                crate::Switch::with_state(crate::SwitchState::MixedUnavailable, None, theme),
                crate::Switch::with_state(crate::SwitchState::DisabledOn, None, theme),
            ]))
    }

    #[test]
    fn semantic_component_state_sheets_render_every_theme_and_scale() {
        for (theme_name, theme) in [
            ("dark", theme()),
            ("light", light_theme()),
            (
                "automatic-dark",
                SemanticTheme::resolve(
                    light_theme().tokens(),
                    theme().tokens(),
                    crate::ResolvedThemePreferences {
                        appearance: crate::ResolvedAppearance::Dark,
                        high_contrast: false,
                        reduced_transparency: false,
                        reduced_motion: false,
                    },
                ),
            ),
            ("high-contrast", high_contrast_theme()),
            (
                "reduced-transparency",
                SemanticTheme::resolve(
                    light_theme().tokens(),
                    theme().tokens(),
                    crate::ResolvedThemePreferences {
                        appearance: crate::ResolvedAppearance::Dark,
                        high_contrast: false,
                        reduced_transparency: true,
                        reduced_motion: false,
                    },
                ),
            ),
            ("reduced-motion", theme().with_reduced_motion()),
        ] {
            let bounds = Rect::new(0.0, 0.0, 720.0, 360.0);
            let mut state = UiStateStore::default();
            let initial = UiFrame::layout_with_state(state_sheet(theme), bounds, &mut state);
            let id = |suffix: &str| {
                initial
                    .resolved_layout()
                    .nodes()
                    .iter()
                    .find(|node| node.id.as_str().ends_with(suffix))
                    .map(|node| node.id.clone())
                    .expect("state-sheet control identity")
            };
            let _ = state.set_hovered(Some(id("button-hovered")));
            let _ = state.set_pressed(Some(id("button-pressed")));
            let _ = state.set_focus(Some(id("button-focused")));
            let _ = state
                .navigation_mut()
                .set_controller_selected(Some(id("button-controller")));
            let tree = UiFrame::layout_with_state_and_diagnostics(
                state_sheet(theme),
                bounds,
                &mut state,
                true,
            );
            assert!(
                tree.diagnostics().is_empty(),
                "{theme_name} state sheet: {:#?}",
                tree.diagnostics()
            );

            for scale in [1.0_f32, 1.25, 2.0] {
                let width = (bounds.size.width * scale) as u32;
                let height = (bounds.size.height * scale) as u32;
                let mut renderer = crate::SoftwareRenderer::new_pixel_buffer(width, height, scale);
                assert!(!renderer.render(tree.commands()).is_empty());
                assert!(renderer.pixels().iter().any(|pixel| pixel.a > 0));
                let image = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(
                    width,
                    height,
                    |x, y| {
                        let pixel = renderer.pixels()[(y * width + x) as usize];
                        image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
                    },
                );
                let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/nickel-ui-snapshots")
                    .join(format!("semantic-{theme_name}-{scale:.2}x.png"));
                std::fs::create_dir_all(output.parent().expect("snapshot parent")).unwrap();
                image.save(output).unwrap();
            }
        }
    }

    #[test]
    fn semantic_button_presentations_resolve_theme_roles() {
        let theme = theme();
        let cases = [
            (
                ButtonPresentation::Primary,
                Some(Background::Solid(theme.accent.ordinary)),
                Some(theme.accent.ordinary),
                theme.accent.on_accent,
            ),
            (
                ButtonPresentation::Secondary,
                Some(Background::Solid(theme.surfaces.raised)),
                Some(theme.borders.ordinary),
                theme.text.primary,
            ),
            (ButtonPresentation::Quiet, None, None, theme.text.primary),
            (
                ButtonPresentation::Destructive,
                Some(Background::Solid(theme.text.danger)),
                Some(theme.text.danger),
                theme.accent.on_accent,
            ),
        ];

        for (presentation, background, border, foreground) in cases {
            let button = Button::semantic(theme, Message::Activate, "Apply", presentation)
                .0
                .0;
            assert_eq!(button.style.background, background);
            assert_eq!(button.style.border, border);
            assert_eq!(button.style.height, Length::Px(theme.sizing.control_height));
            assert_eq!(button.style.corner_radius, theme.radii.control);
            assert_eq!(button.children[0].style.foreground, Some(foreground));
            assert_eq!(button.message, Some(Message::Activate));
        }
    }

    #[test]
    fn semantic_button_paints_hover_pressed_and_focus_from_transient_state() {
        let theme = theme();
        let view = || {
            Button::semantic(
                theme,
                Message::Activate,
                "Apply",
                ButtonPresentation::Primary,
            )
            .id("apply")
        };
        let bounds = Rect::new(0.0, 0.0, 180.0, 60.0);
        let mut state = UiStateStore::default();
        let mut tree = UiFrame::layout_with_state(view(), bounds, &mut state);
        let target = tree
            .semantic_targets_for_message(&Message::Activate)
            .into_iter()
            .next()
            .expect("semantic button has a target")
            .bounds;
        let point = Point {
            x: target.origin.x + target.size.width / 2.0,
            y: target.origin.y + target.size.height / 2.0,
        };

        let _ = tree.handle_event(&mut state, UiEvent::PointerMoved(point));
        tree = UiFrame::layout_with_state(view(), bounds, &mut state);
        assert!(tree.commands().iter().any(|command| matches!(
            command,
            PaintCommand::RoundedFill { color, .. } if *color == theme.surfaces.hover
        )));

        let _ = tree.handle_event(&mut state, UiEvent::PointerPressed(point));
        tree = UiFrame::layout_with_state(view(), bounds, &mut state);
        assert!(tree.commands().iter().any(|command| matches!(
            command,
            PaintCommand::RoundedFill { color, .. } if *color == theme.surfaces.pressed
        )));

        let _ = tree.handle_event(&mut state, UiEvent::PointerReleased(point));
        let _ = tree.handle_event(&mut state, UiEvent::FocusNext);
        tree = UiFrame::layout_with_state(view(), bounds, &mut state);
        let focused = crate::focused_surface(theme.accent.ordinary, theme.borders.focus);
        assert!(
            tree.commands().iter().any(|command| matches!(
                command,
                PaintCommand::RoundedFill { color, .. } if *color == focused
            )),
            "expected {focused:06x} in {:?}",
            tree.commands()
        );
        assert!(!tree.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Stroke { color, .. } if *color == theme.borders.focus
        )));
    }

    #[test]
    fn disabled_button_has_no_hit_region_or_message() {
        let theme = theme();
        let tree = UiFrame::layout(
            Button::semantic(
                theme,
                Message::Activate,
                "Unavailable",
                ButtonPresentation::Disabled,
            )
            .id("disabled"),
            Rect::new(0.0, 0.0, 180.0, 60.0),
        );

        assert!(
            tree.semantic_targets_for_message(&Message::Activate)
                .is_empty()
        );
        let node = tree
            .resolved_layout()
            .nodes()
            .first()
            .expect("disabled button remains in layout");
        assert!(!node.interaction.interactive);
    }

    #[test]
    fn semantic_icon_tints_alpha_and_excludes_decorative_nodes() {
        let source = Arc::new(RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 128])));
        let labeled = UiFrame::layout(
            Icon::<Message>::new(7, Arc::clone(&source), 0x804020, 18.0).label("Settings"),
            Rect::new(0.0, 0.0, 24.0, 24.0),
        );
        assert!(
            labeled
                .accessibility_nodes()
                .iter()
                .any(|node| { node.label.as_deref() == Some("Settings") })
        );
        assert!(labeled.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Image { image, .. }
                if image.get_pixel(0, 0).0 == [0x80, 0x40, 0x20, 128]
        )));

        let decorative = UiFrame::layout(
            Icon::<Message>::new(8, source, 0xffffff, 18.0).decorative(),
            Rect::new(0.0, 0.0, 24.0, 24.0),
        );
        assert!(decorative.accessibility_nodes().is_empty());

        let decorative_image = UiFrame::layout(
            Image::<Message>::new(
                9,
                Arc::new(RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))),
            )
            .decorative(),
            Rect::new(0.0, 0.0, 24.0, 24.0),
        );
        assert!(decorative_image.semantic_nodes().is_empty());
        assert!(decorative_image.accessibility_nodes().is_empty());
        assert!(
            decorative_image
                .commands()
                .iter()
                .any(|command| matches!(command, PaintCommand::Image { .. }))
        );
    }

    #[test]
    fn radio_indicator_is_drawn_without_unicode_and_keeps_typed_activation() {
        let theme = theme();
        let tree = UiFrame::layout(
            RadioButton::semantic(theme, Message::Select, "Dark", true).id("dark"),
            Rect::new(0.0, 0.0, 180.0, 44.0),
        );

        let indicator_circles = tree
            .commands()
            .iter()
            .filter(|command| matches!(command, PaintCommand::RoundedFill { radius, .. } if *radius == 9.0 || *radius == 4.0))
            .count();
        assert_eq!(indicator_circles, 2);
        assert!(!tree.commands().iter().any(|command| {
            matches!(command, PaintCommand::Text { text, .. } if text == "●" || text == "○")
        }));
        assert!(
            !tree
                .semantic_targets_for_message(&Message::Select)
                .is_empty()
        );
    }

    #[test]
    fn unselected_radio_has_no_inner_selection_mark() {
        let tree = UiFrame::layout(
            RadioButton::semantic(theme(), Message::Select, "Light", false),
            Rect::new(0.0, 0.0, 180.0, 44.0),
        );
        let radii = tree
            .commands()
            .iter()
            .filter_map(|command| match command {
                PaintCommand::RoundedFill { radius, .. } => Some(*radius),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(radii.contains(&9.0));
        assert!(!radii.contains(&4.0));
    }

    #[test]
    fn radio_group_owns_semantics_rows_and_typed_selection() {
        let theme = theme();
        let tree = UiFrame::layout(
            RadioGroup::new([
                RadioOption::new(theme, Message::Select, "Headphones", true)
                    .description("Active")
                    .id("headphones"),
                RadioOption::new(theme, Message::SelectOther, "Speakers", false)
                    .description("Available")
                    .id("speakers"),
            ])
            .id("outputs"),
            Rect::new(0.0, 0.0, 360.0, 180.0),
        );

        assert_eq!(
            tree.accessibility_nodes()
                .iter()
                .filter(|node| node.role.as_deref() == Some("radiogroup"))
                .count(),
            1
        );
        let radios = tree
            .accessibility_nodes()
            .iter()
            .filter(|node| node.role.as_deref() == Some("radio"))
            .collect::<Vec<_>>();
        assert_eq!(radios.len(), 2);
        assert_eq!(radios[0].state.as_deref(), Some("selected"));
        assert_eq!(radios[1].state.as_deref(), Some("unselected"));
        assert!(
            !tree
                .semantic_targets_for_message(&Message::SelectOther)
                .is_empty()
        );
    }

    #[test]
    fn radio_group_owns_controller_enter_operate_and_exit_depth() {
        let theme = theme();
        let mut state = UiStateStore::default();
        let tree = UiFrame::layout_with_state(
            RadioGroup::new([
                RadioOption::new(theme, Message::Select, "Headphones", true).id("headphones"),
                RadioOption::new(theme, Message::SelectOther, "Speakers", false).id("speakers"),
            ])
            .id("outputs"),
            Rect::new(0.0, 0.0, 360.0, 180.0),
            &mut state,
        );

        tree.handle_event(&mut state, UiEvent::ControllerDown);
        assert_eq!(
            state.navigation().controller_selected().map(UiId::as_str),
            Some("root/outputs")
        );

        tree.handle_event(&mut state, UiEvent::ControllerActivate);
        assert_eq!(
            state.navigation().controller_scope().map(UiId::as_str),
            Some("root/outputs")
        );
        assert!(
            state
                .navigation()
                .controller_selected()
                .is_some_and(|id| id.as_str().ends_with("/headphones"))
        );

        tree.handle_event(&mut state, UiEvent::ControllerDown);
        assert!(
            state
                .navigation()
                .controller_selected()
                .is_some_and(|id| id.as_str().ends_with("/speakers"))
        );
        let activation = tree.handle_event(&mut state, UiEvent::ControllerActivate);
        assert_eq!(activation.messages, vec![Message::SelectOther]);

        tree.handle_event(&mut state, UiEvent::ControllerBack);
        assert_eq!(state.navigation().controller_scope(), None);
        assert_eq!(
            state.navigation().controller_selected().map(UiId::as_str),
            Some("root/outputs")
        );
    }
}
