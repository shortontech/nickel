use std::sync::Arc;

use image::RgbaImage;

use crate::{Axis, Insets, Point, Rect, TextEditor};

pub type Color = u32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GradientAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearGradient {
    pub start: Color,
    pub end: Color,
    pub axis: GradientAxis,
}

impl LinearGradient {
    pub const fn vertical(start: Color, end: Color) -> Self {
        Self {
            start,
            end,
            axis: GradientAxis::Vertical,
        }
    }

    pub const fn horizontal(start: Color, end: Color) -> Self {
        Self {
            start,
            end,
            axis: GradientAxis::Horizontal,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Background {
    Solid(Color),
    LinearGradient(LinearGradient),
}

impl From<Color> for Background {
    fn from(color: Color) -> Self {
        Self::Solid(color)
    }
}

impl From<LinearGradient> for Background {
    fn from(gradient: LinearGradient) -> Self {
        Self::LinearGradient(gradient)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaintCommand {
    Fill {
        rect: Rect,
        color: Color,
    },
    TopRoundedFill {
        rect: Rect,
        color: Color,
        radius: f32,
    },
    RoundedFill {
        rect: Rect,
        color: Color,
        radius: f32,
    },
    Gradient {
        rect: Rect,
        gradient: LinearGradient,
    },
    Stroke {
        rect: Rect,
        color: Color,
        width: f32,
    },
    OverlayFill {
        rect: Rect,
        color: Color,
    },
    OverlayStroke {
        rect: Rect,
        color: Color,
        width: f32,
    },
    Text {
        bounds: Rect,
        text: String,
        scale: f32,
        color: Color,
        align: TextAlign,
        bold: bool,
    },
    Image {
        bounds: Rect,
        id: u16,
        image: Arc<RgbaImage>,
    },
    PushClip(Rect),
    PopClip,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub background: Option<Background>,
    pub border: Option<Color>,
    pub border_width: f32,
    pub foreground: Option<Color>,
    pub text_align: TextAlign,
    pub padding: Insets,
    pub gap: f32,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub grow: f32,
    pub top_corner_radius: f32,
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Flex(Axis),
    VerticalScroll {
        offset: f32,
        content_height: f32,
    },
    Grid {
        columns: usize,
    },
    Text {
        value: String,
        scale: f32,
        bold: bool,
    },
    Image {
        id: u16,
        image: Arc<RgbaImage>,
    },
    Slider {
        value: f32,
        track: Color,
        fill: Color,
        thumb: Color,
    },
    Dropdown {
        selected: String,
        options: Vec<String>,
        expanded: bool,
        background: Color,
        option_background: Color,
        foreground: Color,
    },
}

#[derive(Clone, Debug)]
pub struct Element<Message = String> {
    kind: Kind,
    style: Style,
    message: Option<Message>,
    message_mapper: Option<fn(f32) -> Message>,
    option_messages: Vec<Message>,
    children: Vec<Element<Message>>,
}

impl<Message> Element<Message> {
    fn flex(axis: Axis) -> Self {
        Self {
            kind: Kind::Flex(axis),
            style: Style::default(),
            message: None,
            message_mapper: None,
            option_messages: Vec::new(),
            children: Vec::new(),
        }
    }

    fn text(value: impl Into<String>, scale: f32) -> Self {
        Self {
            kind: Kind::Text {
                value: value.into(),
                scale,
                bold: false,
            },
            style: Style::default(),
            message: None,
            message_mapper: None,
            option_messages: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn child(mut self, child: impl Component<Message>) -> Self {
        self.children.push(child.into_element());
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Component<Message>>) -> Self {
        self.children
            .extend(children.into_iter().map(Component::into_element));
        self
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.style.background = Some(background.into());
        self
    }

    pub fn border(mut self, color: Color, width: f32) -> Self {
        self.style.border = Some(color);
        self.style.border_width = width;
        self
    }

    pub fn top_corner_radius(mut self, radius: f32) -> Self {
        self.style.top_corner_radius = radius.max(0.0);
        self
    }

    pub fn foreground(mut self, color: Color) -> Self {
        self.style.foreground = Some(color);
        self
    }

    pub fn text_align(mut self, align: TextAlign) -> Self {
        self.style.text_align = align;
        self
    }

    pub fn padding(mut self, padding: Insets) -> Self {
        self.style.padding = padding;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.style.gap = gap;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.style.width = Some(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.style.height = Some(height);
        self
    }

    pub fn grow(mut self, grow: f32) -> Self {
        self.style.grow = grow;
        self
    }

    pub fn message(mut self, message: Message) -> Self {
        self.message = Some(message);
        self
    }

    pub fn map_message<ParentMessage, Map>(self, mut map: Map) -> Element<ParentMessage>
    where
        Map: FnMut(Message) -> ParentMessage,
    {
        self.map_message_with(&mut map)
    }

    fn map_message_with<ParentMessage, Map>(self, map: &mut Map) -> Element<ParentMessage>
    where
        Map: FnMut(Message) -> ParentMessage,
    {
        assert!(
            self.message_mapper.is_none(),
            "map value-producing messages at the control constructor"
        );
        Element {
            kind: self.kind,
            style: self.style,
            message: self.message.map(&mut *map),
            message_mapper: None,
            option_messages: self.option_messages.into_iter().map(&mut *map).collect(),
            children: self
                .children
                .into_iter()
                .map(|child| child.map_message_with(map))
                .collect(),
        }
    }
}

pub trait Component<Message = String> {
    fn into_element(self) -> Element<Message>;
}

impl<Message> Component<Message> for Element<Message> {
    fn into_element(self) -> Element<Message> {
        self
    }
}

macro_rules! flex_component {
    ($name:ident, $axis:expr) => {
        pub struct $name<Message = String>(Element<Message>);

        impl<Message> Default for $name<Message> {
            fn default() -> Self {
                Self::new()
            }
        }

        impl<Message> $name<Message> {
            pub fn new() -> Self {
                Self(Element::flex($axis))
            }

            pub fn child(mut self, child: impl Component<Message>) -> Self {
                self.0 = self.0.child(child);
                self
            }

            pub fn children(
                mut self,
                children: impl IntoIterator<Item = impl Component<Message>>,
            ) -> Self {
                self.0 = self.0.children(children);
                self
            }

            pub fn gap(mut self, gap: f32) -> Self {
                self.0 = self.0.gap(gap);
                self
            }

            pub fn padding(mut self, padding: Insets) -> Self {
                self.0 = self.0.padding(padding);
                self
            }

            pub fn background(mut self, background: impl Into<Background>) -> Self {
                self.0 = self.0.background(background);
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

            pub fn grow(mut self, grow: f32) -> Self {
                self.0 = self.0.grow(grow);
                self
            }
        }

        impl<Message> Component<Message> for $name<Message> {
            fn into_element(self) -> Element<Message> {
                self.0
            }
        }
    };
}

flex_component!(Column, Axis::Vertical);
flex_component!(Row, Axis::Horizontal);

pub struct VerticalScroll<Message = String>(Element<Message>);

impl<Message> VerticalScroll<Message> {
    pub fn new(message: Message, offset: f32, viewport_height: f32, content_height: f32) -> Self {
        let mut element = Element {
            kind: Kind::VerticalScroll {
                offset: offset.clamp(0.0, (content_height - viewport_height).max(0.0)),
                content_height: content_height.max(viewport_height),
            },
            style: Style::default(),
            message: Some(message),
            message_mapper: None,
            option_messages: Vec::new(),
            children: Vec::new(),
        };
        element.style.height = Some(viewport_height.max(0.0));
        Self(element)
    }

    pub fn child(mut self, child: impl Component<Message>) -> Self {
        self.0 = self.0.child(child);
        self
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.0 = self.0.background(background);
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
        Self::columns(2)
    }

    pub fn columns(columns: usize) -> Self {
        Self(Element {
            kind: Kind::Grid {
                columns: columns.max(1),
            },
            style: Style::default(),
            message: None,
            message_mapper: None,
            option_messages: Vec::new(),
            children: Vec::new(),
        })
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Component<Message>>) -> Self {
        self.0 = self.0.children(children);
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.0 = self.0.gap(gap);
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
            grid: Grid::columns(columns),
        }
    }

    pub fn items(mut self, items: impl IntoIterator<Item = FileGridItem<Message>>) -> Self {
        self.grid = self.grid.children(items);
        self
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

    pub fn icon_size(mut self, size: f32) -> Self {
        if let Some(column) = self.0.0.children.first_mut()
            && let Some(icon) = column.children.first_mut()
        {
            icon.style.height = Some(size.max(1.0));
        }
        self
    }
}

impl<Message> Component<Message> for FileGridItem<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

pub struct Text<Message = String>(Element<Message>);

impl<Message> Text<Message> {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Element::text(value, 2.0))
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

    pub fn bold(mut self, bold: bool) -> Self {
        if let Kind::Text {
            bold: text_bold, ..
        } = &mut self.0.kind
        {
            *text_bold = bold;
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

    pub fn align(mut self, align: TextAlign) -> Self {
        self.0 = self.0.text_align(align);
        self
    }
}

impl<Message> Component<Message> for Text<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

pub struct Image<Message = String>(Element<Message>);

impl<Message> Image<Message> {
    pub fn new(id: u16, image: Arc<RgbaImage>) -> Self {
        Self(Element {
            kind: Kind::Image { id, image },
            style: Style::default(),
            message: None,
            message_mapper: None,
            option_messages: Vec::new(),
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

    pub fn display_text(&self) -> &str {
        &self.displayed
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.text = self.text.scale(scale);
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.text = self.text.color(color);
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

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.0 = self.0.background(background);
        self
    }

    pub fn border(mut self, color: Color, width: f32) -> Self {
        self.0 = self.0.border(color, width);
        self
    }

    pub fn top_corner_radius(mut self, radius: f32) -> Self {
        self.0 = self.0.top_corner_radius(radius);
        self
    }

    pub fn padding(mut self, padding: Insets) -> Self {
        self.0 = self.0.padding(padding);
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

    pub fn grow(mut self, grow: f32) -> Self {
        self.0 = self.0.grow(grow);
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
        Self(Column::new().width(width))
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.0 = self.0.background(background);
        self
    }

    pub fn padding(mut self, padding: Insets) -> Self {
        self.0 = self.0.padding(padding);
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.0 = self.0.gap(gap);
        self
    }

    pub fn child(mut self, child: impl Component<Message>) -> Self {
        self.0 = self.0.child(child);
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

impl<Message> Button<Message> {
    pub fn new(message: Message, label: impl Into<String>) -> Self {
        Self::with_label(message, ButtonLabel::new(label))
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

    pub fn width(mut self, width: f32) -> Self {
        self.0 = self.0.width(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.0 = self.0.height(height);
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
}

impl<Message> Component<Message> for ButtonLabel<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

pub struct RadioButton<Message = String>(Container<Message>);

impl<Message> RadioButton<Message> {
    pub fn new(message: Message, label: impl Into<String>, selected: bool) -> Self {
        let indicator = if selected { "●" } else { "○" };
        Self(
            Container::new().height(34.0).message(message).child(
                Row::new()
                    .gap(10.0)
                    .child(
                        Text::new(indicator)
                            .width(22.0)
                            .scale(1.35)
                            .color(if selected { 0x68b8ff } else { 0x8792a8 }),
                    )
                    .child(Text::new(label).scale(1.15).color(0xf4f7ff)),
            ),
        )
    }

    pub fn width(mut self, width: f32) -> Self {
        self.0 = self.0.width(width);
        self
    }

    pub fn colors(mut self, indicator: Color, label: Color) -> Self {
        if let Some(row) = self.0.0.children.first_mut() {
            if let Some(indicator_text) = row.children.first_mut() {
                indicator_text.style.foreground = Some(indicator);
            }
            if let Some(label_text) = row.children.get_mut(1) {
                label_text.style.foreground = Some(label);
            }
        }
        self
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
            kind: Kind::Slider {
                value: value.clamp(0.0, 1.0),
                track: 0x354158,
                fill: 0x68b8ff,
                thumb: 0xf4f7ff,
            },
            style: Style::default(),
            message: Some(message),
            message_mapper: None,
            option_messages: Vec::new(),
            children: Vec::new(),
        };
        element.style.height = Some(24.0);
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
            .map(|(label, message)| (label.into(), message))
            .unzip();
        let mut element = Element {
            kind: Kind::Dropdown {
                selected: selected.into(),
                options,
                expanded: false,
                background: 0x27344c,
                option_background: 0x34445f,
                foreground: 0xf4f7ff,
            },
            style: Style::default(),
            message: Some(toggle_message),
            message_mapper: None,
            option_messages,
            children: Vec::new(),
        };
        element.style.height = Some(42.0);
        Self(element)
    }

    pub fn expanded(mut self, expanded: bool) -> Self {
        if let Kind::Dropdown {
            expanded: is_expanded,
            options,
            ..
        } = &mut self.0.kind
        {
            *is_expanded = expanded;
            self.0.style.height = Some(
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
}

impl<Message> Component<Message> for Dropdown<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

#[derive(Clone, Debug)]
struct HitRegion<Message> {
    rect: Rect,
    message: Message,
    message_mapper: Option<fn(f32) -> Message>,
}

#[derive(Clone, Debug)]
pub struct UiTree<Message = String> {
    commands: Vec<PaintCommand>,
    hits: Vec<HitRegion<Message>>,
}

impl<Message> Default for UiTree<Message> {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
            hits: Vec::new(),
        }
    }
}

impl<Message: Clone> UiTree<Message> {
    pub fn layout(root: impl Component<Message>, bounds: Rect) -> Self {
        let mut tree = Self::default();
        layout_element(&root.into_element(), bounds, None, None, &mut tree);
        tree
    }

    pub fn commands(&self) -> &[PaintCommand] {
        &self.commands
    }

    pub fn message_at(&self, point: Point) -> Option<&Message> {
        self.hits
            .iter()
            .rev()
            .find(|hit| contains(hit.rect, point))
            .map(|hit| &hit.message)
    }

    pub fn message_at_with_horizontal_fraction(&self, point: Point) -> Option<(&Message, f32)> {
        self.hits
            .iter()
            .rev()
            .find(|hit| contains(hit.rect, point))
            .map(|hit| {
                let fraction =
                    ((point.x - hit.rect.origin.x) / hit.rect.size.width.max(1.0)).clamp(0.0, 1.0);
                (&hit.message, fraction)
            })
    }

    pub fn message_at_owned(&self, point: Point) -> Option<Message> {
        self.hits
            .iter()
            .rev()
            .find(|hit| contains(hit.rect, point))
            .map(|hit| {
                let fraction =
                    ((point.x - hit.rect.origin.x) / hit.rect.size.width.max(1.0)).clamp(0.0, 1.0);
                hit.message_mapper
                    .map(|map| map(fraction))
                    .unwrap_or_else(|| hit.message.clone())
            })
    }

    pub fn horizontal_fraction_for_message(&self, message: &Message, x: f32) -> Option<f32>
    where
        Message: PartialEq,
    {
        self.hits
            .iter()
            .rev()
            .find(|hit| &hit.message == message)
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
            .find(|hit| predicate(&hit.message))
            .map(|hit| ((x - hit.rect.origin.x) / hit.rect.size.width.max(1.0)).clamp(0.0, 1.0))
    }

    pub fn messages_intersecting(&self, rect: Rect) -> Vec<&Message> {
        self.hits
            .iter()
            .filter(|hit| rects_intersect(hit.rect, rect))
            .map(|hit| &hit.message)
            .collect()
    }

    pub fn push_overlay_command(&mut self, command: PaintCommand) {
        self.commands.push(command);
    }

    pub fn push_overlay_message(&mut self, rect: Rect, message: Message) {
        self.hits.push(HitRegion {
            rect,
            message,
            message_mapper: None,
        });
    }
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

fn layout_element<Message: Clone>(
    element: &Element<Message>,
    bounds: Rect,
    inherited_foreground: Option<Color>,
    inherited_clip: Option<Rect>,
    tree: &mut UiTree<Message>,
) {
    let width = element
        .style
        .width
        .unwrap_or(bounds.size.width)
        .min(bounds.size.width);
    let height = element
        .style
        .height
        .unwrap_or(bounds.size.height)
        .min(bounds.size.height);
    let rect = Rect::new(bounds.origin.x, bounds.origin.y, width, height);
    if let Some(background) = element.style.background {
        tree.commands.push(match background {
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
    if let Some(message) = &element.message
        && let Some(rect) = inherited_clip
            .map(|clip| intersection(rect, clip))
            .unwrap_or(Some(rect))
    {
        tree.hits.push(HitRegion {
            rect,
            message: message.clone(),
            message_mapper: element.message_mapper,
        });
    }

    let foreground = element.style.foreground.or(inherited_foreground);
    match &element.kind {
        Kind::Text { value, scale, bold } => tree.commands.push(PaintCommand::Text {
            bounds: rect,
            text: value.clone(),
            scale: *scale,
            color: foreground.unwrap_or(0x00ff_ffff),
            align: element.style.text_align,
            bold: *bold,
        }),
        Kind::Image { id, image } => tree.commands.push(PaintCommand::Image {
            bounds: rect,
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
            background,
            option_background,
            foreground,
        } => {
            let header = Rect::new(rect.origin.x, rect.origin.y, rect.size.width, 42.0);
            tree.commands.push(PaintCommand::Fill {
                rect: header,
                color: *background,
            });
            tree.commands.push(PaintCommand::Text {
                bounds: header.inset(Insets {
                    top: 10.0,
                    right: 36.0,
                    bottom: 8.0,
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
                    header.origin.y + 10.0,
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
                        rect.origin.y + 42.0 + index as f32 * 36.0,
                        rect.size.width,
                        36.0,
                    );
                    tree.commands.push(PaintCommand::Fill {
                        rect: option_rect,
                        color: *option_background,
                    });
                    tree.commands.push(PaintCommand::Text {
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
                    if let Some(message) = element.option_messages.get(index) {
                        tree.hits.push(HitRegion {
                            rect: option_rect,
                            message: message.clone(),
                            message_mapper: None,
                        });
                    }
                }
            }
        }
        Kind::Flex(axis) => {
            let content = rect.inset(element.style.padding);
            let child_bounds = flex_bounds(content, *axis, element.style.gap, &element.children);
            for (child, bounds) in element.children.iter().zip(child_bounds) {
                layout_element(child, bounds, foreground, inherited_clip, tree);
            }
        }
        Kind::VerticalScroll {
            offset,
            content_height,
        } => {
            let viewport = rect.inset(element.style.padding);
            let clip = inherited_clip
                .and_then(|parent| intersection(parent, viewport))
                .unwrap_or(viewport);
            tree.commands.push(PaintCommand::PushClip(viewport));
            if let Some(child) = element.children.first() {
                layout_element(
                    child,
                    Rect::new(
                        viewport.origin.x,
                        viewport.origin.y - offset,
                        viewport.size.width,
                        *content_height,
                    ),
                    foreground,
                    Some(clip),
                    tree,
                );
            }
            tree.commands.push(PaintCommand::PopClip);
        }
        Kind::Grid { columns } => {
            let content = rect.inset(element.style.padding);
            let rows = element.children.len().div_ceil(*columns);
            if rows == 0 {
                return;
            }
            let cell_width = (content.size.width
                - element.style.gap * columns.saturating_sub(1) as f32)
                / *columns as f32;
            let cell_height = (content.size.height
                - element.style.gap * rows.saturating_sub(1) as f32)
                / rows as f32;
            for (index, child) in element.children.iter().enumerate() {
                let column = index % columns;
                let row = index / columns;
                layout_element(
                    child,
                    Rect::new(
                        content.origin.x + column as f32 * (cell_width + element.style.gap),
                        content.origin.y + row as f32 * (cell_height + element.style.gap),
                        cell_width,
                        cell_height,
                    ),
                    foreground,
                    inherited_clip,
                    tree,
                );
            }
        }
    }
}

fn flex_bounds<Message>(
    content: Rect,
    axis: Axis,
    gap: f32,
    children: &[Element<Message>],
) -> Vec<Rect> {
    let available = match axis {
        Axis::Horizontal => content.size.width,
        Axis::Vertical => content.size.height,
    };
    let gap_total = gap * children.len().saturating_sub(1) as f32;
    let fixed = children
        .iter()
        .map(|child| child_main_size(child, axis).unwrap_or(0.0))
        .sum::<f32>();
    let flexible: Vec<_> = children
        .iter()
        .enumerate()
        .filter(|(_, child)| child_main_size(child, axis).is_none())
        .collect();
    let grow_total = flexible
        .iter()
        .map(|(_, child)| child.style.grow.max(0.0))
        .sum::<f32>();
    let remaining = (available - fixed - gap_total).max(0.0);
    let default_flexible = if flexible.is_empty() {
        0.0
    } else {
        remaining / flexible.len() as f32
    };
    let mut cursor = match axis {
        Axis::Horizontal => content.origin.x,
        Axis::Vertical => content.origin.y,
    };
    children
        .iter()
        .map(|child| {
            let main = child_main_size(child, axis).unwrap_or_else(|| {
                if grow_total > 0.0 {
                    remaining * child.style.grow.max(0.0) / grow_total
                } else {
                    default_flexible
                }
            });
            let rect = match axis {
                Axis::Horizontal => Rect::new(cursor, content.origin.y, main, content.size.height),
                Axis::Vertical => Rect::new(content.origin.x, cursor, content.size.width, main),
            };
            cursor += main + gap;
            rect
        })
        .collect()
}

fn child_main_size<Message>(child: &Element<Message>, axis: Axis) -> Option<f32> {
    match axis {
        Axis::Horizontal => child.style.width,
        Axis::Vertical => child.style.height.or(match &child.kind {
            Kind::Text { scale, .. } => Some(text_line_height(*scale)),
            Kind::Slider { .. } => Some(24.0),
            Kind::Dropdown {
                options, expanded, ..
            } => Some(
                42.0 + if *expanded {
                    options.len() as f32 * 36.0
                } else {
                    0.0
                },
            ),
            _ => None,
        }),
    }
}

fn text_line_height(scale: f32) -> f32 {
    match scale.round() as i32 {
        0 | 1 => 16.0,
        2 => 22.0,
        3 => 29.0,
        _ => 39.0,
    }
}

fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.origin.x
        && point.x < rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y < rect.origin.y + rect.size.height
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum TestMessage {
        Named(&'static str),
        Option(usize),
        Volume(u8),
    }

    #[test]
    fn nested_button_is_laid_out_and_hit_tested() {
        let tree = UiTree::layout(
            Column::new()
                .height(100.0)
                .child(Header::new("Steam"))
                .child(Button::new(TestMessage::Named("launch"), "Launch").width(100.0)),
            Rect::new(0.0, 0.0, 300.0, 100.0),
        );
        assert_eq!(
            tree.message_at(Point { x: 10.0, y: 40.0 }),
            Some(&TestMessage::Named("launch"))
        );
        assert!(
            tree.commands().iter().any(
                |command| matches!(command, PaintCommand::Text { text, .. } if text == "Steam")
            )
        );
    }

    #[test]
    fn grid_places_all_children() {
        let tree = UiTree::layout(
            Grid::columns(2).children([
                Button::new(TestMessage::Named("one"), "One"),
                Button::new(TestMessage::Named("two"), "Two"),
                Button::new(TestMessage::Named("three"), "Three"),
            ]),
            Rect::new(0.0, 0.0, 200.0, 100.0),
        );
        assert_eq!(tree.hits.len(), 3);
    }

    #[test]
    fn file_grid_tiles_expose_actions_and_centered_labels() {
        let icon = Arc::new(RgbaImage::new(16, 16));
        let tree = UiTree::layout(
            FileGrid::columns(2).gap(8.0).height(120.0).items([
                FileGridItem::new(TestMessage::Named("file:one"), "One", 1, icon.clone())
                    .colors(0x101010, 0x202020, 0xffffff)
                    .icon_size(48.0),
                FileGridItem::new(TestMessage::Named("file:two"), "Two", 2, icon)
                    .colors(0x101010, 0x202020, 0xffffff),
            ]),
            Rect::new(0.0, 0.0, 240.0, 120.0),
        );
        assert_eq!(
            tree.message_at(Point { x: 20.0, y: 20.0 }),
            Some(&TestMessage::Named("file:one"))
        );
        assert_eq!(
            tree.message_at(Point { x: 180.0, y: 20.0 }),
            Some(&TestMessage::Named("file:two"))
        );
        assert!(tree.commands.iter().any(|command| {
            matches!(
                command,
                PaintCommand::Text {
                    text,
                    align: TextAlign::Center,
                    ..
                } if text == "One"
            )
        }));
        assert!(tree.commands.iter().any(
            |command| matches!(command, PaintCommand::Image { bounds, id: 1, .. } if bounds.size.height == 48.0)
        ));
    }

    #[test]
    fn slider_reports_horizontal_fraction() {
        let tree = UiTree::layout(
            Slider::new(TestMessage::Named("volume"), 0.5).width(200.0),
            Rect::new(0.0, 0.0, 200.0, 24.0),
        );
        let (message, fraction) = tree
            .message_at_with_horizontal_fraction(Point { x: 150.0, y: 12.0 })
            .expect("slider hit");
        assert_eq!(message, &TestMessage::Named("volume"));
        assert!((fraction - 0.75).abs() < 0.001);
        assert_eq!(
            tree.horizontal_fraction_for_message(&TestMessage::Named("volume"), 250.0),
            Some(1.0)
        );
    }

    #[test]
    fn value_control_emits_typed_payload_and_component_messages_map() {
        fn volume_message(fraction: f32) -> TestMessage {
            TestMessage::Volume((fraction * 100.0).round() as u8)
        }

        let mapped = Button::new(2_usize, "Second")
            .into_element()
            .map_message(TestMessage::Option);
        let tree = UiTree::layout(
            Column::new()
                .child(mapped)
                .child(Slider::on_change(volume_message, 0.5).width(200.0)),
            Rect::new(0.0, 0.0, 200.0, 66.0),
        );

        assert_eq!(
            tree.message_at_owned(Point { x: 20.0, y: 10.0 }),
            Some(TestMessage::Option(2))
        );
        assert_eq!(
            tree.message_at_owned(Point { x: 150.0, y: 54.0 }),
            Some(TestMessage::Volume(75))
        );
    }

    #[test]
    fn vertical_scroll_clips_painting_and_hit_regions() {
        let tree = UiTree::layout(
            VerticalScroll::new(TestMessage::Named("scroll"), 50.0, 100.0, 200.0).child(
                Column::new()
                    .gap(10.0)
                    .child(
                        Container::new()
                            .height(60.0)
                            .message(TestMessage::Named("one")),
                    )
                    .child(
                        Container::new()
                            .height(60.0)
                            .message(TestMessage::Named("two")),
                    )
                    .child(
                        Container::new()
                            .height(60.0)
                            .message(TestMessage::Named("three")),
                    ),
            ),
            Rect::new(0.0, 0.0, 200.0, 100.0),
        );

        assert!(matches!(
            tree.commands.first(),
            Some(PaintCommand::PushClip(rect)) if *rect == Rect::new(0.0, 0.0, 200.0, 100.0)
        ));
        assert!(matches!(tree.commands.last(), Some(PaintCommand::PopClip)));
        assert_eq!(
            tree.message_at(Point { x: 20.0, y: 5.0 }),
            Some(&TestMessage::Named("one"))
        );
        assert_eq!(
            tree.message_at(Point { x: 20.0, y: 40.0 }),
            Some(&TestMessage::Named("two"))
        );
        assert_eq!(
            tree.message_at(Point { x: 20.0, y: 95.0 }),
            Some(&TestMessage::Named("three"))
        );
        assert_eq!(tree.message_at(Point { x: 20.0, y: 105.0 }), None);
    }

    #[test]
    fn vertical_scroll_clamps_offset_to_content_end() {
        let tree = UiTree::layout(
            VerticalScroll::new(TestMessage::Named("scroll"), 500.0, 100.0, 200.0).child(
                Column::new()
                    .gap(10.0)
                    .child(
                        Container::new()
                            .height(60.0)
                            .message(TestMessage::Named("one")),
                    )
                    .child(
                        Container::new()
                            .height(60.0)
                            .message(TestMessage::Named("two")),
                    ),
            ),
            Rect::new(0.0, 0.0, 200.0, 100.0),
        );

        assert_eq!(
            tree.message_at(Point { x: 20.0, y: 10.0 }),
            Some(&TestMessage::Named("two"))
        );
    }

    #[test]
    fn expanded_dropdown_exposes_option_actions() {
        let tree = UiTree::layout(
            Dropdown::new(
                TestMessage::Named("audio"),
                "Speakers",
                [
                    ("Speakers", TestMessage::Option(0)),
                    ("Headphones", TestMessage::Option(1)),
                ],
            )
            .expanded(true),
            Rect::new(0.0, 0.0, 240.0, 114.0),
        );
        assert_eq!(
            tree.message_at(Point { x: 20.0, y: 96.0 }),
            Some(&TestMessage::Option(1))
        );
    }

    #[test]
    fn sidebar_folder_separates_toggle_and_open_actions() {
        let tree = UiTree::layout(
            Sidebar::new(220.0)
                .child(HorizontalRule::new(0x808080))
                .child(SidebarFolder::new(
                    TestMessage::Named("toggle"),
                    TestMessage::Named("open"),
                    "Desktop",
                    false,
                    0xffffff,
                )),
            Rect::new(0.0, 0.0, 220.0, 100.0),
        );
        assert_eq!(
            tree.message_at(Point { x: 10.0, y: 32.0 }),
            Some(&TestMessage::Named("toggle"))
        );
        assert_eq!(
            tree.message_at(Point { x: 80.0, y: 32.0 }),
            Some(&TestMessage::Named("open"))
        );
        assert!(tree.commands().iter().any(
            |command| matches!(command, PaintCommand::Fill { rect, .. } if rect.size.height == 1.0)
        ));
    }

    #[test]
    fn top_corner_radius_emits_rounded_fill() {
        let tree = UiTree::<()>::layout(
            Container::new()
                .width(120.0)
                .height(32.0)
                .background(0xffffff)
                .top_corner_radius(7.0),
            Rect::new(0.0, 0.0, 120.0, 32.0),
        );
        assert!(tree.commands().iter().any(|command| matches!(
            command,
            PaintCommand::TopRoundedFill { radius, .. } if *radius == 7.0
        )));
    }
}
