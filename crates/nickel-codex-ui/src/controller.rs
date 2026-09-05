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
    AccountState, ApprovalPolicy, BackendChoice, CodexBackend, CodexClient, CodexEvent,
    CommandDecision, FileChangeDecision, ImportProject, InteractionResponse, Model, Project,
    ProjectPage, RemoteHost, ReplayBackend, Selector, ServerRequestId, StartThread, StartTurn,
    Thread, ThreadId, ThreadPage, ThreadPageResult, UserInputAnswer,
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
    LoadThreads,
    NewChat,
    NewChatIn(PathBuf, Option<String>),
    SelectThread(ThreadId),
    Send {
        text: String,
        images: Vec<nickel_codex::TurnImage>,
        model: Option<String>,
        reasoning_effort: Option<String>,
        approval_policy: ApprovalPolicy,
    },
    Shell(String),
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
        thread_error: Option<String>,
    },
    ThreadCreated(Thread),
    ThreadSelected(Thread),
    TurnAccepted,
    ModelRejected {
        model: String,
        message: String,
    },
    ApprovalPolicyAccepted(ApprovalPolicy),
    Protocol(CodexEvent),
    Incompatible(String),
    Unavailable(String),
    OperationFailed(String),
    Failure(String),
}

pub struct ChatController {
    generation: u64,
    commands: Sender<ControllerCommand>,
    events: Receiver<(u64, ControllerEvent)>,
    worker: Option<thread::JoinHandle<()>>,
}

#[derive(Clone, Copy)]
enum SnapshotScope {
    Full,
    ProjectsOnly,
    NewProjectChat,
}

impl ChatController {
    #[cfg(feature = "workbench-fixtures")]
    pub(crate) fn fixture_idle(generation: u64) -> Self {
        let (commands, command_receiver) = mpsc::channel();
        let (_event_sender, events) = mpsc::channel();
        drop(command_receiver);
        Self {
            generation,
            commands,
            events,
            worker: None,
        }
    }

    pub fn spawn(mode: BackendMode) -> Self {
        Self::spawn_generation_with_scope(mode, 1, SnapshotScope::Full)
    }

    pub fn spawn_generation(mode: BackendMode, generation: u64) -> Self {
        Self::spawn_generation_with_scope(mode, generation, SnapshotScope::Full)
    }

    pub fn spawn_project_menu(mode: BackendMode, generation: u64) -> Self {
        Self::spawn_generation_with_scope(mode, generation, SnapshotScope::ProjectsOnly)
    }

    pub fn spawn_project_chat(mode: BackendMode, generation: u64) -> Self {
        Self::spawn_generation_with_scope(mode, generation, SnapshotScope::NewProjectChat)
    }

