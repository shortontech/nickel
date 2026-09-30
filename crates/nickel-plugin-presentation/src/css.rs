//! Bounded CSS subset with inherited custom properties and typed native styles.

use std::collections::{HashMap, HashSet};

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser,
};
use nickel_core::theme::{Appearance, ThemePalette};
use nickel_ui::{Align, Insets, Justify, Length, ReadingDirection, Track};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Display {
    #[default]
    Block,
    Flex,
    Grid,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FlexDirection {
    #[default]
    Row,
    Column,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ControlStyle {
    pub display: Option<Display>,
    pub flex_direction: Option<FlexDirection>,
    pub grid_columns: Option<Vec<Track>>,
    pub width: Option<Length>,
    pub height: Option<Length>,
    pub min_width: Option<f32>,
    pub max_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_height: Option<f32>,
    pub align_items: Option<Align>,
    pub justify_content: Option<Justify>,
    pub shrink: Option<f32>,
    pub basis: Option<Length>,
    pub padding: Option<Insets>,
    pub margin: Option<Insets>,
    pub border_width: Option<f32>,
    pub border_color: Option<u32>,
    pub radius: Option<f32>,
    pub font_size: Option<f32>,
    pub line_height: Option<f32>,
    pub background: Option<u32>,
    pub color: Option<u32>,
    pub grow: Option<f32>,
    pub gap: Option<f32>,
    pub bottom: Option<f32>,
    pub top: Option<f32>,
    pub(crate) custom_properties: HashMap<String, String>,
    pub(crate) ancestors: Vec<(String, Option<String>, Option<String>)>,
}

#[derive(Clone, Debug)]
struct Selector {
    ancestors: Vec<Selector>,
    root: bool,
    kind: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    state: Option<InteractionState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractionState {
    Hover,
    Active,
    Focus,
}

impl Selector {
    fn parse(source: &str) -> Result<Self, String> {
        let parts = source.split_ascii_whitespace().collect::<Vec<_>>();
        if parts.len() > 8 {
            return Err("CSS descendant selector exceeds eight parts".into());
        }
        if parts.len() > 1 {
            let mut selector = Self::parse(parts.last().unwrap())?;
            selector.ancestors = parts[..parts.len() - 1]
                .iter()
                .map(|part| Self::parse(part))
                .collect::<Result<_, _>>()?;
            return Ok(selector);
        }
        let mut parser = Parser::new(source);
        let mut selector = Self {
            ancestors: Vec::new(),
            root: false,
            kind: None,
            id: None,
            classes: Vec::new(),
            state: None,
        };
        while let Ok(token) = parser.next() {
            match token {
                cssparser::Token::Ident(name) if selector.kind.is_none() => {
                    selector.kind = Some(name.to_ascii_lowercase())
                }
                cssparser::Token::IDHash(name) if selector.id.is_none() => {
                    selector.id = Some(name.to_string())
                }
                cssparser::Token::Delim('.') => {
                    let name = parser
                        .expect_ident()
                        .map_err(|_| "class selector needs an identifier")?;
                    selector.classes.push(name.to_string());
                }
                cssparser::Token::Colon if selector.state.is_none() => {
                    let pseudo = parser.expect_ident();
                    if matches!(&pseudo, Ok(name) if name.eq_ignore_ascii_case("root"))
                        && selector.kind.is_none()
                        && selector.id.is_none()
                        && selector.classes.is_empty()
                    {
                        selector.root = true;
                        continue;
                    }
                    selector.state = Some(match pseudo {
                        Ok(name) if name.eq_ignore_ascii_case("hover") => InteractionState::Hover,
                        Ok(name) if name.eq_ignore_ascii_case("active") => InteractionState::Active,
                        Ok(name) if name.eq_ignore_ascii_case("focus") => InteractionState::Focus,
                        _ => return Err("unsupported plugin CSS pseudo-class".into()),
                    });
                }
                _ => return Err("unsupported plugin CSS selector".into()),
            }
        }
        if selector.root
            && (selector.kind.is_some()
                || selector.id.is_some()
                || !selector.classes.is_empty()
                || selector.state.is_some())
        {
            return Err(":root must be a standalone selector".into());
        }
        if !selector.root
            && selector.kind.is_none()
            && selector.id.is_none()
            && selector.classes.is_empty()
        {
            return Err("empty plugin CSS selector".into());
        }
        Ok(selector)
    }

    fn matches(
        &self,
        kind: &str,
        id: Option<&str>,
        class_name: Option<&str>,
        state: Option<InteractionState>,
        ancestors: &[(String, Option<String>, Option<String>)],
    ) -> bool {
        let mut remaining = ancestors;
        for required in self.ancestors.iter().rev() {
            let Some(index) = remaining.iter().rposition(|(kind, id, classes)| {
                required.matches(kind, id.as_deref(), classes.as_deref(), None, &[])
            }) else {
                return false;
            };
            remaining = &remaining[..index];
        }
        !self.root
            && self.state == state
            && self.kind.as_deref().is_none_or(|selector| selector == kind)
            && self
                .id
                .as_deref()
                .is_none_or(|selector| Some(selector) == id)
            && self.classes.iter().all(|class| {
                class_name
                    .is_some_and(|names| names.split_ascii_whitespace().any(|name| name == class))
            })
    }
}

#[derive(Clone, Debug)]
struct Rule {
    selectors: Vec<Selector>,
    declarations: Vec<ParsedDeclaration>,
}

#[derive(Clone, Debug)]
enum ParsedDeclaration {
    Custom(String, String),
    Property(String, String),
}

#[derive(Clone, Debug)]
enum Declaration {
    Display(Display),
    FlexDirection(FlexDirection),
    GridColumns(Vec<Track>),
    Width(Length),
    Height(Length),
    MinWidth(f32),
    MaxWidth(f32),
    MinHeight(f32),
    MaxHeight(f32),
    AlignItems(Align),
    JustifyContent(Justify),
    Shrink(f32),
    Basis(Length),
    Flex(f32),
    Padding(Insets),
    Margin(Insets),
    BorderWidth(f32),
    BorderColor(u32),
    Radius(f32),
    FontSize(f32),
    LineHeight(f32),
    Background(u32),
    Color(u32),
    Grow(f32),
    Gap(f32),
    Bottom(f32),
    Top(f32),
    Border(f32, u32),
}

impl Declaration {
    fn apply(&self, style: &mut ControlStyle) {
        match self {
            Self::Display(value) => style.display = Some(*value),
            Self::FlexDirection(value) => style.flex_direction = Some(*value),
            Self::GridColumns(value) => style.grid_columns = Some(value.clone()),
            Self::Width(value) => style.width = Some(*value),
            Self::Height(value) => style.height = Some(*value),
            Self::MinWidth(value) => style.min_width = Some(*value),
            Self::MaxWidth(value) => style.max_width = Some(*value),
            Self::MinHeight(value) => style.min_height = Some(*value),
            Self::MaxHeight(value) => style.max_height = Some(*value),
            Self::AlignItems(value) => style.align_items = Some(*value),
            Self::JustifyContent(value) => style.justify_content = Some(*value),
            Self::Shrink(value) => style.shrink = Some(*value),
            Self::Basis(value) => style.basis = Some(*value),
            Self::Flex(value) => {
                style.grow = Some(*value);
                style.shrink = Some(1.0);
                style.basis = Some(Length::Percent(0.0));
            }
            Self::Padding(value) => style.padding = Some(*value),
            Self::Margin(value) => style.margin = Some(*value),
            Self::BorderWidth(value) => style.border_width = Some(*value),
            Self::BorderColor(value) => style.border_color = Some(*value),
            Self::Radius(value) => style.radius = Some(*value),
            Self::FontSize(value) => style.font_size = Some(*value),
            Self::LineHeight(value) => style.line_height = Some(*value),
            Self::Background(value) => style.background = Some(*value),
            Self::Color(value) => style.color = Some(*value),
            Self::Grow(value) => style.grow = Some(*value),
            Self::Gap(value) => style.gap = Some(*value),
            Self::Bottom(value) => style.bottom = Some(*value),
            Self::Top(value) => style.top = Some(*value),
            Self::Border(width, color) => {
                style.border_width = Some(*width);
                style.border_color = Some(*color);
            }
        }
    }
}

fn px(source: &str, maximum: f32) -> Result<f32, String> {
    let number = source.trim().strip_suffix("px").unwrap_or(source.trim());
    let value: f32 = number.parse().map_err(|_| "expected a CSS pixel length")?;
    if !value.is_finite()
        || !(0.0..=maximum).contains(&value)
        || (!source.trim().ends_with("px") && value != 0.0)
    {
        return Err("CSS length is outside the supported range".into());
    }
    Ok(value)
}

fn bounded_number(source: &str, maximum: f32, name: &str) -> Result<f32, String> {
    let value: f32 = source.parse().map_err(|_| format!("invalid {name}"))?;
    if !value.is_finite() || !(0.0..=maximum).contains(&value) {
        return Err(format!("{name} is outside the supported range"));
    }
    Ok(value)
}

fn length(source: &str) -> Result<Length, String> {
    let source = source.trim();
    match source {
        "auto" => Ok(Length::Auto),
        "min-content" => Ok(Length::MinContent),
        "max-content" => Ok(Length::MaxContent),
        _ if source.ends_with('%') => {
            let percent = bounded_number(&source[..source.len() - 1], 100.0, "percentage")?;
            Ok(Length::Percent(percent / 100.0))
        }
        _ => Ok(Length::Px(px(source, 8192.0)?)),
    }
}

fn top_level_parts(source: &str, delimiter: char) -> Result<Vec<&str>, String> {
    let mut depth = 0_u8;
    let mut start = 0;
    let mut parts = Vec::new();
    for (index, character) in source.char_indices() {
        match character {
            '(' => {
                depth = depth
                    .checked_add(1)
                    .filter(|depth| *depth <= 4)
                    .ok_or("CSS function nesting is too deep")?
            }
            ')' => depth = depth.checked_sub(1).ok_or("unbalanced CSS function")?,
            _ if depth == 0
                && (character == delimiter
                    || (delimiter == ' ' && character.is_ascii_whitespace())) =>
            {
                let part = source[start..index].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err("unbalanced CSS function".into());
    }
    let last = source[start..].trim();
    if !last.is_empty() {
        parts.push(last);
    }
    Ok(parts)
}

fn track(source: &str) -> Result<Track, String> {
    let source = source.trim();
    if source == "auto" {
        return Ok(Track::Auto);
    }
    if let Some(number) = source.strip_suffix("fr") {
        return Ok(Track::Fraction(bounded_number(
            number,
            100.0,
            "grid fraction",
        )?));
    }
    if let Some(arguments) = source
        .strip_prefix("repeat(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let parts = top_level_parts(arguments, ',')?;
        if parts.len() != 2 {
            return Err("repeat() needs a count and one track".into());
        }
        if parts[0] == "auto-fit" {
            return Ok(Track::repeat_auto_fit(track(parts[1])?));
        }
        let count: usize = parts[0].parse().map_err(|_| "invalid repeat() count")?;
        if !(1..=32).contains(&count) {
            return Err("repeat() count must be 1 to 32".into());
        }
        return Ok(Track::repeat(count, track(parts[1])?));
    }
    if let Some(arguments) = source
        .strip_prefix("minmax(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let parts = top_level_parts(arguments, ',')?;
        if parts.len() != 2 {
            return Err("minmax() needs two tracks".into());
        }
        return Ok(Track::minmax(track(parts[0])?, track(parts[1])?));
    }
    Ok(Track::Px(px(source, 8192.0)?))
}

fn grid_columns(source: &str) -> Result<Vec<Track>, String> {
    let parts = top_level_parts(source, ' ')?;
    if parts.is_empty() || parts.len() > 32 {
        return Err("grid needs 1 to 32 column tracks".into());
    }
    let tracks = parts
        .into_iter()
        .map(track)
        .collect::<Result<Vec<_>, _>>()?;
    fn expanded_count(track: &Track) -> usize {
        match track {
            Track::Repeat(count, nested) => count.saturating_mul(expanded_count(nested)),
            _ => 1,
        }
    }
    if tracks.iter().map(expanded_count).sum::<usize>() > 32 {
        return Err("grid expands beyond 32 columns".into());
    }
    Ok(tracks)
}

fn insets(source: &str) -> Result<Insets, String> {
    let values = source
        .split_ascii_whitespace()
        .map(|value| px(value, 512.0))
        .collect::<Result<Vec<_>, _>>()?;
    let [top, right, bottom, left] = match values.as_slice() {
        [all] => [*all; 4],
        [vertical, horizontal] => [*vertical, *horizontal, *vertical, *horizontal],
        [top, horizontal, bottom] => [*top, *horizontal, *bottom, *horizontal],
        [top, right, bottom, left] => [*top, *right, *bottom, *left],
        _ => return Err("CSS inset shorthand needs one to four lengths".into()),
    };
    Ok(Insets {
        top,
        right,
        bottom,
        left,
    })
}

pub(crate) fn color(source: &str) -> Result<u32, String> {
    let source = source.trim();
    if source.eq_ignore_ascii_case("transparent") {
        return Ok(0);
    }
    if let Some(args) = source
        .strip_prefix("rgba(")
        .and_then(|args| args.strip_suffix(')'))
    {
        let values = args.split(',').map(str::trim).collect::<Vec<_>>();
        let [red, green, blue, alpha] = values.as_slice() else {
            return Err("rgba() needs four arguments".into());
        };
        let channel = |value: &str| -> Result<u32, String> {
            value
                .parse::<u8>()
                .map(u32::from)
                .map_err(|_| "invalid rgba() channel".into())
        };
        let alpha: f32 = alpha.parse().map_err(|_| "invalid rgba() alpha")?;
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
            return Err("rgba() alpha must be 0 to 1".into());
        }
        return Ok(((alpha * 255.0).round() as u32) << 24
            | channel(red)? << 16
            | channel(green)? << 8
            | channel(blue)?);
    }
    let hex = source
        .strip_prefix('#')
        .ok_or("CSS color must be #RGB, #RRGGBB, #RRGGBBAA, or transparent")?;
    let expand = |n: u32| (n << 4) | n;
    match hex.len() {
        3 => {
            let n = u32::from_str_radix(hex, 16).map_err(|_| "invalid CSS color")?;
            Ok(0xff00_0000
                | (expand((n >> 8) & 0xf) << 16)
                | (expand((n >> 4) & 0xf) << 8)
                | expand(n & 0xf))
        }
        6 => Ok(0xff00_0000 | u32::from_str_radix(hex, 16).map_err(|_| "invalid CSS color")?),
        8 => {
            let n = u32::from_str_radix(hex, 16).map_err(|_| "invalid CSS color")?;
            Ok(((n & 0xff) << 24) | (n >> 8))
        }
        _ => Err("invalid CSS color".into()),
    }
}

fn declaration(name: &str, value: &str) -> Result<Declaration, String> {
    let value = value.trim();
    Ok(match name.to_ascii_lowercase().as_str() {
        "display" => Declaration::Display(match value {
            "block" => Display::Block,
            "flex" => Display::Flex,
            "grid" => Display::Grid,
            _ => return Err("display must be block, flex, or grid".into()),
        }),
        "flex-direction" => Declaration::FlexDirection(match value {
            "row" => FlexDirection::Row,
            "column" => FlexDirection::Column,
            _ => return Err("flex-direction must be row or column".into()),
        }),
        "grid-template-columns" => Declaration::GridColumns(grid_columns(value)?),
        "width" => Declaration::Width(length(value)?),
        "height" => Declaration::Height(length(value)?),
        "min-width" => Declaration::MinWidth(px(value, 8192.0)?),
        "max-width" => Declaration::MaxWidth(px(value, 8192.0)?),
        "min-height" => Declaration::MinHeight(px(value, 8192.0)?),
        "max-height" => Declaration::MaxHeight(px(value, 8192.0)?),
        "align-items" => Declaration::AlignItems(match value {
            "flex-start" | "start" => Align::Start,
            "center" => Align::Center,
            "flex-end" | "end" => Align::End,
            "stretch" => Align::Stretch,
            "baseline" => Align::Baseline,
            _ => return Err("unsupported align-items value".into()),
        }),
        "justify-content" => Declaration::JustifyContent(match value {
            "flex-start" | "start" => Justify::Start,
            "center" => Justify::Center,
            "flex-end" | "end" => Justify::End,
            "space-between" => Justify::SpaceBetween,
            "space-around" => Justify::SpaceAround,
            "space-evenly" => Justify::SpaceEvenly,
            _ => return Err("unsupported justify-content value".into()),
        }),
        "flex-shrink" => Declaration::Shrink(bounded_number(value, 100.0, "flex-shrink")?),
        "flex-basis" => Declaration::Basis(length(value)?),
        "flex" => Declaration::Flex(bounded_number(value, 100.0, "flex")?),
        "padding" => Declaration::Padding(insets(value)?),
        "margin" => Declaration::Margin(insets(value)?),
        "border-width" => Declaration::BorderWidth(px(value, 64.0)?),
        "border-color" => Declaration::BorderColor(color(value)?),
        "border" => {
            let mut parts = value.split_ascii_whitespace();
            let width = px(parts.next().ok_or("border needs a width")?, 64.0)?;
            if parts.next() != Some("solid") {
                return Err("only solid borders are supported".into());
            }
            let color = color(parts.next().ok_or("border needs a color")?)?;
            if parts.next().is_some() {
                return Err("border has too many values".into());
            }
            Declaration::Border(width, color)
        }
        "border-radius" => Declaration::Radius(px(value, 512.0)?),
        "font-size" => Declaration::FontSize(px(value, 256.0)?),
        "line-height" => Declaration::LineHeight(px(value, 512.0)?),
        "background-color" | "background" => Declaration::Background(color(value)?),
        "color" => Declaration::Color(color(value)?),
        "flex-grow" => {
            let n: f32 = value.parse().map_err(|_| "invalid flex-grow")?;
            if !n.is_finite() || !(0.0..=100.0).contains(&n) {
                return Err("flex-grow is outside the supported range".into());
            }
            Declaration::Grow(n)
        }
        "gap" => Declaration::Gap(px(value, 512.0)?),
        "bottom" => Declaration::Bottom(px(value, 8192.0)?),
        "top" => Declaration::Top(px(value, 8192.0)?),
        _ => return Err(format!("unsupported plugin CSS property {name:?}")),
    })
}

struct CssRuleParser;

impl<'i> AtRuleParser<'i> for CssRuleParser {
    type Prelude = ();
    type AtRule = Rule;
    type Error = String;
}

impl<'i> QualifiedRuleParser<'i> for CssRuleParser {
    type Prelude = Vec<Selector>;
    type QualifiedRule = Rule;
    type Error = String;

    fn parse_prelude(
        &mut self,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        let start = input.position();
        while input.next_including_whitespace_and_comments().is_ok() {}
        let selectors = input
            .slice_from(start)
            .split(',')
            .map(Selector::parse)
            .collect::<Result<Vec<_>, _>>()
            .map_err(ParseError::custom)?;
        if selectors.len() > 16 {
            return Err(ParseError::custom("too many selectors"));
        }
        Ok(selectors)
    }

    fn parse_block(
        &mut self,
        selectors: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Self::QualifiedRule, ParseError<Self::Error>> {
        let mut parser = CssDeclarationParser;
        let mut declarations = Vec::new();
        for result in RuleBodyParser::new(input, &mut parser) {
            match result {
                Ok(declaration) => declarations.push(declaration),
                Err((error, ..)) => return Err(error),
            }
            if declarations.len() > 64 {
                return Err(ParseError::custom("too many declarations"));
            }
        }
        if selectors.iter().any(|selector| selector.state.is_some()) {
            if !selectors.iter().all(|selector| selector.state.is_some()) {
                return Err(ParseError::custom(
                    "state selectors cannot share a rule with ordinary selectors",
                ));
            }
            if selectors.iter().any(|selector| {
                !matches!(
                    selector.kind.as_deref(),
                    Some(
                        "button"
                            | "text-field"
                            | "select-header"
                            | "option"
                            | "menu-item"
                            | "switch"
                            | "checkbox"
                            | "text-field-menu-item"
                            | "color-swatch"
                    )
                )
            }) {
                return Err(ParseError::custom(
                    "plugin CSS state selectors require an interactive control part",
                ));
            }
            if declarations.iter().any(|parsed| !matches!(parsed, ParsedDeclaration::Property(name, _) if matches!(name.as_str(), "background" | "background-color" | "color" | "border" | "border-color" | "border-width" | "border-radius" | "font-size" | "line-height"))) {
                return Err(ParseError::custom("CSS interaction declarations require paint or typography properties"));
            }
        }
        if declarations
            .iter()
            .any(|declaration| matches!(declaration, ParsedDeclaration::Property(name, _) if name == "bottom" || name == "top"))
            && selectors
                .iter()
                .any(|selector| selector.kind.as_deref() != Some("window"))
        {
            return Err(ParseError::custom(
                "top and bottom are supported only on window selectors",
            ));
        }
        Ok(Rule {
            selectors,
            declarations,
        })
    }
}

struct CssDeclarationParser;
impl<'i> AtRuleParser<'i> for CssDeclarationParser {
    type Prelude = ();
    type AtRule = ParsedDeclaration;
    type Error = String;
}
impl<'i> DeclarationParser<'i> for CssDeclarationParser {
    type Declaration = ParsedDeclaration;
    type Error = String;
    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _start: &ParserState,
    ) -> Result<Self::Declaration, ParseError<Self::Error>> {
        let start = input.position();
        while input.next_including_whitespace_and_comments().is_ok() {}
        let name = name.to_string();
        let value = input.slice_from(start).trim().to_owned();
        if name.starts_with("--") {
            if name.len() <= 2 || name.len() > 128 || value.is_empty() || value.len() > 4096 {
                return Err(ParseError::custom("invalid CSS custom property"));
            }
            validate_var_syntax(&value).map_err(ParseError::custom)?;
            Ok(ParsedDeclaration::Custom(name, value))
        } else {
            let name = name.to_ascii_lowercase();
            validate_var_syntax(&value).map_err(ParseError::custom)?;
            if !value.contains("var(") {
                declaration(&name, &value).map_err(ParseError::custom)?;
            }
            Ok(ParsedDeclaration::Property(name, value))
        }
    }
}
impl<'i> QualifiedRuleParser<'i> for CssDeclarationParser {
    type Prelude = ();
    type QualifiedRule = ParsedDeclaration;
    type Error = String;
}
impl<'i> RuleBodyItemParser<'i, ParsedDeclaration, String> for CssDeclarationParser {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}

fn palette_properties(palette: ThemePalette) -> HashMap<String, String> {
    let mut properties = HashMap::new();
    let blend = |base: u32, foreground: u32, foreground_percent: u32| -> u32 {
        let channel = |shift: u32| {
            let base = (base >> shift) & 0xff;
            let foreground = (foreground >> shift) & 0xff;
            (base * (100 - foreground_percent) + foreground * foreground_percent + 50) / 100
        };
        (channel(16) << 16) | (channel(8) << 8) | channel(0)
    };
    let light = ((palette.text >> 16) & 0xff) < 0x80;
    let raised = if light {
        blend(palette.surface, 0x00ff_ffff, 35)
    } else {
        blend(palette.panel, palette.text, 10)
    };
    let control = if light {
        blend(palette.surface, 0x00ff_ffff, 15)
    } else {
        blend(palette.panel, palette.text, 15)
    };
    for (name, color) in [
        ("background", palette.background),
        ("panel", palette.panel),
        ("surface", palette.surface),
        ("surface-hover", palette.surface_hover),
        ("text", palette.text),
        ("muted", palette.muted),
        ("accent", palette.accent),
        ("accent-soft", palette.accent_soft),
        ("complement", palette.complement),
        ("raised", raised),
        ("control", control),
        ("border", blend(palette.panel, palette.text, 30)),
        ("soft-text", blend(palette.muted, palette.text, 40)),
        ("selected", blend(control, palette.accent, 25)),
        ("selected-border", blend(palette.accent, palette.text, 35)),
    ] {
        properties.insert(
            format!("--nickel-{name}"),
            format!("#{:06x}", color & 0x00ff_ffff),
        );
    }
    for (name, value) in [
        ("surface-raised", format!("#{:06x}", raised & 0x00ff_ffff)),
        (
            "text-muted",
            format!("#{:06x}", palette.muted & 0x00ff_ffff),
        ),
        ("radius-control", "8px".into()),
        ("radius-card", "12px".into()),
        ("spacing-control", "8px".into()),
        ("font-size", "14px".into()),
        ("line-height", "20px".into()),
    ] {
        properties.insert(format!("--nickel-{name}"), value);
    }
    properties
}

fn var_call(source: &str, offset: usize) -> Result<(usize, &str, Option<&str>), String> {
    let arguments_start = offset + 4;
    let mut depth = 1_u8;
    let mut comma = None;
    for (relative, character) in source[arguments_start..].char_indices() {
        match character {
            '(' => {
                depth = depth
                    .checked_add(1)
                    .ok_or("CSS function nesting is too deep")?
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let end = arguments_start + relative;
                    let split = comma.unwrap_or(end);
                    let name = source[arguments_start..split].trim();
                    if !name.starts_with("--") || name.len() <= 2 {
                        return Err("var() needs a custom property name".into());
                    }
                    let fallback = comma.map(|comma| source[comma + 1..end].trim());
                    if fallback.is_some_and(str::is_empty) {
                        return Err("var() fallback cannot be empty".into());
                    }
                    return Ok((end + 1, name, fallback));
                }
            }
            ',' if depth == 1 && comma.is_none() => comma = Some(arguments_start + relative),
            _ => {}
        }
        if depth > 8 {
            return Err("CSS function nesting is too deep".into());
        }
    }
    Err("unclosed var()".into())
}

fn validate_var_syntax(source: &str) -> Result<(), String> {
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find("var(") {
        let offset = cursor + relative;
        let (end, _, fallback) = var_call(source, offset)?;
        if let Some(fallback) = fallback {
            validate_var_syntax(fallback)?;
        }
        cursor = end;
    }
    Ok(())
}

fn resolve_value(
    source: &str,
    properties: &HashMap<String, String>,
    resolving: &mut HashSet<String>,
) -> Result<String, String> {
    if resolving.len() >= 32 {
        return Err("CSS custom property chain is too deep".into());
    }
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find("var(") {
        let offset = cursor + relative;
        output.push_str(&source[cursor..offset]);
        let (end, name, fallback) = var_call(source, offset)?;
        let replacement = if let Some(value) = properties.get(name) {
            if !resolving.insert(name.to_owned()) {
                return Err(format!("cyclic CSS custom property {name}"));
            }
            let result = resolve_value(value, properties, resolving);
            resolving.remove(name);
            result?
        } else if let Some(fallback) = fallback {
            resolve_value(fallback, properties, resolving)?
        } else {
            return Err(format!("undefined CSS custom property {name}"));
        };
        output.push_str(&replacement);
        if output.len() > 16 * 1024 {
            return Err("resolved CSS value is too large".into());
        }
        cursor = end;
    }
    output.push_str(&source[cursor..]);
    if output.len() > 16 * 1024 {
        return Err("resolved CSS value is too large".into());
    }
    Ok(output)
}

#[derive(Clone, Debug, Default)]
pub struct StyleSheet {
    rules: Vec<Rule>,
    palette: Option<ThemePalette>,
    uses_palette: bool,
    reading_direction: ReadingDirection,
}

impl StyleSheet {
    pub fn compile(source: &str) -> Result<Self, String> {
        Self::compile_with_palette(source, ThemePalette::from_appearance(Appearance::default()))
    }

    pub fn compile_with_palette(source: &str, palette: ThemePalette) -> Result<Self, String> {
        if source.len() > nickel_core::plugins::MAX_PLUGIN_CSS_BYTES {
            return Err("plugin stylesheet exceeds 256 KiB".into());
        }
        let mut parser = Parser::new(source);
        let mut rule_parser = CssRuleParser;
        let mut rules = Vec::new();
        for result in StyleSheetParser::new(&mut parser, &mut rule_parser) {
            let rule = result.map_err(|(error, _, location)| {
                format!(
                    "plugin CSS at {}:{}: {error:?}",
                    location.line, location.column
                )
            })?;
            rules.push(rule);
            if rules.len() > 256 {
                return Err("plugin stylesheet has more than 256 rules".into());
            }
        }
        let sheet = Self {
            rules,
            palette: Some(palette),
            uses_palette: source.contains("--nickel-"),
            reading_direction: ReadingDirection::LeftToRight,
        };
        sheet.validate()?;
        Ok(sheet)
    }

    pub fn reading_direction(&self) -> ReadingDirection {
        self.reading_direction
    }

    pub fn set_reading_direction(&mut self, direction: ReadingDirection) -> bool {
        let changed = self.reading_direction != direction;
        self.reading_direction = direction;
        changed
    }

    pub fn set_palette(&mut self, palette: ThemePalette) -> Result<bool, String> {
        if self.palette == Some(palette) {
            return Ok(false);
        }
        self.palette = Some(palette);
        Ok(self.uses_palette)
    }

    fn validate(&self) -> Result<(), String> {
        let palette = self
            .palette
            .unwrap_or_else(|| ThemePalette::from_appearance(Appearance::default()));
        let mut root_properties = palette_properties(palette);
        let mut declared = HashSet::new();
        for rule in &self.rules {
            for parsed in &rule.declarations {
                if let ParsedDeclaration::Custom(name, value) = parsed {
                    declared.insert(name.clone());
                    if rule.selectors.iter().any(|selector| selector.root) {
                        root_properties.insert(name.clone(), value.clone());
                    }
                }
            }
        }
        for rule in &self.rules {
            if rule.selectors.iter().any(|selector| selector.root)
                && rule
                    .declarations
                    .iter()
                    .any(|value| !matches!(value, ParsedDeclaration::Custom(..)))
            {
                return Err(":root accepts only CSS custom properties".into());
            }
            let mut properties = root_properties.clone();
            for parsed in &rule.declarations {
                if let ParsedDeclaration::Custom(name, value) = parsed {
                    properties.insert(name.clone(), value.clone());
                }
            }
            for parsed in &rule.declarations {
                let (name, value) = match parsed {
                    ParsedDeclaration::Custom(name, _) => (None, format!("var({name})")),
                    ParsedDeclaration::Property(name, value) => (Some(name), value.clone()),
                };
                match resolve_value(&value, &properties, &mut HashSet::new()) {
                    Ok(value) => {
                        if let Some(name) = name {
                            declaration(name, &value)?;
                        }
                    }
                    // A variable supplied by a matching ancestor is checked and typed
                    // when the real component tree resolves this declaration.
                    Err(error)
                        if error
                            .strip_prefix("undefined CSS custom property ")
                            .is_some_and(|name| declared.contains(name)) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(())
    }

    pub fn resolve(&self, kind: &str, id: Option<&str>, class_name: Option<&str>) -> ControlStyle {
        self.resolve_with_custom_properties(kind, id, class_name, &HashMap::new())
    }

    pub(crate) fn resolve_with_custom_properties(
        &self,
        kind: &str,
        id: Option<&str>,
        class_name: Option<&str>,
        inherited: &HashMap<String, String>,
    ) -> ControlStyle {
        self.resolve_with_ancestors(kind, id, class_name, inherited, &[])
    }

    pub(crate) fn resolve_with_ancestors(
        &self,
        kind: &str,
        id: Option<&str>,
        class_name: Option<&str>,
        inherited: &HashMap<String, String>,
        ancestors: &[(String, Option<String>, Option<String>)],
    ) -> ControlStyle {
        let mut style = ControlStyle::default();
        style.ancestors = ancestors.to_vec();
        style.ancestors.push((
            kind.into(),
            id.map(str::to_owned),
            class_name.map(str::to_owned),
        ));
        let mut properties = palette_properties(
            self.palette
                .unwrap_or_else(|| ThemePalette::from_appearance(Appearance::default())),
        );
        for rule in &self.rules {
            if rule.selectors.iter().any(|selector| selector.root) {
                for declaration in &rule.declarations {
                    if let ParsedDeclaration::Custom(name, value) = declaration {
                        properties.insert(name.clone(), value.clone());
                    }
                }
            }
        }
        properties.extend(inherited.clone());
        for rule in &self.rules {
            if rule
                .selectors
                .iter()
                .any(|selector| selector.matches(kind, id, class_name, None, ancestors))
            {
                for declaration in &rule.declarations {
                    if let ParsedDeclaration::Custom(name, value) = declaration {
                        properties.insert(name.clone(), value.clone());
                    }
                }
            }
        }
        for rule in &self.rules {
            if rule
                .selectors
                .iter()
                .any(|selector| selector.matches(kind, id, class_name, None, ancestors))
            {
                for parsed in &rule.declarations {
                    if let ParsedDeclaration::Property(name, value) = parsed
                        && let Ok(value) = resolve_value(value, &properties, &mut HashSet::new())
                        && let Ok(declaration) = declaration(name, &value)
                    {
                        declaration.apply(&mut style);
                    }
                }
            }
        }
        style.custom_properties = properties
            .iter()
            .filter_map(|(name, value)| {
                resolve_value(value, &properties, &mut HashSet::new())
                    .ok()
                    .map(|value| (name.clone(), value))
            })
            .collect();
        style
    }

    pub(crate) fn resolve_interaction_paint(
        &self,
        kind: &str,
        id: Option<&str>,
        class_name: Option<&str>,
        state: InteractionState,
        properties: &HashMap<String, String>,
        ancestors: &[(String, Option<String>, Option<String>)],
    ) -> nickel_ui::InteractionPaint {
        let mut style = ControlStyle::default();
        for rule in &self.rules {
            if rule.selectors.iter().any(|selector| {
                selector.state == Some(state)
                    && selector.matches(kind, id, class_name, Some(state), ancestors)
            }) {
                for parsed in &rule.declarations {
                    if let ParsedDeclaration::Property(name, value) = parsed
                        && let Ok(value) = resolve_value(value, properties, &mut HashSet::new())
                        && let Ok(declaration) = declaration(name, &value)
                    {
                        declaration.apply(&mut style);
                    }
                }
            }
        }
        nickel_ui::InteractionPaint {
            background: style.background,
            foreground: style.color,
            border_color: style.border_color,
            border_width: style.border_width,
            radius: style.radius,
            font_size: style.font_size,
            line_height: style.line_height,
        }
    }

    pub fn resolve_interaction_background(
        &self,
        kind: &str,
        id: Option<&str>,
        class_name: Option<&str>,
        state: InteractionState,
    ) -> Option<u32> {
        self.resolve_interaction_background_with_properties(
            kind,
            id,
            class_name,
            state,
            &self.resolve(kind, id, class_name).custom_properties,
            &[],
        )
    }

    pub(crate) fn resolve_interaction_background_with_properties(
        &self,
        kind: &str,
        id: Option<&str>,
        class_name: Option<&str>,
        state: InteractionState,
        properties: &HashMap<String, String>,
        ancestors: &[(String, Option<String>, Option<String>)],
    ) -> Option<u32> {
        let mut background = None;
        for rule in &self.rules {
            if rule
                .selectors
                .iter()
                .any(|selector| selector.matches(kind, id, class_name, Some(state), ancestors))
            {
                for parsed in &rule.declarations {
                    if let ParsedDeclaration::Property(name, value) = parsed
                        && matches!(name.as_str(), "background" | "background-color")
                        && let Ok(value) = resolve_value(value, properties, &mut HashSet::new())
                        && let Ok(Declaration::Background(color)) = declaration(name, &value)
                    {
                        background = Some(color);
                    }
                }
            }
        }
        background
    }

    pub fn estimated_retained_bytes(&self) -> u64 {
        let mut bytes = self.rules.capacity() * std::mem::size_of::<Rule>();
        for rule in &self.rules {
            bytes += rule.selectors.capacity() * std::mem::size_of::<Selector>();
            bytes += rule.declarations.capacity() * std::mem::size_of::<ParsedDeclaration>();
            for selector in &rule.selectors {
                bytes += selector.kind.as_ref().map_or(0, String::capacity);
                bytes += selector.id.as_ref().map_or(0, String::capacity);
                bytes += selector.classes.capacity() * std::mem::size_of::<String>();
                bytes += selector.classes.iter().map(String::capacity).sum::<usize>();
            }
            for declaration in &rule.declarations {
                let (name, value) = match declaration {
                    ParsedDeclaration::Custom(name, value)
                    | ParsedDeclaration::Property(name, value) => (name, value),
                };
                bytes += name.capacity() + value.capacity();
            }
        }
        bytes as u64
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn descendant_rules_match_ordered_ancestors_without_leaking() {
        let css = super::StyleSheet::compile("window.launcher button { background: #123456; } window.settings button { background: #abcdef; }").unwrap();
        let launcher = vec![
            ("window".into(), None, Some("launcher".into())),
            ("div".into(), None, None),
        ];
        let settings = vec![("window".into(), None, Some("settings".into()))];
        assert_eq!(
            css.resolve_with_ancestors("button", None, None, &Default::default(), &launcher)
                .background,
            Some(0xff123456)
        );
        assert_eq!(
            css.resolve_with_ancestors("button", None, None, &Default::default(), &settings)
                .background,
            Some(0xffabcdef)
        );
        assert_eq!(css.resolve("button", None, None).background, None);
    }
    use super::*;

    #[test]
    fn palette_color_tokens_recompile_for_theme_changes() {
        let dark = ThemePalette::from_appearance(Appearance::default());
        let light = ThemePalette::from_appearance(Appearance {
            mode: nickel_core::theme::ThemeMode::Light,
            ..Appearance::default()
        });
        let mut sheet = StyleSheet::compile_with_palette(
            "window { background: var(--nickel-panel); border: 1px solid var(--nickel-muted); } text { color: var(--nickel-text); }",
            dark,
        )
        .unwrap();
        assert_eq!(
            sheet.resolve("window", None, None).background,
            Some(0xff00_0000 | dark.panel)
        );
        assert_eq!(
            sheet.resolve("text", None, None).color,
            Some(0xff00_0000 | dark.text)
        );
        assert!(sheet.set_reading_direction(ReadingDirection::RightToLeft));
        assert!(sheet.set_palette(light).unwrap());
        assert_eq!(sheet.reading_direction(), ReadingDirection::RightToLeft);
        assert_eq!(
            sheet.resolve("window", None, None).background,
            Some(0xff00_0000 | light.panel)
        );
        assert_eq!(
            sheet.resolve("text", None, None).color,
            Some(0xff00_0000 | light.text)
        );
        assert!(!sheet.set_palette(light).unwrap());
        assert!(StyleSheet::compile("text { color: var(--unknown); }").is_err());
    }

    #[test]
    fn custom_properties_cascade_and_inherit_into_typed_declarations() {
        let css = StyleSheet::compile(
            ":root { --theme-text: #112233; --space: 4px; }
             div.parent { --theme-text: #445566; --space: 12px; }
             text { color: var(--theme-text); padding: var(--space); }
             text.local { --theme-text: #778899; }",
        )
        .unwrap();

        let root_text = css.resolve("text", None, None);
        assert_eq!(root_text.color, Some(0xff11_2233));
        assert_eq!(root_text.padding, Some(Insets::all(4.0)));

        let parent = css.resolve("div", None, Some("parent"));
        let child =
            css.resolve_with_custom_properties("text", None, None, &parent.custom_properties);
        assert_eq!(child.color, Some(0xff44_5566));
        assert_eq!(child.padding, Some(Insets::all(12.0)));

        let local = css.resolve_with_custom_properties(
            "text",
            None,
            Some("local"),
            &parent.custom_properties,
        );
        assert_eq!(local.color, Some(0xff77_8899));
    }

    #[test]
    fn var_fallbacks_nest_and_semantic_defaults_remain_overridable() {
        let default =
            StyleSheet::compile("text { color: var(--missing, var(--nickel-text)); }").unwrap();
        assert_eq!(
            default.resolve("text", None, None).color,
            Some(
                0xff00_0000
                    | ThemePalette::from_appearance(Appearance::default()).text & 0x00ff_ffff
            )
        );

        let overridden = StyleSheet::compile(
            ":root { --nickel-text: #abcdef; }
             text { color: var(--missing, var(--nickel-text)); }",
        )
        .unwrap();
        assert_eq!(
            overridden.resolve("text", None, None).color,
            Some(0xffab_cdef)
        );
    }

    #[test]
    fn computed_variables_and_interaction_colors_inherit_without_sibling_leaks() {
        let css = StyleSheet::compile(
            "div { --base: #112233; --alias: var(--base); }
             div.other { --base: 12px; }
             button { --base: #445566; color: var(--alias); }
             button:hover { background: var(--alias); }",
        )
        .unwrap();
        let parent = css.resolve("div", None, None);
        let child =
            css.resolve_with_custom_properties("button", None, None, &parent.custom_properties);
        assert_eq!(child.color, Some(0xff11_2233));
        assert_eq!(
            css.resolve_interaction_background_with_properties(
                "button",
                None,
                None,
                InteractionState::Hover,
                &child.custom_properties,
                &[],
            ),
            Some(0xff11_2233)
        );
    }

    #[test]
    fn semantic_metrics_and_variable_chain_limits_are_available() {
        let css = StyleSheet::compile(
            "text { font-size: var(--nickel-font-size); line-height: var(--nickel-line-height); }
             button { padding: var(--nickel-spacing-control); border-radius: var(--nickel-radius-control); }",
        ).unwrap();
        assert_eq!(css.resolve("text", None, None).font_size, Some(14.0));
        assert_eq!(css.resolve("button", None, None).radius, Some(8.0));
        let mut source = String::from(":root { --v0: #112233;");
        for index in 1..40 {
            source.push_str(&format!("--v{index}: var(--v{});", index - 1));
        }
        source.push_str("} text { color: var(--v39); }");
        assert!(StyleSheet::compile(&source).is_err());
    }

    #[test]
    fn invalid_custom_property_resolution_is_rejected() {
        for css in [
            ":root { --a: var(--b); --b: var(--a); } text { color: var(--a); }",
            ":root { --size: 9000px; } div { width: var(--size); }",
            ":root { --color: 12px; } text { color: var(--color); }",
            "text { color: var(--missing); }",
            "text { color: var(--missing, var(--also-missing)); }",
        ] {
            assert!(StyleSheet::compile(css).is_err(), "{css}");
        }
    }

    #[test]
    fn classes_and_type_selectors_apply_in_order() {
        let css = StyleSheet::compile("button { padding: 2px 4px; color: #abc; } button.primary { padding: 8px; border-radius: 6px; }").unwrap();
        let style = css.resolve("button", None, Some("primary compact"));
        assert_eq!(style.padding, Some(Insets::all(8.0)));
        assert_eq!(style.color, Some(0xffaabbcc));
        assert_eq!(style.radius, Some(6.0));
        assert_eq!(
            css.resolve("text-field", None, Some("primary")).padding,
            None
        );
    }

    #[test]
    fn invalid_and_unbounded_css_is_rejected() {
        for source in [
            "button { unknown: 4px }",
            "button { padding: 10000px }",
            "@import 'remote.css';",
            "div:hover { background: #fff }",
        ] {
            assert!(StyleSheet::compile(source).is_err(), "{source}");
        }
    }

    #[test]
    fn interaction_backgrounds_are_separate_from_base_styles() {
        let css = StyleSheet::compile(
            "button { background: #111; } button.primary:hover { background: #222; } \
             button.primary:active { background: #333; } \
             button.primary:focus { background: #444; } \
             text-field.entry:focus { background: #555; }",
        )
        .unwrap();
        assert_eq!(
            css.resolve("button", None, Some("primary")).background,
            Some(0xff111111)
        );
        assert_eq!(
            css.resolve_interaction_background(
                "button",
                None,
                Some("primary"),
                InteractionState::Hover
            ),
            Some(0xff222222)
        );
        assert_eq!(
            css.resolve_interaction_background(
                "button",
                None,
                Some("primary"),
                InteractionState::Active
            ),
            Some(0xff333333)
        );
        assert_eq!(
            css.resolve_interaction_background(
                "button",
                None,
                Some("primary"),
                InteractionState::Focus
            ),
            Some(0xff444444)
        );
        assert_eq!(
            css.resolve_interaction_background(
                "text-field",
                None,
                Some("entry"),
                InteractionState::Focus
            ),
            Some(0xff555555)
        );
        assert_eq!(
            css.resolve_interaction_background(
                "button",
                None,
                Some("other"),
                InteractionState::Hover
            ),
            None
        );
    }

    #[test]
    fn rgba_and_border_shorthand_compile_to_typed_colors() {
        let css = StyleSheet::compile(
            ".card { background: rgba(10, 20, 30, 0.5); border: 2px solid #abc; gap: 12px; }",
        )
        .unwrap();
        let style = css.resolve("row", None, Some("card"));
        assert_eq!(style.background, Some(0x800a141e));
        assert_eq!(style.border_color, Some(0xffaabbcc));
        assert_eq!(style.border_width, Some(2.0));
        assert_eq!(style.gap, Some(12.0));
        assert!(css.estimated_retained_bytes() > 0);
    }

    #[test]
    fn familiar_flex_grid_and_size_declarations_map_to_nickel_layout() {
        let css = StyleSheet::compile(
            ".row { display: flex; flex-direction: row; width: 100%; gap: 8px; align-items: center; justify-content: space-between; } \
             .item { flex: 1; min-width: 20px; max-width: 120px; } \
             .grid { display: grid; grid-template-columns: repeat(2, minmax(40px, 1fr)); }",
        )
        .unwrap();
        let row = css.resolve("div", None, Some("row"));
        assert_eq!(row.display, Some(Display::Flex));
        assert_eq!(row.flex_direction, Some(FlexDirection::Row));
        assert_eq!(row.width, Some(Length::Percent(1.0)));
        assert_eq!(row.align_items, Some(Align::Center));
        assert_eq!(row.justify_content, Some(Justify::SpaceBetween));
        let item = css.resolve("div", None, Some("item"));
        assert_eq!(item.grow, Some(1.0));
        assert_eq!(item.min_width, Some(20.0));
        assert_eq!(item.max_width, Some(120.0));
        let grid = css.resolve("div", None, Some("grid"));
        assert_eq!(grid.display, Some(Display::Grid));
        assert_eq!(
            grid.grid_columns,
            Some(vec![Track::repeat(
                2,
                Track::minmax(Track::Px(40.0), Track::Fraction(1.0))
            )])
        );
        let auto = StyleSheet::compile(
            ".grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(110px, 1fr)); }",
        )
        .unwrap();
        assert_eq!(
            auto.resolve("div", None, Some("grid")).grid_columns,
            Some(vec![Track::repeat_auto_fit(Track::minmax(
                Track::Px(110.0),
                Track::Fraction(1.0)
            ))])
        );
    }

    #[test]
    fn invalid_layout_values_are_rejected() {
        for css in [
            ".x { width: 200%; }",
            ".x { grid-template-columns: repeat(100, 1fr); }",
            ".x { grid-template-columns: repeat(32, repeat(32, 1fr)); }",
            ".x { flex-direction: diagonal; }",
            ".x { grid-template-columns: minmax(1fr, 40px; }",
        ] {
            assert!(StyleSheet::compile(css).is_err(), "{css}");
        }
    }

    #[test]
    fn positioned_window_distances_are_bounded_and_require_window_selector() {
        let stylesheet = StyleSheet::compile("window.dock { bottom: 20px; top: 12px; }").unwrap();
        assert_eq!(
            stylesheet.resolve("window", None, Some("dock")).bottom,
            Some(20.0)
        );
        assert_eq!(
            stylesheet.resolve("window", None, Some("dock")).top,
            Some(12.0)
        );
        for css in [
            ".dock { bottom: 20px; }",
            "button { bottom: 20px; }",
            ".notice { top: 12px; }",
            "text { top: 12px; }",
            "window.dock { bottom: -1px; }",
            "window.dock { bottom: 9000px; }",
            "window.notice { top: -1px; }",
            "window.notice { top: 9000px; }",
        ] {
            assert!(StyleSheet::compile(css).is_err(), "{css}");
        }
    }

    #[test]
    fn bundled_taskbar_stylesheet_compiles() {
        let source = include_str!("../../../assets/plugins/taskbar/ui.css");
        StyleSheet::compile(source).unwrap();
    }

    #[test]
    fn stock_shell_launcher_stylesheet_compiles() {
        let source = include_str!("../../../assets/plugins/nickel-default/src/styles/launcher.css");
        StyleSheet::compile(source).unwrap();
    }
}
