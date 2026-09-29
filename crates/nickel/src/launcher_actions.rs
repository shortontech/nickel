//! Launcher commands shared by the JSX shell and launcher model.

use crate::launcher::{Launcher, LauncherView};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LauncherAction {
    SetView(LauncherView),
    ActivateResult(usize),
    TogglePin(String),
    RetryPreferencePersistence,
    OpenProject(String),
    SeeAllProjects,
    RequestLogout,
    SetQuery(String),
    Dismiss,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LauncherShellEffect {
    ActivateResult(usize),
    TogglePin(String),
    RetryPreferencePersistence,
    OpenProject(String),
    SeeAllProjects,
    RequestLogout,
    Dismiss,
}

/// Apply policy that does not depend on a particular launcher view.
pub fn reduce_launcher_action(
    launcher: &mut Launcher,
    action: LauncherAction,
) -> Option<LauncherShellEffect> {
    match action {
        LauncherAction::SetView(next) => {
            launcher.set_view(next);
            None
        }
        LauncherAction::ActivateResult(index) => Some(LauncherShellEffect::ActivateResult(index)),
        LauncherAction::TogglePin(id) => Some(LauncherShellEffect::TogglePin(id)),
        LauncherAction::RetryPreferencePersistence => {
            Some(LauncherShellEffect::RetryPreferencePersistence)
        }
        LauncherAction::OpenProject(id) => Some(LauncherShellEffect::OpenProject(id)),
        LauncherAction::SeeAllProjects => Some(LauncherShellEffect::SeeAllProjects),
        LauncherAction::RequestLogout => Some(LauncherShellEffect::RequestLogout),
        LauncherAction::SetQuery(query) => {
            launcher.set_query(&query);
            None
        }
        LauncherAction::Dismiss => Some(LauncherShellEffect::Dismiss),
    }
}
