//! Launcher commands shared by the JSX shell and legacy view fixtures.

use crate::launcher::{Launcher, LauncherView, SettingsDestination};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LauncherAction {
    SetView(LauncherView),
    ActivateResult(usize),
    TogglePin(String),
    RetryPreferencePersistence,
    LaunchApplication(String),
    OpenProject(String),
    SeeAllProjects,
    OpenSettings(SettingsDestination),
    OpenAccount,
    RequestLogout,
    ShowNarrowPrimary,
    SetQuery(String),
    SearchScroll,
    DashboardScroll,
    Dismiss,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LauncherShellEffect {
    ActivateResult(usize),
    TogglePin(String),
    RetryPreferencePersistence,
    LaunchApplication(String),
    OpenProject(String),
    SeeAllProjects,
    OpenSettings(SettingsDestination),
    OpenAccount,
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
        LauncherAction::LaunchApplication(id) => Some(LauncherShellEffect::LaunchApplication(id)),
        LauncherAction::OpenProject(id) => Some(LauncherShellEffect::OpenProject(id)),
        LauncherAction::SeeAllProjects => Some(LauncherShellEffect::SeeAllProjects),
        LauncherAction::OpenSettings(destination) => {
            Some(LauncherShellEffect::OpenSettings(destination))
        }
        LauncherAction::OpenAccount => Some(LauncherShellEffect::OpenAccount),
        LauncherAction::RequestLogout => Some(LauncherShellEffect::RequestLogout),
        LauncherAction::SetQuery(query) => {
            launcher.set_query(&query);
            None
        }
        LauncherAction::ShowNarrowPrimary
        | LauncherAction::SearchScroll
        | LauncherAction::DashboardScroll => None,
        LauncherAction::Dismiss => Some(LauncherShellEffect::Dismiss),
    }
}
