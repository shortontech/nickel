use std::collections::HashMap;

use nucleo_matcher::{
    Config, Matcher,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};

pub use crate::model::Application;
use crate::model::ApplicationId;

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

    pub fn application(&self, id: &ApplicationId) -> Option<&Application> {
        self.applications
            .iter()
            .find(|application| application.application_id() == id)
    }

    pub fn applications(&self) -> impl Iterator<Item = &Application> {
        self.applications.iter()
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

    #[cfg(unix)]
    #[test]
    fn launch_spawns_the_parsed_command_without_a_shell() {
        let application = Application::new(
            "test".into(),
            "Test".into(),
            None,
            None,
            Some(vec!["true".into()]),
        );
        let status = application
            .launch()
            .expect("command starts")
            .wait()
            .expect("command exits");
        assert!(status.success());
    }
}
