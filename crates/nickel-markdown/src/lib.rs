//! Safe typed Markdown documents and declarative Nickel UI presentation.

use std::iter::Peekable;

use nickel_ui::{
    Align, AnyView, Button, Color, Column, ComponentBuilderExt, Container, Insets, Length,
    Overflow, Row, SelectionRegion, StyledText, StyledTextSpan, Text, TextBoundary, UiId, ui,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MarkdownDocument {
    pub source: String,
    pub blocks: Vec<Block>,
    pub diagnostics: Vec<MarkdownDiagnostic>,
}

impl MarkdownDocument {
    #[must_use]
    pub fn parse(source: impl Into<String>) -> Self {
        let source = source.into();
        let options = Options::ENABLE_STRIKETHROUGH;
        let mut events = Parser::new_ext(&source, options).peekable();
        let mut diagnostics = Vec::new();
        let blocks = parse_blocks(&mut events, None, &mut diagnostics);
        Self {
            source,
            blocks,
            diagnostics,
        }
    }

    #[must_use]
    pub fn logical_text(&self) -> String {
        blocks_text(&self.blocks)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Block {
    Paragraph {
        inlines: Vec<Inline>,
    },
    Heading {
        level: u8,
        anchor: String,
        inlines: Vec<Inline>,
    },
    Code {
        language: Option<String>,
        text: String,
    },
    Quote {
        blocks: Vec<Block>,
    },
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    ThematicBreak,
    Unsupported {
        text: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Inline {
    Text {
        text: String,
    },
    Emphasis {
        children: Vec<Inline>,
    },
    Strong {
        children: Vec<Inline>,
    },
    Strikethrough {
        children: Vec<Inline>,
    },
    Code {
        text: String,
    },
    Link {
        destination: String,
        title: String,
        children: Vec<Inline>,
    },
    Break {
        hard: bool,
    },
    Unsupported {
        text: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MarkdownDiagnostic {
    pub kind: DiagnosticKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticKind {
    UnsupportedMarkup,
}

fn parse_blocks<'a, I>(
    events: &mut Peekable<I>,
    until: Option<TagEnd>,
    diagnostics: &mut Vec<MarkdownDiagnostic>,
) -> Vec<Block>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut blocks = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::End(end) if Some(end) == until => break,
            Event::Start(Tag::Paragraph) => blocks.push(Block::Paragraph {
                inlines: parse_inlines(events, TagEnd::Paragraph, diagnostics),
            }),
            Event::Start(Tag::Heading { level, .. }) => {
                let inlines = parse_inlines(events, TagEnd::Heading(level), diagnostics);
                blocks.push(Block::Heading {
                    level: heading_level(level),
                    anchor: heading_anchor(&inline_text(&inlines)),
                    inlines,
                });
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                blocks.push(parse_code(events, kind));
            }
            Event::Start(Tag::BlockQuote(kind)) => blocks.push(Block::Quote {
                blocks: parse_blocks(events, Some(TagEnd::BlockQuote(kind)), diagnostics),
            }),
            Event::Start(Tag::List(start)) => {
                blocks.push(parse_list(events, start, diagnostics));
            }
            Event::Start(Tag::HtmlBlock) => {
                let text = collect_literal(events, TagEnd::HtmlBlock);
                diagnostics.push(unsupported("raw HTML block"));
                blocks.push(Block::Unsupported { text });
            }
            Event::Rule => blocks.push(Block::ThematicBreak),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                blocks.push(Block::Unsupported {
                    text: text.into_string(),
                });
            }
            Event::Start(tag) => {
                let end = tag.to_end();
                let text = collect_literal(events, end);
                diagnostics.push(unsupported(&format!("{end:?}")));
                blocks.push(Block::Unsupported { text });
            }
            Event::End(_) => {}
            Event::Code(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text)
            | Event::FootnoteReference(text) => blocks.push(Block::Unsupported {
                text: text.into_string(),
            }),
            Event::SoftBreak | Event::HardBreak => {
                blocks.push(Block::Unsupported { text: "\n".into() })
            }
            Event::TaskListMarker(checked) => blocks.push(Block::Unsupported {
                text: if checked { "[x]" } else { "[ ]" }.into(),
            }),
        }
    }
    blocks
}

fn parse_list<'a, I>(
    events: &mut Peekable<I>,
    start: Option<u64>,
    diagnostics: &mut Vec<MarkdownDiagnostic>,
) -> Block
where
    I: Iterator<Item = Event<'a>>,
{
    let mut items = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::Item) => {
                items.push(parse_blocks(events, Some(TagEnd::Item), diagnostics));
            }
            Event::End(TagEnd::List(_)) => break,
            Event::Text(text) if !text.trim().is_empty() => {
                items.push(vec![Block::Unsupported {
                    text: text.into_string(),
                }]);
            }
            _ => {}
        }
    }
    Block::List { start, items }
}

