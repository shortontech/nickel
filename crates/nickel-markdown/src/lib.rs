//! Safe typed Markdown documents and declarative Nickel UI presentation.

use std::iter::Peekable;

#[cfg(feature = "view")]
use nickel_ui::{
    Align, AnyView, Button, Color, Column, ComponentBuilderExt, Container, Grid, Insets, Length,
    Overflow, Row, SelectionRegion, SelectionRun, StyledText, StyledTextSpan, Text, TextBoundary,
    Track, UiId, ui,
};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
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
        let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
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
    Table {
        alignments: Vec<TableAlignment>,
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    ThematicBreak,
    Unsupported {
        text: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableAlignment {
    None,
    Left,
    Center,
    Right,
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
            Event::Start(Tag::Table(alignments)) => {
                blocks.push(parse_table(events, alignments, diagnostics));
            }
            event if is_inline_event(&event) => blocks.push(Block::Paragraph {
                inlines: parse_unwrapped_inlines(event, events, until.as_ref(), diagnostics),
            }),
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

fn is_inline_event(event: &Event<'_>) -> bool {
    matches!(
        event,
        Event::Text(_)
            | Event::Code(_)
            | Event::SoftBreak
            | Event::HardBreak
            | Event::Html(_)
            | Event::InlineHtml(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::FootnoteReference(_)
            | Event::TaskListMarker(_)
            | Event::Start(
                Tag::Emphasis
                    | Tag::Strong
                    | Tag::Strikethrough
                    | Tag::Link { .. }
                    | Tag::Image { .. }
            )
    )
}

fn parse_unwrapped_inlines<'a, I>(
    first: Event<'a>,
    events: &mut Peekable<I>,
    boundary: Option<&TagEnd>,
    diagnostics: &mut Vec<MarkdownDiagnostic>,
) -> Vec<Inline>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut depth = usize::from(matches!(&first, Event::Start(_)));
    let mut buffered = vec![first];
    while let Some(event) = events.peek() {
        if depth == 0 && matches!(event, Event::End(end) if boundary == Some(end)) {
            break;
        }
        match event {
            Event::Start(_) if is_inline_event(event) => depth += 1,
            Event::End(_) if depth > 0 => depth -= 1,
            _ if depth == 0 && !is_inline_event(event) => break,
            _ => {}
        }
        buffered.push(events.next().expect("peeked inline event"));
    }
    let mut buffered = buffered.into_iter().peekable();
    parse_inlines(&mut buffered, TagEnd::Paragraph, diagnostics)
}

fn parse_table<'a, I>(
    events: &mut Peekable<I>,
    alignments: Vec<Alignment>,
    diagnostics: &mut Vec<MarkdownDiagnostic>,
) -> Block
where
    I: Iterator<Item = Event<'a>>,
{
    let mut header = Vec::new();
    let mut rows = Vec::new();
    let mut in_header = false;
    let mut current_row = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::TableHead) => in_header = true,
            Event::End(TagEnd::TableHead) => {
                header = std::mem::take(&mut current_row);
                in_header = false;
            }
            Event::Start(Tag::TableRow) => current_row.clear(),
            Event::End(TagEnd::TableRow) => rows.push(std::mem::take(&mut current_row)),
            Event::Start(Tag::TableCell) => {
                current_row.push(parse_inlines(events, TagEnd::TableCell, diagnostics));
            }
            Event::End(TagEnd::Table) => break,
            _ => {}
        }
    }
    if in_header && header.is_empty() {
        header = current_row;
    }
    Block::Table {
        alignments: alignments
            .into_iter()
            .map(|alignment| match alignment {
                Alignment::None => TableAlignment::None,
                Alignment::Left => TableAlignment::Left,
                Alignment::Center => TableAlignment::Center,
                Alignment::Right => TableAlignment::Right,
            })
            .collect(),
        header,
        rows,
    }
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
            Block::Table { header, rows, .. } => {
                for (row_index, row) in std::iter::once(header).chain(rows.iter()).enumerate() {
                    if row_index > 0 {
                        output.push('\n');
                    }
                    for (cell_index, cell) in row.iter().enumerate() {
                        if cell_index > 0 {
                            output.push('\t');
                        }
                        output.push_str(&inline_text(cell));
                    }
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

#[cfg(feature = "view")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MarkdownPalette {
    pub foreground: Color,
    pub muted: Color,
    pub accent: Color,
    pub surface: Color,
    pub border: Color,
    pub code: Color,
}

#[cfg(feature = "view")]
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
#[cfg(feature = "view")]
pub fn markdown_view<Message, Map>(
    document: &MarkdownDocument,
    palette: MarkdownPalette,
    map_link: Map,
) -> AnyView<Message>
where
    Message: Clone + 'static,
    Map: Fn(&str) -> Message + Copy,
{
    let document = SelectionRegion::automatic()
        .id(UiId::new("markdown-document"))
        .child(markdown_content_view(document, palette, "", map_link));
    AnyView::new(ui! { {document} })
}

/// Render Markdown content inside a selection region owned by the caller.
///
/// `scope` makes every rendered text and control identity unique when several
/// documents share one UI tree, such as virtualized chat messages.
#[cfg(feature = "view")]
pub fn markdown_content_view<Message, Map>(
    document: &MarkdownDocument,
    palette: MarkdownPalette,
    scope: &str,
    map_link: Map,
) -> AnyView<Message>
where
    Message: Clone + 'static,
    Map: Fn(&str) -> Message + Copy,
{
    let blocks = document.blocks.iter().enumerate().map(|(index, block)| {
        let id = if scope.is_empty() {
            index.to_string()
        } else {
            format!("{scope}/{index}")
        };
        render_block(block, id, palette, map_link)
    });
    AnyView::new(Column::new().fill_width().gap(12.0).children(blocks))
}

/// Build the logical text runs used by an outer, potentially virtualized,
/// selection document. The identifiers exactly match `markdown_content_view`.
#[cfg(feature = "view")]
pub fn markdown_selection_runs(document: &MarkdownDocument, scope: &str) -> Vec<SelectionRun> {
    let mut runs = Vec::new();
    for (index, block) in document.blocks.iter().enumerate() {
        let id = if scope.is_empty() {
            index.to_string()
        } else {
            format!("{scope}/{index}")
        };
        append_selection_runs(block, id, &mut runs);
    }
    runs
}

#[cfg(feature = "view")]
fn append_selection_runs(block: &Block, id: String, runs: &mut Vec<SelectionRun>) {
    match block {
        Block::Paragraph { inlines } | Block::Heading { inlines, .. } => runs.push(
            SelectionRun::block(format!("markdown-{id}"), inline_text(inlines)),
        ),
        Block::Code { text, .. } | Block::Unsupported { text } => {
            runs.push(SelectionRun::block(format!("markdown-{id}"), text))
        }
        Block::Quote { blocks } => {
            for (index, block) in blocks.iter().enumerate() {
                append_selection_runs(block, format!("{id}-{index}"), runs);
            }
        }
        Block::List { items, .. } => {
            for (item_index, blocks) in items.iter().enumerate() {
                for (block_index, block) in blocks.iter().enumerate() {
                    append_selection_runs(block, format!("{id}-{item_index}-{block_index}"), runs);
                }
            }
        }
        Block::Table { header, rows, .. } => {
            for (row_index, row) in std::iter::once(header).chain(rows.iter()).enumerate() {
                for (column, inlines) in row.iter().enumerate() {
                    runs.push(SelectionRun::block(
                        format!("markdown-{id}-table-{row_index}-{column}"),
                        inline_text(inlines),
                    ));
                }
            }
        }
        Block::ThematicBreak => {}
    }
}

#[cfg(feature = "view")]
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
        Block::Paragraph { inlines } => render_prose(inlines, id, 2.0, false, palette, map_link),
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
                            .scale(1.0)
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
                    Grid::tracks([Track::Auto, Track::Fraction(1.0)])
                        .id(UiId::new(format!("markdown-list-row-{id}-{index}")))
                        .fill_width()
                        .gap(9.0)
                        .children([
                            AnyView::new(Text::new(marker).color(palette.muted).selectable(false)),
                            AnyView::new(Column::new().fill_width().gap(6.0).children(children)),
                        ]),
                )
            });
            AnyView::new(Column::new().fill_width().gap(7.0).children(rows))
        }
        Block::Table {
            alignments,
            header,
            rows,
        } => {
            let columns = alignments
                .len()
                .max(header.len())
                .max(rows.iter().map(Vec::len).max().unwrap_or(0))
                .max(1);
            let cells = std::iter::once((true, header))
                .chain(rows.iter().map(|row| (false, row)))
                .enumerate()
                .flat_map(|(row_index, (heading, row))| {
                    let row_id = id.clone();
                    (0..columns).map(move |column| {
                        let inlines = row.get(column).map_or(&[][..], Vec::as_slice);
                        AnyView::new(
                            Container::new()
                                .padding(Insets::all(9.0))
                                .background(palette.surface)
                                .border(palette.border, 1.0)
                                .child(render_prose(
                                    inlines,
                                    format!("{row_id}-table-{row_index}-{column}"),
                                    2.0,
                                    heading,
                                    palette,
                                    map_link,
                                )),
                        )
                    })
                });
            AnyView::new(
                Row::new()
                    .id(UiId::new(format!("markdown-table-{id}")))
                    .fill_width()
                    .overflow_x(Overflow::Auto)
                    .child(
                        Grid::fixed(columns)
                            .min_width(columns as f32 * 140.0)
                            .fill_width()
                            .children(cells),
                    ),
            )
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

#[cfg(feature = "view")]
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
    let inline_link_ranges = inline_link_ranges(inlines);
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
    let styled = inline_link_ranges.into_iter().fold(
        StyledText::new(text, spans)
            .color(palette.foreground)
            .scale(scale)
            .width_length(Length::Fill)
            .wrap(true)
            .selection_run_id(format!("markdown-{id}"))
            .selection_boundary(TextBoundary::Block),
        |styled, (range, destination)| styled.inline_message(range, map_link(&destination)),
    );
    AnyView::new(
        Column::new()
            .fill_width()
            .gap(6.0)
            .child(styled)
            .children(controls),
    )
}

