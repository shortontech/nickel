use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt::Write,
    ops::Range,
    sync::Arc,
};

use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style as FontStyle, Weight, Wrap,
};
use image::RgbaImage;
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    Align, Axis, Constraints, FlexItem, Insets, Invalidation, Justify, Length, Overflow, Point,
    Rect, SelectionDocument, SelectionEndpoint, SelectionRun, Size, TextBoundary, TextEditor,
    Track, UiId, UiStateStore, layout_flex,
};

pub type Color = u32;

/// How image pixels are mapped into their allocated viewport.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ImageFit {
    /// Preserve aspect ratio and show the complete image.
    #[default]
    Contain,
    /// Preserve aspect ratio and fill the viewport, cropping overflow.
    Cover,
    /// Fill the viewport without preserving aspect ratio.
    Stretch,
    /// Keep the image at its intrinsic logical size.
    Center,
    /// Repeat the image at its intrinsic logical size across the viewport.
    Tile,
    /// Fill one combined desktop viewport while preserving aspect ratio.
    Span,
}

/// Alignment along one image-presentation axis.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ImageAlignment {
    Start,
    #[default]
    Center,
    End,
}

impl ImageAlignment {
    const fn factor(self) -> f32 {
        match self {
            Self::Start => 0.0,
            Self::Center => 0.5,
            Self::End => 1.0,
        }
    }
}

/// Typed, deterministic image sizing and alignment policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ImagePresentation {
    pub fit: ImageFit,
    pub horizontal: ImageAlignment,
    pub vertical: ImageAlignment,
}

impl ImagePresentation {
    pub const fn new(fit: ImageFit) -> Self {
        Self {
            fit,
            horizontal: ImageAlignment::Center,
            vertical: ImageAlignment::Center,
        }
    }

    pub const fn aligned(mut self, horizontal: ImageAlignment, vertical: ImageAlignment) -> Self {
        self.horizontal = horizontal;
        self.vertical = vertical;
        self
    }

    /// Resolve the destination rectangle. Cropping is performed by the image
    /// viewport, so cover and large centered images may extend beyond it.
    pub fn bounds(self, viewport: Rect, source: Size) -> Rect {
        if source.width <= 0.0
            || source.height <= 0.0
            || !source.width.is_finite()
            || !source.height.is_finite()
        {
            return Rect::new(viewport.origin.x, viewport.origin.y, 0.0, 0.0);
        }

        let viewport_width = viewport.size.width.max(0.0);
        let viewport_height = viewport.size.height.max(0.0);
        let (width, height) = match self.fit {
            ImageFit::Stretch => (viewport_width, viewport_height),
            ImageFit::Center => (source.width, source.height),
            ImageFit::Contain | ImageFit::Cover | ImageFit::Span => {
                let horizontal_scale = viewport_width / source.width;
                let vertical_scale = viewport_height / source.height;
                let scale = if self.fit == ImageFit::Contain {
                    horizontal_scale.min(vertical_scale)
                } else {
                    horizontal_scale.max(vertical_scale)
                };
                (source.width * scale, source.height * scale)
            }
            ImageFit::Tile => (source.width, source.height),
        };
        Rect::new(
            viewport.origin.x + (viewport_width - width) * self.horizontal.factor(),
            viewport.origin.y + (viewport_height - height) * self.vertical.factor(),
            width,
            height,
        )
    }
}