fn parse_code<'a, I>(events: &mut Peekable<I>, kind: CodeBlockKind<'a>) -> Block
where
    I: Iterator<Item = Event<'a>>,
{
    let language = match kind {
        CodeBlockKind::Indented => None,
        CodeBlockKind::Fenced(label) => label
            .split_whitespace()
            .next()
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
    };
    Block::Code {
        language,
        text: collect_literal(events, TagEnd::CodeBlock),
    }
}

fn parse_inlines<'a, I>(
    events: &mut Peekable<I>,
    until: TagEnd,
    diagnostics: &mut Vec<MarkdownDiagnostic>,
) -> Vec<Inline>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut inlines = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::End(end) if end == until => break,
            Event::Text(text) => push_text(&mut inlines, text.into_string()),
            Event::Code(text) => inlines.push(Inline::Code {
                text: text.into_string(),
            }),
            Event::SoftBreak => inlines.push(Inline::Break { hard: false }),
            Event::HardBreak => inlines.push(Inline::Break { hard: true }),
            Event::Start(Tag::Emphasis) => inlines.push(Inline::Emphasis {
                children: parse_inlines(events, TagEnd::Emphasis, diagnostics),
            }),
            Event::Start(Tag::Strong) => inlines.push(Inline::Strong {
                children: parse_inlines(events, TagEnd::Strong, diagnostics),
            }),
            Event::Start(Tag::Strikethrough) => inlines.push(Inline::Strikethrough {
                children: parse_inlines(events, TagEnd::Strikethrough, diagnostics),
            }),
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) => inlines.push(Inline::Link {
                destination: dest_url.into_string(),
                title: title.into_string(),
                children: parse_inlines(events, TagEnd::Link, diagnostics),
            }),
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                let alt = parse_inlines(events, TagEnd::Image, diagnostics);
                let label = inline_text(&alt);
                let destination = dest_url.into_string();
                let title = title.into_string();
                inlines.push(Inline::Unsupported {
                    text: format!("![{label}]({destination}{})", title_suffix(&title)),
                });
                diagnostics.push(unsupported("image"));
            }
            Event::Html(text) | Event::InlineHtml(text) => {
                inlines.push(Inline::Unsupported {
                    text: text.into_string(),
                });
                diagnostics.push(unsupported("raw HTML"));
            }
            Event::Start(tag) => {
                let end = tag.to_end();
                let text = collect_literal(events, end);
                inlines.push(Inline::Unsupported { text });
                diagnostics.push(unsupported(&format!("{end:?}")));
            }
            Event::InlineMath(text) | Event::DisplayMath(text) | Event::FootnoteReference(text) => {
                inlines.push(Inline::Unsupported {
                    text: text.into_string(),
                });
                diagnostics.push(unsupported("extension markup"));
            }
            Event::TaskListMarker(checked) => inlines.push(Inline::Unsupported {
                text: if checked { "[x] " } else { "[ ] " }.into(),
            }),
            Event::Rule => inlines.push(Inline::Unsupported { text: "---".into() }),
            Event::End(_) => {}
        }
    }
    inlines
}

fn collect_literal<'a, I>(events: &mut Peekable<I>, until: TagEnd) -> String
where
    I: Iterator<Item = Event<'a>>,
{
    let mut text = String::new();
    let mut depth = 0_usize;
    for event in events.by_ref() {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(end) if depth == 0 && end == until => break,
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Text(value)
            | Event::Code(value)
            | Event::Html(value)
            | Event::InlineHtml(value)
            | Event::InlineMath(value)
            | Event::DisplayMath(value)
            | Event::FootnoteReference(value) => text.push_str(&value),
            Event::SoftBreak | Event::HardBreak => text.push('\n'),
            Event::Rule => text.push_str("---"),
            Event::TaskListMarker(checked) => {
                text.push_str(if checked { "[x] " } else { "[ ] " });
            }
        }
    }
    text
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn heading_anchor(text: &str) -> String {
    let mut anchor = String::new();
    let mut pending_dash = false;
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            if pending_dash && !anchor.is_empty() {
                anchor.push('-');
            }
            pending_dash = false;
            anchor.push(character);
        } else {
            pending_dash = true;
        }
    }
    anchor
}

