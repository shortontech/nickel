#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RunPrompt {
    history: Vec<String>,
    history_open: bool,
    history_selection: Option<usize>,
}

impl RunPrompt {
    pub fn clear(&mut self) {
        self.history_open = false;
        self.history_selection = None;
    }

    pub fn set_history(&mut self, history: Vec<String>) {
        self.history = history;
        self.history_selection = None;
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }

    pub fn history_open(&self) -> bool {
        self.history_open
    }

    pub fn history_selection(&self) -> Option<usize> {
        self.history_selection
    }

    pub fn toggle_history(&mut self) {
        self.history_open = !self.history_open && !self.history.is_empty();
        self.history_selection = self.history_open.then_some(0);
    }

    pub fn select_history_next(&mut self) {
        if self.history.is_empty() {
            return;
        }
        self.history_open = true;
        self.history_selection = Some(
            (self.history_selection.unwrap_or(usize::MAX).wrapping_add(1)) % self.history.len(),
        );
    }

    pub fn select_history_previous(&mut self) {
        if self.history.is_empty() {
            return;
        }
        self.history_open = true;
        self.history_selection = Some(
            self.history_selection
                .unwrap_or(0)
                .checked_sub(1)
                .unwrap_or(self.history.len() - 1),
        );
    }

    pub fn choose_history(&mut self, index: usize) {
        if index < self.history.len() {
            self.history_selection = Some(index);
        }
        self.history_open = false;
    }

    pub fn record(&mut self, command: &str) {
        self.history.retain(|entry| entry != command);
        self.history.insert(0, command.to_owned());
        self.history.truncate(20);
    }

    pub fn selected_history_command(&self) -> Option<&str> {
        self.history_selection
            .and_then(|index| self.history.get(index))
            .map(String::as_str)
    }

    pub fn submission<'a>(&self, text: &'a str) -> Option<&'a str> {
        let command = text.trim();
        (!command.is_empty()).then_some(command)
    }
}

#[cfg(test)]
mod tests {
    use super::RunPrompt;

    #[test]
    fn submission_trims_and_rejects_empty_commands() {
        let prompt = RunPrompt::default();
        assert_eq!(prompt.submission(""), None);
        assert_eq!(prompt.submission("  notepad.exe  "), Some("notepad.exe"));
    }

    #[test]
    fn history_deduplicates_and_selects_recent_commands() {
        let mut prompt = RunPrompt::default();
        prompt.record("notepad.exe");
        prompt.record("ms-settings:network-wifi");
        prompt.record("notepad.exe");
        assert_eq!(
            prompt.history(),
            ["notepad.exe", "ms-settings:network-wifi"]
        );
        prompt.select_history_next();
        assert_eq!(prompt.selected_history_command(), Some("notepad.exe"));
        prompt.select_history_next();
        assert_eq!(
            prompt.selected_history_command(),
            Some("ms-settings:network-wifi")
        );
    }
}