/// One non-overlapping byte range in a styled text stream.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct StyledTextSpan {
    pub range: Range<usize>,
    pub bold: bool,
    pub italic: bool,
    pub monospace: bool,
    pub strikethrough: bool,
    pub color: Option<Color>,
    pub background: Option<Color>,
}

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
    Scroll {
        point: Point,
        delta_y: f32,
    },
    ScrollHorizontal {
        point: Point,
        delta_x: f32,
    },
    FocusNext,
    FocusPrevious,
    ControllerNext,
    ControllerPrevious,
    ActivateFocused,
    KeyboardActivate,
    ControllerActivate,
    /// Moves accessibility focus through the same production focus state used
    /// by keyboard navigation, so the visible focus treatment cannot diverge.
    AccessibilityFocus(UiId),
    AccessibilityActivate(UiId),
    TextInput(String),
    ImePreedit(String),
    TextBackspace,
    TextBackspaceWord,
    TextDelete,
    TextMoveLeft {
        extend_selection: bool,
    },
    TextMoveRight {
        extend_selection: bool,
    },
    TextMoveWordLeft {
        extend_selection: bool,
    },
    TextMoveWordRight {
        extend_selection: bool,
    },
    TextMoveHome {
        extend_selection: bool,
    },
    TextMoveEnd {
        extend_selection: bool,
    },
    TextMoveDocumentHome {
        extend_selection: bool,
    },
    TextMoveDocumentEnd {
        extend_selection: bool,
    },
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PointerIcon {
    #[default]
    Default,
    Hand,
    Text,
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
        wrap: bool,
    },
    StyledText {
        bounds: Rect,
        text: String,
        spans: Vec<StyledTextSpan>,
        scale: f32,
        color: Color,
        align: TextAlign,
    },
    Image {
        bounds: Rect,
        id: u16,
        image: Arc<RgbaImage>,
        high_density: Option<Arc<RgbaImage>>,
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
    /// Semantic background applied while an interactive element is hovered.
    pub hover_background: Option<Background>,
    /// Semantic background applied while an interactive element is pressed.
    pub pressed_background: Option<Background>,
    /// Semantic border applied for keyboard, controller, or accessibility focus.
    pub focus_border: Option<Color>,
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
    pub accessibility_label: Option<String>,
    pub accessibility_description: Option<String>,
    /// Platform-neutral semantic role consumed by accessibility adapters.
    pub accessibility_role: Option<String>,
    pub accessibility_state: Option<String>,
    /// Stable id of the surface controlled by this element, when applicable.
    pub accessibility_controls: Option<UiId>,
    pub accessibility_hidden: bool,
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
            hover_background: None,
            pressed_background: None,
            focus_border: None,
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
            accessibility_label: None,
            accessibility_description: None,
            accessibility_role: None,
            accessibility_state: None,
            accessibility_controls: None,
            accessibility_hidden: false,
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
        controlled: bool,
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
    StyledText {
        value: String,
        spans: Vec<StyledTextSpan>,
        scale: f32,
        wrap: bool,
        line_height: Option<f32>,
    },
    Image {
        id: u16,
        image: Arc<RgbaImage>,
        high_density: Option<Arc<RgbaImage>>,
        presentation: ImagePresentation,
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
            Self::StyledText { .. } => "StyledText",
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
    inline_messages: Vec<(Range<usize>, Message)>,
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
            inline_messages: Vec::new(),
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
            inline_messages: Vec::new(),
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

    pub fn interaction_backgrounds(
        mut self,
        hover: impl Into<Background>,
        pressed: impl Into<Background>,
    ) -> Self {
        self.style.hover_background = Some(hover.into());
        self.style.pressed_background = Some(pressed.into());
        self
    }

    pub fn focus_border(mut self, color: Color) -> Self {
        self.style.focus_border = Some(color);
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

    pub fn accessibility_label(mut self, label: impl Into<String>) -> Self {
        self.style.accessibility_label = Some(label.into());
        self
    }

    pub fn accessibility_description(mut self, description: impl Into<String>) -> Self {
        self.style.accessibility_description = Some(description.into());
        self
    }

    pub fn accessibility_role(mut self, role: impl Into<String>) -> Self {
        self.style.accessibility_role = Some(role.into());
        self
    }

    pub fn accessibility_state(mut self, state: impl Into<String>) -> Self {
        self.style.accessibility_state = Some(state.into());
        self
    }

    pub fn accessibility_controls(mut self, id: impl Into<UiId>) -> Self {
        self.style.accessibility_controls = Some(id.into());
        self
    }

    pub fn accessibility_hidden(mut self, hidden: bool) -> Self {
        self.style.accessibility_hidden = hidden;
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
            inline_messages: self
                .inline_messages
                .into_iter()
                .map(|(range, message)| (range, map(message)))
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct StyledTextMeasureKey {
    text: String,
    spans: Vec<StyledTextSpan>,
    locale: String,
    scale: u32,
    width: u32,
    wrap: bool,
    line_height: u32,
}

struct TextMeasurer {
    font_system: FontSystem,
    plain: HashMap<TextMeasureKey, Size>,
    styled: HashMap<StyledTextMeasureKey, Size>,
}

impl Default for TextMeasurer {
    fn default() -> Self {
        Self {
            font_system: FontSystem::new(),
            plain: HashMap::new(),
            styled: HashMap::new(),
        }
    }
}

thread_local! {
    static TEXT_MEASURER: RefCell<TextMeasurer> = RefCell::new(TextMeasurer::default());
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
            locale: measurer.font_system.locale().to_owned(),
            scale: scale.to_bits(),
            width: width.to_bits(),
            bold,
            wrap,
            line_height: line_height.to_bits(),
            max_lines,
        };
        if let Some(size) = measurer.plain.get(&key).copied() {
            return size;
        }
        let font_system = &mut measurer.font_system;
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
        if measurer.plain.len() >= 2048 {
            measurer.plain.clear();
        }
        measurer.plain.insert(key, measured);
        measured
    })
}

fn styled_attrs(span: Option<&StyledTextSpan>) -> Attrs<'static> {
    let mut attrs = Attrs::new().family(if span.is_some_and(|span| span.monospace) {
        Family::Monospace
    } else {
        Family::SansSerif
    });
    if span.is_some_and(|span| span.bold) {
        attrs = attrs.weight(Weight::BOLD);
    }
    if span.is_some_and(|span| span.italic) {
        attrs = attrs.style(FontStyle::Italic);
    }
    attrs
}