fn push_text(inlines: &mut Vec<Inline>, text: String) {
    if let Some(Inline::Text { text: previous }) = inlines.last_mut() {
        previous.push_str(&text);
    } else {
        inlines.push(Inline::Text { text });
    }
}

fn inline_text(inlines: &[Inline]) -> String {
    let mut output = String::new();
    for inline in inlines {
        match inline {
            Inline::Text { text } | Inline::Code { text } | Inline::Unsupported { text } => {
                output.push_str(text)
            }
            Inline::Emphasis { children }
            | Inline::Strong { children }
            | Inline::Strikethrough { children }
            | Inline::Link { children, .. } => output.push_str(&inline_text(children)),
            Inline::Break { hard } => output.push(if *hard { '\n' } else { ' ' }),
        }
    }
    output
}

fn blocks_text(blocks: &[Block]) -> String {
    let mut output = String::new();
    for block in blocks {
        if !output.is_empty() {
            output.push('\n');
        }
        match block {
            Block::Paragraph { inlines } | Block::Heading { inlines, .. } => {
                output.push_str(&inline_text(inlines));
            }
            Block::Code { text, .. } | Block::Unsupported { text } => output.push_str(text),
            Block::Quote { blocks } => output.push_str(&blocks_text(blocks)),
            Block::List { start, items } => {
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        output.push('\n');
                    }
                    if let Some(start) = start {
                        output.push_str(&format!("{}. ", start + index as u64));
                    } else {
                        output.push_str("• ");
                    }
                    output.push_str(&blocks_text(item));
                }
            }
            Block::ThematicBreak => output.push_str("---"),
        }
    }
    output
}

fn title_suffix(title: &str) -> String {
    if title.is_empty() {
        String::new()
    } else {
        format!(" \"{title}\"")
    }
}

