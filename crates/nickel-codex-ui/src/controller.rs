use std::{
    fs,
    io::ErrorKind,
    path::Path,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

pub fn create_managed_workspace() -> Result<PathBuf, String> {
    let documents = dirs::document_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Documents")))
        .ok_or_else(|| "the user Documents directory is unavailable".to_owned())?;
    create_managed_workspace_at(&documents, &jiff::Zoned::now())
}

fn create_managed_workspace_at(documents: &Path, now: &jiff::Zoned) -> Result<PathBuf, String> {
    let date = now.strftime("%Y-%m-%d").to_string();
    let time = now.strftime("%H%M%S").to_string();
    let parent = documents.join("codex").join(date);
    fs::create_dir_all(&parent).map_err(|error| error.to_string())?;
    for suffix in 0..1000 {
        let name = if suffix == 0 {
            format!("session-{time}")
        } else {
            format!("session-{time}-{suffix}")
        };
        let candidate = parent.join(name);
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("could not allocate a unique managed workspace name".into())
}

use nickel_codex::{
    AccountState, BackendChoice, CodexBackend, CodexClient, CodexEvent, CommandDecision,
    FileChangeDecision, InteractionResponse, Model, ReplayBackend, Selector, ServerRequestId,
    StartThread, StartTurn, Thread, ThreadId, ThreadPage, UserInputAnswer,
};

#[derive(Clone)]
pub enum BackendMode {
    Live {
        choice: BackendChoice,
        cwd: PathBuf,
    },
    Replay {
        backend: ReplayBackend,
        cwd: PathBuf,
    },
}

#[derive(Clone, Debug)]
pub enum ControllerCommand {
    Refresh,
    NewChat,
    SelectThread(ThreadId),
    Send(String),
    Interrupt,
    CommandApproval {
        request_id: ServerRequestId,
        decision: CommandDecision,
    },
    FileApproval {
        request_id: ServerRequestId,
        decision: FileChangeDecision,
    },
    UserInput {
        request_id: ServerRequestId,
        answers: Vec<UserInputAnswer>,
    },
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum ControllerEvent {
    Ready {
        provenance: String,
        account: AccountState,
        models: Vec<Model>,
        threads: Vec<Thread>,
    },
    ThreadCreated(Thread),
    ThreadSelected(Thread),
    Protocol(CodexEvent),
    Incompatible(String),
    OperationFailed(String),
    Failure(String),
}

pub struct ChatController {
    generation: u64,
    commands: Sender<ControllerCommand>,
    events: Receiver<(u64, ControllerEvent)>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ChatController {
    pub fn spawn(mode: BackendMode) -> Self {
        Self::spawn_generation(mode, 1)
    }

    pub fn spawn_generation(mode: BackendMode, generation: u64) -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let worker = thread::spawn(move || run_worker(generation, mode, command_rx, event_tx));
        Self {
            generation,
            commands: command_tx,
            events: event_rx,
            worker: Some(worker),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn send(&self, command: ControllerCommand) -> bool {
        self.commands.send(command).is_ok()
    }

    pub fn try_recv(&self) -> Option<(u64, ControllerEvent)> {
        self.events.try_recv().ok()
    }
}

impl Drop for ChatController {
    fn drop(&mut self) {
        let _ = self.commands.send(ControllerCommand::Shutdown);
        if self
            .worker
            .as_ref()
            .is_some_and(thread::JoinHandle::is_finished)
            && let Some(worker) = self.worker.take()
        {
            let _ = worker.join();
        }
    }
}

fn run_worker(
    generation: u64,
    mode: BackendMode,
    commands: Receiver<ControllerCommand>,
    events: Sender<(u64, ControllerEvent)>,
) {
    let send = |event| events.send((generation, event)).is_ok();
    let (backend, cwd, provenance): (Box<dyn CodexBackend>, PathBuf, String) = match mode {
        BackendMode::Replay { backend, cwd } => (Box::new(backend), cwd, "Replay fixture".into()),
        BackendMode::Live { choice, cwd } => {
            let selection = Selector::platform_default().select(choice);
            let Some(candidate) = selection.selected else {
                let reason = selection
                    .probes
                    .last()
                    .map(|probe| probe.reason.clone())
                    .unwrap_or_else(|| "no Codex candidate was found".into());
                let event = if selection.probes.is_empty() {
                    ControllerEvent::Failure(reason)
                } else {
                    ControllerEvent::Incompatible(reason)
                };
                let _ = send(event);
                return;
            };
            let version = selection
                .probes
                .iter()
                .find(|probe| probe.candidate == candidate)
                .and_then(|probe| probe.version.clone())
                .unwrap_or_else(|| "compatible version".into());
            let provenance = codex_attribution(&version);
            match CodexClient::spawn(&candidate.path, &cwd) {
                Ok(client) => (Box::new(client), cwd, provenance),
                Err(error) => {
                    let _ = send(ControllerEvent::Failure(error.to_string()));
                    return;
                }
            }
        }
    };
    let protocol_events = backend.subscribe();
    if !send(snapshot(&*backend, provenance.clone())) {
        return;
    }
    let mut selected_thread = None;
    let mut active_turn = None;
    let mut new_thread_cwd = cwd;

    loop {
        while let Ok(event) = protocol_events.try_recv() {
            match &event.kind {
                nickel_codex::EventKind::TurnStarted { turn_id, .. } => {
                    active_turn = Some(turn_id.clone())
                }
                nickel_codex::EventKind::TurnCompleted { .. } => active_turn = None,
                _ => {}
            }
            if !send(ControllerEvent::Protocol(event)) {
                return;
            }
        }
        let command = match commands.recv_timeout(Duration::from_millis(16)) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let result = match command {
            ControllerCommand::Refresh => send(snapshot(&*backend, provenance.clone()))
                .then_some(())
                .ok_or_else(|| "UI disconnected".to_owned()),
            ControllerCommand::NewChat => create_managed_workspace().map(|workspace| {
                new_thread_cwd = workspace;
                selected_thread = None;
                active_turn = None;
            }),
            ControllerCommand::SelectThread(id) => backend
                .resume_thread(id)
                .map(|thread| {
                    selected_thread = Some(thread.id.clone());
                    let _ = send(ControllerEvent::ThreadSelected(thread));
                })
                .map_err(|error| error.to_string()),
            ControllerCommand::Send(text) => {
                let thread = match selected_thread.clone() {
                    Some(id) => Ok(id),
                    None => backend
                        .start_thread(StartThread {
                            cwd: new_thread_cwd.clone(),
                            model: None,
                        })
                        .map(|thread| {
                            selected_thread = Some(thread.id.clone());
                            let _ = send(ControllerEvent::ThreadCreated(thread.clone()));
                            thread.id
                        }),
                };
                thread
                    .and_then(|thread_id| backend.start_turn(StartTurn { thread_id, text }))
                    .map(|turn| active_turn = Some(turn.id))
                    .map_err(|error| error.to_string())
            }
            ControllerCommand::Interrupt => match (selected_thread.clone(), active_turn.clone()) {
                (Some(thread), Some(turn)) => backend
                    .interrupt_turn(thread, turn)
                    .map_err(|error| error.to_string()),
                _ => Err("there is no active turn to interrupt".into()),
            },
            ControllerCommand::CommandApproval {
                request_id,
                decision,
            } => backend
                .respond(
                    request_id,
                    InteractionResponse::CommandApproval { decision },
                )
                .map_err(|error| error.to_string()),
            ControllerCommand::FileApproval {
                request_id,
                decision,
            } => backend
                .respond(
                    request_id,
                    InteractionResponse::FileChangeApproval { decision },
                )
                .map_err(|error| error.to_string()),
            ControllerCommand::UserInput {
                request_id,
                answers,
            } => backend
                .respond(request_id, InteractionResponse::UserInput { answers })
                .map_err(|error| error.to_string()),
            ControllerCommand::Shutdown => return,
        };
        if let Err(error) = result
            && !send(ControllerEvent::OperationFailed(error))
        {
            return;
        }
    }
}

pub(crate) fn codex_attribution(version: &str) -> String {
    let version = version
        .trim()
        .strip_prefix("codex-cli ")
        .unwrap_or(version.trim());
    format!("powered by OpenAI Codex CLI v{version}.")
}

fn snapshot(backend: &dyn CodexBackend, provenance: String) -> ControllerEvent {
    let result = (|| {
        Ok::<_, nickel_codex::CodexError>((
            backend.account()?,
            backend.models()?,
            backend.list_threads(ThreadPage {
                cursor: None,
                limit: Some(100),
            })?,
        ))
    })();
    match result {
        Ok((account, models, page)) => ControllerEvent::Ready {
            provenance,
            account,
            models,
            threads: page.threads,
        },
        Err(error) => ControllerEvent::Failure(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::create_managed_workspace_at;

    #[test]
    fn managed_workspaces_are_dated_unique_children_of_documents() {
        let documents = tempfile::tempdir().unwrap();
        let now: jiff::Zoned = "2026-08-23T14:05:06-07:00[America/Los_Angeles]"
            .parse()
            .unwrap();

        let first = create_managed_workspace_at(documents.path(), &now).unwrap();
        let second = create_managed_workspace_at(documents.path(), &now).unwrap();

        assert_eq!(
            first.strip_prefix(documents.path()).unwrap(),
            std::path::Path::new("codex/2026-08-23/session-140506")
        );
        assert_eq!(
            second.strip_prefix(documents.path()).unwrap(),
            std::path::Path::new("codex/2026-08-23/session-140506-1")
        );
        assert!(first.is_dir());
        assert!(second.is_dir());
        assert_ne!(first, documents.path());
    }
}
