//! Bounded CSS subset for plugin presentation. Rules are compiled once at activation.

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser,
};
use nickel_ui::Insets;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ControlStyle {
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
}

#[derive(Clone, Debug)]
struct Selector {
    kind: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
}

impl Selector {
    fn parse(source: &str) -> Result<Self, String> {
        let mut parser = Parser::new(source);
        let mut selector = Self {
            kind: None,
            id: None,
            classes: Vec::new(),
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
                _ => return Err("unsupported plugin CSS selector".into()),
            }
        }
        if selector.kind.is_none() && selector.id.is_none() && selector.classes.is_empty() {
            return Err("empty plugin CSS selector".into());
        }
        Ok(selector)
    }

    fn matches(&self, kind: &str, id: Option<&str>, class_name: Option<&str>) -> bool {
        self.kind.as_deref().is_none_or(|selector| selector == kind)
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
    declarations: Vec<Declaration>,
}

#[derive(Clone, Debug)]
enum Declaration {
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
    Border(f32, u32),
}

impl Declaration {
    fn apply(&self, style: &mut ControlStyle) {
        match self {
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

fn color(source: &str) -> Result<u32, String> {
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
        Ok(Rule {
            selectors,
            declarations,
        })
    }
}

struct CssDeclarationParser;
impl<'i> AtRuleParser<'i> for CssDeclarationParser {
    type Prelude = ();
    type AtRule = Declaration;
    type Error = String;
}
impl<'i> DeclarationParser<'i> for CssDeclarationParser {
    type Declaration = Declaration;
    type Error = String;
    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _start: &ParserState,
    ) -> Result<Self::Declaration, ParseError<Self::Error>> {
        let start = input.position();
        while input.next_including_whitespace_and_comments().is_ok() {}
        declaration(&name, input.slice_from(start)).map_err(ParseError::custom)
    }
}
impl<'i> QualifiedRuleParser<'i> for CssDeclarationParser {
    type Prelude = ();
    type QualifiedRule = Declaration;
    type Error = String;
}
impl<'i> RuleBodyItemParser<'i, Declaration, String> for CssDeclarationParser {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct StyleSheet {
    rules: Vec<Rule>,
}

impl StyleSheet {
    pub fn compile(source: &str) -> Result<Self, String> {
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
        Ok(Self { rules })
    }

    pub fn resolve(&self, kind: &str, id: Option<&str>, class_name: Option<&str>) -> ControlStyle {
        let mut style = ControlStyle::default();
        for rule in &self.rules {
            if rule
                .selectors
                .iter()
                .any(|selector| selector.matches(kind, id, class_name))
            {
                for declaration in &rule.declarations {
                    declaration.apply(&mut style);
                }
            }
        }
        style
    }

    pub fn estimated_retained_bytes(&self) -> u64 {
        let mut bytes = self.rules.capacity() * std::mem::size_of::<Rule>();
        for rule in &self.rules {
            bytes += rule.selectors.capacity() * std::mem::size_of::<Selector>();
            bytes += rule.declarations.capacity() * std::mem::size_of::<Declaration>();
            for selector in &rule.selectors {
                bytes += selector.kind.as_ref().map_or(0, String::capacity);
                bytes += selector.id.as_ref().map_or(0, String::capacity);
                bytes += selector.classes.capacity() * std::mem::size_of::<String>();
                bytes += selector.classes.iter().map(String::capacity).sum::<usize>();
            }
        }
        bytes as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "button:hover { color: #fff }",
        ] {
            assert!(StyleSheet::compile(source).is_err(), "{source}");
        }
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
}
