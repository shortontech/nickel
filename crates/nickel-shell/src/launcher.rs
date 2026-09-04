use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use nickel_codex::{Project, Thread, ThreadId, ThreadRuntime, ThreadRuntimeStatus};
use nickel_core::launcher_preferences::LauncherPreferences;

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

#[derive(Clone, Debug)]
pub struct Launcher {
    query: String,
    preedit: String,
    search_open: bool,
    applications: Vec<Application>,
    results: Vec<usize>,
    preferences: LauncherPreferences,
    place_ids: HashSet<String>,
    view: LauncherView,
    selected: usize,
    dashboard_selected: usize,
    search_selected: usize,
    dashboard_projects: DashboardSection<Vec<DashboardProject>>,
    dashboard_account: DashboardSection<DashboardAccount>,
    codex_availability: CodexAvailability,
    logout_available: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LauncherMode {
    #[default]
    Dashboard,
    Search,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LauncherInput {
    Text(String),
    Preedit(String),
    Backspace,
    Escape,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LauncherInputOutcome {
    Updated,
    DismissRequested,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CodexAvailability {
    #[default]
    Unavailable,
    Recoverable,
    Ready,
}

impl CodexAvailability {
    pub fn shell_entry_visible(self) -> bool {
        self != Self::Unavailable
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProjectActivity {
    Active,
    Idle,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DashboardProject {
    pub id: String,
    pub name: String,
    pub roots: Vec<PathBuf>,
    pub chat_count: Option<usize>,
    pub activity: ProjectActivity,
    pub last_used_at: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DashboardAccount {
    pub display_name: String,
    pub supporting_text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsDestination {
    Nickel,
    KeyboardShortcuts,
    About,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DashboardAction {
    LaunchFavorite(String),
    OpenProject(String),
    SeeAllProjects,
    OpenSettings(SettingsDestination),
    OpenAccount,
    RequestLogout,
    FocusSearch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DashboardSection<T> {
    Loading,
    Empty,
    Ready(T),
    Failed { message: String, recoverable: bool },
    Unavailable(String),
}

pub fn normalize_dashboard_projects(
    projects: &[Project],
    threads: &[Thread],
    runtime: &HashMap<ThreadId, ThreadRuntime>,
) -> Vec<DashboardProject> {
    let mut normalized = projects
        .iter()
        .enumerate()
        .map(|(order, project)| {
            let matching = threads.iter().filter(|thread| {
                let runtime_project = runtime
                    .get(&thread.id)
                    .and_then(|entry| entry.project_id.as_deref());
                runtime_project.map_or_else(
                    || {
                        thread.cwd.as_ref().is_some_and(|cwd| {
                            project
                                .roots
                                .iter()
                                .any(|root| cwd == root || cwd.starts_with(root))
                        })
                    },
                    |id| id == project.id,
                )
            });
            let mut chat_count = 0;
            let mut last_used_at = None;
            let mut activity = ProjectActivity::Unknown;
            for thread in matching {
                chat_count += 1;
                if let Some(used) = thread.last_used_at {
                    last_used_at =
                        Some(last_used_at.map_or(used, |current: i64| current.max(used)));
                }
                activity = match runtime.get(&thread.id).map(|entry| &entry.status) {
                    Some(ThreadRuntimeStatus::Active) => ProjectActivity::Active,
                    Some(ThreadRuntimeStatus::Idle) if activity != ProjectActivity::Active => {
                        ProjectActivity::Idle
                    }
                    _ => activity,
                };
            }
            (
                order,
                DashboardProject {
                    id: project.id.clone(),
                    name: project.name.clone(),
                    roots: project.roots.clone(),
                    chat_count: Some(chat_count),
                    activity,
                    last_used_at,
                },
            )
        })
        .collect::<Vec<_>>();
    normalized.sort_by(|(left_order, left), (right_order, right)| {
        right
            .last_used_at
            .cmp(&left.last_used_at)
            .then_with(|| left_order.cmp(right_order))
    });
    normalized.into_iter().map(|(_, project)| project).collect()
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LauncherView {
    #[default]
    Favorites,
    Applications,
    Places,
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
    pub fn reduce_input(&mut self, input: LauncherInput) -> LauncherInputOutcome {
        match input {
            LauncherInput::Text(text) => self.insert(&text),
            LauncherInput::Preedit(text) => self.set_preedit(&text),
            LauncherInput::Backspace => self.backspace(),
            LauncherInput::Escape => {
                if !self.cancel_preedit() && !self.clear_query() {
                    return LauncherInputOutcome::DismissRequested;
                }
            }
        }
        LauncherInputOutcome::Updated
    }

    pub fn new(applications: Vec<Application>) -> Self {
        let mut launcher = Self {
            query: String::new(),
            preedit: String::new(),
            search_open: false,
            applications,
            results: Vec::new(),
            preferences: LauncherPreferences::default(),
            place_ids: HashSet::new(),
            view: LauncherView::default(),
            selected: 0,
            dashboard_selected: 0,
            search_selected: 0,
            dashboard_projects: DashboardSection::Loading,
            dashboard_account: DashboardSection::Loading,
            codex_availability: CodexAvailability::Unavailable,
            logout_available: true,
        };
        launcher.refresh();
        launcher
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    pub fn mode(&self) -> LauncherMode {
        if !self.search_open && self.query.is_empty() && self.preedit.is_empty() {
            LauncherMode::Dashboard
        } else {
            LauncherMode::Search
        }
    }

    fn effective_query(&self) -> String {
        let mut query = self.query.clone();
        query.push_str(&self.preedit);
        query
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

    pub fn view(&self) -> LauncherView {
        self.view
    }

    pub fn set_view(&mut self, view: LauncherView) {
        self.view = view;
        self.refresh();
    }

    pub fn set_places(&mut self, places: Vec<Application>) {
        self.applications
            .retain(|application| !self.place_ids.contains(application.id()));
        self.place_ids.clear();
        for place in places {
            self.place_ids.insert(place.id().to_owned());
            self.applications.push(place);
        }
        self.refresh();
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

    pub fn favorite_applications(&self) -> Vec<&Application> {
        self.preferences
            .favorites()
            .iter()
            .filter_map(|id| {
                self.applications
                    .iter()
                    .find(|application| application.id() == id)
            })
            .collect()
    }

    pub fn recent_applications(&self) -> Vec<&Application> {
        self.preferences
            .recents()
            .iter()
            .filter_map(|id| {
                self.applications
                    .iter()
                    .find(|application| application.id() == id)
            })
            .collect()
    }

    pub fn preferences(&self) -> &LauncherPreferences {
        &self.preferences
    }

    pub fn set_preferences(&mut self, preferences: LauncherPreferences) {
        self.preferences = preferences;
        self.refresh();
    }

    pub fn record_launch(&mut self, application_id: &str) {
        self.preferences.record_launch(application_id);
    }

    pub fn dashboard_projects(&self) -> &DashboardSection<Vec<DashboardProject>> {
        &self.dashboard_projects
    }

    pub fn codex_available(&self) -> bool {
        self.codex_availability.shell_entry_visible()
    }

    pub fn codex_availability(&self) -> CodexAvailability {
        self.codex_availability
    }

    pub fn set_codex_availability(&mut self, availability: CodexAvailability) -> bool {
        if self.codex_availability == availability {
            return false;
        }
        self.codex_availability = availability;
        if availability == CodexAvailability::Unavailable {
            self.dashboard_projects = DashboardSection::Unavailable(
                "Codex is not installed or the integration is disabled".into(),
            );
        }
        true
    }

    #[cfg(any(test, feature = "workbench-fixtures"))]
    pub fn set_codex_available(&mut self, available: bool) -> bool {
        self.set_codex_availability(if available {
            CodexAvailability::Ready
        } else {
            CodexAvailability::Unavailable
        })
    }

    pub fn set_dashboard_projects(
        &mut self,
        projects: DashboardSection<Vec<DashboardProject>>,
    ) -> bool {
        if self.dashboard_projects == projects {
            return false;
        }
        self.dashboard_projects = projects;
        true
    }

    pub fn dashboard_account(&self) -> &DashboardSection<DashboardAccount> {
        &self.dashboard_account
    }

    pub fn set_dashboard_account(&mut self, account: DashboardSection<DashboardAccount>) -> bool {
        if self.dashboard_account == account {
            return false;
        }
        self.dashboard_account = account;
        true
    }

    pub fn logout_available(&self) -> bool {
        self.logout_available
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
        self.preferences.is_favorite(application_id)
    }

    pub fn toggle_pin(&mut self, application_id: &str) {
        self.preferences.toggle_favorite(application_id);
        self.refresh();
    }

    pub fn set_pins(&mut self, mut pins: Vec<(String, u64)>) {
        pins.sort_by_key(|(_, order)| *order);
        self.preferences
            .replace_favorites(pins.into_iter().map(|(id, _)| id));
        self.refresh();
    }

    pub fn insert(&mut self, text: &str) {
        self.remember_selection();
        self.query
            .extend(text.chars().filter(|character| !character.is_control()));
        self.preedit.clear();
        self.search_selected = 0;
        self.refresh();
    }

    pub fn open_search(&mut self) {
        self.remember_selection();
        self.search_open = true;
        self.view = LauncherView::Applications;
        self.refresh();
    }

    pub fn set_query(&mut self, query: &str) {
        self.remember_selection();
        self.query.clear();
        self.query
            .extend(query.chars().filter(|character| !character.is_control()));
        self.preedit.clear();
        self.search_selected = 0;
        self.refresh();
    }

    pub fn set_preedit(&mut self, preedit: &str) {
        self.remember_selection();
        self.preedit.clear();
        self.preedit
            .extend(preedit.chars().filter(|character| !character.is_control()));
        self.search_selected = 0;
        self.refresh();
    }

    pub fn cancel_preedit(&mut self) -> bool {
        if self.preedit.is_empty() {
            return false;
        }
        self.remember_selection();
        self.preedit.clear();
        self.refresh();
        true
    }

    pub fn clear_query(&mut self) -> bool {
        if self.query.is_empty() && !self.search_open {
            return false;
        }
        self.remember_selection();
        self.query.clear();
        self.search_open = false;
        self.refresh();
        true
    }

    pub fn backspace(&mut self) {
        self.remember_selection();
        self.query.pop();
        if self.query.is_empty() && self.preedit.is_empty() {
            self.search_open = false;
        }
        self.search_selected = 0;
        self.refresh();
    }

    pub fn clear(&mut self) {
        self.remember_selection();
        self.query.clear();
        self.preedit.clear();
        self.search_open = false;
        self.refresh();
    }

    pub fn select_next(&mut self) {
        if !self.results.is_empty() {
            self.selected = (self.selected + 1) % self.results.len();
            self.remember_selection();
        }
    }

    pub fn select_previous(&mut self) {
        if !self.results.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.results.len() - 1);
            self.remember_selection();
        }
    }

    pub fn select(&mut self, index: usize) {
        if index < self.results.len() {
            self.selected = index;
            self.remember_selection();
        }
    }

    fn remember_selection(&mut self) {
        match self.mode() {
            LauncherMode::Dashboard => self.dashboard_selected = self.selected,
            LauncherMode::Search => self.search_selected = self.selected,
        }
    }

    pub fn select_grid_left(&mut self, columns: usize) {
        if columns != 0 && self.selected % columns != 0 {
            self.selected -= 1;
            self.remember_selection();
        }
    }

    pub fn select_grid_right(&mut self, columns: usize) {
        if columns != 0
            && self.selected % columns + 1 < columns
            && self.selected + 1 < self.results.len()
        {
            self.selected += 1;
            self.remember_selection();
        }
    }

    pub fn select_grid_up(&mut self, columns: usize) {
        if columns != 0 && self.selected >= columns {
            self.selected -= columns;
            self.remember_selection();
        }
    }

    pub fn select_grid_down(&mut self, columns: usize) {
        if columns != 0 && self.selected + columns < self.results.len() {
            self.selected += columns;
            self.remember_selection();
        }
    }

    fn refresh(&mut self) {
        let mode = self.mode();
        if mode == LauncherMode::Dashboard {
            let results: Vec<_> = match self.view {
                LauncherView::Favorites => {
                    let mut ids = self.preferences.favorites().to_vec();
                    for id in self.preferences.recents() {
                        if !ids.contains(id) {
                            ids.push(id.clone());
                        }
                    }
                    if ids.is_empty() {
                        self.applications
                            .iter()
                            .enumerate()
                            .filter(|(_, application)| !self.place_ids.contains(application.id()))
                            .map(|(index, _)| index)
                            .collect()
                    } else {
                        ids.into_iter()
                            .filter_map(|id| {
                                self.applications
                                    .iter()
                                    .position(|application| application.id() == id)
                            })
                            .collect()
                    }
                }
                LauncherView::Applications => self
                    .applications
                    .iter()
                    .enumerate()
                    .filter(|(_, application)| !self.place_ids.contains(application.id()))
                    .map(|(index, _)| index)
                    .collect(),
                LauncherView::Places => self
                    .applications
                    .iter()
                    .enumerate()
                    .filter(|(_, application)| self.place_ids.contains(application.id()))
                    .map(|(index, _)| index)
                    .collect(),
            };
            self.results = results;
            self.selected = self
                .dashboard_selected
                .min(self.results.len().saturating_sub(1));
            return;
        }
        let effective_query = self.effective_query();
        let pattern = Pattern::new(
            &effective_query,
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
        self.selected = self
            .search_selected
            .min(self.results.len().saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use nickel_codex::{Project, Thread, ThreadId, ThreadRuntime, ThreadRuntimeStatus};

    use super::{
        Application, CodexAvailability, DashboardProject, DashboardSection, Launcher, LauncherMode,
        LauncherView, ProjectActivity, normalize_dashboard_projects,
    };

    fn project(id: &str, root: &str) -> Project {
        Project {
            id: id.into(),
            name: id.into(),
            roots: vec![PathBuf::from(root)],
        }
    }

    fn thread(id: &str, cwd: &str, last_used_at: Option<i64>) -> Thread {
        Thread {
            id: ThreadId(id.into()),
            title: None,
            cwd: Some(PathBuf::from(cwd)),
            last_used_at,
            turns: Vec::new(),
            model: None,
            reasoning_effort: None,
        }
    }

    #[test]
    fn dashboard_projects_sort_by_activity_with_stable_ties() {
        let projects = vec![
            project("first", "/projects/first"),
            project("second", "/projects/second"),
            project("unused", "/projects/unused"),
        ];
        let threads = vec![
            thread("a", "/projects/first", Some(10)),
            thread("b", "/projects/second", Some(20)),
            thread("c", "/projects/first/subdir", Some(20)),
        ];

        let result = normalize_dashboard_projects(&projects, &threads, &HashMap::new());

        assert_eq!(
            result
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "unused"]
        );
        assert_eq!(result[0].chat_count, Some(2));
        assert_eq!(result[2].last_used_at, None);
    }

    #[test]
    fn dashboard_project_activity_is_conservative_and_active_wins() {
        let projects = vec![project("nickel", "/projects/nickel")];
        let threads = vec![
            thread("idle", "/elsewhere", Some(10)),
            thread("active", "/elsewhere", Some(20)),
            thread("unknown", "/elsewhere", Some(30)),
        ];
        let runtime = HashMap::from([
            (
                ThreadId("idle".into()),
                ThreadRuntime {
                    project_id: Some("nickel".into()),
                    status: ThreadRuntimeStatus::Idle,
                    ..ThreadRuntime::default()
                },
            ),
            (
                ThreadId("active".into()),
                ThreadRuntime {
                    project_id: Some("nickel".into()),
                    status: ThreadRuntimeStatus::Active,
                    ..ThreadRuntime::default()
                },
            ),
            (
                ThreadId("unknown".into()),
                ThreadRuntime {
                    project_id: Some("nickel".into()),
                    status: ThreadRuntimeStatus::SystemError,
                    ..ThreadRuntime::default()
                },
            ),
        ]);

        let result = normalize_dashboard_projects(&projects, &threads, &runtime);

        assert_eq!(result[0].chat_count, Some(3));
        assert_eq!(result[0].activity, ProjectActivity::Active);
        assert_eq!(result[0].last_used_at, Some(30));
    }

    #[test]
    fn explicit_project_identity_overrides_root_fallback() {
        let projects = vec![
            project("outer", "/projects"),
            project("nickel", "/projects/nickel"),
        ];
        let threads = vec![thread("chat", "/projects/nickel", Some(10))];
        let runtime = HashMap::from([(
            ThreadId("chat".into()),
            ThreadRuntime {
                project_id: Some("nickel".into()),
                status: ThreadRuntimeStatus::NotLoaded,
                ..ThreadRuntime::default()
            },
        )]);

        let result = normalize_dashboard_projects(&projects, &threads, &runtime);

        assert_eq!(result[0].id, "nickel");
        assert_eq!(result[0].chat_count, Some(1));
        assert_eq!(result[0].activity, ProjectActivity::Unknown);
        assert_eq!(result[1].chat_count, Some(0));
    }

    #[test]
    fn empty_query_is_dashboard_and_text_or_preedit_is_search() {
        let mut launcher = Launcher::default();
        assert_eq!(launcher.mode(), LauncherMode::Dashboard);

        launcher.set_preedit("fir");
        assert_eq!(launcher.mode(), LauncherMode::Search);
        assert_eq!(
            launcher.selected_result().map(Application::name),
            Some("Firefox")
        );

        assert!(launcher.cancel_preedit());
        assert_eq!(launcher.mode(), LauncherMode::Dashboard);
        launcher.insert("fire");
        assert_eq!(launcher.mode(), LauncherMode::Search);
        assert!(launcher.clear_query());
        assert_eq!(launcher.mode(), LauncherMode::Dashboard);
    }

    #[test]
    fn all_applications_opens_empty_search_and_escape_returns_to_dashboard() {
        let mut launcher = Launcher::default();

        launcher.open_search();

        assert_eq!(launcher.mode(), LauncherMode::Search);
        assert!(launcher.query().is_empty());
        assert_eq!(launcher.view(), LauncherView::Applications);
        assert!(launcher.result_count() > 0);
        assert!(launcher.clear_query());
        assert_eq!(launcher.mode(), LauncherMode::Dashboard);
    }

    #[test]
    fn dashboard_and_search_keep_independent_selection() {
        let mut launcher = Launcher::default();
        launcher.select_next();
        assert_eq!(launcher.selected_index(), 1);

        launcher.insert("cal");
        assert_eq!(launcher.selected_index(), 0);
        launcher.clear_query();

        assert_eq!(launcher.selected_index(), 1);
    }

    #[test]
    fn reopening_empty_search_restores_its_selection() {
        let mut launcher = Launcher::default();
        launcher.open_search();
        launcher.select_next();
        assert_eq!(launcher.selected_index(), 1);

        launcher.clear_query();
        launcher.open_search();

        assert_eq!(launcher.selected_index(), 1);
    }

    #[test]
    fn dashboard_refresh_and_failure_cannot_change_search_results() {
        let mut launcher = Launcher::default();
        launcher.insert("cal");
        let before = launcher
            .results
            .iter()
            .map(|index| launcher.applications[*index].id().to_owned())
            .collect::<Vec<_>>();
        let selected = launcher.selected_index();

        launcher.set_dashboard_projects(DashboardSection::Unavailable("Codex disconnected".into()));
        launcher.set_dashboard_account(DashboardSection::Unavailable("Account unavailable".into()));

        let after = launcher
            .results
            .iter()
            .map(|index| launcher.applications[*index].id().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(launcher.mode(), LauncherMode::Search);
        assert_eq!(launcher.query(), "cal");
        assert_eq!(after, before);
        assert_eq!(launcher.selected_index(), selected);
    }

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
    fn places_are_separate_from_the_application_catalog() {
        let mut launcher = Launcher::default();
        launcher.set_places(vec![Application::new(
            "place:home".into(),
            "Home".into(),
            None,
            None,
            Some(vec!["nickel-file".into(), "/home/example".into()]),
        )]);
        assert!(
            launcher
                .applications()
                .any(|application| application.name() == "Home")
        );
        assert!(
            launcher
                .results
                .iter()
                .all(|index| launcher.applications[*index].name() != "Home")
        );

        launcher.set_view(LauncherView::Places);
        assert_eq!(launcher.result_count(), 1);
        assert_eq!(launcher.result_at(0).map(Application::name), Some("Home"));
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
                state: crate::model::WindowState::default(),
            },
            OpenWindow {
                id: WindowId(2),
                application_id: Some(application_id),
                active: true,
                title: "Second document".into(),
                state: crate::model::WindowState::default(),
            },
            OpenWindow {
                id: WindowId(3),
                application_id: None,
                active: false,
                title: String::new(),
                state: crate::model::WindowState::default(),
            },
        ]);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].application_name, "Editor");
        assert_eq!(groups[0].windows.len(), 2);
        assert!(groups[0].active());
        assert_eq!(groups[0].windows[1].title, "Second document");
        assert_eq!(groups[1].application_name, "Untitled window");
    }

    #[test]
    fn unavailable_codex_clears_retained_projects_and_hides_shell_entry() {
        let mut launcher = Launcher::default();
        launcher.set_codex_availability(CodexAvailability::Ready);
        launcher.set_dashboard_projects(DashboardSection::Ready(vec![DashboardProject {
            id: "private".into(),
            name: "Private project".into(),
            roots: vec![PathBuf::from("/private")],
            chat_count: Some(1),
            activity: ProjectActivity::Idle,
            last_used_at: None,
        }]));

        assert!(launcher.set_codex_availability(CodexAvailability::Unavailable));
        assert!(!launcher.codex_available());
        assert!(matches!(
            launcher.dashboard_projects(),
            DashboardSection::Unavailable(_)
        ));
    }

    #[test]
    fn recoverable_codex_health_keeps_shell_entry_reachable() {
        let mut launcher = Launcher::default();
        launcher.set_codex_availability(CodexAvailability::Recoverable);

        assert!(launcher.codex_available());
        assert_eq!(
            launcher.codex_availability(),
            CodexAvailability::Recoverable
        );
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
