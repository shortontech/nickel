use super::*;
use crate::SemanticTheme;

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
            message_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
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
            message_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
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
            message_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
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
            message_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
        })
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Component<Message>>) -> Self {
        self.0 = self.0.children(children);
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
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
        Self {
            grid: Grid::fixed(columns),
        }
    }

    pub fn auto_fit(minimum_width: f32) -> Self {
        Self {
            grid: Grid::auto_fit(Track::minmax(
                Track::px(minimum_width.max(1.0)),
                Track::fr(1.0),
            )),
        }
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
}

impl<Message> Component<Message> for FileGrid<Message> {
    fn into_element(self) -> Element<Message> {
        self.grid.into_element()
    }
}

pub struct FileGridItem<Message = String>(Container<Message>);

impl<Message> FileGridItem<Message> {
    pub fn new(
        message: Message,
        label: impl Into<String>,
        icon_id: u16,
        icon: Arc<RgbaImage>,
    ) -> Self {
        Self(
            Container::new()
                .padding(Insets {
                    top: 12.0,
                    right: 8.0,
                    bottom: 8.0,
                    left: 8.0,
                })
                .message(message)
                .child(
                    Column::new()
                        .gap(7.0)
                        .child(Image::new(icon_id, icon).height(62.0))
                        .child(
                            Text::new(label)
                                .height(27.0)
                                .scale(1.2)
                                .align(TextAlign::Center),
                        ),
                ),
        )
    }

    pub fn colors(mut self, background: Color, border: Color, foreground: Color) -> Self {
        self.0 = self.0.background(background).border(border, 1.0);
        if let Some(column) = self.0.0.children.first_mut()
            && let Some(label) = column.children.get_mut(1)
        {
            label.style.foreground = Some(foreground);
        }
        self
    }

    pub fn borderless_colors(mut self, background: Color, foreground: Color) -> Self {
        self.0 = self.0.background(background);
        if let Some(column) = self.0.0.children.first_mut()
            && let Some(label) = column.children.get_mut(1)
        {
            label.style.foreground = Some(foreground);
        }
        self
    }

    pub fn borderless_palette(self, colors: (Color, Color)) -> Self {
        self.borderless_colors(colors.0, colors.1)
    }

    pub fn icon_size(mut self, size: f32) -> Self {
        if let Some(column) = self.0.0.children.first_mut()
            && let Some(icon) = column.children.first_mut()
        {
            icon.style.height = Length::Px(size.max(1.0));
        }
        self
    }
}

impl<Message> Component<Message> for FileGridItem<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

pub struct StyledText<Message = String>(Element<Message>);

impl<Message> StyledText<Message> {
    pub fn new(value: impl Into<String>, spans: Vec<StyledTextSpan>) -> Self {
        Self(Element {
            kind: Kind::StyledText {
                value: value.into(),
                spans,
                scale: 2.0,
                wrap: false,
                line_height: None,
            },
            id: None,
            source: None,
            style: Style::default(),
            message: None,
            message_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
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
        Self(Element::text(value, 2.0))
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

impl<Message> Image<Message> {
    pub fn new(id: u16, image: Arc<RgbaImage>) -> Self {
        Self(Element {
            id: None,
            source: None,
            kind: Kind::Image {
                id,
                image,
                presentation: ImagePresentation::default(),
            },
            style: Style {
                overflow_x: Overflow::Clip,
                overflow_y: Overflow::Clip,
                ..Style::default()
            },
            message: None,
            message_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
        })
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
}

impl<Message> Component<Message> for Image<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
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
}

impl<Message> TextField<Message> {
    pub fn new(editor: &TextEditor) -> Self {
        let displayed = editor.display_text_with_caret("▏");
        Self {
            text: Text::new(&displayed),
            displayed,
        }
    }

    pub fn placeholder(editor: &TextEditor, placeholder: impl Into<String>) -> Self {
        if editor.text().is_empty() && editor.preedit().is_empty() {
            let displayed = placeholder.into();
            Self {
                text: Text::new(&displayed),
                displayed,
            }
        } else {
            Self::new(editor)
        }
    }

    pub fn on_change(value: &str, map: fn(String) -> Message) -> Self {
        let mut field = Self {
            text: Text::new(value),
            displayed: value.to_owned(),
        };
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
}

impl<Message> Component<Message> for TextField<Message> {
    fn into_element(self) -> Element<Message> {
        self.text.into_element()
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

    pub fn message(mut self, message: Message) -> Self {
        self.0 = self.0.message(message);
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
        Self(
            Container::new().height(36.0).child(
                Row::new()
                    .child(
                        Container::new()
                            .width(28.0)
                            .height(36.0)
                            .message(toggle_message)
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

    pub fn indent(mut self, depth: usize) -> Self {
        self.0.0.style.padding.left = depth as f32 * 16.0;
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
        Self::with_label(message, ButtonLabel::new(label))
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
        Self(Text::new(value).align(TextAlign::Center))
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
        )
    }

    fn with_colors(
        message: Message,
        label: impl Into<String>,
        selected: bool,
        indicator: Color,
        label_color: Color,
        background: Color,
    ) -> Self {
        Self(
            Container::new().height(34.0).message(message).child(
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
            message_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
        };
        element.style.height = Length::Px(24.0);
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
                overlay: false,
                background: 0x27344c,
                option_background: 0x34445f,
                foreground: 0xf4f7ff,
            },
            style: Style::default(),
            message: Some(toggle_message),
            message_mapper: None,
            text_mapper: None,
            option_messages,
            inline_messages: Vec::new(),
            children: Vec::new(),
        };
        element.style.height = Length::Px(42.0);
        Self(element)
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
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

impl<Message> Menu<Message> {
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
                overlay: true,
                background: 0x171b22,
                option_background: 0x202630,
                foreground: 0xe8edf4,
            },
            style: Style::default(),
            message: Some(toggle_message),
            message_mapper: None,
            text_mapper: None,
            option_messages: items.into_iter().map(|item| item.message).collect(),
            inline_messages: Vec::new(),
            children: Vec::new(),
        };
        element.style.height = Length::Px(30.0);
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
    use crate::{SemanticColors, UiTree};

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Message {
        Activate,
        Select,
    }

    fn theme() -> SemanticTheme {
        SemanticTheme::new(SemanticColors {
            window: 0x101010,
            sidebar: 0x181818,
            card: 0x202020,
            raised: 0x242424,
            hover: 0x303030,
            primary_text: 0xf0f0f0,
            secondary_text: 0xa0a0a0,
            accent: 0x9050e0,
            accent_soft: 0x402060,
            positive: 0x50c080,
        })
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
    fn disabled_button_has_no_hit_region_or_message() {
        let theme = theme();
        let tree = UiTree::layout(
            Button::semantic(
                theme,
                Message::Activate,
                "Unavailable",
                ButtonPresentation::Disabled,
            )
            .id("disabled"),
            Rect::new(0.0, 0.0, 180.0, 60.0),
        );

        assert_eq!(tree.message_rect(&Message::Activate), None);
        let node = tree
            .resolved_layout()
            .nodes()
            .first()
            .expect("disabled button remains in layout");
        assert!(!node.interaction.interactive);
    }

    #[test]
    fn radio_indicator_is_drawn_without_unicode_and_keeps_typed_activation() {
        let theme = theme();
        let tree = UiTree::layout(
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
        assert!(tree.message_rect(&Message::Select).is_some());
    }

    #[test]
    fn unselected_radio_has_no_inner_selection_mark() {
        let tree = UiTree::layout(
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
}
