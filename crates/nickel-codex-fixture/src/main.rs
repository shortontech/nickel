use std::{
    env,
    io::{BufRead, Write},
    path::PathBuf,
    process::ExitCode,
};

use nickel_codex::{BackendChoice, Selector};
use serde_json::{Value, json};

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let executable_name = env::args_os()
        .next()
        .and_then(|path| {
            PathBuf::from(path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    if executable_name.contains("hang-probe")
        && args.first().and_then(|arg| arg.to_str()) == Some("--version")
    {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
    if args.first().and_then(|arg| arg.to_str()) == Some("--version") {
        println!("codex-cli nickel-fixture");
        return ExitCode::SUCCESS;
    }
    if args.first().and_then(|arg| arg.to_str()) == Some("app-server") {
        if args.get(1).and_then(|arg| arg.to_str()) == Some("generate-json-schema") {
            return generate_schema(&args);
        }
        return serve_app_server();
    }
    if args.first().and_then(|arg| arg.to_str()) == Some("compare-schema") {
        let Some(index) = args.iter().position(|arg| arg == "--codex") else {
            return ExitCode::from(2);
        };
        let Some(path) = args.get(index + 1).map(PathBuf::from) else {
            return ExitCode::from(2);
        };
        let selection = Selector::new(None).select(BackendChoice::Path(path));
        let report = serde_json::to_string_pretty(&selection.probes).unwrap();
        println!("{report}");
        if let Some(index) = args.iter().position(|arg| arg == "--out")
            && let Some(path) = args.get(index + 1)
            && std::fs::write(path, &report).is_err()
        {
            return ExitCode::FAILURE;
        }
        return if selection.selected.is_some() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    if args.first().and_then(|arg| arg.to_str()) != Some("validate") || args.len() != 2 {
        eprintln!(
            "usage: nickel-codex-fixture validate FILE-OR-DIRECTORY | compare-schema --codex PATH"
        );
        return ExitCode::from(2);
    }
    let path = PathBuf::from(&args[1]);
    let files = if path.is_dir() {
        match std::fs::read_dir(&path) {
            Ok(entries) => entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "json")
                })
                .collect(),
            Err(error) => {
                eprintln!("{}: {error}", path.display());
                return ExitCode::FAILURE;
            }
        }
    } else {
        vec![path]
    };
    for file in files {
        let result = if file
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("transcript-"))
        {
            std::fs::read_to_string(&file)
                .map_err(nickel_codex_fixture::FixtureError::from)
                .and_then(|input| nickel_codex_fixture::validate_transcript_str(&input).map(|_| ()))
        } else {
            nickel_codex_fixture::validate_file(&file).map(|_| ())
        };
        if let Err(error) = result {
            eprintln!("{}: {error}", file.display());
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn generate_schema(args: &[std::ffi::OsString]) -> ExitCode {
    let Some(index) = args.iter().position(|arg| arg == "--out") else {
        return ExitCode::from(2);
    };
    let Some(directory) = args.get(index + 1).map(PathBuf::from) else {
        return ExitCode::from(2);
    };
    if std::fs::create_dir_all(&directory).is_err() {
        return ExitCode::FAILURE;
    }
    let missing_method = env::args_os()
        .next()
        .and_then(|path| {
            PathBuf::from(path)
                .file_name()
                .map(|name| name.to_string_lossy().contains("missing-method"))
        })
        .unwrap_or(false);
    let files = [
        (
            "ClientRequest.json",
            &[
                "initialize",
                "account/read",
                "model/list",
                "thread/list",
                "thread/start",
                "thread/resume",
                "turn/start",
                "turn/interrupt",
            ][..],
        ),
        ("ClientNotification.json", &["initialized"][..]),
        (
            "ServerRequest.json",
            &[
                "item/commandExecution/requestApproval",
                "item/fileChange/requestApproval",
                "item/tool/requestUserInput",
            ][..],
        ),
        (
            "ServerNotification.json",
            &[
                "thread/started",
                "turn/started",
                "turn/completed",
                "item/started",
                "item/completed",
                "item/agentMessage/delta",
                "item/commandExecution/outputDelta",
                "item/fileChange/outputDelta",
                "item/plan/delta",
                "item/reasoning/summaryTextDelta",
                "account/updated",
                "error",
            ][..],
        ),
    ];
    for (name, methods) in files {
        let methods: Vec<_> = methods
            .iter()
            .copied()
            .filter(|method| !(missing_method && *method == "error"))
            .collect();
        if std::fs::write(
            directory.join(name),
            serde_json::to_vec(&json!({"methods": methods})).unwrap(),
        )
        .is_err()
        {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn serve_app_server() -> ExitCode {
    let mode = std::fs::read_to_string("fixture-mode").unwrap_or_default();
    if PathBuf::from("fixture-mode").is_file() {
        let _ = std::fs::write("fixture-pid", std::process::id().to_string());
    }
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let mut deferred: Option<(Value, String)> = None;
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            return ExitCode::FAILURE;
        };
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            return ExitCode::FAILURE;
        };
        match mode.trim() {
            "malformed" => {
                let _ = writeln!(stdout, "not-json");
                return ExitCode::FAILURE;
            }
            "exit" => return ExitCode::from(17),
            "hang" => {
                std::thread::sleep(std::time::Duration::from_secs(60));
                continue;
            }
            "hang-after-init"
                if request.get("method").and_then(Value::as_str) != Some("initialize") =>
            {
                std::thread::sleep(std::time::Duration::from_secs(60));
                continue;
            }
            "oversized" => {
                let _ = writeln!(stdout, "{}", "x".repeat(8 * 1024 * 1024 + 1));
                return ExitCode::FAILURE;
            }
            "stderr-flood"
                if request.get("method").and_then(Value::as_str) == Some("initialize") =>
            {
                let mut stderr = std::io::stderr().lock();
                let _ = stderr.write_all(&vec![b'e'; 128 * 1024]);
                let _ = stderr.flush();
            }
            _ => {}
        }
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            if PathBuf::from("fixture-mode").is_file() {
                let id = request
                    .get("id")
                    .map(|id| {
                        id.as_str()
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| id.to_string())
                    })
                    .unwrap_or_else(|| "unknown".into());
                let _ = std::fs::write(format!("fixture-response-{id}.json"), line.as_bytes());
            }
            continue;
        };
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        if mode.trim() == "out-of-order" && matches!(method, "account/read" | "model/list") {
            if let Some((old_id, old_method)) = deferred.take() {
                write_json(
                    &mut stdout,
                    &json!({"id":id,"result":fixture_result(method)}),
                );
                write_json(
                    &mut stdout,
                    &json!({"id":old_id,"result":fixture_result(&old_method)}),
                );
            } else {
                deferred = Some((id, method.to_owned()));
            }
            continue;
        }
        let result = match method {
            "initialize" => json!({"userAgent":"nickel-fixture/1"}),
            "account/read" => json!({"account": null}),
            "model/list" => {
                json!({"data":[{"id":"fixture-model","displayName":"Fixture Model"}],"nextCursor":null})
            }
            "thread/list" => json!({"data":[],"nextCursor":null}),
            "thread/start" => {
                write_json(
                    &mut stdout,
                    &json!({"method":"thread/started","params":{"thread":{"id":"fixture-thread"}}}),
                );
                json!({"thread":{"id":"fixture-thread","name":"Fixture thread","cwd":"/fixture"}})
            }
            "thread/resume" => {
                json!({"thread":{"id":request["params"]["threadId"],"name":"Fixture thread","cwd":"/fixture","turns":[]}})
            }
            "turn/start" => {
                write_json(
                    &mut stdout,
                    &json!({"method":"turn/started","params":{"threadId":"fixture-thread","turn":{"id":"fixture-turn","status":"inProgress"}}}),
                );
                write_json(
                    &mut stdout,
                    &json!({"method":"item/started","params":{"threadId":"fixture-thread","turnId":"fixture-turn","item":{"id":"message-1","type":"agentMessage"}}}),
                );
                write_json(
                    &mut stdout,
                    &json!({"method":"item/agentMessage/delta","params":{"threadId":"fixture-thread","turnId":"fixture-turn","itemId":"message-1","delta":"hello"}}),
                );
                if mode.trim() == "flood" {
                    write_json(
                        &mut stdout,
                        &json!({"method":"item/started","params":{"threadId":"fixture-thread","turnId":"fixture-turn","item":{"id":"command-flood","type":"commandExecution"}}}),
                    );
                    for _ in 0..1500 {
                        write_json(
                            &mut stdout,
                            &json!({"method":"item/commandExecution/outputDelta","params":{"threadId":"fixture-thread","turnId":"fixture-turn","itemId":"command-flood","delta":"x"}}),
                        );
                    }
                }
                write_json(
                    &mut stdout,
                    &json!({"id":71,"method":"item/commandExecution/requestApproval","params":{"threadId":"fixture-thread","turnId":"fixture-turn","itemId":"command-1","reason":"fixture approval"}}),
                );
                write_json(
                    &mut stdout,
                    &json!({"id":"input-1","method":"item/tool/requestUserInput","params":{"threadId":"fixture-thread","turnId":"fixture-turn","itemId":"input-item","questions":[{"id":"q1","header":"Choice","question":"Choose","options":[]}]}}),
                );
                if mode.trim() != "wait-for-interrupt" {
                    write_json(
                        &mut stdout,
                        &json!({"method":"turn/completed","params":{"threadId":"fixture-thread","turn":{"id":"fixture-turn","status":"completed"}}}),
                    );
                }
                if mode.trim() == "duplicate-terminal" {
                    write_json(
                        &mut stdout,
                        &json!({"method":"turn/completed","params":{"threadId":"fixture-thread","turn":{"id":"fixture-turn","status":"completed"}}}),
                    );
                }
                json!({"turn":{"id":"fixture-turn","status":"inProgress","items":[]}})
            }
            "turn/interrupt" => {
                write_json(
                    &mut stdout,
                    &json!({"method":"turn/completed","params":{"threadId":"fixture-thread","turn":{"id":"fixture-turn","status":"interrupted"}}}),
                );
                json!({})
            }
            _ => json!({}),
        };
        write_json(&mut stdout, &json!({"id":id,"result":result}));
    }
    ExitCode::SUCCESS
}

fn fixture_result(method: &str) -> Value {
    match method {
        "account/read" => json!({"account": null}),
        "model/list" => {
            json!({"data":[{"id":"fixture-model","displayName":"Fixture Model"}],"nextCursor":null})
        }
        _ => json!({}),
    }
}

fn write_json(writer: &mut impl Write, value: &Value) {
    let _ = serde_json::to_writer(&mut *writer, value);
    let _ = writer.write_all(b"\n");
    let _ = writer.flush();
}
