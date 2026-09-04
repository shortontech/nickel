use std::{
    env,
    io::{BufRead, Read},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

use nickel_codex::{
    BackendChoice, CodexBackend, CodexClient, EventKind, InteractionResponse, ProjectPage,
    ReplayBackend, Selector, StartThread, StartTurn, ThreadId, ThreadPage, TurnId,
};
use serde::Deserialize;
use serde_json::{Value, json};

static INTERRUPTS: AtomicUsize = AtomicUsize::new(0);
static OUTPUT_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

#[derive(Deserialize)]
struct InteractionCommand {
    request_id: String,
    response: InteractionResponse,
}

fn emit(kind: &str, value: Value) {
    let sequence = OUTPUT_SEQUENCE.fetch_add(1, Ordering::SeqCst) + 1;
    println!(
        "{}",
        json!({"schema_version": 1, "sequence": sequence, "session": "local-1", "kind": kind, "value": value})
    );
}

fn failure(code: u8, kind: &str, message: impl std::fmt::Display) -> ExitCode {
    emit(kind, json!({"message": message.to_string()}));
    ExitCode::from(code)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        return failure(2, "invalid_usage", usage());
    };
    match command {
        "probe" => probe(&args[1..]),
        "replay" => replay(&args[1..]),
        "account" | "models" | "projects" | "threads" | "start-thread" | "resume-thread"
        | "turn" | "interrupt" => live(command, &args[1..]),
        _ => failure(2, "invalid_usage", usage()),
    }
}

fn usage() -> &'static str {
    "nickel-codex-test probe [--backend auto|installed|bundled|PATH]\n\
     nickel-codex-test replay SCENARIO.json\n\
     nickel-codex-test account|models|projects|threads [--backend ...]\n\
     nickel-codex-test start-thread --cwd PATH [--model MODEL] [--text TEXT]\n\
     nickel-codex-test resume-thread THREAD_ID\n\
     nickel-codex-test turn THREAD_ID --text TEXT\n\
     nickel-codex-test interrupt THREAD_ID TURN_ID"
}

fn choice(args: &[String]) -> Result<BackendChoice, String> {
    let Some(index) = args.iter().position(|arg| arg == "--backend") else {
        return Ok(BackendChoice::Automatic);
    };
    match args.get(index + 1).map(String::as_str) {
        Some("auto") => Ok(BackendChoice::Automatic),
        Some("installed") => Ok(BackendChoice::Installed),
        Some("bundled") => Ok(BackendChoice::Bundled),
        Some(path) => Ok(BackendChoice::Path(PathBuf::from(path))),
        None => Err("--backend requires a value".into()),
    }
}

fn probe(args: &[String]) -> ExitCode {
    let choice = match choice(args) {
        Ok(choice) => choice,
        Err(error) => return failure(2, "invalid_usage", error),
    };
    let selection = Selector::platform_default().select(choice);
    for probe in &selection.probes {
        emit("probe", serde_json::to_value(probe).unwrap());
    }
    match selection.selected {
        Some(candidate) => {
            emit("selected", serde_json::to_value(candidate).unwrap());
            ExitCode::SUCCESS
        }
        None => failure(3, "backend_unavailable", "no compatible Codex CLI"),
    }
}

fn replay(args: &[String]) -> ExitCode {
    let Some(path) = args.first() else {
        return failure(2, "invalid_usage", "replay requires a scenario path");
    };
    let input = match std::fs::read_to_string(path) {
        Ok(input) => input,
        Err(error) => return failure(2, "invalid_usage", error),
    };
    let backend = match ReplayBackend::from_json(&input) {
        Ok(backend) => backend,
        Err(error) => return failure(4, "protocol_failure", error),
    };
    for event in backend.subscribe() {
        emit("event", serde_json::to_value(event).unwrap());
    }
    ExitCode::SUCCESS
}

