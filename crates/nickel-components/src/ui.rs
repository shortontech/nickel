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
    Gradient {
        rect: Rect,
        gradient: LinearGradient,
    },
    Stroke {
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
    },
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
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Flex(Axis),
    Grid {
        columns: usize,
    },
    Text {
        value: String,
        scale: f32,
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

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    kind: Kind,
    style: Style,
    action: Option<String>,
    children: Vec<Element>,
}

impl Element {
    fn flex(axis: Axis) -> Self {
        Self {
            kind: Kind::Flex(axis),
            style: Style::default(),
            action: None,
            children: Vec::new(),
        }
    }

    fn text(value: impl Into<String>, scale: f32) -> Self {
        Self {
            kind: Kind::Text {
                value: value.into(),
                scale,
            },
            style: Style::default(),
            action: None,
            children: Vec::new(),
        }
    }

    pub fn child(mut self, child: impl Component) -> Self {
        self.children.push(child.into_element());
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Component>) -> Self {
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

    pub fn action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }
}

pub trait Component {
    fn into_element(self) -> Element;
}

impl Component for Element {
    fn into_element(self) -> Element {
        self
    }
}

macro_rules! flex_component {
    ($name:ident, $axis:expr) => {
        pub struct $name(Element);

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl $name {
            pub fn new() -> Self {
                Self(Element::flex($axis))
            }

            pub fn child(mut self, child: impl Component) -> Self {
                self.0 = self.0.child(child);
                self
            }

            pub fn children(mut self, children: impl IntoIterator<Item = impl Component>) -> Self {
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

        impl Component for $name {
            fn into_element(self) -> Element {
                self.0
            }
        }
    };
}

flex_component!(Column, Axis::Vertical);
flex_component!(Row, Axis::Horizontal);

pub struct Grid(Element);

impl Grid {
    pub fn new() -> Self {
        Self::columns(2)
    }

    pub fn columns(columns: usize) -> Self {
        Self(Element {
            kind: Kind::Grid {
                columns: columns.max(1),
            },
            style: Style::default(),
            action: None,
            children: Vec::new(),
        })
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Component>) -> Self {
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
}

impl Default for Grid {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for Grid {
    fn into_element(self) -> Element {
        self.0
    }
}

pub struct Text(Element);

impl Text {
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

impl Component for Text {
    fn into_element(self) -> Element {
        self.0
    }
}

pub struct Header(Text);

impl Header {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Text::new(value).scale(4.0))
    }

    pub fn color(mut self, color: Color) -> Self {
        self.0 = self.0.color(color);
        self
    }
}

pub struct TextField {
    text: Text,
    displayed: String,
}

impl TextField {
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

impl Component for TextField {
    fn into_element(self) -> Element {
        self.text.into_element()
    }
}

impl Component for Header {
    fn into_element(self) -> Element {
        self.0.into_element()
    }
}

pub struct Container(Element);

impl Container {
    pub fn new() -> Self {
        Self(Element::flex(Axis::Vertical))
    }

    pub fn child(mut self, child: impl Component) -> Self {
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

    pub fn action(mut self, action: impl Into<String>) -> Self {
        self.0 = self.0.action(action);
        self
    }
}

impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for Container {
    fn into_element(self) -> Element {
        self.0
    }
}

pub struct Button(Container);

impl Button {
    pub fn new(action: impl Into<String>, label: impl Into<String>) -> Self {
        Self::with_label(action, ButtonLabel::new(label))
    }

    pub fn with_label(action: impl Into<String>, label: ButtonLabel) -> Self {
        Self(
            Container::new()
                .padding(Insets {
                    top: 11.0,
                    right: 12.0,
                    bottom: 8.0,
                    left: 12.0,
                })
                .height(42.0)
                .action(action)
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
}

impl Component for Button {
    fn into_element(self) -> Element {
        self.0.into_element()
    }
}

pub struct ButtonLabel(Text);

impl ButtonLabel {
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

impl Component for ButtonLabel {
    fn into_element(self) -> Element {
        self.0.into_element()
    }
}

pub struct Slider(Element);

impl Slider {
    pub fn new(action: impl Into<String>, value: f32) -> Self {
        let mut element = Element {
            kind: Kind::Slider {
                value: value.clamp(0.0, 1.0),
                track: 0x354158,
                fill: 0x68b8ff,
                thumb: 0xf4f7ff,
            },
            style: Style::default(),
            action: Some(action.into()),
            children: Vec::new(),
        };
        element.style.height = Some(24.0);
        Self(element)
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

impl Component for Slider {
    fn into_element(self) -> Element {
        self.0
    }
}

pub struct Dropdown(Element);

impl Dropdown {
    pub fn new(
        action: impl Into<String>,
        selected: impl Into<String>,
        options: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let options: Vec<_> = options.into_iter().map(Into::into).collect();
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
            action: Some(action.into()),
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

impl Component for Dropdown {
    fn into_element(self) -> Element {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq)]
struct HitRegion {
    rect: Rect,
    action: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiTree {
    commands: Vec<PaintCommand>,
    hits: Vec<HitRegion>,
}

impl UiTree {
    pub fn layout(root: impl Component, bounds: Rect) -> Self {
        let mut tree = Self::default();
        layout_element(&root.into_element(), bounds, None, &mut tree);
        tree
    }

    pub fn commands(&self) -> &[PaintCommand] {
        &self.commands
    }

    pub fn action_at(&self, point: Point) -> Option<&str> {
        self.hits
            .iter()
            .rev()
            .find(|hit| contains(hit.rect, point))
            .map(|hit| hit.action.as_str())
    }

    pub fn action_at_with_horizontal_fraction(&self, point: Point) -> Option<(&str, f32)> {
        self.hits
            .iter()
            .rev()
            .find(|hit| contains(hit.rect, point))
            .map(|hit| {
                let fraction =
                    ((point.x - hit.rect.origin.x) / hit.rect.size.width.max(1.0)).clamp(0.0, 1.0);
                (hit.action.as_str(), fraction)
            })
    }
}

fn layout_element(
    element: &Element,
    bounds: Rect,
    inherited_foreground: Option<Color>,
    tree: &mut UiTree,
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
    if let Some(action) = &element.action {
        tree.hits.push(HitRegion {
            rect,
            action: action.clone(),
        });
    }

    let foreground = element.style.foreground.or(inherited_foreground);
    match &element.kind {
        Kind::Text { value, scale } => tree.commands.push(PaintCommand::Text {
            bounds: rect,
            text: value.clone(),
            scale: *scale,
            color: foreground.unwrap_or(0x00ff_ffff),
            align: element.style.text_align,
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
            });
            if *expanded {
                let action = element.action.as_deref().unwrap_or("dropdown");
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
                    });
                    tree.hits.push(HitRegion {
                        rect: option_rect,
                        action: format!("{action}:option:{index}"),
                    });
                }
            }
        }
        Kind::Flex(axis) => {
            let content = rect.inset(element.style.padding);
            let child_bounds = flex_bounds(content, *axis, element.style.gap, &element.children);
            for (child, bounds) in element.children.iter().zip(child_bounds) {
                layout_element(child, bounds, foreground, tree);
            }
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
                    tree,
                );
            }
        }
    }
}

fn flex_bounds(content: Rect, axis: Axis, gap: f32, children: &[Element]) -> Vec<Rect> {
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

fn child_main_size(child: &Element, axis: Axis) -> Option<f32> {
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

    #[test]
    fn nested_button_is_laid_out_and_hit_tested() {
        let tree = UiTree::layout(
            Column::new()
                .height(100.0)
                .child(Header::new("Steam"))
                .child(Button::new("launch", "Launch").width(100.0)),
            Rect::new(0.0, 0.0, 300.0, 100.0),
        );
        assert_eq!(tree.action_at(Point { x: 10.0, y: 40.0 }), Some("launch"));
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
                Button::new("one", "One"),
                Button::new("two", "Two"),
                Button::new("three", "Three"),
            ]),
            Rect::new(0.0, 0.0, 200.0, 100.0),
        );
        assert_eq!(tree.hits.len(), 3);
    }

    #[test]
    fn slider_reports_horizontal_fraction() {
        let tree = UiTree::layout(
            Slider::new("volume", 0.5).width(200.0),
            Rect::new(0.0, 0.0, 200.0, 24.0),
        );
        let (action, fraction) = tree
            .action_at_with_horizontal_fraction(Point { x: 150.0, y: 12.0 })
            .expect("slider hit");
        assert_eq!(action, "volume");
        assert!((fraction - 0.75).abs() < 0.001);
    }

    #[test]
    fn expanded_dropdown_exposes_option_actions() {
        let tree = UiTree::layout(
            Dropdown::new("audio", "Speakers", ["Speakers", "Headphones"]).expanded(true),
            Rect::new(0.0, 0.0, 240.0, 114.0),
        );
        assert_eq!(
            tree.action_at(Point { x: 20.0, y: 96.0 }),
            Some("audio:option:1")
        );
    }
}