    fn spawn_generation_with_scope(
        mode: BackendMode,
        generation: u64,
        scope: SnapshotScope,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let worker =
            thread::spawn(move || run_worker(generation, mode, scope, command_rx, event_tx));
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
    scope: SnapshotScope,
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
                    let event = selection_failure_event(selection.probes.is_empty(), reason);
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
    let current_snapshot = || match scope {
        SnapshotScope::Full => snapshot(&*backend, provenance.clone()),
        SnapshotScope::ProjectsOnly => project_snapshot(&*backend, provenance.clone()),
        SnapshotScope::NewProjectChat => new_project_chat_snapshot(&*backend, provenance.clone()),
    };
    if !send(current_snapshot()) {
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
            ControllerCommand::Refresh => send(current_snapshot())
                .then_some(())
                .ok_or_else(|| "UI disconnected".to_owned()),
            ControllerCommand::LoadThreads => send(snapshot(&*backend, provenance.clone()))
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
            ControllerCommand::SelectThread(id) => {
                match verify_thread_is_resumable(&*backend, &id)
                    .and_then(|()| backend.resume_thread(id))
                {
                    Ok(thread) => {
                        selected_thread = Some(thread.id.clone());
                        let _ = send(ControllerEvent::ThreadSelected(thread));
                    }
                    Err(error) => {
                        let _ = send(ControllerEvent::OperationFailed(error.to_string()));
                        let _ = send(snapshot(&*backend, provenance.clone()));
                    }
                }
                Ok(())
            }
            ControllerCommand::Send {
                text,
                images,
                model,
                reasoning_effort,
                approval_policy,
            } => {
                let requested_model = model.clone();
                let thread = match selected_thread.clone() {
                    Some(id) => Ok(id),
                    None => backend
                        .start_thread(StartThread {
                            cwd: new_thread_cwd.clone(),
                            model: model.clone(),
                            project_id: new_thread_project_id.clone(),
                            reasoning_effort: reasoning_effort.clone(),
                            approval_policy,
                        })
                        .map(|thread| {
                            selected_thread = Some(thread.id.clone());
                            let _ = send(ControllerEvent::ThreadCreated(thread.clone()));
                            thread.id
                        }),
                };
                let result = thread
                    .and_then(|thread_id| {
                        backend.start_turn(StartTurn {
                            thread_id,
                            text,
                            images,
                            model,
                            reasoning_effort,
                            approval_policy,
                        })
                    })
                    .map(|turn| {
                        active_turn = Some(turn.id);
                        let _ = send(ControllerEvent::TurnAccepted);
                        let _ = send(ControllerEvent::ApprovalPolicyAccepted(approval_policy));
                    });
                match result {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        let message = error.to_string();
                        if let Some(model) = requested_model
                            && error_indicates_model_rejection(&message)
                        {
                            let _ = send(ControllerEvent::ModelRejected { model, message });
                            Ok(())
                        } else {
                            Err(message)
                        }
                    }
                }
            }
            ControllerCommand::Shell(command) => {
                let thread = match selected_thread.clone() {
                    Some(id) => Ok(id),
                    None => backend
                        .start_thread(StartThread {
                            cwd: new_thread_cwd.clone(),
                            model: None,
                            project_id: new_thread_project_id.clone(),
                            reasoning_effort: None,
                            approval_policy: ApprovalPolicy::default(),
                        })
                        .map(|thread| {
                            selected_thread = Some(thread.id.clone());
                            let _ = send(ControllerEvent::ThreadCreated(thread.clone()));
                            thread.id
                        }),
                };
                thread
                    .and_then(|thread_id| backend.shell_command(thread_id, command))
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

fn error_indicates_model_rejection(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("model")
        && [
            "unavailable",
            "unsupported",
            "not supported",
            "not found",
            "unknown",
            "invalid",
            "rejected",
        ]
        .into_iter()
        .any(|reason| message.contains(reason))
}

fn selection_failure_event(no_candidates: bool, reason: String) -> ControllerEvent {
    if no_candidates {
        ControllerEvent::Unavailable(reason)
    } else {
        ControllerEvent::Incompatible(reason)
    }
}

fn verify_thread_is_resumable(
    backend: &dyn CodexBackend,
    id: &ThreadId,
) -> Result<(), nickel_codex::CodexError> {
    let page = list_threads(backend)?;
    let status = page.runtime.get(id).map(|runtime| &runtime.status);
    if matches!(
        status,
        Some(nickel_codex::ThreadRuntimeStatus::Idle)
            | Some(nickel_codex::ThreadRuntimeStatus::NotLoaded)
    ) {
        Ok(())
    } else {
        Err(nickel_codex::CodexError::Unavailable(format!(
            "thread {} is not available for a writable resume",
            id.0
        )))
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
        let mut projects = list_projects(backend)?;
        let (page, thread_error) = match list_threads(backend) {
            Ok(mut page) => {
                if import_missing_thread_projects(backend, &projects, &page.threads, &page.runtime)?
                {
                    projects = list_projects(backend)?;
                    page = list_threads(backend)?;
                }
                (page, None)
            }
            Err(error) => (
                ThreadPageResult {
                    threads: Vec::new(),
                    next_cursor: None,
                    runtime: HashMap::new(),
                },
                Some(error.to_string()),
            ),
        };
        Ok::<_, nickel_codex::CodexError>((
            backend.account()?,
            backend.models()?,
            projects,
            page,
            thread_error,
        ))
    })();
    match result {
        Ok((account, models, projects, page, thread_error)) => ControllerEvent::Ready {
            provenance,
            account,
            models,
            projects,
            threads: page.threads,
            runtime: page.runtime,
            thread_error,
        },
        Err(error) => ControllerEvent::Failure(error.to_string()),
    }
}

fn project_snapshot(backend: &dyn CodexBackend, provenance: String) -> ControllerEvent {
    match (backend.account(), list_projects(backend)) {
        (Ok(account), Ok(mut projects)) => {
            match backend.list_threads(ThreadPage {
                cursor: None,
                limit: Some(100),
            }) {
                Ok(page) => {
                    sort_projects_by_recent_threads(&mut projects, &page.threads, &page.runtime);
                    ControllerEvent::Ready {
                        provenance,
                        account,
                        models: Vec::new(),
                        projects,
                        threads: page.threads,
                        runtime: page.runtime,
                        thread_error: None,
                    }
                }
                Err(error) => ControllerEvent::Ready {
                    provenance,
                    account,
                    models: Vec::new(),
                    projects,
                    threads: Vec::new(),
                    runtime: HashMap::new(),
                    thread_error: Some(error.to_string()),
                },
            }
        }
        (Err(error), _) | (_, Err(error)) => ControllerEvent::Failure(error.to_string()),
    }
}

fn sort_projects_by_recent_threads(
    projects: &mut [Project],
    threads: &[Thread],
    runtime: &HashMap<ThreadId, nickel_codex::ThreadRuntime>,
) {
    let mut recency = HashMap::<String, i64>::new();
    for thread in threads {
        let Some(last_used_at) = thread.last_used_at else {
            continue;
        };
        let project = runtime
            .get(&thread.id)
            .and_then(|runtime| runtime.project_id.as_deref())
            .and_then(|project_id| projects.iter().find(|project| project.id == project_id))
            .or_else(|| {
                thread.cwd.as_ref().and_then(|cwd| {
                    projects.iter().find(|project| {
                        project
                            .roots
                            .iter()
                            .any(|root| cwd == root || cwd.starts_with(root))
                    })
                })
            });
        if let Some(project) = project {
            recency
                .entry(project.id.clone())
                .and_modify(|current| *current = (*current).max(last_used_at))
                .or_insert(last_used_at);
        }
    }
    projects.sort_by(|left, right| recency.get(&right.id).cmp(&recency.get(&left.id)));
}

fn new_project_chat_snapshot(backend: &dyn CodexBackend, provenance: String) -> ControllerEvent {
    match (backend.account(), backend.models(), list_projects(backend)) {
        (Ok(account), Ok(models), Ok(projects)) => ControllerEvent::Ready {
            provenance,
            account,
            models,
            projects,
            threads: Vec::new(),
            runtime: HashMap::new(),
            thread_error: None,
        },
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
            ControllerEvent::Failure(error.to_string())
        }
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
    use std::{collections::HashMap, path::PathBuf};

    use nickel_codex::{Project, ReplayBackend, Thread, ThreadId, ThreadRuntime};

    use super::{
        ControllerEvent, create_managed_workspace_at, error_indicates_model_rejection,
        next_new_thread_cwd, project_snapshot, selection_failure_event, snapshot,
        sort_projects_by_recent_threads, verify_thread_is_resumable,
    };

    fn project(id: &str, root: &str) -> Project {
        Project {
            id: id.into(),
            name: id.into(),
            roots: vec![PathBuf::from(root)],
        }
    }

    fn thread(id: &str, cwd: &str, last_used_at: i64) -> Thread {
        Thread {
            id: ThreadId(id.into()),
            title: None,
            cwd: Some(PathBuf::from(cwd)),
            last_used_at: Some(last_used_at),
            turns: Vec::new(),
            model: None,
            reasoning_effort: None,
        }
    }

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

    #[test]
    fn rejected_thread_list_keeps_canonical_projects_ready() {
        let backend = ReplayBackend::from_json(
            r#"{
                "name":"thread-list-error",
                "projects":[{"id":"nickel","name":"Nickel","roots":["/projects/nickel"]}],
                "thread_error":"duplicate thread id"
            }"#,
        )
        .unwrap();

        let ControllerEvent::Ready {
            projects,
            threads,
            runtime,
            thread_error,
            ..
        } = snapshot(&backend, "Replay fixture".into())
        else {
            panic!("thread failure must not disconnect project discovery");
        };

        assert_eq!(
            projects,
            vec![Project {
                id: "nickel".into(),
                name: "Nickel".into(),
                roots: vec!["/projects/nickel".into()],
            }]
        );
        assert!(threads.is_empty());
        assert!(runtime.is_empty());
        assert_eq!(
            thread_error.as_deref(),
            Some("Codex protocol error: duplicate thread id")
        );
    }

    #[test]
    fn project_menu_sorts_explicit_and_historical_threads_by_recency() {
        let mut projects = vec![
            project("older", "/projects/older"),
            project("newer", "/projects/newer"),
            project("middle", "/projects/middle"),
        ];
        let threads = vec![
            thread("explicit", "/elsewhere", 300),
            thread("cwd", "/projects/middle/subdirectory", 200),
            thread("old", "/projects/older", 100),
        ];
        let runtime = HashMap::from([(
            ThreadId("explicit".into()),
            ThreadRuntime {
                project_id: Some("newer".into()),
                ..ThreadRuntime::default()
            },
        )]);

        sort_projects_by_recent_threads(&mut projects, &threads, &runtime);

        assert_eq!(
            projects
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>(),
            ["newer", "middle", "older"]
        );
    }

    #[test]
    fn projects_without_recency_keep_their_canonical_order() {
        let mut projects = vec![
            project("first", "/projects/first"),
            project("second", "/projects/second"),
            project("third", "/projects/third"),
        ];

        sort_projects_by_recent_threads(&mut projects, &[], &HashMap::new());

        assert_eq!(
            projects
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "third"]
        );
    }

