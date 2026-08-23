use std::{cmp::Ordering, collections::HashMap, sync::Arc};

use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextBoundary {
    #[default]
    Inline,
    Block,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SelectionAffinity {
    #[default]
    Before,
    After,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionRun {
    pub id: String,
    pub text: Arc<str>,
    pub boundary_before: TextBoundary,
}

impl SelectionRun {
    pub fn inline(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: Arc::from(text.into()),
            boundary_before: TextBoundary::Inline,
        }
    }

    pub fn block(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: Arc::from(text.into()),
            boundary_before: TextBoundary::Block,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionEndpoint {
    pub run_id: String,
    pub offset: usize,
    pub affinity: SelectionAffinity,
}

impl SelectionEndpoint {
    pub fn new(run_id: impl Into<String>, offset: usize) -> Self {
        Self {
            run_id: run_id.into(),
            offset,
            affinity: SelectionAffinity::Before,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DocumentSelection {
    pub anchor: Option<SelectionEndpoint>,
    pub focus: Option<SelectionEndpoint>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SelectionDocument {
    runs: Vec<SelectionRun>,
    indexes: HashMap<String, usize>,
}

impl SelectionDocument {
    pub fn new(runs: impl IntoIterator<Item = SelectionRun>) -> Self {
        let runs = runs.into_iter().collect::<Vec<_>>();
        let indexes = runs
            .iter()
            .enumerate()
            .map(|(index, run)| (run.id.clone(), index))
            .collect();
        Self { runs, indexes }
    }

    pub fn runs(&self) -> &[SelectionRun] {
        &self.runs
    }

    pub fn endpoint(&self, run_id: impl Into<String>, offset: usize) -> Option<SelectionEndpoint> {
        let run_id = run_id.into();
        let run = self.run(&run_id)?;
        Some(SelectionEndpoint::new(
            run_id,
            clamp_grapheme_boundary(&run.text, offset),
        ))
    }

    pub fn select_all(&self) -> DocumentSelection {
        let Some(first) = self.runs.first() else {
            return DocumentSelection::default();
        };
        let last = self.runs.last().expect("first run exists");
        DocumentSelection {
            anchor: Some(SelectionEndpoint::new(first.id.clone(), 0)),
            focus: Some(SelectionEndpoint {
                run_id: last.id.clone(),
                offset: last.text.len(),
                affinity: SelectionAffinity::After,
            }),
        }
    }

    pub fn normalized(
        &self,
        selection: &DocumentSelection,
    ) -> Option<(SelectionEndpoint, SelectionEndpoint)> {
        let anchor = self.clamp_endpoint(selection.anchor.as_ref()?)?;
        let focus = self.clamp_endpoint(selection.focus.as_ref()?)?;
        Some(if self.compare(&anchor, &focus)? == Ordering::Greater {
            (focus, anchor)
        } else {
            (anchor, focus)
        })
    }

    pub fn selected_text(&self, selection: &DocumentSelection) -> Option<String> {
        let (start, end) = self.normalized(selection)?;
        if start == end {
            return None;
        }
        let start_index = *self.indexes.get(&start.run_id)?;
        let end_index = *self.indexes.get(&end.run_id)?;
        let mut output = String::new();
        for index in start_index..=end_index {
            let run = &self.runs[index];
            let from = if index == start_index {
                start.offset
            } else {
                0
            };
            let to = if index == end_index {
                end.offset
            } else {
                run.text.len()
            };
            if index > start_index && run.boundary_before == TextBoundary::Block {
                push_single_newline(&mut output, &run.text[from..to]);
            }
            output.push_str(&run.text[from..to]);
        }
        (!output.is_empty()).then_some(output)
    }

    pub fn selected_range_in(
        &self,
        selection: &DocumentSelection,
        run_id: &str,
    ) -> Option<std::ops::Range<usize>> {
        let (start, end) = self.normalized(selection)?;
        let index = *self.indexes.get(run_id)?;
        let start_index = *self.indexes.get(&start.run_id)?;
        let end_index = *self.indexes.get(&end.run_id)?;
        if index < start_index || index > end_index {
            return None;
        }
        let run = &self.runs[index];
        let from = if index == start_index {
            start.offset
        } else {
            0
        };
        let to = if index == end_index {
            end.offset
        } else {
            run.text.len()
        };
        (from != to).then_some(from..to)
    }

    pub fn reconcile(&self, selection: &mut DocumentSelection) {
        self.reconcile_from(self, selection);
    }

    pub fn reconcile_from(&self, previous: &SelectionDocument, selection: &mut DocumentSelection) {
        let anchor = selection
            .anchor
            .as_ref()
            .and_then(|endpoint| self.reconcile_endpoint(previous, endpoint));
        let focus = selection
            .focus
            .as_ref()
            .and_then(|endpoint| self.reconcile_endpoint(previous, endpoint));
        selection.anchor = anchor;
        selection.focus = focus;
        if selection.anchor.is_none() || selection.focus.is_none() {
            selection.anchor = None;
            selection.focus = None;
        }
    }

    pub fn move_grapheme(
        &self,
        endpoint: &SelectionEndpoint,
        direction: isize,
    ) -> Option<SelectionEndpoint> {
        let endpoint = self.clamp_endpoint(endpoint)?;
        let index = *self.indexes.get(&endpoint.run_id)?;
        let run = &self.runs[index];
        if direction.is_negative() {
            if endpoint.offset > 0 {
                return Some(SelectionEndpoint::new(
                    endpoint.run_id,
                    previous_grapheme_boundary(&run.text, endpoint.offset),
                ));
            }
            let previous = index
                .checked_sub(1)
                .and_then(|index| self.runs.get(index))?;
            Some(SelectionEndpoint::new(
                previous.id.clone(),
                previous.text.len(),
            ))
        } else {
            if endpoint.offset < run.text.len() {
                return Some(SelectionEndpoint::new(
                    endpoint.run_id,
                    next_grapheme_boundary(&run.text, endpoint.offset),
                ));
            }
            let next = self.runs.get(index + 1)?;
            Some(SelectionEndpoint::new(next.id.clone(), 0))
        }
    }

    pub fn move_word(
        &self,
        endpoint: &SelectionEndpoint,
        direction: isize,
    ) -> Option<SelectionEndpoint> {
        let mut current = self.clamp_endpoint(endpoint)?;
        let mut saw_word = false;
        loop {
            let next = self.move_grapheme(&current, direction)?;
            let grapheme = self.grapheme_between(&current, &next).unwrap_or("\n");
            let whitespace = grapheme.chars().all(char::is_whitespace);
            if saw_word && whitespace {
                return Some(current);
            }
            saw_word |= !whitespace;
            current = next;
        }
    }

    pub fn block_boundary(
        &self,
        endpoint: &SelectionEndpoint,
        end: bool,
    ) -> Option<SelectionEndpoint> {
        let endpoint = self.clamp_endpoint(endpoint)?;
        let run = self.run(&endpoint.run_id)?;
        Some(SelectionEndpoint::new(
            endpoint.run_id,
            if end { run.text.len() } else { 0 },
        ))
    }

    pub fn document_boundary(&self, end: bool) -> Option<SelectionEndpoint> {
        let run = if end {
            self.runs.last()?
        } else {
            self.runs.first()?
        };
        Some(SelectionEndpoint::new(
            run.id.clone(),
            if end { run.text.len() } else { 0 },
        ))
    }

    fn run(&self, id: &str) -> Option<&SelectionRun> {
        self.indexes.get(id).and_then(|index| self.runs.get(*index))
    }

    fn clamp_endpoint(&self, endpoint: &SelectionEndpoint) -> Option<SelectionEndpoint> {
        let run = self.run(&endpoint.run_id)?;
        Some(SelectionEndpoint {
            run_id: endpoint.run_id.clone(),
            offset: clamp_grapheme_boundary(&run.text, endpoint.offset),
            affinity: endpoint.affinity,
        })
    }

    fn reconcile_endpoint(
        &self,
        previous: &SelectionDocument,
        endpoint: &SelectionEndpoint,
    ) -> Option<SelectionEndpoint> {
        if let Some(mut reconciled) = self.clamp_endpoint(endpoint) {
            if endpoint.affinity == SelectionAffinity::After
                && previous
                    .run(&endpoint.run_id)
                    .is_some_and(|run| endpoint.offset == run.text.len())
                && let Some(run) = self.run(&endpoint.run_id)
            {
                reconciled.offset = run.text.len();
            }
            return Some(reconciled);
        }
        let old_index = previous.indexes.get(&endpoint.run_id).copied().unwrap_or(0);
        let fallback = self
            .runs
            .get(old_index.min(self.runs.len().saturating_sub(1)))?;
        let offset = match endpoint.affinity {
            SelectionAffinity::Before => 0,
            SelectionAffinity::After => fallback.text.len(),
        };
        Some(SelectionEndpoint {
            run_id: fallback.id.clone(),
            offset,
            affinity: endpoint.affinity,
        })
    }

    fn compare(&self, left: &SelectionEndpoint, right: &SelectionEndpoint) -> Option<Ordering> {
        let left_index = self.indexes.get(&left.run_id)?;
        let right_index = self.indexes.get(&right.run_id)?;
        Some(
            left_index
                .cmp(right_index)
                .then_with(|| left.offset.cmp(&right.offset)),
        )
    }

    fn grapheme_between<'a>(
        &'a self,
        left: &SelectionEndpoint,
        right: &SelectionEndpoint,
    ) -> Option<&'a str> {
        if left.run_id == right.run_id {
            let run = self.run(&left.run_id)?;
            let start = left.offset.min(right.offset);
            let end = left.offset.max(right.offset);
            return Some(&run.text[start..end]);
        }
        None
    }
}

fn push_single_newline(output: &mut String, following: &str) {
    if !output.ends_with('\n') && !following.starts_with('\n') {
        output.push('\n');
    }
}

fn clamp_grapheme_boundary(text: &str, offset: usize) -> usize {
    if offset >= text.len() {
        return text.len();
    }
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .take_while(|index| *index <= offset)
        .last()
        .unwrap_or(0)
}

fn previous_grapheme_boundary(text: &str, offset: usize) -> usize {
    text[..offset]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_grapheme_boundary(text: &str, offset: usize) -> usize {
    text[offset..]
        .grapheme_indices(true)
        .nth(1)
        .map_or(text.len(), |(index, _)| offset + index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> SelectionDocument {
        SelectionDocument::new([
            SelectionRun::block("speaker", "Codex"),
            SelectionRun::block("first", "Hello 🦀"),
            SelectionRun::inline("inline", " and 世界"),
            SelectionRun::block("last", "Done"),
        ])
    }

    #[test]
    fn serializes_forward_and_backward_multirun_selection() {
        let document = document();
        let forward = DocumentSelection {
            anchor: document.endpoint("first", 6),
            focus: document.endpoint("last", 4),
        };
        assert_eq!(
            document.selected_text(&forward).as_deref(),
            Some("🦀 and 世界\nDone")
        );
        let backward = DocumentSelection {
            anchor: forward.focus.clone(),
            focus: forward.anchor.clone(),
        };
        assert_eq!(
            document.selected_text(&backward),
            document.selected_text(&forward)
        );
    }

    #[test]
    fn select_all_preserves_inline_and_block_boundaries() {
        let document = document();
        assert_eq!(
            document.selected_text(&document.select_all()).as_deref(),
            Some("Codex\nHello 🦀 and 世界\nDone")
        );
    }

    #[test]
    fn movement_never_splits_graphemes_and_crosses_runs() {
        let document = document();
        let crab_end = document.endpoint("first", "Hello 🦀".len()).unwrap();
        let crab_start = document.move_grapheme(&crab_end, -1).unwrap();
        assert_eq!(
            &document.run("first").unwrap().text[crab_start.offset..],
            "🦀"
        );
        let next = document.move_grapheme(&crab_end, 1).unwrap();
        assert_eq!((next.run_id.as_str(), next.offset), ("inline", 0));
    }

    #[test]
    fn removed_endpoints_reconcile_by_affinity() {
        let document = SelectionDocument::new([SelectionRun::block("remaining", "text")]);
        let mut selection = DocumentSelection {
            anchor: Some(SelectionEndpoint::new("removed", 0)),
            focus: Some(SelectionEndpoint {
                run_id: "removed-too".into(),
                offset: 99,
                affinity: SelectionAffinity::After,
            }),
        };
        document.reconcile(&mut selection);
        assert_eq!(selection.anchor.unwrap().offset, 0);
        assert_eq!(selection.focus.unwrap().offset, 4);
    }

    #[test]
    fn mutation_reconciliation_uses_nearest_order_and_explicit_end_affinity() {
        let previous = SelectionDocument::new([
            SelectionRun::block("a", "A"),
            SelectionRun::block("removed", "old"),
            SelectionRun::block("c", "C"),
        ]);
        let mut removed = DocumentSelection {
            anchor: previous.endpoint("a", 0),
            focus: previous.endpoint("removed", 2),
        };
        let current =
            SelectionDocument::new([SelectionRun::block("a", "A"), SelectionRun::block("c", "C")]);
        current.reconcile_from(&previous, &mut removed);
        assert_eq!(removed.focus.unwrap().run_id, "c");

        let previous = SelectionDocument::new([SelectionRun::block("stream", "old")]);
        let mut fixed = DocumentSelection {
            anchor: previous.endpoint("stream", 0),
            focus: previous.endpoint("stream", 3),
        };
        let mut following_end = previous.select_all();
        let current = SelectionDocument::new([SelectionRun::block("stream", "old appended")]);
        current.reconcile_from(&previous, &mut fixed);
        current.reconcile_from(&previous, &mut following_end);
        assert_eq!(fixed.focus.unwrap().offset, 3);
        assert_eq!(following_end.focus.unwrap().offset, "old appended".len());
    }

    #[test]
    fn unicode_and_newline_serialization_never_splits_graphemes() {
        let document = SelectionDocument::new([
            SelectionRun::block("combining", "e\u{301}"),
            SelectionRun::inline("emoji", "👩‍💻"),
            SelectionRun::block("bidi", "שלום\nالعربية"),
        ]);
        let selection = document.select_all();
        assert_eq!(
            document.selected_text(&selection).as_deref(),
            Some("e\u{301}👩‍💻\nשלום\nالعربية")
        );
        let emoji_end = document.endpoint("emoji", "👩‍💻".len()).unwrap();
        assert_eq!(document.move_grapheme(&emoji_end, -1).unwrap().offset, 0);
    }

    #[test]
    fn reports_only_the_selected_slice_for_each_visible_run() {
        let document = document();
        let selection = DocumentSelection {
            anchor: document.endpoint("first", 6),
            focus: document.endpoint("last", 2),
        };
        assert_eq!(
            document.selected_range_in(&selection, "first"),
            Some(6.."Hello 🦀".len())
        );
        assert_eq!(
            document.selected_range_in(&selection, "inline"),
            Some(0.." and 世界".len())
        );
        assert_eq!(document.selected_range_in(&selection, "last"), Some(0..2));
        assert_eq!(document.selected_range_in(&selection, "speaker"), None);
    }
}
