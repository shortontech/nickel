use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextEditor {
    text: String,
    cursor: usize,
    anchor: usize,
}

impl TextEditor {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self {
            text,
            cursor,
            anchor: cursor,
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
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.anchor = 0;
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

fn next_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .grapheme_indices(true)
        .nth(1)
        .map(|(index, _)| cursor + index)
        .unwrap_or(text.len())
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
}