fn live(command: &str, args: &[String]) -> ExitCode {
    let selected = match Selector::platform_default()
        .select(match choice(args) {
            Ok(choice) => choice,
            Err(error) => return failure(2, "invalid_usage", error),
        })
        .selected
    {
        Some(selected) => selected,
        None => return failure(3, "backend_unavailable", "no compatible Codex CLI"),
    };
    emit("backend", serde_json::to_value(&selected).unwrap());
    let cwd = option(args, "--cwd")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let client = match CodexClient::spawn(&selected.path, &cwd) {
        Ok(client) => client,
        Err(error) => return failure(4, "protocol_failure", error),
    };
    let events = client.subscribe();
    if command == "turn" {
        let values = positional(args);
        let Some(thread_id) = values.first() else {
            client.shutdown();
            return failure(2, "invalid_usage", "turn requires THREAD_ID");
        };
        if let Err(error) = client.resume_thread(ThreadId((**thread_id).clone())) {
            client.shutdown();
            return failure(4, "operation_failed", error);
        }
    }
    let result = backend_operation(&client, command, args, cwd);
    match result {
        Ok(value) => {
            emit("result", value.clone());
            let streams_turn = command == "turn"
                || (command == "start-thread" && option(args, "--text").is_some());
            if streams_turn {
                let thread = if command == "turn" {
                    let values = positional(args);
                    ThreadId(values.first().map(|id| (**id).clone()).unwrap_or_default())
                } else {
                    ThreadId(
                        value["thread"]["id"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                    )
                };
                let turn = client.projection().active_turn;
                match stream_turn(&client, events, thread, turn) {
                    Ok(status) if status == "interrupted" => {
                        client.shutdown();
                        return failure(7, "turn_interrupted", "turn reached interrupted state");
                    }
                    Ok(_) => {}
                    Err(error) => {
                        client.shutdown();
                        return failure(4, "operation_failed", error);
                    }
                }
            } else {
                while let Ok(event) = events.recv_timeout(Duration::from_millis(50)) {
                    emit("event", serde_json::to_value(event).unwrap());
                }
            }
            client.shutdown();
            ExitCode::SUCCESS
        }
        Err(error) => {
            client.shutdown();
            failure(4, "operation_failed", error)
        }
    }
}

fn backend_operation(
    backend: &dyn CodexBackend,
    command: &str,
    args: &[String],
    cwd: PathBuf,
) -> Result<Value, nickel_codex::CodexError> {
    match command {
        "account" => backend.account().and_then(json_value),
        "models" => backend.models().and_then(json_value),
        "projects" => backend
            .list_projects(ProjectPage {
                cursor: option(args, "--cursor").map(Into::into),
                limit: Some(100),
            })
            .and_then(|page| json_value(page.projects)),
        "threads" => backend
            .list_threads(ThreadPage {
                cursor: option(args, "--cursor").map(Into::into),
                limit: Some(100),
            })
            .and_then(|page| json_value(page.threads)),
        "start-thread" => {
            let thread = backend.start_thread(StartThread {
                cwd,
                model: option(args, "--model").map(Into::into),
                project_id: None,
                reasoning_effort: None,
            })?;
            if option(args, "--text").is_some() {
                let text = turn_text(args)?;
                let turn = backend.start_turn(StartTurn {
                    thread_id: thread.id.clone(),
                    text,
                    images: Vec::new(),
                    model: None,
                    reasoning_effort: None,
                })?;
                json_value(json!({"thread": thread, "turn": turn}))
            } else {
                json_value(thread)
            }
        }
        "resume-thread" => match positional(args).first() {
            Some(id) => backend
                .resume_thread(ThreadId((**id).clone()))
                .and_then(json_value),
            None => Err(nickel_codex::CodexError::Protocol(
                "resume-thread requires THREAD_ID".into(),
            )),
        },
        "turn" => {
            let id = positional(args).first().cloned().ok_or_else(|| {
                nickel_codex::CodexError::Protocol("turn requires THREAD_ID".into())
            });
            let text = turn_text(args);
            id.and_then(|id| text.map(|text| (id, text)))
                .and_then(|(id, text)| {
                    backend.start_turn(StartTurn {
                        thread_id: ThreadId(id.clone()),
                        text,
                        images: Vec::new(),
                        model: None,
                        reasoning_effort: None,
                    })
                })
                .and_then(json_value)
        }
        "interrupt" => {
            let values = positional(args);
            match values.as_slice() {
                [thread, turn, ..] => backend
                    .interrupt_turn(ThreadId((**thread).clone()), TurnId((**turn).clone()))
                    .and_then(json_value),
                _ => Err(nickel_codex::CodexError::Protocol(
                    "interrupt requires THREAD_ID TURN_ID".into(),
                )),
            }
        }
        _ => Err(nickel_codex::CodexError::Protocol(format!(
            "unsupported diagnostic operation {command}"
        ))),
    }
}

fn turn_text(args: &[String]) -> Result<String, nickel_codex::CodexError> {
    match option(args, "--text") {
        Some("-") => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text)?;
            Ok(text)
        }
        Some(text) => Ok(text.into()),
        None => Err(nickel_codex::CodexError::Protocol(
            "turn requires --text".into(),
        )),
    }
}

