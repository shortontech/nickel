use std::{
    collections::{BTreeMap, HashMap, HashSet},
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
    FileChangeDecision, ImportProject, InteractionResponse, Model, Project, ProjectPage,
    RemoteHost, ReplayBackend, Selector, ServerRequestId, StartThread, StartTurn, Thread, ThreadId,
    ThreadPage, ThreadPageResult, UserInputAnswer,
};

#[derive(Clone)]
pub enum BackendMode {
    Live {
        choice: BackendChoice,
        cwd: PathBuf,
    },
    Remote {
        host: RemoteHost,
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
    NewChatIn(PathBuf, Option<String>),
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
        projects: Vec<Project>,
        threads: Vec<Thread>,
        runtime: std::collections::HashMap<ThreadId, nickel_codex::ThreadRuntime>,
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
    let (backend, cwd, provenance, remote): (Box<dyn CodexBackend>, PathBuf, String, bool) =
        match mode {
            BackendMode::Replay { backend, cwd } => {
                (Box::new(backend), cwd, "Replay fixture".into(), false)
            }
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
                    Ok(client) => (Box::new(client), cwd, provenance, false),
                    Err(error) => {
                        let _ = send(ControllerEvent::Failure(error.to_string()));
                        return;
                    }
                }
            }
            BackendMode::Remote { host } => {
                let token = match host.token_env.as_deref() {
                    Some(variable) => match std::env::var(variable) {
                        Ok(token) if !token.is_empty() => Some(token),
                        Ok(_) => {
                            let _ = send(ControllerEvent::Failure(format!(
                                "remote token environment variable {variable} is empty"
                            )));
                            return;
                        }
                        Err(_) => {
                            let _ = send(ControllerEvent::Failure(format!(
                                "remote token environment variable {variable} is not set"
                            )));
                            return;
                        }
                    },
                    None => None,
                };
                match CodexClient::connect_remote(&host.endpoint, token.as_deref()) {
                    Ok(client) => (
                        Box::new(client),
                        PathBuf::from(&host.default_cwd),
                        format!(
                            "Remote: {} · {}\npowered by OpenAI Codex CLI.",
                            host.name, host.endpoint
                        ),
                        true,
                    ),
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
    let mut new_thread_cwd = cwd.clone();
    let mut new_thread_project_id = None;

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
            ControllerCommand::NewChat => next_new_thread_cwd(remote, &cwd).map(|workspace| {
                new_thread_cwd = workspace;
                selected_thread = None;
                active_turn = None;
            }),
            ControllerCommand::NewChatIn(workspace, project_id) => {
                new_thread_cwd = workspace;
                new_thread_project_id = project_id;
                selected_thread = None;
                active_turn = None;
                Ok(())
            }
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
                            project_id: new_thread_project_id.clone(),
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

fn next_new_thread_cwd(remote: bool, configured_cwd: &Path) -> Result<PathBuf, String> {
    if remote {
        Ok(configured_cwd.to_owned())
    } else {
        create_managed_workspace()
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
        let mut page = list_threads(backend)?;
        let mut projects = list_projects(backend)?;
        if import_missing_thread_projects(backend, &projects, &page.threads, &page.runtime)? {
            projects = list_projects(backend)?;
            page = list_threads(backend)?;
        }
        Ok::<_, nickel_codex::CodexError>((backend.account()?, backend.models()?, projects, page))
    })();
    match result {
        Ok((account, models, projects, page)) => ControllerEvent::Ready {
            provenance,
            account,
            models,
            projects,
            threads: page.threads,
            runtime: page.runtime,
        },
        Err(error) => ControllerEvent::Failure(error.to_string()),
    }
}

fn list_threads(backend: &dyn CodexBackend) -> Result<ThreadPageResult, nickel_codex::CodexError> {
    let mut result = ThreadPageResult {
        threads: Vec::new(),
        next_cursor: None,
        runtime: std::collections::HashMap::new(),
    };
    let mut cursor = None;
    loop {
        let page = backend.list_threads(ThreadPage {
            cursor: cursor.clone(),
            limit: Some(100),
        })?;
        result.threads.extend(page.threads);
        result.runtime.extend(page.runtime);
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            return Ok(result);
        }
        cursor = page.next_cursor;
    }
}

fn import_missing_thread_projects(
    backend: &dyn CodexBackend,
    projects: &[Project],
    threads: &[Thread],
    runtime: &HashMap<ThreadId, nickel_codex::ThreadRuntime>,
) -> Result<bool, nickel_codex::CodexError> {
    let registered_roots = projects
        .iter()
        .flat_map(|project| project.roots.iter().cloned())
        .collect::<HashSet<_>>();
    let mut directories: BTreeMap<PathBuf, Vec<ThreadId>> = BTreeMap::new();
    for thread in threads {
        if runtime
            .get(&thread.id)
            .and_then(|runtime| runtime.project_id.as_ref())
            .is_some()
        {
            continue;
        }
        let Some(cwd) = thread.cwd.as_ref().filter(|cwd| cwd.is_absolute()) else {
            continue;
        };
        if cwd.starts_with(std::env::temp_dir()) || registered_roots.contains(cwd) {
            continue;
        }
        directories
            .entry(cwd.clone())
            .or_default()
            .push(thread.id.clone());
    }
    let imported = !directories.is_empty();
    for (root, threads) in directories {
        let name = root
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("Project")
            .to_owned();
        let mut hash = 0xcbf29ce484222325_u64;
        for byte in root.to_string_lossy().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        backend.import_project(ImportProject {
            idempotency_key: format!("nickel-import-{hash:016x}"),
            name,
            roots: vec![root],
            threads,
        })?;
    }
    Ok(imported)
}

fn list_projects(backend: &dyn CodexBackend) -> Result<Vec<Project>, nickel_codex::CodexError> {
    let mut projects = Vec::new();
    let mut cursor = None;
    loop {
        let page = backend.list_projects(ProjectPage {
            cursor: cursor.clone(),
            limit: Some(100),
        })?;
        projects.extend(page.projects);
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            return Ok(projects);
        }
        cursor = page.next_cursor;
    }
}

#[cfg(test)]
mod tests {
    use super::{create_managed_workspace_at, next_new_thread_cwd};

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

    #[test]
    fn remote_new_chat_reuses_remote_path_without_local_creation() {
        let directory = tempfile::tempdir().unwrap();
        let nonexistent = directory.path().join("must-not-be-created");
        assert_eq!(
            next_new_thread_cwd(true, &nonexistent).unwrap(),
            nonexistent
        );
        assert!(!nonexistent.exists());
    }
}
