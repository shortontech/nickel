//! Capacity accounting for the owned Markdown tree; shared selection text is counted once here.
use nickel_markdown::{Block, Inline, MarkdownDocument};
use nickel_ui::SelectionRun;

fn inline_bytes(inlines: &Vec<Inline>) -> usize {
    inlines.capacity() * size_of::<Inline>()
        + inlines
            .iter()
            .map(|inline| match inline {
                Inline::Text { text } | Inline::Code { text } | Inline::Unsupported { text } => {
                    text.capacity()
                }
                Inline::Emphasis { children }
                | Inline::Strong { children }
                | Inline::Strikethrough { children } => inline_bytes(children),
                Inline::Link {
                    destination,
                    title,
                    children,
                } => destination.capacity() + title.capacity() + inline_bytes(children),
                Inline::Break { .. } => 0,
            })
            .sum::<usize>()
}

fn blocks_bytes(blocks: &Vec<Block>) -> usize {
    blocks.capacity() * size_of::<Block>()
        + blocks
            .iter()
            .map(|block| match block {
                Block::Paragraph { inlines } => inline_bytes(inlines),
                Block::Heading {
                    anchor, inlines, ..
                } => anchor.capacity() + inline_bytes(inlines),
                Block::Code { language, text } => {
                    language.as_ref().map_or(0, String::capacity) + text.capacity()
                }
                Block::Quote { blocks } => blocks_bytes(blocks),
                Block::List { items, .. } => {
                    items.capacity() * size_of::<Vec<Block>>()
                        + items.iter().map(blocks_bytes).sum::<usize>()
                }
                Block::Table {
                    alignments,
                    header,
                    rows,
                } => {
                    alignments.capacity() * size_of::<nickel_markdown::TableAlignment>()
                        + header.capacity() * size_of::<Vec<Inline>>()
                        + header.iter().map(inline_bytes).sum::<usize>()
                        + rows.capacity() * size_of::<Vec<Vec<Inline>>>()
                        + rows
                            .iter()
                            .map(|row| {
                                row.capacity() * size_of::<Vec<Inline>>()
                                    + row.iter().map(inline_bytes).sum::<usize>()
                            })
                            .sum::<usize>()
                }
                Block::Unsupported { text } => text.capacity(),
                Block::ThematicBreak => 0,
            })
            .sum::<usize>()
}

pub(crate) fn derived_capacity(document: &MarkdownDocument, runs: &Vec<SelectionRun>) -> usize {
    size_of::<MarkdownDocument>()
        + 2 * size_of::<usize>()
        + document.source.capacity()
        + blocks_bytes(&document.blocks)
        + document.diagnostics.capacity() * size_of::<nickel_markdown::MarkdownDiagnostic>()
        + document
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.capacity())
            .sum::<usize>()
        + runs.capacity() * size_of::<SelectionRun>()
        + runs
            .iter()
            .map(|run| run.id.capacity() + run.text.len() + 2 * size_of::<usize>())
            .sum::<usize>()
}
