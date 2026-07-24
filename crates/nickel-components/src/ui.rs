use crate::{Axis, Insets, Point, Rect};

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
    Grid { columns: usize },
    Text { value: String, scale: f32 },
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
        Axis::Vertical => child.style.height.or(match child.kind {
            Kind::Text { scale, .. } => Some(text_line_height(scale)),
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
}
