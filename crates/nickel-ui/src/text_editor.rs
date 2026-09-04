use std::{fmt, ops::Range};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Default, Eq, PartialEq)]
pub struct TextEditor {
    text: String,
    cursor: usize,
    anchor: usize,
    preedit: String,
    preedit_cursor: Option<Range<usize>>,
    undo: Vec<EditorSnapshot>,
    redo: Vec<EditorSnapshot>,
}

// Editor state can contain passwords and clipboard-derived text. Keep its
// Debug representation useful for diagnostics without ever reproducing the
// document, preedit transaction, or history payloads.
impl fmt::Debug for TextEditor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextEditor")
            .field("text_bytes", &self.text.len())
            .field("cursor", &self.cursor)
            .field("anchor", &self.anchor)
            .field("preedit_bytes", &self.preedit.len())
            .field("undo_depth", &self.undo.len())
            .field("redo_depth", &self.redo.len())
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EditorSnapshot {
    text: String,
    cursor: usize,
    anchor: usize,
}

const HISTORY_LIMIT: usize = 100;

impl TextEditor {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self {
            text,
            cursor,
            anchor: cursor,
            preedit: String::new(),
            preedit_cursor: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Opaque generation used to reject commands from a stale context menu.
    /// It deliberately cannot reveal document content.
    pub fn document_generation(&self) -> u64 {
        fingerprint(self.text.as_bytes())
    }

    pub fn selection_generation(&self) -> u64 {
        fingerprint(
            &[
                self.cursor.to_le_bytes().as_slice(),
                self.anchor.to_le_bytes().as_slice(),
            ]
            .concat(),
        )
    }

    pub fn selection(&self) -> Option<Range<usize>> {
        (self.cursor != self.anchor)
            .then(|| self.cursor.min(self.anchor)..self.cursor.max(self.anchor))
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|range| &self.text[range])
    }

    pub fn cut_selection(&mut self) -> Option<String> {
        let selected = self.selected_text()?.to_owned();
        self.record_edit();
        self.delete_selection();
        Some(selected)
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
        self.anchor = self.cursor;
        self.cancel_preedit();
        self.undo.clear();
        self.redo.clear();
    }

    pub fn clear(&mut self) {
        if self.text.is_empty() {
            return;
        }
        self.record_edit();
        self.text.clear();
        self.cursor = 0;
        self.anchor = 0;
        self.cancel_preedit();
    }

    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    pub fn set_preedit(&mut self, text: impl Into<String>, cursor: Option<Range<usize>>) {
        self.preedit = text.into();
        self.preedit_cursor = cursor.map(|range| {
            clamp_boundary(&self.preedit, range.start)..clamp_boundary(&self.preedit, range.end)
        });
    }

    pub fn commit_preedit(&mut self, text: &str) {
        self.cancel_preedit();
        self.insert(text);
    }

    pub fn cancel_preedit(&mut self) {
        self.preedit.clear();
        self.preedit_cursor = None;
    }

    pub fn display_text_with_caret(&self, caret: &str) -> String {
        let replacement = if self.preedit.is_empty() {
            self.cursor..self.cursor
        } else {
            self.selection().unwrap_or(self.cursor..self.cursor)
        };
        let mut displayed =
            String::with_capacity(self.text.len() + self.preedit.len() + caret.len());
        displayed.push_str(&self.text[..replacement.start]);
        if self.preedit.is_empty() {
            displayed.push_str(caret);
        } else {
            let preedit_caret = self
                .preedit_cursor
                .as_ref()
                .map_or(self.preedit.len(), |cursor| cursor.end);
            displayed.push_str(&self.preedit[..preedit_caret]);
            displayed.push_str(caret);
            displayed.push_str(&self.preedit[preedit_caret..]);
        }
        displayed.push_str(&self.text[replacement.end..]);
        displayed
    }

    pub fn display_caret_prefix(&self) -> String {
        if self.preedit.is_empty() {
            return self.text[..self.cursor].to_owned();
        }
        let replacement = self.selection().unwrap_or(self.cursor..self.cursor);
        let preedit_caret = self
            .preedit_cursor
            .as_ref()
            .map_or(self.preedit.len(), |cursor| cursor.end);
        format!(
            "{}{}",
            &self.text[..replacement.start],
            &self.preedit[..preedit_caret]
        )
    }

    pub fn insert(&mut self, text: &str) {
        self.replace_selection(text);
    }

    pub fn replace_selection(&mut self, replacement: &str) {
        let range = self.selection().unwrap_or(self.cursor..self.cursor);
        if &self.text[range.clone()] == replacement {
            return;
        }
        self.record_edit();
        let start = range.start;
        self.text.replace_range(range, replacement);
        self.cursor = start + replacement.len();
        self.anchor = self.cursor;
    }

    pub fn backspace(&mut self) {
        if self.selection().is_some() {
            self.record_edit();
            self.delete_selection();
            return;
        }
        if self.cursor == 0 {
            return;
        }
        self.record_edit();
        let previous = previous_boundary(&self.text, self.cursor);
        self.text.replace_range(previous..self.cursor, "");
        self.cursor = previous;
        self.anchor = previous;
    }

