//! The bounded, path-free boundary for a JavaScript project menu.

use nickel_codex::Project;
use serde::{Deserialize, Serialize};

use crate::{ChatState, ConnectionStatus};

const MAX_PROJECTS: usize = 100;
const MAX_PROJECT_ID_BYTES: usize = 256;
const MAX_PROJECT_NAME_CHARS: usize = 160;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMenuRevision {
    pub connection: u64,
    pub projects: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMenuEntry {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMenuProjection {
    pub revision: ProjectMenuRevision,
    pub status: &'static str,
    pub projects: Vec<ProjectMenuEntry>,
}

impl ProjectMenuProjection {
    pub fn from_state(state: &ChatState) -> Self {
        let status = match state.status {
            ConnectionStatus::Loading => "loading",
            ConnectionStatus::Ready if !state.account.authenticated => "sign-in-required",
            ConnectionStatus::Ready => "ready",
            ConnectionStatus::Unavailable => "unavailable",
            ConnectionStatus::Disconnected => "disconnected",
            ConnectionStatus::Incompatible => "incompatible",
        };
        let projects = if status == "ready" {
            state
                .projects
                .iter()
                .take(MAX_PROJECTS)
                .enumerate()
                .filter(|(_, project)| {
                    !project.id.is_empty()
                        && project.id.len() <= MAX_PROJECT_ID_BYTES
                        && project
                            .roots
                            .first()
                            .is_some_and(|root| !root.as_os_str().is_empty())
                })
                .map(|(index, project)| ProjectMenuEntry {
                    id: index.to_string(),
                    name: project.name.chars().take(MAX_PROJECT_NAME_CHARS).collect(),
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            revision: ProjectMenuRevision {
                connection: state.generation,
                projects: state.project_revision,
            },
            status,
            projects,
        }
    }

    /// Resolve a plugin request to a host-owned project after rechecking its snapshot.
    pub fn resolve_open<'a>(
        &self,
        state: &'a ChatState,
        observed: ProjectMenuRevision,
        project_id: &str,
    ) -> Option<&'a Project> {
        if self.revision != observed
            || self.revision.connection != state.generation
            || self.revision.projects != state.project_revision
            || self.status != "ready"
            || state.status != ConnectionStatus::Ready
            || !state.account.authenticated
        {
            return None;
        }
        let index = project_id.parse::<usize>().ok()?;
        let entry = self
            .projects
            .iter()
            .find(|project| project.id == project_id)?;
        if entry.id != index.to_string() {
            return None;
        }
        let project = state.projects.get(index)?;
        let mut matches = state
            .projects
            .iter()
            .filter(|candidate| candidate.id == project.id);
        matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        project
            .roots
            .first()
            .filter(|root| !root.as_os_str().is_empty())?;
        Some(project)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use nickel_codex::{AccountState, Project};

    use super::*;

    #[test]
    fn projection_hides_roots_and_rejects_stale_or_forged_open_requests() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        state.account.authenticated = true;
        state.projects = vec![Project {
            id: "/private/backend-id".into(),
            name: "Example".into(),
            roots: vec![PathBuf::from("/private/work")],
        }];
        let projection = ProjectMenuProjection::from_state(&state);
        let json = serde_json::to_string(&projection).unwrap();
        assert!(json.contains("\"id\":\"0\""));
        assert!(!json.contains("/private/backend-id"));
        assert!(!json.contains("/private/work"));
        assert_eq!(
            projection
                .resolve_open(&state, projection.revision, "0")
                .map(|project| project.roots[0].as_path()),
            Some(std::path::Path::new("/private/work"))
        );
        assert!(
            projection
                .resolve_open(&state, projection.revision, "forged")
                .is_none()
        );
        state.project_revision += 1;
        assert!(
            projection
                .resolve_open(&state, projection.revision, "0")
                .is_none()
        );
        state.project_revision -= 1;
        state.account.authenticated = false;
        assert!(
            projection
                .resolve_open(&state, projection.revision, "0")
                .is_none()
        );
    }

    #[test]
    fn backend_refresh_invalidates_a_project_menu_request_only_when_projects_change() {
        let mut state = ChatState::default();
        let ready = |root: &str| crate::ControllerEvent::Ready {
            provenance: "test".into(),
            account: AccountState {
                authenticated: true,
                ..Default::default()
            },
            models: Vec::new(),
            projects: vec![Project {
                id: "project-1".into(),
                name: "Example".into(),
                roots: vec![PathBuf::from(root)],
            }],
            threads: Vec::new(),
            runtime: Default::default(),
            thread_error: None,
            thread_next_cursor: None,
        };
        state.apply(state.generation, ready("/first"));
        let projection = ProjectMenuProjection::from_state(&state);
        state.apply(state.generation, ready("/first"));
        assert_eq!(
            projection
                .resolve_open(&state, projection.revision, "0")
                .map(|project| project.roots[0].as_path()),
            Some(std::path::Path::new("/first"))
        );
        state.apply(state.generation, ready("/second"));
        assert!(
            projection
                .resolve_open(&state, projection.revision, "0")
                .is_none()
        );
    }
}
