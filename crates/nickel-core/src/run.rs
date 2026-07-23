#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RunPrompt {
    command: String,
}

impl RunPrompt {
    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn insert(&mut self, text: &str) {
        self.command.push_str(text);
    }

    pub fn backspace(&mut self) {
        self.command.pop();
    }

    pub fn clear(&mut self) {
        self.command.clear();
    }

    pub fn submission(&self) -> Option<&str> {
        let command = self.command.trim();
        (!command.is_empty()).then_some(command)
    }
}

#[cfg(test)]
mod tests {
    use super::RunPrompt;

    #[test]
    fn submission_trims_and_rejects_empty_commands() {
        let mut prompt = RunPrompt::default();
        assert_eq!(prompt.submission(), None);
        prompt.insert("  notepad.exe  ");
        assert_eq!(prompt.submission(), Some("notepad.exe"));
    }
}
