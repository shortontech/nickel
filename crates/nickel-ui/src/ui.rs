use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt::Write,
    ops::Range,
    sync::Arc,
};

use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, Style as FontStyle, Weight, Wrap};
use image::RgbaImage;
use nickel_render_assets::ProcessFontSystem;
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    Align, Axis, Constraints, FlexItem, InputModality, Insets, Invalidation, Justify, Length,
    Overflow, Point, Rect, SelectionDocument, SelectionEndpoint, SelectionRun, Size, TextBoundary,
    TextEditor, Track, UiId, UiStateStore, layout_flex,
};

pub type Color = u32;

pub(crate) enum TextMessageMapper<Message> {
    Function(fn(String) -> Message),
    Closure(Arc<dyn Fn(String) -> Message>),
}

impl<Message> Clone for TextMessageMapper<Message> {
    fn clone(&self) -> Self {
        match self {
            Self::Function(map) => Self::Function(*map),
            Self::Closure(map) => Self::Closure(Arc::clone(map)),
        }
    }
}

impl<Message> std::fmt::Debug for TextMessageMapper<Message> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TextMessageMapper")
    }
}

impl<Message> TextMessageMapper<Message> {
    fn new(map: impl Fn(String) -> Message + 'static) -> Self {
        Self::Closure(Arc::new(map))
    }

    fn call(&self, value: String) -> Message {
        match self {
            Self::Function(map) => map(value),
            Self::Closure(map) => map(value),
        }
    }
}

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

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum TextUnderlineStyle {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// One non-overlapping byte range in a styled text stream.
/// Typed paint overrides for hover, pressed, and keyboard/controller focus.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InteractionPaint {
    pub background: Option<Color>,
    pub foreground: Option<Color>,
    pub border_color: Option<Color>,
    pub border_width: Option<f32>,
    pub radius: Option<f32>,
    pub font_size: Option<f32>,
    pub line_height: Option<f32>,
    /// Optional interaction geometry. Declarative CSS uses this for controls
    /// that grow near the pointer; layout and hit testing remain unified.
    pub width: Option<Length>,
    pub height: Option<Length>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

/// Native distance-based sibling magnification for horizontal containers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProximityMagnification {
    pub maximum_scale: f32,
    pub radius: usize,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct StyledTextSpan {
    pub range: Range<usize>,
    pub bold: bool,
    pub italic: bool,
    pub monospace: bool,
    /// Optional explicit font family. The renderer resolves it through the shared font system.
    pub font_family: Option<Arc<str>>,
    pub strikethrough: bool,
    pub underline: TextUnderlineStyle,
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
    PointerCancelled,
    PointerContext(Point),
    /// Emitted by a platform gesture recognizer after a stationary touch has
    /// crossed its native long-press threshold.
    TouchLongPress(Point),
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
    KeyboardNavigateUp,
    KeyboardNavigateDown,
    KeyboardNavigateLeft,
    KeyboardNavigateRight,
    KeyboardNavigatePageUp,
    KeyboardNavigatePageDown,
    KeyboardNavigateStart,
    KeyboardNavigateEnd,
    KeyboardNavigateBack,
    KeyboardNavigateActivate,
    ControllerUp,
    ControllerDown,
    ControllerLeft,
    ControllerRight,
    /// Legacy linear traversal retained for callers without directional input.
    ControllerNext,
    ControllerPrevious,
    ControllerPreviousPane,
    ControllerNextPane,
    ControllerAdjust(f32),
    ControllerBack,
    ActivateFocused,
    KeyboardActivate,
    ControllerActivate,
    ControllerContextMenu,
    KeyboardContextMenu,
    /// Moves accessibility focus through the same production focus state used
    /// by keyboard navigation, so the visible focus treatment cannot diverge.
    AccessibilityFocus(UiId),
    /// Native logical-row focus. Geometry is prepared from the current native
    /// collection source, never from a package callback or remote node ordinal.
    AccessibilityRevealVirtualRow {
        collection: UiId,
        revision: u64,
        ordinal: usize,
        leading: f32,
        height: f32,
    },
    AccessibilityActivate(UiId),
    AccessibilityContextMenu(UiId),
    TextInput(String),
    ImePreedit(String),
    TextBackspace,
    TextBackspaceWord,
    TextDelete,
    TextUndo,
    TextRedo,
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
    FocusGained,
    FocusLost,
    Suspended,
    DeviceRemoved,
}

/// The lifecycle phase of a pointer drag owned by a declarative UI target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

/// A typed pointer drag delivered through the normal application message path.
///
/// `bounds` is the resolved target rectangle that won pointer capture. Motion
/// and release continue to target that same semantic identity even when the
/// pointer leaves the rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragGesture {
    pub phase: DragPhase,
    pub position: Point,
    pub bounds: Rect,
}

/// A release over a drop target during a captured declarative drag.
#[derive(Clone, Debug, PartialEq)]
pub struct DropGesture {
    pub position: Point,
    pub source_id: UiId,
    pub source_bounds: Rect,
    pub target_id: UiId,
    pub target_bounds: Rect,
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
    pub disposition: EventDisposition,
}

/// Whether a semantic transition consumed the admitted input. Consumption is
/// deliberately independent of visual or application-state changes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EventDisposition {
    Handled,
    #[default]
    Unhandled,
    Rejected(&'static str),
}

impl EventDisposition {
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Rejected(reason), _) | (_, Self::Rejected(reason)) => Self::Rejected(reason),
            (Self::Handled, _) | (_, Self::Handled) => Self::Handled,
            (Self::Unhandled, Self::Unhandled) => Self::Unhandled,
        }
    }
}

