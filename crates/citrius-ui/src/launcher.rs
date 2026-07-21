use nucleo_matcher::{
    Config, Matcher,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};

const CATALOG: [&str; 8] = [
    "Firefox",
    "Files",
    "Konsole",
    "System Settings",
    "Visual Studio Code",
    "Calculator",
    "Calendar",
    "Discover",
];

#[derive(Debug)]
pub struct Launcher {
    query: String,
    results: Vec<&'static str>,
    selected: usize,
}

impl Default for Launcher {
    fn default() -> Self {
        Self {
            query: String::new(),
            results: CATALOG.to_vec(),
            selected: 0,
        }
    }
}

impl Launcher {
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn results(&self) -> &[&'static str] {
        &self.results
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected_result(&self) -> Option<&'static str> {
        self.results.get(self.selected).copied()
    }

    pub fn insert(&mut self, text: &str) {
        self.query
            .extend(text.chars().filter(|character| !character.is_control()));
        self.refresh();
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.refresh();
    }

    pub fn clear(&mut self) {
        self.query.clear();
        self.refresh();
    }

    pub fn select_next(&mut self) {
        if !self.results.is_empty() {
            self.selected = (self.selected + 1) % self.results.len();
        }
    }

    pub fn select_previous(&mut self) {
        if !self.results.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.results.len() - 1);
        }
    }

    fn refresh(&mut self) {
        let pattern = Pattern::new(
            &self.query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut matcher = Matcher::new(Config::DEFAULT);
        self.results = pattern
            .match_list(CATALOG, &mut matcher)
            .into_iter()
            .map(|(item, _score)| item)
            .collect();
        self.selected = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::Launcher;

    #[test]
    fn fuzzy_query_finds_application() {
        let mut launcher = Launcher::default();
        launcher.insert("firfox");
        assert_eq!(launcher.results(), &["Firefox"]);
        assert_eq!(launcher.selected_result(), Some("Firefox"));
    }

    #[test]
    fn selection_wraps_in_both_directions() {
        let mut launcher = Launcher::default();
        launcher.select_previous();
        assert_eq!(launcher.selected_result(), Some("Discover"));
        launcher.select_next();
        assert_eq!(launcher.selected_result(), Some("Firefox"));
    }

    #[test]
    fn editing_resets_selection_to_first_match() {
        let mut launcher = Launcher::default();
        launcher.select_next();
        launcher.insert("cal");
        assert_eq!(launcher.selected_index(), 0);
        assert_eq!(launcher.selected_result(), Some("Calculator"));
        launcher.backspace();
        assert_eq!(launcher.query(), "ca");
    }
}