    pub fn backspace_word(&mut self) {
        if self.selection().is_some() {
            self.record_edit();
            self.delete_selection();
            return;
        }
        if self.cursor == 0 {
            return;
        }
        self.record_edit();
        let previous = previous_word_boundary(&self.text, self.cursor);
        self.text.replace_range(previous..self.cursor, "");
        self.cursor = previous;
        self.anchor = previous;
    }

    pub fn delete(&mut self) {
        if self.selection().is_some() {
            self.record_edit();
            self.delete_selection();
            return;
        }
        if self.cursor == self.text.len() {
            return;
        }
        self.record_edit();
        let next = next_boundary(&self.text, self.cursor);
        self.text.replace_range(self.cursor..next, "");
        self.anchor = self.cursor;
    }

    pub fn move_left(&mut self, extend_selection: bool) {
        if !extend_selection && let Some(selection) = self.selection() {
            self.cursor = selection.start;
            self.anchor = self.cursor;
            return;
        }
        self.move_cursor(previous_boundary(&self.text, self.cursor), extend_selection);
    }

    pub fn move_right(&mut self, extend_selection: bool) {
        if !extend_selection && let Some(selection) = self.selection() {
            self.cursor = selection.end;
            self.anchor = self.cursor;
            return;
        }
        self.move_cursor(next_boundary(&self.text, self.cursor), extend_selection);
    }

    pub fn move_word_left(&mut self, extend_selection: bool) {
        if !extend_selection && let Some(selection) = self.selection() {
            self.move_cursor(selection.start, false);
            return;
        }
        self.move_cursor(
            previous_word_boundary(&self.text, self.cursor),
            extend_selection,
        );
    }

    pub fn move_word_right(&mut self, extend_selection: bool) {
        if !extend_selection && let Some(selection) = self.selection() {
            self.move_cursor(selection.end, false);
            return;
        }
        self.move_cursor(
            next_word_boundary(&self.text, self.cursor),
            extend_selection,
        );
    }

    pub fn move_home(&mut self, extend_selection: bool) {
        self.move_cursor(0, extend_selection);
    }

    pub fn move_end(&mut self, extend_selection: bool) {
        self.move_cursor(self.text.len(), extend_selection);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        let Some(snapshot) = self.undo.pop() else {
            return;
        };
        let current = self.snapshot();
        self.restore(snapshot);
        self.redo.push(current);
    }

    pub fn redo(&mut self) {
        let Some(snapshot) = self.redo.pop() else {
            return;
        };
        let current = self.snapshot();
        self.restore(snapshot);
        self.undo.push(current);
    }

    pub fn delete_selected(&mut self) {
        if self.selection().is_some() {
            self.record_edit();
            self.delete_selection();
        }
    }

    pub fn place_cursor(&mut self, cursor: usize) {
        self.cursor = clamp_grapheme_boundary(&self.text, cursor);
        self.anchor = self.cursor;
    }

    pub fn extend_selection_to(&mut self, cursor: usize) {
        self.cursor = clamp_grapheme_boundary(&self.text, cursor);
    }

    fn move_cursor(&mut self, cursor: usize, extend_selection: bool) {
        self.cursor = cursor;
        if !extend_selection {
            self.anchor = cursor;
        }
    }

    fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection() else {
            return false;
        };
        let start = range.start;
        self.text.replace_range(range, "");
        self.cursor = start;
        self.anchor = start;
        true
    }

    fn snapshot(&self) -> EditorSnapshot {
        EditorSnapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }

    fn record_edit(&mut self) {
        self.cancel_preedit();
        self.undo.push(self.snapshot());
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn restore(&mut self, snapshot: EditorSnapshot) {
        self.text = snapshot.text;
        self.cursor = snapshot.cursor;
        self.anchor = snapshot.anchor;
        self.cancel_preedit();
    }
}

fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn previous_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor]
        .grapheme_indices(true)
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn clamp_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn next_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .grapheme_indices(true)
        .nth(1)
        .map(|(index, _)| cursor + index)
        .unwrap_or(text.len())
}

fn clamp_grapheme_boundary(text: &str, cursor: usize) -> usize {
    if cursor >= text.len() {
        return text.len();
    }
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .take_while(|index| *index <= cursor.min(text.len()))
        .last()
        .unwrap_or(0)
}

fn previous_word_boundary(text: &str, cursor: usize) -> usize {
    let prefix = &text[..cursor];
    let trimmed = prefix.trim_end_matches(char::is_whitespace);
    trimmed
        .grapheme_indices(true)
        .rev()
        .take_while(|(_, grapheme)| !grapheme.chars().all(char::is_whitespace))
        .last()
        .map_or(trimmed.len(), |(index, _)| index)
}