fn unsupported(name: &str) -> MarkdownDiagnostic {
    MarkdownDiagnostic {
        kind: DiagnosticKind::UnsupportedMarkup,
        message: format!("unsupported Markdown remains inert text: {name}"),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MarkdownPalette {
    pub foreground: Color,
    pub muted: Color,
    pub accent: Color,
    pub surface: Color,
    pub border: Color,
    pub code: Color,
}

impl Default for MarkdownPalette {
    fn default() -> Self {
        Self {
            foreground: 0xe7ebf2,
            muted: 0x9aa7bd,
            accent: 0x6ca6e8,
            surface: 0x161b22,
            border: 0x30363d,
            code: 0xc9d1d9,
        }
    }
}

/// Render a parsed document as one selectable, declarative Nickel UI stream.
///
/// Link labels remain part of selectable prose. Each destination is also exposed
/// as an explicit typed activation control immediately after its containing block.
pub fn markdown_view<Message, Map>(
    document: &MarkdownDocument,
    palette: MarkdownPalette,
    map_link: Map,
) -> AnyView<Message>
where
    Message: Clone + 'static,
    Map: Fn(&str) -> Message + Copy,
{
    let blocks = document
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| render_block(block, index.to_string(), palette, map_link));
    let document = SelectionRegion::automatic()
        .id(UiId::new("markdown-document"))
        .child(Column::new().fill_width().gap(12.0).children(blocks));
    AnyView::new(ui! { {document} })
}

fn render_block<Message, Map>(
    block: &Block,
    id: String,
    palette: MarkdownPalette,
    map_link: Map,
) -> AnyView<Message>
where
    Message: Clone + 'static,
    Map: Fn(&str) -> Message + Copy,
{
    match block {
        Block::Paragraph { inlines } => render_prose(inlines, id, 1.0, false, palette, map_link),
        Block::Heading { level, inlines, .. } => {
            render_prose(inlines, id, heading_scale(*level), true, palette, map_link)
        }
        Block::Code { language, text } => {
            let language = language.as_deref().unwrap_or("code");
            AnyView::new(
                Container::new()
                    .fill_width()
                    .padding(Insets::all(10.0))
                    .gap(7.0)
                    .background(palette.surface)
                    .border(palette.border, 1.0)
                    .radius(6.0)
                    .child(
                        Text::new(language)
                            .color(palette.muted)
                            .scale(0.8)
                            .selectable(false),
                    )
                    .child(
                        Row::new()
                            .id(UiId::new(format!("markdown-code-{id}")))
                            .fill_width()
                            .overflow_x(Overflow::Auto)
                            .child(
                                Text::new(text)
                                    .color(palette.code)
                                    .selection_run_id(format!("markdown-{id}"))
                                    .selection_boundary(TextBoundary::Block)
                                    .align_self(Align::Start),
                            ),
                    ),
            )
        }
        Block::Quote { blocks } => {
            let children = blocks.iter().enumerate().map(|(index, block)| {
                render_block(block, format!("{id}-{index}"), palette, map_link)
            });
            AnyView::new(
                Container::new()
                    .fill_width()
                    .padding(Insets::all(12.0))
                    .gap(8.0)
                    .background(palette.surface)
                    .border(palette.border, 1.0)
                    .child(Column::new().fill_width().gap(8.0).children(children)),
            )
        }
        Block::List { start, items } => {
            let rows = items.iter().enumerate().map(|(index, blocks)| {
                let marker = start.map_or_else(
                    || "•".to_owned(),
                    |start| format!("{}.", start + index as u64),
                );
                let children = blocks.iter().enumerate().map(|(child, block)| {
                    render_block(block, format!("{id}-{index}-{child}"), palette, map_link)
                });
                AnyView::new(
                    Row::new()
                        .fill_width()
                        .gap(9.0)
                        .align_items(Align::Start)
                        .child(Text::new(marker).color(palette.muted).selectable(false))
                        .child(Column::new().fill_width().gap(6.0).children(children)),
                )
            });
            AnyView::new(Column::new().fill_width().gap(7.0).children(rows))
        }
        Block::ThematicBreak => AnyView::new(
            Container::new()
                .fill_width()
                .height(1.0)
                .background(palette.border),
        ),
        Block::Unsupported { text } => AnyView::new(
            Text::new(text)
                .color(palette.foreground)
                .width_length(Length::Fill)
                .wrap(true)
                .selection_run_id(format!("markdown-{id}"))
                .selection_boundary(TextBoundary::Block),
        ),
    }
}

fn render_prose<Message, Map>(
    inlines: &[Inline],
    id: String,
    scale: f32,
    bold: bool,
    palette: MarkdownPalette,
    map_link: Map,
) -> AnyView<Message>
where
    Message: Clone + 'static,
    Map: Fn(&str) -> Message + Copy,
{
    let (text, spans) = styled_inline_text(inlines, palette, bold);
    let links = inline_links(inlines);
    let controls = links
        .into_iter()
        .enumerate()
        .map(|(index, (label, destination))| {
            AnyView::new(
                Button::new(map_link(&destination), format!("{label}  ↗"))
                    .id(UiId::new(format!("markdown-link-{id}-{index}")))
                    .background(palette.surface)
                    .color(palette.accent)
                    .align_self(Align::Start),
            )
        });
    AnyView::new(
        Column::new()
            .fill_width()
            .gap(6.0)
            .child(
                StyledText::new(text, spans)
                    .color(palette.foreground)
                    .scale(scale)
                    .width_length(Length::Fill)
                    .wrap(true)
                    .selection_run_id(format!("markdown-{id}"))
                    .selection_boundary(TextBoundary::Block),
            )
            .children(controls),
    )
}

#[derive(Clone, Copy, Default)]
struct InlineStyle {
    bold: bool,
    italic: bool,
    monospace: bool,
    strikethrough: bool,
    color: Option<Color>,
}

fn styled_inline_text(
    inlines: &[Inline],
    palette: MarkdownPalette,
    bold: bool,
) -> (String, Vec<StyledTextSpan>) {
    let mut text = String::new();
    let mut spans = Vec::new();
    append_styled_inlines(
        inlines,
        InlineStyle {
            bold,
            ..InlineStyle::default()
        },
        palette,
        &mut text,
        &mut spans,
    );
    (text, spans)
}

fn append_styled_inlines(
    inlines: &[Inline],
    style: InlineStyle,
    palette: MarkdownPalette,
    text: &mut String,
    spans: &mut Vec<StyledTextSpan>,
) {
    for inline in inlines {
        match inline {
            Inline::Text { text: value } | Inline::Unsupported { text: value } => {
                push_styled_text(value, style, text, spans);
            }
            Inline::Code { text: value } => {
                push_styled_text(
                    value,
                    InlineStyle {
                        monospace: true,
                        color: Some(palette.code),
                        ..style
                    },
                    text,
                    spans,
                );
            }
            Inline::Emphasis { children } => append_styled_inlines(
                children,
                InlineStyle {
                    italic: true,
                    ..style
                },
                palette,
                text,
                spans,
            ),
            Inline::Strong { children } => append_styled_inlines(
                children,
                InlineStyle {
                    bold: true,
                    ..style
                },
                palette,
                text,
                spans,
            ),
            Inline::Strikethrough { children } => append_styled_inlines(
                children,
                InlineStyle {
                    strikethrough: true,
                    ..style
                },
                palette,
                text,
                spans,
            ),
            Inline::Link { children, .. } => append_styled_inlines(
                children,
                InlineStyle {
                    color: Some(palette.accent),
                    ..style
                },
                palette,
                text,
                spans,
            ),
            Inline::Break { hard } => text.push(if *hard { '\n' } else { ' ' }),
        }
    }
}

fn push_styled_text(
    value: &str,
    style: InlineStyle,
    text: &mut String,
    spans: &mut Vec<StyledTextSpan>,
) {
    let start = text.len();
    text.push_str(value);
    let end = text.len();
    if end > start
        && (style.bold
            || style.italic
            || style.monospace
            || style.strikethrough
            || style.color.is_some())
    {
        spans.push(StyledTextSpan {
            range: start..end,
            bold: style.bold,
            italic: style.italic,
            monospace: style.monospace,
            strikethrough: style.strikethrough,
            color: style.color,
        });
    }
}

fn display_inline_text(inlines: &[Inline]) -> String {
    let mut output = String::new();
    for inline in inlines {
        match inline {
            Inline::Text { text } | Inline::Unsupported { text } => output.push_str(text),
            Inline::Emphasis { children } => {
                output.push('*');
                output.push_str(&display_inline_text(children));
                output.push('*');
            }
            Inline::Strong { children } => {
                output.push_str("**");
                output.push_str(&display_inline_text(children));
                output.push_str("**");
            }
            Inline::Strikethrough { children } => {
                output.push_str("~~");
                output.push_str(&display_inline_text(children));
                output.push_str("~~");
            }
            Inline::Code { text } => {
                output.push('‹');
                output.push_str(text);
                output.push('›');
            }
            Inline::Link { children, .. } => output.push_str(&display_inline_text(children)),
            Inline::Break { hard } => output.push(if *hard { '\n' } else { ' ' }),
        }
    }
    output
}

fn inline_links(inlines: &[Inline]) -> Vec<(String, String)> {
    let mut links = Vec::new();
    collect_inline_links(inlines, &mut links);
    links
}

fn collect_inline_links(inlines: &[Inline], links: &mut Vec<(String, String)>) {
    for inline in inlines {
        match inline {
            Inline::Link {
                destination,
                children,
                ..
            } => {
                links.push((display_inline_text(children), destination.clone()));
                collect_inline_links(children, links);
            }
            Inline::Emphasis { children }
            | Inline::Strong { children }
            | Inline::Strikethrough { children } => collect_inline_links(children, links),
            _ => {}
        }
    }
}

fn heading_scale(level: u8) -> f32 {
    match level {
        1 => 1.8,
        2 => 1.55,
        3 => 1.35,
        4 => 1.2,
        5 => 1.08,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_ui::{
        PaintCommand, Point, Rect, SdlComponentRenderer, UiEvent, UiStateStore, UiTree,
    };

    #[derive(Clone, Debug, PartialEq)]
    enum Message {
        Link(String),
    }

    #[test]
    fn parses_admitted_blocks_and_nested_inline_content() {
        let document = MarkdownDocument::parse(
            "# Hello *there*\n\nA **bold** [link](https://example.com) and ~~old~~ `code`.\n\n> quoted\n\n1. one\n2. two\n\n```rust\nfn main() {}\n```\n",
        );
        assert!(document.diagnostics.is_empty());
        assert!(matches!(
            &document.blocks[0],
            Block::Heading { level: 1, anchor, .. } if anchor == "hello-there"
        ));
        assert!(matches!(&document.blocks[2], Block::Quote { .. }));
        assert!(matches!(
            &document.blocks[3],
            Block::List { start: Some(1), items } if items.len() == 2
        ));
        assert!(matches!(
            &document.blocks[4],
            Block::Code { language: Some(language), text } if language == "rust" && text == "fn main() {}\n"
        ));
    }

    #[test]
    fn raw_html_and_images_remain_visible_and_inert() {
        let document = MarkdownDocument::parse("<b>plain</b> ![cat](cat.png \"Cat\")");
        assert_eq!(
            document.logical_text(),
            "<b>plain</b> ![cat](cat.png \"Cat\")"
        );
        assert_eq!(document.diagnostics.len(), 3);
    }

    #[test]
    fn serialization_is_stable_for_equal_input() {
        let left = MarkdownDocument::parse("## Café\n\n- α\n- β");
        let right = MarkdownDocument::parse("## Café\n\n- α\n- β");
        assert_eq!(
            serde_json::to_vec_pretty(&left).unwrap(),
            serde_json::to_vec_pretty(&right).unwrap()
        );
    }

    #[test]
    fn unsupported_unicode_bidi_deep_and_long_content_stays_readable() {
        let nested = (0..64)
            .map(|depth| format!("{}- level {depth}\n", "  ".repeat(depth)))
            .collect::<String>();
        let long = "long ".repeat(10_000);
        let source = format!(
            "<script>alert('inert')</script>\n\n| table | source |\n| --- | --- |\n| remains | visible |\n\nשלום العربية e\u{202e}abc\u{202c}\n\n{nested}\n{long}\n\n![alt](image.png) $math$ [^note]"
        );
        let document = MarkdownDocument::parse(source);
        let logical = document.logical_text();
        for expected in [
            "<script>",
            "table",
            "שלום",
            "العربية",
            "level 63",
            "long long long",
            "![alt](image.png)",
            "$math$",
            "[^note]",
        ] {
            assert!(
                logical.contains(expected),
                "missing fallback text: {expected}"
            );
        }
    }

    #[test]
    fn repeated_rendering_has_identical_typed_paint_commands() {
        let document = MarkdownDocument::parse(
            "# Stable\n\n**bold** *italic* ~~strike~~ `code` [link](guide.md)",
        );
        let render = || {
            UiTree::layout(
                markdown_view(&document, MarkdownPalette::default(), |destination| {
                    Message::Link(destination.to_owned())
                }),
                Rect::new(0.0, 0.0, 640.0, 480.0),
            )
            .commands()
            .to_vec()
        };
        assert_eq!(render(), render());
    }

    #[test]
    fn inline_styles_are_real_spans_without_source_markers() {
        let document = MarkdownDocument::parse(
            "**bold** *italic* ~~strike~~ `code` [link](https://example.com)",
        );
        let tree = UiTree::layout(
            markdown_view(&document, MarkdownPalette::default(), |destination| {
                Message::Link(destination.to_owned())
            }),
            Rect::new(0.0, 0.0, 640.0, 200.0),
        );
        let (text, spans) = tree
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCommand::StyledText { text, spans, .. } => Some((text, spans)),
                _ => None,
            })
            .expect("styled prose");
        assert_eq!(text, "bold italic strike code link");
        assert!(spans.iter().any(|span| span.bold));
        assert!(spans.iter().any(|span| span.italic));
        assert!(spans.iter().any(|span| span.strikethrough));
        assert!(spans.iter().any(|span| span.monospace));
        assert!(spans.iter().any(|span| span.color.is_some()));
    }

    #[test]
    fn link_activation_emits_one_unmodified_typed_destination() {
        let document = MarkdownDocument::parse("Read [the guide](../guide.md#start).");
        let tree = UiTree::layout(
            markdown_view(&document, MarkdownPalette::default(), |destination| {
                Message::Link(destination.to_owned())
            }),
            Rect::new(0.0, 0.0, 640.0, 480.0),
        );
        let message = Message::Link("../guide.md#start".into());
        let rect = tree.message_rect(&message).expect("link control");
        let point = Point {
            x: rect.origin.x + 2.0,
            y: rect.origin.y + 2.0,
        };
        let mut state = UiStateStore::default();
        tree.handle_event(&mut state, UiEvent::PointerPressed(point));
        assert_eq!(
            tree.handle_event(&mut state, UiEvent::PointerReleased(point))
                .messages,
            vec![message]
        );
    }

    #[test]
    fn document_is_selectable_in_logical_block_order() {
        let document = MarkdownDocument::parse("# Heading\n\nFirst *paragraph*.\n\n- One\n- Two");
        let mut state = UiStateStore::default();
        let tree = UiTree::layout_with_state(
            markdown_view(&document, MarkdownPalette::default(), |destination| {
                Message::Link(destination.to_owned())
            }),
            Rect::new(0.0, 0.0, 640.0, 480.0),
            &mut state,
        );
        let first_text = tree
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCommand::StyledText { bounds, text, .. } if text == "Heading" => Some(*bounds),
                _ => None,
            })
            .expect("heading text");
        tree.handle_event(
            &mut state,
            UiEvent::PointerPressed(Point {
                x: first_text.origin.x + 1.0,
                y: first_text.origin.y + first_text.size.height * 0.5,
            }),
        );
        tree.handle_event(&mut state, UiEvent::TextSelectAll);
        assert_eq!(
            tree.handle_event(&mut state, UiEvent::TextCopy)
                .clipboard_text
                .as_deref(),
            Some("Heading\nFirst paragraph.\nOne\nTwo")
        );
    }

    #[test]
    fn representative_layouts_have_finite_nonnegative_geometry() {
        let document = MarkdownDocument::parse(
            "# Long heading 世界\n\nA long paragraph with **strong**, *emphasized*, and `inline` content that wraps on a narrow window.\n\n```rust\nfn main() {}\n```",
        );
        for bounds in [
            Rect::new(0.0, 0.0, 640.0, 480.0),
            Rect::new(0.0, 0.0, 1024.0, 768.0),
            Rect::new(0.0, 0.0, 2048.0, 1536.0),
        ] {
            let tree = UiTree::layout(
                markdown_view(&document, MarkdownPalette::default(), |destination| {
                    Message::Link(destination.to_owned())
                }),
                bounds,
            );
            assert!(tree.resolved_layout().nodes().iter().all(|node| {
                let rect = node.allocated;
                rect.origin.x.is_finite()
                    && rect.origin.y.is_finite()
                    && rect.size.width.is_finite()
                    && rect.size.height.is_finite()
                    && rect.size.width >= 0.0
                    && rect.size.height >= 0.0
            }));
        }
    }

    #[test]
    fn representative_rasters_have_visible_hierarchy_at_three_sizes() {
        let document = MarkdownDocument::parse(
            "# Raster hierarchy\n\n**Bold**, *italic*, ~~strike~~, `code`, and [link](guide.md).\n\n> Quote\n\n```rust\nfn main() {}\n```",
        );
        for (width, height, scale) in [(640, 480, 1.0), (1024, 768, 1.0), (2048, 1536, 2.0)] {
            let tree = UiTree::layout(
                markdown_view(&document, MarkdownPalette::default(), |destination| {
                    Message::Link(destination.to_owned())
                }),
                Rect::new(0.0, 0.0, width as f32 / scale, height as f32 / scale),
            );
            let mut renderer = SdlComponentRenderer::new(width, height, scale);
            renderer.render(tree.commands());
            let visible = renderer.pixels().iter().filter(|pixel| pixel.a > 0).count();
            assert!(visible > 1_000, "raster was unexpectedly empty");
        }
    }

    #[test]
    fn narrow_prose_wraps_and_long_code_owns_horizontal_overflow() {
        let document = MarkdownDocument::parse(format!(
            "{}\n\n```text\n{}\n```",
            "wrapping prose ".repeat(40),
            "code_without_breaks_".repeat(80)
        ));
        let tree = UiTree::layout(
            markdown_view(&document, MarkdownPalette::default(), |destination| {
                Message::Link(destination.to_owned())
            }),
            Rect::new(0.0, 0.0, 320.0, 600.0),
        );
        let prose = tree
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCommand::StyledText { bounds, text, .. }
                    if text.starts_with("wrapping prose") =>
                {
                    Some(*bounds)
                }
                _ => None,
            })
            .expect("prose bounds");
        assert!(prose.size.height > 40.0, "long prose did not wrap");
        let code = tree
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("markdown-code-1"))
            .expect("code container");
        let scroll = code
            .scroll
            .unwrap_or_else(|| panic!("horizontal code overflow: {code:?}"));
        assert!(scroll.content.width > scroll.viewport.width);
    }
}