    #[test]
    fn project_menu_ignores_thread_failure() {
        let backend = ReplayBackend::from_json(
            r#"{
                "name":"project-menu-thread-error",
                "account":{"authenticated":true,"account_type":"chatgpt","email":null},
                "projects":[{"id":"nickel","name":"Nickel","roots":["/projects/nickel"]}],
                "thread_error":"duplicate thread id"
            }"#,
        )
        .unwrap();

        let ControllerEvent::Ready {
            account, projects, ..
        } = project_snapshot(&backend, "Replay fixture".into())
        else {
            panic!("thread failure must not disconnect the project menu");
        };
        assert!(account.authenticated);
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, "nickel");
        assert_eq!(projects[0].name, "Nickel");
        assert_eq!(projects[0].roots, [PathBuf::from("/projects/nickel")]);
    }

    #[test]
    fn writable_resume_rechecks_and_accepts_only_idle_or_not_loaded() {
        for (status, expected) in [
            ("Idle", true),
            ("NotLoaded", true),
            ("Active", false),
            ("SystemError", false),
            ("Unknown", false),
        ] {
            let backend = ReplayBackend::from_json(&format!(
                r#"{{
                    "name":"{status}",
                    "threads":[{{"id":"thread","title":null,"cwd":"/work"}}],
                    "thread_runtime":{{"thread":{{
                        "project_id":null,
                        "status":"{status}",
                        "active_flags":[],
                        "can_accept_direct_input":null
                    }}}}
                }}"#
            ))
            .unwrap();
            let result = verify_thread_is_resumable(&backend, &ThreadId("thread".into()));
            if expected {
                assert!(result.is_ok(), "status {status} should be resumable");
            } else {
                assert_eq!(
                    result.unwrap_err().to_string(),
                    "Codex unavailable: thread thread is not available for a writable resume",
                    "status {status} must explain why resume is refused"
                );
            }
        }
    }

    #[test]
    fn writable_resume_rejects_missing_availability() {
        let backend = ReplayBackend::from_json(
            r#"{"name":"missing","threads":[{"id":"thread","title":null,"cwd":"/work"}]}"#,
        )
        .unwrap();
        assert_eq!(
            verify_thread_is_resumable(&backend, &ThreadId("thread".into()))
                .unwrap_err()
                .to_string(),
            "Codex unavailable: thread thread is not available for a writable resume"
        );
    }

    #[test]
    fn candidate_absence_is_not_reported_as_a_connection_failure() {
        assert!(matches!(
            selection_failure_event(true, "no Codex candidate was found".into()),
            ControllerEvent::Unavailable(_)
        ));
        assert!(matches!(
            selection_failure_event(false, "schema mismatch".into()),
            ControllerEvent::Incompatible(_)
        ));
    }

    #[test]
    fn only_explicit_model_rejections_trigger_model_fallback() {
        for message in [
            "unknown model nickel-2",
            "Model is unavailable",
            "the selected model is not supported",
            "invalid MODEL identifier",
        ] {
            assert!(error_indicates_model_rejection(message), "{message}");
        }
        for message in [
            "connection unavailable",
            "turn/start timed out",
            "model response timed out",
            "invalid approval policy",
        ] {
            assert!(!error_indicates_model_rejection(message), "{message}");
        }
    }
}