#[cfg(feature = "view")]
fn inline_link_ranges(inlines: &[Inline]) -> Vec<(std::ops::Range<usize>, String)> {
    fn collect(
        inlines: &[Inline],
        cursor: &mut usize,
        links: &mut Vec<(std::ops::Range<usize>, String)>,
    ) {
        for inline in inlines {
            match inline {
                Inline::Text { text } | Inline::Code { text } | Inline::Unsupported { text } => {
                    *cursor += text.len();
                }
                Inline::Emphasis { children }
                | Inline::Strong { children }
                | Inline::Strikethrough { children } => collect(children, cursor, links),
                Inline::Link {
                    destination,
                    children,
                    ..
                } => {
                    let start = *cursor;
                    collect(children, cursor, links);
                    links.push((start..*cursor, destination.clone()));
                }
                Inline::Break { .. } => *cursor += 1,
            }
        }
    }

    let mut cursor = 0;
    let mut links = Vec::new();
    collect(inlines, &mut cursor, &mut links);
    links
}

#[cfg(feature = "view")]
#[derive(Clone, Copy, Default)]
struct InlineStyle {
    bold: bool,
    italic: bool,
    monospace: bool,
    strikethrough: bool,
    color: Option<Color>,
    background: Option<Color>,
}

#[cfg(feature = "view")]
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

