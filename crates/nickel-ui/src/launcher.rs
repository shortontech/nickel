use std::collections::HashMap;

use nucleo_matcher::{
    Config, Matcher,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};

pub use crate::model::Application;
use crate::model::{ApplicationId, OpenWindow, WindowGroup};

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

    #[cfg(test)]
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

    pub fn group_windows(&self, windows: &[OpenWindow]) -> Vec<WindowGroup> {
        let mut groups: Vec<WindowGroup> = Vec::new();
        for window in windows {
            let existing = window.application_id.as_ref().and_then(|id| {
                groups
                    .iter()
                    .position(|group| group.application_id.as_ref() == Some(id))
            });
            if let Some(index) = existing {
                groups[index].windows.push(window.clone());
                continue;
            }
            let application_name = window
                .application_id
                .as_ref()
                .and_then(|id| self.application(id))
                .map(|application| application.name().to_owned())
                .or_else(|| (!window.title.is_empty()).then(|| window.title.clone()))
                .unwrap_or_else(|| "Untitled window".into());
            groups.push(WindowGroup {
                application_id: window.application_id.clone(),
                application_name,
                windows: vec![window.clone()],
            });
        }
        groups
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

    pub fn set_query(&mut self, query: &str) {
        self.query.clear();
        self.query
            .extend(query.chars().filter(|character| !character.is_control()));
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

    pub fn select_grid_left(&mut self, columns: usize) {
        if columns != 0 && self.selected % columns != 0 {
            self.selected -= 1;
        }
    }

    pub fn select_grid_right(&mut self, columns: usize) {
        if columns != 0
            && self.selected % columns + 1 < columns
            && self.selected + 1 < self.results.len()
        {
            self.selected += 1;
        }
    }

    pub fn select_grid_up(&mut self, columns: usize) {
        if columns != 0 && self.selected >= columns {
            self.selected -= columns;
        }
    }

    pub fn select_grid_down(&mut self, columns: usize) {
        if columns != 0 && self.selected + columns < self.results.len() {
            self.selected += columns;
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

    #[test]
    fn clear_resets_query_and_selection() {
        let mut launcher = Launcher::default();
        launcher.insert("term");
        launcher.select_next();

        launcher.clear();

        assert_eq!(launcher.query(), "");
        assert_eq!(launcher.selected_index(), 0);
    }

    #[test]
    fn horizontal_grid_navigation_stops_at_row_edges() {
        let mut launcher = Launcher::default();
        launcher.select(5);
        launcher.select_grid_right(6);
        assert_eq!(launcher.selected_index(), 5);
        launcher.select(6);
        launcher.select_grid_left(6);
        assert_eq!(launcher.selected_index(), 6);
    }

    #[test]
    fn vertical_grid_navigation_preserves_column_and_stops_at_partial_row() {
        let mut launcher = Launcher::default();
        launcher.select(1);
        launcher.select_grid_down(6);
        assert_eq!(launcher.selected_index(), 7);
        launcher.select_grid_down(6);
        assert_eq!(launcher.selected_index(), 7);
        launcher.select_grid_up(6);
        assert_eq!(launcher.selected_index(), 1);
    }

    #[test]
    fn windows_group_by_resolved_application_and_keep_titles() {
        use crate::model::{ApplicationId, OpenWindow, WindowId};

        let launcher = Launcher::new(vec![Application::new(
            "org.example.Editor.desktop".into(),
            "Editor".into(),
            None,
            None,
            None,
        )]);
        let application_id = ApplicationId::new("org.example.Editor.desktop");
        let groups = launcher.group_windows(&[
            OpenWindow {
                id: WindowId(1),
                application_id: Some(application_id.clone()),
                active: false,
                title: "First document".into(),
            },
            OpenWindow {
                id: WindowId(2),
                application_id: Some(application_id),
                active: true,
                title: "Second document".into(),
            },
            OpenWindow {
                id: WindowId(3),
                application_id: None,
                active: false,
                title: String::new(),
            },
        ]);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].application_name, "Editor");
        assert_eq!(groups[0].windows.len(), 2);
        assert!(groups[0].active());
        assert_eq!(groups[0].windows[1].title, "Second document");
        assert_eq!(groups[1].application_name, "Untitled window");
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