impl<Message> Default for EventOutcome<Message> {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            clipboard_text: None,
            invalidation: Invalidation::None,
            disposition: EventDisposition::Unhandled,
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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SemanticRole {
    ApplicationPresentation,
    Button,
    Checkbox,
    Dialog,
    Grid,
    GridCell,
    Group,
    Image,
    Link,
    List,
    ListItem,
    Menu,
    MenuItem,
    NavigationItem,
    Option,
    Pane,
    Popover,
    Radio,
    RadioGroup,
    Slider,
    ScrollBar,
    Status,
    Switch,
    Tab,
    TabList,
    TabPanel,
    Text,
    TextField,
    Tooltip,
    GraphicalCustomControl,
}

impl SemanticRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ApplicationPresentation => "application",
            Self::Button => "button",
            Self::Checkbox => "checkbox",
            Self::Dialog => "dialog",
            Self::Grid => "grid",
            Self::GridCell => "gridcell",
            Self::Group => "group",
            Self::Image => "image",
            Self::Link => "link",
            Self::List => "list",
            Self::ListItem => "listitem",
            Self::Menu => "menu",
            Self::MenuItem => "menuitem",
            Self::NavigationItem => "navigation-item",
            Self::Option => "option",
            Self::Pane => "pane",
            Self::Popover => "popover",
            Self::Radio => "radio",
            Self::RadioGroup => "radiogroup",
            Self::Slider => "slider",
            Self::ScrollBar => "scrollbar",
            Self::Status => "status",
            Self::Switch => "switch",
            Self::Tab => "tab",
            Self::TabList => "tablist",
            Self::TabPanel => "tabpanel",
            Self::Text => "text",
            Self::TextField => "textbox",
            Self::Tooltip => "tooltip",
            Self::GraphicalCustomControl => "graphics-object",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ActionKind {
    Activate,
    Cancel,
    ContextMenu,
    Increment,
    Decrement,
    SetValue,
    Expand,
    Collapse,
    Select,
    Dismiss,
    Scroll,
    EnterNavigation,
    ExitNavigation,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum NavigationTraversal {
    Linear,
    /// Form-style traversal where only Up and Down move between children.
    Vertical,
    #[default]
    Spatial,
    Grid,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum NavigationExit {
    #[default]
    Parent,
    Dismiss,
    Contain,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum NavigationEntry {
    #[default]
    First,
    Last,
    Target(UiId),
}

/// A physical direction used by explicit controller-navigation topology.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NavigationDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Optional deterministic destinations declared by a navigation scope.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NavigationNeighbors {
    pub up: Option<UiId>,
    pub down: Option<UiId>,
    pub left: Option<UiId>,
    pub right: Option<UiId>,
}

impl NavigationNeighbors {
    pub fn target(&self, direction: NavigationDirection) -> Option<&UiId> {
        match direction {
            NavigationDirection::Up => self.up.as_ref(),
            NavigationDirection::Down => self.down.as_ref(),
            NavigationDirection::Left => self.left.as_ref(),
            NavigationDirection::Right => self.right.as_ref(),
        }
    }

    pub fn with(mut self, direction: NavigationDirection, target: impl Into<UiId>) -> Self {
        *match direction {
            NavigationDirection::Up => &mut self.up,
            NavigationDirection::Down => &mut self.down,
            NavigationDirection::Left => &mut self.left,
            NavigationDirection::Right => &mut self.right,
        } = Some(target.into());
        self
    }
}

/// Explicit modality-neutral navigation topology declared by a component.
#[derive(Clone, Debug, PartialEq)]
pub struct NavigationScope {
    pub traversal: NavigationTraversal,
    pub entry: NavigationEntry,
    pub exit: NavigationExit,
    pub direction: ReadingDirection,
    pub retain_focus: bool,
    pub scroll_owner: Option<UiId>,
    pub pane: bool,
    pub default_pane: bool,
    pub neighbors: NavigationNeighbors,
}

impl Default for NavigationScope {
    fn default() -> Self {
        Self {
            traversal: NavigationTraversal::Spatial,
            entry: NavigationEntry::First,
            exit: NavigationExit::Parent,
            direction: ReadingDirection::LeftToRight,
            retain_focus: true,
            scroll_owner: None,
            pane: false,
            default_pane: false,
            neighbors: NavigationNeighbors::default(),
        }
    }
}

impl NavigationScope {
    pub fn group() -> Self {
        Self::default()
    }

    pub fn pane(default_pane: bool) -> Self {
        Self {
            pane: true,
            default_pane,
            ..Self::default()
        }
    }

    pub fn neighbor(mut self, direction: NavigationDirection, target: impl Into<UiId>) -> Self {
        self.neighbors = self.neighbors.with(direction, target);
        self
    }

    pub fn traversal(mut self, traversal: NavigationTraversal) -> Self {
        self.traversal = traversal;
        self
    }

    pub fn entry(mut self, entry: NavigationEntry) -> Self {
        self.entry = entry;
        self
    }

    pub fn exit(mut self, exit: NavigationExit) -> Self {
        self.exit = exit;
        self
    }

    pub fn direction(mut self, direction: ReadingDirection) -> Self {
        self.direction = direction;
        self
    }

    pub fn retain_focus(mut self, retain_focus: bool) -> Self {
        self.retain_focus = retain_focus;
        self
    }

    pub fn scroll_owner(mut self, scroll_owner: impl Into<UiId>) -> Self {
        self.scroll_owner = Some(scroll_owner.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SemanticValueInput {
    Boolean(bool),
    Number(f64),
    Text(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum SemanticAction {
    Invoke(ActionKind),
    SetValue(SemanticValueInput),
}

/// The production input channel that originated an interaction transition.
///
/// Tooling selects a real channel when it is proving modality behavior. `Programmatic` is reserved
/// for direct semantic automation and deliberately does not change the user's current modality.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputSource {
    Keyboard,
    Pointer,
    Controller,
    Accessibility,
    Programmatic,
    System,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InteractionIntent {
    Event(UiEvent),
    Invoke {
        target: UiId,
        action: SemanticAction,
    },
}

impl UiEvent {
    pub const fn input_source(&self) -> InputSource {
        match self {
            Self::PointerMoved(_)
            | Self::PointerPressed(_)
            | Self::PointerReleased(_)
            | Self::PointerCancelled
            | Self::PointerContext(_)
            | Self::TouchLongPress(_)
            | Self::Scroll { .. }
            | Self::ScrollHorizontal { .. } => InputSource::Pointer,
            Self::ControllerUp
            | Self::ControllerDown
            | Self::ControllerLeft
            | Self::ControllerRight
            | Self::ControllerNext
            | Self::ControllerPrevious
            | Self::ControllerPreviousPane
            | Self::ControllerNextPane
            | Self::ControllerAdjust(_)
            | Self::ControllerBack
            | Self::ControllerActivate
            | Self::ControllerContextMenu => InputSource::Controller,
            Self::AccessibilityFocus(_)
            | Self::AccessibilityRevealVirtualRow { .. }
            | Self::AccessibilityActivate(_)
            | Self::AccessibilityContextMenu(_) => InputSource::Accessibility,
            Self::FocusNext
            | Self::FocusPrevious
            | Self::KeyboardNavigateUp
            | Self::KeyboardNavigateDown
            | Self::KeyboardNavigateLeft
            | Self::KeyboardNavigateRight
            | Self::KeyboardNavigatePageUp
            | Self::KeyboardNavigatePageDown
            | Self::KeyboardNavigateStart
            | Self::KeyboardNavigateEnd
            | Self::KeyboardNavigateBack
            | Self::KeyboardNavigateActivate
            | Self::ActivateFocused
            | Self::KeyboardActivate
            | Self::KeyboardContextMenu
            | Self::TextInput(_)
            | Self::ImePreedit(_)
            | Self::TextBackspace
            | Self::TextBackspaceWord
            | Self::TextDelete
            | Self::TextUndo
            | Self::TextRedo
            | Self::TextMoveLeft { .. }
            | Self::TextMoveRight { .. }
            | Self::TextMoveWordLeft { .. }
            | Self::TextMoveWordRight { .. }
            | Self::TextMoveHome { .. }
            | Self::TextMoveEnd { .. }
            | Self::TextMoveDocumentHome { .. }
            | Self::TextMoveDocumentEnd { .. }
            | Self::TextSelectAll
            | Self::TextCopy
            | Self::TextCut
            | Self::TextPaste(_)
            | Self::SelectionClear
            | Self::Dismiss => InputSource::Keyboard,
            Self::CaretBlink
            | Self::FocusGained
            | Self::FocusLost
            | Self::Suspended
            | Self::DeviceRemoved => InputSource::System,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SemanticValueSnapshot {
    Boolean(bool),
    Number {
        value: f64,
        minimum: f64,
        maximum: f64,
        step: f64,
    },
    Text(String),
    ProtectedText {
        character_count: usize,
    },
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
    /// Compositor material request. Renderers do not paint this command; a
    /// compositor may sample and filter the scene behind the rounded region.
    BackdropBlur {
        rect: Rect,
        radius: f32,
        blur: f32,
    },
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
    RoundedStroke {
        rect: Rect,
        color: Color,
        width: f32,
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
        /// Exact logical font size for bounded graphical consumers such as terminal cells.
        font_size: Option<f32>,
        color: Color,
        align: TextAlign,
    },
    Image {
        bounds: Rect,
        id: u16,
        generation: u64,
        image: Arc<RgbaImage>,
        high_density: Option<Arc<RgbaImage>>,
    },
    PushClip(Rect),
    PopClip,
}

pub(crate) fn assert_background_color_policy(identity: &str, commands: &[PaintCommand]) {
    if !cfg!(debug_assertions) {
        return;
    }
    // A terminal owns its viewport palette. Its surrounding root must retain a
    // different identity so title bars, menus, and other Nickel chrome remain checked.
    if identity
        .split('/')
        .any(|segment| segment == "terminal-viewport")
    {
        return;
    }
    for (index, command) in commands.iter().enumerate() {
        let colors: &[Color] = match command {
            PaintCommand::Fill { color, .. }
            | PaintCommand::TopRoundedFill { color, .. }
            | PaintCommand::RoundedFill { color, .. }
            | PaintCommand::OverlayFill { color, .. } => std::slice::from_ref(color),
            PaintCommand::Gradient { gradient, .. } => {
                if is_prohibited_background(gradient.start)
                    || is_prohibited_background(gradient.end)
                {
                    panic!(
                        "prohibited pure-black/white UI background in {identity} paint command {index}"
                    );
                }
                continue;
            }
            PaintCommand::BackdropBlur { .. }
            | PaintCommand::RoundedStroke { .. }
            | PaintCommand::Stroke { .. }
            | PaintCommand::OverlayStroke { .. }
            | PaintCommand::Text { .. }
            | PaintCommand::StyledText { .. }
            | PaintCommand::Image { .. }
            | PaintCommand::PushClip(_)
            | PaintCommand::PopClip => continue,
        };
        if let Some(color) = colors
            .iter()
            .copied()
            .find(|color| is_prohibited_background(*color))
        {
            panic!(
                "prohibited pure-black/white UI background in {identity} paint command {index} ({color:#010x})"
            );
        }
    }
}

const fn is_prohibited_background(color: Color) -> bool {
    let rgb = color & 0x00ff_ffff;
    rgb == 0x000000 || rgb == 0xffffff
}

#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub background: Option<Background>,
    pub border: Option<Color>,
    pub border_width: f32,
    pub box_shadow: Option<BoxShadow>,
    pub backdrop_blur: Option<f32>,
    pub proximity_magnification: Option<ProximityMagnification>,
    pub transition_duration_ms: f32,
    pub foreground: Option<Color>,
    /// Semantic background applied while an interactive element is hovered.
    pub interaction_paints: Option<Box<[InteractionPaint; 3]>>,
    pub hover_background: Option<Background>,
    /// Semantic background applied while an interactive element is pressed.
    pub pressed_background: Option<Background>,
    /// Explicit background for a focused control, used by declarative styles.
    pub focus_background: Option<Background>,
    /// Semantic hue/lightness cue applied to the child background for keyboard
    /// or accessibility focus.
    pub focus_background_tint: Option<Color>,
    /// Presentation compilers may supply complete focus cues explicitly.
    pub automatic_focus_tint: bool,
    pub css_paint: bool,
    /// Whether foreground, font size and line height inherit owner state paint.
    pub inherited_state_text: [bool; 3],
    /// Decorative compound parts follow owner state without becoming input targets.
    pub parent_interaction: bool,
    pub auto_focus: bool,
    pub editing_parts: Option<Box<[DropdownPartStyle; 2]>>,
    pub editing_menu: Option<Box<crate::OverlayMenuPresentation>>,
    /// Semantic hue/lightness cue applied to the current controller target.
    pub controller_focus_background_tint: Option<Color>,
    /// Shared semantic scrollbar chrome for this element's viewport.
    pub scrollbar_palette: crate::ScrollbarPalette,
    /// Optional CSS track and thumb; native callers retain semantic defaults.
    pub scrollbar_parts: Option<Box<[DropdownPartStyle; 2]>>,
    /// Semantic background applied to a selected or entered controller scope.
    pub controller_scope_background: Option<Background>,
    pub text_align: TextAlign,
    pub padding: Insets,
    pub gap: f32,
    /// Child origin within an absolute `Layer` parent.
    pub absolute_position: Option<Point>,
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
    /// Explicit exemption for visual-only semantic content. A declared role
    /// without a name is otherwise rejected from semantic/accessibility output.
    pub semantic_decorative: bool,
    pub semantic_role: Option<SemanticRole>,
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
            box_shadow: None,
            backdrop_blur: None,
            proximity_magnification: None,
            transition_duration_ms: 0.0,
            foreground: None,
            interaction_paints: None,
            hover_background: None,
            pressed_background: None,
            focus_background: None,
            focus_background_tint: None,
            automatic_focus_tint: true,
            css_paint: false,
            inherited_state_text: [false; 3],
            parent_interaction: false,
            auto_focus: false,
            editing_parts: None,
            editing_menu: None,
            controller_focus_background_tint: None,
            scrollbar_palette: crate::theme::FALLBACK_SCROLLBAR_PALETTE,
            scrollbar_parts: None,
            controller_scope_background: None,
            text_align: TextAlign::Start,
            padding: Insets::default(),
            gap: 0.0,
            absolute_position: None,
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
            semantic_decorative: false,
            semantic_role: None,
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

#[derive(Clone, Debug)]
enum Kind {
    Flex(Axis),
    Layer,
    VerticalScroll {
        offset: f32,
        controlled: bool,
    },
    Grid {
        columns: GridColumnSpec,
    },
    CustomPaint {
        paint: fn(Rect) -> Vec<PaintCommand>,
    },
    CustomPaintCommands {
        commands: Vec<PaintCommand>,
    },
    Text {
        value: String,
        scale: f32,
        bold: bool,
        wrap: bool,
        line_height: Option<f32>,
        max_lines: Option<usize>,
        ellipsis: bool,
        outline: Option<(Color, f32)>,
        selection_x: Option<(f32, f32)>,
        caret_position: Option<Point>,
        input_value: Option<String>,
        input_placeholder: Option<String>,
        input_mask: Option<char>,
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
        generation: u64,
        image: Arc<RgbaImage>,
        high_density: Option<Arc<RgbaImage>>,
        presentation: ImagePresentation,
    },
    Slider {
        value: f32,
        track: Color,
        fill: Color,
        thumb: Color,
        thumb_border: Color,
        geometry: [f32; 6],
        presentation: Option<Box<[DropdownPartStyle; 3]>>,
    },
    Dropdown {
        selected: String,
        options: Vec<String>,
        expanded: bool,
        open_generation: u64,
        overlay: bool,
        background: Color,
        option_background: Color,
        foreground: Color,
        presentation: Option<Box<[DropdownPartStyle; 3]>>,
        option_presentations: Vec<DropdownPartStyle>,
        resolved_options: Vec<DropdownPartStyle>,
    },
}

impl Kind {
    const fn name(&self) -> &'static str {
        match self {
            Self::Flex(Axis::Horizontal) => "Row",
            Self::Flex(Axis::Vertical) => "Column",
            Self::Layer => "Layer",
            Self::VerticalScroll { .. } => "VerticalScroll",
            Self::Grid { .. } => "Grid",
            Self::CustomPaint { .. } | Self::CustomPaintCommands { .. } => "CustomPaint",
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
    /// Source-owned revision for payloads that cannot be compared through
    /// bounded structural metadata. Without one, retained reconciliation
    /// conservatively rebuilds every content-sensitive phase.
    content_revision: Option<u64>,
    virtual_navigation: Option<VirtualNavigation>,
    kind: Kind,
    style: Style,
    message: Option<Message>,
    context_message: Option<Message>,
    focus_message: Option<Message>,
    blur_message: Option<Message>,
    message_mapper: Option<fn(f32) -> Message>,
    seeded_value_mapper: Option<fn(Message, f32) -> Message>,
    scroll_extent_mapper: Option<fn(ScrollExtent) -> Message>,
    drag_seed: Option<Message>,
    drag_mapper: Option<fn(Message, DragGesture) -> Message>,
    drop_message: Option<Message>,
    drop_mapper: Option<fn(Message, DropGesture) -> Message>,
    text_mapper: Option<TextMessageMapper<Message>>,
    option_messages: Vec<Option<Message>>,
    inline_messages: Vec<(Range<usize>, Message)>,
    children: Vec<Element<Message>>,
    navigation_scope: Option<NavigationScope>,
    adjustment_step: f32,
}

impl<Message> Element<Message> {
    fn flex(axis: Axis) -> Self {
        Self {
            kind: Kind::Flex(axis),
            id: None,
            source: None,
            content_revision: None,
            virtual_navigation: None,
            style: Style::default(),
            message: None,
            context_message: None,
            focus_message: None,
            blur_message: None,
            message_mapper: None,
            seeded_value_mapper: None,
            scroll_extent_mapper: None,
            drag_seed: None,
            drag_mapper: None,
            drop_message: None,
            drop_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
            navigation_scope: None,
            adjustment_step: 0.05,
        }
    }

    fn layer() -> Self {
        let mut element = Self::flex(Axis::Vertical);
        element.kind = Kind::Layer;
        element
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
                outline: None,
                selection_x: None,
                caret_position: None,
                input_value: None,
                input_placeholder: None,
                input_mask: None,
            },
            id: None,
            source: None,
            content_revision: None,
            virtual_navigation: None,
            style: Style::default(),
            message: None,
            context_message: None,
            focus_message: None,
            blur_message: None,
            message_mapper: None,
            seeded_value_mapper: None,
            scroll_extent_mapper: None,
            drag_seed: None,
            drag_mapper: None,
            drop_message: None,
            drop_mapper: None,
            text_mapper: None,
            option_messages: Vec::new(),
            inline_messages: Vec::new(),
            children: Vec::new(),
            navigation_scope: None,
            adjustment_step: 0.05,
        }
    }

    pub fn child(mut self, child: impl Component<Message>) -> Self {
        let mut child = child.into_element();
        if let Some(revision) = self.content_revision {
            child.set_inherited_content_revision(revision);
        }
        self.children.push(child);
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Declare an exact source-owned revision for this component's content.
    /// The revision must change whenever text, styled spans, custom paint
    /// commands, option payloads, or accessibility strings change.
    pub fn content_revision(mut self, revision: u64) -> Self {
        self.content_revision = Some(revision);
        for child in &mut self.children {
            child.set_inherited_content_revision(revision);
        }
        self
    }

    /// Version immutable, callback-free text in a source-owned declaration.
    /// The source must change the revision when text or accessibility payloads
    /// change. Does not assign revisions to editors, selection regions, or
    /// action-bearing nodes; their transient state needs separate lifecycles.
    /// Explicit revisions already supplied by a component are preserved.
    pub fn static_text_content_revision(mut self, revision: u64) -> Self {
        self.set_static_text_content_revision(revision, false);
        self
    }

    fn set_static_text_content_revision(&mut self, revision: u64, selection_region: bool) {
        let selection_region = selection_region
            || self.style.selection_region
            || self.style.selection_document.is_some();
        if !selection_region
            && self.content_revision.is_none()
            && matches!(
                self.kind,
                Kind::Text {
                    input_value: None,
                    ..
                }
            )
            && self.text_mapper.is_none()
            && self.message.is_none()
            && self.context_message.is_none()
            && self.focus_message.is_none()
            && self.blur_message.is_none()
            && self.drag_seed.is_none()
            && self.drop_message.is_none()
            && self.option_messages.is_empty()
            && self.inline_messages.is_empty()
        {
            self.content_revision = Some(revision);
        }
        for child in &mut self.children {
            child.set_static_text_content_revision(revision, selection_region);
        }
    }

    fn set_inherited_content_revision(&mut self, revision: u64) {
        if self.content_revision.is_none() {
            self.content_revision = Some(revision);
        }
        for child in &mut self.children {
            child.set_inherited_content_revision(revision);
        }
    }

    #[doc(hidden)]
    pub fn with_source(mut self, source: SourceLocation) -> Self {
        self.source = Some(source);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Component<Message>>) -> Self {
        self.children.extend(children.into_iter().map(|child| {
            let mut child = child.into_element();
            if let Some(revision) = self.content_revision {
                child.set_inherited_content_revision(revision);
            }
            child
        }));
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

    pub fn box_shadow(mut self, shadow: BoxShadow) -> Self {
        self.style.box_shadow = Some(shadow);
        self
    }

    pub fn backdrop_blur(mut self, radius: f32) -> Self {
        self.style.backdrop_blur = Some(radius.clamp(0.0, 128.0));
        self
    }

    pub fn proximity_magnification(mut self, magnification: ProximityMagnification) -> Self {
        self.style.proximity_magnification = Some(magnification);
        self
    }

    pub fn transition_duration_ms(mut self, duration: f32) -> Self {
        self.style.transition_duration_ms = duration.clamp(0.0, 2_000.0);
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

    pub fn focus_background_tint(mut self, color: Color) -> Self {
        self.style.focus_background_tint = Some(color);
        self
    }

    pub fn focus_background(mut self, background: impl Into<Background>) -> Self {
        self.style.focus_background = Some(background.into());
        self
    }

    pub fn controller_focus_background_tint(mut self, color: Color) -> Self {
        self.style.controller_focus_background_tint = Some(color);
        self
    }

    pub fn scrollbar_theme(mut self, theme: crate::SemanticTheme) -> Self {
        self.style.scrollbar_palette = theme.scrollbar_palette();
        self
    }

    pub fn navigation_scope(mut self, scope: NavigationScope) -> Self {
        self.navigation_scope = Some(scope);
        self
    }

    pub fn controller_scope_background(mut self, background: impl Into<Background>) -> Self {
        self.style.controller_scope_background = Some(background.into());
        self
    }

    /// Sets the normalized left/right adjustment step for sliders.
    pub fn adjustment_step(mut self, step: f32) -> Self {
        self.adjustment_step = step.max(0.0);
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

    pub fn position(mut self, position: Point) -> Self {
        self.style.absolute_position = Some(position);
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

    pub fn on_scroll_extent(mut self, map: fn(ScrollExtent) -> Message) -> Self {
        self.scroll_extent_mapper = Some(map);
        self
    }

    /// Turns this element into a declarative pointer-drag target.
    ///
    /// The seed message supplies target-specific typed data. Nickel UI owns
    /// hit testing and pointer capture, then calls `map` for every drag phase.
    pub fn on_drag(mut self, seed: Message, map: fn(Message, DragGesture) -> Message) -> Self {
        self.drag_seed = Some(seed);
        self.drag_mapper = Some(map);
        self
    }

    /// Accepts a release from a captured drag source over this element.
    pub fn on_drop(mut self, seed: Message, map: fn(Message, DropGesture) -> Message) -> Self {
        self.drop_message = Some(seed);
        self.drop_mapper = Some(map);
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

    pub fn context_message(mut self, message: Message) -> Self {
        self.context_message = Some(message);
        self
    }

    pub fn focus_message(mut self, message: Message) -> Self {
        self.focus_message = Some(message);
        self
    }

    pub fn blur_message(mut self, message: Message) -> Self {
        self.blur_message = Some(message);
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

    pub fn decorative(mut self) -> Self {
        self.style.semantic_decorative = true;
        self
    }

    pub fn semantic_role(mut self, role: SemanticRole) -> Self {
        self.style.semantic_role = Some(role);
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
            self.message_mapper.is_none()
                && self.seeded_value_mapper.is_none()
                && self.drag_mapper.is_none()
                && self.drop_mapper.is_none()
                && self.text_mapper.is_none(),
            "map value-producing messages at the control constructor"
        );
        Element {
            kind: self.kind,
            id: self.id,
            source: self.source,
            content_revision: self.content_revision,
            virtual_navigation: self.virtual_navigation,
            style: self.style,
            message: self.message.map(&mut *map),
            context_message: self.context_message.map(&mut *map),
            focus_message: self.focus_message.map(&mut *map),
            blur_message: self.blur_message.map(&mut *map),
            message_mapper: None,
            seeded_value_mapper: None,
            scroll_extent_mapper: None,
            drag_seed: None,
            drag_mapper: None,
            drop_message: None,
            drop_mapper: None,
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
            navigation_scope: self.navigation_scope,
            adjustment_step: self.adjustment_step,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TextMeasureKey {
    text: String,
    locale: String,
    font_generation: u64,
    scale: u32,
    width: u32,
    bold: bool,
    wrap: bool,
    line_height: u32,
    max_lines: Option<usize>,
    direction: TextDirectionAuthority,
    masking: TextMaskingAuthority,
    fallback: TextFallbackAuthority,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct StyledTextMeasureKey {
    text: String,
    spans: Vec<StyledTextSpan>,
    locale: String,
    font_generation: u64,
    scale: u32,
    width: u32,
    wrap: bool,
    line_height: u32,
    direction: TextDirectionAuthority,
    masking: TextMaskingAuthority,
    fallback: TextFallbackAuthority,
}

struct TextMeasurer {
    font_system: ProcessFontSystem,
    cache_enabled: bool,
    plain: HashMap<TextMeasureKey, Arc<PlainTextLayout>>,
    styled: HashMap<StyledTextMeasureKey, Arc<PlainTextLayout>>,
    plain_bytes: usize,
    styled_bytes: usize,
    observed_font_generation: u64,
    diagnostics: TextLayoutCacheDiagnostics,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum TextDirectionAuthority {
    Auto,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum TextMaskingAuthority {
    Unmasked,
    Masked(char),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum TextFallbackAuthority {
    SansSerif,
    StyledSpans,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TextRetention {
    Public,
    Masked(char),
    Protected,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextLayoutCacheDiagnostics {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub rejections: u64,
    pub generation_invalidations: u64,
    pub entries: usize,
    pub retained_bytes: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct TextClusterPosition {
    pub(crate) line: usize,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) rtl: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TextCaretPosition {
    pub(crate) x: f32,
    pub(crate) offset: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct TextVisualLine {
    pub(crate) carets: Vec<TextCaretPosition>,
}

#[derive(Clone, Debug)]
pub(crate) struct PlainTextLayout {
    pub(crate) size: Size,
    pub(crate) clusters: Vec<TextClusterPosition>,
    pub(crate) line_widths: Vec<f32>,
    pub(crate) visual_lines: Vec<TextVisualLine>,
}

impl PlainTextLayout {
    pub(crate) fn offset_at_x(&self, line: usize, x: f32) -> Option<usize> {
        let carets = &self.visual_lines.get(line)?.carets;
        let after = carets.partition_point(|caret| caret.x < x);
        match (after.checked_sub(1), carets.get(after)) {
            (Some(before), Some(next)) => {
                let previous = carets[before];
                Some(if x - previous.x <= next.x - x {
                    previous.offset
                } else {
                    next.offset
                })
            }
            (Some(before), None) => Some(carets[before].offset),
            (None, Some(next)) => Some(next.offset),
            (None, None) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextMeasureCacheMode {
    #[default]
    Enabled,
    BypassDerived,
}

/// Runs one diagnostic or verification operation with a thread-local text
/// measurement cache policy. Entering and leaving bypass mode clears both
/// plain and styled derived entries, so results cannot leak across modes.
pub fn with_text_measure_cache_mode<R>(
    mode: TextMeasureCacheMode,
    operation: impl FnOnce() -> R,
) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEXT_MEASURER.with(|measurer| {
                let mut measurer = measurer.borrow_mut();
                measurer.cache_enabled = self.0;
                measurer.plain.clear();
                measurer.styled.clear();
                measurer.plain_bytes = 0;
                measurer.styled_bytes = 0;
            });
        }
    }
    let previous = TEXT_MEASURER.with(|measurer| {
        let mut measurer = measurer.borrow_mut();
        let previous = measurer.cache_enabled;
        measurer.cache_enabled = mode == TextMeasureCacheMode::Enabled;
        measurer.plain.clear();
        measurer.styled.clear();
        measurer.plain_bytes = 0;
        measurer.styled_bytes = 0;
        previous
    });
    let restore = Restore(previous);
    let result = operation();
    drop(restore);
    result
}

impl Default for TextMeasurer {
    fn default() -> Self {
        let font_system = ProcessFontSystem::new();
        Self {
            observed_font_generation: font_system.generation(),
            font_system,
            cache_enabled: true,
            plain: HashMap::new(),
            styled: HashMap::new(),
            plain_bytes: 0,
            styled_bytes: 0,
            diagnostics: TextLayoutCacheDiagnostics::default(),
        }
    }
}

const TEXT_MEASURE_CACHE_CAPACITY: usize = 2048;
const TEXT_MEASURE_CACHE_BYTE_BUDGET: usize = 2 * 1024 * 1024;

fn plain_measure_key_bytes(key: &TextMeasureKey) -> usize {
    std::mem::size_of::<TextMeasureKey>() + key.text.len() + key.locale.len()
}

fn plain_layout_bytes(key: &TextMeasureKey, layout: &PlainTextLayout) -> usize {
    plain_measure_key_bytes(key)
        + std::mem::size_of::<PlainTextLayout>()
        + layout.clusters.len() * std::mem::size_of::<TextClusterPosition>()
        + layout.line_widths.len() * std::mem::size_of::<f32>()
        + layout.visual_lines.len() * std::mem::size_of::<TextVisualLine>()
        + layout
            .visual_lines
            .iter()
            .map(|line| line.carets.len() * std::mem::size_of::<TextCaretPosition>())
            .sum::<usize>()
}

fn styled_measure_key_bytes(key: &StyledTextMeasureKey) -> usize {
    std::mem::size_of::<StyledTextMeasureKey>()
        + key.text.len()
        + key.locale.len()
        + key.spans.len() * std::mem::size_of::<StyledTextSpan>()
        + key
            .spans
            .iter()
            .filter_map(|span| span.font_family.as_ref())
            .map(|family| family.len())
            .sum::<usize>()
}

fn styled_layout_bytes(key: &StyledTextMeasureKey, layout: &PlainTextLayout) -> usize {
    styled_measure_key_bytes(key)
        + std::mem::size_of::<PlainTextLayout>()
        + layout.clusters.len() * std::mem::size_of::<TextClusterPosition>()
        + layout.line_widths.len() * std::mem::size_of::<f32>()
        + layout.visual_lines.len() * std::mem::size_of::<TextVisualLine>()
        + layout
            .visual_lines
            .iter()
            .map(|line| line.carets.len() * std::mem::size_of::<TextCaretPosition>())
            .sum::<usize>()
}

thread_local! {
    static TEXT_MEASURER: RefCell<TextMeasurer> = RefCell::new(TextMeasurer::default());
}

/// Bounded counters for the calling thread's derived text-layout cache.
pub fn text_layout_cache_diagnostics() -> TextLayoutCacheDiagnostics {
    TEXT_MEASURER.with(|measurer| {
        let measurer = measurer.borrow();
        TextLayoutCacheDiagnostics {
            entries: measurer.plain.len() + measurer.styled.len(),
            retained_bytes: measurer.plain_bytes + measurer.styled_bytes,
            ..measurer.diagnostics
        }
    })
}

fn synchronize_text_font_generation(measurer: &mut TextMeasurer) -> u64 {
    let generation = measurer.font_system.generation();
    if generation != measurer.observed_font_generation {
        measurer.plain.clear();
        measurer.styled.clear();
        measurer.plain_bytes = 0;
        measurer.styled_bytes = 0;
        measurer.observed_font_generation = generation;
        measurer.diagnostics.generation_invalidations = measurer
            .diagnostics
            .generation_invalidations
            .saturating_add(1);
    }
    generation
}

#[allow(clippy::too_many_arguments)]
fn build_plain_text_layout(
    measurer: &mut TextMeasurer,
    text: &str,
    scale: f32,
    bold: bool,
    wrap: bool,
    line_height: Option<f32>,
    max_lines: Option<usize>,
    max_width: f32,
) -> Arc<PlainTextLayout> {
    let font_size = text_font_size(scale);
    let line_height = line_height.unwrap_or(font_size * 1.3).max(1.0);
    let width = if wrap && max_width.is_finite() {
        max_width.max(1.0)
    } else {
        f32::INFINITY
    };
    let mut font_system = measurer.font_system.lock();
    let mut buffer = Buffer::new(&mut font_system, Metrics::new(font_size, line_height));
    buffer.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
    buffer.set_size(width.is_finite().then_some(width), None);
    let mut attrs = Attrs::new().family(Family::SansSerif);
    if bold {
        attrs = attrs.weight(Weight::BOLD);
    }
    buffer.set_text(text, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(&mut font_system, false);
    build_text_layout_from_buffer(&buffer, text, max_lines, line_height)
}

fn build_text_layout_from_buffer(
    buffer: &Buffer,
    text: &str,
    max_lines: Option<usize>,
    line_height: f32,
) -> Arc<PlainTextLayout> {
    let mut measured = Size::new(0.0, 0.0);
    let mut clusters = Vec::new();
    let mut line_widths: Vec<f32> = Vec::new();
    let mut line_bases = Vec::new();
    let mut base = 0;
    for line in text.split_inclusive('\n') {
        line_bases.push(base);
        base += line.len();
    }
    if line_bases.is_empty() {
        line_bases.push(0);
    }
    let grapheme_boundaries = text
        .grapheme_indices(true)
        .map(|(offset, _)| offset)
        .chain(std::iter::once(text.len()))
        .collect::<Vec<_>>();
    for (visual_line, run) in buffer
        .layout_runs()
        .take(max_lines.unwrap_or(usize::MAX))
        .enumerate()
    {
        measured.width = measured.width.max(run.line_w);
        measured.height += run.line_height;
        line_widths.push(run.line_w);
        let line_base = line_bases.get(run.line_i).copied().unwrap_or(0);
        clusters.extend(run.glyphs.iter().map(|glyph| {
            let raw_start = (line_base + glyph.start).min(text.len());
            let raw_end = (line_base + glyph.end).min(text.len());
            let after_start = grapheme_boundaries.partition_point(|offset| *offset <= raw_start);
            let start = grapheme_boundaries[after_start.saturating_sub(1)];
            let at_end = grapheme_boundaries.partition_point(|offset| *offset < raw_end);
            let end = grapheme_boundaries
                .get(at_end)
                .copied()
                .unwrap_or(text.len());
            TextClusterPosition {
                line: visual_line,
                start,
                end,
                x: glyph.x,
                y: run.line_top,
                width: glyph.w.max(1.0),
                height: run.line_height,
                rtl: glyph.level.is_rtl(),
            }
        }));
    }
    if measured.height == 0.0 {
        measured.height = line_height;
    }
    let mut visual_lines = (0..line_widths.len())
        .map(|_| TextVisualLine { carets: Vec::new() })
        .collect::<Vec<_>>();
    for cluster in &clusters {
        let Some(line) = visual_lines.get_mut(cluster.line) else {
            continue;
        };
        let mut offsets = text
            .get(cluster.start..cluster.end)
            .map(|slice| {
                slice
                    .grapheme_indices(true)
                    .map(|(offset, _)| cluster.start + offset)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if offsets.first().copied() != Some(cluster.start) {
            offsets.insert(0, cluster.start);
        }
        if offsets.last().copied() != Some(cluster.end) {
            offsets.push(cluster.end);
        }
        let intervals = offsets.len().saturating_sub(1).max(1) as f32;
        for (index, offset) in offsets.into_iter().enumerate() {
            let fraction = index as f32 / intervals;
            let fraction = if cluster.rtl {
                1.0 - fraction
            } else {
                fraction
            };
            line.carets.push(TextCaretPosition {
                x: cluster.x + cluster.width * fraction,
                offset,
            });
        }
    }
    for line in &mut visual_lines {
        line.carets.sort_by(|left, right| {
            left.x
                .total_cmp(&right.x)
                .then(left.offset.cmp(&right.offset))
        });
        line.carets.dedup_by(|left, right| {
            left.x.to_bits() == right.x.to_bits() && left.offset == right.offset
        });
    }
    Arc::new(PlainTextLayout {
        size: measured,
        clusters,
        line_widths,
        visual_lines,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn shape_plain_text(
    text: &str,
    scale: f32,
    bold: bool,
    wrap: bool,
    line_height: Option<f32>,
    max_lines: Option<usize>,
    max_width: f32,
    retention: TextRetention,
) -> Arc<PlainTextLayout> {
    let resolved_line_height = line_height
        .unwrap_or_else(|| text_font_size(scale) * 1.3)
        .max(1.0);
    let width = if wrap && max_width.is_finite() {
        max_width.max(1.0)
    } else {
        f32::INFINITY
    };
    TEXT_MEASURER.with(|measurer| {
        let mut measurer = measurer.borrow_mut();
        let font_generation = synchronize_text_font_generation(&mut measurer);
        if retention == TextRetention::Protected {
            measurer.diagnostics.rejections = measurer.diagnostics.rejections.saturating_add(1);
            return build_plain_text_layout(
                &mut measurer,
                text,
                scale,
                bold,
                wrap,
                line_height,
                max_lines,
                max_width,
            );
        }
        let key = TextMeasureKey {
            text: text.to_owned(),
            locale: measurer.font_system.lock().locale().to_owned(),
            font_generation,
            scale: scale.to_bits(),
            width: width.to_bits(),
            bold,
            wrap,
            line_height: resolved_line_height.to_bits(),
            max_lines,
            direction: TextDirectionAuthority::Auto,
            masking: match retention {
                TextRetention::Public => TextMaskingAuthority::Unmasked,
                TextRetention::Masked(mask) => TextMaskingAuthority::Masked(mask),
                TextRetention::Protected => unreachable!("protected text is rejected above"),
            },
            fallback: TextFallbackAuthority::SansSerif,
        };
        if measurer.cache_enabled
            && let Some(layout) = measurer.plain.get(&key).cloned()
        {
            measurer.diagnostics.hits = measurer.diagnostics.hits.saturating_add(1);
            return layout;
        }
        measurer.diagnostics.misses = measurer.diagnostics.misses.saturating_add(1);
        let layout = build_plain_text_layout(
            &mut measurer,
            text,
            scale,
            bold,
            wrap,
            line_height,
            max_lines,
            max_width,
        );
        let key_bytes = plain_layout_bytes(&key, &layout);
        if measurer.cache_enabled {
            if measurer.plain.len().saturating_add(measurer.styled.len())
                >= TEXT_MEASURE_CACHE_CAPACITY
                || measurer
                    .plain_bytes
                    .saturating_add(measurer.styled_bytes)
                    .saturating_add(key_bytes)
                    > TEXT_MEASURE_CACHE_BYTE_BUDGET
            {
                measurer.diagnostics.evictions = measurer.diagnostics.evictions.saturating_add(1);
                measurer.plain.clear();
                measurer.styled.clear();
                measurer.plain_bytes = 0;
                measurer.styled_bytes = 0;
            }
            if key_bytes <= TEXT_MEASURE_CACHE_BYTE_BUDGET {
                measurer.plain_bytes += key_bytes;
                measurer.plain.insert(key, Arc::clone(&layout));
            } else {
                measurer.diagnostics.rejections = measurer.diagnostics.rejections.saturating_add(1);
            }
        }
        layout
    })
}

pub(crate) fn measure_text(
    text: &str,
    scale: f32,
    bold: bool,
    wrap: bool,
    line_height: Option<f32>,
    max_lines: Option<usize>,
    max_width: f32,
) -> Size {
    measure_text_with_retention(
        text,
        scale,
        bold,
        wrap,
        line_height,
        max_lines,
        max_width,
        TextRetention::Public,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn measure_text_with_retention(
    text: &str,
    scale: f32,
    bold: bool,
    wrap: bool,
    line_height: Option<f32>,
    max_lines: Option<usize>,
    max_width: f32,
    retention: TextRetention,
) -> Size {
    shape_plain_text(
        text,
        scale,
        bold,
        wrap,
        line_height,
        max_lines,
        max_width,
        retention,
    )
    .size
}

/// Intrinsic width of a single line using the same font metrics as UI layout.
pub fn intrinsic_text_width(text: &str, scale: f32) -> f32 {
    measure_text(text, scale, false, false, None, Some(1), f32::INFINITY).width
}

fn styled_attrs(span: Option<&StyledTextSpan>) -> Attrs<'_> {
    let family = span
        .and_then(|span| span.font_family.as_deref())
        .filter(|family| !family.eq_ignore_ascii_case("monospace"))
        .map(Family::Name)
        .unwrap_or_else(|| {
            if span.is_some_and(|span| span.monospace) {
                Family::Monospace
            } else {
                Family::SansSerif
            }
        });
    let mut attrs = Attrs::new().family(family);
    if span.is_some_and(|span| span.bold) {
        attrs = attrs.weight(Weight::BOLD);
    }
    if span.is_some_and(|span| span.italic) {
        attrs = attrs.style(FontStyle::Italic);
    }
    attrs
}

fn styled_segments<'a>(text: &'a str, spans: &'a [StyledTextSpan]) -> Vec<(&'a str, Attrs<'a>)> {
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

fn build_styled_text_layout(
    measurer: &mut TextMeasurer,
    text: &str,
    spans: &[StyledTextSpan],
    scale: f32,
    wrap: bool,
    line_height: Option<f32>,
    max_width: f32,
) -> Arc<PlainTextLayout> {
    let font_size = text_font_size(scale);
    let line_height = line_height.unwrap_or(font_size * 1.3).max(1.0);
    let width = if wrap && max_width.is_finite() {
        max_width.max(1.0)
    } else {
        f32::INFINITY
    };
    let mut font_system = measurer.font_system.lock();
    let mut buffer = Buffer::new(&mut font_system, Metrics::new(font_size, line_height));
    buffer.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
    buffer.set_size(width.is_finite().then_some(width), None);
    let defaults = styled_attrs(None);
    buffer.set_rich_text(
        styled_segments(text, spans),
        &defaults,
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(&mut font_system, false);
    build_text_layout_from_buffer(&buffer, text, None, line_height)
}

fn shape_styled_text(
    text: &str,
    spans: &[StyledTextSpan],
    scale: f32,
    wrap: bool,
    line_height: Option<f32>,
    max_width: f32,
) -> Arc<PlainTextLayout> {
    if spans.is_empty() {
        return shape_plain_text(
            text,
            scale,
            false,
            wrap,
            line_height,
            None,
            max_width,
            TextRetention::Public,
        );
    }
    let font_size = text_font_size(scale);
    let line_height = line_height.unwrap_or(font_size * 1.3).max(1.0);
    TEXT_MEASURER.with(|measurer| {
        let mut measurer = measurer.borrow_mut();
        let font_generation = synchronize_text_font_generation(&mut measurer);
        let width = if wrap && max_width.is_finite() {
            max_width.max(1.0)
        } else {
            f32::INFINITY
        };
        let key = StyledTextMeasureKey {
            text: text.to_owned(),
            spans: spans.to_vec(),
            locale: measurer.font_system.lock().locale().to_owned(),
            font_generation,
            scale: scale.to_bits(),
            width: width.to_bits(),
            wrap,
            line_height: line_height.to_bits(),
            direction: TextDirectionAuthority::Auto,
            masking: TextMaskingAuthority::Unmasked,
            fallback: TextFallbackAuthority::StyledSpans,
        };
        if measurer.cache_enabled
            && let Some(layout) = measurer.styled.get(&key).cloned()
        {
            measurer.diagnostics.hits = measurer.diagnostics.hits.saturating_add(1);
            return layout;
        }
        measurer.diagnostics.misses = measurer.diagnostics.misses.saturating_add(1);
        let layout = build_styled_text_layout(
            &mut measurer,
            text,
            spans,
            scale,
            wrap,
            line_height.into(),
            max_width,
        );
        let key_bytes = styled_layout_bytes(&key, &layout);
        if measurer.cache_enabled {
            if measurer.plain.len().saturating_add(measurer.styled.len())
                >= TEXT_MEASURE_CACHE_CAPACITY
                || measurer
                    .plain_bytes
                    .saturating_add(measurer.styled_bytes)
                    .saturating_add(key_bytes)
                    > TEXT_MEASURE_CACHE_BYTE_BUDGET
            {
                measurer.diagnostics.evictions = measurer.diagnostics.evictions.saturating_add(1);
                measurer.plain.clear();
                measurer.styled.clear();
                measurer.plain_bytes = 0;
                measurer.styled_bytes = 0;
            }
            if key_bytes <= TEXT_MEASURE_CACHE_BYTE_BUDGET {
                measurer.styled_bytes += key_bytes;
                measurer.styled.insert(key, Arc::clone(&layout));
            } else {
                measurer.diagnostics.rejections = measurer.diagnostics.rejections.saturating_add(1);
            }
        }
        layout
    })
}

fn measure_styled_text(
    text: &str,
    spans: &[StyledTextSpan],
    scale: f32,
    wrap: bool,
    line_height: Option<f32>,
    max_width: f32,
) -> Size {
    shape_styled_text(text, spans, scale, wrap, line_height, max_width).size
}

pub(crate) fn text_font_size(scale: f32) -> f32 {
    if scale < 0.0 {
        return -scale;
    }
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
#[derive(Clone)]
pub struct AnyView<Message = String>(Element<Message>);

impl<Message> AnyView<Message> {
    pub fn new(component: impl Component<Message>) -> Self {
        Self(component.into_element())
    }

    /// Apply source-owned static-text revisions in place, without moving the
    /// declaration through another by-value component conversion.
    /// See [`Element::static_text_content_revision`] for the source contract.
    pub fn set_static_text_content_revision(&mut self, revision: u64) {
        self.0.set_static_text_content_revision(revision, false);
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
    /// Apply a compiler-owned frame without changing the native component behavior.
    fn css_frame(self, frame: DropdownPartStyle) -> Element<Message> {
        let frame = frame.bounded();
        let mut element = self.into_element();
        element.style.css_paint = true;
        element.style.automatic_focus_tint = false;
        element.style.background = frame
            .background
            .filter(|color| *color != 0)
            .map(Background::Solid);
        element.style.foreground = Some(frame.foreground.unwrap_or(0));
        element.style.border = frame.border_color.filter(|color| *color != 0);
        element.style.border_width = frame.border_width;
        element.style.corner_radius = frame.radius;
        element.style.padding = frame.padding;
        element.style.interaction_paints = Some(Box::new(frame.interaction_paints));
        element
    }

    fn id(self, id: impl Into<UiId>) -> Element<Message> {
        self.into_element().id(id)
    }

    fn content_revision(self, revision: u64) -> Element<Message> {
        self.into_element().content_revision(revision)
    }

    fn padding(self, padding: impl Into<Insets>) -> Element<Message> {
        self.into_element().padding(padding)
    }

    fn context_message(self, message: Message) -> Element<Message> {
        self.into_element().context_message(message)
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

    fn semantic_role(self, role: SemanticRole) -> Element<Message> {
        self.into_element().semantic_role(role)
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

    fn decorative(self) -> Element<Message> {
        self.into_element().decorative()
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

    fn scrollbar_theme(self, theme: crate::SemanticTheme) -> Element<Message> {
        self.into_element().scrollbar_theme(theme)
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

            pub fn scrollbar_theme(mut self, theme: crate::SemanticTheme) -> Self {
                self.0 = self.0.scrollbar_theme(theme);
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

            pub fn on_scroll_extent(mut self, map: fn(ScrollExtent) -> Message) -> Self {
                self.0 = self.0.on_scroll_extent(map);
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
mod collection;
pub use collection::*;

mod responsive_navigation;
pub use responsive_navigation::*;

mod settings_components;
pub use settings_components::*;

mod start_menu_components;
pub use start_menu_components::*;

mod retained;
mod tree;
pub use tree::*;

/// Tessellate an inside rounded border without painting its transparent center.
/// All presenters consume the same bounded logical strips.
pub fn rounded_border_spans(rect: Rect, width: f32, radius: f32) -> impl Iterator<Item = Rect> {
    let width = width
        .max(0.0)
        .min(rect.size.width / 2.0)
        .min(rect.size.height / 2.0);
    let radius = radius
        .max(0.0)
        .min(rect.size.width / 2.0)
        .min(rect.size.height / 2.0);
    let inner = rect.inset(Insets::all(width));
    let inner_radius = (radius - width).max(0.0);
    let inset = |y: f32, height: f32, r: f32| {
        let edge = y.min(height - y);
        if edge >= r {
            0.0
        } else {
            r - (r * r - (r - edge).powi(2)).max(0.0).sqrt()
        }
    };
    (0..rect.size.height.ceil().clamp(0.0, 16384.0) as u32).flat_map(move |row| {
        let y = row as f32;
        let h = (rect.size.height - y).min(1.0);
        let middle = y + h / 2.0;
        let outer_inset = inset(middle, rect.size.height, radius);
        let x = rect.origin.x + outer_inset;
        let right = rect.origin.x + rect.size.width - outer_inset;
        let inner_y = middle - width;
        let hole = width > 0.0 && inner_y >= 0.0 && inner_y < inner.size.height;
        let inner_inset = inset(inner_y, inner.size.height, inner_radius);
        let left_end = if hole {
            inner.origin.x + inner_inset
        } else {
            right
        };
        let right_start = inner.origin.x + inner.size.width - inner_inset;
        [
            Rect::new(x, rect.origin.y + y, (left_end - x).max(0.0), h),
            Rect::new(
                right_start,
                rect.origin.y + y,
                if hole {
                    (right - right_start).max(0.0)
                } else {
                    0.0
                },
                h,
            ),
        ]
        .into_iter()
        .filter(move |span| width > 0.0 && span.size.width > 0.0 && span.size.height > 0.0)
    })
}

/// Antialiased rounded geometry in logical coordinates. Coverage is sampled on
/// the physical pixel grid, including fractional origins and output scaling.
/// Equal spans on adjacent rows are merged, so straight edges stay inexpensive.
pub fn rounded_coverage_spans(
    rect: Rect,
    color: u32,
    radius: f32,
    stroke: Option<f32>,
    top_only: bool,
    scale: f32,
) -> Vec<(Rect, u32)> {
    if !scale.is_finite() || scale <= 0.0 || rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return Vec::new();
    }
    let rect = Rect::new(
        rect.origin.x * scale,
        rect.origin.y * scale,
        rect.size.width * scale,
        rect.size.height * scale,
    );
    let radius = (radius * scale)
        .max(0.0)
        .min(rect.size.width / 2.0)
        .min(rect.size.height / 2.0);
    let width = stroke.map(|w| {
        (w * scale)
            .max(0.0)
            .min(rect.size.width / 2.0)
            .min(rect.size.height / 2.0)
    });
    if width == Some(0.0) {
        return Vec::new();
    }
    let interval = |r: Rect, radius: f32, y: f32| -> Option<(f32, f32)> {
        let y = y - r.origin.y;
        if y < 0.0 || y >= r.size.height || r.size.width <= 0.0 {
            return None;
        }
        let edge = if top_only {
            y
        } else {
            y.min(r.size.height - y)
        };
        let inset = if edge < radius {
            radius - (radius * radius - (radius - edge).powi(2)).max(0.0).sqrt()
        } else {
            0.0
        };
        Some((r.origin.x + inset, r.origin.x + r.size.width - inset))
    };
    let alpha = if color <= 0x00ff_ffff {
        255
    } else {
        color >> 24
    };
    let mut spans: Vec<(Rect, u32)> = Vec::new();
    let mut previous: Vec<usize> = Vec::new();
    let mut row = rect.origin.y.floor() as i32;
    let end = (rect.origin.y + rect.size.height).ceil() as i32;
    let middle_start = (rect.origin.y + radius.max(width.unwrap_or(0.0))).ceil() as i32;
    let bottom_radius = if top_only { 0.0 } else { radius };
    let middle_end =
        (rect.origin.y + rect.size.height - bottom_radius.max(width.unwrap_or(0.0))).floor() as i32;
    while row < end {
        let rows = if row >= middle_start && row < middle_end {
            middle_end - row
        } else {
            1
        };
        // Integrate four horizontal slices per physical pixel. Horizontal
        // coverage is analytic; only the curved vertical profile is sampled.
        let mut slices = Vec::with_capacity(8);
        for sample in 0..4 {
            let y = row as f32 + (sample as f32 + 0.5) / 4.0;
            if let Some((left, right)) = interval(rect, radius, y) {
                let hole = width
                    .and_then(|w| interval(rect.inset(Insets::all(w)), (radius - w).max(0.0), y));
                if let Some((il, ir)) = hole {
                    slices.push((left, il));
                    slices.push((ir, right));
                } else {
                    slices.push((left, right));
                }
            }
        }
        let mut boundaries: Vec<i32> = slices
            .iter()
            .flat_map(|&(l, r)| {
                [
                    l.floor() as i32,
                    l.ceil() as i32,
                    r.floor() as i32,
                    r.ceil() as i32,
                ]
            })
            .collect();
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut current = Vec::new();
        for pair in boundaries.windows(2) {
            let x = pair[0] as f32;
            let length = (pair[1] - pair[0]) as f32;
            if length <= 0.0 {
                continue;
            }
            let coverage: f32 = slices
                .iter()
                .map(|&(l, r)| (r.min(x + length) - l.max(x)).max(0.0))
                .sum::<f32>()
                / (4.0 * length);
            let a = (alpha as f32 * coverage.clamp(0.0, 1.0)).round() as u32;
            // Zero in the packed color format means opaque RGB, not transparent.
            if a == 0 {
                continue;
            }
            let shaded = (color & 0x00ff_ffff) | (a << 24);
            let span = Rect::new(x, row as f32, length, rows as f32);
            let existing = previous.iter().copied().find(|&index| {
                let (old, c) = spans[index];
                c == shaded
                    && old.origin.x == x
                    && old.size.width == length
                    && old.origin.y + old.size.height == row as f32
            });
            let index = if let Some(index) = existing {
                spans[index].0.size.height += rows as f32;
                index
            } else {
                spans.push((span, shaded));
                spans.len() - 1
            };
            current.push(index);
        }
        previous = current;
        row += rows;
    }
    for (span, _) in &mut spans {
        span.origin.x /= scale;
        span.origin.y /= scale;
        span.size.width /= scale;
        span.size.height /= scale;
    }
    spans
}

#[cfg(test)]
mod background_policy_tests {
    use super::*;

    #[test]
    #[should_panic(expected = "prohibited pure-black/white UI background in root/dialog")]
    fn runtime_policy_attributes_extreme_fills_to_the_component_root() {
        assert_background_color_policy(
            "root/dialog",
            &[PaintCommand::Fill {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                color: 0x000000,
            }],
        );
    }

    #[test]
    fn terminal_viewport_has_the_only_runtime_background_exception() {
        assert_background_color_policy(
            "root/terminal-viewport",
            &[PaintCommand::Fill {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                color: 0xffffff,
            }],
        );
    }
}
