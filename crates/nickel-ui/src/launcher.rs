use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use nucleo_matcher::{
    Config, Matcher,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Application {
    id: String,
    name: String,
    icon: Option<String>,
    icon_path: Option<PathBuf>,
    exec: Option<String>,
}

impl Application {
    pub fn new(
        id: String,
        name: String,
        icon: Option<String>,
        icon_path: Option<PathBuf>,
        exec: Option<String>,
    ) -> Self {
        Self {
            id,
            name,
            icon,
            icon_path,
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

    pub fn icon_path(&self) -> Option<&Path> {
        self.icon_path.as_deref()
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
    pins: HashMap<String, u64>,
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
            .map(|name| Application::new(name.to_lowercase(), name.into(), None, None, None))
            .collect(),
        )
    }
}

impl Launcher {
    pub fn new(applications: Vec<Application>) -> Self {
        let mut launcher = Self {
            query: String::new(),
            applications,
            results: Vec::new(),
            pins: HashMap::new(),
            selected: 0,
        };
        launcher.refresh();
        launcher
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

    pub fn is_pinned(&self, application_id: &str) -> bool {
        self.pins.contains_key(application_id)
    }

    pub fn set_pins(&mut self, pins: Vec<(String, u64)>) {
        self.pins = pins.into_iter().collect();
        self.refresh();
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

    pub fn select(&mut self, index: usize) {
        if index < self.results.len() {
            self.selected = index;
        }
    }

    fn refresh(&mut self) {
        if self.query.is_empty() {
            let mut results: Vec<_> = (0..self.applications.len()).collect();
            results.sort_by_key(|index| {
                self.pins
                    .get(self.applications[*index].id())
                    .map_or((1, *index as u64), |order| (0, *order))
            });
            self.results = results;
            self.selected = 0;
            return;
        }
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

    #[test]
    fn pins_lead_empty_query_in_persisted_order() {
        let mut launcher = Launcher::default();
        launcher.set_pins(vec![("discover".into(), 1), ("firefox".into(), 2)]);
        assert_eq!(
            launcher.result_at(0).map(Application::name),
            Some("Discover")
        );
        assert_eq!(
            launcher.result_at(1).map(Application::name),
            Some("Firefox")
        );
        assert!(launcher.is_pinned("discover"));
    }
}
