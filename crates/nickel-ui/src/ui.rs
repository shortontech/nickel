use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt::Write,
    ops::Range,
    sync::Arc,
};

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight, Wrap};
use image::RgbaImage;
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    Align, Axis, Constraints, FlexItem, Insets, Invalidation, Justify, Length, Overflow, Point,
    Rect, SelectionDocument, SelectionEndpoint, SelectionRun, Size, TextBoundary, TextEditor,
    Track, UiId, UiStateStore, layout_flex,
};

pub type Color = u32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    pub color: Color,
    pub width: f32,
}

impl Border {
    pub const fn new(color: Color, width: f32) -> Self {
        Self { color, width }
    }
}

impl From<(Color, f32)> for Border {
    fn from((color, width): (Color, f32)) -> Self {
        Self::new(color, width)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceLocation {
    pub component: &'static str,
    pub file: &'static str,
    pub line: u32,
    pub column: u32,
}

impl SourceLocation {
    pub const fn new(component: &'static str, file: &'static str, line: u32, column: u32) -> Self {
        Self {
            component,
            file,
            line,
            column,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UiEvent {
    PointerMoved(Point),
    PointerPressed(Point),
    PointerReleased(Point),
    Scroll { point: Point, delta_y: f32 },
    ScrollHorizontal { point: Point, delta_x: f32 },
    FocusNext,
    FocusPrevious,
    ControllerNext,
    ControllerPrevious,
    ActivateFocused,
    KeyboardActivate,
    ControllerActivate,
    AccessibilityActivate(UiId),
    TextInput(String),
    ImePreedit(String),
    TextBackspace,
    TextBackspaceWord,
    TextDelete,
    TextMoveLeft { extend_selection: bool },
    TextMoveRight { extend_selection: bool },
    TextMoveWordLeft { extend_selection: bool },
    TextMoveWordRight { extend_selection: bool },
    TextMoveHome { extend_selection: bool },
    TextMoveEnd { extend_selection: bool },
    TextMoveDocumentHome { extend_selection: bool },
    TextMoveDocumentEnd { extend_selection: bool },
    TextSelectAll,
    TextCopy,
    TextCut,
    TextPaste(String),
    SelectionClear,
    Dismiss,
    CaretBlink,
    FocusLost,
    Suspended,
    DeviceRemoved,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EventOutcome<Message> {
    pub messages: Vec<Message>,
    pub clipboard_text: Option<String>,
    pub invalidation: Invalidation,
}

impl<Message> Default for EventOutcome<Message> {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            clipboard_text: None,
            invalidation: Invalidation::None,
        }
    }
}

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Tone {
    #[default]
    Default,
    Muted,
    Accent,
    Danger,
}

impl Tone {
    const fn color(self) -> Option<Color> {
        match self {
            Self::Default => None,
            Self::Muted => Some(0x9aa7bd),
            Self::Accent => Some(0x5b8def),
            Self::Danger => Some(0xe35d6a),
        }
    }
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

#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub background: Option<Background>,
    pub border: Option<Color>,
    pub border_width: f32,
    pub foreground: Option<Color>,
    pub text_align: TextAlign,
    pub padding: Insets,
    pub gap: f32,
    pub width: Length,
    pub height: Length,
    pub min_width: f32,
    pub min_height: f32,
    pub max_width: f32,
    pub max_height: f32,
    pub basis: Length,
    pub grow: f32,
    pub shrink: f32,
    pub align_self: Option<Align>,
    pub align_items: Align,
    pub justify_content: Justify,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    /// Keep a vertical scroll region pinned while the user remains at its end.
    pub follow_scroll_end: bool,
    #[doc(hidden)]
    pub scroll_offset_x: f32,
    #[doc(hidden)]
    pub scroll_offset: f32,
    pub corner_radius: f32,
    pub top_corner_radius: f32,
    #[doc(hidden)]
    pub selection_region: bool,
    #[doc(hidden)]
    pub selection_document: Option<Arc<SelectionDocument>>,
    #[doc(hidden)]
    pub selectable: Option<bool>,
    #[doc(hidden)]
    pub selection_run_id: Option<String>,
    #[doc(hidden)]
    pub selection_boundary: TextBoundary,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            background: None,
            border: None,
            border_width: 1.0,
            foreground: None,
            text_align: TextAlign::Start,
            padding: Insets::default(),
            gap: 0.0,
            width: Length::Auto,
            height: Length::Auto,
            min_width: 0.0,
            min_height: 0.0,
            max_width: f32::INFINITY,
            max_height: f32::INFINITY,
            basis: Length::Auto,
            grow: 0.0,
            shrink: 1.0,
            align_self: None,
            align_items: Align::Stretch,
            justify_content: Justify::Start,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            follow_scroll_end: false,
            scroll_offset_x: 0.0,
            scroll_offset: 0.0,
            corner_radius: 0.0,
            top_corner_radius: 0.0,
            selection_region: false,
            selection_document: None,
            selectable: None,
            selection_run_id: None,
            selection_boundary: TextBoundary::Inline,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Flex(Axis),
    VerticalScroll {
        offset: f32,
    },
    Grid {
        columns: GridColumnSpec,
    },
    Text {
        value: String,
        scale: f32,
        bold: bool,
        wrap: bool,
        line_height: Option<f32>,
        max_lines: Option<usize>,
        ellipsis: bool,
        selection_x: Option<(f32, f32)>,
        caret_position: Option<Point>,
        input_value: Option<String>,
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
        overlay: bool,
        background: Color,
        option_background: Color,
        foreground: Color,
    },
}

impl Kind {
    const fn name(&self) -> &'static str {
        match self {
            Self::Flex(Axis::Horizontal) => "Row",
            Self::Flex(Axis::Vertical) => "Column",
            Self::VerticalScroll { .. } => "VerticalScroll",
            Self::Grid { .. } => "Grid",
            Self::Text { .. } => "Text",
            Self::Image { .. } => "Image",
            Self::Slider { .. } => "Slider",
            Self::Dropdown { .. } => "Dropdown",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum GridColumnSpec {
    Count(usize),
    Tracks(Vec<Track>),
    AutoFit(Track),
}

impl From<usize> for GridColumnSpec {
    fn from(value: usize) -> Self {
        Self::Count(value.max(1))
    }
}

impl From<Track> for GridColumnSpec {
    fn from(value: Track) -> Self {
        match value {
            Track::AutoFit(track) => Self::AutoFit(*track),
            track => Self::Tracks(vec![track]),
        }
    }
}

impl From<Vec<Track>> for GridColumnSpec {
    fn from(value: Vec<Track>) -> Self {
        Self::Tracks(value)
    }
}

#[derive(Clone, Debug)]
pub struct Element<Message = String> {
    id: Option<UiId>,
    source: Option<SourceLocation>,
    kind: Kind,
    style: Style,
    message: Option<Message>,
    message_mapper: Option<fn(f32) -> Message>,
    text_mapper: Option<fn(String) -> Message>,
    option_messages: Vec<Option<Message>>,
    children: Vec<Element<Message>>,
}

impl<Message> Element<Message> {
    fn flex(axis: Axis) -> Self {
        Self {
            kind: Kind::Flex(axis),
            id: None,
            source: None,
            style: Style::default(),
            message: None,
            message_mapper: None,
            text_mapper: None,
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
                wrap: false,
                line_height: None,
                max_lines: None,
                ellipsis: false,
                selection_x: None,
                caret_position: None,
                input_value: None,
            },
            id: None,
            source: None,
            style: Style::default(),
            message: None,
            message_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn child(mut self, child: impl Component<Message>) -> Self {
        self.children.push(child.into_element());
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.id = Some(id.into());
        self
    }

    #[doc(hidden)]
    pub fn with_source(mut self, source: SourceLocation) -> Self {
        self.source = Some(source);
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

    pub fn border_value(self, border: impl Into<Border>) -> Self {
        let border = border.into();
        self.border(border.color, border.width)
    }

    pub fn top_corner_radius(mut self, radius: f32) -> Self {
        self.style.top_corner_radius = radius.max(0.0);
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.style.corner_radius = radius.max(0.0);
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

    pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
        self.style.padding = padding.into();
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.style.gap = gap;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.style.width = Length::Px(width);
        self
    }

    pub fn width_length(mut self, width: Length) -> Self {
        self.style.width = width;
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.style.height = Length::Px(height);
        self
    }

    pub fn height_length(mut self, height: Length) -> Self {
        self.style.height = height;
        self
    }

    pub fn grow(mut self, grow: f32) -> Self {
        self.style.grow = grow;
        self
    }

    pub fn shrink(mut self, shrink: f32) -> Self {
        self.style.shrink = shrink.max(0.0);
        self
    }

    pub fn basis(mut self, basis: impl Into<Length>) -> Self {
        self.style.basis = basis.into();
        self
    }

    pub fn align_self(mut self, align: Align) -> Self {
        self.style.align_self = Some(align);
        self
    }

    pub fn align_items(mut self, align: Align) -> Self {
        self.style.align_items = align;
        self
    }

    pub fn justify_content(mut self, justify: Justify) -> Self {
        self.style.justify_content = justify;
        self
    }

    pub fn overflow(mut self, x: Overflow, y: Overflow) -> Self {
        self.style.overflow_x = x;
        self.style.overflow_y = y;
        self
    }

    pub fn overflow_x(mut self, overflow: Overflow) -> Self {
        self.style.overflow_x = overflow;
        self
    }

    pub fn overflow_y(mut self, overflow: Overflow) -> Self {
        self.style.overflow_y = overflow;
        self
    }

    pub fn follow_scroll_end(mut self, follow: bool) -> Self {
        self.style.follow_scroll_end = follow;
        self
    }

    pub fn on_scroll(mut self, map: fn(f32) -> Message) -> Self {
        self.message_mapper = Some(map);
        self
    }

    pub fn min_width(mut self, width: f32) -> Self {
        self.style.min_width = width.max(0.0);
        self
    }

    pub fn max_width(mut self, width: f32) -> Self {
        self.style.max_width = width.max(0.0);
        self
    }

    pub fn min_height(mut self, height: f32) -> Self {
        self.style.min_height = height.max(0.0);
        self
    }

    pub fn max_height(mut self, height: f32) -> Self {
        self.style.max_height = height.max(0.0);
        self
    }

    pub fn fill_width(mut self) -> Self {
        self.style.width = Length::Fill;
        self
    }

    pub fn fill_height(mut self) -> Self {
        self.style.height = Length::Fill;
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

    pub fn measure(&self, constraints: Constraints) -> Size {
        measure_element(self, constraints)
    }

    fn map_message_with<ParentMessage, Map>(self, map: &mut Map) -> Element<ParentMessage>
    where
        Map: FnMut(Message) -> ParentMessage,
    {
        assert!(
            self.message_mapper.is_none() && self.text_mapper.is_none(),
            "map value-producing messages at the control constructor"
        );
        Element {
            kind: self.kind,
            id: self.id,
            source: self.source,
            style: self.style,
            message: self.message.map(&mut *map),
            message_mapper: None,
            text_mapper: None,
            option_messages: self
                .option_messages
                .into_iter()
                .map(|message| message.map(&mut *map))
                .collect(),
            children: self
                .children
                .into_iter()
                .map(|child| child.map_message_with(map))
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TextMeasureKey {
    text: String,
    locale: String,
    scale: u32,
    width: u32,
    bold: bool,
    wrap: bool,
    line_height: u32,
    max_lines: Option<usize>,
}

thread_local! {
    static TEXT_MEASURER: RefCell<(FontSystem, HashMap<TextMeasureKey, Size>)> =
        RefCell::new((FontSystem::new(), HashMap::new()));
}

fn measure_text(
    text: &str,
    scale: f32,
    bold: bool,
    wrap: bool,
    line_height: Option<f32>,
    max_lines: Option<usize>,
    max_width: f32,
) -> Size {
    let font_size = text_font_size(scale);
    let line_height = line_height.unwrap_or(font_size * 1.3).max(1.0);
    let width = if wrap && max_width.is_finite() {
        max_width.max(1.0)
    } else {
        f32::INFINITY
    };
    TEXT_MEASURER.with(|measurer| {
        let mut measurer = measurer.borrow_mut();
        let key = TextMeasureKey {
            text: text.to_owned(),
            locale: measurer.0.locale().to_owned(),
            scale: scale.to_bits(),
            width: width.to_bits(),
            bold,
            wrap,
            line_height: line_height.to_bits(),
            max_lines,
        };
        if let Some(size) = measurer.1.get(&key).copied() {
            return size;
        }
        let (font_system, cache) = &mut *measurer;
        let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
        buffer.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
        buffer.set_size(width.is_finite().then_some(width), None);
        let mut attrs = Attrs::new().family(Family::SansSerif);
        if bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);
        let mut measured = Size::new(0.0, 0.0);
        for run in buffer.layout_runs().take(max_lines.unwrap_or(usize::MAX)) {
            measured.width = measured.width.max(run.line_w);
            measured.height += run.line_height;
        }
        if measured.height == 0.0 {
            measured.height = line_height;
        }
        if cache.len() >= 2048 {
            cache.clear();
        }
        cache.insert(key, measured);
        measured
    })
}

fn text_font_size(scale: f32) -> f32 {
    match scale.round() as i32 {
        0 | 1 => 12.0,
        2 => 16.0,
        3 => 22.0,
        _ => 30.0,
    }
}

fn text_for_bounds(value: &str, scale: f32, bold: bool, ellipsis: bool, width: f32) -> String {
    if !ellipsis
        || measure_text(value, scale, bold, false, None, Some(1), f32::INFINITY).width <= width
    {
        return value.to_owned();
    }
    let mut text = value.to_owned();
    while !text.is_empty() {
        text.pop();
        let candidate = format!("{text}…");
        if measure_text(&candidate, scale, bold, false, None, Some(1), f32::INFINITY).width <= width
        {
            return candidate;
        }
    }
    "…".to_owned()
}

pub trait Component<Message = String> {
    fn into_element(self) -> Element<Message>;
}

/// Type-erased declarative view used when a collection needs one concrete item type.
pub struct AnyView<Message = String>(Element<Message>);

impl<Message> AnyView<Message> {
    pub fn new(component: impl Component<Message>) -> Self {
        Self(component.into_element())
    }
}

impl<Message> Component<Message> for AnyView<Message> {
    fn into_element(self) -> Element<Message> {
        self.0
    }
}

/// Common builder properties shared by every component representation.
///
/// Component-specific inherent methods keep their concrete wrapper type. These
/// extension methods fill in the same typed style and identity surface for
/// components that do not need a specialized return type.
pub trait ComponentBuilderExt<Message>: Component<Message> + Sized {
    fn id(self, id: impl Into<UiId>) -> Element<Message> {
        self.into_element().id(id)
    }

    fn padding(self, padding: impl Into<Insets>) -> Element<Message> {
        self.into_element().padding(padding)
    }

    fn background(self, background: impl Into<Background>) -> Element<Message> {
        self.into_element().background(background)
    }

    fn border(self, color: Color, width: f32) -> Element<Message> {
        self.into_element().border(color, width)
    }

    fn border_value(self, border: impl Into<Border>) -> Element<Message> {
        self.into_element().border_value(border)
    }

    fn top_corner_radius(self, radius: f32) -> Element<Message> {
        self.into_element().top_corner_radius(radius)
    }

    fn radius(self, radius: f32) -> Element<Message> {
        self.into_element().radius(radius)
    }

    fn foreground(self, color: Color) -> Element<Message> {
        self.into_element().foreground(color)
    }

    fn width(self, width: f32) -> Element<Message> {
        self.into_element().width(width)
    }

    fn width_length(self, width: Length) -> Element<Message> {
        self.into_element().width_length(width)
    }

    fn height(self, height: f32) -> Element<Message> {
        self.into_element().height(height)
    }

    fn height_length(self, height: Length) -> Element<Message> {
        self.into_element().height_length(height)
    }

    fn min_width(self, width: f32) -> Element<Message> {
        self.into_element().min_width(width)
    }

    fn max_width(self, width: f32) -> Element<Message> {
        self.into_element().max_width(width)
    }

    fn min_height(self, height: f32) -> Element<Message> {
        self.into_element().min_height(height)
    }

    fn max_height(self, height: f32) -> Element<Message> {
        self.into_element().max_height(height)
    }

    fn fill_width(self) -> Element<Message> {
        self.into_element().fill_width()
    }

    fn fill_height(self) -> Element<Message> {
        self.into_element().fill_height()
    }

    fn grow(self, grow: f32) -> Element<Message> {
        self.into_element().grow(grow)
    }

    fn shrink(self, shrink: f32) -> Element<Message> {
        self.into_element().shrink(shrink)
    }

    fn basis(self, basis: impl Into<Length>) -> Element<Message> {
        self.into_element().basis(basis)
    }

    fn align_self(self, align: Align) -> Element<Message> {
        self.into_element().align_self(align)
    }

    fn overflow(self, x: Overflow, y: Overflow) -> Element<Message> {
        self.into_element().overflow(x, y)
    }

    fn overflow_x(self, overflow: Overflow) -> Element<Message> {
        self.into_element().overflow_x(overflow)
    }

    fn overflow_y(self, overflow: Overflow) -> Element<Message> {
        self.into_element().overflow_y(overflow)
    }

    fn follow_scroll_end(self, follow: bool) -> Element<Message> {
        self.into_element().follow_scroll_end(follow)
    }
}

impl<Message, T> ComponentBuilderExt<Message> for T where T: Component<Message> {}

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

            pub fn id(mut self, id: impl Into<UiId>) -> Self {
                self.0 = self.0.id(id);
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

            pub fn align_items(mut self, align: Align) -> Self {
                self.0 = self.0.align_items(align);
                self
            }

            pub fn align_self(mut self, align: Align) -> Self {
                self.0 = self.0.align_self(align);
                self
            }

            pub fn align(self, align: Align) -> Self {
                self.align_items(align)
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

            pub fn follow_scroll_end(mut self, follow: bool) -> Self {
                self.0 = self.0.follow_scroll_end(follow);
                self
            }

            pub fn on_scroll(mut self, map: fn(f32) -> Message) -> Self {
                self.0 = self.0.on_scroll(map);
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
            },
            style: Style::default(),
            message: Some(message),
            message_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
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
            kind: Kind::Image { id, image },
            style: Style::default(),
            message: None,
            message_mapper: None,
            text_mapper: None,
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

impl<Message> Button<Message> {
    pub fn new(message: Message, label: impl Into<String>) -> Self {
        Self::with_label(message, ButtonLabel::new(label))
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

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
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
        Self(Row::new().height(30.0).background(0x171b22))
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

#[derive(Clone, Debug)]
struct HitRegion<Message> {
    id: UiId,
    rect: Rect,
    message: Option<Message>,
    message_mapper: Option<fn(f32) -> Message>,
}

#[derive(Clone, Debug)]
struct MessageRegion<Message> {
    id: UiId,
    rect: Rect,
    message: Message,
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
                if !inside && !nearest {
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
    extent: ScrollExtent,
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
    pub component: &'static str,
    pub rect: Rect,
    pub interactive: bool,
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
    pub children: Vec<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedLayout {
    nodes: Vec<ResolvedNode>,
}

impl ResolvedLayout {
    pub fn nodes(&self) -> &[ResolvedNode] {
        &self.nodes
    }

    pub fn find(&self, id: &UiId) -> Option<&ResolvedNode> {
        self.nodes.iter().find(|node| &node.id == id)
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
pub struct UiTree<Message = String> {
    commands: Vec<PaintCommand>,
    overlay_commands: Vec<PaintCommand>,
    hits: Vec<HitRegion<Message>>,
    messages: Vec<MessageRegion<Message>>,
    text_inputs: Vec<TextInputRegion<Message>>,
    selection_regions: Vec<SelectionRegionLayout>,
    selection_paints: HashMap<usize, Vec<Rect>>,
    scrolls: Vec<ScrollRegion<Message>>,
    grids: Vec<ResolvedGrid>,
    accessibility: Vec<AccessibilityNode>,
    resolved: ResolvedLayout,
    diagnostics: Vec<LayoutDiagnostic>,
    diagnostic_keys: HashSet<(DiagnosticKind, UiId)>,
    seen_ids: HashSet<UiId>,
    diagnostics_enabled: bool,
}

impl<Message> Default for UiTree<Message> {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
            overlay_commands: Vec::new(),
            hits: Vec::new(),
            messages: Vec::new(),
            text_inputs: Vec::new(),
            selection_regions: Vec::new(),
            selection_paints: HashMap::new(),
            scrolls: Vec::new(),
            grids: Vec::new(),
            accessibility: Vec::new(),
            resolved: ResolvedLayout::default(),
            diagnostics: Vec::new(),
            diagnostic_keys: HashSet::new(),
            seen_ids: HashSet::new(),
            diagnostics_enabled: false,
        }
    }
}

impl<Message: Clone> UiTree<Message> {
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
            ..Self::default()
        };
        layout_element(&root, &root_id, bounds, None, None, &mut tree);
        tree.selection_regions = collect_selection_regions(&root, &tree.resolved);
        for region in &tree.selection_regions {
            state.touch(region.id.clone());
            state.reconcile_document_selection(region.id.clone(), region.document.clone());
        }
        tree.prepare_selection_paints(state);
        tree.reset_emission();
        emit_element(&root, 0, None, &mut tree);
        tree.commands.append(&mut tree.overlay_commands);
        tree.emit_accessibility_geometry();
        tree.validate_clip_commands();
        for scroll in &tree.scrolls {
            let transient = state.touch(scroll.id.clone());
            transient.scroll_offset_x = scroll.extent.offset_x;
            transient.scroll_offset = scroll.extent.offset;
        }
        state.end_frame();
        tree.apply_interaction_state(state);
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
        tree.commands.append(&mut tree.overlay_commands);
        tree.emit_accessibility_geometry();
        tree.validate_clip_commands();
        tree
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

    pub fn message_for_id(&self, id: &UiId) -> Option<&Message> {
        self.messages
            .iter()
            .rev()
            .find(|region| &region.id == id)
            .map(|region| &region.message)
    }

    pub fn id_for_message(&self, message: &Message) -> Option<&UiId>
    where
        Message: PartialEq,
    {
        self.messages
            .iter()
            .rev()
            .find(|region| &region.message == message)
            .map(|region| &region.id)
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

    pub fn message_rect(&self, message: &Message) -> Option<Rect>
    where
        Message: PartialEq,
    {
        self.hits
            .iter()
            .rev()
            .find(|hit| hit.message.as_ref() == Some(message))
            .map(|hit| hit.rect)
    }

    pub fn message_layout_rect(&self, message: &Message) -> Option<Rect>
    where
        Message: PartialEq,
    {
        self.messages
            .iter()
            .rev()
            .find(|region| &region.message == message)
            .map(|region| region.rect)
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
        for id in retained {
            state.touch(id);
        }
        state.end_frame();
    }

    pub fn handle_event(&self, state: &mut UiStateStore, event: UiEvent) -> EventOutcome<Message> {
        let mut outcome = EventOutcome::default();
        outcome.invalidation = match event {
            UiEvent::PointerMoved(point) => {
                let mut invalidation = state.set_hovered(self.id_at(point).cloned());
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
            UiEvent::PointerPressed(point) => {
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
                        && !clicked.is_some_and(|id| id.starts_with(node.id.as_str()))
                    {
                        dismissed =
                            dismissed.merge(state.set_dropdown_open(node.id.clone(), false));
                    }
                }
                if let Some((region, endpoint)) = self.selection_hit_at(point) {
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
                let released = self.id_at(point);
                let activates = state
                    .captured()
                    .is_some_and(|captured| released == Some(captured));
                if activates && let Some(message) = self.message_at_owned(point) {
                    outcome.messages.push(message);
                }
                let dropdown = state.captured().and_then(|id| {
                    self.resolved_layout()
                        .find(id)
                        .filter(|node| node.component == "Dropdown")
                        .map(|_| id.clone())
                });
                let dropdown_invalidation = dropdown.map_or(Invalidation::None, |id| {
                    let open = !state.state(&id).is_some_and(|entry| entry.dropdown_open);
                    state.set_dropdown_open(id, open)
                });
                let option_parent = state.captured().and_then(|id| {
                    id.as_str()
                        .rsplit_once("/option-")
                        .map(|(parent, _)| UiId::from(parent.to_owned()))
                });
                let option_invalidation = option_parent
                    .map(|id| state.set_dropdown_open(id, false))
                    .unwrap_or(Invalidation::None);
                state
                    .set_pressed(None)
                    .merge(state.set_capture(None))
                    .merge(dropdown_invalidation)
                    .merge(option_invalidation)
            }
            UiEvent::Scroll { point, delta_y } => {
                let Some(scroll) = self
                    .scrolls
                    .iter()
                    .rev()
                    .find(|scroll| contains(scroll.rect, point))
                else {
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
                .find(|scroll| contains(scroll.rect, point))
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
            UiEvent::ControllerNext => self.move_controller(state, 1),
            UiEvent::ControllerPrevious => self.move_controller(state, -1),
            UiEvent::ActivateFocused | UiEvent::KeyboardActivate => {
                if let Some(message) = state
                    .focused()
                    .and_then(|id| self.message_for_id(id))
                    .cloned()
                {
                    outcome.messages.push(message);
                }
                Invalidation::None
            }
            UiEvent::ControllerActivate => {
                if let Some(message) = state
                    .controller_selected()
                    .or_else(|| state.focused())
                    .and_then(|id| self.message_for_id(id))
                    .cloned()
                {
                    outcome.messages.push(message);
                }
                Invalidation::None
            }
            UiEvent::AccessibilityActivate(id) => {
                if let Some(message) = self.message_for_id(&id).cloned() {
                    outcome.messages.push(message);
                }
                Invalidation::None
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
                if let Some(message) = self.edit_focused_text(state, TextEditor::select_all) {
                    outcome.messages.push(message);
                    Invalidation::Paint
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
                outcome.clipboard_text = self.selected_text(state);
                Invalidation::None
            }
            UiEvent::TextCut => {
                let Some(id) = state.focused().cloned() else {
                    return outcome;
                };
                let Some(input) = self.text_inputs.iter().find(|input| input.id == id) else {
                    return outcome;
                };
                let editor = state.editor(id, &input.initial);
                outcome.clipboard_text = editor.cut_selection();
                if outcome.clipboard_text.is_some() {
                    outcome.messages.push((input.map)(editor.text().to_owned()));
                    state.show_caret();
                    Invalidation::Layout
                } else {
                    Invalidation::None
                }
            }
            UiEvent::TextPaste(text) => self
                .edit_focused_text(state, |editor| {
                    let text = text.replace("\r\n", "\n").replace('\r', "\n");
                    editor.insert(&text);
                })
                .map(|message| {
                    outcome.messages.push(message);
                    Invalidation::Layout
                })
                .unwrap_or(Invalidation::None),
            UiEvent::SelectionClear => state.clear_document_selection(),
            UiEvent::Dismiss => self
                .resolved_layout()
                .nodes()
                .iter()
                .filter(|node| node.component == "Dropdown")
                .fold(Invalidation::None, |invalidation, node| {
                    invalidation.merge(state.set_dropdown_open(node.id.clone(), false))
                }),
            UiEvent::CaretBlink => state.toggle_caret(),
            UiEvent::FocusLost => state.focus_lost(),
            UiEvent::Suspended => state.suspended(),
            UiEvent::DeviceRemoved => state.device_removed(),
        };
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
        let mut ids = self.hits.iter().map(|hit| &hit.id).collect::<Vec<_>>();
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
        state.set_focus(Some(ids[next].clone()))
    }

    fn move_controller(&self, state: &mut UiStateStore, direction: isize) -> Invalidation {
        let mut ids = self.hits.iter().map(|hit| &hit.id).collect::<Vec<_>>();
        ids.dedup();
        if ids.is_empty() {
            return state.set_controller_selected(None);
        }
        let current = state
            .controller_selected()
            .and_then(|selected| ids.iter().position(|id| *id == selected));
        let next = match (current, direction.is_negative()) {
            (Some(index), false) => (index + 1) % ids.len(),
            (Some(0), true) | (None, true) => ids.len() - 1,
            (Some(index), true) => index - 1,
            (None, false) => 0,
        };
        state.set_controller_selected(Some(ids[next].clone()))
    }

    pub fn push_overlay_command(&mut self, command: PaintCommand) {
        self.commands.push(command);
    }

    pub fn push_overlay_message(&mut self, rect: Rect, message: Message) {
        self.messages.push(MessageRegion {
            id: UiId::from("overlay"),
            rect,
            message: message.clone(),
        });
        self.hits.push(HitRegion {
            id: UiId::from("overlay"),
            rect,
            message: Some(message),
            message_mapper: None,
        });
    }

    pub fn apply_interaction_state(&mut self, state: &UiStateStore) {
        for node in &mut self.resolved.nodes {
            node.interaction.focused = state.focused() == Some(&node.id);
            node.interaction.hovered = state.hovered() == Some(&node.id);
            node.interaction.pressed = state.pressed() == Some(&node.id);
            node.interaction.captured = state.captured() == Some(&node.id);
            node.interaction.controller_selected = state.controller_selected() == Some(&node.id);
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
        self.accessibility = self
            .resolved
            .nodes
            .iter()
            .filter_map(|node| {
                let rect = node
                    .clip
                    .map(|clip| intersection(node.allocated, clip))
                    .unwrap_or(Some(node.allocated))?;
                Some(AccessibilityNode {
                    id: node.id.clone(),
                    component: node.component,
                    rect,
                    interactive: node.interaction.interactive,
                })
            })
            .collect();
    }

    fn reset_emission(&mut self) {
        self.commands.clear();
        self.overlay_commands.clear();
        self.hits.clear();
        self.messages.clear();
        self.text_inputs.clear();
        for node in &mut self.resolved.nodes {
            node.hit_stack = None;
        }
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

fn measure_element<Message>(element: &Element<Message>, constraints: Constraints) -> Size {
    let horizontal_padding = element.style.padding.width();
    let vertical_padding = element.style.padding.height();
    let child_max = Size::new(
        (constraints.max.width - horizontal_padding).max(0.0),
        (constraints.max.height - vertical_padding).max(0.0),
    );
    let child_constraints = Constraints::loose(child_max);
    let content = match &element.kind {
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
            let sizes = element
                .children
                .iter()
                .map(|child| measure_element(child, child_constraints))
                .collect::<Vec<_>>();
            let widths = resolve_grid_columns(
                columns,
                child_max.width,
                element.style.gap,
                &sizes,
                element.children.len(),
            );
            let columns = widths.len().max(1);
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
    let intrinsic = Size::new(intrinsic_width, content.height + vertical_padding);
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
            || element.message.is_some()
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
        let mut measurer = measurer.borrow_mut();
        let font_system = &mut measurer.0;
        let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
        buffer.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
        buffer.set_size(wrap.then_some(rect.size.width.max(1.0)), None);
        let mut attrs = Attrs::new().family(Family::SansSerif);
        if bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);
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

fn emit_element<Message: Clone>(
    element: &Element<Message>,
    node_index: usize,
    inherited_foreground: Option<Color>,
    tree: &mut UiTree<Message>,
) {
    let node = tree.resolved.nodes[node_index].clone();
    let rect = node.allocated;
    if node
        .clip
        .is_some_and(|clip| intersection(rect, clip).is_none())
    {
        let foreground = element.style.foreground.or(inherited_foreground);
        if matches!(element.kind, Kind::Flex(_) | Kind::Grid { .. }) {
            for (&child_index, child) in node.children.iter().zip(&element.children) {
                emit_element(child, child_index, foreground, tree);
            }
        }
        return;
    }
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
    if let Some(message) = &element.message {
        tree.messages.push(MessageRegion {
            id: node.id.clone(),
            rect,
            message: message.clone(),
        });
        if let Some(hit_rect) = node
            .clip
            .map(|clip| intersection(rect, clip))
            .unwrap_or(Some(rect))
        {
            tree.resolved.nodes[node_index].hit_stack = Some(tree.hits.len());
            tree.hits.push(HitRegion {
                id: node.id.clone(),
                rect: hit_rect,
                message: Some(message.clone()),
                message_mapper: element.message_mapper,
            });
        }
    }
    if let Some(map) = element.text_mapper
        && let Kind::Text {
            value, input_value, ..
        } = &element.kind
    {
        let (scale, bold, line_height) = match &element.kind {
            Kind::Text {
                scale,
                bold,
                line_height,
                ..
            } => (
                *scale,
                *bold,
                line_height.unwrap_or_else(|| text_font_size(*scale) * 1.3),
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
                message: None,
                message_mapper: None,
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
        Kind::Text {
            value,
            scale,
            bold,
            ellipsis,
            ..
        } => tree.commands.push(PaintCommand::Text {
            bounds: rect,
            text: text_for_bounds(value, *scale, *bold, *ellipsis, rect.size.width),
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
            overlay,
            background,
            option_background,
            foreground,
        } => {
            let header_height = if *overlay { 30.0 } else { 42.0 };
            let option_height = if *overlay { 34.0 } else { 36.0 };
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
            });
            if *expanded {
                for (index, option) in options.iter().enumerate() {
                    let option_rect = Rect::new(
                        rect.origin.x,
                        rect.origin.y + header_height + index as f32 * option_height,
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
                    });
                    if let Some(Some(message)) = element.option_messages.get(index) {
                        let option_id = node.id.scoped(format!("option-{index}"));
                        tree.messages.push(MessageRegion {
                            id: option_id.clone(),
                            rect: option_rect,
                            message: message.clone(),
                        });
                        if let Some(hit_rect) = node
                            .clip
                            .map(|clip| intersection(option_rect, clip))
                            .unwrap_or(Some(option_rect))
                        {
                            tree.hits.push(HitRegion {
                                id: option_id,
                                rect: hit_rect,
                                message: Some(message.clone()),
                                message_mapper: None,
                            });
                        }
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
        Kind::Flex(_) | Kind::Grid { .. } => {
            for (&child_index, child) in node.children.iter().zip(&element.children) {
                emit_element(child, child_index, foreground, tree);
            }
        }
    }
    if clips_descendants {
        tree.commands.push(PaintCommand::PopClip);
    }
}

fn layout_element<Message: Clone>(
    element: &Element<Message>,
    id: &UiId,
    bounds: Rect,
    inherited_foreground: Option<Color>,
    inherited_clip: Option<Rect>,
    tree: &mut UiTree<Message>,
) -> usize {
    let rect = bounds;
    let preferred = measure_element(element, Constraints::loose(bounds.size));
    let node_index = tree.resolved.nodes.len();
    let interaction = InteractionState {
        interactive: element.message.is_some() || element.text_mapper.is_some(),
        ..InteractionState::default()
    };
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
        children: Vec::new(),
    });
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
        && inherited_clip.is_some_and(|clip| intersection(rect, clip) != Some(rect))
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
            line_height,
            max_lines,
            ..
        } => Some(measure_text(
            value,
            *scale,
            *bold,
            false,
            *line_height,
            *max_lines,
            f32::INFINITY,
        )),
        Kind::Image { image, .. } => Some(Size::new(image.width() as f32, image.height() as f32)),
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
        Some(
            inherited_clip
                .and_then(|parent| intersection(parent, rect))
                .unwrap_or(rect),
        )
    } else {
        inherited_clip
    };
    match &element.kind {
        Kind::Text { .. } | Kind::Image { .. } | Kind::Slider { .. } | Kind::Dropdown { .. } => {}
        Kind::Flex(axis) => {
            let content = rect.inset(element.style.padding);
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
                            Constraints::loose(Size::new(f32::INFINITY, content.size.height)),
                        )
                        .width
                    })
                    .sum::<f32>()
                    + element.style.gap * element.children.len().saturating_sub(1) as f32
            } else {
                content.size.width
            };
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
                tree.scrolls.push(ScrollRegion {
                    id: id.clone(),
                    message: element.message.clone(),
                    offset_mapper: element.message_mapper,
                    rect: content,
                    extent,
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
                tree.scrolls.push(ScrollRegion {
                    id: id.clone(),
                    message: element.message.clone(),
                    offset_mapper: element.message_mapper,
                    rect: content,
                    extent,
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
            for (index, (child, bounds)) in element.children.iter().zip(child_bounds).enumerate() {
                let child_id = resolved_child_id(id, child, index);
                child_indices.push(layout_element(
                    child,
                    &child_id,
                    bounds,
                    foreground,
                    descendant_clip,
                    tree,
                ));
            }
        }
        Kind::VerticalScroll { offset } => {
            let requested_offset = *offset;
            let viewport = rect.inset(element.style.padding);
            let clip = descendant_clip
                .and_then(|parent| intersection(parent, viewport))
                .unwrap_or(viewport);
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
                if let Some(message) = &element.message {
                    tree.scrolls.push(ScrollRegion {
                        id: id.clone(),
                        message: Some(message.clone()),
                        offset_mapper: element.message_mapper,
                        rect: viewport,
                        extent,
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
            let content = rect.inset(element.style.padding);
            let measured = element
                .children
                .iter()
                .map(|child| measure_element(child, Constraints::loose(content.size)))
                .collect::<Vec<_>>();
            let widths = resolve_grid_columns(
                columns,
                content.size.width,
                element.style.gap,
                &measured,
                element.children.len(),
            );
            let column_count = widths.len().max(1);
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
                    rect: content,
                    extent,
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

fn apply_transient_state<Message>(
    element: &mut Element<Message>,
    id: &UiId,
    state: &mut UiStateStore,
) {
    let owns_state = element.message.is_some()
        || element.text_mapper.is_some()
        || matches!(element.style.overflow_x, Overflow::Scroll | Overflow::Auto)
        || matches!(element.style.overflow_y, Overflow::Scroll | Overflow::Auto)
        || matches!(
            element.kind,
            Kind::VerticalScroll { .. } | Kind::Dropdown { .. }
        );
    if owns_state {
        let (scroll_offset_x, scroll_offset, scroll_at_end, dropdown_open) = {
            let transient = state.touch(id.clone());
            (
                transient.scroll_offset_x,
                transient.scroll_offset,
                transient.scroll_at_end,
                transient.dropdown_open,
            )
        };
        match &mut element.kind {
            Kind::VerticalScroll { offset } => *offset = scroll_offset.max(0.0),
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
                line_height,
                ..
            } = &mut element.kind
        {
            let initial = value.clone();
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
                        measure_text(
                            &editor.text()[..end],
                            *scale,
                            *bold,
                            false,
                            None,
                            Some(1),
                            f32::INFINITY,
                        )
                        .width
                    };
                    (width(selection.start), width(selection.end))
                });
            *caret_position = (focused && caret_visible).then(|| {
                let prefix = editor.display_caret_prefix();
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
            *value = editor.display_text_with_caret("");
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
        widths[index] = base;
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
            .zip(&measured)
            .filter(|(child, _)| child.style.align_self.unwrap_or(align_items) == Align::Baseline)
            .map(|(_, size)| size.height * 0.8)
            .fold(0.0, f32::max)
    } else {
        0.0
    };
    for (index, ((rect, child), measured)) in
        rects.iter_mut().zip(children).zip(&measured).enumerate()
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
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum TestMessage {
        Named(&'static str),
        Option(usize),
        Volume(u8),
        Query(String),
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
            Grid::fixed(2).children([
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
            VerticalScroll::new(TestMessage::Named("scroll"), 50.0).child(
                Column::<TestMessage>::new()
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
            tree.scroll_extent(&TestMessage::Named("scroll")),
            Some(ScrollExtent {
                viewport: Size::new(200.0, 100.0),
                content: Size::new(200.0, 200.0),
                offset_x: 0.0,
                offset: 50.0,
            })
        );
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
        assert!(tree.accessibility_nodes().iter().all(|node| {
            node.rect.origin.y >= 0.0 && node.rect.origin.y + node.rect.size.height <= 100.0
        }));
    }

    #[test]
    fn vertical_scroll_clamps_offset_to_content_end() {
        let tree = UiTree::layout(
            VerticalScroll::new(TestMessage::Named("scroll"), 500.0).child(
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
            tree.scroll_extent(&TestMessage::Named("scroll"))
                .expect("scroll extent")
                .offset,
            30.0
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

    #[test]
    fn intrinsic_measurement_covers_empty_and_nested_flex_content() {
        let empty = Container::<()>::new()
            .padding(Insets::symmetric(8.0, 5.0))
            .into_element();
        assert_eq!(
            empty.measure(Constraints::unbounded()),
            Size::new(16.0, 10.0)
        );

        let nested = Column::<()>::new()
            .gap(4.0)
            .child(Container::new().width(30.0).height(10.0))
            .child(Container::new().width(50.0).height(20.0))
            .into_element();
        assert_eq!(
            nested.measure(Constraints::unbounded()),
            Size::new(50.0, 34.0)
        );

        let row = Row::<()>::new()
            .gap(5.0)
            .padding(Insets::all(2.0))
            .child(Container::new().width(20.0).height(8.0))
            .child(Container::new().width(30.0).height(12.0))
            .into_element();
        assert_eq!(row.measure(Constraints::unbounded()), Size::new(59.0, 16.0));

        let empty_grid = Grid::<()>::new().padding(Insets::all(3.0)).into_element();
        assert_eq!(
            empty_grid.measure(Constraints::unbounded()),
            Size::new(6.0, 6.0)
        );
    }

    #[test]
    fn multiline_button_grows_from_one_line_and_stops_at_its_line_limit() {
        let constraints = Constraints::loose(Size::new(180.0, f32::INFINITY));
        let short = Button::new((), "Short title")
            .max_lines(2)
            .into_element()
            .measure(constraints);
        let wrapped = Button::new((), "A task title long enough to wrap onto another line")
            .max_lines(2)
            .into_element()
            .measure(constraints);
        let much_longer = Button::new(
            (),
            "A task title long enough to wrap onto many more lines than the component permits",
        )
        .max_lines(2)
        .into_element()
        .measure(constraints);

        assert_eq!(short.height, 42.0);
        assert!(wrapped.height > short.height);
        assert_eq!(much_longer.height, wrapped.height);

        let label = "A task title that wraps onto its visible second line";
        let tree = UiTree::layout(
            Button::new((), label)
                .max_lines(2)
                .label_align(TextAlign::Start)
                .background(0x202630)
                .radius(10.0),
            Rect::new(0.0, 0.0, 180.0, 80.0),
        );
        assert!(tree.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Text { text, align: TextAlign::Start, .. } if text == label
        )));
        assert!(tree.commands().iter().any(|command| matches!(
            command,
            PaintCommand::RoundedFill { radius, .. } if *radius == 10.0
        )));
    }

    #[test]
    fn shaped_text_wraps_and_respects_line_height_and_max_lines() {
        let single = Text::<()>::new("The quick brown fox jumps over the lazy dog")
            .scale(2.0)
            .into_element()
            .measure(Constraints::loose(Size::new(500.0, f32::INFINITY)));
        let wrapped = Text::<()>::new("The quick brown fox jumps over the lazy dog")
            .scale(2.0)
            .wrap(true)
            .line_height(24.0)
            .max_lines(2)
            .into_element()
            .measure(Constraints::loose(Size::new(110.0, f32::INFINITY)));
        assert!(wrapped.width <= 110.0);
        assert!(wrapped.height > single.height);
        assert_eq!(wrapped.height, 48.0);

        let min_content = Text::<()>::new("short extraordinarily-long-word")
            .width_length(Length::MinContent)
            .into_element()
            .measure(Constraints::unbounded());
        let max_content = Text::<()>::new("short extraordinarily-long-word")
            .width_length(Length::MaxContent)
            .into_element()
            .measure(Constraints::unbounded());
        assert!(min_content.width < max_content.width);
    }

    #[test]
    fn constrained_image_preserves_intrinsic_aspect_ratio() {
        let image = Arc::new(RgbaImage::new(200, 100));
        let measured = Image::<()>::new(1, image)
            .width(80.0)
            .into_element()
            .measure(Constraints::unbounded());
        assert_eq!(measured, Size::new(80.0, 40.0));
    }

    #[test]
    fn unavailable_image_uses_bounded_fallback_and_reports_its_source() {
        let image = Arc::new(RgbaImage::new(0, 0));
        let element = Image::<()>::new(7, image).id("missing");
        assert_eq!(
            element
                .clone()
                .into_element()
                .measure(Constraints::unbounded()),
            Size::new(16.0, 16.0)
        );
        let tree = UiTree::layout_with_diagnostics(element, Rect::new(0.0, 0.0, 32.0, 32.0));
        assert!(tree.diagnostics().iter().any(|diagnostic| {
            diagnostic.kind == DiagnosticKind::MissingAsset
                && diagnostic.id == UiId::from("root/missing")
        }));
    }

    #[test]
    fn ellipsis_changes_presentation_without_entering_message_payloads() {
        let tree = UiTree::<TestMessage>::layout(
            Text::new("A deliberately long label")
                .width(45.0)
                .ellipsis(true),
            Rect::new(0.0, 0.0, 45.0, 24.0),
        );
        assert!(tree.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text.ends_with('…'))
        ));
    }

    #[test]
    fn flex_alignment_justification_and_limits_resolve_deterministically() {
        let tree = UiTree::layout(
            Row::new()
                .align_items(Align::Center)
                .justify_content(Justify::SpaceBetween)
                .child(
                    Container::new()
                        .width(80.0)
                        .height(20.0)
                        .min_width(70.0)
                        .shrink(1.0)
                        .message(TestMessage::Named("left")),
                )
                .child(
                    Container::new()
                        .width(80.0)
                        .height(10.0)
                        .min_width(20.0)
                        .shrink(1.0)
                        .message(TestMessage::Named("right")),
                ),
            Rect::new(0.0, 0.0, 120.0, 40.0),
        );
        let left = tree
            .message_layout_rect(&TestMessage::Named("left"))
            .unwrap();
        let right = tree
            .message_layout_rect(&TestMessage::Named("right"))
            .unwrap();
        assert_eq!(left.size.width, 70.0);
        assert_eq!(right.size.width, 50.0);
        assert_eq!(left.origin.y, 10.0);
        assert_eq!(right.origin.y, 15.0);
    }

    #[test]
    fn every_justification_and_cross_axis_alignment_mode_is_finite() {
        for justify in [
            Justify::Start,
            Justify::Center,
            Justify::End,
            Justify::SpaceBetween,
            Justify::SpaceAround,
            Justify::SpaceEvenly,
        ] {
            for align in [
                Align::Start,
                Align::Center,
                Align::End,
                Align::Stretch,
                Align::Baseline,
            ] {
                let tree = UiTree::layout(
                    Row::new()
                        .justify_content(justify)
                        .align_items(align)
                        .child(
                            Button::new(TestMessage::Option(0), "A")
                                .width(30.0)
                                .height(20.0),
                        )
                        .child(
                            Button::new(TestMessage::Option(1), "B")
                                .width(30.0)
                                .height(10.0),
                        ),
                    Rect::new(0.0, 0.0, 120.0, 40.0),
                );
                for message in [TestMessage::Option(0), TestMessage::Option(1)] {
                    let rect = tree.message_layout_rect(&message).unwrap();
                    assert!(rect.origin.x.is_finite() && rect.origin.y.is_finite());
                    assert!(rect.size.width >= 0.0 && rect.size.height >= 0.0);
                }
            }
        }
    }

    #[test]
    fn grid_resolves_fixed_auto_fractional_repeated_and_auto_fit_tracks() {
        let fixed = UiTree::layout(
            Grid::tracks([Track::px(40.0), Track::Auto, Track::fr(1.0)]).children([
                Button::new(TestMessage::Option(0), "A"),
                Button::new(TestMessage::Option(1), "A much wider label"),
                Button::new(TestMessage::Option(2), "C"),
            ]),
            Rect::new(0.0, 0.0, 300.0, 60.0),
        );
        assert_eq!(fixed.resolved_grid_columns(), Some(3));
        let repeated = UiTree::layout(
            Grid::tracks([Track::repeat(2, Track::fr(1.0))]).children([
                Button::new(TestMessage::Option(0), "A"),
                Button::new(TestMessage::Option(1), "B"),
            ]),
            Rect::new(0.0, 0.0, 200.0, 60.0),
        );
        assert_eq!(repeated.resolved_grid_columns(), Some(2));
        let auto_fit = UiTree::layout(
            Grid::new()
                .columns(Track::repeat_auto_fit(Track::minmax(80.0, Track::fr(1.0))))
                .children([
                    Button::new(TestMessage::Option(0), "A"),
                    Button::new(TestMessage::Option(1), "B"),
                    Button::new(TestMessage::Option(2), "C"),
                ]),
            Rect::new(0.0, 0.0, 250.0, 120.0),
        );
        assert_eq!(auto_fit.resolved_grid_columns(), Some(3));
    }

    #[test]
    fn generated_valid_layouts_have_finite_nonnegative_geometry() {
        for width in [0.0, 1.0, 37.0, 400.0] {
            for height in [0.0, 1.0, 91.0, 300.0] {
                let tree = UiTree::layout(
                    Row::new()
                        .gap(3.0)
                        .child(Button::new(TestMessage::Option(0), "A").min_width(0.0))
                        .child(Button::new(TestMessage::Option(1), "B").min_width(0.0)),
                    Rect::new(0.0, 0.0, width, height),
                );
                for message in [TestMessage::Option(0), TestMessage::Option(1)] {
                    let rect = tree.message_layout_rect(&message).unwrap();
                    assert!(rect.origin.x.is_finite() && rect.origin.y.is_finite());
                    assert!(rect.size.width.is_finite() && rect.size.height.is_finite());
                    assert!(rect.size.width >= 0.0 && rect.size.height >= 0.0);
                }
            }
        }
    }

    #[test]
    fn explicit_identity_survives_sibling_insertion_and_list_reordering() {
        let first = UiTree::layout(
            Column::new().id("list").children([
                Button::new(TestMessage::Option(1), "One").id("one"),
                Button::new(TestMessage::Option(2), "Two").id("two"),
            ]),
            Rect::new(0.0, 0.0, 200.0, 100.0),
        );
        let reordered = UiTree::layout(
            Column::new().id("list").children([
                Button::new(TestMessage::Option(3), "New").id("new"),
                Button::new(TestMessage::Option(2), "Two").id("two"),
                Button::new(TestMessage::Option(1), "One").id("one"),
            ]),
            Rect::new(0.0, 0.0, 200.0, 120.0),
        );
        assert_eq!(
            first.id_for_message(&TestMessage::Option(1)),
            reordered.id_for_message(&TestMessage::Option(1))
        );
        assert_eq!(
            first.id_for_message(&TestMessage::Option(2)),
            reordered.id_for_message(&TestMessage::Option(2))
        );
    }

    #[test]
    fn pointer_keyboard_controller_and_accessibility_share_typed_activation() {
        let tree = UiTree::layout(
            Button::new(TestMessage::Option(7), "Seven").id("seven"),
            Rect::new(0.0, 0.0, 100.0, 42.0),
        );
        let id = tree
            .id_for_message(&TestMessage::Option(7))
            .unwrap()
            .clone();
        let mut state = UiStateStore::default();
        tree.reconcile_state(&mut state);

        tree.handle_event(
            &mut state,
            UiEvent::PointerPressed(Point { x: 10.0, y: 10.0 }),
        );
        assert_eq!(
            tree.handle_event(
                &mut state,
                UiEvent::PointerReleased(Point { x: 10.0, y: 10.0 })
            )
            .messages,
            vec![TestMessage::Option(7)]
        );
        tree.handle_event(&mut state, UiEvent::FocusNext);
        for event in [UiEvent::KeyboardActivate, UiEvent::ControllerActivate] {
            assert_eq!(
                tree.handle_event(&mut state, event).messages,
                vec![TestMessage::Option(7)]
            );
        }
        assert_eq!(
            tree.handle_event(&mut state, UiEvent::AccessibilityActivate(id))
                .messages,
            vec![TestMessage::Option(7)]
        );

        let controller_tree = || {
            UiTree::layout(
                Row::new().children([
                    Button::new(TestMessage::Option(1), "One").id("one"),
                    Button::new(TestMessage::Option(2), "Two").id("two"),
                ]),
                Rect::new(0.0, 0.0, 200.0, 42.0),
            )
        };
        let first = controller_tree();
        first.handle_event(&mut state, UiEvent::ControllerNext);
        first.handle_event(&mut state, UiEvent::ControllerNext);
        let selected = state.controller_selected().cloned();
        let rebuilt = controller_tree();
        assert_eq!(state.controller_selected(), selected.as_ref());
        assert_eq!(
            rebuilt
                .handle_event(&mut state, UiEvent::ControllerActivate)
                .messages,
            vec![TestMessage::Option(2)]
        );
    }

    #[test]
    fn pointer_drag_selects_visible_text_and_caret_blink_only_changes_paint() {
        fn query(value: String) -> TestMessage {
            TestMessage::Query(value)
        }

        let mut state = UiStateStore::default();
        let first = UiTree::layout_with_state(
            TextField::on_change("select this", query).id("query"),
            Rect::new(0.0, 0.0, 200.0, 32.0),
            &mut state,
        );
        first.handle_event(
            &mut state,
            UiEvent::PointerPressed(Point { x: 2.0, y: 10.0 }),
        );
        first.handle_event(
            &mut state,
            UiEvent::PointerMoved(Point { x: 70.0, y: 10.0 }),
        );
        let selected = state
            .state(&UiId::from("root/query"))
            .and_then(|entry| entry.editor.as_ref())
            .and_then(TextEditor::selection);
        assert!(selected.is_some_and(|range| !range.is_empty()));

        let selected_tree = UiTree::layout_with_state(
            TextField::on_change("select this", query).id("query"),
            Rect::new(0.0, 0.0, 200.0, 32.0),
            &mut state,
        );
        assert!(selected_tree.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Fill {
                color: 0x315a8f,
                ..
            }
        )));
        assert_eq!(
            selected_tree
                .handle_event(&mut state, UiEvent::CaretBlink)
                .invalidation,
            Invalidation::Paint
        );
    }

    #[test]
    fn document_selection_crosses_text_runs_and_skips_buttons() {
        let build = |state: &mut UiStateStore| {
            UiTree::layout_with_state(
                SelectionRegion::automatic().id("document").child(
                    Column::new()
                        .gap(8.0)
                        .child(
                            Text::new("First")
                                .id("first")
                                .selection_boundary(TextBoundary::Block),
                        )
                        .child(Button::new(TestMessage::Named("button"), "Excluded").id("button"))
                        .child(
                            Text::new("Second")
                                .id("second")
                                .selection_boundary(TextBoundary::Block),
                        ),
                ),
                Rect::new(0.0, 0.0, 240.0, 140.0),
                state,
            )
        };

        let mut state = UiStateStore::default();
        let tree = build(&mut state);
        let first = tree
            .resolved_layout()
            .find(&UiId::from("root/document/#0/first"))
            .expect("first text")
            .allocated;
        let second = tree
            .resolved_layout()
            .find(&UiId::from("root/document/#0/second"))
            .expect("second text")
            .allocated;
        tree.handle_event(
            &mut state,
            UiEvent::PointerPressed(Point {
                x: first.origin.x + 1.0,
                y: first.origin.y + first.size.height * 0.5,
            }),
        );
        tree.handle_event(
            &mut state,
            UiEvent::PointerMoved(Point {
                x: second.origin.x + second.size.width - 1.0,
                y: second.origin.y + second.size.height * 0.5,
            }),
        );
        let release = tree.handle_event(
            &mut state,
            UiEvent::PointerReleased(Point {
                x: second.origin.x + second.size.width - 1.0,
                y: second.origin.y + second.size.height * 0.5,
            }),
        );
        assert!(release.messages.is_empty());
        assert_eq!(tree.selected_text(&state).as_deref(), Some("First\nSecond"));
        tree.handle_event(&mut state, UiEvent::TextSelectAll);
        let copied = tree.handle_event(&mut state, UiEvent::TextCopy);
        assert_eq!(copied.clipboard_text.as_deref(), Some("First\nSecond"));
        tree.handle_event(
            &mut state,
            UiEvent::TextMoveLeft {
                extend_selection: true,
            },
        );
        assert_eq!(tree.selected_text(&state).as_deref(), Some("First\nSecon"));
        tree.handle_event(&mut state, UiEvent::SelectionClear);
        assert!(
            tree.handle_event(&mut state, UiEvent::TextCopy)
                .clipboard_text
                .is_none()
        );
        tree.handle_event(
            &mut state,
            UiEvent::PointerPressed(Point {
                x: first.origin.x + 1.0,
                y: first.origin.y + first.size.height * 0.5,
            }),
        );
        tree.handle_event(&mut state, UiEvent::TextSelectAll);
        let cut = tree.handle_event(&mut state, UiEvent::TextCut);
        assert!(cut.clipboard_text.is_none());
        assert!(cut.messages.is_empty());

        let selected = build(&mut state);
        assert!(
            selected
                .commands()
                .iter()
                .filter(|command| matches!(
                    command,
                    PaintCommand::Fill {
                        color: 0x315a8f,
                        ..
                    }
                ))
                .count()
                >= 2
        );

        let button = selected
            .message_rect(&TestMessage::Named("button"))
            .expect("button bounds");
        selected.handle_event(
            &mut state,
            UiEvent::PointerPressed(Point {
                x: button.origin.x + 2.0,
                y: button.origin.y + 2.0,
            }),
        );
        let activated = selected.handle_event(
            &mut state,
            UiEvent::PointerReleased(Point {
                x: button.origin.x + 2.0,
                y: button.origin.y + 2.0,
            }),
        );
        assert_eq!(activated.messages, vec![TestMessage::Named("button")]);
        assert!(state.selection_owner().is_none());
    }

    #[test]
    fn document_drag_edge_autoscrolls_at_a_bounded_rate() {
        let mut state = UiStateStore::default();
        let tree =
            UiTree::layout_with_state(
                Column::<TestMessage>::new()
                    .id("scroll")
                    .height(90.0)
                    .overflow_y(Overflow::Auto)
                    .child(SelectionRegion::automatic().id("document").child(
                        Column::new().children((0..30).map(|index| {
                            Text::new(format!("Line {index}"))
                                .selection_boundary(TextBoundary::Block)
                        })),
                    )),
                Rect::new(0.0, 0.0, 240.0, 90.0),
                &mut state,
            );
        let first = tree
            .selection_regions
            .first()
            .and_then(|region| region.runs.first())
            .and_then(|run| run.glyphs.first())
            .expect("visible selectable glyph")
            .rect;
        tree.handle_event(
            &mut state,
            UiEvent::PointerPressed(Point {
                x: first.origin.x + 1.0,
                y: first.origin.y + 1.0,
            }),
        );
        for _ in 0..4 {
            let outcome = tree.handle_event(
                &mut state,
                UiEvent::PointerMoved(Point { x: 20.0, y: 89.0 }),
            );
            assert!(matches!(outcome.invalidation, Invalidation::Layout));
        }
        let offset = state
            .state(&UiId::from("root/scroll"))
            .expect("scroll state")
            .scroll_offset;
        assert!(offset > 0.0);
        assert!(offset <= 72.0);
    }

    #[test]
    fn structured_diagnostics_are_bounded_deduplicated_and_attributed() {
        let tree = UiTree::layout_with_diagnostics(
            Column::new()
                .id("fixture")
                .min_width(200.0)
                .max_width(20.0)
                .child(
                    Row::new()
                        .id("overflow")
                        .overflow(Overflow::Clip, Overflow::Clip)
                        .child(
                            Button::new(TestMessage::Option(1), "wide")
                                .id("duplicate")
                                .width(180.0)
                                .min_width(180.0)
                                .shrink(0.0),
                        )
                        .child(
                            Button::new(TestMessage::Option(2), "also wide")
                                .id("duplicate")
                                .width(180.0)
                                .min_width(180.0)
                                .shrink(0.0),
                        ),
                )
                .child(
                    VerticalScroll::new(TestMessage::Named("scroll"), 500.0)
                        .id("scroll")
                        .height(30.0)
                        .child(Text::new("short")),
                )
                .child(Text::new("cannot fit").id("text").width(1.0).height(1.0)),
            Rect::new(0.0, 0.0, 100.0, 80.0),
        );
        for expected in [
            DiagnosticKind::ContradictoryConstraints,
            DiagnosticKind::FlexOverflow,
            DiagnosticKind::DuplicateIdentity,
            DiagnosticKind::ClippedInteraction,
            DiagnosticKind::ScrollOffsetClamped,
            DiagnosticKind::UnsatisfiedContent,
        ] {
            assert!(
                tree.diagnostics().iter().any(|item| item.kind == expected),
                "missing {expected:?}: {:?}",
                tree.diagnostics()
            );
        }
        let unique = tree
            .diagnostics()
            .iter()
            .map(|item| (item.kind, item.id.clone()))
            .collect::<HashSet<_>>();
        assert_eq!(unique.len(), tree.diagnostics().len());
        assert!(tree.diagnostics().len() <= 128);
        assert!(
            tree.diagnostics()
                .iter()
                .all(|item| item.id.as_str().starts_with("root/fixture"))
        );
    }

    #[test]
    fn invalid_indefinite_and_unbalanced_layout_conditions_are_reported() {
        let tree = UiTree::layout_with_diagnostics(
            Text::<TestMessage>::new("percent").width_length(Length::percent(0.5)),
            Rect::new(0.0, 0.0, f32::INFINITY, 30.0),
        );
        assert!(
            tree.diagnostics()
                .iter()
                .any(|item| item.kind == DiagnosticKind::InvalidGeometry)
        );
        assert!(
            tree.diagnostics()
                .iter()
                .any(|item| item.kind == DiagnosticKind::IndefinitePercentage)
        );

        let mut malformed = UiTree::<TestMessage> {
            diagnostics_enabled: true,
            ..UiTree::default()
        };
        malformed.commands.push(PaintCommand::PopClip);
        malformed.validate_clip_commands();
        assert_eq!(
            malformed.diagnostics()[0].kind,
            DiagnosticKind::UnbalancedClip
        );
    }

    #[test]
    fn resolved_layout_is_headless_deterministic_and_overlay_is_non_interfering() {
        let build = || {
            UiTree::layout_with_diagnostics(
                Row::new()
                    .id("toolbar")
                    .padding(Insets::all(4.0))
                    .child(Button::new(TestMessage::Named("save"), "Save").id("save")),
                Rect::new(0.0, 0.0, 240.0, 48.0),
            )
        };
        let mut tree = build();
        let before_snapshot = tree.resolved_layout().deterministic_snapshot();
        let before_hit = tree.message_at(Point { x: 20.0, y: 20.0 }).cloned();
        assert!(
            tree.resolved_layout()
                .find(&UiId::from("root/toolbar/save"))
                .is_some()
        );
        tree.enable_diagnostic_overlay(None);
        assert_eq!(
            before_snapshot,
            tree.resolved_layout().deterministic_snapshot()
        );
        assert_eq!(
            before_hit,
            tree.message_at(Point { x: 20.0, y: 20.0 }).cloned()
        );
        assert_eq!(
            before_snapshot,
            build().resolved_layout().deterministic_snapshot()
        );
        assert!(
            UiTree::layout(
                Text::<TestMessage>::new("disabled"),
                Rect::new(0.0, 0.0, 10.0, 10.0)
            )
            .diagnostics()
            .is_empty()
        );
        let disabled = UiTree::layout(
            Column::<TestMessage>::new()
                .children((0..128).map(|index| Text::new(index.to_string()))),
            Rect::new(0.0, 0.0, 200.0, 400.0),
        );
        assert!(disabled.seen_ids.is_empty());
        assert!(disabled.diagnostic_keys.is_empty());
    }

    #[test]
    fn diagnostic_overlay_rasterizes_at_low_and_high_dpi_without_geometry_changes() {
        let build = || {
            UiTree::layout_with_diagnostics(
                Grid::new()
                    .id("scale-fixture")
                    .columns(Track::repeat_auto_fit(Track::minmax(60.0, Track::fr(1.0))))
                    .children((0..6).map(|index| {
                        Button::new(TestMessage::Option(index), format!("item {index}")).id(index)
                    })),
                Rect::new(0.0, 0.0, 240.0, 120.0),
            )
        };
        let baseline = build().resolved_layout().deterministic_snapshot();
        for scale in [1.0, 2.0] {
            let mut tree = build();
            tree.enable_diagnostic_overlay_with_damage(None, &[Rect::new(12.0, 8.0, 80.0, 32.0)]);
            let mut renderer = crate::gpu::SdlComponentRenderer::new(
                (240.0 * scale) as u32,
                (120.0 * scale) as u32,
                scale,
            );
            assert!(!renderer.render(tree.commands()).is_empty());
            assert!(renderer.pixels().iter().any(|pixel| pixel.a > 0));
            assert_eq!(baseline, tree.resolved_layout().deterministic_snapshot());
            assert_eq!(
                tree.message_at(Point { x: 30.0, y: 20.0 }),
                Some(&TestMessage::Option(0))
            );
        }
    }

    #[test]
    fn stateful_layout_rehydrates_dropdown_scroll_focus_and_pointer_capture() {
        let mut state = UiStateStore::default();
        let dropdown = || {
            Dropdown::new(
                TestMessage::Named("toggle"),
                "One",
                [("Two", TestMessage::Option(2))],
            )
            .id("choice")
        };
        let first =
            UiTree::layout_with_state(dropdown(), Rect::new(0.0, 0.0, 180.0, 120.0), &mut state);
        first.handle_event(
            &mut state,
            UiEvent::PointerPressed(Point { x: 10.0, y: 10.0 }),
        );
        assert!(state.captured().is_some());
        let released = first.handle_event(
            &mut state,
            UiEvent::PointerReleased(Point { x: 10.0, y: 10.0 }),
        );
        assert_eq!(released.messages, vec![TestMessage::Named("toggle")]);
        assert_eq!(released.invalidation, Invalidation::Layout);
        assert!(state.captured().is_none());
        assert!(state.focused().is_some());
        let rebuilt =
            UiTree::layout_with_state(dropdown(), Rect::new(0.0, 0.0, 180.0, 120.0), &mut state);
        assert!(
            rebuilt
                .message_for_id(&UiId::from("root/choice/option-0"))
                .is_some()
        );

        let scroll = |state: &mut UiStateStore| {
            UiTree::layout_with_state(
                VerticalScroll::new(TestMessage::Named("scroll"), 0.0)
                    .id("list")
                    .child(Column::new().children((0..8).map(|index| {
                        Text::<TestMessage>::new(format!("row {index}")).height(24.0)
                    }))),
                Rect::new(0.0, 0.0, 120.0, 48.0),
                state,
            )
        };
        let initial = scroll(&mut state);
        assert_eq!(
            initial
                .handle_event(
                    &mut state,
                    UiEvent::Scroll {
                        point: Point { x: 10.0, y: 10.0 },
                        delta_y: 30.0,
                    },
                )
                .invalidation,
            Invalidation::Layout
        );
        assert_eq!(
            scroll(&mut state)
                .scroll_extent(&TestMessage::Named("scroll"))
                .expect("scroll extent")
                .offset,
            30.0
        );
    }

    #[test]
    fn application_menu_overlays_content_selects_items_and_dismisses() {
        let build = |state: &mut UiStateStore| {
            UiTree::layout_with_state(
                Column::new()
                    .child(
                        MenuBar::new().id("bar").child(
                            Menu::new(
                                TestMessage::Named("toggle"),
                                "File",
                                [
                                    MenuItem::new("New", TestMessage::Named("new")),
                                    MenuItem::disabled("Unavailable"),
                                ],
                            )
                            .id("file"),
                        ),
                    )
                    .child(Container::new().id("body").height(100.0)),
                Rect::new(0.0, 0.0, 240.0, 160.0),
                state,
            )
        };
        let mut state = UiStateStore::default();
        let closed = build(&mut state);
        let file_label = closed
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCommand::Text { bounds, text, .. } if text == "File" => Some(*bounds),
                _ => None,
            })
            .expect("menu header label");
        assert!(file_label.size.height >= 20.0);
        assert!(file_label.origin.y + file_label.size.height <= 30.0);
        let body_y = closed
            .resolved_layout()
            .find(&UiId::from("root/body"))
            .unwrap()
            .allocated
            .origin
            .y;
        closed.handle_event(
            &mut state,
            UiEvent::PointerPressed(Point { x: 10.0, y: 10.0 }),
        );
        closed.handle_event(
            &mut state,
            UiEvent::PointerReleased(Point { x: 10.0, y: 10.0 }),
        );

        let open = build(&mut state);
        assert_eq!(
            open.resolved_layout()
                .find(&UiId::from("root/body"))
                .unwrap()
                .allocated
                .origin
                .y,
            body_y
        );
        assert_eq!(
            open.message_at(Point { x: 10.0, y: 47.0 }),
            Some(&TestMessage::Named("new"))
        );
        assert_eq!(open.message_at(Point { x: 10.0, y: 81.0 }), None);
        open.handle_event(
            &mut state,
            UiEvent::PointerPressed(Point { x: 10.0, y: 47.0 }),
        );
        let selected = open.handle_event(
            &mut state,
            UiEvent::PointerReleased(Point { x: 10.0, y: 47.0 }),
        );
        assert_eq!(selected.messages, vec![TestMessage::Named("new")]);
        assert!(
            !state
                .state(&UiId::from("root/bar/file"))
                .unwrap()
                .dropdown_open
        );

        let reopened = build(&mut state);
        reopened.handle_event(
            &mut state,
            UiEvent::PointerPressed(Point { x: 10.0, y: 10.0 }),
        );
        reopened.handle_event(
            &mut state,
            UiEvent::PointerReleased(Point { x: 10.0, y: 10.0 }),
        );
        build(&mut state).handle_event(&mut state, UiEvent::Dismiss);
        assert!(
            !state
                .state(&UiId::from("root/bar/file"))
                .unwrap()
                .dropdown_open
        );
    }

    #[test]
    fn text_field_focus_edit_selection_and_ime_survive_reconstruction_without_click_messages() {
        fn query(value: String) -> TestMessage {
            TestMessage::Query(value)
        }

        let build = |state: &mut UiStateStore, value: &str| {
            UiTree::layout_with_state(
                TextField::on_change(value, query).id("query"),
                Rect::new(0.0, 0.0, 180.0, 32.0),
                state,
            )
        };
        let mut state = UiStateStore::default();
        let first = build(&mut state, "nickel");
        first.handle_event(
            &mut state,
            UiEvent::PointerPressed(Point { x: 10.0, y: 10.0 }),
        );
        assert_eq!(
            first
                .handle_event(
                    &mut state,
                    UiEvent::PointerReleased(Point { x: 10.0, y: 10.0 }),
                )
                .messages,
            Vec::<TestMessage>::new()
        );
        let id = UiId::from("root/query");
        assert_eq!(state.focused(), Some(&id));
        state.editor(id.clone(), "nickel").select_all();
        let changed = first
            .handle_event(&mut state, UiEvent::TextInput("silver".into()))
            .messages;
        assert_eq!(changed, vec![TestMessage::Query("silver".into())]);
        first.handle_event(&mut state, UiEvent::ImePreedit("世界".into()));
        let rebuilt = build(&mut state, "silver");
        assert!(rebuilt.commands().iter().any(|command| {
            matches!(command, PaintCommand::Text { text, .. } if text.contains("世界"))
        }));
        assert_eq!(
            state
                .state(&id)
                .and_then(|transient| transient.editor.as_ref())
                .expect("retained editor")
                .selection(),
            None
        );

        let externally_cleared = build(&mut state, "");
        assert!(externally_cleared.commands().iter().any(
            |command| matches!(command, PaintCommand::Fill { rect, .. } if rect.size.width == 1.5)
        ));
        assert_eq!(
            state
                .state(&id)
                .and_then(|transient| transient.editor.as_ref())
                .expect("controlled editor")
                .text(),
            ""
        );
    }

    #[test]
    fn focused_text_field_supports_navigation_selection_and_deletion_messages() {
        fn query(value: String) -> TestMessage {
            TestMessage::Query(value)
        }

        let mut state = UiStateStore::default();
        let tree = UiTree::layout_with_state(
            TextField::on_change("abc", query).id("query"),
            Rect::new(0.0, 0.0, 180.0, 32.0),
            &mut state,
        );
        tree.handle_event(&mut state, UiEvent::FocusNext);
        assert_eq!(
            tree.handle_event(
                &mut state,
                UiEvent::TextMoveLeft {
                    extend_selection: true,
                },
            )
            .messages,
            vec![TestMessage::Query("abc".into())]
        );
        assert_eq!(
            tree.handle_event(&mut state, UiEvent::TextBackspace)
                .messages,
            vec![TestMessage::Query("ab".into())]
        );
        tree.handle_event(&mut state, UiEvent::TextSelectAll);
        assert_eq!(
            tree.handle_event(&mut state, UiEvent::TextInput("replacement".into()))
                .messages,
            vec![TestMessage::Query("replacement".into())]
        );
        tree.handle_event(
            &mut state,
            UiEvent::TextMoveHome {
                extend_selection: false,
            },
        );
        assert_eq!(
            tree.handle_event(&mut state, UiEvent::TextDelete).messages,
            vec![TestMessage::Query("eplacement".into())]
        );
    }

    #[test]
    fn focused_text_field_copies_cuts_and_pastes_selection() {
        fn query(value: String) -> TestMessage {
            TestMessage::Query(value)
        }

        let mut state = UiStateStore::default();
        let tree = UiTree::layout_with_state(
            TextField::on_change("copy me", query).id("query"),
            Rect::new(0.0, 0.0, 180.0, 32.0),
            &mut state,
        );
        tree.handle_event(&mut state, UiEvent::FocusNext);
        tree.handle_event(&mut state, UiEvent::TextSelectAll);

        let copied = tree.handle_event(&mut state, UiEvent::TextCopy);
        assert_eq!(copied.clipboard_text.as_deref(), Some("copy me"));
        assert!(copied.messages.is_empty());

        let cut = tree.handle_event(&mut state, UiEvent::TextCut);
        assert_eq!(cut.clipboard_text.as_deref(), Some("copy me"));
        assert_eq!(cut.messages, vec![TestMessage::Query(String::new())]);

        let pasted = tree.handle_event(&mut state, UiEvent::TextPaste("世界".into()));
        assert_eq!(pasted.messages, vec![TestMessage::Query("世界".into())]);
        let empty_cut = tree.handle_event(&mut state, UiEvent::TextCut);
        assert!(empty_cut.clipboard_text.is_none());
        assert!(empty_cut.messages.is_empty());
    }

    #[test]
    fn multiline_text_field_hit_testing_and_caret_follow_explicit_lines() {
        fn query(value: String) -> TestMessage {
            TestMessage::Query(value)
        }

        let mut state = UiStateStore::default();
        let tree = UiTree::layout_with_state(
            TextField::on_change("one\ntwo", query)
                .id("query")
                .wrap(true),
            Rect::new(0.0, 0.0, 200.0, 80.0),
            &mut state,
        );
        tree.handle_event(
            &mut state,
            UiEvent::PointerPressed(Point { x: 100.0, y: 28.0 }),
        );
        let inserted = tree.handle_event(&mut state, UiEvent::TextInput("!".into()));
        assert_eq!(
            inserted.messages,
            vec![TestMessage::Query("one\ntwo!".into())]
        );

        let rebuilt = UiTree::layout_with_state(
            TextField::on_change("one\ntwo!", query)
                .id("query")
                .wrap(true),
            Rect::new(0.0, 0.0, 200.0, 80.0),
            &mut state,
        );
        assert!(rebuilt.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Fill { rect, .. }
                if (rect.size.width - 1.5).abs() < f32::EPSILON && rect.origin.y > 10.0
        )));
    }

    #[test]
    fn ordinary_auto_overflow_owns_scroll_state_and_clips_descendants() {
        let build = |state: &mut UiStateStore| {
            UiTree::layout_with_state(
                Column::new()
                    .id("automatic")
                    .height(48.0)
                    .overflow(Overflow::Clip, Overflow::Auto)
                    .children((0..4).map(|index| {
                        Button::new(TestMessage::Option(index), format!("row {index}"))
                            .id(format!("row-{index}"))
                            .height(24.0)
                            .shrink(0.0)
                    })),
                Rect::new(0.0, 0.0, 120.0, 48.0),
                state,
            )
        };
        let mut state = UiStateStore::default();
        let initial = build(&mut state);
        let node = initial
            .resolved_layout()
            .find(&UiId::from("root/automatic"))
            .expect("resolved automatic overflow container");
        assert_eq!(node.scroll.expect("scroll metadata").content.height, 96.0);
        assert_eq!(
            initial.message_at(Point { x: 10.0, y: 60.0 }),
            None,
            "hit regions outside the viewport are clipped"
        );
        assert_eq!(
            initial
                .handle_event(
                    &mut state,
                    UiEvent::Scroll {
                        point: Point { x: 10.0, y: 10.0 },
                        delta_y: 30.0,
                    },
                )
                .invalidation,
            Invalidation::Layout
        );
        let scrolled = build(&mut state);
        assert_eq!(
            scrolled
                .resolved_layout()
                .find(&UiId::from("root/automatic"))
                .and_then(|node| node.scroll)
                .expect("scroll metadata")
                .offset,
            30.0
        );
        assert_eq!(
            scrolled.message_at(Point { x: 10.0, y: 30.0 }),
            Some(&TestMessage::Option(2))
        );

        state.touch(UiId::from("root/automatic")).scroll_offset = 200.0;
        let clamped = build(&mut state);
        assert_eq!(
            clamped
                .resolved_layout()
                .find(&UiId::from("root/automatic"))
                .and_then(|node| node.scroll)
                .expect("scroll metadata")
                .offset,
            48.0
        );
        assert_eq!(
            state
                .state(&UiId::from("root/automatic"))
                .expect("retained state")
                .scroll_offset,
            48.0
        );
    }

    #[test]
    fn follow_scroll_end_pins_growth_until_the_user_scrolls_up() {
        let build = |state: &mut UiStateStore, rows: usize| {
            UiTree::layout_with_state(
                Column::<TestMessage>::new()
                    .id("conversation")
                    .height(60.0)
                    .overflow_y(Overflow::Auto)
                    .follow_scroll_end(true)
                    .children((0..rows).map(|index| {
                        Container::new()
                            .id(index)
                            .height(30.0)
                            .child(Text::new(format!("row {index}")))
                    })),
                Rect::new(0.0, 0.0, 200.0, 60.0),
                state,
            )
        };
        let id = UiId::from("root").scoped("conversation");
        let mut state = UiStateStore::default();
        let initial = build(&mut state, 4);
        let initial_extent = initial.resolved_layout().nodes()[0]
            .scroll
            .expect("scroll extent");
        assert_eq!(initial_extent.offset, 60.0);

        state.scroll_by(id.clone(), -30.0, 60.0);
        let anchored = build(&mut state, 5);
        let anchored_extent = anchored.resolved_layout().nodes()[0]
            .scroll
            .expect("scroll extent");
        assert_eq!(anchored_extent.offset, 30.0);

        state.scroll_by(id, 100.0, 90.0);
        let followed = build(&mut state, 6);
        let followed_extent = followed.resolved_layout().nodes()[0]
            .scroll
            .expect("scroll extent");
        assert_eq!(followed_extent.offset, 120.0);
    }

    #[test]
    fn virtual_window_bounds_variable_height_work_at_start_middle_and_end() {
        let heights = [100.0, 200.0, 300.0];
        let start = VirtualWindow::from_heights(&heights, 10.0, 0.0, 100.0, 0.0);
        assert_eq!(start.range, 0..1);
        assert_eq!(
            (start.leading, start.trailing, start.total),
            (0.0, 520.0, 620.0)
        );

        let middle = VirtualWindow::from_heights(&heights, 10.0, 120.0, 100.0, 0.0);
        assert_eq!(middle.range, 1..2);
        assert_eq!((middle.leading, middle.trailing), (110.0, 310.0));

        let end = VirtualWindow::from_heights(&heights, 10.0, f32::MAX, 100.0, 0.0);
        assert_eq!(end.range, 2..3);
        assert_eq!((end.leading, end.trailing), (320.0, 0.0));

        let empty = VirtualWindow::from_heights(&[], 10.0, 0.0, 100.0, 100.0);
        assert_eq!(empty.range, 0..0);
        assert_eq!(empty.total, 0.0);
    }

    #[test]
    fn vertical_scroll_emits_the_resulting_offset() {
        fn scrolled(offset: f32) -> TestMessage {
            TestMessage::Volume(offset.round() as u8)
        }

        let mut state = UiStateStore::default();
        let tree = UiTree::layout_with_state(
            Column::new()
                .id("scroll")
                .height(60.0)
                .overflow_y(Overflow::Auto)
                .on_scroll(scrolled)
                .children((0..4).map(|_| Spacer::vertical(30.0))),
            Rect::new(0.0, 0.0, 200.0, 60.0),
            &mut state,
        );
        let outcome = tree.handle_event(
            &mut state,
            UiEvent::Scroll {
                point: Point { x: 10.0, y: 10.0 },
                delta_y: 30.0,
            },
        );
        assert_eq!(outcome.messages, vec![TestMessage::Volume(30)]);
    }

    #[test]
    fn horizontal_overflow_policy_scrolls_and_clips_on_its_own_axis() {
        let build = |state: &mut UiStateStore| {
            UiTree::layout_with_state(
                Row::new()
                    .id("horizontal")
                    .width(80.0)
                    .overflow(Overflow::Auto, Overflow::Clip)
                    .children((0..3).map(|index| {
                        Button::new(TestMessage::Option(index), format!("item {index}"))
                            .id(format!("item-{index}"))
                            .width(50.0)
                            .shrink(0.0)
                    })),
                Rect::new(0.0, 0.0, 80.0, 32.0),
                state,
            )
        };
        let mut state = UiStateStore::default();
        let initial = build(&mut state);
        let extent = initial
            .resolved_layout()
            .find(&UiId::from("root/horizontal"))
            .and_then(|node| node.scroll)
            .expect("horizontal scroll metadata");
        assert_eq!(extent.content.width, 150.0);
        assert_eq!(
            initial.handle_event(
                &mut state,
                UiEvent::ScrollHorizontal {
                    point: Point { x: 10.0, y: 10.0 },
                    delta_x: 60.0,
                },
            ),
            EventOutcome {
                messages: Vec::new(),
                clipboard_text: None,
                invalidation: Invalidation::Layout,
            }
        );
        let scrolled = build(&mut state);
        assert_eq!(
            scrolled
                .resolved_layout()
                .find(&UiId::from("root/horizontal"))
                .and_then(|node| node.scroll)
                .expect("horizontal scroll metadata")
                .offset_x,
            60.0
        );
        assert_eq!(
            scrolled.message_at(Point { x: 65.0, y: 10.0 }),
            Some(&TestMessage::Option(2))
        );
        assert!(scrolled.accessibility_nodes().iter().all(|node| {
            node.rect.origin.x >= 0.0 && node.rect.origin.x + node.rect.size.width <= 80.0
        }));
    }
}
