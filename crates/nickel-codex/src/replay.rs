use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
};

use serde::{Deserialize, Serialize};

use crate::{
    AccountState, CodexBackend, CodexError, CodexEvent, EventKind, InteractionResponse, Model,
    Project, ProjectPage, ProjectPageResult, ServerRequestId, StartThread, StartTurn, Thread,
    ThreadId, ThreadPage, ThreadPageResult, ThreadRuntime, Turn, TurnId,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayScenario {
    pub name: String,
    #[serde(default)]
    pub account: AccountState,
    #[serde(default)]
    pub models: Vec<Model>,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub threads: Vec<Thread>,
    #[serde(default)]
    pub thread_runtime: HashMap<ThreadId, ThreadRuntime>,
    #[serde(default)]
    pub thread_error: Option<String>,
    #[serde(default)]
    pub events: Vec<CodexEvent>,
}

#[derive(Clone)]
pub struct ReplayBackend {
    scenario: Arc<ReplayScenario>,
    pending: Arc<Mutex<HashMap<String, String>>>,
    started_turns: Arc<Mutex<Vec<StartTurn>>>,
    resumed_threads: Arc<Mutex<Vec<ThreadId>>>,
}

impl ReplayBackend {
    pub fn from_json(input: &str) -> Result<Self, CodexError> {
        let scenario: ReplayScenario = serde_json::from_str(input)?;
        let pending = scenario
            .events
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::ApprovalRequested {
                    request_id,
                    approval_type,
                    ..
                } => Some((request_id.0.clone(), approval_type.clone())),
                EventKind::UserInputRequested { request_id, .. } => {
                    Some((request_id.0.clone(), "user_input".into()))
                }
                _ => None,
            })
            .collect();
        Ok(Self {
            scenario: Arc::new(scenario),
            pending: Arc::new(Mutex::new(pending)),
            started_turns: Arc::new(Mutex::new(Vec::new())),
            resumed_threads: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub fn started_turns(&self) -> Vec<StartTurn> {
        self.started_turns.lock().unwrap().clone()
    }

    pub fn resumed_threads(&self) -> Vec<ThreadId> {
        self.resumed_threads.lock().unwrap().clone()
    }
}

impl CodexBackend for ReplayBackend {
    fn account(&self) -> Result<AccountState, CodexError> {
        Ok(self.scenario.account.clone())
    }
    fn models(&self) -> Result<Vec<Model>, CodexError> {
        Ok(self.scenario.models.clone())
    }
    fn list_projects(&self, page: ProjectPage) -> Result<ProjectPageResult, CodexError> {
        let (start, end, next_cursor) = page_bounds(
            self.scenario.projects.len(),
            page.cursor.as_deref(),
            page.limit,
        )?;
        Ok(ProjectPageResult {
            projects: self.scenario.projects[start..end].to_vec(),
            next_cursor,
        })
    }
    fn import_project(&self, project: crate::ImportProject) -> Result<Project, CodexError> {
        Ok(Project {
            id: project.idempotency_key,
            name: project.name,
            roots: project.roots,
        })
    }
    fn list_threads(&self, page: ThreadPage) -> Result<ThreadPageResult, CodexError> {
        if let Some(message) = &self.scenario.thread_error {
            return Err(CodexError::Protocol(message.clone()));
        }
        let (start, end, next_cursor) = page_bounds(
            self.scenario.threads.len(),
            page.cursor.as_deref(),
            page.limit,
        )?;
        let threads = self.scenario.threads[start..end].to_vec();
        let runtime = threads
            .iter()
            .filter_map(|thread| {
                self.scenario
                    .thread_runtime
                    .get(&thread.id)
                    .cloned()
                    .map(|runtime| (thread.id.clone(), runtime))
            })
            .collect();
        Ok(ThreadPageResult {
            threads,
            next_cursor,
            runtime,
        })
    }
    fn start_thread(&self, request: StartThread) -> Result<Thread, CodexError> {
        Ok(Thread {
            id: ThreadId("fixture-thread".into()),
            title: Some("Fixture thread".into()),
            cwd: Some(request.cwd),
            last_used_at: None,
            turns: Vec::new(),
            model: request.model,
            reasoning_effort: request.reasoning_effort,
        })
    }
    fn resume_thread(&self, id: ThreadId) -> Result<Thread, CodexError> {
        let thread = self
            .scenario
            .threads
            .iter()
            .find(|thread| thread.id == id)
            .cloned()
            .ok_or_else(|| CodexError::Protocol("fixture thread not found".into()))?;
        self.resumed_threads.lock().unwrap().push(id);
        Ok(thread)
    }
    fn start_turn(&self, request: StartTurn) -> Result<Turn, CodexError> {
        self.started_turns.lock().unwrap().push(request.clone());
        Ok(Turn {
            id: TurnId("fixture-turn".into()),
            thread_id: request.thread_id,
            status: "inProgress".into(),
        })
    }
    fn shell_command(&self, thread: ThreadId, command: String) -> Result<(), CodexError> {
        if command.trim().is_empty() {
            return Err(CodexError::Protocol("shell command is blank".into()));
        }
        if self
            .scenario
            .threads
            .iter()
            .any(|candidate| candidate.id == thread)
        {
            Ok(())
        } else {
            Err(CodexError::Protocol("thread is unavailable".into()))
        }
    }
    fn interrupt_turn(&self, _: ThreadId, _: TurnId) -> Result<(), CodexError> {
        Ok(())
    }
    fn respond(&self, request: ServerRequestId, _: InteractionResponse) -> Result<(), CodexError> {
        self.pending
            .lock()
            .unwrap()
            .remove(&request.0)
            .map(|_| ())
            .ok_or_else(|| CodexError::InvalidInteraction("fixture request is not pending".into()))
    }
    fn subscribe(&self) -> mpsc::Receiver<CodexEvent> {
        let (tx, rx) = mpsc::channel();
        for event in &self.scenario.events {
            let _ = tx.send(event.clone());
        }
        rx
    }
}

fn page_bounds(
    len: usize,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> Result<(usize, usize, Option<String>), CodexError> {
    let start = cursor
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| CodexError::Protocol("fixture cursor is invalid".into()))?
        .min(len);
    let limit = usize::try_from(limit.unwrap_or(100).max(1)).unwrap_or(usize::MAX);
    let end = start.saturating_add(limit).min(len);
    let next_cursor = (end < len).then(|| end.to_string());
    Ok((start, end, next_cursor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_is_deterministic_and_double_response_fails() {
        let backend = ReplayBackend::from_json(
            r#"{"name":"approval","events":[{"sequence":1,"kind":{"type":"approval_requested","request_id":"r1","approval_type":"command","summary":null}}]}"#,
        )
        .unwrap();
        let first: Vec<_> = backend.subscribe().into_iter().collect();
        let second: Vec<_> = backend.subscribe().into_iter().collect();
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );
        assert!(matches!(
            first.as_slice(),
            [CodexEvent {
                sequence: 1,
                kind: EventKind::ApprovalRequested { request_id, .. },
            }] if request_id.0 == "r1"
        ));
        backend
            .respond(
                ServerRequestId("r1".into()),
                InteractionResponse::CommandApproval {
                    decision: crate::CommandDecision::Decline,
                },
            )
            .unwrap();
        assert!(
            backend
                .respond(
                    ServerRequestId("r1".into()),
                    InteractionResponse::CommandApproval {
                        decision: crate::CommandDecision::Decline
                    }
                )
                .is_err()
        );
    }

    #[test]
    fn replay_projects_and_threads_are_cursor_paginated() {
        let backend = ReplayBackend::from_json(
            r#"{
                "name":"pagination",
                "projects":[
                    {"id":"one","name":"One","roots":["/one"]},
                    {"id":"two","name":"Two","roots":["/two"]}
                ],
                "threads":[
                    {"id":"one","title":null,"cwd":"/one"},
                    {"id":"two","title":null,"cwd":"/two"}
                ]
            }"#,
        )
        .unwrap();
        let first_projects = backend
            .list_projects(ProjectPage {
                cursor: None,
                limit: Some(1),
            })
            .unwrap();
        assert_eq!(first_projects.projects[0].id, "one");
        assert_eq!(first_projects.next_cursor.as_deref(), Some("1"));
        let second_projects = backend
            .list_projects(ProjectPage {
                cursor: first_projects.next_cursor,
                limit: Some(1),
            })
            .unwrap();
        assert_eq!(second_projects.projects[0].id, "two");
        assert!(second_projects.next_cursor.is_none());

        let first_threads = backend
            .list_threads(ThreadPage {
                cursor: None,
                limit: Some(1),
            })
            .unwrap();
        assert_eq!(first_threads.threads[0].id, ThreadId("one".into()));
        assert_eq!(first_threads.next_cursor.as_deref(), Some("1"));
        let second_threads = backend
            .list_threads(ThreadPage {
                cursor: first_threads.next_cursor,
                limit: Some(1),
            })
            .unwrap();
        assert_eq!(second_threads.threads[0].id, ThreadId("two".into()));
        assert!(second_threads.next_cursor.is_none());
    }
}
