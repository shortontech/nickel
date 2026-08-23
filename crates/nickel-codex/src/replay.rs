use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
};

use serde::{Deserialize, Serialize};

use crate::{
    AccountState, CodexBackend, CodexError, CodexEvent, EventKind, InteractionResponse, Model,
    ServerRequestId, StartThread, StartTurn, Thread, ThreadId, ThreadPage, ThreadPageResult, Turn,
    TurnId,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayScenario {
    pub name: String,
    #[serde(default)]
    pub account: AccountState,
    #[serde(default)]
    pub models: Vec<Model>,
    #[serde(default)]
    pub threads: Vec<Thread>,
    #[serde(default)]
    pub events: Vec<CodexEvent>,
}

#[derive(Clone)]
pub struct ReplayBackend {
    scenario: Arc<ReplayScenario>,
    pending: Arc<Mutex<HashMap<String, String>>>,
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
        })
    }
}

impl CodexBackend for ReplayBackend {
    fn account(&self) -> Result<AccountState, CodexError> {
        Ok(self.scenario.account.clone())
    }
    fn models(&self) -> Result<Vec<Model>, CodexError> {
        Ok(self.scenario.models.clone())
    }
    fn list_threads(&self, _: ThreadPage) -> Result<ThreadPageResult, CodexError> {
        Ok(ThreadPageResult {
            threads: self.scenario.threads.clone(),
            next_cursor: None,
        })
    }
    fn start_thread(&self, request: StartThread) -> Result<Thread, CodexError> {
        Ok(Thread {
            id: ThreadId("fixture-thread".into()),
            title: Some("Fixture thread".into()),
            cwd: Some(request.cwd),
            turns: Vec::new(),
        })
    }
    fn resume_thread(&self, id: ThreadId) -> Result<Thread, CodexError> {
        self.scenario
            .threads
            .iter()
            .find(|thread| thread.id == id)
            .cloned()
            .ok_or_else(|| CodexError::Protocol("fixture thread not found".into()))
    }
    fn start_turn(&self, request: StartTurn) -> Result<Turn, CodexError> {
        Ok(Turn {
            id: TurnId("fixture-turn".into()),
            thread_id: request.thread_id,
            status: "inProgress".into(),
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_is_deterministic_and_double_response_fails() {
        let backend = ReplayBackend::from_json(r#"{"name":"approval","events":[{"sequence":1,"kind":{"type":"approval_requested","request_id":"r1","approval_type":"command","summary":null}}]}"#).unwrap();
        let first: Vec<_> = backend.subscribe().into_iter().collect();
        let second: Vec<_> = backend.subscribe().into_iter().collect();
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );
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
}