fn stream_turn(
    client: &CodexClient,
    events: mpsc::Receiver<nickel_codex::CodexEvent>,
    thread: ThreadId,
    initial_turn: Option<TurnId>,
) -> Result<String, nickel_codex::CodexError> {
    INTERRUPTS.store(0, Ordering::SeqCst);
    let _ = ctrlc::set_handler(|| {
        INTERRUPTS.fetch_add(1, Ordering::SeqCst);
    });
    let (input_tx, input_rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            let _ = input_tx.send(line);
        }
    });
    let mut turn = initial_turn;
    let started = std::time::Instant::now();
    loop {
        match INTERRUPTS.load(Ordering::SeqCst) {
            0 => {}
            1 => {
                if let Some(turn_id) = turn.clone()
                    && INTERRUPTS
                        .compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                {
                    client.interrupt_turn(thread.clone(), turn_id)?;
                }
            }
            2 => {}
            _ => {
                return Err(nickel_codex::CodexError::Stopped(
                    "second interrupt requested shutdown".into(),
                ));
            }
        }
        let event = match events.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => event,
            Err(mpsc::RecvTimeoutError::Timeout)
                if started.elapsed() < Duration::from_secs(300) =>
            {
                continue;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(nickel_codex::CodexError::Timeout(
                    "turn event stream stalled".into(),
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(nickel_codex::CodexError::Stopped(
                    "turn event stream disconnected".into(),
                ));
            }
        };
        emit("event", serde_json::to_value(&event)?);
        match &event.kind {
            EventKind::TurnStarted { turn_id, .. } => turn = Some(turn_id.clone()),
            EventKind::TurnCompleted { status, .. } => return Ok(status.clone()),
            EventKind::ApprovalRequested {
                request_id,
                approval_type,
                ..
            } => {
                let response = read_interaction(&input_rx, &request_id.0).unwrap_or_else(|| {
                    if approval_type.contains("fileChange") {
                        InteractionResponse::FileChangeApproval {
                            decision: nickel_codex::FileChangeDecision::Decline,
                        }
                    } else {
                        InteractionResponse::CommandApproval {
                            decision: nickel_codex::CommandDecision::Decline,
                        }
                    }
                });
                client.respond(request_id.clone(), response)?;
            }
            EventKind::UserInputRequested { request_id, .. } => {
                let response = read_interaction(&input_rx, &request_id.0).unwrap_or(
                    InteractionResponse::UserInput {
                        answers: Vec::new(),
                    },
                );
                client.respond(request_id.clone(), response)?;
            }
            EventKind::Connection { state } if state == "failed" => {
                return Err(nickel_codex::CodexError::Stopped(
                    "connection failed during turn".into(),
                ));
            }
            _ => {}
        }
    }
}

fn read_interaction(
    receiver: &mpsc::Receiver<String>,
    expected: &str,
) -> Option<InteractionResponse> {
    while let Ok(line) = receiver.recv_timeout(Duration::from_secs(30)) {
        match serde_json::from_str::<InteractionCommand>(&line) {
            Ok(command) if command.request_id == expected => return Some(command.response),
            Ok(_) | Err(_) => continue,
        }
    }
    None
}

fn json_value<T: serde::Serialize>(value: T) -> Result<Value, nickel_codex::CodexError> {
    Ok(serde_json::to_value(value)?)
}
fn option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}
fn positional(args: &[String]) -> Vec<&String> {
    args.iter()
        .enumerate()
        .filter(|(index, arg)| {
            !arg.starts_with('-')
                && (*index == 0 || args[*index - 1] != "--backend")
                && (*index == 0 || args[*index - 1] != "--cwd")
                && (*index == 0 || args[*index - 1] != "--model")
                && (*index == 0 || args[*index - 1] != "--text")
                && (*index == 0 || args[*index - 1] != "--cursor")
        })
        .map(|(_, arg)| arg)
        .collect()
}

#[allow(dead_code)]
fn _is_absolute(path: &Path) -> bool {
    path.is_absolute()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> ReplayBackend {
        ReplayBackend::from_json(
            r#"{
                "name":"commands",
                "account":{"authenticated":false,"account_type":null,"email":null},
                "models":[{"id":"fixture-model","display_name":"Fixture Model"}],
                "threads":[{"id":"known-thread","title":"Known","cwd":"/fixture"}],
                "events":[]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn every_backend_subcommand_runs_against_the_replay_backend() {
        let backend = backend();
        let cwd = PathBuf::from("/fixture");
        for (command, args) in [
            ("account", vec![]),
            ("models", vec![]),
            ("projects", vec![]),
            ("threads", vec![]),
            ("start-thread", vec!["--cwd".into(), "/fixture".into()]),
            (
                "start-thread",
                vec![
                    "--cwd".into(),
                    "/fixture".into(),
                    "--text".into(),
                    "first turn".into(),
                ],
            ),
            ("resume-thread", vec!["known-thread".into()]),
            (
                "turn",
                vec!["known-thread".into(), "--text".into(), "hello".into()],
            ),
            (
                "interrupt",
                vec!["known-thread".into(), "fixture-turn".into()],
            ),
        ] {
            assert!(
                backend_operation(&backend, command, &args, cwd.clone()).is_ok(),
                "{command} failed"
            );
        }
    }
}
