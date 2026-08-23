use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextEditor {
    text: String,
    cursor: usize,
    anchor: usize,
    preedit: String,
    preedit_cursor: Option<Range<usize>>,
}

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
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn selection(&self) -> Option<Range<usize>> {
        (self.cursor != self.anchor)
            .then(|| self.cursor.min(self.anchor)..self.cursor.max(self.anchor))
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|range| &self.text[range])
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
        self.anchor = self.cursor;
        self.cancel_preedit();
    }

    pub fn clear(&mut self) {
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
        let start = range.start;
        self.text.replace_range(range, replacement);
        self.cursor = start + replacement.len();
        self.anchor = self.cursor;
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() || self.cursor == 0 {
            return;
        }
        let previous = previous_boundary(&self.text, self.cursor);
        self.text.replace_range(previous..self.cursor, "");
        self.cursor = previous;
        self.anchor = previous;
    }

    pub fn backspace_word(&mut self) {
        if self.delete_selection() || self.cursor == 0 {
            return;
        }
        let previous = previous_word_boundary(&self.text, self.cursor);
        self.text.replace_range(previous..self.cursor, "");
        self.cursor = previous;
        self.anchor = previous;
    }

    pub fn delete(&mut self) {
        if self.delete_selection() || self.cursor == self.text.len() {
            return;
        }
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
}
