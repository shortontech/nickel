use std::{cmp::Ordering, collections::HashMap};

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
    pub text: String,
    pub boundary_before: TextBoundary,
}

impl SelectionRun {
    pub fn inline(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            boundary_before: TextBoundary::Inline,
        }
    }

    pub fn block(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
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
        let anchor = selection
            .anchor
            .as_ref()
            .and_then(|endpoint| self.reconcile_endpoint(endpoint));
        let focus = selection
            .focus
            .as_ref()
            .and_then(|endpoint| self.reconcile_endpoint(endpoint));
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

    fn reconcile_endpoint(&self, endpoint: &SelectionEndpoint) -> Option<SelectionEndpoint> {
        if let Some(endpoint) = self.clamp_endpoint(endpoint) {
            return Some(endpoint);
        }
        let fallback = match endpoint.affinity {
            SelectionAffinity::Before => self.runs.first().map(|run| (run, 0)),
            SelectionAffinity::After => self.runs.last().map(|run| (run, run.text.len())),
        }?;
        Some(SelectionEndpoint {
            run_id: fallback.0.id.clone(),
            offset: fallback.1,
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
