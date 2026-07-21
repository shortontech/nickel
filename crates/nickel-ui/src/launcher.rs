use nucleo_matcher::{
    Config, Matcher,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};

const MAX_RESULTS: usize = 9;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Application {
    id: String,
    name: String,
    icon: Option<String>,
    exec: Option<String>,
}

impl Application {
    pub fn new(id: String, name: String, icon: Option<String>, exec: Option<String>) -> Self {
        Self {
            id,
            name,
            icon,
            exec,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    pub fn exec(&self) -> Option<&str> {
        self.exec.as_deref()
    }
}

#[derive(Clone, Copy)]
struct Candidate<'a> {
    index: usize,
    name: &'a str,
}

impl AsRef<str> for Candidate<'_> {
    fn as_ref(&self) -> &str {
        self.name
    }
}

#[derive(Debug)]
pub struct Launcher {
    query: String,
    applications: Vec<Application>,
    results: Vec<usize>,
    selected: usize,
}

impl Default for Launcher {
    fn default() -> Self {
        Self::new(
            [
                "Firefox",
                "Files",
                "Konsole",
                "System Settings",
                "Visual Studio Code",
                "Calculator",
                "Calendar",
                "Discover",
            ]
            .into_iter()
            .map(|name| Application::new(name.to_lowercase(), name.into(), None, None))
            .collect(),
        )
    }
}

impl Launcher {
    pub fn new(applications: Vec<Application>) -> Self {
        let results = (0..applications.len().min(MAX_RESULTS)).collect();
        Self {
            query: String::new(),
            applications,
            results,
            selected: 0,
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn result_count(&self) -> usize {
        self.results.len()
    }

    pub fn result_at(&self, index: usize) -> Option<&Application> {
        self.results
            .get(index)
            .and_then(|application| self.applications.get(*application))
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected_result(&self) -> Option<&Application> {
        self.result_at(self.selected)
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
        let candidates = self
            .applications
            .iter()
            .enumerate()
            .map(|(index, application)| Candidate {
                index,
                name: application.name(),
            });
        self.results = pattern
            .match_list(candidates, &mut matcher)
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(candidate, _score)| candidate.index)
            .collect();
        self.selected = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{Application, Launcher};

    #[test]
    fn fuzzy_query_finds_application() {
        let mut launcher = Launcher::default();
        launcher.insert("firfox");
        assert_eq!(launcher.result_count(), 1);
        assert_eq!(
            launcher.selected_result().map(Application::name),
            Some("Firefox")
        );
    }

    #[test]
    fn selection_wraps_in_both_directions() {
        let mut launcher = Launcher::default();
        launcher.select_previous();
        assert_eq!(
            launcher.selected_result().map(Application::name),
            Some("Discover")
        );
        launcher.select_next();
        assert_eq!(
            launcher.selected_result().map(Application::name),
            Some("Firefox")
        );
    }

    #[test]
    fn editing_resets_selection_to_first_match() {
        let mut launcher = Launcher::default();
        launcher.select_next();
        launcher.insert("cal");
        assert_eq!(launcher.selected_index(), 0);
        assert_eq!(
            launcher.selected_result().map(Application::name),
            Some("Calculator")
        );
        launcher.backspace();
        assert_eq!(launcher.query(), "ca");
    }
}