fn styled_segments<'a>(text: &'a str, spans: &[StyledTextSpan]) -> Vec<(&'a str, Attrs<'static>)> {
    let mut segments = Vec::new();
    let mut cursor = 0;
    for span in spans {
        let start = span.range.start.min(text.len());
        let end = span.range.end.min(text.len());
        if start > cursor && text.is_char_boundary(cursor) && text.is_char_boundary(start) {
            segments.push((&text[cursor..start], styled_attrs(None)));
        }
        if end > start && text.is_char_boundary(start) && text.is_char_boundary(end) {
            segments.push((&text[start..end], styled_attrs(Some(span))));
            cursor = end;
        }
    }
    if cursor < text.len() && text.is_char_boundary(cursor) {
        segments.push((&text[cursor..], styled_attrs(None)));
    }
    if segments.is_empty() {
        segments.push((text, styled_attrs(None)));
    }
    segments
}

fn measure_styled_text(
    text: &str,
    spans: &[StyledTextSpan],
    scale: f32,
    wrap: bool,
    line_height: Option<f32>,
    max_width: f32,
) -> Size {
    let font_size = text_font_size(scale);
    let line_height = line_height.unwrap_or(font_size * 1.3).max(1.0);
    TEXT_MEASURER.with(|measurer| {
        let mut measurer = measurer.borrow_mut();
        let width = if wrap && max_width.is_finite() {
            max_width.max(1.0)
        } else {
            f32::INFINITY
        };
        let key = StyledTextMeasureKey {
            text: text.to_owned(),
            spans: spans.to_vec(),
            locale: measurer.font_system.locale().to_owned(),
            scale: scale.to_bits(),
            width: width.to_bits(),
            wrap,
            line_height: line_height.to_bits(),
        };
        if let Some(size) = measurer.styled.get(&key).copied() {
            return size;
        }
        let font_system = &mut measurer.font_system;
        let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
        buffer.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
        buffer.set_size(width.is_finite().then_some(width), None);
        let defaults = styled_attrs(None);
        buffer.set_rich_text(
            styled_segments(text, spans),
            &defaults,
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(font_system, false);
        let mut measured = Size::new(0.0, 0.0);
        for run in buffer.layout_runs() {
            measured.width = measured.width.max(run.line_w);
            measured.height += run.line_height;
        }
        if measured.height == 0.0 {
            measured.height = line_height;
        }
        if measurer.styled.len() >= 2048 {
            measurer.styled.clear();
        }
        measurer.styled.insert(key, measured);
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

    fn accessibility_label(self, label: impl Into<String>) -> Element<Message> {
        self.into_element().accessibility_label(label)
    }

    fn accessibility_description(self, description: impl Into<String>) -> Element<Message> {
        self.into_element().accessibility_description(description)
    }

    fn accessibility_role(self, role: impl Into<String>) -> Element<Message> {
        self.into_element().accessibility_role(role)
    }

    fn accessibility_state(self, state: impl Into<String>) -> Element<Message> {
        self.into_element().accessibility_state(state)
    }

    fn accessibility_controls(self, id: impl Into<UiId>) -> Element<Message> {
        self.into_element().accessibility_controls(id)
    }

    fn accessibility_hidden(self, hidden: bool) -> Element<Message> {
        self.into_element().accessibility_hidden(hidden)
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

            /// Reverse visual child order without changing the direction of
            /// text or artwork inside the children.
            pub fn reverse(mut self) -> Self {
                self.0.children.reverse();
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

mod components;
pub use components::*;

mod settings_components;
pub use settings_components::*;

mod start_menu_components;
pub use start_menu_components::*;

mod tree;
pub use tree::*;