fn next_word_boundary(text: &str, cursor: usize) -> usize {
    let suffix = &text[cursor..];
    let mut seen_word = false;
    for (index, grapheme) in suffix.grapheme_indices(true) {
        let whitespace = grapheme.chars().all(char::is_whitespace);
        if seen_word && whitespace {
            return cursor + index;
        }
        seen_word |= !whitespace;
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::TextEditor;

    #[test]
    fn inserts_at_cursor_and_replaces_selection() {
        let mut editor = TextEditor::new("wifi");
        editor.move_left(false);
        editor.move_left(true);
        editor.insert("F");
        assert_eq!(editor.text(), "wiFi");
        assert_eq!(editor.cursor(), 3);
        assert_eq!(editor.selection(), None);
    }

    #[test]
    fn cursor_moves_across_utf8_characters() {
        let mut editor = TextEditor::new("aé🦀");
        editor.move_left(false);
        assert_eq!(editor.cursor(), "aé".len());
        editor.backspace();
        assert_eq!(editor.text(), "a🦀");
        editor.delete();
        assert_eq!(editor.text(), "a");
    }

    #[test]
    fn selection_collapses_toward_motion() {
        let mut editor = TextEditor::new("nickel");
        editor.move_home(false);
        editor.move_right(true);
        editor.move_right(true);
        assert_eq!(editor.selected_text(), Some("ni"));
        editor.move_right(false);
        assert_eq!(editor.cursor(), 2);
        assert_eq!(editor.selection(), None);
    }

    #[test]
    fn select_all_can_be_replaced_by_paste() {
        let mut editor = TextEditor::new("old");
        editor.select_all();
        editor.replace_selection("ms-settings:");
        assert_eq!(editor.text(), "ms-settings:");
    }

    #[test]
    fn cursor_does_not_split_combining_marks_or_emoji_sequences() {
        let mut editor = TextEditor::new("e\u{301}👨‍👩‍👧");
        editor.move_left(false);
        editor.backspace();
        assert_eq!(editor.text(), "👨‍👩‍👧");
        editor.delete();
        assert_eq!(editor.text(), "");
    }

    #[test]
    fn preedit_is_presented_without_committing_and_then_commits_once() {
        let mut editor = TextEditor::new("run ");
        editor.set_preedit("にほ", Some(6..6));
        assert_eq!(editor.text(), "run ");
        assert_eq!(editor.display_text_with_caret("|"), "run にほ|");
        editor.commit_preedit("日本");
        assert_eq!(editor.text(), "run 日本");
        assert_eq!(editor.preedit(), "");
    }

    #[test]
    fn preedit_visually_replaces_selection() {
        let mut editor = TextEditor::new("old");
        editor.select_all();
        editor.set_preedit("新", None);
        assert_eq!(editor.display_text_with_caret("|"), "新|");
        editor.cancel_preedit();
        assert_eq!(editor.text(), "old");
    }

    #[test]
    fn selection_stays_visible_and_word_backspace_respects_unicode() {
        let mut editor = TextEditor::new("hello brave 世界");
        editor.select_all();
        assert_eq!(editor.display_text_with_caret("|"), "hello brave 世界|");
        editor.move_end(false);
        editor.backspace_word();
        assert_eq!(editor.text(), "hello brave ");
        editor.backspace_word();
        assert_eq!(editor.text(), "hello ");
    }

    #[test]
    fn pointer_cursor_can_reach_the_end_of_the_final_grapheme() {
        let mut editor = TextEditor::new("abc");
        editor.place_cursor(0);
        editor.place_cursor("abc".len());
        assert_eq!(editor.cursor(), "abc".len());

        let mut unicode = TextEditor::new("a🦀");
        unicode.place_cursor("a🦀".len());
        assert_eq!(unicode.cursor(), "a🦀".len());
    }

    #[test]
    fn copy_cut_and_paste_preserve_unicode_selection_boundaries() {
        let mut editor = TextEditor::new("copy 🦀 here");
        editor.move_home(false);
        for _ in 0..5 {
            editor.move_right(false);
        }
        editor.move_right(true);
        assert_eq!(editor.selected_text(), Some("🦀"));
        assert_eq!(editor.cut_selection().as_deref(), Some("🦀"));
        assert_eq!(editor.text(), "copy  here");
        editor.insert("世界");
        assert_eq!(editor.text(), "copy 世界 here");
        assert_eq!(editor.cut_selection(), None);
    }

    #[test]
    fn history_restores_text_and_selection_across_multiline_unicode() {
        let mut editor = TextEditor::new("aé🦀\nsecond");
        editor.move_home(false);
        editor.move_right(true);
        editor.insert("z");
        assert_eq!(editor.text(), "zé🦀\nsecond");
        editor.undo();
        assert_eq!(editor.text(), "aé🦀\nsecond");
        assert_eq!(editor.selection(), Some(0..1));
        editor.redo();
        assert_eq!(editor.text(), "zé🦀\nsecond");
        assert_eq!(editor.selection(), None);
    }

    #[test]
    fn debug_never_reproduces_document_preedit_or_history_payloads() {
        let mut editor = TextEditor::new("highly-secret-password");
        editor.select_all();
        editor.insert("previous-secret");
        editor.set_preedit("composition-secret", None);
        let debug = format!("{editor:?}");
        for secret in [
            "highly-secret-password",
            "previous-secret",
            "composition-secret",
        ] {
            assert!(!debug.contains(secret), "Debug leaked {secret:?}: {debug}");
        }
        assert!(debug.contains("text_bytes"));
    }
}