#[cfg(feature = "view")]
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
                        background: Some(palette.surface),
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

#[cfg(feature = "view")]
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
            background: style.background,
        });
    }
}

#[cfg(feature = "view")]
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

#[cfg(feature = "view")]
fn inline_links(inlines: &[Inline]) -> Vec<(String, String)> {
    let mut links = Vec::new();
    collect_inline_links(inlines, &mut links);
    links
}

#[cfg(feature = "view")]
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

#[cfg(feature = "view")]
fn heading_scale(level: u8) -> f32 {
    match level {
        1 => 4.0,
        2 => 3.0,
        _ => 2.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "view")]
    use nickel_ui::{
        PaintCommand, Point, PointerIcon, Rect, SdlComponentRenderer, UiEvent, UiStateStore, UiTree,
    };

    #[cfg(feature = "view")]
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
    fn tables_preserve_alignment_inline_content_and_logical_copy_order() {
        let document = MarkdownDocument::parse(
            "| Name | Result |\n| :--- | ---: |\n| **Eileen** | [20/25](results.md) |\n",
        );

        assert!(document.diagnostics.is_empty());
        assert!(matches!(
            document.blocks.as_slice(),
            [Block::Table {
                alignments,
                header,
                rows,
            }] if alignments == &[TableAlignment::Left, TableAlignment::Right]
                && header.len() == 2
                && rows.len() == 1
        ));
        assert_eq!(document.logical_text(), "Name\tResult\nEileen\t20/25");
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
    #[cfg(feature = "view")]
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
    #[cfg(feature = "view")]
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
        assert!(
            spans
                .iter()
                .any(|span| span.monospace && span.background.is_some())
        );
        assert!(spans.iter().any(|span| span.color.is_some()));
    }

    #[test]
    #[cfg(feature = "view")]
    fn link_activation_emits_one_unmodified_typed_destination() {
        let document = MarkdownDocument::parse("Read [the guide](../guide.md#start).");
        let mut state = UiStateStore::default();
        let build = |state: &mut UiStateStore| {
            UiTree::layout_with_state(
                markdown_view(&document, MarkdownPalette::default(), |destination| {
                    Message::Link(destination.to_owned())
                }),
                Rect::new(0.0, 0.0, 640.0, 480.0),
                state,
            )
        };
        let tree = build(&mut state);
        let message = Message::Link("../guide.md#start".into());
        let rect = tree.message_rect(&message).expect("link control");
        let point = Point {
            x: rect.origin.x + 2.0,
            y: rect.origin.y + 2.0,
        };
        assert_eq!(tree.pointer_icon_at(point), PointerIcon::Hand);
        tree.handle_event(&mut state, UiEvent::PointerPressed(point));
        let rebuilt = build(&mut state);
        assert_eq!(
            rebuilt
                .handle_event(&mut state, UiEvent::PointerReleased(point))
                .messages,
            vec![message]
        );
    }

    #[test]
    #[cfg(feature = "view")]
    fn document_is_selectable_in_logical_block_order() {
        let document = MarkdownDocument::parse(
            "# Heading\n\nFirst *paragraph*.\n\n- One\n- Two\n\n> Quoted\n\n```text\ncode();\n```",
        );
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
            Some("Heading\nFirst paragraph.\nOne\nTwo\nQuoted\ncode();\n")
        );
    }

    #[test]
    #[cfg(feature = "view")]
    fn caller_owned_selection_runs_match_scoped_render_ids() {
        let document = MarkdownDocument::parse(
            "# Heading\n\nParagraph\n\n- First\n- Second\n\n| A | B |\n| - | - |\n| 1 | 2 |",
        );
        let scope = "message-7/body";
        let runs = markdown_selection_runs(&document, scope);
        assert!(
            runs.iter()
                .all(|run| run.id.starts_with("markdown-message-7/body/"))
        );
        let selection_document = std::sync::Arc::new(nickel_ui::SelectionDocument::new(runs));
        let mut state = UiStateStore::default();
        let build = |state: &mut UiStateStore| {
            UiTree::layout_with_state(
                SelectionRegion::new(selection_document.clone())
                    .id("test-markdown-region")
                    .child(markdown_content_view(
                        &document,
                        MarkdownPalette::default(),
                        scope,
                        |destination| Message::Link(destination.to_owned()),
                    )),
                Rect::new(0.0, 0.0, 640.0, 480.0),
                state,
            )
        };
        let initial = build(&mut state);
        let region_id = initial
            .selection_region_ids()
            .next()
            .expect("scoped selection region")
            .clone();
        state.set_selection_owner(Some(region_id.clone()));
        *state.document_selection_mut(region_id) = selection_document.select_all();
        let tree = build(&mut state);
        assert!(tree.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Fill {
                color: 0x315a8f,
                ..
            }
        )));
    }

    #[test]
    #[cfg(feature = "view")]
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
    #[cfg(feature = "view")]
    fn styled_list_prefix_stays_bold_and_inline_when_it_fits() {
        let document = MarkdownDocument::parse(
            "- **Nickel UI** — the desktop shell, taskbar, launcher, task switcher, and system controls",
        );
        let tree = UiTree::layout(
            markdown_view(&document, MarkdownPalette::default(), |destination| {
                Message::Link(destination.to_owned())
            }),
            Rect::new(0.0, 0.0, 900.0, 200.0),
        );
        let (bounds, text, spans) = tree
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCommand::StyledText {
                    bounds,
                    text,
                    spans,
                    ..
                } if text.starts_with("Nickel UI") => Some((bounds, text, spans)),
                _ => None,
            })
            .expect("styled list text");
        assert_eq!(
            text,
            "Nickel UI — the desktop shell, taskbar, launcher, task switcher, and system controls"
        );
        assert!(spans.iter().any(|span| span.range == (0..9) && span.bold));
        assert!(
            bounds.size.width > 800.0,
            "list body was narrow: {bounds:?}"
        );
        assert!(bounds.size.height < 30.0, "list body wrapped: {bounds:?}");
    }

    #[test]
    #[cfg(feature = "view")]
    fn wrapped_ordered_list_items_allocate_their_full_height() {
        let document = MarkdownDocument::parse(
            "1. Definitions. License shall mean the terms and conditions for use, reproduction, and distribution as defined by this document and all following sections.\n2. Grant. Each contributor hereby grants a perpetual worldwide license under these terms.\n",
        );
        let tree = UiTree::layout(
            markdown_view(&document, MarkdownPalette::default(), |destination| {
                Message::Link(destination.to_owned())
            }),
            Rect::new(0.0, 0.0, 420.0, 600.0),
        );
        let row = |suffix: &str| {
            tree.resolved_layout()
                .nodes()
                .iter()
                .find(|node| node.id.as_str().ends_with(suffix))
                .expect("ordered list row")
                .allocated
        };
        let first = row("markdown-list-row-0-0");
        let second = row("markdown-list-row-0-1");
        assert!(
            first.size.height > 40.0,
            "first item did not wrap: {first:?}"
        );
        assert!(
            second.origin.y >= first.origin.y + first.size.height,
            "ordered list rows overlap: {first:?}, {second:?}"
        );
    }

    #[test]
    #[cfg(feature = "view")]
    fn apache_license_continuations_stay_inside_their_list_rows() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../LICENSE-APACHE"),
        )
        .expect("Apache license fixture");
        let document = MarkdownDocument::parse(source);
        let tree = UiTree::layout(
            markdown_view(&document, MarkdownPalette::default(), |destination| {
                Message::Link(destination.to_owned())
            }),
            Rect::new(0.0, 0.0, 844.0, 20_000.0),
        );
        let nodes = tree.resolved_layout().nodes();
        let styled = tree
            .commands()
            .iter()
            .filter_map(|command| match command {
                PaintCommand::StyledText { bounds, text, .. } => Some((*bounds, text.as_str())),
                _ => None,
            })
            .collect::<Vec<_>>();
        for (index, (left, left_text)) in styled.iter().enumerate() {
            for (right, right_text) in &styled[index + 1..] {
                let overlaps = left.origin.x < right.origin.x + right.size.width
                    && right.origin.x < left.origin.x + left.size.width
                    && left.origin.y < right.origin.y + right.size.height
                    && right.origin.y < left.origin.y + left.size.height;
                assert!(
                    !overlaps,
                    "Apache text rectangles overlap: {left:?} {left_text:?}; {right:?} {right_text:?}"
                );
            }
        }
        for row in nodes
            .iter()
            .filter(|node| node.id.as_str().contains("markdown-list-row-"))
        {
            let prefix = format!("{}/", row.id.as_str());
            let descendant_bottom = nodes
                .iter()
                .filter(|node| node.id.as_str().starts_with(&prefix))
                .map(|node| node.allocated.origin.y + node.allocated.size.height)
                .fold(row.allocated.origin.y, f32::max);
            let row_bottom = row.allocated.origin.y + row.allocated.size.height;
            assert!(
                descendant_bottom <= row_bottom + 0.01,
                "Apache list content escaped its row: {:?}, bottom={descendant_bottom}",
                row.allocated
            );
            let body_prefix = format!("{}/#1/", row.id.as_str());
            let mut blocks = nodes
                .iter()
                .filter(|node| {
                    node.id
                        .as_str()
                        .strip_prefix(&body_prefix)
                        .is_some_and(|tail| !tail.contains('/'))
                })
                .map(|node| node.allocated)
                .collect::<Vec<_>>();
            blocks.sort_by(|left, right| left.origin.y.total_cmp(&right.origin.y));
            for pair in blocks.windows(2) {
                assert!(
                    pair[0].origin.y + pair[0].size.height <= pair[1].origin.y + 0.01,
                    "Apache continuation blocks overlap: {:?}, {:?}",
                    pair[0],
                    pair[1]
                );
            }
        }
    }

    #[test]
    #[cfg(feature = "view")]
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
    #[cfg(feature = "view")]
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
